//! Encoder tests, including the ISO/IEC 18004 Annex I reference symbol.

use crate::{Options, encode, matrix};
use alloc::vec;
use alloc::vec::Vec;
use qr_core::{
    BitMatrix, EcLevel, Mode, Version, bch::format_info_decode, ec_blocks, is_function_module,
    reed_solomon::rs_correct,
};

/// The ISO Annex I data codewords for "01234567" at version 1, level M.
const ANNEX_I_DATA: [u8; 16] = [
    0x10, 0x20, 0x0C, 0x56, 0x61, 0x80, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11,
];
/// The matching 10 Reed-Solomon EC codewords (ISO Annex I).
const ANNEX_I_EC: [u8; 10] = [0xA5, 0x24, 0xD4, 0xC1, 0xED, 0x36, 0xC7, 0x87, 0x2C, 0x55];

// ---- bit-stream / codeword assembly --------------------------------------

#[test]
fn annex_i_data_codewords() {
    let v = Version::new(1).unwrap();
    let cw = crate::build_data_codewords(b"01234567", Mode::Numeric, v, EcLevel::M);
    assert_eq!(cw, ANNEX_I_DATA);
}

#[test]
fn annex_i_interleave_produces_reference_ec() {
    let v = Version::new(1).unwrap();
    let blocks = ec_blocks(v, EcLevel::M);
    let cw = crate::build_data_codewords(b"01234567", Mode::Numeric, v, EcLevel::M);
    let inter = matrix::interleave(&cw, blocks);
    // v1-M is a single block: 16 data followed by 10 EC.
    assert_eq!(inter.len(), 26);
    assert_eq!(&inter[..16], &ANNEX_I_DATA);
    assert_eq!(&inter[16..], &ANNEX_I_EC);
}

// ---- full-symbol round trip ----------------------------------------------

/// Reads the first copy of the 15-bit format information from a symbol.
fn read_format_bits(m: &BitMatrix) -> u16 {
    let get = |x: u32, y: u32| if m.get(x, y) { 1u16 } else { 0 };
    let mut bits = 0u16;
    for i in 0..6u16 {
        bits |= get(8, i as u32) << i;
    }
    bits |= get(8, 7) << 6;
    bits |= get(8, 8) << 7;
    bits |= get(7, 8) << 8;
    for i in 9..15u16 {
        bits |= get(14 - i as u32, 8) << i;
    }
    bits
}

/// Extracts the interleaved codeword stream from an unmasked symbol using the
/// same zigzag order the encoder writes with.
fn extract_interleaved(m: &BitMatrix, version: Version, n_codewords: usize) -> Vec<u8> {
    let size = version.size();
    let total_bits = n_codewords * 8;
    let mut out = vec![0u8; n_codewords];
    let mut bit_index = 0usize;
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
                if is_function_module(version, x, y) {
                    continue;
                }
                if bit_index < total_bits {
                    if m.get(x, y) {
                        out[bit_index >> 3] |= 1 << (7 - (bit_index & 7));
                    }
                    bit_index += 1;
                }
            }
        }
        right -= 2;
    }
    out
}

#[test]
fn annex_i_full_symbol_roundtrip() {
    let m = encode(b"01234567", Options::new(EcLevel::M)).unwrap();
    let v = Version::new(1).unwrap();
    assert_eq!(m.size(), 21);

    // Format info must decode to level M and a valid mask via the core BCH path.
    let (ec, mask) = format_info_decode(read_format_bits(&m)).expect("format decodes");
    assert_eq!(ec, EcLevel::M);
    assert!(mask < 8);

    // Undo the mask (XOR is its own inverse), then read the codewords back.
    let mut unmasked = m.clone();
    matrix::apply_mask(&mut unmasked, v, mask);
    let inter = extract_interleaved(&unmasked, v, 26);
    assert_eq!(&inter[..16], &ANNEX_I_DATA);
    assert_eq!(&inter[16..], &ANNEX_I_EC);
}

#[test]
fn annex_i_selected_mask_is_two() {
    // ISO Annex I applies the 7.8.3 penalty and selects data mask 010 (2) for
    // this symbol; the auto-selector must reproduce that choice.
    let m = encode(b"01234567", Options::new(EcLevel::M)).unwrap();
    let (_, mask) = format_info_decode(read_format_bits(&m)).unwrap();
    assert_eq!(mask, 2);
}

// ---- forced options ------------------------------------------------------

#[test]
fn forced_mask_is_honoured_and_verifiable() {
    for forced in 0..8u8 {
        let opts = Options {
            ec: EcLevel::Q,
            version: None,
            mask: Some(forced),
            mode: None,
        };
        let m = encode(b"HELLO WORLD", opts).unwrap();
        let (ec, mask) = format_info_decode(read_format_bits(&m)).unwrap();
        assert_eq!(ec, EcLevel::Q);
        assert_eq!(mask, forced);
    }
}

