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
//! 5. **Perspective refinement.** That affine map is only accepted if it puts
//!    every timing boundary where the walk measured it. When it does not — the
//!    symbol is tilted — the symbol's far corner is searched for and a genuine
//!    four-point homography replaces it. See [`refine_bottom_right`].
//! 6. **Validation.** The sampled grid must actually contain a micro symbol's
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

/// Grid coordinate of the first boundary a timing walk reports: the separator
/// edge at grid 7. Entry `i` of a walk is the boundary at grid `7 + i`, and the
/// last entry is the symbol's outer edge at grid `dim`.
const FIRST_TIMING_BOUNDARY: f64 = 7.0;

/// How far, in *pixels*, the affine map may misplace a measured timing boundary
/// before the perspective refinement is run.
///
/// This is the switch that keeps an untilted symbol on the fast path, and it is
/// in pixels rather than in modules because that is where the noise it has to
/// clear comes from: a timing boundary is a binarized edge, so it is located to
/// about half a pixel however large the modules are. Measured over every upright
/// fixture in the suite, from 3-pixel to 20-pixel modules, the worst affine
/// residual is 0.26 pixels — and it does scale with the pixel, not the module:
/// the same figure in module units ranges over 0.013 to 0.030. 0.45 keeps every
/// one of them on the fast path with 70% of margin.
///
/// Rotated symbols at small module sizes do overshoot it — resampling makes
/// their module edges ragged, and at 3 pixels per module that is worth 1.3
/// pixels — so they pay for a refinement they did not need. That is a cost, not
/// a risk: nothing the refinement produces is adopted unless it samples at least
/// as well as the affine map it would replace.
const AFFINE_RESIDUAL_TOLERANCE_PX: f64 = 0.45;

/// Half-width of the first stage of the timing-fit search, as a fraction of the
/// symbol's span. The affine corner is displaced from the true one by several
/// modules under the tilts this exists to handle — 7.4 modules for an M4 under a
/// 25% keystone — so the first stage has to be coarse and wide.
const FIT_RANGE: f64 = 0.5;

/// Lattice points per axis in one search stage, for both stages of the search.
/// Odd, so every stage re-tests its own incumbent and neither search can move
/// away from a candidate it has already judged better.
const FIT_LATTICE: usize = 7;

/// Stages of the timing-fit search. Each stage searches +/- one step of the
/// previous one, so the resolution improves threefold per stage: after four the
/// lattice is `FIT_RANGE / 27` of a span, a tenth of a module on an M4.
///
/// Worst case is `FIT_STAGES * FIT_LATTICE^2` = 196 candidate homographies,
/// scored on the 10 to 22 boundaries the walks already measured — arithmetic
/// only, not one image read.
const FIT_STAGES: usize = 4;

/// Half-width of the first stage of the probe search, in modules. The timing fit
/// lands within about a module of the true corner, so this is deliberately
/// small: it is what keeps the probe score, which knows nothing about what the
/// symbol should contain, from wandering off into the quiet zone.
const PROBE_RANGE: f64 = 1.6;

/// Lattice points per axis and stages of the probe search: 3 stages of 5x5 = 75
/// candidates, ending at a resolution of a fifth of a module.
const PROBE_LATTICE: usize = 5;
const PROBE_STAGES: usize = 3;

/// How far from a module centre the probes sit, in modules. Far enough that half
/// a module of grid error pushes them over the boundary into the neighbouring
/// module, close enough that a ragged binarized edge does not.
const PROBE_OFFSET: f64 = 0.32;

