//! A small hand-rolled benchmark harness.
//!
//! The workspace carries no external dependencies beyond `image` and
//! wasm-bindgen, and benchmarking is not a good enough reason to break that, so
//! this is criterion's job done in ~200 lines of `std`:
//!
//! * [`bench`] auto-calibrates an iteration count, warms up, then times each
//!   iteration individually and reports min / median / p95 / max.
//! * [`bench_units`] adds a throughput (or ns/op) column for benches where a
//!   per-iteration work quantity is meaningful.
//! * [`group`] prints an aligned table header; every following line lands under
//!   it.
//!
//! # Statistics, honestly
//!
//! This is a *timing table*, not a statistical regression detector. There is no
//! outlier rejection, no bootstrapping, no confidence interval, and no
//! comparison against a stored baseline. Read the median for "how fast", the
//! min for "how fast when nothing interferes", and the p95/max spread for "how
//! jittery". Anything finer wants a real statistics package.
//!
//! Every timed iteration pays one [`Instant::now`] pair (tens of nanoseconds on
//! Apple Silicon). For work in that same range — a single GF(256) multiply —
//! the clock would be the measurement, so those benches loop internally and
//! report ns/op via [`Unit::Ops`].
//!
//! # Modes
//!
//! Full mode aims each bench at [`TARGET_NANOS`] (~1s) of wall time, using the
//! caller's `iters_hint` as a ceiling. Setting `BENCH_QUICK=1` switches to smoke
//! mode: [`QUICK_ITERS`] iterations, one warmup, no calibration. Quick-mode
//! numbers are for proving the bench runs, not for reading.

use std::hint::black_box;
use std::time::{Duration, Instant};

/// Wall-clock time each bench aims for in full mode.
const TARGET_NANOS: u128 = 1_000_000_000;

/// Time the calibration loop must accumulate before its estimate is trusted.
const CALIBRATE_NANOS: u128 = 20_000_000;

/// Iterations run per bench when `BENCH_QUICK=1`.
const QUICK_ITERS: u64 = 3;

/// Column widths for the output table.
const NAME_W: usize = 48;
const ITERS_W: usize = 9;
const TIME_W: usize = 11;
const RATE_W: usize = 15;

/// A per-iteration work quantity, turned into the table's rightmost column.
#[derive(Clone, Copy, Debug)]
pub enum Unit {
    /// Bytes processed per iteration; reported as MiB/s.
    Bytes(u64),
    /// Discrete work items per iteration; reported as items/s.
    Items(u64),
    /// Primitive operations per iteration; reported as ns/op.
    ///
    /// Use this for benches whose closure loops internally to get above the
    /// clock's resolution.
    Ops(u64),
}

/// Whether smoke mode is on (`BENCH_QUICK=1`).
///
/// Any non-empty value other than `0` counts as on.
#[must_use]
pub fn quick() -> bool {
    match std::env::var("BENCH_QUICK") {
        Ok(v) => !v.is_empty() && v != "0",
        Err(_) => false,
    }
}

/// Prints a table header for a group of benches.
pub fn group(title: &str) {
    println!();
    println!("{title}");
    println!(
        "{:<NAME_W$} {:>ITERS_W$} {:>TIME_W$} {:>TIME_W$} {:>TIME_W$} {:>TIME_W$} {:>RATE_W$}",
        "bench", "iters", "min", "median", "p95", "max", "rate"
    );
    println!("{}", "-".repeat(NAME_W + ITERS_W + 4 * TIME_W + RATE_W + 6));
}

/// Times `f` and prints one table row.
///
/// `iters_hint` caps the iteration count; calibration may choose fewer if a
/// single iteration is expensive. Pass a hint you would be happy to run at:
/// there is no upper bound on the time a bench may take beyond this.
pub fn bench<R>(name: &str, iters_hint: u64, f: impl FnMut() -> R) {
    run(name, iters_hint, None, f);
}

/// [`bench`] with a throughput column derived from the median iteration.
pub fn bench_units<R>(name: &str, iters_hint: u64, unit: Unit, f: impl FnMut() -> R) {
    run(name, iters_hint, Some(unit), f);
}

