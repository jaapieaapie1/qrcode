//! End-to-end Micro QR detection tests over synthetic fixtures.
//!
//! Symbols are built here from `qr-core`'s micro primitives — the placement
//! iterator, the four micro masks and the format-information writer — so the
//! fixtures depend on nothing that is still being built (qr-encode's
//! `encode_micro` and qr-decode's `decode_micro` are concurrent work). One
//! fixture is the real thing: the ISO Annex I.4 M2 symbol, transcribed module
//! for module.
//!
//! The data payload of the generated fixtures is pseudo-random rather than
//! Reed-Solomon valid: detection samples modules and never inspects the
//! codeword stream, and inventing an RS layout here would duplicate (and risk
//! contradicting) the encoder. The end-to-end encode -> detect -> decode
//! roundtrip is Step 5e's job.
//!
//! The rendering and warping helpers are deliberately a copy of the ones in
//! `detect.rs`: each integration test file stays self-contained, and these are
//! the fixtures' own scaffolding, not code under test.

#![cfg(feature = "micro")]

use qr_core::BitMatrix;
use qr_core::micro::{
    MicroVersion, micro_data_modules, micro_format_info_encode, micro_format_info_write,
    micro_mask_bit,
};
use qr_detect::{detect_all_luma, detect_micro_luma};

// ---------------------------------------------------------------------------
// Micro symbol fixtures
// ---------------------------------------------------------------------------

/// Builds a structurally valid micro symbol: finder, separator, both timing
/// patterns, format information for `mask`, and masked pseudo-random data in
/// every remaining module.
fn micro_fixture(version: MicroVersion, mask: u8, seed: u32) -> BitMatrix {
    let size = version.size();
    let mut m = BitMatrix::new(size);

    // Single finder pattern at the top-left corner. The separator (row 7 and
    // column 7) is light, which is the default.
    for y in 0..7 {
        for x in 0..7 {
            let border = x == 0 || x == 6 || y == 0 || y == 6;
            let core = (2..=4).contains(&x) && (2..=4).contains(&y);
            m.set(x, y, border || core);
        }
    }

    // Timing patterns run the whole of row 0 and column 0.
    for k in 8..size {
        m.set(k, 0, k % 2 == 0);
        m.set(0, k, k % 2 == 0);
    }

    // Masked pseudo-random data.
    let mut state = seed | 1;
    for (x, y) in micro_data_modules(version) {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let bit = (state >> 20) & 1 == 1;
        m.set(x, y, bit ^ micro_mask_bit(mask, x, y));
    }

    let ec = version.ec_levels()[0];
    let format = micro_format_info_encode(version, ec, mask).expect("format info");
    micro_format_info_write(&mut m, format);
    m
}

/// ISO/IEC 18004 Annex I.4, Figure I.4: the M2-L symbol encoding "01234567"
/// (`#` = dark). Cross-checked against `qr-core`'s own Annex I.3 test, which
/// rebuilds this grid from the primitives.
fn figure_i4() -> BitMatrix {
    const ROWS: [&str; 13] = [
        "#######.#.#.#",
        "#.....#.###.#",
        "#.###.#..##.#",
        "#.###.#..####",
        "#.###.#.###..",
        "#.....#.#...#",
        "#######..####",
        ".........##..",
        "##.#....#...#",
        ".##.#.#.#.#.#",
        "###..#######.",
        "...#.#....##.",
        "###.#..##.###",
    ];
    let mut m = BitMatrix::new(13);
    for (y, row) in ROWS.iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            m.set(x as u32, y as u32, ch == '#');
        }
    }
    m
}