/// How many modules of the ring just outside the symbol may be dark before a
/// refined map is rejected.
///
/// ISO/IEC 18004 6.3.6 requires a two-module quiet zone, so under the right map
/// that ring is entirely light; under a map that has slipped a whole module — or
/// fitted an 11-module grid inside a 17-module symbol, which is what a timing
/// walk that under-counts under a steep tilt produces — it runs straight through
/// the data area and picks up dark modules by the handful. Two is enough slack
/// for a ragged edge and nowhere near enough to hide that.
const MAX_QUIET_ZONE_DARK: u32 = 2;

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
/// refines it for perspective if the affine map does not explain the walks,
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

    let row = walk_timing(bin, origin, u, v, module)?;
    let col = walk_timing(bin, origin, v, u, module)?;
    if walk_dim(&row) != walk_dim(&col) {
        return None;
    }
    let dim = walk_dim(&row);
    let version = MicroVersion::from_size(dim)?;
    let dim_f = dim as f64;

    // Two measured points on the symbol's far edges, at the outer end of each
    // timing pattern: grid (dim, 0.5) and grid (0.5, dim).
    let p_row = grid_point(origin, u, v, module, walk_edge(&row), 0.5);
    let p_col = grid_point(origin, v, u, module, walk_edge(&col), 0.5);
    // Three correspondences fix an affine map; the fourth is the parallelogram
    // completion in both spaces, so `quad_to_quad` reproduces exactly that map.
    let src = [
        Point::new(3.5, 3.5),
        Point::new(dim_f, 0.5),
        Point::new(dim_f - 3.0, dim_f - 3.0),
        Point::new(0.5, dim_f),
    ];
    let dst = [center, p_row, sub(add(p_row, p_col), center), p_col];
    let affine = PerspectiveTransform::quad_to_quad(src, dst);

    // Every boundary the walks crossed is a measurement of where the grid lines
    // are, and the map that is the right map reproduces all of them. Where an
    // affine map cannot, what is left over is perspective.
    let fits = [
        TimingFit::new(origin, u, v, module, row, true),
        TimingFit::new(origin, v, u, module, col, false),
    ];
    let affine_worst = timing_residuals(&affine, dim_f, &fits)?.1;

    if affine_worst * module <= AFFINE_RESIDUAL_TOLERANCE_PX {
        let mismatches = function_mismatches(bin, &affine, version);
        if mismatches > MAX_FUNCTION_MISMATCHES {
            return None;
        }
        return Some((mismatches, sample_symbol(bin, &affine, dim)));
    }

    note_refinement();
    let refined = refine_bottom_right(bin, &affine, center, p_row, p_col, dim_f, module, &fits);
    // The refinement is adopted only if it samples at least as well as the map
    // it would replace. Both searches inside it are monotone, but they optimise
    // proxies — two lines of the symbol, then a corner's worth of module centres
    // — and a frame that did not need refining at all can reach here on nothing
    // but binarisation noise. Ranking the two by the same two scores that decide
    // everything else is what makes that harmless.
    // The quiet-zone check gates the refinement only. The affine map is exactly
    // what it always was, and what it does with a binarisation that has already
    // failed is settled behaviour the retry pass is built around (see the crate
    // docs); the refinement is the new freedom, so it is the one that has to
    // prove it is still looking at the same symbol.
    // Affine first: `min_by_key` keeps the *first* minimum, so on a tie the
    // incumbent holds the frame. Ties are not exotic — rotated symbols at small
    // module sizes enter the refinement without needing it and then tie at
    // (0, 0) — and nothing has been proven against the settled map in one.
    let candidates: Vec<PerspectiveTransform> =
        if quiet_zone_dark(bin, &refined, dim) <= MAX_QUIET_ZONE_DARK {
            vec![affine, refined]
        } else {
            vec![affine]
        };
    let (mismatches, transform) = candidates
        .into_iter()
        .map(|t| {
            (
                (
                    function_mismatches(bin, &t, version),
                    grid_disagreement(bin, &t, dim),
                ),
                t,
            )
        })
        .min_by_key(|&(score, _)| score)
        .map(|((m, _), t)| (m, t))
        .expect("the affine map is always a candidate");
    if mismatches > MAX_FUNCTION_MISMATCHES {
        return None;
    }
    Some((mismatches, sample_symbol(bin, &transform, dim)))
}

/// Samples every module of a `dim`-sized symbol through `transform`.
fn sample_symbol(bin: &Binary, transform: &PerspectiveTransform, dim: u32) -> MicroSampled {
    let mut matrix = BitMatrix::new(dim);
    for row in 0..dim {
        for col in 0..dim {
            let p = transform.map(col as f64 + 0.5, row as f64 + 0.5);
            matrix.set(col, row, sample_at(bin, p));
        }
    }
    let dim_f = dim as f64;
    let corners = [
        transform.map(0.0, 0.0),
        transform.map(dim_f, 0.0),
        transform.map(dim_f, dim_f),
        transform.map(0.0, dim_f),
    ];
    MicroSampled { matrix, corners }
}

