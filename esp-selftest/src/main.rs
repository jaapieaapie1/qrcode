//! ESP32-class execution self-test for the `qr-core` / `qr-encode` /
//! `qr-decode` stack.
//!
//! `cargo check --target riscv32imc-unknown-none-elf` proves the crates
//! *compile* for an ESP32-C3-class core. This binary proves they *run* there:
//! it is a bare-metal firmware for the QEMU `virt` machine — no `std`, no
//! operating system, no external crate, a hand-rolled reset vector, a 192 KiB
//! bump heap and a UART for output — that exercises the encoder and decoder end
//! to end and reports through the SiFive test finisher so QEMU's exit status is
//! the verdict.
//!
//! Four suites run, in order:
//!
//! 1. **QR roundtrip** — versions 1, 2, 3, 5, 7, 8 and 10 (so both character
//!    count widths and both with and without version information) at all four
//!    error-correction levels, each filled to within three codewords of
//!    capacity, encoded and decoded back byte-exact.
//! 2. **Micro QR** — every one of the eight symbols ISO Table 13 defines
//!    (M1-Detection, M2-L/M, M3-L/M, M4-L/M/Q). Numeric capacity is *probed*
//!    rather than tabulated here, and the largest payload that fits is
//!    roundtripped; M3 and M4 additionally roundtrip a byte-mode payload.
//! 3. **Reed-Solomon recovery** — corrupted codewords, single-block and
//!    interleaved multi-block, recovered to the exact original payload.
//! 4. **ISO 7.5.1 misdecode protection** — the symbols whose correction
//!    capacity `r` is below the Reed-Solomon bound must *reject* the extra
//!    error rather than hand back a confident wrong answer.
//!
//! # Building
//!
//! Build from **inside `esp-selftest/`**:
//!
//! ```text
//! cd esp-selftest && cargo build --release
//! qemu-system-riscv32 -machine virt -nographic -bios none \
//!     -kernel target/riscv32imc-unknown-none-elf/release/esp-selftest
//! ```
//!
//! The bare-metal target comes from `.cargo/config.toml`, which Cargo discovers
//! from the *working directory* and not from the manifest — so
//! `cargo build --manifest-path esp-selftest/Cargo.toml` from the repository
//! root builds for the host instead and dies in a pile of assembler errors. The
//! linker script comes from `build.rs`, which runs whatever the caller does.

#![no_std]
#![no_main]

extern crate alloc;

#[macro_use]
mod runtime;

use alloc::vec::Vec;

use qr_core::micro::{MicroEcLevel, MicroVersion};
use qr_core::{BitMatrix, EcLevel, Version, ec_blocks, is_function_module};
use qr_decode::{DecodeError, decode, decode_micro};
use qr_encode::{MicroOptions, Options, encode, encode_micro};

/// A suite failure. The detail is printed at the point of failure, so the
/// value itself only has to propagate.
struct Failed;

type Check = Result<(), Failed>;

/// Asserts a condition, printing the formatted detail and aborting the suite.
macro_rules! ensure {
    ($cond:expr, $($arg:tt)*) => {
        if !($cond) {
            println!("\n    FAIL: {}", format_args!($($arg)*));
            return Err(Failed);
        }
    };
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// A deterministic non-alphanumeric payload of `n` bytes.
///
/// Lowercase letters are outside the QR alphanumeric character set, so this
/// always selects byte mode and the decoded bytes must come back identical.
fn payload(n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| b'a' + (i * 7 + 3).rem_euclid(26) as u8)
        .collect()
}

/// The largest byte-mode payload that fits `version` at `ec`, less a three
/// codeword margin for the mode indicator, character count and terminator.
fn fill_size(version: Version, ec: EcLevel) -> usize {
    ec_blocks(version, ec).total_data_codewords() as usize - 3
}

/// Misdecode protection codewords `p` (ISO/IEC 18004:2024 §7.5.1): the error
/// correction codewords a symbol reserves to *detect* rather than correct.
///
/// Reimplemented here rather than imported, because `qr-decode` keeps its copy
/// private and because a test that asks the implementation what the answer
/// should be is not a test.
fn misdecode_protection(version: Version, ec: EcLevel) -> usize {
    match (version.number(), ec) {
        (1, EcLevel::L) => 3,
        (1, EcLevel::M) | (2, EcLevel::L) => 2,
        (1, EcLevel::Q) | (1, EcLevel::H) | (3, EcLevel::L) => 1,
        _ => 0,
    }
}

