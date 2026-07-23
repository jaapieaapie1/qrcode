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

    // Right angle at the top-left corner: normalised |cos| must be small.
    let v1 = (tr.x - tl.x, tr.y - tl.y);
    let v2 = (bl.x - tl.x, bl.y - tl.y);
    let dot = v1.0 * v2.0 + v1.1 * v2.1;
    let cos = dot / (leg_top * leg_left);
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
    let module = (fi.module + fj.module + fk.module) / 3.0;

    // Dimension from centre-to-centre leg length: finder centres are 3.5 modules
    // in from each edge, so their separation spans `dimension - 7` modules.
    let raw_dim = ((leg_top + leg_left) / 2.0) / module + 7.0;
    let dimension = snap_dimension(raw_dim)?;

    // The snapped dimension should be consistent with the measured legs.
    let expected_leg = (dimension as f64 - 7.0) * module;
    let leg_err = ((leg_top - expected_leg).abs() + (leg_left - expected_leg).abs())
        / (2.0 * expected_leg);
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
    scored.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(core::cmp::Ordering::Equal));

    let mut used = vec![false; n];
    let mut out = Vec::new();
    for s in scored {
        if s.idx.iter().any(|&i| used[i]) {
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
}
