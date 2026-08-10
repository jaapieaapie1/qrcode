//! Micro QR (ISO/IEC 18004:2024, M1-M4) detection.
//!
//! A micro symbol carries a *single* finder pattern, so the QR pipeline's
//! "three finders at right angles" grouping cannot see it. Everything up to and
//! including the finder scan is shared — [`crate::binarize`] and
//! [`crate::finder`] know nothing about symbology — and this module takes over
//! from a lone finder cluster:
//!
//! 1. **Square geometry.** Rays are cast from the finder centre and the radius
//!    of the outer dark ring is measured in every direction. The radius is
//!    minimal along the square's own axes and maximal at its corners, which
//!    yields the finder's rotation (modulo 90 degrees), its four corners and a
//!    rotation-invariant module size. [`crate::finder::Finder::module`] cannot
//!    be used directly: it is measured from axis-aligned scanline runs and is
//!    inflated by rotation (see `grouping::axis_scale`).
//! 2. **Orientation.** Modulo-90 leaves four candidate frames. The symbol lies
//!    in exactly one quadrant from the finder, and the two timing patterns —
//!    which in a micro symbol run the *whole* of row 0 and column 0 rather than
//!    between finders — pick it out: three of the four frames walk straight into
//!    the quiet zone and die on the first module.
//! 3. **Dimension.** Walking a timing pattern outward from the finder gives a
//!    run of alternating one-module cells; their count fixes the symbol size,
//!    which must land on one of 11/13/15/17. Both timing patterns must agree.
//!    Counting *transitions* rather than sampling predicted module centres makes
//!    the walk self-calibrating: an error in the initial module estimate shifts
//!    every transition but does not change how many there are.
//! 4. **Sampling.** The two measured timing endpoints plus the finder centre fix
//!    an affine grid->image map (completed to a quad for
//!    [`PerspectiveTransform`]); every module centre is then sampled.
//! 5. **Validation.** The sampled grid must actually contain a micro symbol's
//!    function patterns — finder, separator and both timing lines. This is the
//!    discriminator that keeps a lone finder-shaped blob, or a QR finder whose
//!    triple was rejected, from being reported as a micro symbol.

use crate::binarize::Binary;
use crate::finder::Finder;
use crate::geometry::{PerspectiveTransform, Point};
use qr_core::BitMatrix;
use qr_core::micro::{MicroVersion, micro_is_function_module};

/// A micro symbol sampled out of the image.
pub struct MicroSampled {
    /// The raw module grid (not unmasked).
    pub matrix: BitMatrix,
    /// Symbol corners in image space: top-left, top-right, bottom-right,
    /// bottom-left, where "top-left" is the finder's corner.
    pub corners: [Point; 4],
}

/// Ray resolution of the finder-square scan: 720 rays = half a degree.
const RAYS: usize = 720;

/// Angular tweaks (degrees) tried around the measured frame. Corner
/// localisation is good to a fraction of a module, which over a 7-module
/// baseline is still worth half a degree or so of rotation error; at 17 modules
/// that would drag the far end of a timing walk off its row.
const ANGLE_TWEAKS: [f64; 5] = [0.0, -0.5, 0.5, -1.0, 1.0];

/// How many function modules may disagree before a candidate is rejected.
///
/// The check covers the 49 finder modules, the 15 separator modules and both
/// timing patterns — 68 to 82 modules depending on the version — so one
/// binarisation casualty is absorbed and a second is not. The budget is
/// deliberately tight: two modules of slack is exactly the width of the
/// dimension-overshoot phantom, where a speck in the quiet zone one module past
/// the symbol edge adds a transition pair to both timing walks and lands the
/// walk on `dim + 2`, itself a valid micro size.
const MAX_FUNCTION_MISMATCHES: u32 = 1;

