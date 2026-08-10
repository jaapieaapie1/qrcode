//! Groups located finder patterns into QR symbol candidates.
//!
//! For each set of three finders we identify the right-angle corner (the
//! top-left finder), order the other two as top-right / bottom-left by
//! handedness, score the geometry (square, right-angled, consistent module
//! size), then greedily accept non-overlapping triples so multiple codes in one
//! image are handled.

use crate::finder::Finder;
use crate::geometry::Point;

/// A grouped QR symbol candidate: three finder centres plus derived metrics.
#[derive(Clone, Copy, Debug)]
pub struct QrCandidate {
    /// Top-left finder centre (the right-angle corner).
    pub tl: Point,
    /// Top-right finder centre.
    pub tr: Point,
    /// Bottom-left finder centre.
    pub bl: Point,
    /// Estimated symbol side length in modules (a valid QR dimension).
    pub dimension: u32,
}

/// Ordered corners of a triple with a match score (lower is better).
struct Scored {
    idx: [usize; 3],
    cand: QrCandidate,
    score: f64,
}

fn cross(o: Point, a: Point, b: Point) -> f64 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

/// Orders three points as (top-left, top-right, bottom-left).
fn order_corners(a: Point, b: Point, c: Point) -> (Point, Point, Point) {
    let ab = a.distance_sq(b);
    let bc = b.distance_sq(c);
    let ca = c.distance_sq(a);
    // The top-left finder is the vertex NOT on the longest edge (hypotenuse).
    let (tl, p, q) = if ab >= bc && ab >= ca {
        (c, a, b)
    } else if bc >= ab && bc >= ca {
        (a, b, c)
    } else {
        (b, c, a)
    };
    // With y growing downward, an upright symbol has a positive cross product
    // for (TR-TL) x (BL-TL).
    if cross(tl, p, q) >= 0.0 {
        (tl, p, q)
    } else {
        (tl, q, p)
    }
}

/// Snaps a raw dimension estimate to the nearest valid QR side length
/// (`d ≡ 1 mod 4`, `21..=177`). Returns `None` if hopelessly out of range.
fn snap_dimension(raw: f64) -> Option<u32> {
    if !raw.is_finite() || raw < 17.0 {
        return None;
    }
    let mut d = raw.round() as i64;
    match d.rem_euclid(4) {
        0 => d += 1,
        2 => d -= 1,
        3 => d += 2, // -> ≡1
        _ => {}
    }
    if (21..=177).contains(&d) {
        Some(d as u32)
    } else {
        None
    }
}

/// Converts an axis-aligned module measurement into a true, rotation-invariant
/// module size, given the symbol's own unit axis vectors `u` (top-left ->
/// top-right) and `v` (top-left -> bottom-left).
///
/// [`Finder::module`](crate::finder::Finder::module) is derived from horizontal
/// and vertical *scanline run lengths*. A horizontal ray from the centre of a
/// finder rotated by `θ` leaves the concentric square rings at
/// `a / max(|cos θ|, |sin θ|)` rather than `a`, so every run — and therefore the
/// module estimate — is stretched by `1 / max(|cos θ|, |sin θ|)`. The same holds
/// for the vertical profile. Multiplying by that factor undoes the stretch
/// exactly; it folds at 45 degrees, matching the observed behaviour of the
/// uncorrected estimate.
///
/// `max(|u.x|, |u.y|)` *is* `max(|cos θ|, |sin θ|)` for the symbol's x axis, and
/// likewise for `v`; both are equal under a pure rotation, so averaging them
/// costs nothing there and averages out mild shear/perspective. The factor is in
/// `(0.707, 1.0]`, so an upright symbol is left untouched.
fn axis_scale(u: (f64, f64), v: (f64, f64)) -> f64 {
    (u.0.abs().max(u.1.abs()) + v.0.abs().max(v.1.abs())) / 2.0
}

