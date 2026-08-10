//! Turns a grouped QR candidate into a sampled [`BitMatrix`].
//!
//! For versions >= 2 we first refine the bottom-right corner by locating the
//! nearest alignment pattern, which pins down the perspective. We then build a
//! grid -> image homography and sample the centre of every module.

use crate::binarize::Binary;
use crate::geometry::{PerspectiveTransform, Point};
use crate::grouping::QrCandidate;
use qr_core::{BitMatrix, Version};

/// Result of sampling: the raw module grid plus the four symbol corners in
/// image space (top-left, top-right, bottom-right, bottom-left).
pub struct Sampled {
    pub matrix: BitMatrix,
    pub corners: [Point; 4],
}

/// Samples the module grid for one candidate. Returns `None` if the geometry is
/// invalid or the grid would fall outside the image.
pub fn sample(bin: &Binary, cand: &QrCandidate) -> Option<Sampled> {
    let dim = cand.dimension;
    let version = Version::new(((dim - 17) / 4) as u8).ok()?;
    let dim_f = dim as f64;

    // Grid-space centres of the three finder patterns (module-centre coords).
    let g_tl = Point::new(3.5, 3.5);
    let g_tr = Point::new(dim_f - 3.5, 3.5);
    let g_bl = Point::new(3.5, dim_f - 3.5);

    // Provisional (parallelogram) bottom-right in image space.
    let bottom_right = Point::new(
        cand.tr.x + cand.bl.x - cand.tl.x,
        cand.tr.y + cand.bl.y - cand.tl.y,
    );
    let g_br_default = Point::new(dim_f - 3.5, dim_f - 3.5);

    // Provisional transform from three finders (+ parallelogram corner).
    let provisional = PerspectiveTransform::quad_to_quad(
        [g_tl, g_tr, g_br_default, g_bl],
        [cand.tl, cand.tr, bottom_right, cand.bl],
    );

    // Refine the fourth correspondence with an alignment pattern for v >= 2.
    let (g_br, img_br) =
        refine_alignment(bin, version, &provisional).unwrap_or((g_br_default, bottom_right));

    let transform = PerspectiveTransform::quad_to_quad(
        [g_tl, g_tr, g_br, g_bl],
        [cand.tl, cand.tr, img_br, cand.bl],
    );

    // Sample each module centre.
    let mut matrix = BitMatrix::new(dim);
    for row in 0..dim {
        for col in 0..dim {
            let p = transform.map(col as f64 + 0.5, row as f64 + 0.5);
            let dark = bin.get(p.x.round() as i64, p.y.round() as i64);
            matrix.set(col, row, dark);
        }
    }

    // Symbol corners in image space (outer grid corners).
    let corners = [
        transform.map(0.0, 0.0),
        transform.map(dim_f, 0.0),
        transform.map(dim_f, dim_f),
        transform.map(0.0, dim_f),
    ];

    Some(Sampled { matrix, corners })
}