/// Detects a micro symbol anchored on `finder`, or `None` if the surroundings
/// are not a micro symbol.
pub fn detect_from_finder(bin: &Binary, finder: &Finder) -> Option<MicroSampled> {
    let (a, b) = square_axes(bin, finder)?;
    let module = (len(a) + len(b)) / 14.0;
    if !module.is_finite() || module < 1.0 {
        return None;
    }
    let ua = mul(a, 1.0 / len(a));
    let ub = mul(b, 1.0 / len(b));

    // The symbol occupies one of the four quadrants around the finder; `(u, v)`
    // is always right-handed in image coordinates (y downward), matching an
    // upright symbol's (rightward, downward) axes.
    let frames = [(ua, ub), (ub, neg(ua)), (neg(ua), neg(ub)), (neg(ub), ua)];

    let mut best: Option<(u32, MicroSampled)> = None;
    for (u0, v0) in frames {
        for tweak in ANGLE_TWEAKS {
            let rad = tweak.to_radians();
            let Some((mismatches, sampled)) =
                try_frame(bin, finder.center, rot(u0, rad), rot(v0, rad), module)
            else {
                continue;
            };
            if mismatches == 0 {
                return Some(sampled);
            }
            if best.as_ref().is_none_or(|(m, _)| mismatches < *m) {
                best = Some((mismatches, sampled));
            }
        }
    }
    best.map(|(_, s)| s)
}

/// Measures the finder's outer dark square and returns its two edge vectors
/// (each of length `7 * module`), ordered so that they are right-handed in
/// image coordinates.
///
/// A ray leaving the centre of a square of half-width `a` at angle `phi` exits
/// at `a / max(|cos(phi - theta)|, |sin(phi - theta)|)`: minimal on the square's
/// axes, maximal (`a * sqrt(2)`) at its corners. Summing the radius over the
/// four rays 90 degrees apart and minimising over `theta` therefore recovers the
/// rotation modulo 90 degrees; the corners then fall in the four diagonal
/// sectors, and the *points* at maximum radius are the corners themselves. Edge
/// vectors from opposite pairs of corners are averaged, which is far more
/// accurate than an angle read off a single ray.
fn square_axes(bin: &Binary, finder: &Finder) -> Option<(Point, Point)> {
    // `finder.module` is inflated by rotation, never deflated, so 6x it is a
    // safe bound for the 3.5-module half-width of the finder.
    let max_r = finder.module * 6.0;
    let step = (finder.module * 0.05).clamp(0.05, 0.5);
    // `finder.module` overstates the module size under rotation, never
    // understates it, so a third of it is a conservative speck threshold.
    let min_run = finder.module * 0.35;

    let mut radius = [0.0f64; RAYS];
    let mut valid = [false; RAYS];
    for (i, (r, ok)) in radius.iter_mut().zip(valid.iter_mut()).enumerate() {
        let phi = i as f64 * core::f64::consts::TAU / RAYS as f64;
        let (s, c) = phi.sin_cos();
        if let Some(hit) = boundary_radius(bin, finder.center, c, s, step, max_r, min_run) {
            *r = hit;
            *ok = true;
        }
    }

    const QUARTER: usize = RAYS / 4;
    let mut best: Option<(f64, usize)> = None;
    for t in 0..QUARTER {
        let mut sum = 0.0;
        let mut ok = true;
        for k in 0..4 {
            let idx = t + k * QUARTER;
            if !valid[idx] {
                ok = false;
                break;
            }
            sum += radius[idx];
        }
        if ok && best.is_none_or(|(bs, _)| sum < bs) {
            best = Some((sum, t));
        }
    }
    let (_, axis_idx) = best?;

    // Corners: the maximum-radius point in each diagonal sector.
    let mut corners = [Point::new(0.0, 0.0); 4];
    for (k, corner) in corners.iter_mut().enumerate() {
        let mid = (axis_idx + RAYS / 8 + k * QUARTER) as i32;
        let span = (RAYS / 16) as i32; // +/- 22.5 degrees
        let mut found: Option<(f64, Point)> = None;
        for d in -span..=span {
            let idx = (mid + d).rem_euclid(RAYS as i32) as usize;
            if !valid[idx] || found.is_some_and(|(r, _)| radius[idx] <= r) {
                continue;
            }
            let phi = idx as f64 * core::f64::consts::TAU / RAYS as f64;
            let (s, c) = phi.sin_cos();
            found = Some((
                radius[idx],
                Point::new(
                    finder.center.x + radius[idx] * c,
                    finder.center.y + radius[idx] * s,
                ),
            ));
        }
        *corner = found?.1;
    }

    // Opposite edges of a square are equal vectors; average each pair.
    let a = mul(
        add(sub(corners[1], corners[0]), sub(corners[2], corners[3])),
        0.5,
    );
    let b = mul(
        add(sub(corners[3], corners[0]), sub(corners[2], corners[1])),
        0.5,
    );
    let (la, lb) = (len(a), len(b));
    if la < 7.0 || lb < 7.0 || la > lb * 1.35 || lb > la * 1.35 {
        return None;
    }
    if (dot(a, b) / (la * lb)).abs() > 0.25 {
        return None;
    }
    if cross2(a, b) < 0.0 {
        Some((b, a))
    } else {
        Some((a, b))
    }
}

