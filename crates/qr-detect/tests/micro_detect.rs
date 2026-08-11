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
use qr_detect::{detect_all_luma, detect_micro_luma, detect_micro_luma_single_pass};

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
    render_contrast(m, scale, quiet, 0)
}

/// [`render`] with the dark modules at an arbitrary luma instead of 0, i.e. at
/// arbitrary print contrast.
///
/// Ink is never actually black, and micro symbols are where that bites hardest:
/// a micro finder core is a larger fraction of its frame than a QR one, so it is
/// the first thing to hollow out under a binarization rule that mistakes "very
/// dark" for "large".
fn render_contrast(m: &BitMatrix, scale: u32, quiet: u32, dark: u8) -> Gray {
    let dim = m.size();
    let side = (dim + 2 * quiet) * scale;
    let mut g = Gray::new(side, side, 255);
    for y in 0..dim {
        for x in 0..dim {
            let v = if m.get(x, y) { dark } else { 255 };
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

/// Which edge of the image a keystone warp pulls in.
///
/// All four are tested, and they are not equivalent: a micro symbol's finder is
/// at one corner, so the two edges that touch it are measured directly while the
/// two opposite ones are inferred, and a tilt that compresses the far edges is
/// the harder case. `Diagonal` tilts about a corner, which foreshortens both
/// axes at once.
#[derive(Clone, Copy, Debug)]
enum Tilt {
    Top,
    Bottom,
    Left,
    Right,
    Diagonal,
}

impl Tilt {
    const ALL: [Tilt; 5] = [
        Tilt::Top,
        Tilt::Bottom,
        Tilt::Left,
        Tilt::Right,
        Tilt::Diagonal,
    ];
}

/// Warps `g` by pulling one edge in by `shrink` of the image's span on each
/// side — a symbol tilted away from the camera about the opposite edge.
fn keystone(g: &Gray, shrink: f64, tilt: Tilt) -> Gray {
    let (w, h) = (g.w as f64 - 1.0, g.h as f64 - 1.0);
    let dst = match tilt {
        Tilt::Top => [
            (w * shrink, 0.0),
            (w * (1.0 - shrink), 0.0),
            (w, h),
            (0.0, h),
        ],
        Tilt::Bottom => [
            (0.0, 0.0),
            (w, 0.0),
            (w * (1.0 - shrink), h),
            (w * shrink, h),
        ],
        Tilt::Left => [
            (0.0, h * shrink),
            (w, 0.0),
            (w, h),
            (0.0, h * (1.0 - shrink)),
        ],
        Tilt::Right => [
            (0.0, 0.0),
            (w, h * shrink),
            (w, h * (1.0 - shrink)),
            (0.0, h),
        ],
        Tilt::Diagonal => [
            (w * shrink, h * shrink),
            (w * (1.0 - shrink), 0.0),
            (w, h),
            (0.0, h * (1.0 - shrink)),
        ],
    };
    perspective(g, dst, g.w, g.h)
}

/// Renders `version` and returns it keystoned by `shrink` about `tilt`.
fn tilted(version: MicroVersion, scale: u32, shrink: f64, tilt: Tilt) -> (Gray, BitMatrix) {
    let m = micro_fixture(version, 0, 1234);
    let g = pad(&render(&m, scale, 4), scale * 2);
    (keystone(&g, shrink, tilt), m)
}

/// The 40 fixtures the envelope is pinned over: every version, two module sizes
/// and all five tilt directions.
fn envelope_matrix(shrink: f64) -> impl Iterator<Item = (String, Gray, BitMatrix)> {
    MicroVersion::ALL.into_iter().flat_map(move |version| {
        [8u32, 12].into_iter().flat_map(move |scale| {
            Tilt::ALL.into_iter().map(move |tilt| {
                let (g, m) = tilted(version, scale, shrink, tilt);
                (format!("{version:?} at {scale} px, {tilt:?}"), g, m)
            })
        })
    })
}

/// The documented perspective envelope, measured rather than assumed.
///
/// A micro symbol has no alignment pattern, so three of its four corners are all
/// that can be measured directly; the fourth used to be the parallelogram
/// completion, which capped this at a 4% keystone. It is now searched for — see
/// `micro::refine_bottom_right` — and the envelope moves accordingly. Measured
/// over 120 configurations — four versions x three module sizes (8, 12, 20 px)
/// x masks 0 and 2 x five tilt directions, every one of them
/// `micro_fixture(version, mask, 1234)` — exact recovery goes from
///
/// | keystone | before | after |
/// |----------|--------|-------|
/// |       2% |  116   |  120  |
/// |       4% |   93   |  120  |
/// |       5% |   62   |  120  |
/// |       8% |   20   |  117  |
/// |      10% |   14   |  111  |
/// |      12% |    1   |  104  |
/// |      15% |    0   |   49  |
///
/// This test pins the 40-configuration subset at two module sizes, which runs in
/// a couple of seconds: everything exact to 5%, and a floor on how much of the
/// matrix survives at 10% and 12%. The floors are stated as counts rather than
/// as "all of them" because the far edge of the envelope is genuinely ragged —
/// which configuration falls over first depends on where module centres land —
/// and pinning individual cases there would be pinning noise.
#[test]
fn perspective_envelope() {
    for shrink in [0.0, 0.02, 0.04, 0.05] {
        for (label, g, m) in envelope_matrix(shrink) {
            assert!(
                recovers(&g, &m),
                "{label} under a {}% keystone was not recovered exactly",
                shrink * 100.0
            );
        }
    }

    for (shrink, floor) in [(0.10, 33), (0.12, 30)] {
        let mut ok = 0;
        let mut total = 0;
        for (_, g, m) in envelope_matrix(shrink) {
            total += 1;
            ok += u32::from(recovers(&g, &m));
        }
        assert!(
            ok >= floor,
            "only {ok} of {total} configurations survived a {}% keystone (floor {floor})",
            shrink * 100.0
        );
    }
}

/// Projects `g` as a plane rotated by `degrees` about an axis through its
/// centre, seen by a pinhole camera `distance` pixels away.
///
/// `axis_degrees` orients the tilt axis in the image plane: 0 tilts about the
/// horizontal, 90 about the vertical, 45 about a diagonal. The focal length is
/// the camera distance, so the centre of the image keeps its scale and all that
/// is left is the keystone.
///
/// The distance is in pixels, not in image widths, because what a tilt does to a
/// symbol depends on how large that symbol is *relative to the camera distance*
/// and on nothing else. A degrees figure on its own says very little: the same
/// 15 degree tilt is a 6% keystone across a symbol two of its own spans from the
/// lens and under 2% across one six spans away. Callers pass a multiple of the
/// symbol's span, which is the ratio a person holding a phone actually controls.
fn tilt_3d(g: &Gray, degrees: f64, d: f64, axis_degrees: f64) -> Gray {
    let (w, h) = (g.w as f64 - 1.0, g.h as f64 - 1.0);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (sa, ca) = axis_degrees.to_radians().sin_cos();
    let (st, ct) = degrees.to_radians().sin_cos();
    let project = |x: f64, y: f64| {
        let (px, py) = (x - cx, y - cy);
        // Split into components along and across the tilt axis; only the second
        // leaves the image plane.
        let along = px * ca + py * sa;
        let across = -px * sa + py * ca;
        let (tilted, z) = (across * ct, across * st);
        let k = d / (d + z);
        (
            cx + (along * ca - tilted * sa) * k,
            cy + (along * sa + tilted * ca) * k,
        )
    };
    let dst = [
        project(0.0, 0.0),
        project(w, 0.0),
        project(w, h),
        project(0.0, h),
    ];
    perspective(g, dst, g.w, g.h)
}

/// `12345` as an M1-Detection symbol, mask 0.
///
/// Lifted verbatim from `tests-integration/tests/micro_golden.rs`, where it is a
/// **segno 1.6.6** vector. M1 is the version the Project Owner's phone was
/// looking at, and unlike `micro_fixture` this one actually decodes, which is
/// what the scenario below has to assert: a detection that samples a wrong grid
/// is not a detection, it is a wrong answer waiting to be reported.
const M1_GOLDEN_12345: &[&str] = &[
    "11111110101",
    "10000010110",
    "10111010000",
    "10111010101",
    "10111010010",
    "10000010010",
    "11111110000",
    "00000000111",
    "11000100011",
    "01111011001",
    "10111001010",
];

fn golden_m1() -> BitMatrix {
    let mut m = BitMatrix::new(11);
    for (y, row) in M1_GOLDEN_12345.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            m.set(x as u32, y as u32, c == b'1');
        }
    }
    m
}

/// The Project Owner's report, approximated: an M1 on a bright rectangle inside
/// a larger, dimmer frame, photographed off-axis.
///
/// This is the scenario the whole step exists for. The symbol was located —
/// corners drawn, "found but unreadable" — and never decoded, while a full QR
/// symbol at the same angle did, because the QR path solves a homography from
/// three finders and the micro path was completing a parallelogram.
///
/// The tilt is a real pinhole projection rather than a hand-picked quad, swept
/// over angle, tilt axis and camera distance. Distances are in spans of the
/// symbol itself: 2 is a close macro shot, 6 is a phone held at a comfortable
/// arm's length from a small label. Measured on the pre-refinement detector,
/// this fixture fails from 10 degrees at 2 spans and from 15 at 4, which is
/// exactly the report.
///
/// The floor at 25 degrees is a count rather than "all of them" for the same
/// reason as in `perspective_envelope`: past the middle of the envelope which
/// combination falls over first is a question about where module centres land.
#[cfg(feature = "decode")]
#[test]
fn a_micro_symbol_on_a_screen_photographed_off_axis_decodes() {
    for degrees in [10.0f64, 15.0] {
        for (label, g) in off_axis_scenes(degrees) {
            let dets = detect_micro_luma(&g.px, g.w, g.h);
            assert_eq!(dets.len(), 1, "{label}: expected one micro detection");
            let decoded = dets[0]
                .decoded
                .as_ref()
                .unwrap_or_else(|| panic!("{label}: located but not decoded"));
            assert_eq!(decoded.bytes, b"12345", "{label}: payload");
        }
    }

    let (mut ok, mut total) = (0, 0);
    for (_, g) in off_axis_scenes(25.0) {
        total += 1;
        let dets = detect_micro_luma(&g.px, g.w, g.h);
        ok += u32::from(dets.len() == 1 && dets[0].decoded.is_some());
    }
    assert!(
        ok >= 14,
        "only {ok} of {total} scenes decoded at 25 degrees"
    );
}

/// The same scenario without `decode`, where the assertion has to be on the
/// sampled grid instead. Same scenes, same bar: every module.
#[cfg(not(feature = "decode"))]
#[test]
fn a_micro_symbol_on_a_screen_photographed_off_axis_is_sampled_exactly() {
    let m = golden_m1();
    for degrees in [10.0f64, 15.0] {
        for (label, g) in off_axis_scenes(degrees) {
            assert_micro_recovers(&g, &m, &label);
        }
    }

    let (mut ok, mut total) = (0, 0);
    for (_, g) in off_axis_scenes(25.0) {
        total += 1;
        ok += u32::from(recovers(&g, &m));
    }
    assert!(
        ok >= 14,
        "only {ok} of {total} scenes recovered at 25 degrees"
    );
}

/// The 16 scenes the off-axis scenario is measured over: four camera distances,
/// in spans of the symbol, and four tilt axes.
fn off_axis_scenes(degrees: f64) -> Vec<(String, Gray)> {
    const SCALE: u32 = 9;
    let m = golden_m1();
    let span = (m.size() * SCALE) as f64;
    let mut out = Vec::new();
    for spans in [2.0f64, 3.0, 4.0, 6.0] {
        for axis in [0.0f64, 45.0, 90.0, 135.0] {
            // A dim room (luma 150) with a bright rectangle in it — the screen —
            // carrying the symbol well inside its own quiet zone.
            let symbol = render(&m, SCALE, 4);
            let mut screen = Gray::new(symbol.w + 120, symbol.h + 80, 255);
            screen.blit(&symbol, 60, 40);
            let mut frame = Gray::new(640, 480, 150);
            frame.blit(&screen, (640 - screen.w) / 2, (480 - screen.h) / 2);
            out.push((
                format!("{degrees} degrees about a {axis} degree axis, {spans} spans away"),
                tilt_3d(&frame, degrees, spans * span, axis),
            ));
        }
    }
    out
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

/// The rejections above, under the tilts the refinement exists to handle.
///
/// This is the risk the refinement introduces and the reason it is gated the way
/// it is. It has two free parameters and two searches, one of which scores
/// candidates on the image itself without knowing what the symbol should
/// contain; left to itself it will happily find *a* grid for anything
/// finder-shaped. What stops it is that nothing it produces is adopted until it
/// has passed the same function-pattern budget as before, plus a quiet-zone
/// check that the affine map never needed — see `micro::quiet_zone_dark`.
///
/// Four hand-built negatives, each under five keystones in each of five
/// directions — 25 warps per fixture, 100 in all. A phantom that only appears
/// under one of them is still a phantom.
#[test]
fn tilting_does_not_resurrect_a_rejected_candidate() {
    // A bare finder: nothing beyond the separator but quiet zone.
    let mut bare = BitMatrix::new(11);
    for y in 0..7 {
        for x in 0..7 {
            let border = x == 0 || x == 6 || y == 0 || y == 6;
            let core = (2..=4).contains(&x) && (2..=4).contains(&y);
            bare.set(x, y, border || core);
        }
    }

    // A real micro symbol with its horizontal timing pattern filled in solid.
    let mut broken = micro_fixture(MicroVersion::M3, 0, 31337);
    for k in 8..broken.size() {
        broken.set(k, 0, true);
    }

    // The nested-triple phantom: a finder pattern whose own dark ring reads as a
    // second, larger `1:1:3:1:1` run. The QR path rejects it as a triple; the
    // micro path must not adopt it either.
    let mut nested = BitMatrix::new(15);
    for y in 0..11 {
        for x in 0..11 {
            let outer = x == 0 || x == 10 || y == 0 || y == 10;
            let ring = (2..=8).contains(&x)
                && (2..=8).contains(&y)
                && (x == 2 || x == 8 || y == 2 || y == 8);
            let core = (4..=6).contains(&x) && (4..=6).contains(&y);
            nested.set(x, y, outer || ring || core);
        }
    }

    let cases: [(&str, Gray); 4] = [
        ("bare finder", pad(&render(&bare, 10, 6), 40)),
        ("broken timing", pad(&render(&broken, 10, 4), 40)),
        ("nested triple", pad(&render(&nested, 10, 6), 40)),
        ("noise", {
            let mut g = Gray::new(320, 320, 255);
            add_noise(&mut g, 120);
            g
        }),
    ];

    for (label, base) in cases {
        for tilt in Tilt::ALL {
            for shrink in [0.0, 0.05, 0.10, 0.15, 0.20] {
                let g = keystone(&base, shrink, tilt);
                assert!(
                    detect_micro_luma(&g.px, g.w, g.h).is_empty(),
                    "{label} at a {}% keystone about {tilt:?} was reported as a micro symbol",
                    shrink * 100.0
                );
            }
        }
    }
}

/// A QR symbol under tilt must not shed phantom micro symbols either. Its three
/// finders are consumed by the triple, but a triple that the grouping rejects
/// under a steep tilt would release them again.
#[test]
fn a_tilted_qr_symbol_yields_no_micro() {
    for version in [1u8, 2, 7] {
        let base = pad(&render(&qr_fixture(version), 8, 4), 32);
        for tilt in Tilt::ALL {
            for shrink in [0.0, 0.05, 0.10, 0.15] {
                let g = keystone(&base, shrink, tilt);
                let (_, micros) = detect_all_luma(&g.px, g.w, g.h);
                assert!(
                    micros.is_empty(),
                    "v{version} at a {}% keystone about {tilt:?}: {} phantom micro symbols",
                    shrink * 100.0,
                    micros.len()
                );
            }
        }
    }
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

// ---------------------------------------------------------------------------
// Print contrast
// ---------------------------------------------------------------------------

#[test]
fn micro_symbols_survive_worn_print_contrast() {
    // Regression lock from the KG-11 round-1 review, which measured exactly
    // these four fixtures failing from reflectance 0.30 upward under a
    // binarization rule that deferred to a wider window based on how dark a
    // feature was rather than how large. Every luma-0 fixture in this file
    // stayed green throughout, which is why the axis is pinned explicitly now.
    //
    // 0.55 is worn thermal print, the value the vignette fixtures already model;
    // 0.60 is a step past it, and 0.30 is where the round-1 failures began.
    //
    // `detect_micro_luma_single_pass` is asserted rather than
    // `detect_micro_luma`: these must be carried by the primary pass. If the
    // retry were quietly rescuing ordinary print, every such frame would pay two
    // full binarizations for a case the primary is supposed to own.
    let cases = [
        (MicroVersion::M2, 8u32, 4u32, 1u8),
        (MicroVersion::M3, 10, 4, 1),
        (MicroVersion::M4, 20, 3, 1),
        (MicroVersion::M2, 10, 2, 1),
    ];
    for (version, scale, quiet, mask) in cases {
        let m = micro_fixture(version, mask, 4242);
        for refl in [0.0f64, 0.20, 0.30, 0.35, 0.45, 0.55, 0.60] {
            let dark = (255.0 * refl).round() as u8;
            let g = render_contrast(&m, scale, quiet, dark);
            let label = format!("{version:?} @ {scale}px q{quiet}, reflectance {refl}");

            let dets = detect_micro_luma_single_pass(&g.px, g.w, g.h);
            assert_eq!(dets.len(), 1, "{label}: primary pass lost the symbol");
            assert_eq!(
                mismatches(&dets[0].matrix, &m),
                0,
                "{label}: module mismatches"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Illumination: the primary pass and the retry pass
// ---------------------------------------------------------------------------

/// An M4-Q symbol carrying `"12345"`, mask 1.
///
/// Lifted verbatim from `tests-integration/tests/micro_golden.rs`, where it is
/// one of the vectors generated by **segno 1.6.6** and read back by
/// **zxing-cpp 3.1.1** — implementations that share no code with ours. It is
/// used here for the one property `micro_fixture` cannot provide: it actually
/// decodes. A test that the retry recovers a *decodable* payload cannot be
/// written against a structurally-valid-but-meaningless grid, because both
/// passes would fail to decode it and the trigger would have nothing to key on.
///
/// Row-major, `'1'` dark, top row first.
const M4_GOLDEN_12345: &[&str] = &[
    "11111110101010101",
    "10000010010011100",
    "10111010001100011",
    "10111010111110011",
    "10111010010111110",
    "10000010110110011",
    "11111110011010110",
    "00000000100101110",
    "10110001111101110",
    "01100100010011111",
    "11100010001011000",
    "00110001010110011",
    "11101000000100101",
    "00001000000110001",
    "10110101010100111",
    "01011001000001111",
    "10100010100011100",
];

fn golden_m4() -> BitMatrix {
    let mut m = BitMatrix::new(17);
    for (y, row) in M4_GOLDEN_12345.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            m.set(x as u32, y as u32, c == b'1');
        }
    }
    m
}

/// Renders `m` at `scale` px/module into the corner of a 1280x720 camera frame
/// under a radial vignette, at worn-thermal-print contrast (0.55).
///
/// The surrounding frame is plain white paper, so the quiet zone is implicit and
/// far larger than the 2 modules ISO 6.3.6 requires.
fn vignetted_micro_frame(m: &BitMatrix, scale: u32, depth: f64) -> Gray {
    let (w, h) = (1280u32, 720u32);
    let (ox, oy) = (40u32, 40u32);
    let span = m.size() * scale;
    let cx = (w - 1) as f64 / 2.0;
    let cy = (h - 1) as f64 / 2.0;
    let r_max = (cx * cx + cy * cy).sqrt();

    let mut g = Gray::new(w, h, 255);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f64 - cx, y as f64 - cy);
            let r = (dx * dx + dy * dy).sqrt() / r_max;
            let illumination = 1.0 - depth * r * r;
            let mut reflectance = 1.0f64;
            if x >= ox
                && y >= oy
                && x < ox + span
                && y < oy + span
                && m.get((x - ox) / scale, (y - oy) / scale)
            {
                reflectance = 0.55;
            }
            g.set(
                x,
                y,
                (255.0 * illumination * reflectance)
                    .round()
                    .clamp(0.0, 255.0) as u8,
            );
        }
    }
    g
}

/// The regression for KG-11 round-2 BLOCKER-2.
///
/// A micro symbol is anchored on a single finder. Near a vignetted frame corner
/// that finder survives a binarization the timing pattern and data area do not,
/// so the primary pass samples a *wrong grid* and returns a detection anyway.
/// Under the old candidate-count trigger that detection suppressed the retry —
/// on a frame the retry resolves exactly. Measured here at 28 wrong modules out
/// of an M4's 289, far outside any error-correction budget.
///
/// The invariant is asserted over a depth band rather than at one pinned depth.
/// The exact depths at which the primary degrades this way are an artifact of
/// where module centres land under the vignette, and they will move if sampling
/// ever changes; the *rule* — a degraded primary sample must never be the final
/// answer when the retry can decode — must not. `found` guards against the loop
/// silently going vacuous, which is the failure mode that let round 2 through.
#[cfg(feature = "decode")]
#[test]
fn a_degraded_micro_sample_does_not_suppress_the_retry() {
    let m = golden_m4();
    let mut found = 0;

    for step in 914..=928u32 {
        let depth = step as f64 / 1000.0;
        let g = vignetted_micro_frame(&m, 8, depth);

        let primary = detect_micro_luma_single_pass(&g.px, g.w, g.h);
        let degraded = primary.len() == 1 && primary[0].decoded.is_none();
        if !degraded {
            continue;
        }
        found += 1;
        assert!(
            mismatches(&primary[0].matrix, &m) > 0,
            "depth {depth}: a non-decoding sample should be a wrong grid"
        );

        let dets = detect_micro_luma(&g.px, g.w, g.h);
        assert_eq!(dets.len(), 1, "depth {depth}: retry must return one symbol");
        assert_eq!(
            mismatches(&dets[0].matrix, &m),
            0,
            "depth {depth}: the retry's grid must be exact"
        );
        let decoded = dets[0]
            .decoded
            .as_ref()
            .unwrap_or_else(|| panic!("depth {depth}: the retry's detection must decode"));
        assert_eq!(decoded.bytes, b"12345", "depth {depth}: recovered payload");
    }

    assert!(
        found >= 3,
        "fixture no longer produces degraded primary samples ({found} found); \
         the bypass is no longer under test"
    );
}

/// The `--no-default-features` half of the same rule (specifically: `micro` on,
/// `decode` off).
///
/// Without `decode` there is no decode outcome to key on, so the trigger stays a
/// candidate count and these frames keep the old, weaker behaviour: the degraded
/// primary detection is returned as-is and no retry happens. That is a
/// documented limitation rather than a bug — such builds decode downstream and
/// are the only ones able to judge a sample — and it is pinned so it stays
/// deliberate rather than drifting.
#[cfg(not(feature = "decode"))]
#[test]
fn without_decode_the_trigger_stays_a_candidate_count() {
    let m = golden_m4();
    let mut found = 0;

    for step in 914..=928u32 {
        let depth = step as f64 / 1000.0;
        let g = vignetted_micro_frame(&m, 8, depth);

        let primary = detect_micro_luma_single_pass(&g.px, g.w, g.h);
        if primary.len() != 1 {
            continue;
        }
        found += 1;
        let dets = detect_micro_luma(&g.px, g.w, g.h);
        assert_eq!(dets.len(), 1, "depth {depth}");
        assert_eq!(
            mismatches(&dets[0].matrix, &m),
            mismatches(&primary[0].matrix, &m),
            "depth {depth}: a no-decode build must return the primary's result unchanged"
        );
    }

    assert!(
        found >= 3,
        "fixture produced no primary detections to check"
    );
}

/// The combined entry point must not let one degraded micro detection suppress
/// the retry for the QR side as well — the review measured exactly that, two
/// payloads lost to one bad sample.
#[cfg(feature = "decode")]
#[test]
fn a_degraded_micro_sample_does_not_suppress_the_retry_in_detect_all() {
    let m = golden_m4();
    let mut found = 0;

    for step in 914..=928u32 {
        let depth = step as f64 / 1000.0;
        let g = vignetted_micro_frame(&m, 8, depth);

        let primary = detect_micro_luma_single_pass(&g.px, g.w, g.h);
        if !(primary.len() == 1 && primary[0].decoded.is_none()) {
            continue;
        }
        found += 1;

        let (qr, micro) = detect_all_luma(&g.px, g.w, g.h);
        assert!(qr.is_empty(), "depth {depth}: no QR symbol in this frame");
        assert_eq!(micro.len(), 1, "depth {depth}");
        assert_eq!(
            mismatches(&micro[0].matrix, &m),
            0,
            "depth {depth}: grid must be exact"
        );
        assert_eq!(
            micro[0].decoded.as_ref().expect("must decode").bytes,
            b"12345",
            "depth {depth}"
        );
    }

    assert!(found >= 3, "the bypass is no longer under test");
}