/// One timing walk, reduced to what a perspective fit needs: the image-space ray
/// it was measured along, and the boundary positions along that ray.
///
/// `origin` is the ray at parameter zero and `dir` the image displacement per
/// unit of parameter, so a measurement `t` is the image point
/// `origin + t * dir`. `along_x` says which of the two timing patterns this is:
/// the one filling row 0, along which the grid's x coordinate varies, or the one
/// filling column 0.
struct TimingFit {
    origin: Point,
    dir: Point,
    transitions: Vec<f64>,
    along_x: bool,
}

impl TimingFit {
    fn new(
        grid_origin: Point,
        along: Point,
        across: Point,
        module: f64,
        transitions: Vec<f64>,
        along_x: bool,
    ) -> TimingFit {
        TimingFit {
            origin: grid_point(grid_origin, along, across, module, 0.0, 0.5),
            dir: mul(along, module),
            transitions,
            along_x,
        }
    }
}

/// Scores a candidate grid->image map against the measured timing boundaries,
/// returning `(sum of squared residuals, worst absolute residual)` in modules.
///
/// A boundary measured at ray position `t` asserts exactly one thing: the image
/// of the grid line through that boundary crosses the ray there. So the
/// candidate's own image of that grid line is intersected with the ray and the
/// two positions compared. Intersecting, rather than comparing against the
/// module centre `map(k, 0.5)`, is what makes the residual mean only that: the
/// ray is the *initial* estimate's row and drifts across the true one under
/// tilt, and that drift would otherwise enter the residual as a bias exactly
/// where the signal is.
///
/// `None` if a candidate maps a grid line parallel to the ray, which no symbol
/// does and which would make the intersection meaningless.
fn timing_residuals(
    transform: &PerspectiveTransform,
    dim_f: f64,
    fits: &[TimingFit; 2],
) -> Option<(f64, f64)> {
    let mut sse = 0.0;
    let mut worst: f64 = 0.0;
    for fit in fits {
        for (i, &measured) in fit.transitions.iter().enumerate() {
            let k = FIRST_TIMING_BOUNDARY + i as f64;
            let (p0, p1) = if fit.along_x {
                (transform.map(k, 0.0), transform.map(k, dim_f))
            } else {
                (transform.map(0.0, k), transform.map(dim_f, k))
            };
            let edge = sub(p1, p0);
            let denom = cross2(fit.dir, edge);
            if denom.abs() < 1e-9 {
                return None;
            }
            let predicted = cross2(sub(p0, fit.origin), edge) / denom;
            if !predicted.is_finite() {
                return None;
            }
            let residual = predicted - measured;
            sse += residual * residual;
            worst = worst.max(residual.abs());
        }
    }
    Some((sse, worst))
}