/// Builds a scored candidate from three finders, or `None` if the geometry is
/// not plausibly a QR symbol.
fn score_triple(finders: &[Finder], i: usize, j: usize, k: usize) -> Option<Scored> {
    let (fi, fj, fk) = (finders[i], finders[j], finders[k]);
    let (tl, tr, bl) = order_corners(fi.center, fj.center, fk.center);

    let leg_top = tl.distance(tr);
    let leg_left = tl.distance(bl);
    if leg_top < 1.0 || leg_left < 1.0 {
        return None;
    }

    // Legs must be similar (square symbol).
    let leg_ratio = leg_top / leg_left;
    if !(0.7..=1.4).contains(&leg_ratio) {
        return None;
    }

    // Unit vectors along the symbol's own axes: `u` = top-left -> top-right,
    // `v` = top-left -> bottom-left.
    let u = ((tr.x - tl.x) / leg_top, (tr.y - tl.y) / leg_top);
    let v = ((bl.x - tl.x) / leg_left, (bl.y - tl.y) / leg_left);

    // Right angle at the top-left corner: normalised |cos| must be small.
    let cos = u.0 * v.0 + u.1 * v.1;
    if cos.abs() > 0.30 {
        return None;
    }

    // Module sizes must agree across the three finders.
    let modules = [fi.module, fj.module, fk.module];
    let m_min = modules.iter().cloned().fold(f64::MAX, f64::min);
    let m_max = modules.iter().cloned().fold(0.0, f64::max);
    if m_max > m_min * 2.0 {
        return None;
    }
    // `Finder.module` is measured from *axis-aligned* scanline runs, so it is
    // inflated by rotation; the legs are Euclidean distances and are not.
    // Rescale the module before mixing the two (see `axis_scale`).
    let module = ((fi.module + fj.module + fk.module) / 3.0) * axis_scale(u, v);

    // Dimension from centre-to-centre leg length: finder centres are 3.5 modules
    // in from each edge, so their separation spans `dimension - 7` modules.
    let raw_dim = ((leg_top + leg_left) / 2.0) / module + 7.0;
    let dimension = snap_dimension(raw_dim)?;

    // The snapped dimension should be consistent with the measured legs.
    let expected_leg = (dimension as f64 - 7.0) * module;
    let leg_err =
        ((leg_top - expected_leg).abs() + (leg_left - expected_leg).abs()) / (2.0 * expected_leg);
    if leg_err > 0.20 {
        return None;
    }

    // Finders confirmed by more scanlines are more trustworthy; fold that in.
    let min_votes = fi.votes.min(fj.votes).min(fk.votes).max(1) as f64;
    let score =
        cos.abs() + (leg_ratio - 1.0).abs() + leg_err + (m_max / m_min - 1.0) + 1.0 / min_votes;
    Some(Scored {
        idx: [i, j, k],
        cand: QrCandidate {
            tl,
            tr,
            bl,
            dimension,
        },
        score,
    })
}

/// Expresses `p` in `cand`'s own affine frame: `(0, 0)` at the top-left finder
/// centre, `(1, 0)` at the top-right one and `(0, 1)` at the bottom-left one.
/// Returns `None` for a degenerate frame.
fn frame_coords(cand: &QrCandidate, p: Point) -> Option<(f64, f64)> {
    let (ux, uy) = (cand.tr.x - cand.tl.x, cand.tr.y - cand.tl.y);
    let (vx, vy) = (cand.bl.x - cand.tl.x, cand.bl.y - cand.tl.y);
    let det = ux * vy - uy * vx;
    if det.abs() < f64::EPSILON {
        return None;
    }
    let (dx, dy) = (p.x - cand.tl.x, p.y - cand.tl.y);
    Some(((dx * vy - dy * vx) / det, (ux * dy - uy * dx) / det))
}

/// Whether every finder centre of `triple` lies inside `cand`'s symbol area.
///
/// Two real symbols never overlap — a quiet zone separates them — so a triple
/// nested wholly inside an already accepted symbol cannot be a second symbol.
/// It is a phantom assembled from interior structure: alignment patterns and
/// dense data runs occasionally clear the `1:1:3:1:1` test on a handful of
/// scanlines, and three of them at roughly right angles then score as a small
/// symbol. Testing all three centres, rather than any one, keeps the rule
/// conservative — only fully nested triples are dropped.
///
/// The frame runs `0..=1` between finder *centres*, which sit 3.5 modules in
/// from each edge, so the symbol itself extends `3.5 / (dimension - 7)` beyond
/// that on every side.
fn is_nested_in(cand: &QrCandidate, triple: &QrCandidate) -> bool {
    let margin = 3.5 / (cand.dimension as f64 - 7.0);
    let bounds = -margin..=(1.0 + margin);
    [triple.tl, triple.tr, triple.bl].iter().all(|&p| {
        matches!(frame_coords(cand, p), Some((a, b))
            if bounds.contains(&a) && bounds.contains(&b))
    })
}