/// A full QR symbol (correct function patterns plus deterministic data), used
/// to prove that QR and micro detection do not steal each other's finders.
fn qr_fixture(version: u8) -> BitMatrix {
    let dim = 17 + 4 * version as u32;
    let mut m = BitMatrix::new(dim);

    for y in 0..dim {
        for x in 0..dim {
            let mut h = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)).wrapping_add(1);
            h ^= h >> 13;
            h = h.wrapping_mul(0x5bd1_e995);
            h ^= h >> 15;
            m.set(x, y, h & 1 == 0);
        }
    }
    for (ox, oy) in [(0, 0), (dim - 7, 0), (0, dim - 7)] {
        for dy in 0..7 {
            for dx in 0..7 {
                let border = dx == 0 || dx == 6 || dy == 0 || dy == 6;
                let core = (2..=4).contains(&dx) && (2..=4).contains(&dy);
                m.set(ox + dx, oy + dy, border || core);
            }
        }
    }
    for i in 0..8 {
        m.set(7, i, false);
        m.set(i, 7, false);
        m.set(dim - 8, i, false);
        m.set(dim - 1 - i, 7, false);
        m.set(7, dim - 1 - i, false);
        m.set(i, dim - 8, false);
    }
    for k in 8..dim - 8 {
        m.set(k, 6, k % 2 == 0);
        m.set(6, k, k % 2 == 0);
    }
    if version >= 2 {
        let c = dim - 7;
        for dy in 0..5i32 {
            for dx in 0..5i32 {
                let ring = dx == 0 || dx == 4 || dy == 0 || dy == 4;
                let center = dx == 2 && dy == 2;
                m.set(
                    (c as i32 - 2 + dx) as u32,
                    (c as i32 - 2 + dy) as u32,
                    ring || center,
                );
            }
        }
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
    /// Blits `other` at `(ox, oy)`.
    fn blit(&mut self, other: &Gray, ox: u32, oy: u32) {
        for y in 0..other.h {
            for x in 0..other.w {
                self.set(ox + x, oy + y, other.px[(y * other.w + x) as usize]);
            }
        }
    }
}

/// Renders a module grid: `scale` px per module, `quiet` modules of white
/// border. Dark modules -> 0, light -> 255.
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
            out.set(
                x,
                y,
                src.sample(cx + dx * c + dy * s, cy - dx * s + dy * c, 255),
            );
        }
    }
    out
}

/// Widens `src` with `px` pixels of white on every side.
fn pad(src: &Gray, px: u32) -> Gray {
    let mut out = Gray::new(src.w + 2 * px, src.h + 2 * px, 255);
    out.blit(src, px, px);
    out
}

/// Projectively warps `src` so that its rectangle lands on `dst_corners`.
fn perspective(src: &Gray, dst_corners: [(f64, f64); 4], out_w: u32, out_h: u32) -> Gray {
    let src_corners = [
        (0.0, 0.0),
        ((src.w - 1) as f64, 0.0),
        ((src.w - 1) as f64, (src.h - 1) as f64),
        (0.0, (src.h - 1) as f64),
    ];
    // Sampling is inverse, so solve destination -> source directly.
    let h = homography(dst_corners, src_corners);
    let mut out = Gray::new(out_w, out_h, 255);
    for y in 0..out_h {
        for x in 0..out_w {
            let w = h[6] * x as f64 + h[7] * y as f64 + h[8];
            let sx = (h[0] * x as f64 + h[1] * y as f64 + h[2]) / w;
            let sy = (h[3] * x as f64 + h[4] * y as f64 + h[5]) / w;
            out.set(x, y, src.sample(sx, sy, 255));
        }
    }
    out
}

/// Solves the 8-DOF homography mapping four `from` points to four `to` points.
fn homography(from: [(f64, f64); 4], to: [(f64, f64); 4]) -> [f64; 9] {
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
    // Gaussian elimination with partial pivoting.
    for col in 0..8 {
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
    let mut h = [0.0f64; 9];
    for i in 0..8 {
        h[i] = b[i] / a[i][i];
    }
    h[8] = 1.0;
    h
}

/// Adds bounded deterministic noise.
fn add_noise(g: &mut Gray, amplitude: i32) {
    let mut state: u32 = 0x1234_5678;
    for p in g.px.iter_mut() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let n = ((state >> 8) % (2 * amplitude as u32 + 1)) as i32 - amplitude;
        *p = (*p as i32 + n).clamp(0, 255) as u8;
    }
}