/// Searches for the symbol's bottom-right corner and returns the four-point
/// homography it implies.
///
/// A micro symbol has no alignment pattern, so three of its four corners are all
/// that can be measured directly, and the fourth — the one furthest from the
/// finder, where a tilt does the most damage — has to be inferred. The affine
/// map infers it as the parallelogram completion, and that assumption is what
/// caps the unrefined envelope at a 4% keystone.
///
/// Two searches run, in this order, because they fail in opposite ways:
///
/// 1. **The timing fit** minimises disagreement with the measured timing
///    boundaries. It costs no image reads at all and it is unconditionally safe
///    — the timing patterns are function patterns, so their measurements cannot
///    be wrong about what they are — but it is weakly conditioned. Both walks
///    project onto a ray, and a projection cannot see where the grid lines sit
///    *across* that ray; measured on this suite it lands within about a module
///    of the corner and no closer. That is enough to bring the corner into
///    reach, and not enough to sample by.
/// 2. **The probe search** minimises [`grid_disagreement`] over the far quadrant,
///    which measures the one thing that actually matters — whether module centres
///    land on module centres — and measures it right where the corner is. It
///    reads the image, and it knows nothing about what the symbol should contain,
///    so it is run last and over a range of barely more than a module, where the
///    timing fit has already established roughly where the corner is.
///
/// Scoring the first search on function-pattern agreement instead, which is
/// cheap because those modules are sampled anyway, does not work, and it is
/// worth recording why: every function module of a micro symbol lies in row 0,
/// column 0 or the finder block, all of them clustered at the corner the search
/// is *not* moving. Measured on a 17-module symbol under a 15% keystone,
/// function-pattern agreement is already perfect with the corner still half of
/// its error away, at which point 90 of the 172 data modules still sample the
/// wrong cell. It is a sound accept/reject gate, which is what it stays; it is
/// not a usable objective.
#[allow(clippy::too_many_arguments)]
fn refine_bottom_right(
    bin: &Binary,
    affine: &PerspectiveTransform,
    center: Point,
    p_row: Point,
    p_col: Point,
    dim_f: f64,
    module: f64,
    fits: &[TimingFit; 2],
) -> PerspectiveTransform {
    // Unlike the affine quad, this one names the symbol's actual far corner, so
    // a candidate is exactly "where the bottom-right corner is".
    let src = [
        Point::new(3.5, 3.5),
        Point::new(dim_f, 0.5),
        Point::new(dim_f, dim_f),
        Point::new(0.5, dim_f),
    ];
    let build = |br: Point| PerspectiveTransform::quad_to_quad(src, [center, p_row, br, p_col]);

    let start = affine.map(dim_f, dim_f);
    let Some((start_sse, _)) = timing_residuals(&build(start), dim_f, fits) else {
        return *affine;
    };
    let fitted = search(
        start,
        FIT_RANGE * module * dim_f,
        FIT_LATTICE,
        FIT_STAGES,
        start_sse,
        |br| timing_residuals(&build(br), dim_f, fits).map(|(sse, _)| sse),
    );

    let dim = dim_f as u32;
    let probed = search(
        fitted,
        PROBE_RANGE * module,
        PROBE_LATTICE,
        PROBE_STAGES,
        grid_disagreement(bin, &build(fitted), dim),
        |br| Some(grid_disagreement(bin, &build(br), dim)),
    );
    build(probed)
}

/// Coarse-to-fine lattice search for the point minimising `score`, starting from
/// `start` with a half-width of `radius` and narrowing to one step per stage.
///
/// Every stage re-tests its own incumbent, so the returned point never scores
/// worse than `start` did.
///
/// **The returned point never leaves `radius` of `start` on either axis.** That
/// is not automatic and it is the whole reason for the clamp: each stage
/// recentres on its own incumbent *before* narrowing, so the reachable set is
/// otherwise the sum of the stage radii, `radius * sum(q^i)` for
/// `q = 2 / (lattice - 1)`. Unclamped that is 2.8 modules for the probe search's
/// 1.6 (three stages, `q = 1/2`) and 0.741 of a span for the timing fit's 0.5
/// (four stages, `q = 1/3`).
///
/// Both callers document a bound, and for one of them — the probe search, scored
/// by [`grid_disagreement`], which knows nothing about what the symbol should
/// contain — that bound *is* the argument that it cannot wander into the quiet
/// zone. Clamping here makes the argument true by construction, and keeps it
/// true if someone later adds a stage.
fn search<S: PartialOrd + Copy>(
    start: Point,
    radius: f64,
    lattice: usize,
    stages: usize,
    start_score: S,
    score: impl Fn(Point) -> Option<S>,
) -> Point {
    let bound = radius;
    let mut best = start;
    let mut best_score = start_score;
    let mut radius = radius;
    for _ in 0..stages {
        let step = 2.0 * radius / (lattice - 1) as f64;
        let (mut stage_best, mut stage_score) = (best, best_score);
        for iy in 0..lattice {
            for ix in 0..lattice {
                let cand = Point::new(
                    (best.x - radius + ix as f64 * step).clamp(start.x - bound, start.x + bound),
                    (best.y - radius + iy as f64 * step).clamp(start.y - bound, start.y + bound),
                );
                if let Some(s) = score(cand)
                    && s < stage_score
                {
                    stage_score = s;
                    stage_best = cand;
                }
            }
        }
        best = stage_best;
        best_score = stage_score;
        radius = step;
    }
    best
}

