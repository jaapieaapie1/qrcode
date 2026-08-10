//! End-to-end detection tests over synthetic fixtures.
//!
//! Everything here is generated in-test: we hand-draw a QR-shaped module grid
//! (three finder patterns, separators, timing, and — for v >= 2 — the
//! bottom-right alignment pattern, plus deterministic "data" modules), render it
//! to a grayscale image, optionally warp it (rotate / scale / perspective /
//! noise) with our own transform code, then run detection and check the sampled
//! `BitMatrix` matches the source module-for-module.
//!
//! No dependency on qr-encode / qr-render (both built concurrently); the
//! roundtrip through those lands in Step 3b.

use qr_core::BitMatrix;
use qr_detect::detect_luma;

// ---------------------------------------------------------------------------
// Synthetic QR module grid
// ---------------------------------------------------------------------------

fn dimension_for(version: u8) -> u32 {
    17 + 4 * version as u32
}

/// Draws a 7x7 finder pattern with its top-left module at `(ox, oy)`.
fn draw_finder(m: &mut BitMatrix, ox: u32, oy: u32) {
    for dy in 0..7 {
        for dx in 0..7 {
            let border = dx == 0 || dx == 6 || dy == 0 || dy == 6;
            let core = (2..=4).contains(&dx) && (2..=4).contains(&dy);
            m.set(ox + dx, oy + dy, border || core);
        }
    }
}

/// Draws a 5x5 alignment pattern centred at module `(cx, cy)`.
fn draw_alignment(m: &mut BitMatrix, cx: u32, cy: u32) {
    for dy in 0..5i32 {
        for dx in 0..5i32 {
            let ax = cx as i32 - 2 + dx;
            let ay = cy as i32 - 2 + dy;
            let ring = dx == 0 || dx == 4 || dy == 0 || dy == 4;
            let center = dx == 2 && dy == 2;
            m.set(ax as u32, ay as u32, ring || center);
        }
    }
}

/// Builds a synthetic QR module grid for the given version. Not a valid
/// payload — just correct function patterns plus deterministic data so the
/// geometry is unambiguous and reproducible.
fn synth_matrix(version: u8) -> BitMatrix {
    let dim = dimension_for(version);
    let mut m = BitMatrix::new(dim);

    // Deterministic pseudo-random "data" fill, faithful to a real (RS-coded)
    // data region — unlike a perfect checkerboard, which would mimic the
    // alignment pattern's 1:1:1:1:1 signature everywhere.
    for y in 0..dim {
        for x in 0..dim {
            let mut h = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)).wrapping_add(1);
            h ^= h >> 13;
            h = h.wrapping_mul(0x5bd1_e995);
            h ^= h >> 15;
            m.set(x, y, h & 1 == 0);
        }
    }

    // Finder patterns (overwrite whatever data was there).
    draw_finder(&mut m, 0, 0);
    draw_finder(&mut m, dim - 7, 0);
    draw_finder(&mut m, 0, dim - 7);

    // Separators: 1-module light border around each finder.
    for i in 0..8 {
        m.set(7, i, false);
        m.set(i, 7, false);
        m.set(dim - 8, i, false);
        m.set(dim - 1 - i, 7, false);
        m.set(7, dim - 1 - i, false);
        m.set(i, dim - 8, false);
    }

    // Timing patterns on row/col 6.
    for k in 8..dim - 8 {
        m.set(k, 6, k % 2 == 0);
        m.set(6, k, k % 2 == 0);
    }

    // Alignment pattern (bottom-right) for v >= 2.
    if version >= 2 {
        let last = dim - 7; // matches qr-core alignment table for the BR pattern
        draw_alignment(&mut m, last, last);
    }

    m
}

// ---------------------------------------------------------------------------
// Rendering + warps (hand-rolled)
// ---------------------------------------------------------------------------

