//! `qr-detect` — locate and sample QR symbols from a grayscale image.
//!
//! The pipeline is fully hand-rolled (the `image` crate is used only for its
//! buffer types):
//!
//! 1. [`binarize`] — adaptive-window threshold with an Otsu fallback.
//! 2. [`finder`] — `1:1:3:1:1` finder-pattern scan with vertical cross-check
//!    and centre clustering.
//! 3. [`grouping`] — triple selection (right angle + size similarity),
//!    orientation and dimension estimation. Multiple codes per image supported.
//! 4. [`sampler`] — alignment-pattern refinement (versions >= 2), grid ->
//!    image homography, and module sampling into a [`BitMatrix`].
//!
//! # Coordinates
//!
//! All coordinates are `(x, y)` = (column, row) with the origin top-left and
//! `y` increasing downward, matching image row order.
//!
//! # Micro QR
//!
//! With the `micro` cargo feature (enabled by default), single-finder Micro QR
//! symbols (ISO/IEC 18004 M1-M4) are detected too, through a parallel API:
//! [`MicroDetection`], [`detect_micro`] and [`detect_all`]. Micro symbols are
//! anchored on finder clusters that no QR triple claimed; the existing
//! [`Detection`] type and functions are unaffected.
//!
//! # Decoding
//!
//! With the `decode` cargo feature (enabled by default), each detection is
//! also run through `qr_decode::decode`. Disable with `--no-default-features`
//! for a detection-only build. With `micro` and `decode` both on, micro symbols
//! go through `qr_decode::micro::decode_micro` the same way.

mod binarize;
mod finder;
mod geometry;
mod grouping;
#[cfg(feature = "micro")]
mod micro;
mod sampler;

pub use geometry::{PerspectiveTransform, Point};

use binarize::Binary;
use finder::Finder;
use grouping::QrCandidate;
use image::GrayImage;
use qr_core::BitMatrix;

/// One detected QR symbol.
pub struct Detection {
    /// Symbol corners in image space, ordered top-left, top-right,
    /// bottom-right, bottom-left. `(x, y)` = (column, row).
    pub corners: [Point; 4],
    /// The sampled module grid (raw modules; not yet unmasked).
    pub matrix: BitMatrix,
    /// The decoded payload, if the `decode` feature is enabled and decoding
    /// succeeded.
    #[cfg(feature = "decode")]
    pub decoded: Option<qr_decode::Decoded>,
}

/// Detects every QR symbol in a grayscale image.
///
/// Returns one [`Detection`] per located symbol (possibly empty).
pub fn detect(img: &GrayImage) -> Vec<Detection> {
    detect_luma(img.as_raw(), img.width(), img.height())
}

/// Detects every QR symbol in a raw luma buffer.
///
/// `luma` must be exactly `width * height` bytes, row-major. Returns an empty
/// vector on dimension mismatch or when nothing is found.
pub fn detect_luma(luma: &[u8], width: u32, height: u32) -> Vec<Detection> {
    let Some((bin, _, candidates, _)) = scan(luma, width, height) else {
        return Vec::new();
    };
    collect_qr(&bin, &candidates)
}

/// What [`scan`] produces: the binarized image, every located finder, the QR
/// candidates they grouped into, and a flag per finder saying whether a triple
/// claimed it.
type Scan = (Binary, Vec<Finder>, Vec<QrCandidate>, Vec<bool>);

/// Shared front of the pipeline: binarize, locate finder patterns, group them
/// into QR candidates (see [`grouping::group`]).
fn scan(luma: &[u8], width: u32, height: u32) -> Option<Scan> {
    let bin = binarize::binarize(luma, width, height)?;
    let finders = finder::find_finders(&bin);
    let (candidates, used) = grouping::group(&finders);
    Some((bin, finders, candidates, used))
}

fn collect_qr(bin: &Binary, candidates: &[QrCandidate]) -> Vec<Detection> {
    let mut out = Vec::new();
    for cand in candidates {
        if let Some(sampled) = sampler::sample(bin, cand) {
            out.push(build_detection(sampled.corners, sampled.matrix));
        }
    }
    out
}

/// Detects and returns the single best (first) symbol, or `None`.
pub fn read_one(img: &GrayImage) -> Option<Detection> {
    read_one_luma(img.as_raw(), img.width(), img.height())
}

