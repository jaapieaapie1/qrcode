//! BCH encoding/decoding for the format and version information areas.

use crate::EcLevel;

/// Generator polynomial for the 15-bit format info: `x^10 + x^8 + x^5 + x^4 +
/// x^2 + x + 1`.
const FORMAT_GEN: u32 = 0x537;
/// XOR mask applied to the 15-bit format information (ISO 8.9).
const FORMAT_MASK: u16 = 0x5412;

/// Generator polynomial for the 18-bit version info: `x^12 + x^11 + x^10 + x^9
/// + x^8 + x^5 + x^2 + 1`.
const VERSION_GEN: u32 = 0x1F25;

/// Divides `value` by `gen` (degree `gen_deg`) over GF(2), returning the
/// `gen_deg`-bit remainder. `data_bits` is the number of information bits that
/// sit above the remainder.
fn bch_remainder(value: u32, generator: u32, gen_deg: u32, data_bits: u32) -> u32 {
    let mut v = value;
    let mut i = data_bits;
    while i > 0 {
        i -= 1;
        if v & (1 << (gen_deg + i)) != 0 {
            v ^= generator << i;
        }
    }
    v & ((1 << gen_deg) - 1)
}

/// Encodes the 15-bit format information for an EC level and mask pattern
/// (`0..=7`), including the constant XOR mask (ISO Table 25).
pub fn format_info_encode(ec: EcLevel, mask: u8) -> u16 {
    let data = ((ec.format_bits() as u32) << 3) | ((mask & 0x7) as u32);
    let rem = bch_remainder(data << 10, FORMAT_GEN, 10, 5);
    (((data << 10) | rem) as u16) ^ FORMAT_MASK
}

/// Decodes 15 bits of format information, correcting up to 3 bit errors.
///
/// Returns the recovered `(EcLevel, mask)` or `None` if no candidate is within
/// Hamming distance 3.
pub fn format_info_decode(raw: u16) -> Option<(EcLevel, u8)> {
    let raw = raw & 0x7FFF;
    let mut best: Option<(EcLevel, u8)> = None;
    let mut best_dist = 4u32;
    for &ec in &EcLevel::ALL {
        for mask in 0..8u8 {
            let cand = format_info_encode(ec, mask);
            let dist = (cand ^ raw).count_ones();
            if dist < best_dist {
                best_dist = dist;
                best = Some((ec, mask));
            }
        }
    }
    best
}

/// Encodes the 18-bit version information for versions `7..=40`.
///
/// Returns `None` for versions that carry no version information (`< 7`) or are
/// out of range (`> 40`).
pub fn version_info_encode(version: u8) -> Option<u32> {
    if !(7..=40).contains(&version) {
        return None;
    }
    let data = version as u32;
    let rem = bch_remainder(data << 12, VERSION_GEN, 12, 6);
    Some((data << 12) | rem)
}

/// Decodes 18 bits of version information, correcting up to 3 bit errors.
///
/// Returns the recovered version (`7..=40`) or `None` if no candidate is within
/// Hamming distance 3.
pub fn version_info_decode(raw: u32) -> Option<u8> {
    let raw = raw & 0x3FFFF;
    let mut best: Option<u8> = None;
    let mut best_dist = 4u32;
    for version in 7..=40u8 {
        let cand = version_info_encode(version).unwrap();
        let dist = (cand ^ raw).count_ones();
        if dist < best_dist {
            best_dist = dist;
            best = Some(version);
        }
    }
    best
}