struct Gray {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

impl Gray {
    fn new(w: u32, h: u32, fill: u8) -> Gray {
        Gray {
            w,
            h,
            px: vec![fill; (w * h) as usize],
        }
    }
    fn set(&mut self, x: u32, y: u32, v: u8) {
        self.px[(y * self.w + x) as usize] = v;
    }
    /// Bilinear sample; out-of-bounds returns `bg`.
    fn sample(&self, x: f64, y: f64, bg: u8) -> u8 {
        if x < 0.0 || y < 0.0 || x > (self.w - 1) as f64 || y > (self.h - 1) as f64 {
            return bg;
        }
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(self.w - 1);
        let y1 = (y0 + 1).min(self.h - 1);
        let fx = x - x0 as f64;
        let fy = y - y0 as f64;
        let p = |xx: u32, yy: u32| self.px[(yy * self.w + xx) as usize] as f64;
        let top = p(x0, y0) * (1.0 - fx) + p(x1, y0) * fx;
        let bot = p(x0, y1) * (1.0 - fx) + p(x1, y1) * fx;
        (top * (1.0 - fy) + bot * fy).round() as u8
    }
}

/// Renders a module grid to grayscale: `scale` px per module, `quiet` modules of
/// white border. Dark modules -> 0, light -> 255.
fn render(m: &BitMatrix, scale: u32, quiet: u32) -> Gray {
    let dim = m.size();
    let side = (dim + 2 * quiet) * scale;
    let mut g = Gray::new(side, side, 255);
    for y in 0..dim {
        for x in 0..dim {
            let v = if m.get(x, y) { 0 } else { 255 };
            for py in 0..scale {
                for px in 0..scale {
                    g.set((quiet + x) * scale + px, (quiet + y) * scale + py, v);
                }
            }
        }
    }
    g
}

/// Rotates an image by `angle` radians about its centre (inverse mapping).
fn rotate(src: &Gray, angle: f64) -> Gray {
    let cx = (src.w - 1) as f64 / 2.0;
    let cy = (src.h - 1) as f64 / 2.0;
    let (s, c) = angle.sin_cos();
    let mut out = Gray::new(src.w, src.h, 255);
    for y in 0..src.h {
        for x in 0..src.w {
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            // inverse rotation
            let sx = cx + dx * c + dy * s;
            let sy = cy - dx * s + dy * c;
            out.set(x, y, src.sample(sx, sy, 255));
        }
    }
    out
}

/// Widens `src` with `px` pixels of white on every side, so that a rotation
/// about the centre cannot push the symbol off the canvas.
fn pad(src: &Gray, px: u32) -> Gray {
    let mut out = Gray::new(src.w + 2 * px, src.h + 2 * px, 255);
    for y in 0..src.h {
        for x in 0..src.w {
            out.set(x + px, y + px, src.px[(y * src.w + x) as usize]);
        }
    }
    out
}

/// Applies a projective warp given by four destination corners of the source
/// rectangle `(0,0)-(w,h)`. Uses inverse mapping via a solved homography.
fn perspective(src: &Gray, dst_corners: [(f64, f64); 4], out_w: u32, out_h: u32) -> Gray {
    // Forward homography maps source-rect corners -> dst_corners; we need the
    // inverse for sampling, so solve dst -> src directly.
    let src_corners = [
        (0.0, 0.0),
        ((src.w - 1) as f64, 0.0),
        ((src.w - 1) as f64, (src.h - 1) as f64),
        (0.0, (src.h - 1) as f64),
    ];
    let h = homography(dst_corners, src_corners);
    let mut out = Gray::new(out_w, out_h, 255);
    for y in 0..out_h {
        for x in 0..out_w {
            let (sx, sy) = apply_h(&h, x as f64, y as f64);
            out.set(x, y, src.sample(sx, sy, 255));
        }
    }
    out
}

/// Solves the 8-DOF homography mapping four `from` points to four `to` points.
fn homography(from: [(f64, f64); 4], to: [(f64, f64); 4]) -> [f64; 9] {
    // Build 8x8 linear system A h = b, with h33 = 1.
    let mut a = [[0.0f64; 8]; 8];
    let mut b = [0.0f64; 8];
    for i in 0..4 {
        let (x, y) = from[i];
        let (u, v) = to[i];
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y];
        b[2 * i] = u;
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y];
        b[2 * i + 1] = v;
    }
    let sol = solve8(a, b);
    [
        sol[0], sol[1], sol[2], sol[3], sol[4], sol[5], sol[6], sol[7], 1.0,
    ]
}

fn apply_h(h: &[f64; 9], x: f64, y: f64) -> (f64, f64) {
    let w = h[6] * x + h[7] * y + h[8];
    (
        (h[0] * x + h[1] * y + h[2]) / w,
        (h[3] * x + h[4] * y + h[5]) / w,
    )
}

