//! Finder-pattern location.
//!
//! Scans every row for the canonical `1:1:3:1:1` dark/light run signature of a
//! QR finder pattern, cross-checks each horizontal hit against the vertical
//! profile through its centre, then clusters coincident hits into stable
//! candidate centres.

use crate::binarize::Binary;
use crate::geometry::Point;

/// A located finder-pattern centre with its estimated module size (in pixels).
#[derive(Clone, Copy, Debug)]
pub struct Finder {
    /// Sub-pixel centre.
    pub center: Point,
    /// Estimated module (unit cell) size in pixels.
    pub module: f64,
    /// Number of independent scanline hits that agreed on this centre.
    pub votes: u32,
}

/// Checks whether five consecutive run lengths match `1:1:3:1:1`.
///
/// Returns the estimated module size on success.
fn matches_ratio(runs: [f64; 5]) -> Option<f64> {
    let total: f64 = runs.iter().sum();
    if total < 7.0 {
        return None;
    }
    let module = total / 7.0;
    let max_var = module / 2.0;
    if (runs[0] - module).abs() < max_var
        && (runs[1] - module).abs() < max_var
        && (runs[2] - 3.0 * module).abs() < 3.0 * max_var
        && (runs[3] - module).abs() < max_var
        && (runs[4] - module).abs() < max_var
    {
        Some(module)
    } else {
        None
    }
}

/// Walks a full vertical column through `x`, returning the run lengths of the
/// five bands centred on row `y` (which must be dark), or `None` if the profile
/// there is not `1:1:3:1:1`. On success also returns the sub-pixel centre row.
fn vertical_check(bin: &Binary, x: i64, y: i64, est_module: f64) -> Option<(f64, f64)> {
    if !bin.get(x, y) {
        return None;
    }
    let max_row = bin.height() as i64;

    // Centre dark band.
    let mut c2 = 1.0;
    let mut yy = y - 1;
    while yy >= 0 && bin.get(x, yy) {
        c2 += 1.0;
        yy -= 1;
    }
    let top_dark_edge = yy; // first light row above (or -1)
    let mut c1 = 0.0;
    while yy >= 0 && !bin.get(x, yy) && c1 < est_module * 2.0 {
        c1 += 1.0;
        yy -= 1;
    }
    let mut c0 = 0.0;
    while yy >= 0 && bin.get(x, yy) && c0 < est_module * 2.0 {
        c0 += 1.0;
        yy -= 1;
    }

    let mut yy = y + 1;
    while yy < max_row && bin.get(x, yy) {
        c2 += 1.0;
        yy += 1;
    }
    let bottom_dark_edge = yy; // first light row below
    let mut c3 = 0.0;
    while yy < max_row && !bin.get(x, yy) && c3 < est_module * 2.0 {
        c3 += 1.0;
        yy += 1;
    }
    let mut c4 = 0.0;
    while yy < max_row && bin.get(x, yy) && c4 < est_module * 2.0 {
        c4 += 1.0;
        yy += 1;
    }

    matches_ratio([c0, c1, c2, c3, c4])?;
    // Centre of the middle dark band.
    let center = (top_dark_edge as f64 + bottom_dark_edge as f64) / 2.0;
    Some((center, (c0 + c1 + c2 + c3 + c4) / 7.0))
}

/// Refines the horizontal centre by walking left/right through the centre dark
/// band at row `y`.
fn horizontal_center(bin: &Binary, x: i64, y: i64) -> f64 {
    let mut left = x;
    while left > 0 && bin.get(left - 1, y) {
        left -= 1;
    }
    let mut right = x;
    while right + 1 < bin.width() as i64 && bin.get(right + 1, y) {
        right += 1;
    }
    (left as f64 + right as f64) / 2.0
}

/// Scans the binary image and returns clustered finder candidates.
pub fn find_finders(bin: &Binary) -> Vec<Finder> {
    let width = bin.width() as i64;
    let height = bin.height() as i64;
    let mut raw: Vec<(f64, f64, f64)> = Vec::new(); // (cx, cy, module)

    for y in 0..height {
        // Collect runs across the row as (is_dark, length, start_x).
        let mut runs: Vec<(bool, i64, i64)> = Vec::new();
        let mut x = 0;
        while x < width {
            let color = bin.get(x, y);
            let start = x;
            while x < width && bin.get(x, y) == color {
                x += 1;
            }
            runs.push((color, x - start, start));
        }

        // Slide a window of five runs: dark, light, dark, light, dark.
        if runs.len() < 5 {
            continue;
        }
        for i in 0..=runs.len() - 5 {
            let w = &runs[i..i + 5];
            if !(w[0].0 && !w[1].0 && w[2].0 && !w[3].0 && w[4].0) {
                continue;
            }
            let lens = [
                w[0].1 as f64,
                w[1].1 as f64,
                w[2].1 as f64,
                w[3].1 as f64,
                w[4].1 as f64,
            ];
            let Some(module) = matches_ratio(lens) else {
                continue;
            };
            // Centre of the middle (3x) dark run.
            let mid_start = w[2].2;
            let cx_guess = mid_start + w[2].1 / 2;
            let Some((cy, vmodule)) = vertical_check(bin, cx_guess, y, module) else {
                continue;
            };
            // Modules from horizontal and vertical scans should agree.
            if (module - vmodule).abs() > module.max(vmodule) * 0.5 {
                continue;
            }
            let cx = horizontal_center(bin, cx_guess, y);
            raw.push((cx, cy, (module + vmodule) / 2.0));
        }
    }

    cluster(raw)
}

/// Merges coincident raw hits into averaged finder centres.
fn cluster(raw: Vec<(f64, f64, f64)>) -> Vec<Finder> {
    let mut clusters: Vec<(f64, f64, f64, u32)> = Vec::new(); // sum_x, sum_y, sum_m, n
    for (cx, cy, m) in raw {
        let mut merged = false;
        for c in &mut clusters {
            let mean_x = c.0 / c.3 as f64;
            let mean_y = c.1 / c.3 as f64;
            let mean_m = c.2 / c.3 as f64;
            let dx = cx - mean_x;
            let dy = cy - mean_y;
            let tol = mean_m.max(m) * 1.5;
            if dx * dx + dy * dy < tol * tol && (mean_m - m).abs() < mean_m.max(m) * 0.6 {
                c.0 += cx;
                c.1 += cy;
                c.2 += m;
                c.3 += 1;
                merged = true;
                break;
            }
        }
        if !merged {
            clusters.push((cx, cy, m, 1));
        }
    }

    clusters
        .into_iter()
        // A genuine finder pattern is crossed by several scanlines; drop
        // clusters backed by only one or two incidental hits (random-data
        // flukes).
        .filter(|&(_, _, _, n)| n >= 3)
        .map(|(sx, sy, sm, n)| Finder {
            center: Point::new(sx / n as f64, sy / n as f64),
            module: sm / n as f64,
            votes: n,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_accepts_clean_pattern() {
        assert!(matches_ratio([4.0, 4.0, 12.0, 4.0, 4.0]).is_some());
        assert_eq!(matches_ratio([4.0, 4.0, 12.0, 4.0, 4.0]).unwrap(), 4.0);
    }

    #[test]
    fn ratio_rejects_bad_pattern() {
        assert!(matches_ratio([4.0, 4.0, 4.0, 4.0, 4.0]).is_none());
        assert!(matches_ratio([1.0, 8.0, 12.0, 1.0, 4.0]).is_none());
    }
}
