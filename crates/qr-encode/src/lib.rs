//! `qr-encode` — turn a byte payload into a finished QR-symbol [`BitMatrix`].
//!
//! The encoder performs the full ISO/IEC 18004 data path: mode selection,
//! bit-stream assembly (mode indicator, character count, payload, terminator
//! and pad codewords), smallest-fitting version selection, Reed-Solomon block
//! splitting and interleaving (via [`qr_core`]), zigzag module placement,
//! evaluation of all eight data masks against the ISO penalty rules and
//! placement of the format (and, for versions >= 7, version) information.
//!
//! # Coordinates
//!
//! Output matrices use `(x, y)` = (column, row) with the origin at the
//! top-left, matching [`qr_core::BitMatrix`].
//!
//! # Scope (v1)
//!
//! Single-mode segmentation only (numeric / alphanumeric / byte auto-selected
//! for the whole payload). Mixed-mode segmentation, Kanji and ECI are not
//! implemented — see the crate Known Gaps.
//!
//! # Features
//!
//! `micro` (default) adds the Micro QR encoder in the `micro` module:
//! `encode_micro`, `MicroOptions` and `MicroEncodeError`, parallel to
//! [`encode`], [`Options`] and [`EncodeError`]. Opt out with
//! `--no-default-features`; the QR path is unaffected either way. The names are
//! written unlinked here because they do not exist when the feature is off.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(test)]
extern crate std;

mod bits;
mod matrix;
mod segment;

#[cfg(feature = "micro")]
pub mod micro;

#[cfg(feature = "micro")]
pub use micro::{MicroEncodeError, MicroOptions, encode_micro};

use alloc::vec::Vec;
use bits::BitBuffer;
use qr_core::{BitMatrix, EcLevel, Mode, Version, ec_blocks};

/// Errors returned by [`encode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// The payload does not fit in the requested (or any) version at the given
    /// EC level.
    DataTooLarge,
    /// A caller-supplied [`Mode`] cannot represent the payload, or is an
    /// unsupported mode (Kanji/ECI).
    ModeMismatch,
    /// A caller-supplied version is out of the `1..=40` range.
    InvalidVersion,
    /// A caller-supplied mask id is outside `0..=7`.
    InvalidMask,
}

impl core::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            EncodeError::DataTooLarge => "payload does not fit the requested version",
            EncodeError::ModeMismatch => "mode cannot represent the payload",
            EncodeError::InvalidVersion => "version out of range (1..=40)",
            EncodeError::InvalidMask => "mask id out of range (0..=7)",
        };
        f.write_str(msg)
    }
}

impl core::error::Error for EncodeError {}

/// Options controlling how a payload is encoded.
///
/// Leave optional fields as `None` to let the encoder choose automatically.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Error-correction level. Required.
    pub ec: EcLevel,
    /// Force a specific version; otherwise the smallest fitting version is used.
    pub version: Option<Version>,
    /// Force a specific data mask (`0..=7`); otherwise the lowest-penalty mask
    /// is chosen.
    pub mask: Option<u8>,
    /// Force a specific encoding mode; otherwise the most compact supported mode
    /// is selected from the payload.
    pub mode: Option<Mode>,
}

impl Options {
    /// Options that auto-select version, mask and mode for the given EC level.
    pub fn new(ec: EcLevel) -> Self {
        Options {
            ec,
            version: None,
            mask: None,
            mode: None,
        }
    }
}

/// Data-codeword capacity (in bits) of `version` at `ec`.
fn data_capacity_bits(version: Version, ec: EcLevel) -> usize {
    ec_blocks(version, ec).total_data_codewords() as usize * 8
}

/// Chooses the encoding mode, validating any caller-forced choice.
fn resolve_mode(data: &[u8], forced: Option<Mode>) -> Result<Mode, EncodeError> {
    match forced {
        Some(m) => {
            if segment::mode_supports(m, data) {
                Ok(m)
            } else {
                Err(EncodeError::ModeMismatch)
            }
        }
        None => Ok(segment::select_mode(data)),
    }
}