/// Per-block correction capacity `r` — ISO 7.5.1 Formula (3) with no erasures,
/// i.e. `t <= (d - p) / 2`. This is the `r` column of ISO Table 9, and it is at
/// or below the `d / 2` that Reed-Solomon alone would solve.
fn correction_capacity(version: Version, ec: EcLevel) -> usize {
    let d = ec_blocks(version, ec).ec_per_block as usize;
    (d - misdecode_protection(version, ec)) / 2
}

/// The module coordinates of the symbol's codeword stream, in placement order.
///
/// This is the two-column zigzag of ISO 7.7.3: column pairs from the right edge
/// leftward, alternating upward and downward, skipping the vertical timing
/// column and every function module. Element `8 * i` is therefore the most
/// significant bit of interleaved codeword `i`, which is what lets the
/// corruption suites damage an exact number of codewords instead of an
/// approximate number of modules.
///
/// This is an **independent reimplementation** of the walk in
/// `qr-encode/src/matrix.rs`, not a call into it, and it is maintained by hand:
/// the two agreeing is the thing being tested, so sharing the code would test
/// nothing. Drift is caught rather than tolerated — suite 4 asserts both sides
/// of four capacity boundaries, and corruption at the wrong granularity misses
/// every one of them.
fn codeword_modules(version: Version) -> Vec<(u32, u32)> {
    let size = version.size();
    let mut out = Vec::new();
    let mut right = size as i32 - 1;
    while right >= 1 {
        if right == 6 {
            right = 5;
        }
        for vert in 0..size {
            for j in 0..2 {
                let x = (right - j) as u32;
                let upward = ((right + 1) & 2) == 0;
                let y = if upward { size - 1 - vert } else { vert };
                if !is_function_module(version, x, y) {
                    out.push((x, y));
                }
            }
        }
        right -= 2;
    }
    out
}

/// Flips one module in each of the first `n` interleaved codewords, producing
/// exactly `n` erroneous codewords in the stream.
fn corrupt(matrix: &mut BitMatrix, version: Version, n: usize) {
    let modules = codeword_modules(version);
    for i in 0..n {
        let (x, y) = modules[i * 8];
        matrix.toggle(x, y);
    }
}

/// Encodes `data` into a symbol pinned to `version` and `ec`.
fn encode_at(data: &[u8], version: Version, ec: EcLevel) -> Result<BitMatrix, Failed> {
    let mut options = Options::new(ec);
    options.version = Some(version);
    match encode(data, options) {
        Ok(m) => Ok(m),
        Err(e) => {
            println!("\n    FAIL: encode v{} {:?}: {}", version.number(), ec, e);
            Err(Failed)
        }
    }
}

// ---------------------------------------------------------------------------
// Suite 1 — QR roundtrip
// ---------------------------------------------------------------------------

/// Versions sampled across the 1..=10 range: 1..=3 are single-block, 5 is
/// four-block interleaved, 7 is the first version carrying version information
/// and 10 is the first with a 16-bit byte-mode character count.
const SAMPLED_VERSIONS: [u8; 7] = [1, 2, 3, 5, 7, 8, 10];
const EC_LEVELS: [EcLevel; 4] = [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H];

