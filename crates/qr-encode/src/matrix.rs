//! Module placement, data masking, penalty scoring and format/version info.
//!
//! Coordinates are `(x, y)` = (column, row) with the origin at the top-left,
//! matching [`qr_core::BitMatrix`].

use alloc::vec::Vec;
use qr_core::{
    BitMatrix, EcLevel, Version,
    bch::{format_info_encode, version_info_encode},
    is_function_module, mask_bit,
};

/// Places the interleaved codeword bit stream into the matrix using the ISO
/// zigzag (two-column upward/downward) ordering, skipping every function
/// module. Bits are consumed most-significant-first from each codeword byte.
///
/// The number of bits placed is `codewords.len() * 8`; any leftover remainder
/// modules keep their default (light) state.
pub fn place_data(matrix: &mut BitMatrix, version: Version, codewords: &[u8]) {
    let size = version.size();
    let mut bit_index = 0usize;
    let total_bits = codewords.len() * 8;

    // Iterate column pairs from the right edge leftward.
    let mut right = size as i32 - 1;
    while right >= 1 {
        // Skip the vertical timing column at x == 6.
        if right == 6 {
            right = 5;
        }
        for vert in 0..size {
            for j in 0..2 {
                let x = (right - j) as u32;
                // Columns move upward when the pair index (from the right) is
                // even, downward otherwise.
                let upward = ((right + 1) & 2) == 0;
                let y = if upward { size - 1 - vert } else { vert };
                if is_function_module(version, x, y) {
                    continue;
                }
                if bit_index < total_bits {
                    let byte = codewords[bit_index >> 3];
                    let bit = (byte >> (7 - (bit_index & 7))) & 1 == 1;
                    matrix.set(x, y, bit);
                    bit_index += 1;
                }
            }
        }
        right -= 2;
    }
}

/// XORs the data mask `mask_id` into every non-function module.
pub fn apply_mask(matrix: &mut BitMatrix, version: Version, mask_id: u8) {
    let size = version.size();
    for y in 0..size {
        for x in 0..size {
            if is_function_module(version, x, y) {
                continue;
            }
            if mask_bit(mask_id, x, y) {
                matrix.toggle(x, y);
            }
        }
    }
}

/// Writes the two copies of the 15-bit format information for `(ec, mask)`.
pub fn place_format_info(matrix: &mut BitMatrix, version: Version, ec: EcLevel, mask: u8) {
    let bits = format_info_encode(ec, mask);
    let size = version.size();
    let get = |i: u8| (bits >> i) & 1 == 1;

    // First copy: around the top-left finder.
    for i in 0..6u8 {
        matrix.set(8, i as u32, get(i));
    }
    matrix.set(8, 7, get(6));
    matrix.set(8, 8, get(7));
    matrix.set(7, 8, get(8));
    for i in 9..15u8 {
        matrix.set(14 - i as u32, 8, get(i));
    }

    // Second copy: split beside the top-right and bottom-left finders.
    for i in 0..8u8 {
        matrix.set(size - 1 - i as u32, 8, get(i));
    }
    for i in 8..15u8 {
        matrix.set(8, size - 15 + i as u32, get(i));
    }
    // The always-dark module.
    matrix.set(8, size - 8, true);
}

/// Writes the two copies of the 18-bit version information (versions >= 7).
pub fn place_version_info(matrix: &mut BitMatrix, version: Version) {
    let Some(bits) = version_info_encode(version.number()) else {
        return;
    };
    let size = version.size();
    for i in 0..18u32 {
        let bit = (bits >> i) & 1 == 1;
        let a = size - 11 + i % 3;
        let b = i / 3;
        matrix.set(a, b, bit); // top-right block
        matrix.set(b, a, bit); // bottom-left block
    }
}

/// Computes the ISO 7.8.3 penalty score (N1–N4) for the whole symbol.
pub fn penalty_score(matrix: &BitMatrix) -> u32 {
    let [a, b, c, d] = penalty_components(matrix);
    a + b + c + d
}