/// Gaussian elimination with partial pivoting for an 8x8 system.
fn solve8(mut a: [[f64; 8]; 8], mut b: [f64; 8]) -> [f64; 8] {
    for col in 0..8 {
        // pivot
        let mut piv = col;
        for r in (col + 1)..8 {
            if a[r][col].abs() > a[piv][col].abs() {
                piv = r;
            }
        }
        a.swap(col, piv);
        b.swap(col, piv);
        let d = a[col][col];
        for r in 0..8 {
            if r == col {
                continue;
            }
            let f = a[r][col] / d;
            #[allow(clippy::needless_range_loop)]
            for c in col..8 {
                a[r][c] -= f * a[col][c];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = [0.0f64; 8];
    for i in 0..8 {
        x[i] = b[i] / a[i][i];
    }
    x
}

/// Adds deterministic salt/pepper-ish noise (bounded, so thresholding still
/// recovers modules).
fn add_noise(g: &mut Gray, amplitude: i32) {
    let mut state: u32 = 0x1234_5678;
    for p in g.px.iter_mut() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let n = ((state >> 8) % (2 * amplitude as u32 + 1)) as i32 - amplitude;
        *p = (*p as i32 + n).clamp(0, 255) as u8;
    }
}

// ---------------------------------------------------------------------------
// Verification helper
// ---------------------------------------------------------------------------

/// Detects in `g` and asserts exactly one symbol whose sampled matrix equals
/// `expected`.
fn assert_recovers(g: &Gray, expected: &BitMatrix, label: &str) {
    let dets = detect_luma(&g.px, g.w, g.h);
    assert_eq!(dets.len(), 1, "{label}: expected exactly one detection");
    let got = &dets[0].matrix;
    assert_eq!(got.size(), expected.size(), "{label}: dimension mismatch");
    let dim = expected.size();
    let mut mismatches = 0;
    for y in 0..dim {
        for x in 0..dim {
            if got.get(x, y) != expected.get(x, y) {
                mismatches += 1;
            }
        }
    }
    assert_eq!(mismatches, 0, "{label}: {mismatches} module mismatches");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn upright_v1() {
    let m = synth_matrix(1);
    let g = render(&m, 8, 4);
    assert_recovers(&g, &m, "upright v1");
}

#[test]
fn upright_v2_with_alignment() {
    let m = synth_matrix(2);
    let g = render(&m, 8, 4);
    assert_recovers(&g, &m, "upright v2");
}

#[test]
fn upright_v7() {
    let m = synth_matrix(7);
    let g = render(&m, 6, 4);
    assert_recovers(&g, &m, "upright v7");
}

#[test]
fn scaled_small_module() {
    let m = synth_matrix(2);
    let g = render(&m, 4, 4);
    assert_recovers(&g, &m, "scaled small");
}

#[test]
fn scaled_large_module() {
    let m = synth_matrix(1);
    let g = render(&m, 14, 5);
    assert_recovers(&g, &m, "scaled large");
}

#[test]
fn rotated_small_angle() {
    let m = synth_matrix(2);
    let g = render(&m, 10, 6);
    let r = rotate(&g, 0.12); // ~7 degrees
    assert_recovers(&r, &m, "rotated 7deg");
}

#[test]
fn rotated_larger_angle() {
    let m = synth_matrix(2);
    let g = render(&m, 10, 6);
    let r = rotate(&g, 0.35); // ~20 degrees
    assert_recovers(&r, &m, "rotated 20deg");
}

// ---------------------------------------------------------------------------
// Rotation matrix
//
// `Finder.module` is measured from axis-aligned scanline runs, so it is
// stretched by `1 / max(|cos θ|, |sin θ|)` under rotation while the
// finder-to-finder legs are not. Before `grouping::axis_scale` corrected for
// that, the dimension estimate fell short by `(dimension - 7) * (1 - cos θ)`
// and the detector silently snapped to a *smaller* valid version. The envelope
// shrank as the version grew: v5 broke at 23 degrees, v7 at 20, v10 at 17,
// v20 at 12. Each case below sat outside that envelope.
// ---------------------------------------------------------------------------

/// Renders `version`, pads it so no corner can be clipped, rotates by `degrees`
/// and requires the sampled grid to match module-for-module.
fn assert_recovers_rotated(version: u8, scale: u32, degrees: f64) {
    let m = synth_matrix(version);
    let g = render(&m, scale, 4);
    // A rotation about the centre needs `(sqrt(2) - 1) / 2` of the side on each
    // edge to stay clear of the canvas at any angle.
    let g = pad(&g, g.w * 3 / 10);
    let r = rotate(&g, degrees.to_radians());
    assert_recovers(&r, &m, &format!("v{version} rotated {degrees}deg"));
}

#[test]
fn rotated_v5_25deg() {
    assert_recovers_rotated(5, 10, 25.0);
}

#[test]
fn rotated_v5_30deg() {
    assert_recovers_rotated(5, 10, 30.0);
}

#[test]
fn rotated_v7_20deg() {
    assert_recovers_rotated(7, 10, 20.0);
}

#[test]
fn rotated_v10_17deg() {
    assert_recovers_rotated(10, 8, 17.0);
}

#[test]
fn rotated_v20_15deg() {
    assert_recovers_rotated(20, 8, 15.0);
}

/// The correction folds at 45 degrees, so the worst case for a large symbol is
/// the middle of the range rather than either end.
#[test]
fn rotated_v10_40deg() {
    assert_recovers_rotated(10, 8, 40.0);
}

#[test]
fn rotated_v5_45deg() {
    assert_recovers_rotated(5, 10, 45.0);
}

/// Past 45 degrees the axis-aligned measurement folds back; a symbol at 60
/// degrees must behave like one at 30.
#[test]
fn rotated_v5_60deg() {
    assert_recovers_rotated(5, 10, 60.0);
}

#[test]
fn perspective_warp_v2() {
    let m = synth_matrix(2);
    let g = render(&m, 10, 6);
    // Mild keystone: push the top edge inward.
    let w = g.w as f64;
    let h = g.h as f64;
    let corners = [
        (0.10 * w, 0.05 * h),
        (0.88 * w, 0.02 * h),
        (0.97 * w, 0.95 * h),
        (0.04 * w, 0.90 * h),
    ];
    let p = perspective(&g, corners, g.w, g.h);
    assert_recovers(&p, &m, "perspective v2");
}

#[test]
fn perspective_warp_v7() {
    let m = synth_matrix(7);
    let g = render(&m, 8, 6);
    let w = g.w as f64;
    let h = g.h as f64;
    let corners = [
        (0.06 * w, 0.10 * h),
        (0.94 * w, 0.04 * h),
        (0.90 * w, 0.92 * h),
        (0.08 * w, 0.96 * h),
    ];
    let p = perspective(&g, corners, g.w, g.h);
    assert_recovers(&p, &m, "perspective v7");
}

#[test]
fn noisy_image() {
    let m = synth_matrix(2);
    let mut g = render(&m, 10, 6);
    add_noise(&mut g, 40);
    assert_recovers(&g, &m, "noisy v2");
}

#[test]
fn two_codes_in_one_image() {
    let m = synth_matrix(1);
    let a = render(&m, 8, 4);
    // Compose two copies side by side into one wider image.
    let gap = 40u32;
    let w = a.w * 2 + gap;
    let h = a.h;
    let mut combined = Gray::new(w, h, 255);
    for y in 0..a.h {
        for x in 0..a.w {
            let v = a.px[(y * a.w + x) as usize];
            combined.set(x, y, v);
            combined.set(x + a.w + gap, y, v);
        }
    }
    let dets = detect_luma(&combined.px, combined.w, combined.h);
    assert_eq!(dets.len(), 2, "two codes: expected two detections");
    for d in &dets {
        assert_eq!(d.matrix.size(), m.size());
    }
}

#[test]
fn blank_image_yields_nothing() {
    let g = Gray::new(200, 200, 255);
    assert!(detect_luma(&g.px, g.w, g.h).is_empty());
}

/// Smoke test for the decode wiring: the synthetic fixture is not a valid QR
/// payload, so `qr_decode::decode` must return an error gracefully (no panic)
/// and leave `decoded` as `None`. A full encode -> detect -> decode roundtrip
/// lands in Step 3b (needs qr-encode, excluded here).
#[cfg(feature = "decode")]
#[test]
fn decode_field_present_and_graceful() {
    let m = synth_matrix(2);
    let g = render(&m, 8, 4);
    let dets = detect_luma(&g.px, g.w, g.h);
    assert_eq!(dets.len(), 1);
    assert!(dets[0].decoded.is_none());
}
