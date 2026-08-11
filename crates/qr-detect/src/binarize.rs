//! Grayscale-to-binary conversion.
//!
//! Primary path is Bradley-style adaptive thresholding: each pixel is compared
//! against the mean of a local window, computed in O(1) from a sliding column
//! sum and a per-row prefix (see [`adaptive_at`]). For very small images — or
//! when the adaptive result degenerates to a single colour — we fall back to a
//! global Otsu threshold.
//!
//! Two things about that comparison are parameters rather than constants, and
//! for the same reason: no single setting of either holds every envelope, and
//! the conflicts are resolved a layer up by running the frame twice.
//!
//! * The **window radius** — [`PRIMARY_WINDOW_DIV`] or [`RETRY_WINDOW_DIV`]. A
//!   wide window is required to keep solid features solid at any print contrast;
//!   a narrow window is required to track a steep illumination gradient.
//! * The **margin** — [`PRIMARY_MARGIN`] or [`NARROW_MARGIN`] — being how far
//!   below its local mean a pixel must sit to count as dark. A wide margin is
//!   required to keep sensor noise from speckling a well-lit frame; a narrow one
//!   is required to see a symbol whose print contrast is thin. See
//!   [`PRIMARY_MARGIN`] for the measurement that says those two cross.
//!
//! A third degradation, sensor noise, is not a setting of the comparison at all
//! and is answered before it: [`denoise`] smooths the frame, on the retry rung
//! and only when [`noise_reaches`] says there is noise to remove.
//!
//! The pipeline binarizes at the primary settings and, only when that yields
//! nothing at all, runs [`binarize_retry`] over the whole frame — every one of
//! the three moved at once. See the crate docs for what that costs.

use core::ops::{Add, Mul, Sub};

/// A binarized image. `true` = dark module, `false` = light. Coordinates are
/// `(x, y)` = (column, row), origin top-left.
pub struct Binary {
    width: u32,
    height: u32,
    dark: Vec<bool>,
    radius_invariant: bool,
}

impl Binary {
    /// Width in pixels.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Whether this image would come out identical at any other window radius.
    ///
    /// True when the adaptive pass was never run because the frame ruled it out
    /// — the image is too small for a window to mean anything, or its dynamic
    /// range is too narrow for any pixel to fall below its local mean (see
    /// [`can_be_dark`]). Both conditions are properties of the frame alone, and
    /// the global fallback that produced this image does not take a radius, so a
    /// second pass at the retry radius would recompute this exact result.
    ///
    /// The pipeline uses it to skip that pass. It is not merely "the fallback
    /// ran": when the fallback runs *because the adaptive pass came back empty*,
    /// another radius genuinely might not, and this stays false.
    ///
    /// The retry also changes the *margin*, and the [`can_be_dark`] condition
    /// does depend on that, so on that arm this is a deliberate judgement rather
    /// than a proof — see [`can_be_dark`] for why a frame narrow enough to reach
    /// it is one the [`otsu`] fallback already answers better than any adaptive
    /// pass would.
    #[inline]
    pub fn is_radius_invariant(&self) -> bool {
        self.radius_invariant
    }

    /// Row `y` as a contiguous slice, left to right.
    ///
    /// The finder scan is the only stage that reads every pixel, and reading
    /// them one [`get`](Self::get) at a time costs it a bounds check and an
    /// `i64 -> usize` conversion per pixel. Handing it the row lets it walk a
    /// slice instead.
    #[inline]
    pub fn row(&self, y: u32) -> &[bool] {
        let w = self.width as usize;
        &self.dark[(y as usize) * w..(y as usize) * w + w]
    }

    /// Returns whether the pixel at `(x, y)` is dark. Out-of-bounds reads are
    /// treated as light (`false`).
    #[inline]
    pub fn get(&self, x: i64, y: i64) -> bool {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return false;
        }
        self.dark[(y as usize) * (self.width as usize) + (x as usize)]
    }
}

/// Window divisor for the primary pass: radius ~1/8 of the smaller side.
///
/// Wide enough that a window never fits inside the largest solid feature a
/// symbol can present — a finder pattern's 3-module core — which is the
/// condition for that core's interior to hollow out. See
/// [`tests::a_solid_block_does_not_hollow_out`].
pub const PRIMARY_WINDOW_DIV: usize = 8;

/// Window divisor for the retry pass: radius ~1/16 of the smaller side.
///
/// Tracks uneven illumination roughly twice as closely as the primary, which is
/// what a small low-contrast symbol in the corner of a vignetted frame needs.
/// It cannot be the primary: at this radius a large finder core hollows out.
/// See [`tests::the_retry_window_holds_a_vignette_the_primary_loses`].
pub const RETRY_WINDOW_DIV: usize = 16;

/// The denominator every Bradley margin is expressed over.
///
/// Fixed at 25 rather than 100 so both margins reduce against it, which is what
/// keeps [`fits_u32`]'s overflow bound — and therefore the accumulator-width
/// routing — the same arithmetic it has always been.
const MARGIN_DEN: u32 = 25;

/// Margin for the primary pass: a pixel is dark when it sits at least **12%**
/// below its local mean (`22/25`).
///
/// This is Bradley's own `t`, and it is the right setting for a well-lit frame:
/// the margin is what stops sensor noise from speckling a flat region, and 12%
/// of a paper-white local mean is ~30 luma of protection.
///
/// It is the wrong setting for a thin-contrast one, and measurably so. The test
/// is *relative*, so the contrast a symbol must present to clear it scales with
/// the background: against a background `B` with a data region about half dark,
/// a dark module at `B - d` is only seen when `d > 0.47 * B`. At `B = 220` that
/// is `d > 47` — so an ordinary worn or dimly-lit print at `d = 40` binarizes to
/// nothing at all, and no amount of error correction downstream recovers a
/// symbol whose modules were never distinguished. Measured over a 720-cell
/// degradation grid (four symbols x three scales x three illumination models x
/// four contrasts x five noise levels), this margin binarizes 100 cells
/// perfectly; [`NARROW_MARGIN`] binarizes 196.
///
/// The two do not order, which is why both exist. Under sensor noise at *full*
/// contrast the ranking reverses — of the 108 grid cells at the widest contrast
/// and sigma >= 8, this margin beats the narrow one on 57. A wide margin is
/// noise protection and a narrow margin is contrast sensitivity; a frame needs
/// one or the other, so the pipeline tries one and then the other.
pub const PRIMARY_MARGIN: u32 = 22;

/// Margin for the retry pass: **4%** below the local mean (`24/25`).
///
/// Chosen by sweeping the whole margin range over the degradation grid described
/// on [`PRIMARY_MARGIN`], which is flat across 96-97% and falls away either side
/// — 92% is still losing thin-contrast symbols, 100% (no margin at all) has
/// nothing left to reject noise with and drops back below both. 96% is the
/// setting inside that plateau that reduces against [`MARGIN_DEN`].
pub const NARROW_MARGIN: u32 = 24;