fn run<R>(name: &str, iters_hint: u64, unit: Option<Unit>, mut f: impl FnMut() -> R) {
    assert!(iters_hint > 0, "{name}: iters_hint must be positive");

    let iters = if quick() {
        black_box(f());
        QUICK_ITERS.min(iters_hint)
    } else {
        let per_iter = calibrate(&mut f, iters_hint);
        // A zero estimate means the iteration is below the clock's resolution;
        // fall back to the caller's ceiling rather than dividing by zero.
        let target = TARGET_NANOS
            .checked_div(per_iter)
            .map_or(iters_hint, |n| u64::try_from(n).unwrap_or(u64::MAX));
        let iters = target.clamp(1, iters_hint);
        for _ in 0..(iters / 10).max(3) {
            black_box(f());
        }
        iters
    };

    let mut samples = Vec::with_capacity(iters as usize);
    for _ in 0..iters {
        let start = Instant::now();
        black_box(f());
        samples.push(start.elapsed());
    }
    samples.sort_unstable();

    let median = percentile(&samples, 0.50);
    println!(
        "{:<NAME_W$} {:>ITERS_W$} {:>TIME_W$} {:>TIME_W$} {:>TIME_W$} {:>TIME_W$} {:>RATE_W$}",
        truncate(name, NAME_W),
        iters,
        fmt_dur(samples[0]),
        fmt_dur(median),
        fmt_dur(percentile(&samples, 0.95)),
        fmt_dur(samples[samples.len() - 1]),
        unit.map_or_else(String::new, |u| fmt_rate(u, median)),
    );
}

/// Estimates one iteration's cost, doubling the batch until the batch is long
/// enough to outrun clock noise. Doubles as the first warmup.
fn calibrate<R>(f: &mut impl FnMut() -> R, iters_hint: u64) -> u128 {
    let mut n: u64 = 1;
    loop {
        let start = Instant::now();
        for _ in 0..n {
            black_box(f());
        }
        let elapsed = start.elapsed().as_nanos();
        if elapsed >= CALIBRATE_NANOS || n >= iters_hint {
            return elapsed / u128::from(n);
        }
        n = n.saturating_mul(4);
    }
}

/// Nearest-rank percentile over an already-sorted slice.
fn percentile(sorted: &[Duration], q: f64) -> Duration {
    debug_assert!(!sorted.is_empty());
    let idx = ((sorted.len() as f64) * q) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    s.chars().take(width - 1).chain(['~']).collect()
}

fn fmt_dur(d: Duration) -> String {
    let ns = d.as_nanos() as f64;
    if ns < 1_000.0 {
        format!("{ns:.0} ns")
    } else if ns < 1_000_000.0 {
        format!("{:.2} µs", ns / 1e3)
    } else if ns < 1e9 {
        format!("{:.2} ms", ns / 1e6)
    } else {
        format!("{:.3} s", ns / 1e9)
    }
}

fn fmt_rate(unit: Unit, median: Duration) -> String {
    let secs = median.as_nanos() as f64 / 1e9;
    if secs <= 0.0 {
        return "-".to_string();
    }
    match unit {
        Unit::Bytes(n) => {
            let per_sec = n as f64 / secs;
            if per_sec >= 1024.0 * 1024.0 {
                format!("{:.1} MiB/s", per_sec / (1024.0 * 1024.0))
            } else {
                format!("{:.1} KiB/s", per_sec / 1024.0)
            }
        }
        Unit::Items(n) => {
            let per_sec = n as f64 / secs;
            if per_sec >= 1e6 {
                format!("{:.2} M/s", per_sec / 1e6)
            } else if per_sec >= 1e3 {
                format!("{:.2} k/s", per_sec / 1e3)
            } else {
                format!("{per_sec:.1} /s")
            }
        }
        Unit::Ops(n) => {
            assert!(n > 0, "Unit::Ops(0) has no meaningful rate");
            format!("{:.2} ns/op", median.as_nanos() as f64 / n as f64)
        }
    }
}
