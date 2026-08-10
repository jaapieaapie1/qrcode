//! Shared helpers for the cross-crate integration suites.
//!
//! Nothing here re-implements a crate under test beyond what is needed to
//! *independently* express the ISO behaviour we are checking:
//!
//! * capacity arithmetic (so a forced version can be filled to the brim),
//! * an independently written zigzag module walk (validates encoder placement
//!   geometry against the ISO Annex I codeword stream),
//! * hand-rolled image warps (rotate / scale / perspective / noise),
//! * a deterministic xorshift PRNG for the seeded fuzz suite.
//!
//! Coordinates everywhere are `(x, y)` = (column, row), origin top-left.

#![allow(dead_code)]

use image::{DynamicImage, GrayImage, RgbaImage};
use qr_core::{BitMatrix, EcBlocks, EcLevel, Mode, Version, ec_blocks, is_function_module, mask_bit};

// ---------------------------------------------------------------------------
// Versions / EC levels
// ---------------------------------------------------------------------------

/// Every error-correction level, in ascending order of redundancy.
pub const ALL_EC: [EcLevel; 4] = [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H];

/// Short constructor for a known-good version number.
pub fn ver(n: u8) -> Version {
    Version::new(n).expect("version in 1..=40")
}

/// Human-readable EC label for assertion messages.
pub fn ec_label(ec: EcLevel) -> &'static str {
    match ec {
        EcLevel::L => "L",
        EcLevel::M => "M",
        EcLevel::Q => "Q",
        EcLevel::H => "H",
    }
}

// ---------------------------------------------------------------------------
// Payload generation
// ---------------------------------------------------------------------------

/// The kind of payload a test wants, and therefore the encoding mode the
/// encoder's auto-selection is expected to land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    /// ASCII digits only.
    Numeric,
    /// The 45-character alphanumeric set, always containing a letter.
    Alphanumeric,
    /// Arbitrary bytes, always containing `0x00` and `0xFF`.
    Binary,
    /// Valid UTF-8 with multibyte code points (2-, 3- and 4-byte).
    Utf8,
}

/// All four content kinds, for cross-product sweeps.
pub const ALL_CONTENT: [Content; 4] = [
    Content::Numeric,
    Content::Alphanumeric,
    Content::Binary,
    Content::Utf8,
];

/// The 45-character ISO alphanumeric set, in index order.
const ALNUM: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

/// Multibyte code points used to build UTF-8 payloads (2, 3 and 4 byte forms).
const MULTIBYTE: [char; 6] = ['e', 'o', '\u{00e9}', '\u{00fc}', '\u{65e5}', '\u{1f389}'];

impl Content {
    /// The mode the encoder should auto-select for this content.
    pub fn mode(self) -> Mode {
        match self {
            Content::Numeric => Mode::Numeric,
            Content::Alphanumeric => Mode::Alphanumeric,
            Content::Binary | Content::Utf8 => Mode::Byte,
        }
    }

    /// Label for assertion messages.
    pub fn label(self) -> &'static str {
        match self {
            Content::Numeric => "numeric",
            Content::Alphanumeric => "alnum",
            Content::Binary => "binary",
            Content::Utf8 => "utf8",
        }
    }
}

/// Builds a payload of `n` characters (bytes, for the byte-mode kinds) of the
/// requested kind. The result is guaranteed to make the encoder's auto mode
/// selection pick [`Content::mode`].
pub fn payload(kind: Content, n: usize) -> Vec<u8> {
    if n == 0 {
        return Vec::new();
    }
    match kind {
        Content::Numeric => (0..n).map(|i| b'0' + (i % 10) as u8).collect(),
        Content::Alphanumeric => (0..n)
            .map(|i| {
                if i == 0 {
                    b'A' // guarantees alphanumeric (not numeric) mode selection
                } else {
                    ALNUM[(i * 7) % ALNUM.len()]
                }
            })
            .collect(),
        Content::Binary => (0..n)
            .map(|i| match i {
                0 => 0x00,
                1 => 0xFF,
                _ => (i.wrapping_mul(37).wrapping_add(11) % 256) as u8,
            })
            .collect(),
        Content::Utf8 => utf8_payload(n),
    }
}

/// Builds valid UTF-8 of at most `n` bytes, ending on a character boundary and
/// containing multibyte code points whenever `n` allows.
fn utf8_payload(n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    let mut i = 0usize;
    loop {
        let ch = MULTIBYTE[i % MULTIBYTE.len()];
        let len = ch.len_utf8();
        if out.len() + len > n {
            break;
        }
        let mut buf = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        i += 1;
    }
    // Top up with ASCII so we sit exactly at capacity.
    while out.len() < n {
        out.push(b'x');
    }
    debug_assert!(core::str::from_utf8(&out).is_ok());
    out
}

