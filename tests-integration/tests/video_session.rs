//! Suite 4 — a synthetic video session driven through `qr-video`.
//!
//! Frames are composited from a real `qr-encode` + `qr-render` symbol moving
//! across a white canvas, so the whole stack (render -> detect -> decode ->
//! temporal de-duplication) runs per frame.
//!
//! Alongside the enter/leave/re-enter event sequence this closes the two
//! video-review gaps recorded as **KG-5** in `handoff/BUILD-LOG.md`:
//!
//! * a code sitting on the frame border, where ROI padding must clamp;
//! * the periodic full-scan refresh, driven from `Config::full_scan_interval`
//!   (including the `0` = never-refresh setting) and the ROI-miss fallback.

mod common;

use common::{Gray, compose, render_gray};
use qr_core::EcLevel;
use qr_encode::{Options, encode};
use qr_video::{Config, EventKind, Frame, FrameSource, VideoDetector};

const PAYLOAD: &[u8] = b"VIDEO-SESSION-2026";

/// Milliseconds between synthetic frames.
const FRAME_MS: u64 = 100;

/// Renders the session's QR symbol once; every frame composites this bitmap.
fn code() -> Gray {
    let m = encode(PAYLOAD, Options::new(EcLevel::M)).expect("encode session payload");
    render_gray(&m, 6, 4)
}

/// A pre-rendered clip. Frames are owned by the clip and lent out one at a
/// time, matching the `FrameSource` borrow contract.
struct Clip {
    frames: Vec<Gray>,
    next: usize,
}

impl Clip {
    fn new(frames: Vec<Gray>) -> Clip {
        Clip { frames, next: 0 }
    }
}

impl FrameSource for Clip {
    fn next_frame(&mut self) -> Option<Frame<'_>> {
        if self.next >= self.frames.len() {
            return None;
        }
        let idx = self.next;
        self.next += 1;
        let ts = idx as u64 * FRAME_MS;
        let g = &self.frames[idx];
        Some(Frame::new(&g.px, g.w, g.h, ts))
    }
}

/// Builds a canvas holding the code at `pos`, or a blank canvas for `None`.
fn frame_at(w: u32, h: u32, code: &Gray, pos: Option<(i64, i64)>) -> Gray {
    match pos {
        Some((x, y)) => compose(w, h, code, x, y),
        None => Gray::new(w, h, 255),
    }
}

// ---------------------------------------------------------------------------
// The session
// ---------------------------------------------------------------------------

/// Twenty frames: the code enters and drifts right, leaves the scene, then
/// re-enters further along. The de-duplicated event stream must be exactly
/// `FirstSeen`, `Lost`, `FirstSeen`, and ROI tracking must have engaged while
/// the code was visible.
#[test]
fn session_enter_move_leave_reenter() {
    let code = code();
    let (w, h) = (640u32, 300u32);

    // Frames 0..=5 visible (drifting right), 6..=12 empty, 13..=19 visible again.
    let mut frames = Vec::new();
    for i in 0..20u32 {
        let pos = match i {
            0..=5 => Some((20 + i as i64 * 30, 60)),
            6..=12 => None,
            _ => Some((340 + (i as i64 - 13) * 20, 60)),
        };
        frames.push(frame_at(w, h, &code, pos));
    }

    let mut det = VideoDetector::with_defaults();
    let mut clip = Clip::new(frames);
    let mut events = Vec::new();
    det.run(&mut clip, |obs| events.push(obs));

    let kinds: Vec<EventKind> = events.iter().map(|o| o.kind).collect();
    assert_eq!(
        kinds,
        vec![EventKind::FirstSeen, EventKind::Lost, EventKind::FirstSeen],
        "unexpected event sequence: {kinds:?}"
    );

    // Every event refers to the same tracked payload.
    assert_eq!(events[0].bytes.as_deref(), Some(PAYLOAD), "first sighting payload");
    assert_eq!(
        events[2].bytes.as_deref(),
        Some(PAYLOAD),
        "re-entry payload"
    );
    assert_eq!(events[0].key, events[1].key, "Lost refers to the tracked key");
    assert_eq!(
        events[0].text.as_deref(),
        Some("VIDEO-SESSION-2026"),
        "payload also decodes as text"
    );
    assert!(events[0].corners.is_some(), "FirstSeen carries corners");
    assert!(events[1].corners.is_none(), "Lost carries no corners");

    // The code disappears after frame 5 (t = 500) and the default timeout is
    // 500 ms, so Lost lands on the first frame at t >= 1000.
    assert_eq!(events[0].timestamp_millis, 0, "first sighting on frame 0");
    assert_eq!(events[1].timestamp_millis, 1000, "Lost after the 500 ms timeout");
    assert_eq!(events[2].timestamp_millis, 1300, "re-entry on frame 13");

    let stats = det.stats();
    assert_eq!(stats.frames, 20);
    assert_eq!(stats.first_seen, 2);
    assert_eq!(stats.lost, 1);
    assert!(stats.roi_scans > 0, "ROI tracking never engaged: {stats:?}");
    assert!(stats.roi_hits > 0, "ROI scans never hit: {stats:?}");
    assert!(
        stats.repeated >= 8,
        "repeat suppression should absorb the in-between sightings: {stats:?}"
    );
    assert_eq!(
        stats.detections,
        13,
        "13 frames contain the code: {stats:?}"
    );
    assert!(
        det.active_roi().is_some(),
        "ROI should still be active on the final visible frame"
    );
}

