//! Equivalence tests: every packed routine against the per-module reference in
//! [`crate::matrix::reference`], which is the executable statement of the ISO
//! rules this module reimplements.

use super::*;
use crate::matrix::reference;
use alloc::vec::Vec;
use qr_core::{ec_blocks, is_function_module};

/// Deterministic xorshift64* — the crate takes no dependencies, and a fixed
/// seed keeps failures reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| (self.next() >> 24) as u8).collect()
    }
}

fn versions() -> impl Iterator<Item = Version> {
    (1..=40u8).map(|n| Version::new(n).unwrap())
}

/// Packed rows holding the same modules as `matrix`.
fn pack(matrix: &BitMatrix) -> Vec<u64> {
    let size = matrix.size();
    let words = words_per_row(size);
    let mut rows = vec![0u64; words * size as usize];
    for y in 0..size {
        for x in 0..size {
            set_bit(&mut rows, words, x, y, matrix.get(x, y));
        }
    }
    rows
}

/// An all-light `BitMatrix` filled from packed rows.
fn unpack(rows: &[u64], size: u32) -> BitMatrix {
    let mut m = BitMatrix::new(size);
    write_bitmatrix(rows, words_per_row(size), size, &mut m);
    m
}

#[test]
fn transpose64_matches_naive() {
    let mut rng = Rng(0x1234_5678_9abc_def0);
    for _ in 0..64 {
        let mut a = [0u64; 64];
        for slot in a.iter_mut() {
            *slot = rng.next();
        }
        let src = a;
        transpose64(&mut a);
        for (r, &row) in src.iter().enumerate() {
            for (k, &col) in a.iter().enumerate() {
                assert_eq!(
                    (col >> r) & 1,
                    (row >> k) & 1,
                    "bit ({r}, {k}) after transpose"
                );
            }
        }
    }
}

#[test]
fn transpose_matches_naive_for_every_version() {
    let mut rng = Rng(0xfeed_face_dead_beef);
    for v in versions() {
        let size = v.size();
        let words = words_per_row(size);
        let mut rows = vec![0u64; words * size as usize];
        for y in 0..size {
            for x in 0..size {
                set_bit(&mut rows, words, x, y, rng.next() & 1 == 1);
            }
        }
        let mut out = vec![0u64; rows.len()];
        transpose(&rows, words, size, &mut out);
        for y in 0..size {
            for x in 0..size {
                assert_eq!(
                    get_bit(&out, words, y, x),
                    get_bit(&rows, words, x, y),
                    "v{} ({x}, {y})",
                    v.number()
                );
            }
        }
    }
}

