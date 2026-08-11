//! `qr-detect` — locate and sample QR symbols from a grayscale image.
//!
//! The pipeline is fully hand-rolled (the `image` crate is used only for its
//! buffer types):
//!
//! 1. [`binarize`] — adaptive-window threshold with an Otsu fallback.
//! 2. [`finder`] — `1:1:3:1:1` finder-pattern scan with vertical cross-check
//!    and centre clustering.
//! 3. [`grouping`] — triple selection (right angle + size similarity),
//!    orientation and dimension estimation. Multiple codes per image supported.
//! 4. [`sampler`] — alignment-pattern refinement (versions >= 2), grid ->
//!    image homography, and module sampling into a [`BitMatrix`].
//!
//! # The retry pass
//!
//! Binarization has two settings that no single value holds every image at, and
//! they conflict in the same shape:
//!
//! * **Window radius.** A wide window keeps solid features solid — a finder
//!   pattern's 3-module core hollows out under a narrow one, at any print
//!   contrast — while a narrow window is what tracks a steep illumination
//!   gradient, such as a small symbol in the corner of a vignetted camera frame.
//! * **Bradley margin.** A wide margin is what keeps sensor noise from speckling
//!   a well-lit frame, while a narrow one is what sees a symbol whose print
//!   contrast is thin — a dimly lit or worn print, where the dark modules sit
//!   only a few luma below the paper. Because the test is relative, the wide
//!   margin does not merely degrade on such a frame: it binarizes the symbol
//!   away entirely, and nothing downstream recovers modules that were never
//!   distinguished.
//! * **Smoothing.** Averaging a 3x3 box over the frame divides sensor noise by
//!   three, and costs nothing while that box sits inside a module — but a symbol
//!   rendered at 3 px/module has no such margin, and smoothing loses it. So this
//!   one is gated on a noise estimate rather than merely deferred, which is what
//!   keeps every clean frame on the path it had before.
//!
//! So the pipeline runs twice, in order. The primary pass binarizes at
//! `binarize::PRIMARY_WINDOW_DIV` and `binarize::PRIMARY_MARGIN`, and is what
//! nearly every image is decoded by. Only if it yields nothing usable does the
//! whole frame go through `binarize::binarize_retry` — all three settings moved
//! at once — and get rescanned. Success on the first pass costs nothing, and a
//! miss still costs exactly two passes: the rungs stack settings rather than
//! multiplying passes.
//!
//! ## What counts as "usable", and why it is not "found something"
//!
//! With the `decode` feature on, the trigger is **no successful decode**, not
//! **no detection**. The difference is not academic. A detection is produced for
//! every candidate that *sampled*, decoded or not, and on the micro path a
//! candidate can sample from a binarization that has already failed: a micro
//! symbol is anchored on a single finder, and near a vignetted frame corner that
//! finder survives while the timing pattern and data area rot. The pipeline then
//! samples a wrong grid — measured at 23 wrong modules out of an M4's 289, far
//! outside any error-correction budget — and reports a detection, which under a
//! count-based trigger would suppress the retry that resolves the same frame
//! exactly. QR's three-finder geometry degrades to "no candidate" instead, so
//! the distinction only bites on micro; the trigger is uniform anyway, because
//! the failure is a property of the evidence, not of the symbology.
//!
//! Without `decode` there is no decode outcome to key on, so the trigger falls
//! back to a candidate count and those builds keep the weaker guarantee. That is
//! a real limitation, not an oversight: such builds decode downstream and are
//! the only ones that can judge a sample. They should use
//! [`detect_luma_single_pass`] / [`detect_micro_luma_single_pass`] and drive the
//! second radius themselves when their own decode fails.
//!
//! When both passes have run, the one returned is whichever ranks higher on
//! `(usable, found)`, with ties going to the primary. Ranking on the second
//! element is what stops the retry from ever losing ground: if neither pass
//! decoded anything, a pass that at least located a symbol still beats one that
//! located none.
//!
//! ## When the retry is skipped outright
//!
//! Some frames settle the question at the first pass: the binarization never
//! consulted the radius it was handed, so the second radius would rebuild the
//! same image and rescan it to the same answer. That happens when the frame is
//! too small for an adaptive window to mean anything, and — the case that costs
//! real time — when its dynamic range is too narrow for any pixel to fall below
//! its local mean, which is every blank or near-blank camera frame. Those return
//! after one pass. See `binarize::Binary::is_radius_invariant`.
//!
//! On the dynamic-range arm the second rung's *margin* would not have been a
//! no-op, so that skip is a judgement rather than a proof. It is the right one:
//! a frame that narrow is globally flat by construction, and the global Otsu
//! threshold such a frame falls back to is the better answer on it than any
//! adaptive window. `binarize::can_be_dark` carries the argument.
//!
//! ## What the retry does not cover
//!
//! The trigger is "nothing usable", not "nothing new". An image holding two
//! symbols where one decodes at the primary radius and the other needs the retry
//! returns only the first: the primary pass is a success, so no retry happens.
//! Making it complete would mean running both passes on every frame and merging,
//! which doubles the cost of every successful scan to serve a case (multiple
//! codes, one of them deeply vignetted) that no fixture in this workspace
//! exercises. If that case turns up, this is the paragraph to revisit.
//!
//! Callers that have already narrowed the image themselves — cropping to a
//! tracked region of interest, say — should use [`detect_luma_single_pass`],
//! which stops after the primary. Inside a tight crop the retry is both more
//! expensive per pixel of value and less likely to help, because the symbol
//! fills the frame and its modules are large relative to the window.
//!
//! # Coordinates
//!
//! All coordinates are `(x, y)` = (column, row) with the origin top-left and
//! `y` increasing downward, matching image row order.
//!
//! # Micro QR
//!
//! With the `micro` cargo feature (enabled by default), single-finder Micro QR
//! symbols (ISO/IEC 18004 M1-M4) are detected too, through a parallel API:
//! [`MicroDetection`], [`detect_micro`] and [`detect_all`]. Micro symbols are
//! anchored on finder clusters that no QR triple claimed; the existing
//! [`Detection`] type and functions are unaffected.
//!
//! # Decoding
//!
//! With the `decode` cargo feature (enabled by default), each detection is
//! also run through `qr_decode::decode`. Disable with `--no-default-features`
//! for a detection-only build. With `micro` and `decode` both on, micro symbols
//! go through `qr_decode::micro::decode_micro` the same way.