/// Row stride of the [`noise_reaches`] noise estimate.
///
/// The statistic is a median over hundreds of thousands of samples even at this
/// stride, and it reads the same value here as it does at stride 1 on every
/// fixture it was calibrated against — so the remaining fifteen-sixteenths of
/// the frame buy nothing but memory traffic on a path that is already a miss.
const NOISE_ROW_STRIDE: usize = 16;

/// Which quantile of the second difference [`noise_reaches`] reads, as a
/// reciprocal: 4 is the lower quartile.
///
/// Not the median, and the reason is the one clean frame a high-frequency
/// statistic finds hardest to call quiet. A pixel in the *interior* of a module
/// run has a second difference of exactly zero; one on a module boundary has a
/// large one. At `s` px/module the interior fraction is at worst `(s-2)/s` —
/// one third at the 3 px/module the pinned deep-vignette fixture renders at, and
/// that is the strict alternating case, with a real pseudo-random data region
/// running higher. A median needs *half* the samples quiet and would call that
/// frame noisy; a lower quartile needs a quarter and does not.
///
/// It costs nothing to read a quarter instead of a half — it is the same
/// counter against a different fraction — and it is the whole margin the gate
/// has on the side where being wrong is expensive.
const NOISE_QUANTILE: u32 = 4;

/// The noise floor at or above which the retry rung smooths the frame, in the
/// units [`noise_reaches`] measures.
///
/// Measured lower quartiles: a noise-free frame reads **0** at every module
/// pitch from 3 px up, and at every symbol size and illumination model on the
/// degradation grid; sigma 4 reads 4, sigma 8 reads 8, sigma 12 reads 11, sigma
/// 16 reads 15. 3 sits in the gap, and the gap is what the gate is: the
/// smoothing starts to pay from about sigma 4, and every clean frame must stay
/// on the unsmoothed path.
const DENOISE_TRIGGER: u32 = 3;

/// Whether the frame's noise floor reaches `threshold` — the lower quartile (see
/// [`NOISE_QUANTILE`]) of `|2*p[x] - p[x-1] - p[x+1]|` over every
/// [`NOISE_ROW_STRIDE`]-th row.
///
/// The second difference annihilates anything locally linear — flat paper, and
/// any illumination gradient or vignette, which is what makes this an estimate
/// of *noise* rather than of image energy. What survives it is noise and real
/// edges, and a low quantile is what separates those: an edge is a large
/// contribution from a minority of pixels, and a quantile ignores minorities
/// however large their values are. Noise, by contrast, is in every pixel,
/// including the ones in the middle of a module where nothing else is happening
/// — which is exactly what a *floor* measures.
///
/// For white noise of standard deviation `s` the second difference has variance
/// `6 s^2`; the lower quartile of its absolute value is about `0.79 * s`.
/// Measured on the degradation grid: sigma 4 gives 4, sigma 8 gives 8, sigma 12
/// gives 11, sigma 16 gives 15.
///
/// # Why a comparison and not the quantile itself
///
/// The caller only ever asks whether the floor clears one constant, and the
/// difference in how that is computed is not a micro-optimisation. Tallying the
/// differences into a histogram to read a quantile off it is the exact shape
/// this file already documents at [`HISTOGRAM_LANES`]: on a smooth frame nearly
/// every sample lands in the same bin, and the whole loop serialises on
/// store-to-load forwarding. Measured, that cost the miss path 110 us — a tenth
/// of it — for a statistic that reads 57 000 pixels. Counting how many samples
/// fall *below* the threshold answers the same question with one accumulator and
/// no memory-dependent indexing, and the same benchmark moves by 2%.
///
/// `below * NOISE_QUANTILE < n` is "strictly fewer than a quarter of the samples
/// are under the threshold", which is the lower quartile reaching it. Strict, so
/// an exact quarter reads as quiet — ties resolve to leaving the frame alone.
///
/// Returns `false` for a buffer whose length does not match its dimensions, or
/// one too narrow to hold a second difference.
pub fn noise_reaches(luma: &[u8], width: u32, height: u32, threshold: u32) -> bool {
    let (w, h) = (width as usize, height as usize);
    if w < 3 || h == 0 || luma.len() != w * h {
        return false;
    }
    let (mut below, mut n) = (0u32, 0u32);
    for y in (0..h).step_by(NOISE_ROW_STRIDE) {
        for win in luma[y * w..y * w + w].windows(3) {
            let d = 2 * i32::from(win[1]) - i32::from(win[0]) - i32::from(win[2]);
            below += u32::from(d.unsigned_abs() < threshold);
            n += 1;
        }
    }
    below * NOISE_QUANTILE < n
}

/// Smooths `luma` with a symmetric 3x3 box, returning a new buffer.
///
/// The retry rung runs this over the frame before binarizing it — when
/// [`noise_reaches`] says the frame is noisy — and it is the only thing in the
/// pipeline that addresses sensor noise rather than illumination or contrast.
/// Noise is not a small perturbation of the dark test:
/// the test compares one pixel against a window mean over thousands, so the mean
/// is already noise-free and the whole of a pixel's noise lands on one side of
/// the comparison. At sigma 12 that is enough to flip module centres in bulk —
/// measured at the retry settings over the degradation grid, mean module errors
/// at full contrast and sigma 12 run 20.5 unsmoothed against 0.3 smoothed, and
/// the count of grid cells that binarize perfectly goes 0 of 36 to 35 of 36.
///
/// Averaging nine samples divides the noise by three while leaving the signal
/// alone anywhere the window sits inside one module, which is why a box this
/// small is enough and why it does not cost resolution: the sampler reads module
/// *centres*, and a symmetric kernel does not move them. Measured down to
/// 4 px/module, the smallest this workspace's fixtures render, the smoothed
/// error is never the larger of the two.
///
/// It is separable, so the cost is two elementwise passes rather than nine
/// multiplies per pixel, and it is on the retry rung alone — a frame that
/// decoded on the primary pass never pays for it.
///
/// It is also gated, and has to be. Averaging over a 3 px window costs nothing
/// while that window sits inside a module, and everything once it spans one: at
/// 3 px/module — which is where the pinned deep-vignette fixture renders — an
/// unconditional smoothing loses the symbol the retry exists to recover. The
/// gate is what keeps every noise-free frame on exactly the path it had before,
/// and every noise-free fixture measured has a median of 0.
///
/// The one-pixel border keeps its original value; the alternative is inventing
/// samples off the edge of the frame, and no symbol is ever found in a border
/// that thin.
///
/// Returns `None` on the same dimension mismatch [`binarize_with`] rejects.
pub fn denoise(luma: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || luma.len() != w * h {
        return None;
    }
    if w < 3 || h < 3 {
        return Some(luma.to_vec());
    }

    // Horizontal, into `tmp`; then vertical, into `out`. Both are flat walks
    // over slices with no per-pixel bounds check.
    let mut tmp = luma.to_vec();
    for y in 0..h {
        let src = &luma[y * w..y * w + w];
        let dst = &mut tmp[y * w..y * w + w];
        for (d, win) in dst[1..w - 1].iter_mut().zip(src.windows(3)) {
            *d = ((u32::from(win[0]) + u32::from(win[1]) + u32::from(win[2]) + 1) / 3) as u8;
        }
    }

    let mut out = tmp.clone();
    for y in 1..h - 1 {
        let top = &tmp[(y - 1) * w + 1..(y - 1) * w + w - 1];
        let mid = &tmp[y * w + 1..y * w + w - 1];
        let bot = &tmp[(y + 1) * w + 1..(y + 1) * w + w - 1];
        let dst = &mut out[y * w + 1..y * w + w - 1];
        for (d, ((&a, &b), &c)) in dst.iter_mut().zip(top.iter().zip(mid).zip(bot)) {
            *d = ((u32::from(a) + u32::from(b) + u32::from(c) + 1) / 3) as u8;
        }
    }
    Some(out)
}

