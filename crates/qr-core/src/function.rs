//! The function-pattern map: the single source of truth for which modules are
//! reserved (finders, separators, timing, alignment, format/version info and
//! the dark module) and therefore never carry data.

use crate::Version;

/// Returns `true` when the module at column `x`, row `y` belongs to a function
/// pattern for this `version` and must not be overwritten by data.
///
/// Coordinates are `(x, y)` = (column, row) with the origin at the top-left.
/// Out-of-range coordinates return `false`.
pub fn is_function_module(version: Version, x: u32, y: u32) -> bool {
    let size = version.size();
    if x >= size || y >= size {
        return false;
    }

    // Finder patterns plus their separators occupy an 8x8 block in three
    // corners.
    if x < 8 && y < 8 {
        return true; // top-left
    }
    if x >= size - 8 && y < 8 {
        return true; // top-right
    }
    if x < 8 && y >= size - 8 {
        return true; // bottom-left
    }

    // Timing patterns run along row 6 and column 6.
    if x == 6 || y == 6 {
        return true;
    }

    // Format information: an L around the top-left finder, plus strips beside
    // the top-right and bottom-left finders. Column 8 with y == size-8 is the
    // always-dark module, which lives in the same reserved band.
    if (y == 8 && (x <= 8 || x >= size - 8)) || (x == 8 && (y <= 8 || y >= size - 8)) {
        return true;
    }

    // Version information (versions >= 7): two 6x3 blocks by the top-right and
    // bottom-left finders.
    if version.number() >= 7 {
        if x >= size - 11 && x < size - 8 && y < 6 {
            return true; // top-right block
        }
        if y >= size - 11 && y < size - 8 && x < 6 {
            return true; // bottom-left block
        }
    }

    // Alignment patterns: 5x5 centered on each valid grid coordinate.
    let coords = version.alignment_pattern_positions();
    if !coords.is_empty() {
        let last = coords.len() - 1;
        for (ri, &cy) in coords.iter().enumerate() {
            let cy = cy as u32;
            for (ci, &cx) in coords.iter().enumerate() {
                let cx = cx as u32;
                // Skip the three centers that collide with the finder patterns:
                // top-left, top-right and bottom-left of the coordinate grid.
                let tl = ci == 0 && ri == 0;
                let tr = ci == last && ri == 0;
                let bl = ci == 0 && ri == last;
                if tl || tr || bl {
                    continue;
                }
                if x + 2 >= cx && x <= cx + 2 && y + 2 >= cy && y <= cy + 2 {
                    return true;
                }
            }
        }
    }

    false
}
