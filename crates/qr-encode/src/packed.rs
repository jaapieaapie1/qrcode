//! Word-packed symbol construction: placement, masking and ISO 7.8.3 penalty
//! scoring over `u64` rows instead of per-module [`BitMatrix`] accesses.
//!
//! Mask selection dominates `encode()` — eight candidate symbols are built and
//! scored, each walking every module several times. This module keeps the
//! symbol as packed rows: one bit per module, row-major, `ceil(size / 64)` `u64`
//! words per row, bit `x` of row `y` at word `x / 64`, bit `x % 64` counted
//! from the least-significant end. Bits at `x >= size` are held at zero, which
//! makes the padding read as light modules and keeps every scan free of a
//! bounds test.
//!
//! Three properties make the word form pay off:
//!
//! * The function-module map is computed once, structurally, instead of asking
//!   [`qr_core::is_function_module`] per module per candidate (that predicate
//!   scans the alignment grid, so it is far from free at v40).
//! * The eight mask patterns are periodic with a vertical period dividing 12,
//!   so 12 packed rows per mask describe every row of every symbol size. A
//!   candidate is then `data_row XOR (mask_row AND NOT function_row)` — no
//!   per-module predicate at all.
//! * N1/N3 need run lengths along a line, which come out of a packed row as
//!   transition bits (`v ^ (v << 1)`) walked with `trailing_zeros`; N2 and N4
//!   are pure word logic. The column direction reuses the row code after a
//!   64x64 blocked bit transpose.
//!
//! Scores are bit-identical to the straightforward per-module implementation
//! kept as `matrix::reference` for the tests to check against.

use alloc::vec;
use qr_core::{
    BitMatrix, EcLevel, Version,
    bch::{format_info_encode, version_info_encode},
    mask_bit,
};

/// Vertical period shared by all eight data masks (lcm of 2, 3, 4 and 6).
const MASK_PERIOD: usize = 12;

/// Number of `u64` words per packed row for a symbol `size` modules across.
#[inline]
fn words_per_row(size: u32) -> usize {
    (size as usize).div_ceil(64)
}

/// Low `n` bits set (`n <= 64`).
#[inline]
fn low_mask(n: u32) -> u64 {
    if n >= 64 { !0 } else { (1u64 << n) - 1 }
}

/// Bits of word `w` that correspond to `x < limit`.
#[inline]
fn limit_mask(w: usize, limit: u32) -> u64 {
    let base = (w * 64) as u32;
    if limit <= base {
        0
    } else {
        low_mask(limit - base)
    }
}

#[inline]
fn get_bit(rows: &[u64], words: usize, x: u32, y: u32) -> bool {
    (rows[y as usize * words + (x as usize >> 6)] >> (x & 63)) & 1 == 1
}

#[inline]
fn set_bit(rows: &mut [u64], words: usize, x: u32, y: u32, value: bool) {
    let i = y as usize * words + (x as usize >> 6);
    let bit = 1u64 << (x & 63);
    if value {
        rows[i] |= bit;
    } else {
        rows[i] &= !bit;
    }
}

/// Copies the packed grid into `out`, which must already be `size` across and
/// all-light: only the dark modules are written.
fn write_bitmatrix(rows: &[u64], words: usize, size: u32, out: &mut BitMatrix) {
    for y in 0..size {
        for (wi, &word) in rows[y as usize * words..][..words].iter().enumerate() {
            let mut w = word;
            while w != 0 {
                out.set((wi * 64) as u32 + w.trailing_zeros(), y, true);
                w &= w - 1;
            }
        }
    }
}

