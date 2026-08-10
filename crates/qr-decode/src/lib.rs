//! `qr-decode` — turn a fully sampled [`BitMatrix`] back into its payload.
//!
//! The input is a clean module grid (as produced by a detector or an encoder),
//! not an image. The pipeline reads the format information, un-masks and
//! extracts the interleaved codewords, de-interleaves and Reed-Solomon corrects
//! each block, then parses the segment structure into bytes.
//!
//! Symbols that were mirrored (reflected across the main diagonal) are handled
//! transparently: a failed decode is retried on the transposed matrix.
//!
//! # Coordinates
//!
//! All coordinates are `(x, y)` = (column, row) with the origin at the
//! top-left, matching [`qr_core`].
//!
//! # Cargo features
//!
//! - `micro` (default): adds the [`micro`] module with [`decode_micro`] and
//!   [`DecodedMicro`], the Micro QR (M1-M4) counterparts of [`decode`] and
//!   [`Decoded`]. Opt out with `--no-default-features`; the full-QR path is
//!   unaffected either way.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(test)]
extern crate std;

mod bits;
mod extract;
mod segment;

#[cfg(feature = "micro")]
pub mod micro;

use alloc::string::String;
use alloc::vec::Vec;

use qr_core::{BitMatrix, EcLevel, Version};

use extract::{extract_data, read_format_info, read_version_info};
use segment::parse_segments;

#[cfg(feature = "micro")]
pub use micro::{DecodedMicro, decode_micro};

/// The result of decoding a QR symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The raw payload bytes (ASCII for numeric/alphanumeric, verbatim for
    /// byte mode, Shift-JIS for Kanji).
    pub bytes: Vec<u8>,
    /// The payload decoded as UTF-8, if it is valid UTF-8. This is best-effort
    /// and covers the common ASCII/UTF-8 (and ECI 26) cases.
    pub text: Option<String>,
    /// The symbol version (`1..=40`).
    pub version: Version,
    /// The error-correction level read from the format information.
    pub ec: EcLevel,
    /// The data-mask pattern (`0..=7`) read from the format information.
    pub mask: u8,
    /// Total number of codewords Reed-Solomon had to correct across all blocks.
    pub errors_corrected: usize,
}

/// Errors returned by [`decode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The matrix side length does not correspond to any version `1..=40`.
    InvalidSize,
    /// The format information could not be recovered from either copy.
    FormatInfo,
    /// The version information contradicts the version implied by the size.
    VersionInfo,
    /// A Reed-Solomon block had more errors than could be corrected.
    BlockCorrection,
    /// The corrected data did not parse as a valid segment stream.
    Malformed,
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            DecodeError::InvalidSize => "matrix size is not a valid QR version",
            DecodeError::FormatInfo => "could not decode format information",
            DecodeError::VersionInfo => "version information is inconsistent",
            DecodeError::BlockCorrection => "too many errors to correct a block",
            DecodeError::Malformed => "malformed data segment stream",
        };
        f.write_str(msg)
    }
}

impl core::error::Error for DecodeError {}

/// Decodes a QR symbol from its module grid.
///
/// `matrix` must be a square grid whose side length matches a version
/// (`21, 25, … 177`); a set module (`true`) is dark. If the straight decode
/// fails the matrix is transposed and retried, transparently handling mirrored
/// symbols.
pub fn decode(matrix: &BitMatrix) -> Result<Decoded, DecodeError> {
    match decode_oriented(matrix) {
        Ok(decoded) => Ok(decoded),
        Err(first) => {
            let mirrored = transpose(matrix);
            decode_oriented(&mirrored).map_err(|_| first)
        }
    }
}

/// Decodes without attempting the mirror retry.
fn decode_oriented(matrix: &BitMatrix) -> Result<Decoded, DecodeError> {
    let size = matrix.size();
    let version = version_from_size(size).ok_or(DecodeError::InvalidSize)?;

    let (ec, mask) = read_format_info(matrix, size).ok_or(DecodeError::FormatInfo)?;

    // For versions with version information, use it to reject a matrix whose
    // self-described version contradicts its geometry (a strong mis-read
    // signal). The size stays authoritative when the field is unreadable.
    if version.number() >= 7
        && let Some(vi) = read_version_info(matrix, size)
        && vi != version.number()
    {
        return Err(DecodeError::VersionInfo);
    }

    let (data, errors_corrected) = extract_data(matrix, version, ec, mask)?;
    let bytes = parse_segments(&data, version).ok_or(DecodeError::Malformed)?;
    let text = String::from_utf8(bytes.clone()).ok();

    Ok(Decoded {
        bytes,
        text,
        version,
        ec,
        mask,
        errors_corrected,
    })
}

/// Maps a matrix side length to its version, or `None` if it is not a valid
/// QR size. `size = 17 + 4 * version`.
fn version_from_size(size: u32) -> Option<Version> {
    if !(21..=177).contains(&size) || !(size - 17).is_multiple_of(4) {
        return None;
    }
    Version::new(((size - 17) / 4) as u8).ok()
}

/// Reflects a matrix across its main diagonal (`(x, y) -> (y, x)`).
fn transpose(m: &BitMatrix) -> BitMatrix {
    let size = m.size();
    let mut out = BitMatrix::new(size);
    for y in 0..size {
        for x in 0..size {
            if m.get(x, y) {
                out.set(y, x, true);
            }
        }
    }
    out
}
