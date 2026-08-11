//! qr-decode: `decode()` on clean matrices at three sizes, on a matrix damaged
//! up to the point Reed-Solomon can still repair it, and `decode_micro()`.
//!
//! The corrupted fixture is built by flipping evenly-spread data modules and
//! then walking the flip count down until the symbol decodes again — so it is
//! genuinely the *most* damage this symbol survives, which is where the
//! error-locator work is heaviest.

mod common;

use benchmarks::{Unit, bench_units, group};
use common::{must_encode, qr_fixture};
use qr_core::{BitMatrix, EcLevel, Version, ec_blocks, is_function_module};
use qr_decode::decode;
use qr_encode::Options;
use std::hint::black_box;

/// Repetitions per timed iteration for the two symbols that decode in under
/// ~2 µs. See the comment at the v1 bench.
const SMALL_BATCH: u64 = 64;

/// Flips `flips` modules, evenly spread across the symbol's non-function
/// modules so the damage lands in as many Reed-Solomon blocks as possible.
fn corrupt(matrix: &BitMatrix, version: Version, flips: usize) -> BitMatrix {
    let data_positions: Vec<(u32, u32)> = (0..matrix.size())
        .flat_map(|y| (0..matrix.size()).map(move |x| (x, y)))
        .filter(|&(x, y)| !is_function_module(version, x, y))
        .collect();
    assert!(flips > 0 && flips <= data_positions.len());

    let mut out = matrix.clone();
    let stride = data_positions.len() / flips;
    for i in 0..flips {
        let (x, y) = data_positions[i * stride];
        out.toggle(x, y);
    }
    out
}

/// The most-damaged version of `matrix` that still decodes, plus the flip count
/// and the number of codewords Reed-Solomon had to repair.
fn worst_correctable(matrix: &BitMatrix, version: Version, ec: EcLevel) -> (BitMatrix, usize) {
    let blocks = ec_blocks(version, ec);
    let ceiling = usize::from(blocks.total_blocks()) * usize::from(blocks.ec_per_block / 2);
    for flips in (1..=ceiling).rev() {
        let damaged = corrupt(matrix, version, flips);
        if let Ok(d) = decode(&damaged) {
            assert!(d.errors_corrected > 0, "fixture must actually be damaged");
            return (damaged, flips);
        }
    }
    panic!("no correctable corruption found for v{}", version.number());
}

fn main() {
    let v = |n: u8| Version::new(n).expect("valid version");

    let v1 = must_encode(b"01234567", Options::new(EcLevel::M));
    let (v10, v10_payload) = qr_fixture(10, EcLevel::M, 0x0bad_c0de);
    let (v40, v40_payload) = qr_fixture(40, EcLevel::L, 0xdead_beef);
    let (v10_len, v40_len) = (v10_payload.len(), v40_payload.len());
    assert_eq!(v1.size(), 21);

    let (v10_damaged, flips) = worst_correctable(&v10, v(10), EcLevel::M);
    let corrected = decode(&v10_damaged)
        .expect("fixture decodes")
        .errors_corrected;

    group("qr-decode — decode()");

    // v1 decodes in ~1.7 µs, which is only ~41 ticks of this machine's 41.67 ns
    // `Instant` granularity — enough that the clock is a couple of percent of
    // the reading. Batch it like the qr-core benches; the larger versions are
    // three and four orders of magnitude clear of the clock and stay unbatched.
    bench_units(
        &format!("decode v1-M, 8 numeric digits x{SMALL_BATCH}"),
        20_000,
        Unit::Bytes(8 * SMALL_BATCH),
        || {
            for _ in 0..SMALL_BATCH {
                black_box(decode(black_box(&v1)).expect("clean fixture decodes"));
            }
        },
    );

    for (label, matrix, payload_len) in [
        (format!("decode v10-M, {v10_len}B byte"), &v10, v10_len),
        (format!("decode v40-L, {v40_len}B byte"), &v40, v40_len),
    ] {
        bench_units(&label, 20_000, Unit::Bytes(payload_len as u64), || {
            decode(black_box(matrix)).expect("clean fixture decodes")
        });
    }

    bench_units(
        &format!("decode v10-M, {flips} modules flipped ({corrected} cw fixed)"),
        20_000,
        Unit::Bytes(v10_len as u64),
        || decode(black_box(&v10_damaged)).expect("damaged fixture decodes"),
    );

    #[cfg(feature = "micro")]
    {
        use qr_core::micro::{MicroEcLevel, MicroVersion};
        use qr_decode::decode_micro;
        use qr_encode::MicroOptions;

        group("qr-decode — decode_micro()");

        let payload = b"HELLO M4Q";
        let m4 = qr_encode::encode_micro(
            payload,
            MicroOptions {
                version: Some(MicroVersion::M4),
                ..MicroOptions::new(MicroEcLevel::Q)
            },
        )
        .expect("M4-Q fixture must encode");
        assert_eq!(m4.size(), 17);

        bench_units(
            &format!("decode_micro M4-Q, 9 alphanum chars x{SMALL_BATCH}"),
            20_000,
            Unit::Bytes(payload.len() as u64 * SMALL_BATCH),
            || {
                for _ in 0..SMALL_BATCH {
                    black_box(decode_micro(black_box(&m4)).expect("M4-Q fixture decodes"));
                }
            },
        );
    }

    println!();
}
