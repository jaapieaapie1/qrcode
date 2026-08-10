//! `qr-video` — stateful QR detection across a stream of video frames.
//!
//! This crate adds three things on top of the single-image [`qr_detect`]
//! pipeline, without pulling in any capture backend:
//!
//! 1. A capture-agnostic frame abstraction ([`Frame`] + [`FrameSource`]).
//! 2. Region-of-interest (ROI) tracking: after a hit, the next frames are
//!    scanned only inside a padded box around the last detection, with a
//!    full-frame fallback on miss and a periodic full-frame refresh.
//! 3. Temporal de-duplication: the same payload seen on consecutive frames is
//!    reported once ([`EventKind::FirstSeen`]); disappearance is reported as
//!    [`EventKind::Lost`].
//!
//! # No capture dependency
//!
//! The caller owns frame acquisition. A [`Frame`] is just a borrowed luma
//! buffer plus dimensions and a caller-supplied timestamp (milliseconds). No
//! `std::time` type appears in the API, so a camera driver, a video demuxer, or
//! a test harness can all feed frames the same way.
//!
//! # Coordinates
//!
//! All coordinates are `(x, y)` = (column, row), origin top-left, `y` growing
//! downward — matching [`qr_detect`] and the underlying image row order.

mod detector;

pub use detector::{Config, Observation, Stats, VideoDetector};
pub use qr_detect::Point;

/// A single video frame: a borrowed grayscale (luma) buffer plus geometry and a
/// timestamp.
///
/// `luma` must be exactly `width * height` bytes, row-major. The lifetime `'a`
/// ties the frame to whatever owns the pixels (a capture buffer, a decoder's
/// scratch, a test fixture) so no copy is required to hand a frame to the
/// detector.
///
/// `timestamp_millis` is caller-supplied and only needs to be monotonically
/// non-decreasing across a stream; its absolute epoch is irrelevant. It drives
/// the de-duplication debounce and the "lost" timeout.
pub struct Frame<'a> {
    /// Row-major luma samples, `width * height` bytes.
    pub luma: &'a [u8],
    /// Frame width in pixels (number of columns).
    pub width: u32,
    /// Frame height in pixels (number of rows).
    pub height: u32,
    /// Caller-supplied capture time in milliseconds (monotonic, arbitrary
    /// epoch).
    pub timestamp_millis: u64,
}

impl<'a> Frame<'a> {
    /// Creates a frame from its parts. Does not validate that
    /// `luma.len() == width * height`; the detector treats a size mismatch as
    /// an empty frame rather than panicking.
    pub fn new(luma: &'a [u8], width: u32, height: u32, timestamp_millis: u64) -> Frame<'a> {
        Frame {
            luma,
            width,
            height,
            timestamp_millis,
        }
    }

    /// Returns `true` when `luma` holds exactly `width * height` samples.
    pub fn is_well_formed(&self) -> bool {
        self.luma.len() as u64 == self.width as u64 * self.height as u64
    }
}

/// A source of video frames.
///
/// # Borrow design (why this shape)
///
/// `next_frame` returns a `Frame<'_>` whose lifetime is tied to `&mut self`.
/// This is the *lending iterator* pattern: the frame borrows storage owned by
/// the source (typically a single reusable capture buffer), so pulling the next
/// frame invalidates the previous one. That is exactly the ownership model of a
/// real camera or video-decode loop — one live frame at a time, reused buffer,
/// zero per-frame allocation.
///
/// We deliberately do **not** implement [`Iterator`]: `Iterator::next` cannot
/// borrow from `self` in its item type, which would force every source to
/// allocate and hand out an owned buffer per frame. A generic-associated-type
/// (GAT) `LendingIterator` trait would also express this, but a plain inherent
/// method with an elided `'_` lifetime is the simplest sound encoding — no GATs,
/// no `unsafe` — and reads naturally. The cost is that you process each frame
/// before requesting the next, which is precisely what streaming detection
/// wants.
///
/// Drive a source with [`VideoDetector::run`].
pub trait FrameSource {
    /// Returns the next frame, or `None` when the stream is exhausted.
    ///
    /// The returned frame borrows `self`; it must be dropped (or fully
    /// processed) before the next call.
    fn next_frame(&mut self) -> Option<Frame<'_>>;
}

/// What a returned [`Observation`] represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// A payload not currently being tracked (new, or re-appearing after it was
    /// lost or after the debounce window elapsed).
    FirstSeen,
    /// The same payload seen again within the debounce window. Suppressed from
    /// output unless [`Config::emit_repeated`] is set.
    Repeated,
    /// A previously tracked payload that has not been seen for at least
    /// [`Config::lost_after_millis`]. Carries no corners.
    Lost,
}