// ---------------------------------------------------------------------------
// Capacity arithmetic (independent of qr-encode's private helpers)
// ---------------------------------------------------------------------------

/// The largest number of characters of `mode` that fit in `version` at `ec`.
///
/// Mirrors ISO/IEC 18004 7.4: 4 mode bits + character-count indicator + the
/// per-mode payload encoding, capped by the character-count indicator's own
/// range.
pub fn max_chars(mode: Mode, version: Version, ec: EcLevel) -> usize {
    let capacity_bits = ec_blocks(version, ec).total_data_codewords() as usize * 8;
    let cc = mode.char_count_bits(version).expect("supported mode") as usize;
    let avail = capacity_bits.saturating_sub(4 + cc);
    let by_bits = match mode {
        Mode::Numeric => {
            let triples = avail / 10;
            let rem = avail % 10;
            triples * 3 + if rem >= 7 { 2 } else if rem >= 4 { 1 } else { 0 }
        }
        Mode::Alphanumeric => {
            let pairs = avail / 11;
            let rem = avail % 11;
            pairs * 2 + usize::from(rem >= 6)
        }
        Mode::Byte => avail / 8,
        _ => 0,
    };
    let by_indicator = (1usize << cc) - 1;
    by_bits.min(by_indicator)
}

// ---------------------------------------------------------------------------
// Module placement walk (independent zigzag implementation)
// ---------------------------------------------------------------------------

/// The order in which data/EC bits occupy modules, per ISO/IEC 18004 7.7.3.
///
/// Two-module-wide columns are traversed right to left (skipping the vertical
/// timing column at `x == 6`); each column pair runs upward then downward,
/// alternating; within a row the right module precedes the left. Function
/// modules are skipped.
pub fn data_module_order(version: Version) -> Vec<(u32, u32)> {
    let size = version.size();
    let mut out = Vec::with_capacity((size * size) as usize);
    let mut col = size as i32 - 1;
    let mut upward = true;
    while col >= 1 {
        if col == 6 {
            col = 5; // the vertical timing column is not part of a pair
        }
        for k in 0..size {
            let y = if upward { size - 1 - k } else { k };
            for dx in 0..2u32 {
                let x = col as u32 - dx;
                if !is_function_module(version, x, y) {
                    out.push((x, y));
                }
            }
        }
        upward = !upward;
        col -= 2;
    }
    out
}

/// Reads the un-masked codeword stream straight out of a finished symbol, using
/// [`data_module_order`]. Trailing remainder bits are dropped.
pub fn codewords_from_matrix(m: &BitMatrix, version: Version, mask: u8) -> Vec<u8> {
    let order = data_module_order(version);
    let mut out = Vec::with_capacity(order.len() / 8);
    let mut acc = 0u8;
    let mut nbits = 0u8;
    for &(x, y) in &order {
        let bit = m.get(x, y) ^ mask_bit(mask, x, y);
        acc = (acc << 1) | u8::from(bit);
        nbits += 1;
        if nbits == 8 {
            out.push(acc);
            acc = 0;
            nbits = 0;
        }
    }
    out
}

/// Flips every module of interleaved codeword `index`, turning it into a
/// guaranteed byte error for Reed-Solomon.
pub fn corrupt_codeword(m: &mut BitMatrix, order: &[(u32, u32)], index: usize) {
    for &(x, y) in &order[index * 8..index * 8 + 8] {
        let v = m.get(x, y);
        m.set(x, y, !v);
    }
}