/// Builds the map of reserved (function) modules for `version`.
///
/// Structurally equivalent to [`qr_core::is_function_module`] but written as
/// region fills, so it costs one pass over the function modules rather than a
/// predicate call per module. `packed::tests` checks the two agree for every
/// version.
fn function_map(rows: &mut [u64], words: usize, version: Version) {
    let size = version.size();

    // Finder patterns plus separators: three 8x8 corner blocks.
    for i in 0..8u32 {
        for j in 0..8u32 {
            set_bit(rows, words, j, i, true); // top-left
            set_bit(rows, words, size - 1 - j, i, true); // top-right
            set_bit(rows, words, j, size - 1 - i, true); // bottom-left
        }
    }

    // Timing patterns.
    for i in 0..size {
        set_bit(rows, words, 6, i, true);
        set_bit(rows, words, i, 6, true);
    }

    // Format information (and the fixed dark module's reserved band).
    for i in 0..=8u32 {
        set_bit(rows, words, i, 8, true);
        set_bit(rows, words, 8, i, true);
    }
    for i in size - 8..size {
        set_bit(rows, words, i, 8, true);
        set_bit(rows, words, 8, i, true);
    }

    // Version information (versions >= 7).
    if version.number() >= 7 {
        for y in 0..6u32 {
            for x in size - 11..size - 8 {
                set_bit(rows, words, x, y, true);
            }
        }
        for y in size - 11..size - 8 {
            for x in 0..6u32 {
                set_bit(rows, words, x, y, true);
            }
        }
    }

    // Alignment patterns, skipping the three centres that collide with finders.
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
                let (cx, cy) = (cx as u32, cy as u32);
                for y in cy - 2..=cy + 2 {
                    for x in cx - 2..=cx + 2 {
                        set_bit(rows, words, x, y, true);
                    }
                }
            }
        }
    }
}

/// Draws the finder patterns, timing patterns and alignment patterns.
fn place_function_patterns(rows: &mut [u64], words: usize, version: Version) {
    let size = version.size();

    for &(ox, oy) in &[(0u32, 0u32), (size - 7, 0), (0, size - 7)] {
        for dy in 0..7 {
            for dx in 0..7 {
                let edge = dx == 0 || dx == 6 || dy == 0 || dy == 6;
                let core = (2..=4).contains(&dx) && (2..=4).contains(&dy);
                set_bit(rows, words, ox + dx, oy + dy, edge || core);
            }
        }
    }

    for i in 8..size - 8 {
        let dark = i % 2 == 0;
        set_bit(rows, words, i, 6, dark);
        set_bit(rows, words, 6, i, dark);
    }

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
                for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        let ring = dx.abs() == 2 || dy.abs() == 2;
                        let centre = dx == 0 && dy == 0;
                        let x = (cx as i32 + dx) as u32;
                        let y = (cy as i32 + dy) as u32;
                        set_bit(rows, words, x, y, ring || centre);
                    }
                }
            }
        }
    }
}

/// Places the interleaved codeword stream in the ISO zigzag order, skipping the
/// modules marked in `funcs`.
fn place_data(rows: &mut [u64], words: usize, funcs: &[u64], version: Version, codewords: &[u8]) {
    let size = version.size();
    let mut bit_index = 0usize;
    let total_bits = codewords.len() * 8;

    let mut right = size as i32 - 1;
    while right >= 1 {
        if right == 6 {
            right = 5;
        }
        for vert in 0..size {
            for j in 0..2 {
                let x = (right - j) as u32;
                let upward = ((right + 1) & 2) == 0;
                let y = if upward { size - 1 - vert } else { vert };
                if get_bit(funcs, words, x, y) {
                    continue;
                }
                if bit_index < total_bits {
                    let byte = codewords[bit_index >> 3];
                    let bit = (byte >> (7 - (bit_index & 7))) & 1 == 1;
                    set_bit(rows, words, x, y, bit);
                    bit_index += 1;
                }
            }
        }
        right -= 2;
    }
}

/// Writes the two copies of the 18-bit version information (versions >= 7).
fn place_version_info(rows: &mut [u64], words: usize, version: Version) {
    let Some(bits) = version_info_encode(version.number()) else {
        return;
    };
    let size = version.size();
    for i in 0..18u32 {
        let bit = (bits >> i) & 1 == 1;
        let a = size - 11 + i % 3;
        let b = i / 3;
        set_bit(rows, words, a, b, bit);
        set_bit(rows, words, b, a, bit);
    }
}

