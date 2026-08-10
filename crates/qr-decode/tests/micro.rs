//! End-to-end Micro QR decode tests.
//!
//! The whole file is gated on the `micro` feature so that
//! `--no-default-features` builds compile it out entirely.
//!
//! Ground truth comes from ISO/IEC 18004:2024 Figure I.4 — the printed M2-L
//! symbol encoding `"01234567"` — transcribed below. Everything else is built by
//! [`build_symbol`], a fixture encoder assembled from `qr-core` primitives
//! (this crate must not depend on `qr-encode`, which is being built
//! concurrently). [`fixture_builder_reproduces_figure_i4`] pins that builder
//! against Figure I.4, so the derived fixtures inherit the ISO ground truth.

#![cfg(feature = "micro")]

use qr_core::micro::{
    MicroEcLevel, MicroMode, MicroSymbolInfo, MicroVersion, micro_data_modules,
    micro_format_info_encode, micro_format_info_write, micro_mask_bit, micro_symbol_info,
};
use qr_core::reed_solomon::rs_encode;
use qr_core::{BitMatrix, EcLevel};

use qr_decode::{DecodeError, DecodedMicro, decode_micro};

// ----- ISO Figure I.4 -------------------------------------------------------

/// ISO/IEC 18004:2024 Figure I.4: the final M2-L symbol encoding `"01234567"`,
/// data mask 01. `#` is a dark module.
const FIGURE_I4: [&str; 13] = [
    "#######.#.#.#",
    "#.....#.###.#",
    "#.###.#..##.#",
    "#.###.#..####",
    "#.###.#.###..",
    "#.....#.#...#",
    "#######..####",
    ".........##..",
    "##.#....#...#",
    ".##.#.#.#.#.#",
    "###..#######.",
    "...#.#....##.",
    "###.#..##.###",
];

/// Turns an ASCII art grid into a [`BitMatrix`].
fn grid(rows: &[&str]) -> BitMatrix {
    let size = rows.len() as u32;
    let mut m = BitMatrix::new(size);
    for (y, row) in rows.iter().enumerate() {
        assert_eq!(row.len(), size as usize, "row {y} is not square");
        for (x, ch) in row.chars().enumerate() {
            m.set(x as u32, y as u32, ch == '#');
        }
    }
    m
}

// ----- fixture construction -------------------------------------------------

/// A most-significant-bit-first bit stream builder, the inverse of the
/// decoder's reader.
#[derive(Default)]
struct Bits(Vec<bool>);