/// Groups finders into QR candidates (best, non-overlapping first).
pub fn group(finders: &[Finder]) -> Vec<QrCandidate> {
    let n = finders.len();
    if n < 3 {
        return Vec::new();
    }

    let mut scored: Vec<Scored> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            for k in (j + 1)..n {
                if let Some(s) = score_triple(finders, i, j, k) {
                    scored.push(s);
                }
            }
        }
    }
    scored.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(core::cmp::Ordering::Equal)
    });

    let mut used = vec![false; n];
    let mut out = Vec::new();
    for s in scored {
        if s.idx.iter().any(|&i| used[i]) {
            continue;
        }
        if out.iter().any(|c| is_nested_in(c, &s.cand)) {
            continue;
        }
        for &i in &s.idx {
            used[i] = true;
        }
        out.push(s.cand);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(x: f64, y: f64, m: f64) -> Finder {
        Finder {
            center: Point::new(x, y),
            module: m,
            votes: 5,
        }
    }

    #[test]
    fn orders_upright_corners() {
        // Upright symbol: TL top-left, TR top-right, BL bottom-left.
        let (tl, tr, bl) = order_corners(
            Point::new(10.0, 10.0),
            Point::new(100.0, 10.0),
            Point::new(10.0, 100.0),
        );
        assert_eq!(tl, Point::new(10.0, 10.0));
        assert_eq!(tr, Point::new(100.0, 10.0));
        assert_eq!(bl, Point::new(10.0, 100.0));
    }

    #[test]
    fn groups_single_v1_symbol() {
        // Version 1 = 21 modules. Finder centres are 3.5 modules in; with a
        // module of 4 px, centre separation = (21-7)*4 = 56 px.
        let m = 4.0;
        let base = 14.0; // 3.5 * 4
        let sep = 56.0;
        let finders = [
            f(base, base, m),
            f(base + sep, base, m),
            f(base, base + sep, m),
        ];
        let groups = group(&finders);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].dimension, 21);
    }

    #[test]
    fn groups_two_symbols() {
        let m = 4.0;
        let sep = 56.0;
        let mut finders = Vec::new();
        for &ox in &[0.0, 300.0] {
            finders.push(f(ox + 14.0, 14.0, m));
            finders.push(f(ox + 14.0 + sep, 14.0, m));
            finders.push(f(ox + 14.0, 14.0 + sep, m));
        }
        let groups = group(&finders);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn axis_scale_is_identity_when_upright() {
        assert!((axis_scale((1.0, 0.0), (0.0, 1.0)) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn axis_scale_undoes_rotation_stretch() {
        // Axis-aligned scanline runs are stretched by 1 / max(|cos|, |sin|);
        // the factor must fold at 45 degrees, so 60 degrees behaves like 30.
        for (deg, expect) in [
            (0.0f64, 1.0),
            (30.0, 30f64.to_radians().cos()),
            (45.0, 0.5f64.sqrt()),
        ] {
            let (s, c) = deg.to_radians().sin_cos();
            let got = axis_scale((c, s), (-s, c));
            assert!((got - expect).abs() < 1e-12, "{deg} deg: {got} != {expect}");
        }
        let a = axis_scale(
            (30f64.to_radians().cos(), 30f64.to_radians().sin()),
            (-30f64.to_radians().sin(), 30f64.to_radians().cos()),
        );
        let b = axis_scale(
            (60f64.to_radians().cos(), 60f64.to_radians().sin()),
            (-60f64.to_radians().sin(), 60f64.to_radians().cos()),
        );
        assert!((a - b).abs() < 1e-12, "the factor must fold at 45 degrees");
    }

    #[test]
    fn rotated_symbol_keeps_its_true_dimension() {
        // A v10 symbol (57 modules, 8 px each) rotated 30 degrees. The legs stay
        // 400 px; only the scanline-derived module is stretched, by 1 / cos 30.
        let (module, sep) = (8.0, 50.0 * 8.0);
        let stretched = module / 30f64.to_radians().cos();
        let (s, c) = 30f64.to_radians().sin_cos();
        let tl = (100.0, 400.0);
        let finders = [
            f(tl.0, tl.1, stretched),
            f(tl.0 + sep * c, tl.1 + sep * s, stretched),
            f(tl.0 - sep * s, tl.1 + sep * c, stretched),
        ];
        let groups = group(&finders);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].dimension, 57);
    }

    #[test]
    fn nested_phantom_triple_is_rejected() {
        // A real v7 symbol (45 modules, 8 px each) plus three spurious finders
        // sitting inside it — the CI-2 shape, where interior structure scores as
        // a second, smaller symbol.
        let (m, sep) = (8.0, 38.0 * 8.0); // (45 - 7) * 8 = 304 px
        let real = [
            f(28.0, 28.0, m),
            f(28.0 + sep, 28.0, m),
            f(28.0, 28.0 + sep, m),
        ];
        // Alignment patterns 16 modules apart read as ~5.7 px "modules".
        let phantom_m = 5.7;
        let phantom_sep = 16.0 * 8.0;
        let phantom = [
            f(76.0, 76.0, phantom_m),
            f(76.0 + phantom_sep, 76.0, phantom_m),
            f(76.0, 76.0 + phantom_sep, phantom_m),
        ];

        let mut all = real.to_vec();
        all.extend_from_slice(&phantom);
        let groups = group(&all);
        assert_eq!(groups.len(), 1, "the nested triple must not be accepted");
        assert_eq!(groups[0].dimension, 45);

        // Without the real symbol around it the same triple is a valid candidate,
        // so the rejection is the nesting rule and not the geometry.
        assert_eq!(group(&phantom).len(), 1);
    }

    #[test]
    fn adjacent_symbols_are_not_treated_as_nested() {
        // Two v1 symbols side by side with a 4-module quiet zone between them:
        // neither may be swallowed by the other's symbol area.
        let m = 4.0;
        let sep = 56.0;
        let a = QrCandidate {
            tl: Point::new(14.0, 14.0),
            tr: Point::new(14.0 + sep, 14.0),
            bl: Point::new(14.0, 14.0 + sep),
            dimension: 21,
        };
        let dx = (21.0 + 8.0) * m;
        let b = QrCandidate {
            tl: Point::new(14.0 + dx, 14.0),
            tr: Point::new(14.0 + dx + sep, 14.0),
            bl: Point::new(14.0 + dx, 14.0 + sep),
            dimension: 21,
        };
        assert!(!is_nested_in(&a, &b));
        assert!(!is_nested_in(&b, &a));
    }
}
