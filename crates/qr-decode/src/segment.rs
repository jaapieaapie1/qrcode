//! Parsing the decoded data codewords into their payload bytes.
//!
//! Walks the segment structure (ISO/IEC 18004 §8.4): a 4-bit mode indicator
//! followed by a mode-specific character-count indicator and data, repeated
//! until a terminator or the padding is reached.

use alloc::vec::Vec;

use qr_core::{Mode, Version};

use crate::bits::BitReader;

/// The 45-character alphanumeric set (ISO Table 5), indexed by value.
const ALNUM: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

/// Parses the concatenated data codewords into the payload byte stream.
///
/// Numeric and alphanumeric segments are expanded to their ASCII characters,
/// byte segments are copied verbatim, and Kanji segments are emitted as their
/// two Shift-JIS bytes. ECI designators and FNC1/structured-append markers are
/// parsed for length but do not themselves add payload bytes. Returns `None`
/// on a malformed bit stream.
pub(crate) fn parse_segments(data: &[u8], version: Version) -> Option<Vec<u8>> {
    let mut reader = BitReader::new(data);
    let mut out = Vec::new();

    loop {
        // Fewer than 4 bits left means we are in the trailing padding.
        if reader.remaining() < 4 {
            break;
        }
        let mode = reader.read(4)?;
        match mode {
            // Terminator.
            0b0000 => break,
            // Numeric.
            0b0001 => {
                let cc = Mode::Numeric.char_count_bits(version)? as u32;
                let mut count = reader.read(cc)? as usize;
                while count >= 3 {
                    let v = reader.read(10)?;
                    if v > 999 {
                        return None;
                    }
                    out.push(b'0' + (v / 100) as u8);
                    out.push(b'0' + ((v / 10) % 10) as u8);
                    out.push(b'0' + (v % 10) as u8);
                    count -= 3;
                }
                if count == 2 {
                    let v = reader.read(7)?;
                    if v > 99 {
                        return None;
                    }
                    out.push(b'0' + (v / 10) as u8);
                    out.push(b'0' + (v % 10) as u8);
                } else if count == 1 {
                    let v = reader.read(4)?;
                    if v > 9 {
                        return None;
                    }
                    out.push(b'0' + v as u8);
                }
            }
            // Alphanumeric.
            0b0010 => {
                let cc = Mode::Alphanumeric.char_count_bits(version)? as u32;
                let mut count = reader.read(cc)? as usize;
                while count >= 2 {
                    let v = reader.read(11)? as usize;
                    if v >= 45 * 45 {
                        return None;
                    }
                    out.push(ALNUM[v / 45]);
                    out.push(ALNUM[v % 45]);
                    count -= 2;
                }
                if count == 1 {
                    let v = reader.read(6)? as usize;
                    if v >= 45 {
                        return None;
                    }
                    out.push(ALNUM[v]);
                }
            }
            // Byte.
            0b0100 => {
                let cc = Mode::Byte.char_count_bits(version)? as u32;
                let count = reader.read(cc)? as usize;
                for _ in 0..count {
                    out.push(reader.read(8)? as u8);
                }
            }
            // Kanji (Shift-JIS double byte).
            0b1000 => {
                let cc = Mode::Kanji.char_count_bits(version)? as u32;
                let count = reader.read(cc)? as usize;
                for _ in 0..count {
                    let v = reader.read(13)?;
                    let mut sjis = ((v / 0xC0) << 8) | (v % 0xC0);
                    sjis += if sjis < 0x1F00 { 0x8140 } else { 0xC140 };
                    out.push((sjis >> 8) as u8);
                    out.push((sjis & 0xFF) as u8);
                }
            }
            // ECI designator: 1, 2 or 3 bytes selected by the leading bits.
            0b0111 => {
                let b0 = reader.read(8)?;
                if b0 & 0x80 == 0 {
                    // 1-byte designator, value already consumed.
                } else if b0 & 0xC0 == 0x80 {
                    reader.read(8)?;
                } else if b0 & 0xE0 == 0xC0 {
                    reader.read(16)?;
                } else {
                    return None;
                }
            }
            // FNC1 in first position: no payload.
            0b0101 => {}
            // FNC1 in second position: 8-bit application indicator.
            0b1001 => {
                reader.read(8)?;
            }
            // Structured append: 4+4+8 header, tolerated and skipped.
            0b0011 => {
                reader.read(16)?;
            }
            _ => return None,
        }
    }

    Some(out)
}
