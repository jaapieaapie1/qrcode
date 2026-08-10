//! Data analysis and per-mode bit encoding.
//!
//! v1 uses single-mode selection: the whole payload is encoded in one mode,
//! chosen as the most compact mode that can represent every input byte
//! (numeric < alphanumeric < byte). Greedy mixed-mode segmentation is a
//! documented Known Gap.

use crate::bits::BitBuffer;
use qr_core::{Mode, Version};

/// Maps an ASCII byte to its alphanumeric value (ISO Table 5), or `None` if the
/// byte is not in the 45-character alphanumeric set.
fn alnum_value(b: u8) -> Option<u32> {
    let v = match b {
        b'0'..=b'9' => b - b'0',
        b'A'..=b'Z' => b - b'A' + 10,
        b' ' => 36,
        b'$' => 37,
        b'%' => 38,
        b'*' => 39,
        b'+' => 40,
        b'-' => 41,
        b'.' => 42,
        b'/' => 43,
        b':' => 44,
        _ => return None,
    };
    Some(v as u32)
}

fn is_numeric(b: u8) -> bool {
    b.is_ascii_digit()
}

fn is_alnum(b: u8) -> bool {
    alnum_value(b).is_some()
}

/// Selects the most compact single mode that can represent every byte in `data`.
///
/// Empty input encodes as [`Mode::Byte`].
pub fn select_mode(data: &[u8]) -> Mode {
    if !data.is_empty() && data.iter().all(|&b| is_numeric(b)) {
        Mode::Numeric
    } else if !data.is_empty() && data.iter().all(|&b| is_alnum(b)) {
        Mode::Alphanumeric
    } else {
        Mode::Byte
    }
}

/// Returns `true` if every byte of `data` can be encoded in `mode`.
///
/// [`Mode::Kanji`] and [`Mode::Eci`] are not supported by this encoder.
pub fn mode_supports(mode: Mode, data: &[u8]) -> bool {
    match mode {
        Mode::Numeric => data.iter().all(|&b| is_numeric(b)),
        Mode::Alphanumeric => data.iter().all(|&b| is_alnum(b)),
        Mode::Byte => true,
        Mode::Kanji | Mode::Eci => false,
    }
}

/// Number of payload bits (excluding mode indicator and char-count indicator)
/// needed to encode `len` characters in `mode`.
pub fn payload_bits(mode: Mode, len: usize) -> usize {
    match mode {
        Mode::Numeric => {
            let groups = len / 3;
            let rem = len % 3;
            groups * 10
                + match rem {
                    1 => 4,
                    2 => 7,
                    _ => 0,
                }
        }
        Mode::Alphanumeric => (len / 2) * 11 + (len % 2) * 6,
        Mode::Byte => len * 8,
        Mode::Kanji | Mode::Eci => 0,
    }
}

/// Total bits (mode indicator + char count + payload) for `data` in `mode` at
/// `version`.
pub fn total_bits(mode: Mode, data: &[u8], version: Version) -> Option<usize> {
    let ccbits = mode.char_count_bits(version)? as usize;
    Some(4 + ccbits + payload_bits(mode, data.len()))
}

/// Appends the character-count indicator to `buf` at `version`.
pub fn write_char_count(buf: &mut BitBuffer, mode: Mode, len: usize, version: Version) {
    let ccbits = mode.char_count_bits(version).unwrap_or(0);
    buf.push_bits(len as u32, ccbits);
}

/// Appends the mode indicator followed by the encoded payload for `data`.
///
/// The caller is responsible for having written the char-count indicator with
/// [`write_char_count`] between the indicator and payload; this helper writes
/// only the 4-bit mode indicator plus the payload, so the ordering is:
/// `mode indicator`, then char count (caller), then payload.
pub fn write_mode_indicator(buf: &mut BitBuffer, mode: Mode) {
    buf.push_bits(mode.indicator() as u32, 4);
}

/// Appends the encoded payload bits for `data` in `mode`.
pub fn write_payload(buf: &mut BitBuffer, mode: Mode, data: &[u8]) {
    match mode {
        Mode::Numeric => {
            let mut chunks = data.chunks_exact(3);
            for c in &mut chunks {
                let v =
                    (c[0] - b'0') as u32 * 100 + (c[1] - b'0') as u32 * 10 + (c[2] - b'0') as u32;
                buf.push_bits(v, 10);
            }
            let rem = chunks.remainder();
            match rem.len() {
                1 => buf.push_bits((rem[0] - b'0') as u32, 4),
                2 => {
                    let v = (rem[0] - b'0') as u32 * 10 + (rem[1] - b'0') as u32;
                    buf.push_bits(v, 7);
                }
                _ => {}
            }
        }
        Mode::Alphanumeric => {
            let mut chunks = data.chunks_exact(2);
            for c in &mut chunks {
                let v = alnum_value(c[0]).unwrap_or(0) * 45 + alnum_value(c[1]).unwrap_or(0);
                buf.push_bits(v, 11);
            }
            let rem = chunks.remainder();
            if rem.len() == 1 {
                buf.push_bits(alnum_value(rem[0]).unwrap_or(0), 6);
            }
        }
        Mode::Byte => {
            for &b in data {
                buf.push_bits(b as u32, 8);
            }
        }
        Mode::Kanji | Mode::Eci => {}
    }
}