// ---------------------------------------------------------------------------
// Verification helpers
// ---------------------------------------------------------------------------

fn mismatches(got: &BitMatrix, expected: &BitMatrix) -> u32 {
    assert_eq!(got.size(), expected.size(), "dimension mismatch");
    let mut n = 0;
    for y in 0..expected.size() {
        for x in 0..expected.size() {
            if got.get(x, y) != expected.get(x, y) {
                n += 1;
            }
        }
    }
    n
}

/// Detects in `g` and asserts exactly one micro symbol matching `expected`
/// module for module.
fn assert_micro_recovers(g: &Gray, expected: &BitMatrix, label: &str) {
    let dets = detect_micro_luma(&g.px, g.w, g.h);
    assert_eq!(
        dets.len(),
        1,
        "{label}: expected exactly one micro detection"
    );
    assert_eq!(
        mismatches(&dets[0].matrix, expected),
        0,
        "{label}: module mismatches"
    );
}

/// Whether the single micro symbol in `g` is recovered exactly (used by the
/// rotation sweep, which asserts a documented envelope rather than every case).
fn recovers(g: &Gray, expected: &BitMatrix) -> bool {
    let dets = detect_micro_luma(&g.px, g.w, g.h);
    dets.len() == 1 && mismatches(&dets[0].matrix, expected) == 0
}

// ---------------------------------------------------------------------------
// Upright
// ---------------------------------------------------------------------------

#[test]
fn upright_every_version() {
    for (i, version) in MicroVersion::ALL.into_iter().enumerate() {
        let m = micro_fixture(version, 0, 0x51 + i as u32 * 977);
        let g = render(&m, 8, 4);
        assert_micro_recovers(&g, &m, &format!("upright {version:?}"));
    }
}

#[test]
fn upright_every_mask() {
    for mask in 0..4u8 {
        let m = micro_fixture(MicroVersion::M3, mask, 0x1234 + mask as u32);
        let g = render(&m, 8, 4);
        assert_micro_recovers(&g, &m, &format!("mask {mask}"));
    }
}

#[test]
fn iso_figure_i4_symbol() {
    let m = figure_i4();
    let g = render(&m, 8, 4);
    assert_micro_recovers(&g, &m, "ISO Figure I.4");
}

#[test]
fn small_module_size() {
    let m = micro_fixture(MicroVersion::M2, 1, 7);
    let g = render(&m, 3, 4);
    assert_micro_recovers(&g, &m, "3 px modules");
}

#[test]
fn large_module_size() {
    let m = micro_fixture(MicroVersion::M4, 2, 11);
    let g = render(&m, 20, 3);
    assert_micro_recovers(&g, &m, "20 px modules");
}

#[test]
fn minimum_quiet_zone() {
    // ISO requires a two-module quiet zone around a micro symbol.
    let m = micro_fixture(MicroVersion::M2, 0, 99);
    let g = render(&m, 10, 2);
    assert_micro_recovers(&g, &m, "2-module quiet zone");
}

#[test]
fn noisy_image() {
    let m = micro_fixture(MicroVersion::M3, 1, 4242);
    let mut g = render(&m, 10, 4);
    add_noise(&mut g, 40);
    assert_micro_recovers(&g, &m, "noisy");
}

// ---------------------------------------------------------------------------
// Rotation
// ---------------------------------------------------------------------------

fn rotated(version: MicroVersion, scale: u32, degrees: f64) -> (Gray, BitMatrix) {
    let m = micro_fixture(version, 1, 0x2024 + degrees as u32);
    let g = pad(&render(&m, scale, 4), scale * 6);
    (rotate(&g, degrees.to_radians()), m)
}