mod binarize;
mod finder;
mod geometry;
mod grouping;
#[cfg(feature = "micro")]
mod micro;
mod sampler;

pub use geometry::{PerspectiveTransform, Point};

use binarize::Binary;
use finder::Finder;
use grouping::QrCandidate;
use image::GrayImage;
use qr_core::BitMatrix;

/// One detected QR symbol.
pub struct Detection {
    /// Symbol corners in image space, ordered top-left, top-right,
    /// bottom-right, bottom-left. `(x, y)` = (column, row).
    pub corners: [Point; 4],
    /// The sampled module grid (raw modules; not yet unmasked).
    pub matrix: BitMatrix,
    /// The decoded payload, if the `decode` feature is enabled and decoding
    /// succeeded.
    #[cfg(feature = "decode")]
    pub decoded: Option<qr_decode::Decoded>,
}

/// Detects every QR symbol in a grayscale image.
///
/// Returns one [`Detection`] per located symbol (possibly empty).
pub fn detect(img: &GrayImage) -> Vec<Detection> {
    detect_luma(img.as_raw(), img.width(), img.height())
}

/// Detects every QR symbol in a raw luma buffer.
///
/// `luma` must be exactly `width * height` bytes, row-major. Returns an empty
/// vector on dimension mismatch or when nothing is found.
///
/// Runs the retry pass when the primary yields nothing usable — see the crate
/// docs for what "usable" means and how the two passes are compared.
pub fn detect_luma(luma: &[u8], width: u32, height: u32) -> Vec<Detection> {
    let (primary, settled) = detect_qr_at(luma, width, height, Pass::Primary);
    if settled || usable_qr(&primary) > 0 {
        return primary;
    }
    let (retry, _) = detect_qr_at(luma, width, height, Pass::Retry);
    better_pass(primary, retry, usable_qr)
}

/// How many of a pass's QR detections are actually *usable*, which is what the
/// retry trigger keys on.
///
/// With `decode` on, a detection counts only if it decoded. This matters: a
/// candidate that sampled but did not decode is not evidence the binarization
/// worked, and treating it as evidence is what suppressed the retry on frames it
/// resolves perfectly.
#[cfg(feature = "decode")]
fn usable_qr(dets: &[Detection]) -> usize {
    dets.iter().filter(|d| d.decoded.is_some()).count()
}