/// Writes the two copies of the 15-bit format information for `(ec, mask)`.
///
/// The fixed dark module is not written here: it does not depend on the mask
/// and is placed once into the shared base grid.
fn place_format_info(rows: &mut [u64], words: usize, size: u32, ec: EcLevel, mask: u8) {
    let bits = format_info_encode(ec, mask);
    let get = |i: u8| (bits >> i) & 1 == 1;

    for i in 0..6u8 {
        set_bit(rows, words, 8, i as u32, get(i));
    }
    set_bit(rows, words, 8, 7, get(6));
    set_bit(rows, words, 8, 8, get(7));
    set_bit(rows, words, 7, 8, get(8));
    for i in 9..15u8 {
        set_bit(rows, words, 14 - i as u32, 8, get(i));
    }

    for i in 0..8u8 {
        set_bit(rows, words, size - 1 - i as u32, 8, get(i));
    }
    for i in 8..15u8 {
        set_bit(rows, words, 8, size - 15 + i as u32, get(i));
    }
}

/// Packs mask `m` into `table` as `MASK_PERIOD` rows.
///
/// Every ISO mask repeats vertically with a period of 1, 2, 3, 4 or 6 rows, so
/// row `y` of the mask is row `y % MASK_PERIOD` of the table.
fn mask_pattern_rows(table: &mut [u64], size: u32, words: usize, m: u8) {
    table.fill(0);
    for y in 0..MASK_PERIOD {
        let base = y * words;
        for x in 0..size {
            if mask_bit(m, x, y as u32) {
                table[base + (x as usize >> 6)] |= 1u64 << (x & 63);
            }
        }
    }
}

/// Fills `dst` with the base grid masked by the `MASK_PERIOD` pattern rows in
/// `pattern`, leaving function modules untouched.
fn build_candidate(
    dst: &mut [u64],
    base: &[u64],
    funcs: &[u64],
    pattern: &[u64],
    words: usize,
    size: u32,
) {
    for y in 0..size as usize {
        let src = (y % MASK_PERIOD) * words;
        let off = y * words;
        for w in 0..words {
            dst[off + w] = base[off + w] ^ (pattern[src + w] & !funcs[off + w]);
        }
    }
}

// ---- penalty scoring ------------------------------------------------------

/// Transposes a 64x64 bit block in place: on return, bit `r` of `a[k]` is bit
/// `k` of the original `a[r]`, counting bits from the least-significant end.
///
/// Recursive halving: a transpose of the whole block is the transpose of each
/// quadrant with the two off-diagonal quadrants exchanged, so each round swaps
/// the top `j` bits of row `k` with the bottom `j` bits of row `k + j` and then
/// halves `j`. `m` selects the bottom `j` bits of every `2j`-wide field.
fn transpose64(a: &mut [u64; 64]) {
    let mut j = 32usize;
    let mut m = 0x0000_0000_FFFF_FFFFu64;
    while j != 0 {
        let mut k = 0usize;
        while k < 64 {
            let t = ((a[k] >> j) ^ a[k + j]) & m;
            a[k] ^= t << j;
            a[k + j] ^= t;
            k = (k + j + 1) & !j;
        }
        j >>= 1;
        m ^= m << j;
    }
}

/// Writes the transpose of `rows` into `out`, in 64x64 blocks.
fn transpose(rows: &[u64], words: usize, size: u32, out: &mut [u64]) {
    let mut block = [0u64; 64];
    for bi in 0..words {
        for bj in 0..words {
            for (r, slot) in block.iter_mut().enumerate() {
                let y = bi * 64 + r;
                *slot = if y < size as usize {
                    rows[y * words + bj]
                } else {
                    0
                };
            }
            transpose64(&mut block);
            for (k, &word) in block.iter().enumerate() {
                let x = bj * 64 + k;
                if x < size as usize {
                    out[x * words + bi] = word;
                }
            }
        }
    }
}