/// For each interleaved *data* codeword position, the block it belongs to.
///
/// Mirrors ISO/IEC 18004 7.6: data codewords are emitted column-by-column
/// across blocks, short (group 1) blocks dropping out of the final round.
pub fn data_block_indices(b: EcBlocks) -> Vec<usize> {
    let mut lens = Vec::with_capacity(b.total_blocks() as usize);
    for _ in 0..b.group1_blocks {
        lens.push(b.group1_data as usize);
    }
    for _ in 0..b.group2_blocks {
        lens.push(b.group2_data as usize);
    }
    let max_len = lens.iter().copied().max().unwrap_or(0);
    let mut out = Vec::with_capacity(b.total_data_codewords() as usize);
    for round in 0..max_len {
        for (bi, &len) in lens.iter().enumerate() {
            if round < len {
                out.push(bi);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Grayscale buffer + warps
// ---------------------------------------------------------------------------

/// A plain 8-bit grayscale buffer. Row-major, `w * h` samples.
#[derive(Clone)]
pub struct Gray {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl Gray {
    /// A `w x h` buffer filled with `fill`.
    pub fn new(w: u32, h: u32, fill: u8) -> Gray {
        Gray {
            w,
            h,
            px: vec![fill; (w as usize) * (h as usize)],
        }
    }

    /// Sets the sample at `(x, y)`; out-of-bounds writes are ignored.
    pub fn set(&mut self, x: u32, y: u32, v: u8) {
        if x < self.w && y < self.h {
            self.px[(y as usize) * (self.w as usize) + x as usize] = v;
        }
    }

    /// Reads the sample at `(x, y)`; out-of-bounds reads return `255`.
    pub fn get(&self, x: u32, y: u32) -> u8 {
        if x < self.w && y < self.h {
            self.px[(y as usize) * (self.w as usize) + x as usize]
        } else {
            255
        }
    }

    /// Bilinear sample in continuous coordinates; outside the buffer returns
    /// `bg`.
    pub fn sample(&self, x: f64, y: f64, bg: u8) -> u8 {
        if x < 0.0 || y < 0.0 || x > (self.w - 1) as f64 || y > (self.h - 1) as f64 {
            return bg;
        }
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(self.w - 1);
        let y1 = (y0 + 1).min(self.h - 1);
        let fx = x - x0 as f64;
        let fy = y - y0 as f64;
        let p = |xx: u32, yy: u32| self.get(xx, yy) as f64;
        let top = p(x0, y0) * (1.0 - fx) + p(x1, y0) * fx;
        let bot = p(x0, y1) * (1.0 - fx) + p(x1, y1) * fx;
        (top * (1.0 - fy) + bot * fy).round() as u8
    }

    /// Converts to an [`image::GrayImage`].
    pub fn to_image(&self) -> GrayImage {
        GrayImage::from_raw(self.w, self.h, self.px.clone()).expect("dimensions match buffer")
    }

    /// Wraps an [`image::GrayImage`].
    pub fn from_image(img: &GrayImage) -> Gray {
        Gray {
            w: img.width(),
            h: img.height(),
            px: img.as_raw().clone(),
        }
    }
}

/// Rasterises a matrix through `qr-render` and converts the result to
/// grayscale — the exact encode -> render -> detect path the suites exercise.
pub fn render_gray(matrix: &BitMatrix, scale: u32, quiet_zone: u32) -> Gray {
    let opts = qr_render::Options::default()
        .scale(scale)
        .quiet_zone(quiet_zone);
    let rgba: RgbaImage = qr_render::render_image(matrix, &opts);
    let luma = DynamicImage::ImageRgba8(rgba).to_luma8();
    Gray::from_image(&luma)
}

/// Embeds `src` centred in a larger white canvas with `margin` px on each side.
pub fn pad(src: &Gray, margin: u32) -> Gray {
    let mut out = Gray::new(src.w + 2 * margin, src.h + 2 * margin, 255);
    for y in 0..src.h {
        for x in 0..src.w {
            out.set(x + margin, y + margin, src.get(x, y));
        }
    }
    out
}

/// Rotates about the image centre by `angle` radians (inverse mapping, so the
/// output keeps the input dimensions — pad first for large angles).
pub fn rotate(src: &Gray, angle: f64) -> Gray {
    let cx = (src.w - 1) as f64 / 2.0;
    let cy = (src.h - 1) as f64 / 2.0;
    let (s, c) = angle.sin_cos();
    let mut out = Gray::new(src.w, src.h, 255);
    for y in 0..src.h {
        for x in 0..src.w {
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            out.set(x, y, src.sample(cx + dx * c + dy * s, cy - dx * s + dy * c, 255));
        }
    }
    out
}

/// Exact quarter-turn rotation (no resampling), `quarters` clockwise.
pub fn rotate_quarters(src: &Gray, quarters: u32) -> Gray {
    let q = quarters % 4;
    if q == 0 {
        return src.clone();
    }
    let (ow, oh) = if q == 2 { (src.w, src.h) } else { (src.h, src.w) };
    let mut out = Gray::new(ow, oh, 255);
    for y in 0..src.h {
        for x in 0..src.w {
            let v = src.get(x, y);
            let (nx, ny) = match q {
                1 => (src.h - 1 - y, x),
                2 => (src.w - 1 - x, src.h - 1 - y),
                _ => (y, src.w - 1 - x),
            };
            out.set(nx, ny, v);
        }
    }
    out
}

/// Rescales by `factor` (`< 1.0` shrinks) with bilinear sampling.
pub fn rescale(src: &Gray, factor: f64) -> Gray {
    let ow = ((src.w as f64) * factor).round().max(1.0) as u32;
    let oh = ((src.h as f64) * factor).round().max(1.0) as u32;
    let mut out = Gray::new(ow, oh, 255);
    for y in 0..oh {
        for x in 0..ow {
            let sx = x as f64 / factor;
            let sy = y as f64 / factor;
            out.set(x, y, src.sample(sx, sy, 255));
        }
    }
    out
}

/// Projective warp: the source rectangle's corners land on `dst_corners`
/// (top-left, top-right, bottom-right, bottom-left) in an `out_w x out_h`
/// canvas. Inverse-mapped, so no holes.
pub fn perspective(src: &Gray, dst_corners: [(f64, f64); 4], out_w: u32, out_h: u32) -> Gray {
    let src_corners = [
        (0.0, 0.0),
        ((src.w - 1) as f64, 0.0),
        ((src.w - 1) as f64, (src.h - 1) as f64),
        (0.0, (src.h - 1) as f64),
    ];
    let h = homography(dst_corners, src_corners);
    let mut out = Gray::new(out_w, out_h, 255);
    for y in 0..out_h {
        for x in 0..out_w {
            let (sx, sy) = apply_h(&h, x as f64, y as f64);
            out.set(x, y, src.sample(sx, sy, 255));
        }
    }
    out
}

/// Solves the 8-DOF homography mapping four `from` points onto four `to`
/// points.
fn homography(from: [(f64, f64); 4], to: [(f64, f64); 4]) -> [f64; 9] {
    let mut a = [[0.0f64; 8]; 8];
    let mut b = [0.0f64; 8];
    for i in 0..4 {
        let (x, y) = from[i];
        let (u, v) = to[i];
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y];
        b[2 * i] = u;
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y];
        b[2 * i + 1] = v;
    }
    let s = solve8(a, b);
    [s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7], 1.0]
}

fn apply_h(h: &[f64; 9], x: f64, y: f64) -> (f64, f64) {
    let w = h[6] * x + h[7] * y + h[8];
    (
        (h[0] * x + h[1] * y + h[2]) / w,
        (h[3] * x + h[4] * y + h[5]) / w,
    )
}

/// Gauss-Jordan elimination with partial pivoting for an 8x8 system.
fn solve8(mut a: [[f64; 8]; 8], mut b: [f64; 8]) -> [f64; 8] {
    for col in 0..8 {
        let mut piv = col;
        for r in (col + 1)..8 {
            if a[r][col].abs() > a[piv][col].abs() {
                piv = r;
            }
        }
        a.swap(col, piv);
        b.swap(col, piv);
        let d = a[col][col];
        for r in 0..8 {
            if r == col {
                continue;
            }
            let f = a[r][col] / d;
            #[allow(clippy::needless_range_loop)]
            for c in col..8 {
                a[r][c] -= f * a[col][c];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = [0.0f64; 8];
    for i in 0..8 {
        x[i] = b[i] / a[i][i];
    }
    x
}

/// Adds deterministic, roughly bell-shaped noise (sum of four uniforms) with
/// the given standard-deviation-ish `amplitude`.
pub fn add_noise(g: &mut Gray, amplitude: i32, seed: u64) {
    let mut rng = Rng::new(seed);
    for p in g.px.iter_mut() {
        let mut acc = 0i32;
        for _ in 0..4 {
            acc += (rng.next_u32() % (2 * amplitude as u32 + 1)) as i32 - amplitude;
        }
        *p = (*p as i32 + acc / 2).clamp(0, 255) as u8;
    }
}

/// Composites `code` into a white canvas with its top-left corner at
/// `(x, y)`; parts falling outside the canvas are clipped.
pub fn compose(canvas_w: u32, canvas_h: u32, code: &Gray, x: i64, y: i64) -> Gray {
    let mut out = Gray::new(canvas_w, canvas_h, 255);
    for sy in 0..code.h {
        let ty = y + sy as i64;
        if ty < 0 || ty >= canvas_h as i64 {
            continue;
        }
        for sx in 0..code.w {
            let tx = x + sx as i64;
            if tx < 0 || tx >= canvas_w as i64 {
                continue;
            }
            out.set(tx as u32, ty as u32, code.get(sx, sy));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Deterministic PRNG
// ---------------------------------------------------------------------------

/// xorshift64* — deterministic, seedable, and good enough to shuffle test
/// inputs reproducibly.
pub struct Rng(u64);

impl Rng {
    /// Creates a generator; a zero seed is replaced (xorshift has no escape
    /// from the all-zero state).
    pub fn new(seed: u64) -> Rng {
        Rng(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    /// Next 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Next 32-bit value.
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform-ish value in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}