#[test]
fn mask_patterns_repeat_every_twelve_rows() {
    for m in 0..8u8 {
        for y in 0..200u32 {
            for x in 0..200u32 {
                assert_eq!(
                    mask_bit(m, x, y),
                    mask_bit(m, x, y % MASK_PERIOD as u32),
                    "mask {m} at ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn function_map_matches_is_function_module() {
    for v in versions() {
        let words = words_per_row(v.size());
        let mut map = vec![0u64; words * v.size() as usize];
        function_map(&mut map, words, v);
        for y in 0..v.size() {
            for x in 0..v.size() {
                assert_eq!(
                    get_bit(&map, words, x, y),
                    is_function_module(v, x, y),
                    "v{} ({x}, {y})",
                    v.number()
                );
            }
        }
    }
}

#[test]
fn packed_placement_matches_reference() {
    let mut rng = Rng(0x0bad_c0de_0bad_c0de);
    for v in versions() {
        let total = ec_blocks(v, EcLevel::L).total_codewords() as usize;
        let stream = rng.bytes(total);

        let mut expected = BitMatrix::new(v.size());
        reference::place_function_patterns(&mut expected, v);
        reference::place_data(&mut expected, v, &stream);
        reference::place_version_info(&mut expected, v);

        let words = words_per_row(v.size());
        let cells = words * v.size() as usize;
        let mut funcs = vec![0u64; cells];
        function_map(&mut funcs, words, v);
        let mut got = vec![0u64; cells];
        place_function_patterns(&mut got, words, v);
        place_data(&mut got, words, &funcs, v, &stream);
        place_version_info(&mut got, words, v);

        for y in 0..v.size() {
            for x in 0..v.size() {
                assert_eq!(
                    get_bit(&got, words, x, y),
                    expected.get(x, y),
                    "v{} ({x}, {y})",
                    v.number()
                );
            }
        }
    }
}

#[test]
fn packed_score_matches_reference_on_random_grids() {
    // Random module patterns exercise the scorer far outside the shapes a real
    // symbol produces: long runs, dense 2x2 blocks and scaled finder patterns.
    let mut rng = Rng(0x5eed_5eed_5eed_5eed);
    let sizes = [21u32, 25, 45, 63, 64, 65, 77, 127, 128, 129, 141, 177];
    for &size in &sizes {
        let words = words_per_row(size);
        for round in 0..24 {
            let mut rows = vec![0u64; words * size as usize];
            for y in 0..size {
                for x in 0..size {
                    // Vary the density so runs of every length show up.
                    let bit = match round % 4 {
                        0 => rng.next() & 1 == 1,
                        1 => rng.next().is_multiple_of(5),
                        2 => !rng.next().is_multiple_of(5),
                        _ => (rng.next() % 7) < 3 && (x / 3 + y / 3).is_multiple_of(2),
                    };
                    set_bit(&mut rows, words, x, y, bit);
                }
            }
            let matrix = unpack(&rows, size);
            let mut transposed = vec![0u64; rows.len()];
            assert_eq!(
                penalty_score(&rows, words, size, &mut transposed),
                reference::penalty_score(&matrix),
                "size {size} round {round}"
            );
        }
    }
}

#[test]
fn packed_score_components_match_reference_on_symbols() {
    let mut rng = Rng(0xabad_1dea_abad_1dea);
    for v in versions() {
        let total = ec_blocks(v, EcLevel::Q).total_codewords() as usize;
        let stream = rng.bytes(total);
        for mask in 0..8u8 {
            let mut matrix = BitMatrix::new(v.size());
            reference::place_function_patterns(&mut matrix, v);
            reference::place_data(&mut matrix, v, &stream);
            reference::apply_mask(&mut matrix, v, mask);
            reference::place_format_info(&mut matrix, v, EcLevel::Q, mask);
            if v.number() >= 7 {
                reference::place_version_info(&mut matrix, v);
            }

            let rows = pack(&matrix);
            let mut transposed = vec![0u64; rows.len()];
            assert_eq!(
                penalty_score(&rows, words_per_row(v.size()), v.size(), &mut transposed),
                reference::penalty_score(&matrix),
                "v{} mask {mask}",
                v.number()
            );
        }
    }
}

#[test]
fn build_symbol_matches_reference_symbol() {
    let mut rng = Rng(0x1357_9bdf_2468_ace0);
    for v in versions() {
        for ec in [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H] {
            let blocks = ec_blocks(v, ec);
            let data = rng.bytes(blocks.total_data_codewords() as usize);
            let stream = crate::matrix::interleave(&data, blocks);

            // Reference: build eight candidates and keep the lowest score,
            // lowest mask id on a tie.
            let build_unmasked = || {
                let mut m = BitMatrix::new(v.size());
                reference::place_function_patterns(&mut m, v);
                reference::place_data(&mut m, v, &stream);
                m
            };
            let (_, expected) = reference::choose_mask(build_unmasked, v, ec);
            assert_eq!(
                build_symbol(v, ec, &stream, None),
                expected,
                "v{} auto",
                v.number()
            );

            for mask in 0..8u8 {
                let mut m = build_unmasked();
                reference::apply_mask(&mut m, v, mask);
                reference::place_format_info(&mut m, v, ec, mask);
                if v.number() >= 7 {
                    reference::place_version_info(&mut m, v);
                }
                assert_eq!(
                    build_symbol(v, ec, &stream, Some(mask)),
                    m,
                    "v{} mask {mask}",
                    v.number()
                );
            }
        }
    }
}