/// Rolling seven-run history behind the N3 finder-pattern test.
///
/// Same semantics as `matrix::reference::FinderHistory`: a run of `n` dark,
/// `n` light, `3n` dark, `n` light, `n` dark bordered by `4n` light on at least
/// one side scores once, for any scale `n`.
struct History {
    runs: [u32; 7],
    empty: bool,
    line_len: u32,
}

impl History {
    #[inline]
    fn push(&mut self, mut run: u32) {
        // `empty` plays the role of the reference scorer's `runs[0] == 0`
        // check: both detect "no run recorded yet" — equivalent because the
        // reference never records a zero-length first run either.
        if self.empty {
            run += self.line_len; // implicit light border before the first run
            self.empty = false;
        }
        self.runs[6] = self.runs[5];
        self.runs[5] = self.runs[4];
        self.runs[4] = self.runs[3];
        self.runs[3] = self.runs[2];
        self.runs[2] = self.runs[1];
        self.runs[1] = self.runs[0];
        self.runs[0] = run;
    }

    #[inline]
    fn count(&self) -> u32 {
        let r = &self.runs;
        let n = r[1];
        if n == 0 || r[2] != n || r[3] != n * 3 || r[4] != n || r[5] != n {
            return 0;
        }
        let lead = r[0] >= n * 4 && r[6] >= n;
        let trail = r[6] >= n * 4 && r[0] >= n;
        u32::from(lead || trail)
    }
}

/// The N1 (run) and N3 (finder-like) penalties of one packed line.
///
/// Runs are recovered from the transition mask `v ^ (v << 1)` and walked with
/// `trailing_zeros`, so the cost is one step per run rather than per module.
fn line_penalties(line: &[u64], size: u32) -> (u32, u32) {
    let mut n1 = 0u32;
    let mut n3 = 0u32;
    let mut hist = History {
        runs: [0; 7],
        empty: true,
        line_len: size,
    };

    // A line is conceptually bordered by light, so a line that opens dark
    // starts with a zero-length light run (matching the reference walk).
    let mut dark = line[0] & 1 != 0;
    if dark {
        hist.push(0);
        n3 += hist.count() * 40;
    }

    let mut start = 0u32;
    let mut carry = line[0] & 1; // suppresses a spurious transition at x == 0
    for (wi, &v) in line.iter().enumerate() {
        let base = (wi * 64) as u32;
        let mut changes = v ^ ((v << 1) | carry);
        carry = v >> 63;
        let rem = size - base;
        if rem < 64 {
            changes &= low_mask(rem);
        }
        while changes != 0 {
            let idx = base + changes.trailing_zeros();
            let len = idx - start;
            if len >= 5 {
                n1 += len - 2;
            }
            hist.push(len);
            if !dark {
                n3 += hist.count() * 40;
            }
            dark = !dark;
            start = idx;
            changes &= changes - 1;
        }
    }

    // The final run is closed by the light border on the far side of the line.
    let len = size - start;
    if len >= 5 {
        n1 += len - 2;
    }
    if dark {
        hist.push(len);
        hist.push(size);
    } else {
        hist.push(len + size);
    }
    n3 += hist.count() * 40;

    (n1, n3)
}

/// The N2 penalty: three points per 2x2 block of one colour.
fn n2_penalty(rows: &[u64], words: usize, size: u32) -> u32 {
    let mut blocks = 0u32;
    for y in 0..size as usize - 1 {
        let top = &rows[y * words..][..words];
        let bottom = &rows[(y + 1) * words..][..words];
        for w in 0..words {
            let (tn, bn) = if w + 1 < words {
                (top[w + 1], bottom[w + 1])
            } else {
                (0, 0)
            };
            let t = top[w];
            let b = bottom[w];
            let t1 = (t >> 1) | (tn << 63);
            let b1 = (b >> 1) | (bn << 63);
            // (x,y)==(x+1,y) and (x,y)==(x,y+1) and (x+1,y)==(x+1,y+1).
            let same = !(t ^ t1) & !(t ^ b) & !(t1 ^ b1);
            blocks += (same & limit_mask(w, size - 1)).count_ones();
        }
    }
    blocks * 3
}