impl Bits {
    fn push(&mut self, value: u32, n: u32) {
        for i in (0..n).rev() {
            self.0.push((value >> i) & 1 == 1);
        }
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

/// Builds the data bit stream for a single-segment micro symbol: the
/// version-sized mode indicator, the character count and the encoded data
/// (ISO Tables 2 and 3).
fn segment_bits(info: &MicroSymbolInfo, mode: MicroMode, data: &[u8]) -> Bits {
    let version = info.version;
    let mut bits = Bits::default();

    let (indicator, indicator_bits) = mode.indicator(version).expect("mode available");
    bits.push(indicator as u32, indicator_bits as u32);

    let count = match mode {
        MicroMode::Kanji => data.len() / 2,
        _ => data.len(),
    };
    bits.push(count as u32, mode.char_count_bits(version).unwrap() as u32);

    match mode {
        MicroMode::Numeric => {
            for chunk in data.chunks(3) {
                let value: u32 = chunk
                    .iter()
                    .fold(0, |acc, d| acc * 10 + u32::from(*d - b'0'));
                bits.push(value, [0, 4, 7, 10][chunk.len()]);
            }
        }
        MicroMode::Alphanumeric => {
            const ALNUM: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";
            let value = |c: u8| ALNUM.iter().position(|&a| a == c).expect("alnum") as u32;
            for chunk in data.chunks(2) {
                if chunk.len() == 2 {
                    bits.push(value(chunk[0]) * 45 + value(chunk[1]), 11);
                } else {
                    bits.push(value(chunk[0]), 6);
                }
            }
        }
        MicroMode::Byte => {
            for &b in data {
                bits.push(u32::from(b), 8);
            }
        }
        MicroMode::Kanji => {
            for pair in data.chunks(2) {
                let sjis = (u32::from(pair[0]) << 8) | u32::from(pair[1]);
                let shifted = if sjis < 0xE040 {
                    sjis - 0x8140
                } else {
                    sjis - 0xC140
                };
                bits.push((shifted >> 8) * 0xC0 + (shifted & 0xFF), 13);
            }
        }
    }

    bits
}

/// Terminates and pads a data bit stream, then packs it into data codewords.
///
/// Follows ISO 7.4.10 and 7.4.11: the terminator is `2n + 1` zero bits,
/// abbreviated to whatever room is left; zero bits pad to the next codeword
/// boundary; remaining codewords are `0xEC` and `0x11` alternately, and the
/// 4-bit final codeword of M1/M3 pads as `0000`.
fn pad_to_codewords(mut bits: Bits, info: &MicroSymbolInfo) -> Vec<u8> {
    let capacity = info.data_bits as usize;
    assert!(bits.len() <= capacity, "fixture overflows the symbol");

    let terminator = (info.version.terminator_bits() as usize).min(capacity - bits.len());
    for _ in 0..terminator {
        bits.0.push(false);
    }

    // Codeword boundaries: 8 bits each, except a trailing 4-bit half codeword.
    let widths: Vec<usize> = (0..info.data_codewords as usize)
        .map(|i| {
            if info.has_half_final_data_codeword() && i + 1 == info.data_codewords as usize {
                4
            } else {
                8
            }
        })
        .collect();

    // Zero-pad to the boundary of the codeword the stream currently ends in.
    let mut boundary = 0usize;
    for width in &widths {
        boundary += width;
        if boundary >= bits.len() {
            break;
        }
    }
    while bits.len() < boundary {
        bits.0.push(false);
    }

    let mut codewords = Vec::new();
    let mut pos = 0usize;
    let mut pad = 0usize;
    for &width in &widths {
        if pos >= bits.len() {
            // Pad codeword: 0xEC / 0x11 alternately, 0b0000 for a half codeword.
            codewords.push(if width == 4 {
                0
            } else {
                pad += 1;
                if pad % 2 == 1 { 0xEC } else { 0x11 }
            });
        } else {
            let mut value = 0u8;
            for bit in 0..width {
                if bits.0.get(pos + bit).copied().unwrap_or(false) {
                    value |= 1 << (width - 1 - bit);
                }
            }
            codewords.push(value);
        }
        pos += width;
    }

    assert_eq!(codewords.len(), info.data_codewords as usize);
    codewords
}

/// Draws the micro function patterns: the single finder with its separator and
/// the two edge-to-edge timing patterns (ISO 5.3.3.2, 5.3.5).
fn draw_function_patterns(version: MicroVersion, m: &mut BitMatrix) {
    let size = version.size();
    for y in 0..7u32 {
        for x in 0..7u32 {
            let ring = x == 0 || x == 6 || y == 0 || y == 6;
            let core = (2..=4).contains(&x) && (2..=4).contains(&y);
            m.set(x, y, ring || core);
        }
    }
    for x in 8..size {
        m.set(x, 0, x.is_multiple_of(2));
    }
    for y in 8..size {
        m.set(0, y, y.is_multiple_of(2));
    }
}

/// Builds a complete micro symbol from data codewords: Reed-Solomon, placement
/// through the shared core iterator, masking, function patterns and format
/// information.
fn build_symbol(info: &MicroSymbolInfo, mask: u8, data: &[u8]) -> BitMatrix {
    let version = info.version;
    let ec = rs_encode(data, info.ec_codewords as usize);

    let mut block = data.to_vec();
    block.extend_from_slice(&ec);
    assert_eq!(block.len(), info.total_codewords as usize);

    let mut m = BitMatrix::new(version.size());
    draw_function_patterns(version, &mut m);

    let width = |i: usize| {
        if info.has_half_final_data_codeword() && i + 1 == info.data_codewords as usize {
            4u32
        } else {
            8
        }
    };

    let mut index = 0usize;
    let mut bit = 0u32;
    let mut placed = 0u32;
    for (x, y) in micro_data_modules(version) {
        let w = width(index);
        let value = (block[index] >> (w - 1 - bit)) & 1 == 1;
        m.set(x, y, value ^ micro_mask_bit(mask, x, y));
        bit += 1;
        if bit == w {
            index += 1;
            bit = 0;
        }
        placed += 1;
    }
    assert_eq!(placed, version.data_modules());
    assert_eq!(index, block.len());

    let format = micro_format_info_encode(version, info.ec, mask).expect("valid symbol");
    micro_format_info_write(&mut m, format);
    m
}

/// One-shot fixture: encode `data` in `mode` into an `(version, ec)` symbol
/// masked with `mask`.
fn fixture(
    version: MicroVersion,
    ec: MicroEcLevel,
    mask: u8,
    mode: MicroMode,
    data: &[u8],
) -> BitMatrix {
    let info = micro_symbol_info(version, ec).expect("valid symbol");
    let codewords = pad_to_codewords(segment_bits(&info, mode, data), &info);
    build_symbol(&info, mask, &codewords)
}

/// Flips the module carrying bit `n` of the placement order, which corrupts the
/// codeword that bit belongs to.
fn flip_data_bit(m: &mut BitMatrix, version: MicroVersion, n: usize) {
    let (x, y) = micro_data_modules(version).nth(n).expect("bit in range");
    let value = m.get(x, y);
    m.set(x, y, !value);
}

fn assert_payload(decoded: &DecodedMicro, text: &str) {
    assert_eq!(decoded.bytes, text.as_bytes());
    assert_eq!(decoded.text.as_deref(), Some(text));
}

// ----- ISO ground truth -----------------------------------------------------

#[test]
fn iso_figure_i4_decodes() {
    let decoded = decode_micro(&grid(&FIGURE_I4)).expect("Figure I.4 decodes");
    assert_payload(&decoded, "01234567");
    assert_eq!(decoded.version, MicroVersion::M2);
    assert_eq!(decoded.ec, MicroEcLevel::L);
    assert_eq!(decoded.mask, 1);
    assert_eq!(decoded.errors_corrected, 0);
}

/// The fixture builder must reproduce Figure I.4 exactly, module for module —
/// that is what licenses every other fixture in this file.
#[test]
fn fixture_builder_reproduces_figure_i4() {
    let built = fixture(
        MicroVersion::M2,
        MicroEcLevel::L,
        1,
        MicroMode::Numeric,
        b"01234567",
    );
    let reference = grid(&FIGURE_I4);
    for y in 0..reference.size() {
        for x in 0..reference.size() {
            assert_eq!(
                built.get(x, y),
                reference.get(x, y),
                "module ({x}, {y}) differs from Figure I.4"
            );
        }
    }
}

/// The Annex I.3 data codewords, independently confirming the packing in
/// `pad_to_codewords`.
#[test]
fn annex_i3_data_codewords() {
    let info = micro_symbol_info(MicroVersion::M2, MicroEcLevel::L).unwrap();
    let codewords = pad_to_codewords(segment_bits(&info, MicroMode::Numeric, b"01234567"), &info);
    assert_eq!(codewords, [0x40, 0x18, 0xAC, 0xC3, 0x00]);
}

// ----- happy path, every version and level ----------------------------------

#[test]
fn m1_numeric_roundtrip() {
    // M1 holds 20 data bits: a 3-bit count plus five digits (10 + 7 bits).
    let m = fixture(
        MicroVersion::M1,
        MicroEcLevel::Detection,
        0,
        MicroMode::Numeric,
        b"12345",
    );
    let decoded = decode_micro(&m).expect("M1 decodes");
    assert_payload(&decoded, "12345");
    assert_eq!(decoded.version, MicroVersion::M1);
    assert_eq!(decoded.ec, MicroEcLevel::Detection);
    assert_eq!(decoded.errors_corrected, 0);
}

#[test]
fn m2_alphanumeric_roundtrip() {
    let m = fixture(
        MicroVersion::M2,
        MicroEcLevel::M,
        3,
        MicroMode::Alphanumeric,
        b"AB1",
    );
    let decoded = decode_micro(&m).expect("M2-M decodes");
    assert_payload(&decoded, "AB1");
    assert_eq!(decoded.ec, MicroEcLevel::M);
    assert_eq!(decoded.mask, 3);
}

/// M3 has a 4-bit final data codeword; byte mode exercises it with a payload
/// that does not land on a codeword boundary.
#[test]
fn m3_byte_roundtrip_across_the_half_codeword() {
    let m = fixture(
        MicroVersion::M3,
        MicroEcLevel::L,
        2,
        MicroMode::Byte,
        b"Hello!!",
    );
    let decoded = decode_micro(&m).expect("M3-L decodes");
    assert_payload(&decoded, "Hello!!");
    assert_eq!(decoded.version, MicroVersion::M3);
}

/// M3-M is the tighter half-codeword case: 9 data codewords, 68 data bits.
#[test]
fn m3_numeric_fills_the_half_codeword_exactly() {
    let info = micro_symbol_info(MicroVersion::M3, MicroEcLevel::M).unwrap();
    assert_eq!(info.data_bits, 68);
    assert!(info.has_half_final_data_codeword());

    let m = fixture(
        MicroVersion::M3,
        MicroEcLevel::M,
        1,
        MicroMode::Numeric,
        b"012345678901234567",
    );
    let decoded = decode_micro(&m).expect("M3-M decodes");
    assert_payload(&decoded, "012345678901234567");
}

#[test]
fn m4_every_level_and_mask() {
    for &ec in MicroVersion::M4.ec_levels() {
        for mask in 0..4u8 {
            let m = fixture(MicroVersion::M4, ec, mask, MicroMode::Byte, b"micro");
            let decoded = decode_micro(&m).unwrap_or_else(|e| panic!("M4 {ec:?} mask {mask}: {e}"));
            assert_payload(&decoded, "micro");
            assert_eq!(decoded.version, MicroVersion::M4);
            assert_eq!(decoded.ec, ec);
            assert_eq!(decoded.mask, mask);
        }
    }
}

#[test]
fn m4_kanji_roundtrip() {
    // Two Shift-JIS characters, one from each of the two SJIS ranges.
    let sjis: [u8; 4] = [0x93, 0x5F, 0xE4, 0xAA];
    let m = fixture(
        MicroVersion::M4,
        MicroEcLevel::L,
        0,
        MicroMode::Kanji,
        &sjis,
    );
    let decoded = decode_micro(&m).expect("M4 kanji decodes");
    assert_eq!(decoded.bytes, sjis);
    assert_eq!(decoded.text, None, "Shift-JIS is not valid UTF-8 here");
}

/// Every version/level pair decodes, and the reported level maps back onto the
/// full-QR [`EcLevel`] for M2-M4.
#[test]
fn every_symbol_number_roundtrips() {
    for version in MicroVersion::ALL {
        for &ec in version.ec_levels() {
            let m = fixture(version, ec, 0, MicroMode::Numeric, b"123");
            let decoded =
                decode_micro(&m).unwrap_or_else(|e| panic!("{version:?} {ec:?} failed: {e}"));
            assert_payload(&decoded, "123");
            assert_eq!(decoded.version, version);
            assert_eq!(decoded.ec, ec);
            if version != MicroVersion::M1 {
                assert!(matches!(
                    ec.ec_level(),
                    Some(EcLevel::L | EcLevel::M | EcLevel::Q)
                ));
            }
        }
    }
}

// ----- error correction -----------------------------------------------------

/// M2-L corrects one codeword (ISO Table 9, `r = 1`).
#[test]
fn m2_corrects_one_codeword() {
    let mut m = grid(&FIGURE_I4);
    flip_data_bit(&mut m, MicroVersion::M2, 3);
    flip_data_bit(&mut m, MicroVersion::M2, 5);

    let decoded = decode_micro(&m).expect("one damaged codeword is recoverable");
    assert_payload(&decoded, "01234567");
    assert_eq!(decoded.errors_corrected, 1);
}

/// Two damaged codewords exceed M2-L's capacity of one. The three surplus error
/// correction codewords are misdecode protection (`p = 3`), so this must be
/// rejected rather than "corrected" by the generic `d / 2` bound.
#[test]
fn m2_rejects_two_damaged_codewords() {
    let mut m = grid(&FIGURE_I4);
    flip_data_bit(&mut m, MicroVersion::M2, 3);
    flip_data_bit(&mut m, MicroVersion::M2, 11);

    assert_eq!(decode_micro(&m), Err(DecodeError::BlockCorrection));
}

/// M2-M has `r = 2`.
#[test]
fn m2_m_corrects_two_codewords() {
    let mut m = fixture(
        MicroVersion::M2,
        MicroEcLevel::M,
        0,
        MicroMode::Numeric,
        b"01234",
    );
    flip_data_bit(&mut m, MicroVersion::M2, 1);
    flip_data_bit(&mut m, MicroVersion::M2, 9);

    let decoded = decode_micro(&m).expect("two damaged codewords are recoverable at M2-M");
    assert_payload(&decoded, "01234");
    assert_eq!(decoded.errors_corrected, 2);
}

/// M3-L has `r = 2`; the damage here also lands in the half codeword region.
#[test]
fn m3_corrects_within_capacity() {
    let mut m = fixture(
        MicroVersion::M3,
        MicroEcLevel::L,
        0,
        MicroMode::Byte,
        b"Hello!!",
    );
    flip_data_bit(&mut m, MicroVersion::M3, 2);
    flip_data_bit(&mut m, MicroVersion::M3, 20);

    let decoded = decode_micro(&m).expect("M3-L corrects two codewords");
    assert_payload(&decoded, "Hello!!");
    assert_eq!(decoded.errors_corrected, 2);
}

/// M4-Q has `r = 7`, the deepest correction any micro symbol offers, and
/// `p = 0` so the whole `d / 2` bound is usable.
#[test]
fn m4_q_corrects_seven_codewords() {
    let info = micro_symbol_info(MicroVersion::M4, MicroEcLevel::Q).unwrap();
    assert_eq!(info.error_correction_capacity, 7);
    assert_eq!(info.misdecode_protection, 0);

    let mut m = fixture(MicroVersion::M4, MicroEcLevel::Q, 0, MicroMode::Byte, b"go");
    for n in (0..56).step_by(8) {
        flip_data_bit(&mut m, MicroVersion::M4, n);
    }
    let decoded = decode_micro(&m).expect("M4-Q corrects seven codewords");
    assert_payload(&decoded, "go");
    assert_eq!(decoded.errors_corrected, 7);
}

/// The misdecode-protection cap, tested where it actually bites: M4-L and M3-L
/// both publish `r` one below `d / 2`, so a block needing `d / 2` corrections is
/// mathematically solvable yet must be rejected (ISO 7.5.1, Formula 3).
#[test]
fn misdecode_protection_caps_correction_below_the_reed_solomon_bound() {
    for (version, ec, mode) in [
        (MicroVersion::M4, MicroEcLevel::L, MicroMode::Byte),
        (MicroVersion::M3, MicroEcLevel::L, MicroMode::Byte),
    ] {
        let info = micro_symbol_info(version, ec).unwrap();
        let r = info.error_correction_capacity as usize;
        assert!(info.misdecode_protection > 0);
        assert_eq!(r + 1, info.ec_codewords as usize / 2, "{version:?} {ec:?}");

        let build = || fixture(version, ec, 0, mode, b"go");

        let mut at_capacity = build();
        for n in (0..r * 8).step_by(8) {
            flip_data_bit(&mut at_capacity, version, n);
        }
        let decoded = decode_micro(&at_capacity)
            .unwrap_or_else(|e| panic!("{version:?} {ec:?} at capacity: {e}"));
        assert_payload(&decoded, "go");
        assert_eq!(decoded.errors_corrected, r);

        let mut beyond = build();
        for n in (0..(r + 1) * 8).step_by(8) {
            flip_data_bit(&mut beyond, version, n);
        }
        assert_eq!(
            decode_micro(&beyond),
            Err(DecodeError::BlockCorrection),
            "{version:?} {ec:?} accepted {} corrections past r = {r}",
            r + 1
        );
    }
}

// ----- M1: detection only ---------------------------------------------------

/// M1's two error correction codewords detect but never correct (ISO Table 9
/// gives `r = 0`). A single flipped module must therefore fail the symbol, not
/// be silently "repaired" into a different payload.
#[test]
fn m1_rejects_any_damage_without_correcting() {
    let clean = fixture(
        MicroVersion::M1,
        MicroEcLevel::Detection,
        0,
        MicroMode::Numeric,
        b"12345",
    );
    assert!(decode_micro(&clean).is_ok(), "the clean fixture decodes");

    // Every single-module flip in the data region must be detected.
    for n in 0..MicroVersion::M1.data_modules() as usize {
        let mut damaged = clean.clone();
        flip_data_bit(&mut damaged, MicroVersion::M1, n);
        assert_eq!(
            decode_micro(&damaged),
            Err(DecodeError::BlockCorrection),
            "M1 accepted a flip at placement bit {n}"
        );
    }
}

// ----- rejection of nonsense ------------------------------------------------

#[test]
fn wrong_sizes_are_rejected() {
    for size in [1u32, 10, 12, 14, 16, 18, 21, 25] {
        assert_eq!(
            decode_micro(&BitMatrix::new(size)),
            Err(DecodeError::InvalidSize),
            "size {size} was accepted"
        );
    }
}

/// A format information word whose symbol number names a different version than
/// the symbol size must be rejected: micro symbols carry a single format copy,
/// so this cross-check is the only structural guard before Reed-Solomon.
#[test]
fn symbol_number_must_agree_with_the_size() {
    let info = micro_symbol_info(MicroVersion::M2, MicroEcLevel::L).unwrap();
    let codewords = pad_to_codewords(segment_bits(&info, MicroMode::Numeric, b"01234567"), &info);
    let mut m = build_symbol(&info, 1, &codewords);

    // Overwrite the format information with M4-L (symbol number 4) in a 13 x 13
    // (M2) symbol.
    let wrong = micro_format_info_encode(MicroVersion::M4, MicroEcLevel::L, 1).unwrap();
    micro_format_info_write(&mut m, wrong);

    assert_eq!(decode_micro(&m), Err(DecodeError::VersionInfo));
}

/// The BCH(15,5) format code "corrects" a large fraction of arbitrary words, so
/// format decode success is never evidence of a symbol. Random grids must die at
/// the Reed-Solomon gate with a clean error and no panic.
#[test]
fn random_grids_are_rejected_cleanly() {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let mut format_survived = 0usize;
    for version in MicroVersion::ALL {
        let size = version.size();
        for _ in 0..300 {
            let mut m = BitMatrix::new(size);
            for y in 0..size {
                for x in 0..size {
                    m.set(x, y, next() & 1 == 1);
                }
            }
            match decode_micro(&m) {
                Err(DecodeError::FormatInfo) => {}
                Err(DecodeError::VersionInfo) | Err(DecodeError::BlockCorrection) => {
                    format_survived += 1;
                }
                Err(DecodeError::Malformed) => format_survived += 1,
                other => panic!("random {size}x{size} grid decoded: {other:?}"),
            }
        }
    }

    // The point of the test: plenty of random grids get past the format
    // information, and every one of them is still rejected downstream.
    assert!(
        format_survived > 100,
        "expected the BCH code to pass many random words through, saw {format_survived}"
    );
}

#[test]
fn blank_and_solid_grids_are_rejected() {
    for version in MicroVersion::ALL {
        let size = version.size();
        let blank = BitMatrix::new(size);
        assert!(decode_micro(&blank).is_err(), "blank {size} decoded");

        let mut solid = BitMatrix::new(size);
        for y in 0..size {
            for x in 0..size {
                solid.set(x, y, true);
            }
        }
        assert!(decode_micro(&solid).is_err(), "solid {size} decoded");
    }
}

// ----- mirroring ------------------------------------------------------------

/// Reflecting a symbol across its main diagonal is handled by the same
/// transpose-and-retry the full-QR path uses.
#[test]
fn mirrored_symbols_decode() {
    for version in MicroVersion::ALL {
        for &ec in version.ec_levels() {
            let m = fixture(version, ec, 1, MicroMode::Numeric, b"4567");
            let size = m.size();
            let mut mirrored = BitMatrix::new(size);
            for y in 0..size {
                for x in 0..size {
                    mirrored.set(y, x, m.get(x, y));
                }
            }

            let decoded = decode_micro(&mirrored)
                .unwrap_or_else(|e| panic!("mirrored {version:?} {ec:?}: {e}"));
            assert_payload(&decoded, "4567");
            assert_eq!(decoded.version, version);
            assert_eq!(decoded.ec, ec);
        }
    }
}

#[test]
fn mirrored_figure_i4_decodes() {
    let m = grid(&FIGURE_I4);
    let size = m.size();
    let mut mirrored = BitMatrix::new(size);
    for y in 0..size {
        for x in 0..size {
            mirrored.set(y, x, m.get(x, y));
        }
    }
    assert_payload(&decode_micro(&mirrored).expect("mirror retry"), "01234567");
}

// ----- bit-stream edge cases ------------------------------------------------

/// A payload that exactly fills the data capacity leaves no room for the
/// terminator; the parser must stop on the capacity limit instead of reading
/// into the error correction codewords.
#[test]
fn full_capacity_payload_needs_no_terminator() {
    let info = micro_symbol_info(MicroVersion::M1, MicroEcLevel::Detection).unwrap();
    let bits = segment_bits(&info, MicroMode::Numeric, b"12345");
    assert_eq!(bits.len(), info.data_bits as usize, "M1 is exactly full");

    let m = build_symbol(&info, 0, &pad_to_codewords(bits, &info));
    assert_payload(&decode_micro(&m).expect("decodes"), "12345");
}

/// A short payload leaves pad codewords (`0xEC` / `0x11`) after the terminator.
/// Those bytes look like plausible segment headers, so the terminator must stop
/// the parse before they are reached.
#[test]
fn padding_after_the_terminator_is_not_parsed() {
    let m = fixture(
        MicroVersion::M4,
        MicroEcLevel::L,
        0,
        MicroMode::Numeric,
        b"1",
    );
    assert_payload(&decode_micro(&m).expect("decodes"), "1");
}

/// An empty payload is a bare terminator.
#[test]
fn empty_payload_decodes_to_nothing() {
    let info = micro_symbol_info(MicroVersion::M3, MicroEcLevel::L).unwrap();
    let m = build_symbol(&info, 0, &pad_to_codewords(Bits::default(), &info));
    let decoded = decode_micro(&m).expect("decodes");
    assert!(decoded.bytes.is_empty());
    assert_eq!(decoded.text.as_deref(), Some(""));
}

/// A mode indicator that no micro version defines (M4 values 4-7) is malformed.
#[test]
fn unknown_mode_indicator_is_malformed() {
    let info = micro_symbol_info(MicroVersion::M4, MicroEcLevel::L).unwrap();
    let mut bits = Bits::default();
    bits.push(0b111, 3);
    bits.push(0, 5);
    let m = build_symbol(&info, 0, &pad_to_codewords(bits, &info));
    assert_eq!(decode_micro(&m), Err(DecodeError::Malformed));
}

/// A character count larger than the remaining capacity is malformed rather
/// than a panic or a truncated payload.
#[test]
fn overlong_character_count_is_malformed() {
    let info = micro_symbol_info(MicroVersion::M4, MicroEcLevel::L).unwrap();
    let mut bits = Bits::default();
    bits.push(0b010, 3); // byte mode
    bits.push(31, 5); // 31 bytes, far beyond the 15 the symbol holds
    let m = build_symbol(&info, 0, &pad_to_codewords(bits, &info));
    assert_eq!(decode_micro(&m), Err(DecodeError::Malformed));
}
