//! Reading the format/version information and the interleaved codeword stream
//! out of a `BitMatrix`, then de-interleaving and Reed-Solomon correcting it.
//!
//! Coordinates are `(x, y)` = (column, row), origin top-left, matching
//! `qr-core`.

use alloc::vec;
use alloc::vec::Vec;

use qr_core::bch::{format_info_decode, version_info_decode};
use qr_core::reed_solomon::rs_correct;
use qr_core::{ec_blocks, is_function_module, mask_bit, BitMatrix, EcLevel, Version};

use crate::DecodeError;

/// Reads the 15-bit format information, correcting up to 3 bit errors.
///
/// Both copies (the L-shape around the top-left finder and the split strip by
/// the top-right/bottom-left finders) are tried; the first that decodes wins.
pub(crate) fn read_format_info(m: &BitMatrix, size: u32) -> Option<(EcLevel, u8)> {
    // Copy 1: down column 8, then left along row 8 (ISO Figure 25).
    let mut raw1 = 0u16;
    for i in 0..6u32 {
        if m.get(8, i) {
            raw1 |= 1 << i;
        }
    }
    if m.get(8, 7) {
        raw1 |= 1 << 6;
    }
    if m.get(8, 8) {
        raw1 |= 1 << 7;
    }
    if m.get(7, 8) {
        raw1 |= 1 << 8;
    }
    for i in 9..15u32 {
        if m.get(14 - i, 8) {
            raw1 |= 1 << i;
        }
    }

    // Copy 2: left-to-right along row 8 by the top-right finder, then down
    // column 8 by the bottom-left finder.
    let mut raw2 = 0u16;
    for i in 0..8u32 {
        if m.get(size - 1 - i, 8) {
            raw2 |= 1 << i;
        }
    }
    for i in 8..15u32 {
        if m.get(8, size - 15 + i) {
            raw2 |= 1 << i;
        }
    }

    format_info_decode(raw1).or_else(|| format_info_decode(raw2))
}

/// Reads the 18-bit version information (versions ≥ 7), correcting up to 3 bit
/// errors. Both copies are tried. Returns the decoded version number.
pub(crate) fn read_version_info(m: &BitMatrix, size: u32) -> Option<u8> {
    let mut raw1 = 0u32;
    let mut raw2 = 0u32;
    for i in 0..18u32 {
        let a = size - 11 + i % 3;
        let b = i / 3;
        // Top-right block.
        if m.get(a, b) {
            raw1 |= 1 << i;
        }
        // Bottom-left block (transposed placement).
        if m.get(b, a) {
            raw2 |= 1 << i;
        }
    }
    version_info_decode(raw1).or_else(|| version_info_decode(raw2))
}

/// Extracts the data codewords from a symbol: walks the zig-zag placement,
/// un-masks each module, de-interleaves the blocks and Reed-Solomon corrects
/// them.
///
/// Returns the concatenated corrected data codewords and the number of
/// codewords that had to be corrected.
pub(crate) fn extract_data(
    m: &BitMatrix,
    version: Version,
    ec: EcLevel,
    mask: u8,
) -> Result<(Vec<u8>, usize), DecodeError> {
    let stream = read_codeword_stream(m, version, mask);
    deinterleave_and_correct(&stream, version, ec)
}

/// Reads the interleaved codeword stream by walking the standard two-column
/// zig-zag from the bottom-right, skipping function modules and the vertical
/// timing column, un-masking each data module as it goes.
fn read_codeword_stream(m: &BitMatrix, version: Version, mask: u8) -> Vec<u8> {
    let size = m.size();
    let total_cw = version.total_codewords() as usize;
    let total_bits = total_cw * 8;
    let mut stream = vec![0u8; total_cw];
    let mut bit = 0usize;

    let mut right = size as i64 - 1;
    while right >= 1 {
        // Column 6 is the vertical timing pattern; the pair shifts one left.
        if right == 6 {
            right = 5;
        }
        for vert in 0..size {
            for j in 0..2i64 {
                let x = (right - j) as u32;
                let upward = (right + 1) & 2 == 0;
                let y = if upward { size - 1 - vert } else { vert };
                if !is_function_module(version, x, y) && bit < total_bits {
                    let module = m.get(x, y) ^ mask_bit(mask, x, y);
                    if module {
                        stream[bit >> 3] |= 1 << (7 - (bit & 7));
                    }
                    bit += 1;
                }
            }
        }
        right -= 2;
    }
    stream
}

/// Splits the interleaved `stream` back into Reed-Solomon blocks, corrects each
/// and concatenates the recovered data codewords in block order.
fn deinterleave_and_correct(
    stream: &[u8],
    version: Version,
    ec: EcLevel,
) -> Result<(Vec<u8>, usize), DecodeError> {
    let bl = ec_blocks(version, ec);
    let n_blocks = bl.total_blocks() as usize;
    let g1_blocks = bl.group1_blocks as usize;
    let g1_data = bl.group1_data as usize;
    let g2_data = bl.group2_data as usize;
    let ecc = bl.ec_per_block as usize;

    if stream.len() != version.total_codewords() as usize {
        return Err(DecodeError::BlockCorrection);
    }

    let data_len = |b: usize| if b < g1_blocks { g1_data } else { g2_data };
    let max_data = g1_data.max(g2_data);

    // Allocate one buffer per block sized data + EC.
    let mut blocks: Vec<Vec<u8>> = (0..n_blocks)
        .map(|b| vec![0u8; data_len(b) + ecc])
        .collect();

    let mut pos = 0usize;
    for i in 0..max_data {
        for (b, block) in blocks.iter_mut().enumerate() {
            if i < data_len(b) {
                block[i] = stream[pos];
                pos += 1;
            }
        }
    }
    for i in 0..ecc {
        for (b, block) in blocks.iter_mut().enumerate() {
            block[data_len(b) + i] = stream[pos];
            pos += 1;
        }
    }

    let mut data = Vec::with_capacity(bl.total_data_codewords() as usize);
    let mut errors_corrected = 0usize;
    for (b, block) in blocks.iter_mut().enumerate() {
        match rs_correct(block, ecc) {
            Ok(fixed) => {
                errors_corrected += fixed;
                data.extend_from_slice(&block[..data_len(b)]);
            }
            Err(_) => return Err(DecodeError::BlockCorrection),
        }
    }

    Ok((data, errors_corrected))
}