/// Attempts to locate the bottom-right alignment pattern by template matching
/// the 5x5 structure using local basis vectors derived from the provisional
/// transform. Returns the matching grid-space and image-space centres.
///
/// This does not rely on clean `1:1:1:1:1` run isolation (the pattern's outer
/// border commonly merges with adjacent data modules), and it follows the
/// perspective because the basis vectors come from the provisional transform.
fn refine_alignment(
    bin: &Binary,
    version: Version,
    provisional: &PerspectiveTransform,
) -> Option<(Point, Point)> {
    let positions = version.alignment_pattern_positions();
    // The bottom-right alignment centre is at the last coordinate in both axes.
    let last = *positions.last()? as f64;
    let g_align = Point::new(last + 0.5, last + 0.5);

    let center0 = provisional.map(g_align.x, g_align.y);
    // Local per-module basis vectors in image space near the alignment pattern.
    let right = provisional.map(g_align.x + 1.0, g_align.y);
    let down = provisional.map(g_align.x, g_align.y + 1.0);
    let ex = Point::new(right.x - center0.x, right.y - center0.y);
    let ey = Point::new(down.x - center0.x, down.y - center0.y);
    let modpx = ((ex.x * ex.x + ex.y * ex.y).sqrt() + (ey.x * ey.x + ey.y * ey.y).sqrt()) / 2.0;
    if !modpx.is_finite() || modpx < 1.0 {
        return None;
    }

    // Search integer pixel offsets within ~2.5 modules of the expected centre.
    let search = (modpx * 2.5).ceil() as i64;
    let mut best: Option<(i32, f64, i64, i64)> = None; // (score, -dist, ox, oy)
    for oy in -search..=search {
        for ox in -search..=search {
            let px = center0.x + ox as f64;
            let py = center0.y + oy as f64;
            let score = template_score(bin, px, py, ex, ey);
            // Require a strong match (of 25 template cells).
            if score < 15 {
                continue;
            }
            let dist = (ox * ox + oy * oy) as f64;
            let better = match best {
                None => true,
                Some((bs, bnd, _, _)) => score > bs || (score == bs && -dist > bnd),
            };
            if better {
                best = Some((score, -dist, ox, oy));
            }
        }
    }

    let (_, _, ox, oy) = best?;
    let px = center0.x + ox as f64;
    let py = center0.y + oy as f64;
    // Sub-pixel refine on the central dark module blob.
    let refined = blob_center(bin, px.round() as i64, py.round() as i64);
    Some((g_align, refined))
}

/// Scores the 5x5 alignment template at image centre `(px, py)` using per-module
/// basis vectors `ex` (one column right) and `ey` (one row down). Each of the 25
/// cells contributes +1 when the sampled darkness matches the template and -1
/// otherwise; the maximum score is 25.
fn template_score(bin: &Binary, px: f64, py: f64, ex: Point, ey: Point) -> i32 {
    let mut score = 0;
    for j in -2i32..=2 {
        for i in -2i32..=2 {
            let sx = px + i as f64 * ex.x + j as f64 * ey.x;
            let sy = py + i as f64 * ex.y + j as f64 * ey.y;
            let dark = bin.get(sx.round() as i64, sy.round() as i64);
            // Alignment pattern: dark border ring (radius 2), light gap ring
            // (radius 1), dark centre.
            let expect_dark = i == 0 && j == 0 || i.abs() == 2 || j.abs() == 2;
            score += if dark == expect_dark { 1 } else { -1 };
        }
    }
    score
}

/// Sub-pixel centre of the connected dark blob containing `(x, y)`, found by
/// walking its horizontal and vertical extents (the central dark module of an
/// alignment pattern is isolated by the light gap ring).
fn blob_center(bin: &Binary, x: i64, y: i64) -> Point {
    let width = bin.width() as i64;
    let height = bin.height() as i64;

    let mut left = x;
    while left > 0 && bin.get(left - 1, y) {
        left -= 1;
    }
    let mut right = x;
    while right + 1 < width && bin.get(right + 1, y) {
        right += 1;
    }
    let cx = (left + right) as f64 / 2.0;

    let mid_x = cx.round() as i64;
    let mut top = y;
    while top > 0 && bin.get(mid_x, top - 1) {
        top -= 1;
    }
    let mut bottom = y;
    while bottom + 1 < height && bin.get(mid_x, bottom + 1) {
        bottom += 1;
    }
    let cy = (top + bottom) as f64 / 2.0;

    // Re-centre x at the true centre row.
    let mid_y = cy.round() as i64;
    let mut left = mid_x;
    while left > 0 && bin.get(left - 1, mid_y) {
        left -= 1;
    }
    let mut right = mid_x;
    while right + 1 < width && bin.get(right + 1, mid_y) {
        right += 1;
    }
    Point::new((left + right) as f64 / 2.0, cy)
}