/// Without `decode` there is no decode outcome to key on, so a sampled
/// candidate is the only evidence available and the trigger stays "found
/// nothing". Such builds decode externally and cannot be second-guessed from in
/// here; when they need the other radius they should call
/// [`detect_luma_single_pass`] and drive the retry themselves.
#[cfg(not(feature = "decode"))]
fn usable_qr(dets: &[Detection]) -> usize {
    dets.len()
}

/// Chooses between the two passes once both have run.
///
/// Ranked on `(usable, found)` and biased to the primary on a tie, which makes
/// the choice total and deterministic. The second element is what stops the
/// retry from *losing* a result: if neither pass decoded anything, the pass that
/// at least located a symbol still wins over one that located none.
fn better_pass<T>(primary: Vec<T>, retry: Vec<T>, usable: impl Fn(&[T]) -> usize) -> Vec<T> {
    if (usable(&retry), retry.len()) > (usable(&primary), primary.len()) {
        retry
    } else {
        primary
    }
}

/// [`detect_luma`] without the retry pass, for callers that have already cropped
/// the image to a region of interest.
///
/// Inside a tight crop the symbol fills the frame, so its modules are large
/// relative to the adaptive window and the retry radius is the wrong one for
/// them; running it would cost a full second pass on every miss to serve a case
/// that a crop has already ruled out.
///
/// This is also how the tests tell the two passes apart: a fixture that
/// [`detect_luma`] recovers and this function does not is, by construction, one
/// the retry pass found.
pub fn detect_luma_single_pass(luma: &[u8], width: u32, height: u32) -> Vec<Detection> {
    detect_qr_at(luma, width, height, Pass::Primary).0
}

/// One rung of the binarization ladder.
///
/// The two rungs differ in *both* of binarize's settings — window radius and
/// Bradley margin — so naming them here, rather than threading two loose
/// numbers through the pipeline, is what keeps "which pass is this" a single
/// decision with a single answer. [`binarize_for`] is the only place the
/// settings themselves appear.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pass {
    Primary,
    Retry,
}

/// Binarizes `luma` at `pass`'s settings.
fn binarize_for(luma: &[u8], width: u32, height: u32, pass: Pass) -> Option<Binary> {
    match pass {
        Pass::Primary => binarize::binarize(luma, width, height, binarize::PRIMARY_WINDOW_DIV),
        Pass::Retry => binarize::binarize_retry(luma, width, height),
    }
}

/// A pass's detections, plus whether the frame *settled* it — see
/// [`settled_by_binarization`].
type Outcome<T> = (Vec<T>, bool);

/// Whether a second pass at another radius is provably pointless.
///
/// True when the binarization did not depend on the radius it was given
/// (`Binary::is_radius_invariant`), so the retry would rebuild the same image,
/// rescan it and reach the same answer. A blank camera frame is the case that
/// matters: it used to pay two full binarizations per pass and two passes.
///
/// The `None` arm folds in the other dead retry — a luma buffer whose length
/// does not match its stated dimensions is rejected identically at both radii.
fn settled_by_binarization(bin: Option<&Binary>) -> bool {
    bin.is_none_or(Binary::is_radius_invariant)
}

fn detect_qr_at(luma: &[u8], width: u32, height: u32, pass: Pass) -> Outcome<Detection> {
    let scanned = scan(luma, width, height, pass);
    let settled = settled_by_binarization(scanned.as_ref().map(|(bin, ..)| bin));
    let dets = scanned
        .map(|(bin, _, candidates, _)| collect_qr(&bin, &candidates))
        .unwrap_or_default();
    (dets, settled)
}

/// What [`scan`] produces: the binarized image, every located finder, the QR
/// candidates they grouped into, and a flag per finder saying whether a triple
/// claimed it.
type Scan = (Binary, Vec<Finder>, Vec<QrCandidate>, Vec<bool>);

/// Shared front of the pipeline: binarize at the given window radius, locate
/// finder patterns, group them into QR candidates (see [`grouping::group`]).
fn scan(luma: &[u8], width: u32, height: u32, pass: Pass) -> Option<Scan> {
    let bin = binarize_for(luma, width, height, pass)?;
    let finders = finder::find_finders(&bin);
    let (candidates, used) = grouping::group(&finders);
    Some((bin, finders, candidates, used))
}

