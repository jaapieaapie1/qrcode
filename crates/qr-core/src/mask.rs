//! The eight data-mask predicates (ISO Table 10), shared by encode and decode.

/// Returns whether the mask numbered `mask_id` (`0..=7`) inverts the module at
/// column `x`, row `y`.
///
/// Coordinates are `(x, y)` = (column, row) with the origin at the top-left.
/// Mask ids outside `0..=7` return `false`.
#[inline]
pub fn mask_bit(mask_id: u8, x: u32, y: u32) -> bool {
    let i = y as u64; // row
    let j = x as u64; // column
    match mask_id {
        0 => (i + j).is_multiple_of(2),
        1 => i.is_multiple_of(2),
        2 => j.is_multiple_of(3),
        3 => (i + j).is_multiple_of(3),
        4 => (i / 2 + j / 3).is_multiple_of(2),
        5 => (i * j) % 2 + (i * j) % 3 == 0,
        6 => ((i * j) % 2 + (i * j) % 3).is_multiple_of(2),
        7 => ((i + j) % 2 + (i * j) % 3).is_multiple_of(2),
        _ => false,
    }
}