/// The full ISO 7.8.3 penalty score of a packed symbol.
fn penalty_score(rows: &[u64], words: usize, size: u32, transposed: &mut [u64]) -> u32 {
    let mut n1 = 0u32;
    let mut n3 = 0u32;

    for y in 0..size as usize {
        let (a, b) = line_penalties(&rows[y * words..][..words], size);
        n1 += a;
        n3 += b;
    }

    transpose(rows, words, size, transposed);
    for x in 0..size as usize {
        let (a, b) = line_penalties(&transposed[x * words..][..words], size);
        n1 += a;
        n3 += b;
    }

    let n2 = n2_penalty(rows, words, size);

    let dark: u32 = rows.iter().map(|w| w.count_ones()).sum();
    let total = size * size;
    let deviation = (dark * 20).abs_diff(total * 10);
    let n4 = deviation.div_ceil(total).saturating_sub(1) * 10;

    n1 + n2 + n3 + n4
}

// ---- entry point ----------------------------------------------------------

/// Builds the finished symbol for `interleaved`, either with the caller's
/// `forced` mask or with the lowest-penalty mask of the eight.
///
/// Ties go to the lowest mask id.
///
/// All working storage is one `Vec`, allocated after the output matrix and
/// released before returning. That ordering matters on the bare-metal target,
/// whose bump allocator can only reclaim the most recent live block: the
/// encoder's peak stays at one output matrix plus one scratch buffer no matter
/// how many symbols are encoded back to back.
pub(crate) fn build_symbol(
    version: Version,
    ec: EcLevel,
    interleaved: &[u8],
    forced: Option<u8>,
) -> BitMatrix {
    const ALL_MASKS: [u8; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

    let size = version.size();
    let words = words_per_row(size);
    let cells = words * size as usize;
    let stride = MASK_PERIOD * words;

    // funcs, base, candidate, plus the transpose buffer when eight masks are
    // compared. The winning candidate is rebuilt from its mask id rather than
    // kept in a fifth buffer, and the mask pattern table holds one mask at a
    // time: both cost a handful of microseconds and save a grid apiece.
    let grids = if forced.is_some() { 3 } else { 4 };

    let mut out = BitMatrix::new(size);
    let mut scratch = vec![0u64; grids * cells + stride];
    {
        let (funcs, rest) = scratch.split_at_mut(cells);
        let (base, rest) = rest.split_at_mut(cells);
        let (candidate, rest) = rest.split_at_mut(cells);
        // NOTE: with a forced mask (grids = 3) `rest` is empty here, so
        // `transposed` is a zero-length slice — sound, because the column
        // penalties are only computed on the eight-mask search path, but any
        // future use of `transposed` outside that path must widen `grids`.
        let (table, transposed) = rest.split_at_mut(stride);

        function_map(funcs, words, version);
        place_function_patterns(base, words, version);
        place_data(base, words, funcs, version, interleaved);
        place_version_info(base, words, version);
        set_bit(base, words, 8, size - 8, true); // fixed dark module

        let mut chosen = 0u8;
        if let Some(mask) = forced {
            chosen = mask;
        } else {
            let mut best_score = u32::MAX;
            for mask in ALL_MASKS {
                mask_pattern_rows(table, size, words, mask);
                build_candidate(candidate, base, funcs, table, words, size);
                place_format_info(candidate, words, size, ec, mask);
                let score = penalty_score(candidate, words, size, transposed);
                if score < best_score {
                    best_score = score;
                    chosen = mask;
                }
            }
        }

        mask_pattern_rows(table, size, words, chosen);
        build_candidate(candidate, base, funcs, table, words, size);
        place_format_info(candidate, words, size, ec, chosen);
        write_bitmatrix(candidate, words, size, &mut out);
    }
    drop(scratch);
    out
}

#[cfg(test)]
mod tests;