/// Binarizes a luma buffer at the primary Bradley margin.
///
/// The four-argument form, and what the pipeline's primary pass runs. See
/// [`binarize_with`] for the parameters and the return contract.
pub fn binarize(luma: &[u8], width: u32, height: u32, window_div: usize) -> Option<Binary> {
    binarize_with(luma, width, height, window_div, PRIMARY_MARGIN)
}

/// Binarizes a luma buffer at every one of the retry rung's settings.
///
/// The retry differs from the primary in three things, and this is the one place
/// all three are stated: a narrower window ([`RETRY_WINDOW_DIV`]), a narrower
/// margin ([`NARROW_MARGIN`]), and — on a frame [`noise_reaches`] calls noisy — a
/// [`denoise`] pre-pass. Each answers a different way a camera frame goes wrong,
/// and they are on the same rung rather than on three because a frame that has
/// already failed the primary pass is worth exactly one more attempt, not three.
pub fn binarize_retry(luma: &[u8], width: u32, height: u32) -> Option<Binary> {
    let settings = |buf: &[u8]| binarize_with(buf, width, height, RETRY_WINDOW_DIV, NARROW_MARGIN);
    if noise_reaches(luma, width, height, DENOISE_TRIGGER) {
        settings(&denoise(luma, width, height)?)
    } else {
        settings(luma)
    }
}

/// Binarizes a luma buffer (`width * height` bytes, row-major).
///
/// `window_div` selects the adaptive window radius as a fraction of the smaller
/// image side — [`PRIMARY_WINDOW_DIV`] or [`RETRY_WINDOW_DIV`] — and `margin`
/// how far below its local mean a pixel must sit to be dark, over
/// [`MARGIN_DEN`] — [`PRIMARY_MARGIN`] or [`NARROW_MARGIN`]. Both are
/// parameters rather than constants because no single setting of either holds
/// every envelope; the pipeline runs the primary settings and, only if it finds
/// nothing at all, tries again at the retry settings.
///
/// `window_div` must be at least 3, so that the window fits inside any frame
/// this reaches the adaptive pass with (`2 * radius + 1 <= min(w, h)`), and
/// `margin` at most [`MARGIN_DEN`], which is what the all-dark argument in the
/// body rests on. Both hold for every constant above; they are debug-asserted
/// rather than checked because the module is private and the call sites are
/// this file and the pipeline.
///
/// Returns `None` if the dimensions do not match the buffer length or the image
/// is empty.
pub fn binarize_with(
    luma: &[u8],
    width: u32,
    height: u32,
    window_div: usize,
    margin: u32,
) -> Option<Binary> {
    debug_assert!(window_div >= 3, "window_div {window_div} is too small");
    debug_assert!(
        margin <= MARGIN_DEN,
        "margin {margin} exceeds the denominator"
    );
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || luma.len() != w * h {
        return None;
    }

    // Small images: adaptive windows are meaningless, use a global threshold.
    // Note this makes the retry pass a no-op for them — both passes land on the
    // same Otsu result — which is correct: there is no second opinion to have.
    // `radius_invariant` is what tells the pipeline so.
    if width < 24 || height < 24 {
        return Some(otsu(luma, width, height, true));
    }

    // A frame whose dynamic range is too narrow cannot produce a single dark
    // pixel on the adaptive pass, and we can prove that from two bytes rather
    // than from a full window evaluation per pixel. Skipping straight to the
    // fallback here is what a blank camera frame used to pay an entire wasted
    // adaptive pass for.
    if !can_be_dark(luma, margin) {
        return Some(otsu(luma, width, height, true));
    }

    let bin = adaptive(luma, width, height, window_div, margin);

    // Guard against a degenerate all-light result (e.g. a flat gradient with no
    // local dip anywhere) by falling back to Otsu. `any` rather than a count:
    // the only degenerate case reachable here is "no dark pixel at all", so a
    // frame with content stops on its first dark pixel instead of walking the
    // whole buffer.
    //
    // The mirror case — *every* pixel dark — cannot occur and is not tested for.
    // Take the pixel holding the frame's maximum value `m`: every pixel in its
    // window is at most `m`, so `sum <= count * m`, and its own test reads
    // `m * count * MARGIN_DEN < sum * margin <= count * m * margin`. Since
    // `margin <= MARGIN_DEN` that is false, at either margin. The brightest
    // pixel in a frame is never dark.
    if !bin.dark.iter().any(|&d| d) {
        return Some(otsu(luma, width, height, false));
    }
    Some(bin)
}

/// Whether the adaptive pass could mark *any* pixel dark at `margin`, decided
/// from the frame's dynamic range alone.
///
/// A pixel is dark when `value * count * MARGIN_DEN < sum * margin`. Every pixel
/// in a window is at most the frame maximum, so `sum <= count * max`; every
/// `value` is at least the frame minimum. When `min * MARGIN_DEN >= max * margin`
/// the test is therefore false at every pixel, and the adaptive pass is a
/// guaranteed no-op.
///
/// The bound reads only the extremes, so it does not mention the window radius:
/// a frame that fails it fails it at *every* radius. That is what makes the
/// result [radius-invariant](Binary::is_radius_invariant) and lets the pipeline
/// skip its retry pass on such frames.
///
/// It *does* mention the margin, and the pipeline's retry changes the margin as
/// well as the radius, so skipping the retry on this ground is a judgement and
/// not a proof. It is a sound one, and the bound itself is what shows why. A
/// frame that fails at [`PRIMARY_MARGIN`] has under 12% of global dynamic range;
/// illumination and reflectance both live inside that range, so the light across
/// such a frame varies by at most 12% end to end and no surface in it reflects
/// under 88% of any other. That is the definition of a globally flat scene, and
/// on a globally flat scene one global threshold is not an approximation of the
/// adaptive answer — it is the better answer, and it is exactly what the [`otsu`]
/// arm below returns. Pinned by
/// [`tests::a_thin_contrast_symbol_on_a_flat_frame_goes_through_otsu`].
fn can_be_dark(luma: &[u8], margin: u32) -> bool {
    let (mut lo, mut hi) = (u8::MAX, u8::MIN);
    for &p in luma {
        lo = lo.min(p);
        hi = hi.max(p);
    }
    u32::from(lo) * MARGIN_DEN < u32::from(hi) * margin
}