/// [`read_one`] over a raw luma buffer.
pub fn read_one_luma(luma: &[u8], width: u32, height: u32) -> Option<Detection> {
    let mut all = detect_luma(luma, width, height);
    if all.is_empty() {
        return None;
    }
    // Prefer a detection that actually decoded, when decoding is enabled.
    #[cfg(feature = "decode")]
    {
        if let Some(pos) = all.iter().position(|d| d.decoded.is_some()) {
            return Some(all.swap_remove(pos));
        }
    }
    Some(all.swap_remove(0))
}

#[cfg(feature = "decode")]
fn build_detection(corners: [Point; 4], matrix: BitMatrix) -> Detection {
    let decoded = qr_decode::decode(&matrix).ok();
    Detection {
        corners,
        matrix,
        decoded,
    }
}

#[cfg(not(feature = "decode"))]
fn build_detection(corners: [Point; 4], matrix: BitMatrix) -> Detection {
    Detection { corners, matrix }
}

// ---------------------------------------------------------------------------
// Micro QR
// ---------------------------------------------------------------------------

/// One detected Micro QR symbol (ISO/IEC 18004 M1-M4).
///
/// The micro counterpart of [`Detection`]; the two are deliberately separate
/// types, because the two symbologies share no version, capacity or format
/// information.
#[cfg(feature = "micro")]
pub struct MicroDetection {
    /// Symbol corners in image space, ordered top-left, top-right,
    /// bottom-right, bottom-left, where "top-left" is the finder's corner.
    /// `(x, y)` = (column, row).
    pub corners: [Point; 4],
    /// The sampled module grid (raw modules; not yet unmasked). Its size is one
    /// of 11, 13, 15, 17.
    pub matrix: BitMatrix,
    /// The decoded payload, if the `decode` feature is enabled and decoding
    /// succeeded.
    #[cfg(feature = "decode")]
    pub decoded: Option<qr_decode::micro::DecodedMicro>,
}

/// Detects every Micro QR symbol in a grayscale image.
///
/// Micro symbols are anchored on finder patterns that no QR triple claimed, so
/// a QR symbol in the same image never yields phantom micro detections.
#[cfg(feature = "micro")]
pub fn detect_micro(img: &GrayImage) -> Vec<MicroDetection> {
    detect_micro_luma(img.as_raw(), img.width(), img.height())
}

/// [`detect_micro`] over a raw luma buffer (`width * height` bytes, row-major).
#[cfg(feature = "micro")]
pub fn detect_micro_luma(luma: &[u8], width: u32, height: u32) -> Vec<MicroDetection> {
    let Some((bin, finders, _, used)) = scan(luma, width, height) else {
        return Vec::new();
    };
    collect_micro(&bin, &finders, &used)
}

/// Detects QR and Micro QR symbols in one pass, returning `(qr, micro)`.
///
/// Equivalent to calling [`detect`] and [`detect_micro`], but binarizes and
/// scans for finder patterns only once.
#[cfg(feature = "micro")]
pub fn detect_all(img: &GrayImage) -> (Vec<Detection>, Vec<MicroDetection>) {
    detect_all_luma(img.as_raw(), img.width(), img.height())
}

/// [`detect_all`] over a raw luma buffer (`width * height` bytes, row-major).
#[cfg(feature = "micro")]
pub fn detect_all_luma(
    luma: &[u8],
    width: u32,
    height: u32,
) -> (Vec<Detection>, Vec<MicroDetection>) {
    let Some((bin, finders, candidates, used)) = scan(luma, width, height) else {
        return (Vec::new(), Vec::new());
    };
    (
        collect_qr(&bin, &candidates),
        collect_micro(&bin, &finders, &used),
    )
}

#[cfg(feature = "micro")]
fn collect_micro(bin: &Binary, finders: &[Finder], used: &[bool]) -> Vec<MicroDetection> {
    let mut out = Vec::new();
    for (i, f) in finders.iter().enumerate() {
        if used[i] {
            continue;
        }
        if let Some(sampled) = micro::detect_from_finder(bin, f) {
            out.push(build_micro_detection(sampled.corners, sampled.matrix));
        }
    }
    out
}

#[cfg(all(feature = "micro", feature = "decode"))]
fn build_micro_detection(corners: [Point; 4], matrix: BitMatrix) -> MicroDetection {
    let decoded = qr_decode::micro::decode_micro(&matrix).ok();
    MicroDetection {
        corners,
        matrix,
        decoded,
    }
}

#[cfg(all(feature = "micro", not(feature = "decode")))]
fn build_micro_detection(corners: [Point; 4], matrix: BitMatrix) -> MicroDetection {
    MicroDetection { corners, matrix }
}