fn collect_qr(bin: &Binary, candidates: &[QrCandidate]) -> Vec<Detection> {
    let mut out = Vec::new();
    for cand in candidates {
        if let Some(sampled) = sampler::sample(bin, cand) {
            out.push(build_detection(sampled.corners, sampled.matrix));
        }
    }
    out
}

/// Detects and returns the single best (first) symbol, or `None`.
pub fn read_one(img: &GrayImage) -> Option<Detection> {
    read_one_luma(img.as_raw(), img.width(), img.height())
}

/// [`read_one`] over a raw luma buffer.
pub fn read_one_luma(luma: &[u8], width: u32, height: u32) -> Option<Detection> {
    let mut all = detect_luma(luma, width, height);
    if all.is_empty() {
        return None;
    }
    // Prefer a detection that actually decoded, when decoding is enabled.
    #[cfg(feature = "decode")]
    {
        if let Some(pos) = all.iter().position(|d| d.decoded.is_some()) {
            return Some(all.swap_remove(pos));
        }
    }
    Some(all.swap_remove(0))
}

#[cfg(feature = "decode")]
fn build_detection(corners: [Point; 4], matrix: BitMatrix) -> Detection {
    let decoded = qr_decode::decode(&matrix).ok();
    Detection {
        corners,
        matrix,
        decoded,
    }
}

#[cfg(not(feature = "decode"))]
fn build_detection(corners: [Point; 4], matrix: BitMatrix) -> Detection {
    Detection { corners, matrix }
}

// ---------------------------------------------------------------------------
// Micro QR
// ---------------------------------------------------------------------------

/// One detected Micro QR symbol (ISO/IEC 18004 M1-M4).
///
/// The micro counterpart of [`Detection`]; the two are deliberately separate
/// types, because the two symbologies share no version, capacity or format
/// information.
#[cfg(feature = "micro")]
pub struct MicroDetection {
    /// Symbol corners in image space, ordered top-left, top-right,
    /// bottom-right, bottom-left, where "top-left" is the finder's corner.
    /// `(x, y)` = (column, row).
    pub corners: [Point; 4],
    /// The sampled module grid (raw modules; not yet unmasked). Its size is one
    /// of 11, 13, 15, 17.
    pub matrix: BitMatrix,
    /// The decoded payload, if the `decode` feature is enabled and decoding
    /// succeeded.
    #[cfg(feature = "decode")]
    pub decoded: Option<qr_decode::micro::DecodedMicro>,
}

/// Detects every Micro QR symbol in a grayscale image.
///
/// Micro symbols are anchored on finder patterns that no QR triple claimed, so
/// a QR symbol in the same image never yields phantom micro detections.
#[cfg(feature = "micro")]
pub fn detect_micro(img: &GrayImage) -> Vec<MicroDetection> {
    detect_micro_luma(img.as_raw(), img.width(), img.height())
}

/// [`detect_micro`] over a raw luma buffer (`width * height` bytes, row-major).
///
/// Runs the retry pass when the primary yields no usable micro symbol — see the
/// crate docs.
///
/// This path is the reason the trigger is decode-aware rather than a candidate
/// count. A micro symbol is anchored on a *single* finder, so near a vignetted
/// frame corner that finder can survive a binarization the timing pattern and
/// data area do not: the pipeline samples a wrong grid and reports a detection.
/// QR's three-finder geometry degrades to "no candidate" instead, which is why
/// the distinction does not arise there.
#[cfg(feature = "micro")]
pub fn detect_micro_luma(luma: &[u8], width: u32, height: u32) -> Vec<MicroDetection> {
    let (primary, settled) = detect_micro_at(luma, width, height, Pass::Primary);
    if settled || usable_micro(&primary) > 0 {
        return primary;
    }
    let (retry, _) = detect_micro_at(luma, width, height, Pass::Retry);
    better_pass(primary, retry, usable_micro)
}

/// The micro counterpart of [`usable_qr`], with the same reasoning.
#[cfg(all(feature = "micro", feature = "decode"))]
fn usable_micro(dets: &[MicroDetection]) -> usize {
    dets.iter().filter(|d| d.decoded.is_some()).count()
}

#[cfg(all(feature = "micro", not(feature = "decode")))]
fn usable_micro(dets: &[MicroDetection]) -> usize {
    dets.len()
}