#[test]
fn rotation_sweep_is_within_the_documented_envelope() {
    // Documented envelope: every micro version is recovered exactly at every
    // multiple of 2.5 degrees from 0 to 45 (and the frame search is 90-degree
    // symmetric, so that covers the circle). Any regression here is a real one.
    for version in MicroVersion::ALL {
        for step in 0..=18 {
            let degrees = step as f64 * 2.5;
            let (g, m) = rotated(version, 10, degrees);
            assert!(
                recovers(&g, &m),
                "{version:?} at {degrees} degrees was not recovered exactly"
            );
        }
    }
}

#[test]
fn rotation_beyond_45_degrees_folds() {
    // The frame search covers all four quadrants, so a symbol rotated past 45
    // degrees is simply one rotated the other way.
    for degrees in [50.0, 90.0, 137.0, 180.0, 225.0, 310.0] {
        let (g, m) = rotated(MicroVersion::M2, 10, degrees);
        assert!(recovers(&g, &m), "M2 at {degrees} degrees");
    }
}

#[test]
fn rotation_at_a_small_module_size() {
    // Rotation and a small module together: the module edges are the most
    // ragged here, which is what the timing walk's minimum-run filter exists
    // for.
    for degrees in [10.0f64, 20.0, 30.0, 40.0] {
        let m = micro_fixture(MicroVersion::M2, 1, 66);
        let g = rotate(&pad(&render(&m, 5, 4), 30), degrees.to_radians());
        assert!(recovers(&g, &m), "M2 at 5 px modules, {degrees} degrees");
    }
}

#[test]
fn awkward_angles() {
    for degrees in [7.5, 12.3, 22.5, 33.7, 41.1] {
        let (g, m) = rotated(MicroVersion::M4, 12, degrees);
        assert!(recovers(&g, &m), "M4 at {degrees} degrees");
    }
}

// ---------------------------------------------------------------------------
// Perspective
// ---------------------------------------------------------------------------

/// Warps a rendered symbol by pulling its top edge in by `shrink` of the image
/// width on each side (a symbol tilted away from the camera).
fn tilted(version: MicroVersion, scale: u32, shrink: f64) -> (Gray, BitMatrix) {
    let m = micro_fixture(version, 0, 1234);
    let g = pad(&render(&m, scale, 4), scale * 2);
    let (w, h) = (g.w as f64 - 1.0, g.h as f64 - 1.0);
    let dst = [
        (w * shrink, 0.0),
        (w * (1.0 - shrink), 0.0),
        (w, h),
        (0.0, h),
    ];
    (perspective(&g, dst, g.w, g.h), m)
}

#[test]
fn mild_perspective() {
    // A micro symbol has no alignment pattern, so there is no fourth
    // independent measurement and the grid map is affine. All the tolerance for
    // perspective therefore comes from the half-module margin around each
    // sampled module centre — a 17-module symbol has far less of it than a
    // 177-module QR symbol has with its alignment refinement. This is the
    // documented envelope: a tilt that pulls the top edge in by 4% of the
    // symbol width is recovered exactly.
    for version in MicroVersion::ALL {
        let (g, m) = tilted(version, 12, 0.04);
        assert!(recovers(&g, &m), "{version:?} under a 4% tilt");
    }
}

// ---------------------------------------------------------------------------
// Coexistence and rejection
// ---------------------------------------------------------------------------