#[test]
fn byte_mode_roundtrips_through_ec_correction() {
    // Encode arbitrary bytes, then confirm every RS block corrects cleanly
    // (zero errors) once the mask is removed and the stream deinterleaved.
    let payload: Vec<u8> = (0..40u8).collect();
    let opts = Options {
        ec: EcLevel::H,
        version: Some(Version::new(5).unwrap()),
        mask: None,
        mode: Some(Mode::Byte),
    };
    let m = encode(&payload, opts).unwrap();
    let v = Version::new(5).unwrap();
    let blocks = ec_blocks(v, EcLevel::H);

    let (_, mask) = format_info_decode(read_format_bits(&m)).unwrap();
    let mut unmasked = m.clone();
    matrix::apply_mask(&mut unmasked, v, mask);

    let total = blocks.total_codewords() as usize;
    let inter = extract_interleaved(&unmasked, v, total);

    // Deinterleave into per-block (data||ec) codewords, then RS-correct each.
    let mut deint = deinterleave(&inter, blocks);
    for blk in &mut deint {
        let corrected = rs_correct(blk, blocks.ec_per_block as usize).expect("block corrects");
        assert_eq!(corrected, 0, "a clean symbol needs no corrections");
    }
}

/// Inverse of `matrix::interleave`: splits the interleaved stream back into
/// per-block `data || ec` codeword vectors.
fn deinterleave(inter: &[u8], blocks: qr_core::EcBlocks) -> Vec<Vec<u8>> {
    let g1b = blocks.group1_blocks as usize;
    let g1d = blocks.group1_data as usize;
    let g2b = blocks.group2_blocks as usize;
    let g2d = blocks.group2_data as usize;
    let ec_len = blocks.ec_per_block as usize;
    let n_blocks = g1b + g2b;

    let block_data_len = |b: usize| if b < g1b { g1d } else { g2d };
    let mut out: Vec<Vec<u8>> = (0..n_blocks).map(|_| Vec::new()).collect();

    let max_data = g1d.max(g2d);
    let mut idx = 0usize;
    for i in 0..max_data {
        for (b, blk) in out.iter_mut().enumerate() {
            if i < block_data_len(b) {
                blk.push(inter[idx]);
                idx += 1;
            }
        }
    }
    // EC codewords occupy the tail, interleaved column-by-column across blocks.
    let ec_start = idx;
    for i in 0..ec_len {
        for (b, blk) in out.iter_mut().enumerate() {
            blk.push(inter[ec_start + i * n_blocks + b]);
        }
    }
    out
}

// ---- penalty rules -------------------------------------------------------

#[test]
fn n1_run_penalty_counts_long_runs() {
    // A run of exactly five same-colour modules scores 3; each extra adds 1.
    let five = [true, true, true, true, true, false];
    assert_eq!(matrix::line_run_penalty(|i| five[i as usize], 6), 3);
    let seven = [true; 7];
    assert_eq!(matrix::line_run_penalty(|i| seven[i as usize], 7), 5);
    let short = [true, true, false, true, true, false, true];
    assert_eq!(matrix::line_run_penalty(|i| short[i as usize], 7), 0);
}

#[test]
fn n3_finder_penalty_detects_pattern() {
    // A 1:1:3:1:1 pattern with a 4-wide light area scores 40 for its existence
    // (once, per ISO NOTE 3 — not per side).
    let line = [
        false, false, false, false, true, false, true, true, true, false, true,
    ];
    assert_eq!(
        matrix::finder_penalty(|i| line[i as usize], line.len() as u32),
        40
    );
    // A line with no finder-like pattern scores nothing.
    let plain = [false, true, false, true, false, true, false, true, false];
    assert_eq!(
        matrix::finder_penalty(|i| plain[i as usize], plain.len() as u32),
        0
    );
}

// ---- smoke across versions ----------------------------------------------

#[test]
fn smoke_encode_all_versions() {
    // A payload sized to land in each version, checking the symbol is
    // well-formed (correct size, valid format info) end to end.
    for n in 1..=40u8 {
        let v = Version::new(n).unwrap();
        let cap = ec_blocks(v, EcLevel::L).total_data_codewords() as usize;
        // Fill roughly to half capacity with byte data.
        let payload: Vec<u8> = (0..(cap / 2).max(1)).map(|i| (i % 251) as u8).collect();
        let opts = Options {
            ec: EcLevel::L,
            version: Some(v),
            mask: None,
            mode: Some(Mode::Byte),
        };
        let m = encode(&payload, opts).unwrap_or_else(|e| panic!("v{n}: {e}"));
        assert_eq!(m.size(), v.size());
        let (ec, _) = format_info_decode(read_format_bits(&m)).expect("format decodes");
        assert_eq!(ec, EcLevel::L);
    }
}

#[test]
fn too_large_payload_is_rejected() {
    let opts = Options {
        ec: EcLevel::H,
        version: Some(Version::new(1).unwrap()),
        mask: None,
        mode: Some(Mode::Byte),
    };
    let big = vec![0u8; 1000];
    assert!(encode(&big, opts).is_err());
}
