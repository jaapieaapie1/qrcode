//! qr-core primitives: Reed-Solomon encode/correct, format-info BCH, GF(256).
//!
//! Several of these run in well under a microsecond, which is the same order as
//! the two `Instant::now` calls that would bracket them. Those benches loop `B`
//! times inside one timed iteration — the name carries the batch as `xB`, and
//! the rate column is per-item, not per-iteration.

mod common;

use benchmarks::{Unit, bench_units, group};
use qr_core::{EcLevel, Version, bch, ec_blocks, gf, reed_solomon};
use std::hint::black_box;

/// Batch sizes: enough repetitions that one timed iteration is comfortably
/// longer than the clock's own cost.
const RS_SMALL_BATCH: u64 = 256;
const RS_LARGE_BATCH: u64 = 32;
const BCH_BATCH: u64 = 256;
const GF_OPS: u64 = 1 << 16;

/// One Reed-Solomon block shape: `data` payload bytes plus `ec` check bytes.
struct BlockShape {
    label: &'static str,
    data: Vec<u8>,
    ec_len: usize,
}

impl BlockShape {
    fn new(label: &'static str, version: u8, ec: EcLevel, seed: u32) -> BlockShape {
        let blocks = ec_blocks(Version::new(version).expect("valid version"), ec);
        let data = common::pseudo_bytes(usize::from(blocks.group1_data), seed);
        BlockShape {
            label,
            data,
            ec_len: usize::from(blocks.ec_per_block),
        }
    }

    /// The full codeword block (data followed by EC), as a decoder sees it.
    fn codewords(&self) -> Vec<u8> {
        let mut cw = self.data.clone();
        cw.extend_from_slice(&reed_solomon::rs_encode(&self.data, self.ec_len));
        cw
    }

    /// [`Self::codewords`] with `n` codewords corrupted, spread evenly so the
    /// damage is not clustered at one end of the block.
    fn corrupted(&self, n: usize) -> Vec<u8> {
        let mut cw = self.codewords();
        let stride = cw.len() / n.max(1);
        for i in 0..n {
            let pos = i * stride;
            cw[pos] ^= 0x5a | (i as u8);
        }
        cw
    }

    /// The guaranteed correction capacity: floor(ec / 2) codewords.
    fn capacity(&self) -> usize {
        self.ec_len / 2
    }
}

fn bench_rs(shape: &BlockShape, batch: u64) {
    let data_bytes = shape.data.len() as u64;

    bench_units(
        &format!(
            "rs_encode {} ({}d/{}ec) x{batch}",
            shape.label,
            shape.data.len(),
            shape.ec_len
        ),
        200_000,
        Unit::Bytes(data_bytes * batch),
        || {
            for _ in 0..batch {
                black_box(reed_solomon::rs_encode(
                    black_box(&shape.data),
                    shape.ec_len,
                ));
            }
        },
    );

    let clean = shape.codewords();
    let damaged = shape.corrupted(shape.capacity());
    // Correction is in-place, so each repetition restores the block first. The
    // copy is inside the timed region; at ~150 bytes it is a rounding error
    // next to the syndrome/Chien work it feeds.
    let mut scratch = vec![0u8; clean.len()];

    let err_tag = format!("{} err", shape.capacity());
    for (tag, src) in [("clean", &clean), (err_tag.as_str(), &damaged)] {
        bench_units(
            &format!(
                "rs_correct {} ({}cw) {tag} x{batch}",
                shape.label,
                clean.len()
            ),
            200_000,
            Unit::Ops(batch),
            || {
                for _ in 0..batch {
                    scratch.copy_from_slice(src);
                    black_box(reed_solomon::rs_correct(
                        black_box(&mut scratch),
                        shape.ec_len,
                    ))
                    .expect("within correction capacity");
                }
            },
        );
    }
}

fn main() {
    group("qr-core — Reed-Solomon");

    let v1m = BlockShape::new("v1-M", 1, EcLevel::M, 0x1234_5678);
    let v40l = BlockShape::new("v40-L", 40, EcLevel::L, 0x9e37_79b9);
    bench_rs(&v1m, RS_SMALL_BATCH);
    bench_rs(&v40l, RS_LARGE_BATCH);

    group("qr-core — BCH format information");

    // Every legal (EC level, mask) pair: 4 x 8 = 32 format codes.
    let codes: Vec<u16> = EcLevel::ALL
        .iter()
        .flat_map(|&ec| (0..8u8).map(move |mask| bch::format_info_encode(ec, mask)))
        .collect();
    bench_units(
        &format!("format_info_decode, 32 valid codes x{BCH_BATCH}"),
        20_000,
        Unit::Ops(32 * BCH_BATCH),
        || {
            for _ in 0..BCH_BATCH {
                for &c in &codes {
                    black_box(bch::format_info_decode(black_box(c))).expect("valid format code");
                }
            }
        },
    );

    // One flipped bit per code: the correcting path, not the exact-match path.
    let damaged: Vec<u16> = codes
        .iter()
        .enumerate()
        .map(|(i, &c)| c ^ (1 << (i % 15)))
        .collect();
    bench_units(
        &format!("format_info_decode, 1-bit errors x{BCH_BATCH}"),
        20_000,
        Unit::Ops(32 * BCH_BATCH),
        || {
            for _ in 0..BCH_BATCH {
                for &c in &damaged {
                    black_box(bch::format_info_decode(black_box(c)))
                        .expect("1-bit error is correctable");
                }
            }
        },
    );

    group("qr-core — GF(256)");

    bench_units(
        &format!("gf::mul, {GF_OPS} ops"),
        20_000,
        Unit::Ops(GF_OPS),
        || {
            let mut acc = 0u8;
            for i in 0..GF_OPS {
                let a = (i & 0xff) as u8;
                let b = ((i >> 8) & 0xff) as u8;
                acc ^= gf::mul(black_box(a), black_box(b));
            }
            acc
        },
    );

    bench_units(
        &format!("gf::inv, {GF_OPS} ops"),
        20_000,
        Unit::Ops(GF_OPS),
        || {
            let mut acc = 0u8;
            for i in 0..GF_OPS {
                // 1..=255: inverting zero is undefined in GF(256).
                let a = (i % 255 + 1) as u8;
                acc ^= gf::inv(black_box(a));
            }
            acc
        },
    );

    println!();
}