/// [`detect_micro_luma`] without the retry pass — the micro counterpart of
/// [`detect_luma_single_pass`], and the same two uses: callers that have already
/// cropped to a region of interest, and tests that need to tell the two passes
/// apart.
#[cfg(feature = "micro")]
pub fn detect_micro_luma_single_pass(luma: &[u8], width: u32, height: u32) -> Vec<MicroDetection> {
    detect_micro_at(luma, width, height, Pass::Primary).0
}

#[cfg(feature = "micro")]
fn detect_micro_at(luma: &[u8], width: u32, height: u32, pass: Pass) -> Outcome<MicroDetection> {
    let scanned = scan(luma, width, height, pass);
    let settled = settled_by_binarization(scanned.as_ref().map(|(bin, ..)| bin));
    let dets = scanned
        .map(|(bin, finders, _, used)| collect_micro(&bin, &finders, &used))
        .unwrap_or_default();
    (dets, settled)
}

/// Detects QR and Micro QR symbols in one pass, returning `(qr, micro)`.
///
/// Equivalent to calling [`detect`] and [`detect_micro`], but binarizes and
/// scans for finder patterns only once.
#[cfg(feature = "micro")]
pub fn detect_all(img: &GrayImage) -> (Vec<Detection>, Vec<MicroDetection>) {
    detect_all_luma(img.as_raw(), img.width(), img.height())
}

/// [`detect_all`] over a raw luma buffer (`width * height` bytes, row-major).
///
/// Runs the retry pass only when the primary yields nothing usable from *either*
/// symbology — see the crate docs. A frame that yields one kind and not the
/// other is a hit, not a miss, and is returned as it stands.
///
/// The trigger sums across both symbologies deliberately. Counting candidates
/// instead would let one degraded micro detection — a wrong grid sampled off a
/// surviving finder — suppress the retry for the QR side as well, losing two
/// payloads to one bad sample.
#[cfg(feature = "micro")]
pub fn detect_all_luma(
    luma: &[u8],
    width: u32,
    height: u32,
) -> (Vec<Detection>, Vec<MicroDetection>) {
    let ((qr, micro), settled) = detect_all_at(luma, width, height, Pass::Primary);
    if settled || usable_qr(&qr) + usable_micro(&micro) > 0 {
        return (qr, micro);
    }
    let ((retry_qr, retry_micro), _) = detect_all_at(luma, width, height, Pass::Retry);

    // Same ranking as the single-symbology paths, over the combined totals.
    let primary_score = (
        usable_qr(&qr) + usable_micro(&micro),
        qr.len() + micro.len(),
    );
    let retry_score = (
        usable_qr(&retry_qr) + usable_micro(&retry_micro),
        retry_qr.len() + retry_micro.len(),
    );
    if retry_score > primary_score {
        (retry_qr, retry_micro)
    } else {
        (qr, micro)
    }
}

#[cfg(feature = "micro")]
fn detect_all_at(
    luma: &[u8],
    width: u32,
    height: u32,
    pass: Pass,
) -> ((Vec<Detection>, Vec<MicroDetection>), bool) {
    let scanned = scan(luma, width, height, pass);
    let settled = settled_by_binarization(scanned.as_ref().map(|(bin, ..)| bin));
    let dets = scanned
        .map(|(bin, finders, candidates, used)| {
            (
                collect_qr(&bin, &candidates),
                collect_micro(&bin, &finders, &used),
            )
        })
        .unwrap_or_default();
    (dets, settled)
}

#[cfg(feature = "micro")]
fn collect_micro(bin: &Binary, finders: &[Finder], used: &[bool]) -> Vec<MicroDetection> {
    let mut out = Vec::new();
    for (i, f) in finders.iter().enumerate() {
        if used[i] {
            continue;
        }
        if let Some(sampled) = micro::detect_from_finder(bin, f) {
            out.push(build_micro_detection(sampled.corners, sampled.matrix));
        }
    }
    out
}

#[cfg(all(feature = "micro", feature = "decode"))]
fn build_micro_detection(corners: [Point; 4], matrix: BitMatrix) -> MicroDetection {
    let decoded = qr_decode::micro::decode_micro(&matrix).ok();
    MicroDetection {
        corners,
        matrix,
        decoded,
    }
}

#[cfg(all(feature = "micro", not(feature = "decode")))]
fn build_micro_detection(corners: [Point; 4], matrix: BitMatrix) -> MicroDetection {
    MicroDetection { corners, matrix }
}