/// Radius at which a ray leaving the finder centre in direction `(cx, cy)`
/// crosses the outer edge of the finder's outer dark ring.
///
/// The profile out of the centre is dark (the 3x3 core), light (the gap ring),
/// dark (the outer ring), light (the separator or quiet zone), so the third
/// colour change is the edge we want. Anything else — including running out of
/// image — is a miss.
fn boundary_radius(
    bin: &Binary,
    center: Point,
    cx: f64,
    cy: f64,
    step: f64,
    max_r: f64,
    min_run: f64,
) -> Option<f64> {
    let sample = |r: f64| {
        bin.get(
            (center.x + cx * r).round() as i64,
            (center.y + cy * r).round() as i64,
        )
    };
    if !sample(0.0) {
        return None;
    }
    let changes = ray_transitions(sample, 0.0, max_r, step, min_run);
    changes.get(2).copied()
}

/// Walks a ray and returns the positions at which its colour changes, ignoring
/// runs shorter than `min_run`.
///
/// The filter is not optional. A rotated symbol is resampled, so its module
/// edges are anti-aliased and binarize into a ragged staircase; a ray crossing
/// such an edge at a shallow angle picks up one- and two-pixel specks. Without
/// the minimum-run rule those specks read as extra bands, which throws the
/// finder-square measurement off by several degrees and derails the timing
/// walk. Every real band on these rays is at least one module wide, so a
/// threshold of a third of a module is safe.
fn ray_transitions(
    sample: impl Fn(f64) -> bool,
    start: f64,
    end: f64,
    step: f64,
    min_run: f64,
) -> Vec<f64> {
    let mut changes = Vec::new();
    let mut color = sample(start);
    // A colour change that has not yet lasted `min_run`: (new colour, where it
    // started).
    let mut pending: Option<(bool, f64)> = None;
    let mut t = start;
    while t <= end {
        let c = sample(t);
        match pending {
            None => {
                if c != color {
                    pending = Some((c, t - step * 0.5));
                }
            }
            Some((pending_color, began)) => {
                if c != pending_color {
                    pending = None; // a speck, not a band
                } else if t - began >= min_run {
                    changes.push(began);
                    color = pending_color;
                    pending = None;
                }
            }
        }
        t += step;
    }
    changes
}