/// Counts probe points that disagree with the module centre they belong to, over
/// the quadrant of the symbol furthest from the finder.
///
/// A grid that lands on module centres reads the same colour a third of a module
/// either side of every centre; a grid that is half a module out straddles the
/// boundary and reads the neighbour instead. Unlike the function-pattern score
/// this needs no knowledge of what a module should contain, which is what makes
/// it usable in the data area — and the far quadrant is both where a
/// bottom-right corner error shows up and where the function patterns say
/// nothing at all.
fn grid_disagreement(bin: &Binary, transform: &PerspectiveTransform, dim: u32) -> u32 {
    let half = dim / 2;
    let mut bad = 0;
    for y in half..dim {
        for x in half..dim {
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            let center = sample_at(bin, transform.map(cx, cy));
            for (dx, dy) in [
                (PROBE_OFFSET, 0.0),
                (-PROBE_OFFSET, 0.0),
                (0.0, PROBE_OFFSET),
                (0.0, -PROBE_OFFSET),
            ] {
                if sample_at(bin, transform.map(cx + dx, cy + dy)) != center {
                    bad += 1;
                }
            }
        }
    }
    bad
}

/// Walks one timing pattern outward from the finder and returns the measured
/// position of every module boundary along it, in the ray's own grid units:
/// entry `i` is the boundary at grid `7 + i`, and the last entry is the symbol's
/// outer edge (nominally the dimension itself; the measured value absorbs the
/// error in the initial module estimate).
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
///
/// The interior boundaries used to be discarded once they had been counted. They
/// are kept now because they are the only measurement of the symbol's
/// perspective there is — see [`refine_bottom_right`].
fn walk_timing(
    bin: &Binary,
    origin: Point,
    along: Point,
    across: Point,
    module: f64,
) -> Option<Vec<f64>> {
    const STEP: f64 = 0.02;
    const START: f64 = 6.5;
    const END: f64 = 20.5;

    let sample = |t: f64| sample_at(bin, grid_point(origin, along, across, module, t, 0.5));

    if !sample(START) {
        // Not inside the finder's outer ring: the frame is wrong or the module
        // estimate is badly off.
        return None;
    }
    // Distances here are in modules, so the speck threshold is too.
    let mut transitions = ray_transitions(sample, START, END, STEP, 0.35);

    // The separator edge anchors the walk.
    if transitions.is_empty() || (transitions[0] - 7.0).abs() > 0.6 {
        return None;
    }
    // The module size along the ray is not constant: under perspective the far
    // end of a timing pattern is foreshortened or stretched relative to the
    // finder that set the ray's scale, by up to a factor of two over a 17-module
    // symbol. A fixed band ends the walk part way along and under-counts the
    // dimension, which is what used to cap M4 at a 12% keystone. So the band
    // tracks the spacing actually seen.
    //
    // Be clear about what that trades. Per step, against its own drifting
    // reference, this is the same +/-45% the fixed band allowed. In *absolute*
    // terms it is strictly wider: the EMA converges on whatever the gaps are
    // doing, so the accepted absolute band converges towards the outer clamp,
    // [0.5, 2.0), against the old fixed [0.55, 1.45]. Three gaps at the ceiling
    // take `scale` to 1.39, and an M4 walk has ten of them. Measured over ~810
    // damaged-timing warps (the KG-13 surface: one broken timing module x
    // {M3, M4} x every k x three module sizes x five tilts x three keystones)
    // that widening reports 2 more smaller-crop detections and 1 fewer, all of
    // them KG-13-class and already logged. That is the price of M4 12% -> 14%
    // and M2/M3 17% -> 23%.
    //
    // The outer clamp is exclusive at 2.0 for a reason that is not about phantom
    // safety at all: a gap of exactly two modules is one *bridged* boundary, and
    // `timing_residuals` maps entry `i` to grid `7 + i`. Walking through one
    // would silently renumber every measurement the refinement then fits to.
    let mut scale = 1.0f64;
    let mut last = 0usize;
    for i in 1..transitions.len() {
        let gap = transitions[i] - transitions[i - 1];
        if !(0.55 * scale..=1.45 * scale).contains(&gap) || !(0.5..2.0).contains(&gap) {
            break;
        }
        last = i;
        scale = 0.5 * (scale + gap);
    }
    // An odd index is a light -> dark change, which cannot be the symbol edge:
    // drop back to the previous one (a dark speck in the quiet zone).
    if last % 2 == 1 {
        last -= 1;
    }
    transitions.truncate(last + 1);
    MicroVersion::from_size(walk_dim(&transitions))?;
    Some(transitions)
}