/// A sighting reported from an ROI scan must be flagged `via_roi`, with corners
/// mapped back into full-frame coordinates.
#[test]
fn roi_sourced_observation_reports_full_frame_corners() {
    let code = code();
    let (w, h) = (640u32, 300u32);
    let x = 300i64;
    let y = 80i64;

    let mut det = VideoDetector::new(Config {
        full_scan_interval: 0,
        emit_repeated: true,
        ..Config::default()
    });

    // Frame 1 establishes the ROI, frame 2 is scanned inside it.
    let mut last = Vec::new();
    for i in 0..2u64 {
        let g = frame_at(w, h, &code, Some((x, y)));
        last = det.process_frame(Frame::new(&g.px, g.w, g.h, i * FRAME_MS));
    }

    assert_eq!(last.len(), 1, "second frame should report the repeat");
    let obs = &last[0];
    assert!(obs.via_roi, "second frame should have been an ROI scan");
    let corners = obs.corners.expect("repeat carries corners");
    for c in corners {
        assert!(
            c.x > x as f64 && c.x < (x + code.w as i64) as f64,
            "corner x {} outside the composited code, ROI offset not applied",
            c.x
        );
        assert!(
            c.y > y as f64 && c.y < (y + code.h as i64) as f64,
            "corner y {} outside the composited code, ROI offset not applied",
            c.y
        );
    }
    assert!(det.stats().roi_hits >= 1);
}

// ---------------------------------------------------------------------------
// KG-5: border positions
// ---------------------------------------------------------------------------

/// The code flush against the top-left corner. ROI padding would run negative,
/// so it must clamp to `(0, 0)` — and the code must still be found.
#[test]
fn code_at_top_left_border() {
    let code = code();
    let (w, h) = (400u32, 260u32);
    let g = frame_at(w, h, &code, Some((0, 0)));

    let mut det = VideoDetector::with_defaults();
    let events = det.process_frame(Frame::new(&g.px, g.w, g.h, 0));
    assert_eq!(events.len(), 1, "code at the top-left border must be found");
    assert_eq!(events[0].bytes.as_deref(), Some(PAYLOAD));

    let (rx, ry, rw, rh) = det.active_roi().expect("ROI established");
    assert_eq!((rx, ry), (0, 0), "ROI must clamp at the top-left border");
    assert!(rx + rw <= w && ry + rh <= h, "ROI must stay inside the frame");

    // The next frame is scanned through that clamped ROI and must still hit.
    let g2 = frame_at(w, h, &code, Some((0, 0)));
    det.process_frame(Frame::new(&g2.px, g2.w, g2.h, FRAME_MS));
    let stats = det.stats();
    assert_eq!(stats.roi_scans, 1, "second frame uses the ROI: {stats:?}");
    assert_eq!(stats.roi_hits, 1, "clamped ROI must still hit: {stats:?}");
}

/// The mirror case: flush against the bottom-right corner, where padding would
/// run past the frame extent.
#[test]
fn code_at_bottom_right_border() {
    let code = code();
    let (w, h) = (400u32, 260u32);
    let g = frame_at(
        w,
        h,
        &code,
        Some((w as i64 - code.w as i64, h as i64 - code.h as i64)),
    );

    let mut det = VideoDetector::with_defaults();
    let events = det.process_frame(Frame::new(&g.px, g.w, g.h, 0));
    assert_eq!(events.len(), 1, "code at the bottom-right border must be found");

    let (rx, ry, rw, rh) = det.active_roi().expect("ROI established");
    assert_eq!(rx + rw, w, "ROI must clamp at the right edge");
    assert_eq!(ry + rh, h, "ROI must clamp at the bottom edge");
}

/// A code only partially inside the frame is simply not detected; the detector
/// must handle that without panicking and must leave no stale ROI behind.
#[test]
fn code_partially_off_frame_is_handled() {
    let code = code();
    let (w, h) = (400u32, 260u32);
    let g = frame_at(w, h, &code, Some((-(code.w as i64) / 2, 40)));

    let mut det = VideoDetector::with_defaults();
    let events = det.process_frame(Frame::new(&g.px, g.w, g.h, 0));
    assert!(events.is_empty(), "half a symbol must not decode");
    assert!(det.active_roi().is_none(), "no ROI without a detection");
}