/// The integer width the window sums are accumulated at.
///
/// The adaptive pass is memory- and throughput-bound, and its threshold loop
/// only vectorizes at a width the target has vector multiply for. `u32` is that
/// width and covers every frame up to 4K; `u64` is the fallback for images too
/// large to keep the products in 32 bits. Both instantiations run identical
/// logic, so they agree bit for bit — pinned by
/// [`tests::the_two_accumulator_widths_agree`].
trait Acc: Copy + Ord + Add<Output = Self> + Sub<Output = Self> + Mul<Output = Self> {
    const ZERO: Self;
    fn from_luma(v: u8) -> Self;
    /// `n` is a pixel count or one of the two small Bradley constants. Callers
    /// reach the `u32` instantiation only through [`fits_u32`], which is what
    /// makes the narrowing conversion exact.
    fn from_count(n: usize) -> Self;
}

impl Acc for u32 {
    const ZERO: Self = 0;
    #[inline]
    fn from_luma(v: u8) -> Self {
        Self::from(v)
    }
    #[inline]
    #[allow(clippy::cast_possible_truncation)]
    fn from_count(n: usize) -> Self {
        n as Self
    }
}

impl Acc for u64 {
    const ZERO: Self = 0;
    #[inline]
    fn from_luma(v: u8) -> Self {
        Self::from(v)
    }
    #[inline]
    fn from_count(n: usize) -> Self {
        n as Self
    }
}

/// Whether every product the `u32` accumulator has to hold fits in 32 bits.
///
/// Three quantities are bounded: the running row prefix, which reaches the whole
/// frame's luma sum (`w * h * 255`); the scaled window sum
/// (`count * 255 * margin`); and the pixel side of the comparison
/// (`255 * count * MARGIN_DEN`). The last binds — and it binds independently of
/// which margin is in force, since every margin is at most [`MARGIN_DEN`], which
/// is why this takes no margin argument.
///
/// Written as divisions so the check itself cannot overflow.
fn fits_u32(w: usize, h: usize, radius: usize) -> bool {
    let side = 2 * radius + 1;
    let count = side.min(w) * side.min(h);
    let limit = u32::MAX as usize;
    w * h <= limit / 255 && count <= limit / (255 * 25)
}

/// Bradley adaptive threshold, with the window radius set to `1/window_div` of
/// the smaller image side and the dark test set `margin/MARGIN_DEN` below the
/// local mean.
fn adaptive(luma: &[u8], width: u32, height: u32, window_div: usize, margin: u32) -> Binary {
    let (w, h) = (width as usize, height as usize);
    // Window radius ~ 1/window_div of the smaller side, at least 4 px. Since
    // `window_div >= 3` and the caller has ruled out sides below 24, the window
    // is never wider than the frame: `2 * radius + 1 <= min(w, h)`.
    let radius = (w.min(h) / window_div.max(1)).max(4);

    let dark = if fits_u32(w, h, radius) {
        adaptive_at::<u32>(luma, w, h, radius, margin)
    } else {
        adaptive_at::<u64>(luma, w, h, radius, margin)
    };

    Binary {
        width,
        height,
        dark,
        radius_invariant: false,
    }
}

/// One row of luma folded into the per-column window sums.
#[inline]
fn add_row<A: Acc>(colsum: &mut [A], row: &[u8]) {
    for (c, &p) in colsum.iter_mut().zip(row) {
        *c = *c + A::from_luma(p);
    }
}

/// The inverse of [`add_row`], for the row leaving the window's top.
#[inline]
fn sub_row<A: Acc>(colsum: &mut [A], row: &[u8]) {
    for (c, &p) in colsum.iter_mut().zip(row) {
        *c = *c - A::from_luma(p);
    }
}

/// Elements per block in [`prefix_sum`].
const PREFIX_BLOCK: usize = 8;

/// Prefix sum of `src` into `dst[1..]`, with `dst[0] = 0`.
///
/// Every other loop in the adaptive pass is a flat elementwise walk; this one
/// carries a dependency, so at 1280 columns per row it is what sets the row's
/// latency rather than its throughput. Blocking is what breaks that up: each
/// block's internal prefix is computed from zero, so consecutive blocks are
/// independent and the core can overlap them, and only the one add that folds
/// the running carry into the next block stays on the critical path — one add
/// per [`PREFIX_BLOCK`] elements instead of one per element.
fn prefix_sum<A: Acc>(src: &[A], dst: &mut [A]) {
    debug_assert_eq!(dst.len(), src.len() + 1);
    dst[0] = A::ZERO;
    let mut carry = A::ZERO;

    let mut blocks = src.chunks_exact(PREFIX_BLOCK);
    let mut out = dst[1..].chunks_exact_mut(PREFIX_BLOCK);
    for (block, slot) in blocks.by_ref().zip(out.by_ref()) {
        let mut local = [A::ZERO; PREFIX_BLOCK];
        let mut acc = A::ZERO;
        for (l, &s) in local.iter_mut().zip(block) {
            acc = acc + s;
            *l = acc;
        }
        for (d, &l) in slot.iter_mut().zip(local.iter()) {
            *d = carry + l;
        }
        carry = carry + acc;
    }

    for (d, &s) in out.into_remainder().iter_mut().zip(blocks.remainder()) {
        carry = carry + s;
        *d = carry;
    }
}