fn suite_qr_roundtrip() -> Result<usize, Failed> {
    let mut symbols = 0usize;
    for number in SAMPLED_VERSIONS {
        let Ok(version) = Version::new(number) else {
            println!("\n    FAIL: version {number} rejected");
            return Err(Failed);
        };
        for ec in EC_LEVELS {
            let data = payload(fill_size(version, ec));
            let matrix = encode_at(&data, version, ec)?;
            ensure!(
                matrix.size() == version.size(),
                "v{number} {ec:?}: matrix is {} modules, expected {}",
                matrix.size(),
                version.size()
            );

            let decoded = match decode(&matrix) {
                Ok(d) => d,
                Err(e) => {
                    println!("\n    FAIL: v{number} {ec:?} decode: {e}");
                    return Err(Failed);
                }
            };
            ensure!(
                decoded.bytes == data,
                "v{number} {ec:?}: payload roundtrip differs ({} of {} bytes)",
                decoded.bytes.len(),
                data.len()
            );
            ensure!(
                decoded.version == version,
                "v{number} {ec:?}: decoded version {}",
                decoded.version.number()
            );
            ensure!(
                decoded.ec == ec,
                "v{number} {ec:?}: decoded level {:?}",
                decoded.ec
            );
            ensure!(
                decoded.errors_corrected == 0,
                "v{number} {ec:?}: {} corrections on a clean symbol",
                decoded.errors_corrected
            );
            symbols += 1;
        }
    }

    // One auto-selected symbol, to prove the version/mode search runs too.
    let text = b"https://example.invalid/esp32-selftest";
    let matrix = match encode(text, Options::new(EcLevel::M)) {
        Ok(m) => m,
        Err(e) => {
            println!("\n    FAIL: auto-version encode: {e}");
            return Err(Failed);
        }
    };
    let decoded = match decode(&matrix) {
        Ok(d) => d,
        Err(e) => {
            println!("\n    FAIL: auto-version decode: {e}");
            return Err(Failed);
        }
    };
    ensure!(
        decoded.bytes == text,
        "auto-version symbol did not roundtrip"
    );
    ensure!(
        decoded.text.as_deref() == Some("https://example.invalid/esp32-selftest"),
        "auto-version symbol did not decode as UTF-8"
    );
    Ok(symbols + 1)
}

// ---------------------------------------------------------------------------
// Suite 2 — Micro QR, all eight symbols
// ---------------------------------------------------------------------------

/// A deterministic numeric payload of `n` digits.
fn digits(n: usize) -> Vec<u8> {
    (0..n).map(|i| b'0' + (i % 10) as u8).collect()
}

fn micro_at(
    data: &[u8],
    version: MicroVersion,
    ec: MicroEcLevel,
) -> Result<Option<BitMatrix>, Failed> {
    let mut options = MicroOptions::new(ec);
    options.version = Some(version);
    match encode_micro(data, options) {
        Ok(m) => Ok(Some(m)),
        Err(qr_encode::MicroEncodeError::DataTooLarge) => Ok(None),
        Err(e) => {
            println!("\n    FAIL: M{} {:?} encode: {}", version.number(), ec, e);
            Err(Failed)
        }
    }
}

/// Roundtrips `data` through the given micro symbol.
fn micro_roundtrip(data: &[u8], version: MicroVersion, ec: MicroEcLevel) -> Check {
    let Some(matrix) = micro_at(data, version, ec)? else {
        println!(
            "\n    FAIL: M{} {:?}: {} bytes reported too large",
            version.number(),
            ec,
            data.len()
        );
        return Err(Failed);
    };
    ensure!(
        matrix.size() == version.size(),
        "M{} {ec:?}: matrix is {} modules, expected {}",
        version.number(),
        matrix.size(),
        version.size()
    );
    let decoded = match decode_micro(&matrix) {
        Ok(d) => d,
        Err(e) => {
            println!("\n    FAIL: M{} {:?} decode: {}", version.number(), ec, e);
            return Err(Failed);
        }
    };
    ensure!(
        decoded.bytes == data,
        "M{} {ec:?}: payload roundtrip differs",
        version.number()
    );
    ensure!(
        decoded.version == version,
        "M{} {ec:?}: decoded version M{}",
        version.number(),
        decoded.version.number()
    );
    ensure!(
        decoded.ec == ec,
        "M{} {ec:?}: decoded level {:?}",
        version.number(),
        decoded.ec
    );
    Ok(())
}