#[test]
fn qr_and_micro_in_one_image() {
    let qr = qr_fixture(2);
    let micro = micro_fixture(MicroVersion::M4, 3, 555);
    let gq = render(&qr, 8, 4);
    let gm = render(&micro, 8, 4);

    let mut canvas = Gray::new(gq.w + gm.w + 40, gq.h.max(gm.h) + 20, 255);
    canvas.blit(&gq, 0, 10);
    canvas.blit(&gm, gq.w + 40, 10);

    let (qrs, micros) = detect_all_luma(&canvas.px, canvas.w, canvas.h);
    assert_eq!(qrs.len(), 1, "the QR symbol must be detected exactly once");
    assert_eq!(qrs[0].matrix.size(), qr.size());
    assert_eq!(mismatches(&qrs[0].matrix, &qr), 0, "QR modules");
    assert_eq!(micros.len(), 1, "the micro symbol must be detected once");
    assert_eq!(mismatches(&micros[0].matrix, &micro), 0, "micro modules");
}

#[test]
fn a_qr_symbol_alone_yields_no_micro() {
    // Its three finders are consumed by the triple, so none of them can anchor
    // a phantom micro symbol.
    for version in [1u8, 2, 7] {
        let qr = qr_fixture(version);
        let g = render(&qr, 8, 4);
        let (qrs, micros) = detect_all_luma(&g.px, g.w, g.h);
        assert_eq!(qrs.len(), 1, "v{version}: QR detection");
        assert!(micros.is_empty(), "v{version}: phantom micro symbols found");
    }
}

#[test]
fn isolated_finder_pattern_is_not_a_micro_symbol() {
    // A finder-shaped blob on its own passes the finder scan (that is its job)
    // and the square-geometry stage; the timing-pattern walk is what rejects
    // it, because there is nothing but quiet zone beyond the separator.
    let mut m = BitMatrix::new(11);
    for y in 0..7 {
        for x in 0..7 {
            let border = x == 0 || x == 6 || y == 0 || y == 6;
            let core = (2..=4).contains(&x) && (2..=4).contains(&y);
            m.set(x, y, border || core);
        }
    }
    let g = render(&m, 10, 6);
    assert!(
        detect_micro_luma(&g.px, g.w, g.h).is_empty(),
        "a bare finder pattern must not be reported as a micro symbol"
    );
}

#[test]
fn finder_with_broken_timing_is_rejected() {
    // Same symbol, but the horizontal timing pattern is solid dark instead of
    // alternating: the walk finds no module-sized runs and the candidate dies.
    let mut m = micro_fixture(MicroVersion::M3, 0, 31337);
    for k in 8..m.size() {
        m.set(k, 0, true);
    }
    let g = render(&m, 10, 4);
    assert!(
        detect_micro_luma(&g.px, g.w, g.h).is_empty(),
        "a symbol with no horizontal timing pattern must be rejected"
    );
}

#[test]
fn two_micro_symbols_in_one_image() {
    let a = micro_fixture(MicroVersion::M2, 0, 1);
    let b = micro_fixture(MicroVersion::M4, 3, 2);
    let ga = render(&a, 9, 4);
    let gb = render(&b, 9, 4);
    let mut canvas = Gray::new(ga.w + gb.w + 36, ga.h.max(gb.h) + 18, 255);
    canvas.blit(&ga, 0, 9);
    canvas.blit(&gb, ga.w + 36, 9);

    let dets = detect_micro_luma(&canvas.px, canvas.w, canvas.h);
    assert_eq!(dets.len(), 2, "both micro symbols must be detected");
    let mut sizes: Vec<u32> = dets.iter().map(|d| d.matrix.size()).collect();
    sizes.sort_unstable();
    assert_eq!(sizes, vec![a.size(), b.size()]);
}

#[test]
fn blank_image_yields_nothing() {
    let g = Gray::new(120, 120, 255);
    assert!(detect_micro_luma(&g.px, g.w, g.h).is_empty());
    let (qrs, micros) = detect_all_luma(&g.px, g.w, g.h);
    assert!(qrs.is_empty() && micros.is_empty());
}