/// Chooses the smallest version that fits, or validates a caller-forced one.
fn resolve_version(
    data: &[u8],
    mode: Mode,
    ec: EcLevel,
    forced: Option<Version>,
) -> Result<Version, EncodeError> {
    match forced {
        Some(v) => {
            let bits = segment::total_bits(mode, data, v).ok_or(EncodeError::ModeMismatch)?;
            if bits <= data_capacity_bits(v, ec) {
                Ok(v)
            } else {
                Err(EncodeError::DataTooLarge)
            }
        }
        None => {
            for n in 1..=40u8 {
                let v = Version::new(n).map_err(|_| EncodeError::InvalidVersion)?;
                let bits = segment::total_bits(mode, data, v).ok_or(EncodeError::ModeMismatch)?;
                if bits <= data_capacity_bits(v, ec) {
                    return Ok(v);
                }
            }
            Err(EncodeError::DataTooLarge)
        }
    }
}

/// Assembles the padded data-codeword stream (before block splitting).
fn build_data_codewords(data: &[u8], mode: Mode, version: Version, ec: EcLevel) -> Vec<u8> {
    let capacity_bits = data_capacity_bits(version, ec);
    let mut buf = BitBuffer::new();

    segment::write_mode_indicator(&mut buf, mode);
    segment::write_char_count(&mut buf, mode, data.len(), version);
    segment::write_payload(&mut buf, mode, data);

    // Terminator: up to four zero bits, not exceeding capacity.
    let terminator = core::cmp::min(4, capacity_bits.saturating_sub(buf.len()));
    for _ in 0..terminator {
        buf.push_bit(false);
    }
    // Pad to a byte boundary.
    while !buf.len().is_multiple_of(8) {
        buf.push_bit(false);
    }

    let mut codewords = buf.into_bytes();
    let capacity_bytes = capacity_bits / 8;
    let pad = [0xECu8, 0x11u8];
    let mut i = 0;
    while codewords.len() < capacity_bytes {
        codewords.push(pad[i % 2]);
        i += 1;
    }
    codewords
}

/// Encodes `data` into a finished QR-symbol matrix.
///
/// The returned [`BitMatrix`] contains the function patterns, masked data,
/// format information and (for versions >= 7) version information, ready to be
/// rendered.
///
/// # Errors
///
/// Returns [`EncodeError`] when the payload does not fit, or when a
/// caller-forced mode, version or mask is invalid.
pub fn encode(data: &[u8], options: Options) -> Result<BitMatrix, EncodeError> {
    if let Some(m) = options.mask
        && m > 7
    {
        return Err(EncodeError::InvalidMask);
    }

    let mode = resolve_mode(data, options.mode)?;
    let version = resolve_version(data, mode, options.ec, options.version)?;
    let ec = options.ec;

    let data_codewords = build_data_codewords(data, mode, version, ec);
    let blocks = ec_blocks(version, ec);
    let interleaved = matrix::interleave(&data_codewords, blocks);

    // Factory producing a fresh function-patterned, data-placed (unmasked)
    // matrix. Used once per candidate mask during selection.
    let build_unmasked = || {
        let mut m = BitMatrix::new(version.size());
        matrix::place_function_patterns(&mut m, version);
        matrix::place_data(&mut m, version, &interleaved);
        m
    };

    let final_matrix = match options.mask {
        Some(mask) => {
            let mut m = build_unmasked();
            matrix::apply_mask(&mut m, version, mask);
            matrix::place_format_info(&mut m, version, ec, mask);
            if version.number() >= 7 {
                matrix::place_version_info(&mut m, version);
            }
            m
        }
        None => matrix::choose_mask(build_unmasked, version, ec).1,
    };

    Ok(final_matrix)
}

#[cfg(test)]
mod tests;