fn suite_micro() -> Result<usize, Failed> {
    let mut symbols = 0usize;
    for version in MicroVersion::ALL {
        for &ec in version.ec_levels() {
            // Probe the numeric capacity instead of tabulating it: the largest
            // payload that still encodes is the interesting one to roundtrip,
            // and finding it exercises the capacity arithmetic as well.
            const PROBE_CEILING: usize = 40;
            let mut capacity = 0usize;
            for n in 1..=PROBE_CEILING {
                if micro_at(&digits(n), version, ec)?.is_some() {
                    capacity = n;
                } else {
                    break;
                }
            }
            ensure!(
                capacity > 0,
                "M{} {ec:?}: no numeric payload fits at all",
                version.number()
            );
            // The probe must have found a real boundary, not run out of road.
            // M4-L is the largest at 35 digits, so there are five to spare
            // today; if a capacity ever reached the ceiling the probe would
            // silently roundtrip a sub-maximal payload and still report PASS.
            ensure!(
                capacity < PROBE_CEILING,
                "M{} {ec:?}: probe hit its {PROBE_CEILING}-digit ceiling, capacity not bounded",
                version.number()
            );
            micro_roundtrip(&digits(capacity), version, ec)?;

            // M1 and M2 have no byte mode (ISO Table 2); M3 and M4 do, and a
            // five byte payload is inside even the smallest of them (M4-Q).
            if version.number() >= 3 {
                micro_roundtrip(b"esp32", version, ec)?;
            }
            symbols += 1;
        }
    }
    ensure!(symbols == 8, "expected 8 micro symbols, ran {symbols}");
    Ok(symbols)
}

// ---------------------------------------------------------------------------
// Suite 3 — Reed-Solomon recovery
// ---------------------------------------------------------------------------

/// Corrupts `errors` codewords of a freshly encoded symbol and requires the
/// decoder to return the original payload having corrected exactly that many.
fn recovers(version: Version, ec: EcLevel, errors: usize, data: &[u8]) -> Check {
    let mut matrix = encode_at(data, version, ec)?;
    corrupt(&mut matrix, version, errors);
    let decoded = match decode(&matrix) {
        Ok(d) => d,
        Err(e) => {
            println!(
                "\n    FAIL: v{} {:?} lost {} corrupted codewords: {}",
                version.number(),
                ec,
                errors,
                e
            );
            return Err(Failed);
        }
    };
    ensure!(
        decoded.bytes == data,
        "v{} {ec:?}: recovered payload differs after {errors} errors",
        version.number()
    );
    ensure!(
        decoded.errors_corrected == errors,
        "v{} {ec:?}: reported {} corrections, injected {errors}",
        version.number(),
        decoded.errors_corrected
    );
    Ok(())
}

/// Recovery at exactly the published capacity `r`, with `r` checked against the
/// ISO Table 9 value the caller expects before it is used.
fn recovers_at_capacity(version: Version, ec: EcLevel, expected_r: usize) -> Check {
    let r = correction_capacity(version, ec);
    ensure!(
        r == expected_r,
        "v{} {ec:?}: capacity is {r}, ISO Table 9 says {expected_r}",
        version.number()
    );
    recovers(version, ec, r, &payload(fill_size(version, ec)))
}

fn suite_reed_solomon() -> Result<usize, Failed> {
    let v1 = Version::new(1).expect("version 1");
    let v5 = Version::new(5).expect("version 5");

    // 1-L: (7 - 3) / 2 = 2 after the ISO 7.5.1 reservation.
    recovers_at_capacity(v1, EcLevel::L, 2)?;
    // 1-H: 17 error correction codewords, p = 1, so (17 - 1) / 2 = 8.
    recovers_at_capacity(v1, EcLevel::H, 8)?;

    // 5-Q is four interleaved blocks of 18 error correction codewords with
    // p = 0, so r = 9 per block. Interleaving deals consecutive stream
    // codewords round-robin, so damaging the first 36 puts exactly the full
    // capacity of 9 errors into each of the four blocks — the hardest case the
    // multi-block path has.
    let blocks = ec_blocks(v5, EcLevel::Q);
    let per_block = correction_capacity(v5, EcLevel::Q);
    ensure!(
        per_block == 9 && blocks.total_blocks() == 4,
        "v5 Q: expected 4 blocks of capacity 9, found {} of {per_block}",
        blocks.total_blocks()
    );
    let total = per_block * blocks.total_blocks() as usize;
    recovers(v5, EcLevel::Q, total, &payload(fill_size(v5, EcLevel::Q)))?;

    Ok(3)
}

// ---------------------------------------------------------------------------
// Suite 4 — ISO 7.5.1 misdecode protection
// ---------------------------------------------------------------------------