/// The adaptive threshold itself, at a chosen accumulator width.
///
/// No integral image. The window sum is separated into a vertical and a
/// horizontal half: `colsum[x]` holds the sum of the window's rows in column
/// `x` and is slid down the frame one row at a time (add the row entering the
/// bottom, subtract the one leaving the top), then a prefix sum along `colsum`
/// turns any horizontal span into one subtraction. State is two `w`-element
/// buffers that stay in cache, instead of a `(w+1) * (h+1)` integral image —
/// 7.4 MB of traffic per pass at 720p, which is what made this loop
/// bandwidth-bound.
///
/// The payoff is in the shape as much as the size: the row slide, the prefix
/// and the threshold comparison are each a flat walk over slices with no
/// per-pixel bounds check and no data-dependent indexing, and the interior of
/// every row — everywhere the window does not hit a frame edge — has a constant
/// pixel `count`, so the comparison is a single loop over three slices that the
/// backend can vectorize.
fn adaptive_at<A: Acc>(luma: &[u8], w: usize, h: usize, radius: usize, margin: u32) -> Vec<bool> {
    // The edge/interior split below assumes the window never spans the whole
    // frame; callers guarantee it via the window_div >= 8 / min-side rules.
    debug_assert!(
        w > 2 * radius && h > 2 * radius,
        "adaptive window ({radius}) must fit inside the frame ({w}x{h})"
    );
    let mut colsum = vec![A::ZERO; w];
    let mut prefix = vec![A::ZERO; w + 1];
    let mut dark = vec![false; w * h];

    // Bradley "t" parameter: a pixel is dark when it is far enough below the
    // local mean, i.e. `value * count * MARGIN_DEN < sum * margin`. Both
    // margins are already expressed over 25 rather than 100, which is exact for
    // each of them and buys four bits of headroom in the accumulator.
    let t_num = A::from_count(margin as usize);
    let t_den = A::from_count(MARGIN_DEN as usize);

    // Rows currently folded into `colsum`, as a half-open range.
    let (mut top, mut bottom) = (0usize, 0usize);

    for y in 0..h {
        let y0 = y.saturating_sub(radius);
        let y1 = (y + radius).min(h - 1);
        while bottom <= y1 {
            add_row(&mut colsum, &luma[bottom * w..bottom * w + w]);
            bottom += 1;
        }
        while top < y0 {
            sub_row(&mut colsum, &luma[top * w..top * w + w]);
            top += 1;
        }
        let rows = y1 - y0 + 1;

        prefix_sum(&colsum, &mut prefix);

        let lrow = &luma[y * w..y * w + w];
        let drow = &mut dark[y * w..y * w + w];

        // Interior: the window is fully inside the frame horizontally, so
        // `count` is the same at every column and the whole span is one flat
        // loop. `w >= 2 * radius + 1` (see `adaptive`), so this span is never
        // empty and neither prefix slice underflows.
        let (lo, hi) = (radius, w - radius);
        let k = A::from_count(rows * (2 * radius + 1)) * t_den;
        let (ahead, behind) = (&prefix[2 * radius + 1..=w], &prefix[..=w - 2 * radius - 1]);
        for (d, (&v, (&a, &b))) in drow[lo..hi]
            .iter_mut()
            .zip(lrow[lo..hi].iter().zip(ahead.iter().zip(behind)))
        {
            *d = A::from_luma(v) * k < (a - b) * t_num;
        }

        // Edges: the window is clamped against a frame border, so `count` grows
        // (left) or shrinks (right) by `rows` per column. Written as two flat
        // loops with the clamp already resolved rather than one loop that
        // re-derives the bounds per pixel — these are only `2 * radius` of the
        // row's columns but they were costing as much as the whole interior.
        let left_sums = &prefix[radius + 1..=2 * radius];
        for (i, ((d, &v), &s)) in drow[..lo]
            .iter_mut()
            .zip(&lrow[..lo])
            .zip(left_sums)
            .enumerate()
        {
            // x = i, window is columns 0 ..= i + radius.
            let count = A::from_count(rows * (i + radius + 1));
            *d = A::from_luma(v) * count * t_den < s * t_num;
        }

        let total = prefix[w];
        let right_sums = &prefix[w - 2 * radius..w - radius];
        for (i, ((d, &v), &s)) in drow[hi..]
            .iter_mut()
            .zip(&lrow[hi..])
            .zip(right_sums)
            .enumerate()
        {
            // x = hi + i, window is columns x - radius ..= w - 1.
            let count = A::from_count(rows * (2 * radius - i));
            *d = A::from_luma(v) * count * t_den < (total - s) * t_num;
        }
    }

    dark
}

/// Number of interleaved sub-histograms in [`histogram`].
///
/// A single `hist[p] += 1` is a load and a store to the same address whenever
/// two neighbouring pixels share a value, so a flat or near-flat frame — the
/// one that reaches Otsu most often — serialises the whole buffer on
/// store-to-load forwarding latency. Independent tallies give the core that
/// many chains to interleave, and the merge is one pass over 1024 counters.
///
/// Four, measured: on a uniform 720p frame the tally goes 1141 µs at one lane
/// to 179 µs at four, and back up past eight as the lane arrays start costing
/// more in cache and address arithmetic than the chains they break. A textured
/// frame sits at ~180 µs throughout — it has no chain to break — so the choice
/// is made entirely by the degenerate case, which is the one that reaches here.
const HISTOGRAM_LANES: usize = 4;

/// Luma histogram, tallied in [`HISTOGRAM_LANES`] independent lanes.
fn histogram(luma: &[u8]) -> [u64; 256] {
    let mut lanes = [[0u64; 256]; HISTOGRAM_LANES];
    let mut chunks = luma.chunks_exact(HISTOGRAM_LANES);
    for chunk in &mut chunks {
        for (lane, &p) in lanes.iter_mut().zip(chunk) {
            lane[p as usize] += 1;
        }
    }
    for &p in chunks.remainder() {
        lanes[0][p as usize] += 1;
    }

    let mut hist = [0u64; 256];
    for lane in &lanes {
        for (h, &c) in hist.iter_mut().zip(lane) {
            *h += c;
        }
    }
    hist
}