/// Returns the four ISO penalty components `[N1, N2, N3, N4]` separately.
pub fn penalty_components(matrix: &BitMatrix) -> [u32; 4] {
    let size = matrix.size();
    let mut n1 = 0u32;
    let mut n2 = 0u32;
    let mut n3 = 0u32;

    // N1: runs of five or more same-colour modules in each row and column.
    for y in 0..size {
        n1 += line_run_penalty(|x| matrix.get(x, y), size);
    }
    for x in 0..size {
        n1 += line_run_penalty(|y| matrix.get(x, y), size);
    }

    // N2: 2x2 blocks of a single colour.
    for y in 0..size - 1 {
        for x in 0..size - 1 {
            let c = matrix.get(x, y);
            if matrix.get(x + 1, y) == c
                && matrix.get(x, y + 1) == c
                && matrix.get(x + 1, y + 1) == c
            {
                n2 += 3;
            }
        }
    }

    // N3: 1:1:3:1:1 finder-like patterns with a 4-module light margin, in both
    // orientations.
    for y in 0..size {
        n3 += finder_penalty(|x| matrix.get(x, y), size);
    }
    for x in 0..size {
        n3 += finder_penalty(|y| matrix.get(x, y), size);
    }

    // N4: deviation of the dark-module proportion from 50%. Using the exact ISO
    // integer form: k = ceil(|20·dark/total − 10|) − 1 five-percent steps.
    let mut dark = 0u32;
    for y in 0..size {
        for x in 0..size {
            if matrix.get(x, y) {
                dark += 1;
            }
        }
    }
    let total = size * size;
    let deviation = (dark * 20).abs_diff(total * 10);
    let n4 = deviation.div_ceil(total).saturating_sub(1) * 10;

    [n1, n2, n3, n4]
}

/// N1 penalty for a single row or column, addressed by `at(index)`.
pub(crate) fn line_run_penalty(at: impl Fn(u32) -> bool, size: u32) -> u32 {
    let mut score = 0u32;
    let mut run_colour = at(0);
    let mut run_len = 1u32;
    for i in 1..size {
        let c = at(i);
        if c == run_colour {
            run_len += 1;
        } else {
            if run_len >= 5 {
                score += 3 + (run_len - 5);
            }
            run_colour = c;
            run_len = 1;
        }
    }
    if run_len >= 5 {
        score += 3 + (run_len - 5);
    }
    score
}

/// A sliding window over the last seven run lengths of a line, used to detect
/// the 1:1:3:1:1 finder-like pattern with its four-module light border for N3.
///
/// This mirrors the ISO 7.8.3 definition: a run of `n` dark, `n` light, `3n`
/// dark, `n` light, `n` dark, bordered on at least one side by `4n` light,
/// scores 40. Handling the border via the run history (rather than a literal
/// scan) is what makes the score ISO-conformant for scaled patterns.
struct FinderHistory {
    line_len: u32,
    runs: [u32; 7],
}

impl FinderHistory {
    fn new(line_len: u32) -> Self {
        FinderHistory {
            line_len,
            runs: [0; 7],
        }
    }

    /// Pushes a completed run length, adding the implicit light border when the
    /// history is still empty (the very first run of the line).
    fn push(&mut self, mut run: u32) {
        if self.runs[0] == 0 {
            run += self.line_len; // implicit light border before the first run
        }
        self.runs.copy_within(0..6, 1);
        self.runs[0] = run;
    }

    /// Returns 1 if a finder-like pattern exists at the current window, else 0.
    ///
    /// Per ISO NOTE 3 the pattern scores once for its *existence* when a
    /// 4-module-wide light area lies before OR after it — not once per side.
    /// Only meaningful immediately after a light run has been pushed.
    fn count(&self) -> u32 {
        let r = &self.runs;
        let n = r[1];
        let core = n > 0 && r[2] == n && r[3] == n * 3 && r[4] == n && r[5] == n;
        let lead = core && r[0] >= n * 4 && r[6] >= n;
        let trail = core && r[6] >= n * 4 && r[0] >= n;
        if lead || trail { 1 } else { 0 }
    }
}

/// N3 penalty for a single row or column, addressed by `at(index)`.
pub(crate) fn finder_penalty(at: impl Fn(u32) -> bool, size: u32) -> u32 {
    let mut score = 0u32;
    let mut hist = FinderHistory::new(size);
    let mut run_colour = false; // lines are conceptually bordered by light
    let mut run_len = 0u32;
    for i in 0..size {
        let c = at(i);
        if c == run_colour {
            run_len += 1;
        } else {
            hist.push(run_len);
            if !run_colour {
                score += hist.count() * 40;
            }
            run_colour = c;
            run_len = 1;
        }
    }
    // Terminate: flush the final dark run (if any), then the trailing light
    // border, and count once more.
    if run_colour {
        hist.push(run_len);
        run_len = 0;
    }
    hist.push(run_len + size);
    score += hist.count() * 40;
    score
}

