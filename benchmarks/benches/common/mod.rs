//! Fixture builders shared by the bench targets.
//!
//! Every fixture is generated in-process from `qr-encode` + `qr-render`, so a
//! run depends on nothing but the workspace itself: no image files, no network,
//! no clock. Construction always happens outside the timed loop.
//!
//! Each bench target compiles this module separately, so items only one target
//! uses look dead to the others — hence the blanket allow.

#![allow(dead_code)]

use image::RgbaImage;
use qr_core::{BitMatrix, EcLevel, Version, ec_blocks};
use qr_encode::Options;

/// 720p, the frame size every detector bench runs at.
pub const FRAME_W: u32 = 1280;
pub const FRAME_H: u32 = 720;

/// Luma value of the synthetic background (paper white).
const BG: u8 = 255;

/// Rec. 601 luma, matching what a camera pipeline would hand us.
#[must_use]
pub fn to_luma(img: &RgbaImage) -> Vec<u8> {
    img.pixels()
        .map(|p| {
            let [r, g, b, _] = p.0;
            let y = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
            y.round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

/// Rasterises `matrix` at `scale` px/module with a 4-module quiet zone and
/// returns `(luma, width, height)`.
#[must_use]
pub fn render_luma(matrix: &BitMatrix, scale: u32) -> (Vec<u8>, u32, u32) {
    let opts = qr_render::Options::default().scale(scale).quiet_zone(4);
    let img = qr_render::render_image(matrix, &opts);
    let (w, h) = (img.width(), img.height());
    (to_luma(&img), w, h)
}

/// A blank 720p frame: flat background, nothing to find.
#[must_use]
pub fn blank_frame() -> Vec<u8> {
    vec![BG; (FRAME_W * FRAME_H) as usize]
}

/// A 720p frame with no symbol in it, but plenty for the detector to chew on:
/// a smooth diagonal gradient overlaid with deterministic rectangular clutter.
///
/// This is the miss-path fixture. It has no finder pattern anywhere, so both
/// the primary and the retry binarization pass run to completion and find
/// nothing — the detector's genuine worst case.
#[must_use]
pub fn cluttered_frame() -> Vec<u8> {
    let mut buf = vec![BG; (FRAME_W * FRAME_H) as usize];
    for y in 0..FRAME_H {
        for x in 0..FRAME_W {
            // Gradient in [96, 255]: enough local contrast to keep the adaptive
            // threshold working rather than short-circuiting on a flat tile.
            let g = 96 + ((x + y) * 159 / (FRAME_W + FRAME_H)) as u8;
            buf[(y * FRAME_W + x) as usize] = g;
        }
    }
    // Code-free clutter: bars of alternating tone, none of them 1:1:3:1:1.
    for (i, &(bx, by, bw, bh)) in [
        (100u32, 80u32, 220u32, 40u32),
        (420, 200, 60, 300),
        (700, 90, 380, 25),
        (180, 480, 500, 70),
        (980, 400, 120, 260),
    ]
    .iter()
    .enumerate()
    {
        let tone = if i % 2 == 0 { 30 } else { 200 };
        for y in by..(by + bh).min(FRAME_H) {
            for x in bx..(bx + bw).min(FRAME_W) {
                buf[(y * FRAME_W + x) as usize] = tone;
            }
        }
    }
    buf
}

/// Blits a `tw x th` tile onto a `FRAME_W x FRAME_H` canvas at `(ox, oy)`.
///
/// Panics if the tile would run off the canvas — a silently clipped fixture
/// would be a silently different benchmark.
pub fn blit(canvas: &mut [u8], tile: &[u8], tw: u32, th: u32, ox: u32, oy: u32) {
    assert!(
        ox + tw <= FRAME_W && oy + th <= FRAME_H,
        "tile {tw}x{th} at ({ox},{oy}) does not fit in {FRAME_W}x{FRAME_H}"
    );
    for y in 0..th {
        let dst = ((oy + y) * FRAME_W + ox) as usize;
        let src = (y * tw) as usize;
        canvas[dst..dst + tw as usize].copy_from_slice(&tile[src..src + tw as usize]);
    }
}

/// A 720p frame carrying one rendered symbol, centred.
#[must_use]
pub fn frame_with(matrix: &BitMatrix, scale: u32) -> Vec<u8> {
    let (tile, tw, th) = render_luma(matrix, scale);
    let mut canvas = blank_frame();
    blit(
        &mut canvas,
        &tile,
        tw,
        th,
        (FRAME_W - tw) / 2,
        (FRAME_H - th) / 2,
    );
    canvas
}

/// A 720p frame carrying one rendered symbol at a chosen top-left offset.
#[must_use]
pub fn frame_with_at(matrix: &BitMatrix, scale: u32, ox: u32, oy: u32) -> Vec<u8> {
    let (tile, tw, th) = render_luma(matrix, scale);
    let mut canvas = blank_frame();
    blit(&mut canvas, &tile, tw, th, ox, oy);
    canvas
}

/// Rotates a 720p frame about its centre by `degrees`, bilinearly, filling
/// anything that maps outside the source with background.
///
/// Deterministic: no randomness, no clock, and the same input always yields the
/// same bytes.
#[must_use]
pub fn rotate(src: &[u8], degrees: f64) -> Vec<u8> {
    assert_eq!(src.len(), (FRAME_W * FRAME_H) as usize);
    let (cx, cy) = (f64::from(FRAME_W) / 2.0, f64::from(FRAME_H) / 2.0);
    let theta = degrees.to_radians();
    // Inverse map: rotating the destination back into source space.
    let (sin, cos) = ((-theta).sin(), (-theta).cos());

    let mut out = vec![BG; src.len()];
    for y in 0..FRAME_H {
        for x in 0..FRAME_W {
            let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
            let sx = dx * cos - dy * sin + cx;
            let sy = dx * sin + dy * cos + cy;
            if sx < 0.0 || sy < 0.0 {
                continue;
            }
            let (x0, y0) = (sx.floor() as u32, sy.floor() as u32);
            if x0 + 1 >= FRAME_W || y0 + 1 >= FRAME_H {
                continue;
            }
            let (fx, fy) = (sx - f64::from(x0), sy - f64::from(y0));
            let at = |px: u32, py: u32| f64::from(src[(py * FRAME_W + px) as usize]);
            let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
            let bot = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
            out[(y * FRAME_W + x) as usize] = (top * (1.0 - fy) + bot * fy).round() as u8;
        }
    }
    out
}

/// Deterministic pseudo-random bytes (xorshift-ish), for payloads that must not
/// collapse into alphanumeric mode.
#[must_use]
pub fn pseudo_bytes(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            // Keep out of the alphanumeric set so byte mode is genuinely forced.
            0x80 | (state >> 8) as u8 & 0x7f
        })
        .collect()
}

/// Encodes `data`, asserting success — fixtures that fail to build are a bug in
/// the bench, not a result.
#[must_use]
pub fn must_encode(data: &[u8], options: Options) -> BitMatrix {
    qr_encode::encode(data, options).expect("bench fixture must encode")
}

/// The longest byte-mode payload `version` holds at `ec`: the data capacity in
/// bits, less the 4-bit mode indicator and the character-count field.
#[must_use]
pub fn max_byte_len(version: Version, ec: EcLevel) -> usize {
    let capacity_bits = usize::from(ec_blocks(version, ec).total_data_codewords()) * 8;
    let count_bits = if version.number() <= 9 { 8 } else { 16 };
    (capacity_bits - 4 - count_bits) / 8
}

/// A symbol filled to `version`'s byte capacity at `ec`, with the payload that
/// produced it.
///
/// Filling the version exactly is also what pins it: one byte more than the
/// previous version holds cannot land anywhere smaller. The returned size is
/// asserted, so a capacity-table change fails the bench rather than silently
/// moving it to a different version.
#[must_use]
pub fn qr_fixture(version: u8, ec: EcLevel, seed: u32) -> (BitMatrix, Vec<u8>) {
    let v = Version::new(version).expect("valid version");
    let payload = pseudo_bytes(max_byte_len(v, ec), seed);
    let matrix = must_encode(&payload, Options::new(ec));
    assert_eq!(
        matrix.size(),
        v.size(),
        "fixture must land on version {version}"
    );
    (matrix, payload)
}