/// Symbol dimension implied by a timing walk: its boundaries run from grid 7 to
/// grid `dim` inclusive.
#[inline]
fn walk_dim(transitions: &[f64]) -> u32 {
    FIRST_TIMING_BOUNDARY as u32 + transitions.len() as u32 - 1
}

/// Measured position of the symbol's outer edge along a timing walk.
#[inline]
fn walk_edge(transitions: &[f64]) -> f64 {
    transitions[transitions.len() - 1]
}

/// Every function module of a micro symbol of `version`, as
/// `(column, row, expected darkness)`: the 7x7 finder, its one-module separator
/// along row 7 and column 7, and the timing patterns filling the rest of row 0
/// and column 0.
///
/// The format information is deliberately excluded — it is symbol-dependent, and
/// checking it here would duplicate what `decode_micro` does with error
/// correction.
fn function_modules(version: MicroVersion) -> impl Iterator<Item = (u32, u32, bool)> {
    let block = (0..8u32).flat_map(|y| {
        (0..8u32).map(move |x| {
            let expect = if x == 7 || y == 7 {
                false // separator
            } else {
                x == 0
                    || x == 6
                    || y == 0
                    || y == 6
                    || ((2..=4).contains(&x) && (2..=4).contains(&y))
            };
            (x, y, expect)
        })
    });
    let timing = (8..version.size()).flat_map(|k| [(k, 0, k % 2 == 0), (0, k, k % 2 == 0)]);
    block.chain(timing)
}

/// Counts the function modules a candidate grid->image map samples wrongly.
///
/// Only the function modules are read, which is what makes this affordable as a
/// gate ahead of the full sample and as a tie-break between two candidate maps:
/// 68 to 82 module reads against the 121 to 289 of a whole symbol.
fn function_mismatches(
    bin: &Binary,
    transform: &PerspectiveTransform,
    version: MicroVersion,
) -> u32 {
    let mut bad = 0;
    for (x, y, expect) in function_modules(version) {
        debug_assert!(x < 8 && y < 8 || micro_is_function_module(version, x, y));
        if sample_at(bin, transform.map(x as f64 + 0.5, y as f64 + 0.5)) != expect {
            bad += 1;
        }
    }
    bad
}

/// Counts dark modules in the ring immediately outside the symbol, half a module
/// clear of its edge.
///
/// This is the check that a refined map is a map of *this* symbol rather than a
/// smaller grid fitted inside it. The refinement is free to move the far corner
/// by a couple of modules and the function patterns, all of them clustered at
/// the near corner, would not notice; the quiet zone is the one piece of
/// evidence that surrounds the whole symbol.
fn quiet_zone_dark(bin: &Binary, transform: &PerspectiveTransform, dim: u32) -> u32 {
    let dim_f = dim as f64;
    let mut dark = 0;
    for k in 0..dim {
        let c = k as f64 + 0.5;
        for p in [
            transform.map(c, -0.5),
            transform.map(c, dim_f + 0.5),
            transform.map(-0.5, c),
            transform.map(dim_f + 0.5, c),
        ] {
            if sample_at(bin, p) {
                dark += 1;
            }
        }
    }
    dark
}

/// Reads the binarized image at a sub-pixel point, rounding to the nearest
/// pixel.
#[inline]
fn sample_at(bin: &Binary, p: Point) -> bool {
    bin.get(p.x.round() as i64, p.y.round() as i64)
}