/// Tries one candidate frame: walks both timing patterns, builds the grid map,
/// samples the symbol and scores its function patterns. Returns the mismatch
/// count (lower is better) with the sampled symbol.
fn try_frame(
    bin: &Binary,
    center: Point,
    u: Point,
    v: Point,
    module: f64,
) -> Option<(u32, MicroSampled)> {
    // Grid origin (module (0, 0)'s outer corner): the finder centre sits 3.5
    // modules in along both axes.
    let origin = sub(center, mul(add(u, v), 3.5 * module));

    let (dim_row, edge_row) = walk_timing(bin, origin, u, v, module)?;
    let (dim_col, edge_col) = walk_timing(bin, origin, v, u, module)?;
    if dim_row != dim_col {
        return None;
    }
    let dim = dim_row;
    let version = MicroVersion::from_size(dim)?;
    let dim_f = dim as f64;

    // Two measured points on the symbol's far edges, at the outer end of each
    // timing pattern: grid (dim, 0.5) and grid (0.5, dim).
    let p_row = grid_point(origin, u, v, module, edge_row, 0.5);
    let p_col = grid_point(origin, v, u, module, edge_col, 0.5);
    // Three correspondences fix an affine map; the fourth is the parallelogram
    // completion in both spaces, so `quad_to_quad` reproduces exactly that map.
    // A micro symbol has no alignment pattern, so there is no fourth
    // independent measurement to pin down true perspective.
    let src = [
        Point::new(3.5, 3.5),
        Point::new(dim_f, 0.5),
        Point::new(dim_f - 3.0, dim_f - 3.0),
        Point::new(0.5, dim_f),
    ];
    let dst = [center, p_row, sub(add(p_row, p_col), center), p_col];
    let transform = PerspectiveTransform::quad_to_quad(src, dst);

    let mut matrix = BitMatrix::new(dim);
    for row in 0..dim {
        for col in 0..dim {
            let p = transform.map(col as f64 + 0.5, row as f64 + 0.5);
            matrix.set(col, row, bin.get(p.x.round() as i64, p.y.round() as i64));
        }
    }

    let mismatches = function_mismatches(&matrix, version);
    if mismatches > MAX_FUNCTION_MISMATCHES {
        return None;
    }

    let corners = [
        transform.map(0.0, 0.0),
        transform.map(dim_f, 0.0),
        transform.map(dim_f, dim_f),
        transform.map(0.0, dim_f),
    ];
    Some((mismatches, MicroSampled { matrix, corners }))
}

/// Walks one timing pattern outward from the finder and returns the symbol
/// dimension it implies together with the grid coordinate of the symbol's outer
/// edge along that axis (nominally the dimension itself; the measured value
/// absorbs the error in the initial module estimate).
///
/// `along` is the direction the timing pattern runs in, `across` the other
/// symbol axis. The ray is offset half a module across so that it travels down
/// the centre of row 0 (or column 0), starting inside the finder's outer ring
/// at grid coordinate 6.5.
///
/// The expected transition sequence is: dark ring -> separator at grid 7,
/// -> first timing module at 8, then one per module up to the symbol edge at
/// `dim`. Because the symbol edge is a dark -> light change and the walk starts
/// dark, that final transition has an even index.
fn walk_timing(
    bin: &Binary,
    origin: Point,
    along: Point,
    across: Point,
    module: f64,
) -> Option<(u32, f64)> {
    const STEP: f64 = 0.02;
    const START: f64 = 6.5;
    const END: f64 = 20.5;

    let sample = |t: f64| {
        let p = grid_point(origin, along, across, module, t, 0.5);
        bin.get(p.x.round() as i64, p.y.round() as i64)
    };

    if !sample(START) {
        // Not inside the finder's outer ring: the frame is wrong or the module
        // estimate is badly off.
        return None;
    }
    // Distances here are in modules, so the speck threshold is too.
    let transitions = ray_transitions(sample, START, END, STEP, 0.35);

    // The separator edge anchors the walk.
    if transitions.is_empty() || (transitions[0] - 7.0).abs() > 0.6 {
        return None;
    }
    let mut last = 0usize;
    for i in 1..transitions.len() {
        let gap = transitions[i] - transitions[i - 1];
        if !(0.55..=1.45).contains(&gap) {
            break;
        }
        last = i;
    }
    // An odd index is a light -> dark change, which cannot be the symbol edge:
    // drop back to the previous one (a dark speck in the quiet zone).
    if last % 2 == 1 {
        last -= 1;
    }
    let dim = 7 + last as u32;
    MicroVersion::from_size(dim)?;
    Some((dim, transitions[last]))
}

