//! Suite 3 — detection and decoding of geometrically abused renders.
//!
//! Real symbols are never axis-aligned, never pixel-perfect and never noise
//! free. Each case here starts from a genuine `qr-encode` + `qr-render` output,
//! puts it through a hand-rolled warp (rotation, downscale, projective keystone,
//! additive noise, or a combination), and requires `qr-detect` + `qr-decode` to
//! return the original bytes.
//!
//! The warps are written here rather than imported: `qr-detect`'s own helpers
//! live in its `tests/` directory and are not reachable from another crate.

mod common;

use common::{Gray, add_noise, pad, perspective, render_gray, rescale, rotate, rotate_quarters};
use qr_core::EcLevel;
use qr_encode::{Options, encode};

/// A payload with enough structure that a partial decode could not fake it.
const PAYLOAD: &[u8] = b"WARP-TEST-2026-ABCDEFGHIJKLMNOP-0123456789";

/// Encodes [`PAYLOAD`] and renders it to grayscale.
fn base(ec: EcLevel, version: Option<u8>, scale: u32, quiet: u32) -> Gray {
    let mut opts = Options::new(ec);
    opts.version = version.map(common::ver);
    let m = encode(PAYLOAD, opts).expect("encode warp fixture");
    render_gray(&m, scale, quiet)
}

/// Detects in `g` and asserts the decoded payload is [`PAYLOAD`].
fn assert_decodes(g: &Gray, label: &str) {
    let det = qr_detect::read_one(&g.to_image())
        .unwrap_or_else(|| panic!("{label}: detector found no symbol"));
    let decoded = det
        .decoded
        .unwrap_or_else(|| panic!("{label}: detected but decode failed"));
    assert_eq!(decoded.bytes, PAYLOAD, "{label}: payload mismatch");
}

// ---------------------------------------------------------------------------
// Rotation
// ---------------------------------------------------------------------------

/// 20 degrees off-axis, on a canvas padded enough that no corner is clipped.
/// This sits inside the detector's current rotation envelope (see
/// [`rotated_30_degrees`]).
#[test]
fn rotated_20_degrees() {
    let g = pad(&base(EcLevel::M, None, 8, 4), 130);
    let r = rotate(&g, 20f64.to_radians());
    assert_decodes(&r, "rotated 20deg");
}

/// 30 degrees off-axis.
///
/// IGNORED — genuine cross-crate defect **CI-1**, reported in
/// `handoff/REVIEW-REQUEST-integration.md`; not fixed here because `qr-detect`
/// is out of Step 3b's scope.
///
/// `qr_detect::grouping::score_triple` derives the symbol dimension as
/// `leg / module + 7`, where `leg` is the (rotation-invariant) finder
/// centre-to-centre distance but `module` comes from `Finder.module`, measured
/// as horizontal/vertical scanline run lengths. For a symbol rotated by `θ`
/// those runs are stretched by `1 / cos θ`, so the dimension estimate is short
/// by `(dimension - 7) * (1 - cos θ)` modules. `snap_dimension` tolerates about
/// two modules of error, so detection silently returns a *smaller* valid
/// version and the decode fails.
///
/// Measured breakdown angle (rendered at 10 px/module, `θ` folded into
/// `0..=45`): v1 34 deg, v2 30, v3 27, v4 25, v5 23, v7 20, v10 17, v15 14,
/// v20 12 — matching `(dimension - 7) * (1 - cos θ) ≈ 2` exactly.
#[test]
#[ignore = "CI-1: qr-detect dimension estimate is not rotation-corrected"]
fn rotated_30_degrees() {
    let g = pad(&base(EcLevel::M, None, 8, 4), 130);
    let r = rotate(&g, std::f64::consts::FRAC_PI_6);
    assert_decodes(&r, "rotated 30deg");
}

/// A single rotated symbol must yield exactly one candidate.
///
/// IGNORED — genuine cross-crate defect **CI-2**, reported in
/// `handoff/REVIEW-REQUEST-integration.md`. Under rotation
/// `qr_detect::grouping::group` can emit a second, phantom candidate built from
/// the same three finder patterns but snapped to a smaller dimension, so
/// `detect()` returns a non-decodable extra `Detection`. `read_one` is immune
/// (it prefers a detection that decoded), but anything counting detections —
/// including `qr_video::Stats::detections` — over-counts.
///
/// Repro: v7-M, 10 px/module, 200 px pad, 20 deg -> `sizes = [45, 21]`;
/// 22 deg -> `sizes = [41, 29]`.
#[test]
#[ignore = "CI-2: qr-detect emits phantom extra candidates under rotation"]
fn single_rotated_symbol_yields_single_candidate() {
    let mut opts = Options::new(EcLevel::M);
    opts.version = Some(common::ver(7));
    let m = encode(b"WARP", opts).expect("encode v7 fixture");
    let g = pad(&render_gray(&m, 10, 5), 200);
    let r = rotate(&g, 20f64.to_radians());
    assert_eq!(
        qr_detect::detect(&r.to_image()).len(),
        1,
        "one symbol in the image should give one candidate"
    );
}

/// Pins the *shape* of CI-1 so a future fix is detected: at 30 degrees the
/// detector still finds a symbol, but reports a smaller-than-actual dimension.
/// Delete this test together with the `#[ignore]` above once CI-1 is fixed.
#[test]
fn rotation_dimension_underestimate_is_still_present() {
    let mut opts = Options::new(EcLevel::M);
    opts.version = Some(common::ver(5));
    let m = encode(PAYLOAD, opts).expect("encode v5 fixture");
    let g = pad(&render_gray(&m, 10, 5), 200);
    let r = rotate(&g, std::f64::consts::FRAC_PI_6);

    let dets = qr_detect::detect(&r.to_image());
    assert_eq!(dets.len(), 1, "a symbol is still located at 30 degrees");
    assert!(
        dets[0].matrix.size() < m.size(),
        "CI-1 appears fixed: sampled dimension {} now matches the true {} — \
         drop this test and un-ignore rotated_30_degrees",
        dets[0].matrix.size(),
        m.size()
    );
}