/// Records that a frame needed perspective refinement.
///
/// Compiled into the test build only. The untilted fast path is load-bearing —
/// an unnecessary refinement is silent, so nothing else would catch it
/// regressing — and this is the only way to assert it directly.
#[cfg(test)]
fn note_refinement() {
    REFINEMENTS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
static REFINEMENTS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

#[cfg(not(test))]
#[inline]
fn note_refinement() {}

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
    use crate::binarize::{PRIMARY_WINDOW_DIV, binarize};
    use crate::finder::find_finders;
    use core::sync::atomic::Ordering;

    /// A structurally valid micro symbol: correct function patterns, arbitrary
    /// but deterministic data. Detection never inspects the codeword stream, so
    /// the data only has to be there.
    fn micro_symbol(version: MicroVersion) -> BitMatrix {
        let size = version.size();
        let mut m = BitMatrix::new(size);
        for (x, y, dark) in function_modules(version) {
            m.set(x, y, dark);
        }
        let mut state = 0x2468_ace0u32;
        for y in 0..size {
            for x in 0..size {
                if micro_is_function_module(version, x, y) {
                    continue;
                }
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                m.set(x, y, (state >> 20) & 1 == 1);
            }
        }
        m
    }

    /// Renders `m` upright at `scale` pixels per module inside a four-module
    /// quiet zone, returning the luma buffer and its side length.
    fn render_upright(m: &BitMatrix, scale: u32) -> (Vec<u8>, u32) {
        const QUIET: u32 = 4;
        let side = (m.size() + 2 * QUIET) * scale;
        let mut luma = vec![255u8; (side * side) as usize];
        for y in 0..m.size() {
            for x in 0..m.size() {
                if !m.get(x, y) {
                    continue;
                }
                for py in 0..scale {
                    for px in 0..scale {
                        let ix = (QUIET + x) * scale + px;
                        let iy = (QUIET + y) * scale + py;
                        luma[(iy * side + ix) as usize] = 0;
                    }
                }
            }
        }
        (luma, side)
    }

    /// The untilted fast path and its counterpart, asserted directly rather than
    /// inferred, and in one test because [`REFINEMENTS`] is process-global and
    /// the harness is parallel.
    ///
    /// That is only safe because this is the sole *unit* test in the crate that
    /// reaches `detect_from_finder` — every other `#[cfg(test)]` module here
    /// stops well short of it. **A second one would make this intermittently
    /// flaky**, so anyone adding one should fold their case in here or put both
    /// behind a shared lock.
    ///
    /// An unnecessary refinement is silent — it costs time and changes nothing
    /// visible — so no end-to-end test can see it. This one counts. It is also
    /// the only test that can: [`REFINEMENTS`] is crate-internal, and the
    /// integration suites link the library compiled without `cfg(test)`, where
    /// the counter does not exist at all.
    ///
    /// The second half is what stops the first from passing vacuously, by making
    /// the refinement unreachable.
    #[test]
    fn the_refinement_runs_when_tilted_and_not_when_upright() {
        let before = REFINEMENTS.load(Ordering::Relaxed);
        for version in MicroVersion::ALL {
            for scale in [4u32, 8, 12, 20] {
                let m = micro_symbol(version);
                let (luma, side) = render_upright(&m, scale);
                let bin = binarize(&luma, side, side, PRIMARY_WINDOW_DIV).expect("binarize");
                let finders = find_finders(&bin);
                assert_eq!(finders.len(), 1, "{version:?} at {scale} px: finders");
                let sampled =
                    detect_from_finder(&bin, &finders[0]).expect("upright symbol must detect");
                assert_eq!(sampled.matrix.size(), m.size(), "{version:?} at {scale} px");
                for y in 0..m.size() {
                    for x in 0..m.size() {
                        assert_eq!(
                            sampled.matrix.get(x, y),
                            m.get(x, y),
                            "{version:?} at {scale} px: module ({x}, {y})"
                        );
                    }
                }
            }
        }
        assert_eq!(
            REFINEMENTS.load(Ordering::Relaxed),
            before,
            "an upright symbol took the perspective refinement path"
        );

        // Now a symbol that is not a parallelogram in the image: a shear that
        // grows with x, which no affine map built from the finder's own axes
        // reproduces.
        let m = micro_symbol(MicroVersion::M3);
        let (luma, side) = render_upright(&m, 12);
        let mut sheared = vec![255u8; luma.len()];
        for y in 0..side {
            for x in 0..side {
                let sy = y as i64 + (x as i64 - side as i64 / 2) * (y as i64) / (5 * side as i64);
                if (0..side as i64).contains(&sy) {
                    sheared[(y * side + x) as usize] = luma[(sy as u32 * side + x) as usize];
                }
            }
        }
        let bin = binarize(&sheared, side, side, PRIMARY_WINDOW_DIV).expect("binarize");
        let before = REFINEMENTS.load(Ordering::Relaxed);
        for f in &find_finders(&bin) {
            let _ = detect_from_finder(&bin, f);
        }
        assert!(
            REFINEMENTS.load(Ordering::Relaxed) > before,
            "a distorted symbol never reached the refinement"
        );
    }

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