/// Counts function modules that disagree with what a micro symbol of `version`
/// must contain: the 7x7 finder, its one-module separator along row 7 and
/// column 7, and the timing patterns filling the rest of row 0 and column 0.
///
/// The format information is deliberately excluded — it is symbol-dependent, and
/// checking it here would duplicate what `decode_micro` does with error
/// correction.
fn function_mismatches(matrix: &BitMatrix, version: MicroVersion) -> u32 {
    let size = version.size();
    let mut bad = 0;
    for y in 0..8 {
        for x in 0..8 {
            let expect = if x == 7 || y == 7 {
                false // separator
            } else {
                x == 0
                    || x == 6
                    || y == 0
                    || y == 6
                    || ((2..=4).contains(&x) && (2..=4).contains(&y))
            };
            if matrix.get(x, y) != expect {
                bad += 1;
            }
        }
    }
    for k in 8..size {
        debug_assert!(micro_is_function_module(version, k, 0));
        if matrix.get(k, 0) != (k % 2 == 0) {
            bad += 1;
        }
        if matrix.get(0, k) != (k % 2 == 0) {
            bad += 1;
        }
    }
    bad
}

// ---------------------------------------------------------------------------
// Small vector helpers. `Point` doubles as a vector here; the module works in
// symbol-axis coordinates throughout, so naming a separate type would only add
// conversions.
// ---------------------------------------------------------------------------

/// Image-space point at grid coordinate `t` along `along` and `s` along
/// `across`, measured in modules from the symbol's origin corner.
#[inline]
fn grid_point(origin: Point, along: Point, across: Point, module: f64, t: f64, s: f64) -> Point {
    Point::new(
        origin.x + module * (t * along.x + s * across.x),
        origin.y + module * (t * along.y + s * across.y),
    )
}

#[inline]
fn add(a: Point, b: Point) -> Point {
    Point::new(a.x + b.x, a.y + b.y)
}

#[inline]
fn sub(a: Point, b: Point) -> Point {
    Point::new(a.x - b.x, a.y - b.y)
}

#[inline]
fn mul(a: Point, k: f64) -> Point {
    Point::new(a.x * k, a.y * k)
}

#[inline]
fn neg(a: Point) -> Point {
    Point::new(-a.x, -a.y)
}

#[inline]
fn len(a: Point) -> f64 {
    (a.x * a.x + a.y * a.y).sqrt()
}

#[inline]
fn dot(a: Point, b: Point) -> f64 {
    a.x * b.x + a.y * b.y
}

#[inline]
fn cross2(a: Point, b: Point) -> f64 {
    a.x * b.y - a.y * b.x
}

/// Rotates a vector by `angle` radians (clockwise on screen, since `y` grows
/// downward).
#[inline]
fn rot(a: Point, angle: f64) -> Point {
    let (s, c) = angle.sin_cos();
    Point::new(a.x * c - a.y * s, a.x * s + a.y * c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_right_handed() {
        // (u, v) must have the handedness of (rightward, downward).
        let u = Point::new(1.0, 0.0);
        let v = Point::new(0.0, 1.0);
        assert!(cross2(u, v) > 0.0);
        assert!(cross2(v, neg(u)) > 0.0);
        assert!(cross2(neg(u), neg(v)) > 0.0);
        assert!(cross2(neg(v), u) > 0.0);
    }

    #[test]
    fn rotation_is_positive_toward_positive_y() {
        let r = rot(Point::new(1.0, 0.0), core::f64::consts::FRAC_PI_2);
        assert!((r.x - 0.0).abs() < 1e-12 && (r.y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn grid_point_places_module_centres() {
        let o = Point::new(10.0, 20.0);
        let p = grid_point(o, Point::new(1.0, 0.0), Point::new(0.0, 1.0), 4.0, 2.5, 0.5);
        assert_eq!(p, Point::new(20.0, 22.0));
    }
}