/// Returns the mask id (`0..=7`) that minimises the penalty score, given a
/// factory that produces a fresh data-placed (unmasked) matrix. The returned
/// matrix already has that mask and its format info applied.
pub fn choose_mask(
    build_unmasked: impl Fn() -> BitMatrix,
    version: Version,
    ec: EcLevel,
) -> (u8, BitMatrix) {
    let mut best: Option<(u32, u8, BitMatrix)> = None;
    for mask in 0..8u8 {
        let mut m = build_unmasked();
        apply_mask(&mut m, version, mask);
        place_format_info(&mut m, version, ec, mask);
        if version.number() >= 7 {
            place_version_info(&mut m, version);
        }
        let score = penalty_score(&m);
        match &best {
            Some((bs, _, _)) if *bs <= score => {}
            _ => best = Some((score, mask, m)),
        }
    }
    let (_, mask, matrix) = best.expect("eight masks always yield a candidate");
    (mask, matrix)
}

/// Draws the three finder patterns with separators, the timing patterns, the
/// alignment patterns and the fixed dark module into `matrix`.
pub fn place_function_patterns(matrix: &mut BitMatrix, version: Version) {
    let size = version.size();

    // Finder patterns (7x7) at the three corners.
    for &(ox, oy) in &[(0u32, 0u32), (size - 7, 0), (0, size - 7)] {
        draw_finder(matrix, ox, oy);
    }

    // Timing patterns: alternating modules along row 6 and column 6, starting
    // dark at index 8.
    for i in 8..size - 8 {
        let dark = i % 2 == 0;
        matrix.set(i, 6, dark);
        matrix.set(6, i, dark);
    }

    // Alignment patterns.
    let coords = version.alignment_pattern_positions();
    if !coords.is_empty() {
        let last = coords.len() - 1;
        for (ri, &cy) in coords.iter().enumerate() {
            for (ci, &cx) in coords.iter().enumerate() {
                let tl = ci == 0 && ri == 0;
                let tr = ci == last && ri == 0;
                let bl = ci == 0 && ri == last;
                if tl || tr || bl {
                    continue;
                }
                draw_alignment(matrix, cx as u32, cy as u32);
            }
        }
    }
}

/// Draws one 7x7 finder pattern with its top-left corner at `(ox, oy)`.
fn draw_finder(matrix: &mut BitMatrix, ox: u32, oy: u32) {
    for dy in 0..7 {
        for dx in 0..7 {
            let edge = dx == 0 || dx == 6 || dy == 0 || dy == 6;
            let core = (2..=4).contains(&dx) && (2..=4).contains(&dy);
            matrix.set(ox + dx, oy + dy, edge || core);
        }
    }
}

/// Draws one 5x5 alignment pattern centred at `(cx, cy)`.
fn draw_alignment(matrix: &mut BitMatrix, cx: u32, cy: u32) {
    for dy in -2i32..=2 {
        for dx in -2i32..=2 {
            let ring = dx.abs() == 2 || dy.abs() == 2;
            let centre = dx == 0 && dy == 0;
            let x = (cx as i32 + dx) as u32;
            let y = (cy as i32 + dy) as u32;
            matrix.set(x, y, ring || centre);
        }
    }
}

/// Builds the interleaved codeword sequence for `data_codewords` according to
/// the ISO block structure, appending the EC codewords produced per block.
pub fn interleave(data_codewords: &[u8], blocks: qr_core::EcBlocks) -> Vec<u8> {
    use qr_core::reed_solomon::rs_encode;

    let g1b = blocks.group1_blocks as usize;
    let g1d = blocks.group1_data as usize;
    let g2b = blocks.group2_blocks as usize;
    let g2d = blocks.group2_data as usize;
    let ec_len = blocks.ec_per_block as usize;

    // Split data into per-block slices and compute EC for each.
    let mut data_blocks: Vec<&[u8]> = Vec::with_capacity(g1b + g2b);
    let mut ec_blocks_vec: Vec<Vec<u8>> = Vec::with_capacity(g1b + g2b);
    let mut offset = 0usize;
    for _ in 0..g1b {
        let blk = &data_codewords[offset..offset + g1d];
        ec_blocks_vec.push(rs_encode(blk, ec_len));
        data_blocks.push(blk);
        offset += g1d;
    }
    for _ in 0..g2b {
        let blk = &data_codewords[offset..offset + g2d];
        ec_blocks_vec.push(rs_encode(blk, ec_len));
        data_blocks.push(blk);
        offset += g2d;
    }

    let max_data = g1d.max(g2d);
    let mut out = Vec::with_capacity(data_codewords.len() + ec_len * (g1b + g2b));

    // Interleave data codewords column-by-column across blocks.
    for i in 0..max_data {
        for blk in &data_blocks {
            if i < blk.len() {
                out.push(blk[i]);
            }
        }
    }
    // Interleave EC codewords column-by-column across blocks.
    for i in 0..ec_len {
        for ec in &ec_blocks_vec {
            out.push(ec[i]);
        }
    }

    out
}