/// An exact quarter turn: no resampling, so this isolates the detector's
/// orientation logic from any interpolation blur.
#[test]
fn rotated_90_degrees() {
    let g = base(EcLevel::M, None, 8, 4);
    let r = rotate_quarters(&g, 1);
    assert_decodes(&r, "rotated 90deg");
}

/// Three quarter turns — the other handedness of the same test.
#[test]
fn rotated_270_degrees() {
    let g = base(EcLevel::M, None, 8, 4);
    let r = rotate_quarters(&g, 3);
    assert_decodes(&r, "rotated 270deg");
}

/// Upside down. The three finder patterns still fix the orientation, so the
/// sampled grid must come back the right way up.
#[test]
fn rotated_180_degrees() {
    let g = base(EcLevel::M, None, 8, 4);
    let r = rotate_quarters(&g, 2);
    assert_decodes(&r, "rotated 180deg");
}

// ---------------------------------------------------------------------------
// Scale
// ---------------------------------------------------------------------------

/// Roughly 2.5 pixels per module: rendered at 10 px/module and bilinearly
/// resampled to a quarter size, so module edges land off the pixel grid.
#[test]
fn downscaled_to_two_and_a_half_pixels_per_module() {
    let g = base(EcLevel::M, None, 10, 5);
    let small = rescale(&g, 0.25);
    assert_decodes(&small, "2.5 px/module");
}

/// The same downscale at high EC, where the symbol is larger and the modules
/// therefore even less forgiving.
#[test]
fn downscaled_high_ec() {
    let g = base(EcLevel::H, None, 10, 5);
    let small = rescale(&g, 0.25);
    assert_decodes(&small, "2.5 px/module, EC H");
}

/// A non-integer downscale that does not land on a clean ratio.
#[test]
fn downscaled_non_integer_ratio() {
    let g = base(EcLevel::M, None, 9, 5);
    let small = rescale(&g, 0.31);
    assert_decodes(&small, "~2.8 px/module");
}

// ---------------------------------------------------------------------------
// Perspective
// ---------------------------------------------------------------------------

/// Mild keystone — the top edge pushed in, as if the camera looked down at the
/// code.
#[test]
fn perspective_keystone() {
    let g = base(EcLevel::M, None, 9, 5);
    let (w, h) = (g.w as f64, g.h as f64);
    let corners = [
        (0.11 * w, 0.05 * h),
        (0.89 * w, 0.02 * h),
        (0.97 * w, 0.95 * h),
        (0.03 * w, 0.91 * h),
    ];
    let p = perspective(&g, corners, g.w, g.h);
    assert_decodes(&p, "keystone");
}

/// Keystone on the other axis, and on a version with an alignment pattern grid
/// so the sampler's alignment refinement is in play.
#[test]
fn perspective_keystone_v7() {
    let g = base(EcLevel::L, Some(7), 9, 5);
    let (w, h) = (g.w as f64, g.h as f64);
    let corners = [
        (0.04 * w, 0.10 * h),
        (0.96 * w, 0.03 * h),
        (0.92 * w, 0.90 * h),
        (0.07 * w, 0.97 * h),
    ];
    let p = perspective(&g, corners, g.w, g.h);
    assert_decodes(&p, "keystone v7");
}

// ---------------------------------------------------------------------------
// Noise
// ---------------------------------------------------------------------------

/// Bell-shaped additive noise at an amplitude that visibly degrades the image
/// but stays below the binarisation threshold margin.
#[test]
fn noisy_render() {
    let mut g = base(EcLevel::M, None, 8, 4);
    add_noise(&mut g, 45, 0xA5A5_1234);
    assert_decodes(&g, "noise a=45");
}

/// Heavier noise, relying on Reed-Solomon to mop up the modules that flip.
#[test]
fn heavy_noise_with_rs_recovery() {
    let mut g = base(EcLevel::H, None, 8, 4);
    add_noise(&mut g, 70, 0x1357_9BDF);
    assert_decodes(&g, "noise a=70, EC H");
}

// ---------------------------------------------------------------------------
// Combinations
// ---------------------------------------------------------------------------

/// Rotation, downscale and noise together — the closest thing here to a phone
/// camera pointed at a sticker.
#[test]
fn rotated_downscaled_and_noisy() {
    let g = pad(&base(EcLevel::Q, None, 12, 5), 90);
    let r = rotate(&g, 0.30); // ~17 degrees, inside the CI-1 envelope
    let mut s = rescale(&r, 0.35); // ~4.2 px/module
    add_noise(&mut s, 30, 0x0BAD_F00D);
    assert_decodes(&s, "rotate + downscale + noise");
}

/// Perspective plus noise on a symbol with alignment patterns.
#[test]
fn keystone_and_noise() {
    let g = base(EcLevel::Q, Some(5), 10, 5);
    let (w, h) = (g.w as f64, g.h as f64);
    let corners = [
        (0.08 * w, 0.04 * h),
        (0.93 * w, 0.06 * h),
        (0.96 * w, 0.93 * h),
        (0.04 * w, 0.96 * h),
    ];
    let mut p = perspective(&g, corners, g.w, g.h);
    add_noise(&mut p, 35, 0xFACE_B00C);
    assert_decodes(&p, "keystone + noise");
}
