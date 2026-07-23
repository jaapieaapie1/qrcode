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
//! # Decoding
//!
//! With the `decode` cargo feature (enabled by default), each detection is
//! also run through `qr_decode::decode`. Disable with `--no-default-features`
//! for a detection-only build.

mod binarize;
mod finder;
mod geometry;
mod grouping;
mod sampler;

pub use geometry::{PerspectiveTransform, Point};

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
    let Some(bin) = binarize::binarize(luma, width, height) else {
        return Vec::new();
    };
    let finders = finder::find_finders(&bin);
    let candidates = grouping::group(&finders);

    let mut out = Vec::new();
    for cand in &candidates {
        if let Some(sampled) = sampler::sample(&bin, cand) {
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