/// Requires the decoder to reject `errors` corrupted codewords outright.
///
/// Reed-Solomon can solve these; ISO 7.5.1 Formula (3) reserves `p` of the
/// error correction codewords for detection and forbids it, because accepting
/// the correction would return a confidently wrong payload.
fn rejects(version: Version, ec: EcLevel, errors: usize) -> Check {
    let data = payload(fill_size(version, ec));
    let mut matrix = encode_at(&data, version, ec)?;
    corrupt(&mut matrix, version, errors);
    match decode(&matrix) {
        Err(DecodeError::BlockCorrection) => Ok(()),
        Err(e) => {
            println!(
                "\n    FAIL: v{} {:?} with {} errors: expected BlockCorrection, got {}",
                version.number(),
                ec,
                errors,
                e
            );
            Err(Failed)
        }
        Ok(d) => {
            println!(
                "\n    FAIL: v{} {:?} accepted {} errors and decoded {} bytes",
                version.number(),
                ec,
                errors,
                d.bytes.len()
            );
            Err(Failed)
        }
    }
}

/// Version number, level, and the capacity `r` published in ISO Table 9.
///
/// The literals are the point: they come from the standard, not from the
/// formula, so [`correction_capacity`] is checked against them rather than
/// trusted. 1-L is the headline — 7 error correction codewords give a
/// Reed-Solomon bound of 3, but `p = 3` caps it at 2. 1-M and 2-L have `p = 2`
/// against a bound of 5. 1-Q is the control: `p = 1`, but `(13 - 1) / 2` is
/// still 6, so the cap must not have narrowed anything there.
const RCAP_CASES: [(u8, EcLevel, usize); 4] = [
    (1, EcLevel::L, 2),
    (1, EcLevel::M, 4),
    (2, EcLevel::L, 4),
    (1, EcLevel::Q, 6),
];

fn suite_rcap() -> Result<usize, Failed> {
    let mut cases = 0usize;
    for (number, ec, expected_r) in RCAP_CASES {
        let Ok(version) = Version::new(number) else {
            println!("\n    FAIL: version {number} rejected");
            return Err(Failed);
        };
        // Both sides of the boundary: r must be corrected, r + 1 must be
        // refused outright even though Reed-Solomon could still solve it.
        recovers_at_capacity(version, ec, expected_r)?;
        rejects(version, ec, expected_r + 1)?;
        cases += 2;
    }
    Ok(cases)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Runs a suite, printing its name padded to a fixed width, then its verdict.
///
/// The heap is released before each suite. A bump allocator only reclaims
/// blocks freed in reverse order, so without this the holes left by the
/// encoder's and decoder's interleaved allocations accumulate across suites and
/// the reported peak measures fragmentation rather than working set.
fn run(index: usize, name: &str, suite: fn() -> Result<usize, Failed>, unit: &str) -> bool {
    // SAFETY: nothing allocated by a previous suite is still live — a suite
    // returns only a count, and this runs before the next one allocates.
    unsafe { runtime::heap_reset() };

    print!("[{index}/4] {name}");
    for _ in name.len()..38 {
        print!(".");
    }
    match suite() {
        Ok(count) => {
            println!(" PASS ({count} {unit})");
            true
        }
        Err(Failed) => {
            println!("[{index}/4] {name}: FAILED");
            false
        }
    }
}

/// Called from `_start` with a valid stack and a zeroed `.bss`. Never returns:
/// every path ends in the SiFive test finisher, which is what gives QEMU its
/// exit status.
#[unsafe(no_mangle)]
pub extern "C" fn rust_main() -> ! {
    println!();
    println!("qr-selftest: riscv32imc-unknown-none-elf on qemu virt");
    println!("             qr-core + qr-encode + qr-decode, no_std + alloc");
    println!();

    let ok = run(
        1,
        "qr roundtrip, v1-v10 sampled",
        suite_qr_roundtrip,
        "symbols",
    ) && run(2, "micro qr, all 8 symbols", suite_micro, "symbols")
        && run(3, "reed-solomon recovery", suite_reed_solomon, "cases")
        && run(4, "iso 7.5.1 misdecode protection", suite_rcap, "cases");

    println!();
    println!(
        "heap peak {} of {} bytes (largest single suite)",
        runtime::heap_peak(),
        runtime::HEAP_SIZE
    );
    if ok {
        println!("ALL PASS");
    } else {
        println!("FAILED");
    }
    runtime::exit(ok)
}