/// Global Otsu threshold. Pixels with value `<=` the threshold are dark.
///
/// `radius_invariant` records whether the caller reached this fallback for a
/// reason that holds at every window radius — see
/// [`Binary::is_radius_invariant`].
fn otsu(luma: &[u8], width: u32, height: u32, radius_invariant: bool) -> Binary {
    let hist = histogram(luma);
    let total = luma.len() as u64;

    let sum_total: u64 = hist.iter().enumerate().map(|(i, &c)| i as u64 * c).sum();
    let mut sum_back = 0u64;
    let mut weight_back = 0u64;
    let mut max_between = -1.0f64;
    let mut threshold = 127u8;

    for (i, &count) in hist.iter().enumerate() {
        weight_back += count;
        if weight_back == 0 {
            continue;
        }
        let weight_fore = total - weight_back;
        if weight_fore == 0 {
            break;
        }
        sum_back += i as u64 * count;
        let mean_back = sum_back as f64 / weight_back as f64;
        let mean_fore = (sum_total - sum_back) as f64 / weight_fore as f64;
        let diff = mean_back - mean_fore;
        let between = weight_back as f64 * weight_fore as f64 * diff * diff;
        if between > max_between {
            max_between = between;
            threshold = i as u8;
        }
    }

    let dark = luma.iter().map(|&p| p <= threshold).collect();
    Binary {
        width,
        height,
        dark,
        radius_invariant,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn otsu_splits_bimodal() {
        // 16x2 image: left half black (10), right half white (240).
        let w = 16u32;
        let h = 2u32;
        let mut luma = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                luma[(y * w + x) as usize] = if x < 8 { 10 } else { 240 };
            }
        }
        let b = binarize(&luma, w, h, PRIMARY_WINDOW_DIV).unwrap();
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                assert_eq!(b.get(x, y), x < 8, "pixel ({x},{y})");
            }
        }
    }

    #[test]
    fn the_two_accumulator_widths_agree() {
        // `adaptive` picks `u32` for anything up to 4K and `u64` beyond, and no
        // fixture in this workspace is large enough to reach the wide path, so
        // this is the only thing standing between it and silent rot. Both are
        // driven over the same frame — a vignette with dense worn-print modules,
        // which exercises the whole range of window sums — and compared bit for
        // bit.
        let (w, h) = (320usize, 240usize);
        let mut luma = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f64 - 160.0, y as f64 - 120.0);
                let fall = 1.0 - 0.6 * (dx * dx + dy * dy) / (160.0 * 160.0 + 120.0 * 120.0);
                let module = (x / 3 + y / 3) % 3 == 0;
                let refl = if module { 0.55 } else { 1.0 };
                luma[y * w + x] = (255.0 * fall * refl).round().clamp(0.0, 255.0) as u8;
            }
        }
        for radius in [4usize, 15, 30] {
            for margin in [PRIMARY_MARGIN, NARROW_MARGIN] {
                let narrow = adaptive_at::<u32>(&luma, w, h, radius, margin);
                let wide = adaptive_at::<u64>(&luma, w, h, radius, margin);
                assert_eq!(
                    narrow, wide,
                    "widths disagree at radius {radius}, margin {margin}"
                );
                // Guard the guard: a fixture that binarized to one flat colour
                // would let two broken implementations agree.
                assert!(narrow.iter().any(|&d| d) && narrow.iter().any(|&d| !d));
            }
        }
    }

    #[test]
    fn a_frame_too_flat_to_threshold_is_settled_at_any_radius() {
        // The pipeline skips its retry pass on these frames, and that is only
        // sound if the two radii genuinely agree on them. Both halves are
        // pinned: the flag the pipeline reads, and the output it is a claim
        // about.
        //
        // 232 on 255 is a 9% spread — under the 12% Bradley needs — so it stands
        // for the near-blank camera frame rather than only the perfectly uniform
        // one, which any implementation would get right by accident.
        let (w, h) = (96u32, 96u32);
        for (label, fill) in [("uniform", 255u8), ("near-flat", 232)] {
            let mut luma = vec![255u8; (w * h) as usize];
            for (i, p) in luma.iter_mut().enumerate() {
                if i % 3 == 0 {
                    *p = fill;
                }
            }
            let primary = binarize(&luma, w, h, PRIMARY_WINDOW_DIV).unwrap();
            let retry = binarize(&luma, w, h, RETRY_WINDOW_DIV).unwrap();
            assert!(
                primary.is_radius_invariant() && retry.is_radius_invariant(),
                "{label} frame must report itself settled"
            );
            assert_eq!(primary.dark, retry.dark, "{label} frame differs by radius");
        }

        // And the flag must not be set on a frame the radius does decide, or the
        // pipeline would skip a retry that matters.
        let side = 168u32;
        let mut luma = vec![255u8; (side * side) as usize];
        for y in 60..84 {
            for x in 60..84 {
                luma[(y * side + x) as usize] = 0;
            }
        }
        assert!(
            !binarize(&luma, side, side, PRIMARY_WINDOW_DIV)
                .unwrap()
                .is_radius_invariant(),
            "a frame with real dark content must not be reported settled"
        );
    }

    #[test]
    fn adaptive_handles_gradient_with_marks() {
        // 64x64 gradient background with thin dark lines (module-scale marks that
        // stay well below their local window mean, as real QR modules do).
        let w = 64u32;
        let h = 64u32;
        let mut luma = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let bg = 80 + x; // 80..144 gradient
                luma[(y * w + x) as usize] = bg as u8;
            }
        }
        // Vertical dark lines 2 px wide every 6 px.
        for y in 0..h {
            for x in 0..w {
                if x % 6 < 2 {
                    luma[(y * w + x) as usize] = 0;
                }
            }
        }
        let b = binarize(&luma, w, h, PRIMARY_WINDOW_DIV).unwrap();
        assert!(b.get(24, 30), "dark line should be dark"); // 24 % 6 == 0
        assert!(!b.get(27, 30), "gap should be light"); // 27 % 6 == 3
    }

    #[test]
    fn a_solid_block_does_not_hollow_out() {
        // The 3x3 core of a finder pattern is the largest solid feature in any
        // symbol, and in a tightly framed shot it covers a good fraction of the
        // image: an 11-module micro symbol with its quiet zone is 15 modules
        // across, so the core is a fifth of the frame.
        //
        // Where the adaptive window fits *inside* such a block the local mean is
        // the block's own colour, the pixel is not below it, and the centre of
        // the block flips to light. The finder scan's vertical cross-check walks
        // the contiguous dark band through the centre, so a hole there costs the
        // whole symbol. The window must stay wider than the block.
        let side = 168u32; // 21 modules of 8 px, as a micro symbol renders
        let block = 24u32; // three modules
        let mut luma = vec![255u8; (side * side) as usize];
        let lo = (side - block) / 2;
        for y in lo..lo + block {
            for x in lo..lo + block {
                luma[(y * side + x) as usize] = 0;
            }
        }
        let b = binarize(&luma, side, side, PRIMARY_WINDOW_DIV).unwrap();
        for y in lo..lo + block {
            for x in lo..lo + block {
                assert!(b.get(x as i64, y as i64), "({x}, {y}) hollowed out");
            }
        }
    }

    /// Renders a worn-print symbol in the corner of a camera frame under a
    /// radial vignette, and returns how many of its module centres binarize
    /// wrongly.
    ///
    /// `depth` is the vignette depth: illumination falls as
    /// `1 - depth * (r / r_max)^2` from the frame centre. Print contrast is 45%
    /// (dark modules reflect 0.55 of the light ones), which is worn thermal
    /// print rather than fresh laser.
    fn vignetted_module_errors(depth: f64, window_div: usize) -> u32 {
        let (w, h) = (1280u32, 720u32);
        let (modules, scale) = (25u32, 3u32);
        let (ox, oy) = (40u32, 40u32); // near the frame corner, where it hurts
        let cx = (w - 1) as f64 / 2.0;
        let cy = (h - 1) as f64 / 2.0;
        let r_max = (cx * cx + cy * cy).sqrt();

        // Deterministic module pattern, as dense as real data.
        let dark_module = |mx: u32, my: u32| {
            let mut v = (mx.wrapping_mul(73_856_093) ^ my.wrapping_mul(19_349_663)).wrapping_add(1);
            v ^= v >> 13;
            v = v.wrapping_mul(0x5bd1_e995);
            v ^= v >> 15;
            v & 1 == 0
        };

        let mut luma = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f64 - cx, y as f64 - cy);
                let r = (dx * dx + dy * dy).sqrt() / r_max;
                let illumination = 1.0 - depth * r * r;
                let mut reflectance = 1.0f64;
                if x >= ox
                    && y >= oy
                    && x < ox + modules * scale
                    && y < oy + modules * scale
                    && dark_module((x - ox) / scale, (y - oy) / scale)
                {
                    reflectance = 0.55;
                }
                luma[(y * w + x) as usize] = (255.0 * illumination * reflectance)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }

        let b = binarize(&luma, w, h, window_div).unwrap();
        let mut bad = 0;
        for my in 0..modules {
            for mx in 0..modules {
                let px = (ox + mx * scale + scale / 2) as i64;
                let py = (oy + my * scale + scale / 2) as i64;
                if b.get(px, py) != dark_module(mx, my) {
                    bad += 1;
                }
            }
        }
        bad
    }

    #[test]
    fn adaptive_survives_a_vignette_to_the_documented_depth() {
        // The other side of the window-radius trade-off, and the direction it
        // costs. A wider window tracks illumination less closely, so a small
        // low-contrast symbol in the corner of a large frame — where the window
        // is clamped against the border and the vignette curves inside it — is
        // where the primary `/8` radius runs out first.
        //
        // Measured on this exact fixture, module errors out of 625:
        //
        //   depth  0.50  0.70  0.85  0.90  0.95
        //   /8        0     0     0    13    32
        //   /16       0     0     0     0     0
        //
        // 0.85 is what the *primary* pass holds, and it is not negotiable that
        // the primary is `/8`: at `/16` a micro symbol's 3-module finder core
        // hollows out (see the two tests above) and, worse, so does any solid
        // feature at ordinary print contrast. Past 0.85 the primary pass is not
        // where the answer comes from — the pipeline's retry is, at the radius
        // pinned in the next test.
        assert_eq!(
            vignetted_module_errors(0.85, PRIMARY_WINDOW_DIV),
            0,
            "a 0.85-depth vignette must still binarize cleanly on the primary pass"
        );
    }

    #[test]
    fn the_retry_window_holds_a_vignette_the_primary_loses() {
        // The reason the retry pass exists, stated as a measurement rather than
        // as an intention — and pinned at the *crossover* rather than at two
        // comfortable points either side of it, so the gap between the radii is
        // what the test actually holds.
        //
        // 0.90 is the first depth in the table above where the two disagree.
        // Both assertions are about that one depth: the primary has given up on
        // it and the retry is still clean on it. The first also guards the
        // fixture — if it ever stops distinguishing the radii, this fails rather
        // than passing vacuously.
        assert!(
            vignetted_module_errors(0.90, PRIMARY_WINDOW_DIV) > 0,
            "fixture no longer distinguishes the two radii; the retry would be untested"
        );
        assert_eq!(
            vignetted_module_errors(0.90, RETRY_WINDOW_DIV),
            0,
            "the retry radius must hold at the depth the primary loses"
        );
        assert_eq!(
            vignetted_module_errors(0.95, RETRY_WINDOW_DIV),
            0,
            "and must keep holding well past it"
        );

        // This pins the binarization half only. That the *pipeline* reaches it,
        // and recovers a symbol it otherwise loses, is pinned end-to-end by
        // `detect::a_deep_vignette_is_recovered_by_the_retry_pass` and — for the
        // degraded-sample case the trigger has to catch — by
        // `micro_detect::a_degraded_micro_sample_does_not_suppress_the_retry`.
    }

    /// Deterministic zero-mean normal noise, Box-Muller over a xorshift.
    fn noisy(luma: &mut [u8], sigma: f64, seed: u64) {
        let mut s = seed | 1;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        for p in luma.iter_mut() {
            let (u1, u2) = (next(), next());
            let n = (-2.0 * u1.ln()).sqrt() * (core::f64::consts::TAU * u2).cos();
            *p = (f64::from(*p) + sigma * n).round().clamp(0.0, 255.0) as u8;
        }
    }

    /// Whether module `(mx, my)` of the synthetic texture is dark.
    ///
    /// `alternating` gives a strict one-module checkerboard, which is the worst
    /// case for any estimator that keys on high-frequency energy: it minimises
    /// the fraction of pixels in the interior of a run.
    fn texture_module(mx: u32, my: u32, alternating: bool) -> bool {
        if alternating {
            return (mx ^ my) & 1 == 0;
        }
        let mut v = mx.wrapping_mul(73_856_093) ^ my.wrapping_mul(19_349_663);
        v ^= v >> 13;
        v = v.wrapping_mul(0x5bd1_e995);
        v ^= v >> 15;
        v & 1 == 0
    }

    /// A frame tiled edge to edge with `scale`-px modules: dark modules at
    /// `bg - delta` luma against a `bg` background, the whole thing scaled by a
    /// radial illumination falloff of depth `vignette`.
    ///
    /// This is the degradation model the low-light work was measured on, minus
    /// the noise (see [`noisy`]) — contrast and illumination varied
    /// independently, because the two failures they cause are independent.
    fn textured_frame(
        w: u32,
        h: u32,
        scale: u32,
        bg: f64,
        delta: f64,
        vignette: f64,
        alternating: bool,
    ) -> Vec<u8> {
        let (cx, cy) = (f64::from(w - 1) / 2.0, f64::from(h - 1) / 2.0);
        let r_max = cx * cx + cy * cy;
        let mut luma = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
                let fall = 1.0 - vignette * (dx * dx + dy * dy) / r_max;
                let dark = texture_module(x / scale, y / scale, alternating);
                let base = if dark { bg - delta } else { bg };
                luma[(y * w + x) as usize] = (fall * base).round().clamp(0.0, 255.0) as u8;
            }
        }
        luma
    }

    /// Module-centre binarization errors against the pattern the frame was
    /// painted from, over the interior modules of a [`textured_frame`].
    fn texture_errors(bin: &Binary, w: u32, h: u32, scale: u32) -> (u32, u32) {
        let (mut bad, mut total) = (0u32, 0u32);
        for my in 1..h / scale - 1 {
            for mx in 1..w / scale - 1 {
                let (px, py) = (
                    (mx * scale + scale / 2) as i64,
                    (my * scale + scale / 2) as i64,
                );
                total += 1;
                bad += u32::from(bin.get(px, py) != texture_module(mx, my, false));
            }
        }
        (bad, total)
    }

    #[test]
    fn the_noise_estimate_separates_sensor_noise_from_texture() {
        // The whole denoise gate rests on this discrimination, and the hard half
        // is the *negative*: a dense symbol is nothing but high-frequency
        // structure, so an estimator that mistakes structure for noise smooths a
        // frame that must not be smoothed. Swept down to 3 px/module — where the
        // pinned deep-vignette fixture renders, and where smoothing costs the
        // symbol — across the contrast range, and including the strict
        // alternating checkerboard, which minimises the fraction of quiet pixels
        // the quantile relies on.
        let (w, h) = (320u32, 240u32);
        for scale in [3u32, 4, 6, 8] {
            for alternating in [false, true] {
                for delta in [156.0f64, 60.0, 25.0] {
                    let clean = textured_frame(w, h, scale, 240.0, delta, 0.6, alternating);
                    assert!(
                        !noise_reaches(&clean, w, h, DENOISE_TRIGGER),
                        "noise-free frame read as noisy: {scale}px modules, \
                         delta {delta}, alternating {alternating}"
                    );
                }
            }
        }

        // And the positive: the floor tracks sigma at about 0.79 * sigma, so it
        // must clear a threshold well below that and fail one well above.
        // Bracketing rather than asserting a point value is what makes this a
        // calibration test and not a golden value.
        for (sigma, lo, hi) in [(4.0, 3u32, 7u32), (8.0, 6, 12), (16.0, 12, 20)] {
            let mut frame = textured_frame(w, h, 6, 240.0, 156.0, 0.6, false);
            noisy(&mut frame, sigma, 0x51ee_d000 + sigma as u64);
            assert!(
                noise_reaches(&frame, w, h, lo) && !noise_reaches(&frame, w, h, hi),
                "sigma {sigma}: noise floor outside [{lo}, {hi})"
            );
            assert!(
                noise_reaches(&frame, w, h, DENOISE_TRIGGER),
                "sigma {sigma} must trip the gate"
            );
        }
    }

    #[test]
    fn smoothing_recovers_module_centres_that_noise_flips() {
        // The measurement the retry rung's denoise exists for, at the settings
        // it runs at. Noise is not a small perturbation of the dark test: the
        // window mean is taken over thousands of pixels and is already
        // noise-free, so the whole of a pixel's noise lands on one side of the
        // comparison. Averaging nine samples divides it by three.
        //
        // Asserted as a ratio rather than an absolute count, so the test states
        // the claim — much better — rather than pinning a number that any
        // harmless change to the fixture would move. The first assertion guards
        // the fixture: if noise stops damaging it, the second proves nothing.
        let (w, h, scale) = (320u32, 240u32, 6u32);
        let mut frame = textured_frame(w, h, scale, 220.0, 60.0, 0.0, false);
        noisy(&mut frame, 16.0, 0xd00d);
        assert!(
            noise_reaches(&frame, w, h, DENOISE_TRIGGER),
            "fixture must trip the gate"
        );

        let raw = binarize_with(&frame, w, h, RETRY_WINDOW_DIV, NARROW_MARGIN).expect("raw");
        let smoothed = binarize_retry(&frame, w, h).expect("retry rung");
        let (bad_raw, total) = texture_errors(&raw, w, h, scale);
        let (bad_smooth, _) = texture_errors(&smoothed, w, h, scale);

        assert!(
            bad_raw * 20 > total,
            "fixture no longer damaged by noise ({bad_raw} of {total}); the test proves nothing"
        );
        assert!(
            bad_smooth * 4 < bad_raw,
            "smoothing must cut module errors by more than 4x: {bad_raw} -> {bad_smooth}"
        );
    }

    #[test]
    fn the_narrow_margin_sees_a_print_the_primary_margin_cannot() {
        // The mechanism behind the whole low-light rework, as an arithmetic
        // statement about one frame rather than as a story. The Bradley test is
        // relative, so a symbol whose modules sit a fixed number of luma below
        // the paper has to clear a threshold that scales with the paper. Against
        // a 240 background that threshold is about 51 luma, so at every contrast
        // swept here the primary margin does not merely degrade — it returns a
        // blank, and error correction downstream has nothing to work on.
        //
        // The vignette is what makes the case sharp rather than academic. On a
        // *flat* frame this same contrast trips `can_be_dark`, the adaptive pass
        // returns nothing at all, and the Otsu fallback rescues it — which is
        // the subject of `a_thin_contrast_symbol_on_a_flat_frame_goes_through_otsu`.
        // Add any illumination gradient and both of those escapes close: the
        // frame's dynamic range is wide enough to run the adaptive pass, the
        // adaptive pass marks just enough stray pixels to stay off the fallback,
        // and a global threshold could not have resolved the frame anyway. That
        // combination — thin print under uneven light — is the one real cameras
        // hand us, and the margin is the only thing that answers it.
        let (w, h, scale) = (320u32, 240u32, 6u32);
        // 35 is the thin end of what the narrow margin holds under this
        // gradient, not of what it holds at all: on a flat frame it resolves 25
        // outright. Below that, under uneven light, the contrast is past what
        // any threshold recovers and the envelope table says so.
        for delta in [45.0f64, 40.0, 35.0] {
            let luma = textured_frame(w, h, scale, 240.0, delta, 0.5, false);
            let wide = binarize(&luma, w, h, RETRY_WINDOW_DIV).expect("wide margin");
            let narrow =
                binarize_with(&luma, w, h, RETRY_WINDOW_DIV, NARROW_MARGIN).expect("narrow");
            let (bad_wide, total) = texture_errors(&wide, w, h, scale);
            let (bad_narrow, _) = texture_errors(&narrow, w, h, scale);

            // The wide margin loses essentially every dark module — about half
            // the frame — while the narrow one lands inside any error-correction
            // budget. Stated as proportions of the frame rather than as counts,
            // because the claim is about which side of "recoverable" each margin
            // falls on, not about a golden number.
            assert!(
                bad_wide * 3 > total,
                "delta {delta}: the primary margin should lose most of the symbol, \
                 lost {bad_wide} of {total}"
            );
            assert!(
                bad_narrow * 50 < total,
                "delta {delta}: the narrow margin should land under 2% module error, \
                 got {bad_narrow} of {total}"
            );
        }
    }

    #[test]
    fn a_thin_contrast_symbol_on_a_flat_frame_goes_through_otsu() {
        // Pins the one place the retry's margin change is knowingly not
        // consulted. A frame under 12% of global dynamic range fails
        // `can_be_dark` at the primary margin, returns the Otsu fallback, and is
        // marked settled — so the retry, whose margin *would* have found
        // something, never runs. That is sound because such a frame is globally
        // flat by construction, and a global threshold is the right answer on
        // it: this asserts the fallback actually resolves the modules, which is
        // what makes skipping the retry free rather than lossy.
        let (w, h, scale) = (320u32, 240u32, 6u32);
        let luma = textured_frame(w, h, scale, 220.0, 20.0, 0.0, false);
        let bin = binarize(&luma, w, h, PRIMARY_WINDOW_DIV).expect("binarizes");
        assert!(
            bin.is_radius_invariant(),
            "a frame this flat must short-circuit to the global threshold"
        );
        assert_eq!(
            texture_errors(&bin, w, h, scale).0,
            0,
            "the Otsu fallback must resolve every module centre"
        );
    }

    #[test]
    fn a_solid_block_at_worn_print_contrast_does_not_hollow_out() {
        // The axis the fixture set was missing, and the one that killed the
        // two-scale discriminator: contrast and illumination are independent,
        // and every other hollowing fixture here paints luma 0 on luma 255.
        //
        // Real worn thermal print is nothing like that. At 45% contrast a dark
        // module is luma 140 on a 255 background — the exact value
        // `vignetted_module_errors` already models on the illumination axis —
        // and any rule that keys off *how dark* a feature is rather than *how
        // big* it is silently stops firing here while every full-contrast test
        // in the file stays green.
        //
        // Swept across the range a print can plausibly land in, so the pin does
        // not sit on one lucky value.
        for dark in [0u8, 51, 76, 102, 115, 140, 153] {
            let side = 168u32; // 21 modules of 8 px, as a micro symbol renders
            let block = 24u32; // three modules
            let mut luma = vec![255u8; (side * side) as usize];
            let lo = (side - block) / 2;
            for y in lo..lo + block {
                for x in lo..lo + block {
                    luma[(y * side + x) as usize] = dark;
                }
            }
            let b = binarize(&luma, side, side, PRIMARY_WINDOW_DIV).unwrap();
            for y in lo..lo + block {
                for x in lo..lo + block {
                    assert!(
                        b.get(x as i64, y as i64),
                        "({x}, {y}) hollowed out at dark level {dark}"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod fits_u32_tests {
    use super::fits_u32;

    /// Pins the accumulator-width routing at the boundaries the doc comment
    /// argues from: 4K-class frames stay in u32, 8K-class frames do not.
    #[test]
    fn accumulator_routing_boundaries() {
        // 3840x2160 with the /8 window fits u32.
        assert!(fits_u32(3840, 2160, 2160 / 8));
        // 7680x4320 blows the whole-frame prefix bound.
        assert!(!fits_u32(7680, 4320, 4320 / 8));
        // The frame-sum bound itself: w*h == floor(u32::MAX/255) passes, one
        // row more fails (radius kept small so the count bound stays inert).
        let limit = u32::MAX as usize / 255;
        let w = 4096;
        assert!(fits_u32(w, limit / w, 4));
        assert!(!fits_u32(w, limit / w + 1, 4));
    }
}