#[test]
fn detect_all_matches_the_single_symbology_calls() {
    let micro = micro_fixture(MicroVersion::M2, 2, 8);
    let g = render(&micro, 8, 4);
    let (qrs, micros) = detect_all_luma(&g.px, g.w, g.h);
    assert!(qrs.is_empty());
    assert_eq!(micros.len(), detect_micro_luma(&g.px, g.w, g.h).len());
}

#[test]
fn corners_bound_the_symbol() {
    let m = micro_fixture(MicroVersion::M2, 0, 17);
    let (scale, quiet) = (10u32, 4u32);
    let g = render(&m, scale, quiet);
    let dets = detect_micro_luma(&g.px, g.w, g.h);
    assert_eq!(dets.len(), 1);

    let expected = [
        (quiet * scale, quiet * scale),
        ((quiet + m.size()) * scale, quiet * scale),
        ((quiet + m.size()) * scale, (quiet + m.size()) * scale),
        (quiet * scale, (quiet + m.size()) * scale),
    ];
    for (i, (ex, ey)) in expected.into_iter().enumerate() {
        let c = dets[0].corners[i];
        assert!(
            (c.x - ex as f64).abs() < scale as f64 && (c.y - ey as f64).abs() < scale as f64,
            "corner {i}: ({}, {}) is not within a module of ({ex}, {ey})",
            c.x,
            c.y
        );
    }
}

/// With `decode` on (the default), a detected micro symbol is run through
/// `qr_decode::micro::decode_micro`. Figure I.4 is a genuine M2-L symbol, so it
/// must come back as its payload; the generated fixtures carry pseudo-random
/// data rather than a Reed-Solomon-valid stream, so they must come back as
/// `None` rather than as a wrong answer.
#[cfg(feature = "decode")]
#[test]
fn detected_symbols_are_decoded() {
    let m = figure_i4();
    let g = render(&m, 8, 4);
    let dets = detect_micro_luma(&g.px, g.w, g.h);
    assert_eq!(dets.len(), 1);
    let decoded = dets[0].decoded.as_ref().expect("Figure I.4 must decode");
    assert_eq!(decoded.text.as_deref(), Some("01234567"));

    let synthetic = micro_fixture(MicroVersion::M3, 0, 909);
    let g = render(&synthetic, 8, 4);
    let dets = detect_micro_luma(&g.px, g.w, g.h);
    assert_eq!(dets.len(), 1);
    assert!(
        dets[0].decoded.is_none(),
        "a symbol with an invalid codeword stream must not decode"
    );
}

/// The function-pattern budget, exercised in both directions on a fixture that
/// actually reaches the validator.
///
/// The four rejection tests above all die inside the timing walk, so none of
/// them says anything about `MAX_FUNCTION_MISMATCHES`. These two do: the
/// corruption is confined to modules the validator checks and the timing
/// patterns are untouched, so the walk runs to completion and the verdict is the
/// validator's.
///
/// The modules chosen are corners of the finder's 3x3 core, turned light. They
/// are the only function modules that can be corrupted without disturbing the
/// stages before the validator: darkening a separator module would extend the
/// finder's outer ring into it and inflate the ray that measures the square,
/// and punching a hole in the ring would invalidate that ray entirely. Making a
/// core corner light only merges it into the gap ring, which leaves every
/// boundary radius — and therefore the whole frame — where it was. That the
/// one-casualty case is still detected is the proof that this corruption
/// reaches the validator rather than tripping something earlier.
#[test]
fn the_function_pattern_budget_is_one_module() {
    let mut one_casualty = micro_fixture(MicroVersion::M3, 0, 0xBEEF);
    one_casualty.set(2, 2, false);
    assert_micro_recovers(&render(&one_casualty, 10, 4), &one_casualty, "one casualty");

    let mut two_casualties = one_casualty.clone();
    two_casualties.set(4, 4, false);
    let g = render(&two_casualties, 10, 4);
    assert!(
        detect_micro_luma(&g.px, g.w, g.h).is_empty(),
        "two damaged function modules must be rejected by the validator"
    );
}