// ---------------------------------------------------------------------------
// KG-5: full-scan refresh cadence and ROI abandonment
// ---------------------------------------------------------------------------

/// With `full_scan_interval = 3`, every third frame ignores the ROI and rescans
/// the whole image, so a second code appearing outside the tracked box is still
/// found.
#[test]
fn full_scan_refresh_follows_configured_cadence() {
    let code = code();
    let (w, h) = (400u32, 260u32);
    let frames: Vec<Gray> = (0..9)
        .map(|_| frame_at(w, h, &code, Some((60, 40))))
        .collect();

    let mut det = VideoDetector::new(Config {
        full_scan_interval: 3,
        ..Config::default()
    });
    let mut clip = Clip::new(frames);
    det.run(&mut clip, |_| {});

    let stats = det.stats();
    // Frame 1 has no ROI yet; frames 3, 6 and 9 are cadence refreshes.
    assert_eq!(stats.full_scans, 4, "cadence refreshes: {stats:?}");
    assert_eq!(stats.roi_scans, 5, "remaining frames use the ROI: {stats:?}");
    assert_eq!(stats.roi_hits, 5, "stationary code always hits: {stats:?}");
    assert_eq!(stats.roi_misses, 0, "{stats:?}");
    assert_eq!(stats.roi_fallbacks, 0, "{stats:?}");
}

/// A refresh must actually widen coverage: with the cadence disabled a code
/// that appears outside the ROI is missed on that frame (and only recovered by
/// the miss-fallback), whereas the refresh frame sees it directly.
#[test]
fn full_scan_refresh_zero_disables_periodic_rescan() {
    let code = code();
    let (w, h) = (400u32, 260u32);
    let frames: Vec<Gray> = (0..6)
        .map(|_| frame_at(w, h, &code, Some((60, 40))))
        .collect();

    let mut det = VideoDetector::new(Config {
        full_scan_interval: 0,
        ..Config::default()
    });
    let mut clip = Clip::new(frames);
    det.run(&mut clip, |_| {});

    let stats = det.stats();
    assert_eq!(stats.full_scans, 1, "only the very first frame: {stats:?}");
    assert_eq!(stats.roi_scans, 5, "every later frame stays in ROI: {stats:?}");
    assert_eq!(stats.roi_hits, 5, "{stats:?}");
}

/// When the code jumps outside the tracked box the ROI scan misses, a
/// same-frame full-frame fallback recovers it, and the ROI re-centres.
#[test]
fn roi_miss_triggers_full_frame_fallback() {
    let code = code();
    let (w, h) = (640u32, 300u32);
    let positions = [(40i64, 60i64), (40, 60), (420, 60), (420, 60)];
    let frames: Vec<Gray> = positions
        .iter()
        .map(|&p| frame_at(w, h, &code, Some(p)))
        .collect();

    let mut det = VideoDetector::new(Config {
        full_scan_interval: 0,
        ..Config::default()
    });
    let mut clip = Clip::new(frames);
    let mut events = Vec::new();
    det.run(&mut clip, |o| events.push(o));

    let stats = det.stats();
    assert_eq!(stats.roi_scans, 3, "frames 2..4 attempt the ROI: {stats:?}");
    assert_eq!(stats.roi_misses, 1, "the jump must miss once: {stats:?}");
    assert_eq!(stats.roi_fallbacks, 1, "and fall back once: {stats:?}");
    assert_eq!(stats.roi_hits, 2, "{stats:?}");
    assert_eq!(
        stats.detections, 4,
        "the fallback must not lose the frame: {stats:?}"
    );
    // Same payload throughout, so the jump is absorbed by de-duplication.
    assert_eq!(stats.first_seen, 1, "{stats:?}");
    assert_eq!(events.len(), 1, "only the initial FirstSeen is emitted");
}

/// A malformed frame (luma length not matching the geometry) must be treated as
/// empty rather than panicking, and must still drive the Lost timer.
#[test]
fn malformed_frame_is_survivable() {
    let code = code();
    let (w, h) = (400u32, 260u32);
    let g = frame_at(w, h, &code, Some((60, 40)));

    let mut det = VideoDetector::with_defaults();
    let first = det.process_frame(Frame::new(&g.px, g.w, g.h, 0));
    assert_eq!(first.len(), 1);

    let truncated = &g.px[..g.px.len() / 2];
    let events = det.process_frame(Frame::new(truncated, w, h, 900));
    assert_eq!(events.len(), 1, "the tracked code times out");
    assert_eq!(events[0].kind, EventKind::Lost);
    assert!(det.active_roi().is_none(), "ROI dropped on an empty frame");
}
