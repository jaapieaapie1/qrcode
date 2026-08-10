//! End-to-end tests for the stateful video detector.
//!
//! Fixtures are generated in-test: we hand-draw a QR-shaped module grid (three
//! finder patterns, separators, timing, and — for v >= 2 — the bottom-right
//! alignment pattern, plus deterministic "data" modules), render it to a
//! grayscale tile, and blit that tile onto a larger canvas at chosen offsets to
//! synthesise a moving code. This mirrors the fixture strategy in
//! `qr-detect/tests/detect.rs` and needs no dependency on qr-encode / qr-render.
//!
//! The synthetic grid is not a valid QR payload, so `qr-decode` returns `None`;
//! the detector then falls back to hashing the sampled module grid for identity,
//! which is exactly the code path these tests exercise.

use qr_core::BitMatrix;
use qr_video::{Config, EventKind, Frame, FrameSource, VideoDetector};

// ---------------------------------------------------------------------------
// Synthetic QR module grid (adapted from qr-detect's test fixtures)
// ---------------------------------------------------------------------------

fn dimension_for(version: u8) -> u32 {
    17 + 4 * version as u32
}

fn draw_finder(m: &mut BitMatrix, ox: u32, oy: u32) {
    for dy in 0..7 {
        for dx in 0..7 {
            let border = dx == 0 || dx == 6 || dy == 0 || dy == 6;
            let core = (2..=4).contains(&dx) && (2..=4).contains(&dy);
            m.set(ox + dx, oy + dy, border || core);
        }
    }
}

fn draw_alignment(m: &mut BitMatrix, cx: u32, cy: u32) {
    for dy in 0..5i32 {
        for dx in 0..5i32 {
            let ax = cx as i32 - 2 + dx;
            let ay = cy as i32 - 2 + dy;
            let ring = dx == 0 || dx == 4 || dy == 0 || dy == 4;
            let center = dx == 2 && dy == 2;
            m.set(ax as u32, ay as u32, ring || center);
        }
    }
}

/// A distinct deterministic module grid per `seed`, so two fixtures hash to
/// different identities.
fn synth_matrix(version: u8, seed: u32) -> BitMatrix {
    let dim = dimension_for(version);
    let mut m = BitMatrix::new(dim);

    for y in 0..dim {
        for x in 0..dim {
            let mut h = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663))
                .wrapping_add(seed.wrapping_mul(2_654_435_761))
                .wrapping_add(1);
            h ^= h >> 13;
            h = h.wrapping_mul(0x5bd1_e995);
            h ^= h >> 15;
            m.set(x, y, h & 1 == 0);
        }
    }

    draw_finder(&mut m, 0, 0);
    draw_finder(&mut m, dim - 7, 0);
    draw_finder(&mut m, 0, dim - 7);

    for i in 0..8 {
        m.set(7, i, false);
        m.set(i, 7, false);
        m.set(dim - 8, i, false);
        m.set(dim - 1 - i, 7, false);
        m.set(7, dim - 1 - i, false);
        m.set(i, dim - 8, false);
    }

    for k in 8..dim - 8 {
        m.set(k, 6, k % 2 == 0);
        m.set(6, k, k % 2 == 0);
    }

    if version >= 2 {
        let last = dim - 7;
        draw_alignment(&mut m, last, last);
    }

    m
}

// ---------------------------------------------------------------------------
// Rendering + canvas compositing
// ---------------------------------------------------------------------------

struct Tile {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

/// Renders a module grid to a grayscale tile: `scale` px per module, `quiet`
/// modules of white border. Dark modules -> 0, light -> 255.
fn render_tile(m: &BitMatrix, scale: u32, quiet: u32) -> Tile {
    let dim = m.size();
    let side = (dim + 2 * quiet) * scale;
    let mut px = vec![255u8; (side * side) as usize];
    for y in 0..dim {
        for x in 0..dim {
            let v = if m.get(x, y) { 0 } else { 255 };
            for py in 0..scale {
                for pxx in 0..scale {
                    let cx = (quiet + x) * scale + pxx;
                    let cy = (quiet + y) * scale + py;
                    px[(cy * side + cx) as usize] = v;
                }
            }
        }
    }
    Tile {
        w: side,
        h: side,
        px,
    }
}

/// A white canvas that tiles can be blitted onto.
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Canvas {
        Canvas {
            w,
            h,
            px: vec![255u8; (w * h) as usize],
        }
    }

    fn clear(&mut self) {
        self.px.iter_mut().for_each(|p| *p = 255);
    }

    /// Blits `tile` with its top-left corner at `(ox, oy)`, clipped to bounds.
    fn blit(&mut self, tile: &Tile, ox: u32, oy: u32) {
        for ty in 0..tile.h {
            let cy = oy + ty;
            if cy >= self.h {
                break;
            }
            for tx in 0..tile.w {
                let cx = ox + tx;
                if cx >= self.w {
                    break;
                }
                self.px[(cy * self.w + cx) as usize] = tile.px[(ty * tile.w + tx) as usize];
            }
        }
    }
}

// ---------------------------------------------------------------------------
// A synthetic moving-QR frame source (the tests-only FrameSource)
// ---------------------------------------------------------------------------

/// Emits `count` frames of one QR tile translated by `step` px per frame along
/// a diagonal. Owns a single reusable canvas buffer, so each returned [`Frame`]
/// borrows it — exercising the lending [`FrameSource`] contract.
struct MovingQrSource {
    canvas: Canvas,
    tile: Tile,
    step: u32,
    start: (u32, u32),
    count: u32,
    index: u32,
    frame_ms: u64,
}

impl MovingQrSource {
    fn new(
        tile: Tile,
        canvas_w: u32,
        canvas_h: u32,
        start: (u32, u32),
        step: u32,
        count: u32,
    ) -> MovingQrSource {
        MovingQrSource {
            canvas: Canvas::new(canvas_w, canvas_h),
            tile,
            step,
            start,
            count,
            index: 0,
            frame_ms: 33,
        }
    }
}

impl FrameSource for MovingQrSource {
    fn next_frame(&mut self) -> Option<Frame<'_>> {
        if self.index >= self.count {
            return None;
        }
        let i = self.index;
        self.index += 1;
        let ox = self.start.0 + i * self.step;
        let oy = self.start.1 + i * self.step;
        self.canvas.clear();
        // Disjoint field borrows: blit reads `self.tile`, writes `self.canvas`.
        self.canvas.blit(&self.tile, ox, oy);
        let ts = i as u64 * self.frame_ms;
        Some(Frame::new(
            &self.canvas.px,
            self.canvas.w,
            self.canvas.h,
            ts,
        ))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Config that keeps the ROI path engaged: no periodic full-scan refresh, and
/// repeats surfaced so tests can inspect `via_roi` on every frame.
fn roi_config() -> Config {
    Config {
        full_scan_interval: 0,
        emit_repeated: true,
        ..Config::default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// After a full-frame hit, the very next frame must be served by the ROI path,
/// and the resulting detection must be flagged `via_roi`.
#[test]
fn roi_tracking_engages_after_first_hit() {
    let m = synth_matrix(2, 1);
    let tile = render_tile(&m, 8, 4);
    let canvas_w = tile.w + 120;
    let canvas_h = tile.h + 120;
    let mut canvas = Canvas::new(canvas_w, canvas_h);
    canvas.blit(&tile, 30, 30);

    let mut det = VideoDetector::new(roi_config());

    // Frame 0: cold — full-frame scan, first sighting.
    let f0 = Frame::new(&canvas.px, canvas.w, canvas.h, 0);
    let o0 = det.process_frame(f0);
    assert_eq!(o0.len(), 1, "frame 0 should report one FirstSeen");
    assert_eq!(o0[0].kind, EventKind::FirstSeen);
    assert!(!o0[0].via_roi, "frame 0 is a full-frame scan");
    assert!(
        det.active_roi().is_some(),
        "an ROI must be armed after a hit"
    );
    let s0 = det.stats();
    assert_eq!(s0.full_scans, 1);
    assert_eq!(s0.roi_scans, 0);

    // Frame 1: warm — ROI scan path, and it must hit.
    let f1 = Frame::new(&canvas.px, canvas.w, canvas.h, 33);
    let o1 = det.process_frame(f1);
    let s1 = det.stats();
    assert_eq!(s1.roi_scans, 1, "frame 1 must take the ROI path");
    assert_eq!(s1.roi_hits, 1, "the ROI scan must find the code");
    assert_eq!(s1.roi_misses, 0);
    assert_eq!(o1.len(), 1, "frame 1 reports the code again (Repeated)");
    assert_eq!(o1[0].kind, EventKind::Repeated);
    assert!(o1[0].via_roi, "frame 1 detection came from the ROI path");
}

/// The same static code across many closely-spaced frames yields exactly one
/// FirstSeen; the rest are debounced repeats.
#[test]
fn dedup_debounces_repeated_payload() {
    let m = synth_matrix(1, 7);
    let tile = render_tile(&m, 8, 4);
    let mut canvas = Canvas::new(tile.w + 80, tile.h + 80);
    canvas.blit(&tile, 20, 20);

    // Default config suppresses Repeated from the output.
    let mut det = VideoDetector::with_defaults();
    let mut first_seen = 0;
    for i in 0..8u64 {
        let f = Frame::new(&canvas.px, canvas.w, canvas.h, i * 33);
        for obs in det.process_frame(f) {
            match obs.kind {
                EventKind::FirstSeen => first_seen += 1,
                EventKind::Repeated => panic!("repeats must be suppressed by default"),
                EventKind::Lost => panic!("code never disappears"),
            }
        }
    }
    assert_eq!(first_seen, 1, "one FirstSeen across the whole burst");
    let s = det.stats();
    assert_eq!(s.first_seen, 1);
    assert_eq!(s.repeated, 7, "the other seven sightings are debounced");
}

/// A code that appears, vanishes long enough to time out, then reappears must
/// produce FirstSeen -> Lost -> FirstSeen.
#[test]
fn lost_then_refound_is_reported() {
    let m = synth_matrix(2, 3);
    let tile = render_tile(&m, 8, 4);
    let mut present = Canvas::new(tile.w + 80, tile.h + 80);
    present.blit(&tile, 20, 20);
    let blank = Canvas::new(tile.w + 80, tile.h + 80);

    // lost_after < the blank gap; debounce short enough that the return counts
    // as fresh regardless.
    let cfg = Config {
        full_scan_interval: 0,
        lost_after_millis: 100,
        debounce_millis: 100,
        ..Config::default()
    };
    let mut det = VideoDetector::new(cfg);
    let mut kinds = Vec::new();

    // Present for three frames.
    for i in 0..3u64 {
        let f = Frame::new(&present.px, present.w, present.h, i * 33);
        kinds.extend(det.process_frame(f).into_iter().map(|o| o.kind));
    }
    // Absent for three frames, well past the 100 ms timeout.
    for i in 0..3u64 {
        let ts = 200 + i * 200;
        let f = Frame::new(&blank.px, blank.w, blank.h, ts);
        kinds.extend(det.process_frame(f).into_iter().map(|o| o.kind));
    }
    // Present again.
    for i in 0..3u64 {
        let ts = 1000 + i * 33;
        let f = Frame::new(&present.px, present.w, present.h, ts);
        kinds.extend(det.process_frame(f).into_iter().map(|o| o.kind));
    }

    assert_eq!(
        kinds,
        vec![EventKind::FirstSeen, EventKind::Lost, EventKind::FirstSeen,],
        "expected FirstSeen -> Lost -> FirstSeen, got {kinds:?}"
    );
    let s = det.stats();
    assert_eq!(s.first_seen, 2);
    assert_eq!(s.lost, 1);
}

/// A moving code is tracked as a single identity across >= 10 frames, mostly via
/// the ROI path, and reported exactly once.
#[test]
fn moving_code_tracked_across_frames() {
    let m = synth_matrix(2, 9);
    // Move by whole modules (one `scale`) each frame so the rasterised tile stays
    // pixel-identical and samples to the same module grid (stable identity).
    let scale = 8;
    let tile = render_tile(&m, scale, 4);
    let frames = 12u32;
    let step = scale; // one module per frame
    let travel = frames * step;
    let canvas_w = tile.w + travel + 60;
    let canvas_h = tile.h + travel + 60;

    let mut source = MovingQrSource::new(tile, canvas_w, canvas_h, (20, 20), step, frames);
    let mut det = VideoDetector::new(roi_config());

    let mut first_seen_keys = Vec::new();
    let mut detected_frames = 0u32;
    det.run(&mut source, |obs| match obs.kind {
        EventKind::FirstSeen => first_seen_keys.push(obs.key),
        EventKind::Repeated => detected_frames += 1,
        EventKind::Lost => panic!("moving code should never be lost while on-screen"),
    });

    assert_eq!(
        first_seen_keys.len(),
        1,
        "the moving code is one identity, reported once"
    );
    // Every frame after the first re-detected the same code.
    assert_eq!(
        detected_frames,
        frames - 1,
        "all subsequent frames re-detect the moving code"
    );

    let s = det.stats();
    assert_eq!(s.frames, frames as u64);
    assert!(
        s.roi_hits >= frames as u64 - 2,
        "ROI tracking should carry almost every frame, got {} hits",
        s.roi_hits
    );
    assert_eq!(s.first_seen, 1);
}

/// Two distinct codes in one frame are tracked as two independent identities.
#[test]
fn two_codes_two_identities() {
    let a = render_tile(&synth_matrix(1, 11), 8, 4);
    let b = render_tile(&synth_matrix(1, 22), 8, 4);
    let gap = 40u32;
    let mut canvas = Canvas::new(a.w + b.w + gap + 40, a.h.max(b.h) + 40);
    canvas.blit(&a, 20, 20);
    canvas.blit(&b, 20 + a.w + gap, 20);

    let mut det = VideoDetector::with_defaults();
    let obs = det.process_frame(Frame::new(&canvas.px, canvas.w, canvas.h, 0));
    assert_eq!(obs.len(), 2, "two codes -> two FirstSeen");
    assert!(obs.iter().all(|o| o.kind == EventKind::FirstSeen));
    assert_ne!(obs[0].key, obs[1].key, "distinct codes -> distinct keys");
}

/// A malformed frame (luma length mismatch) is handled gracefully: no panic, no
/// detections, timers still advance.
#[test]
fn malformed_frame_is_graceful() {
    let mut det = VideoDetector::with_defaults();
    let short = vec![255u8; 10];
    let obs = det.process_frame(Frame::new(&short, 100, 100, 0));
    assert!(obs.is_empty());
    assert_eq!(det.stats().frames, 1);
    assert_eq!(det.stats().detections, 0);
    assert!(det.active_roi().is_none());
}

/// A blank stream produces nothing and arms no ROI.
#[test]
fn blank_frames_yield_nothing() {
    let blank = Canvas::new(200, 200);
    let mut det = VideoDetector::with_defaults();
    for i in 0..5u64 {
        let obs = det.process_frame(Frame::new(&blank.px, blank.w, blank.h, i * 33));
        assert!(obs.is_empty());
    }
    assert!(det.active_roi().is_none());
    let s = det.stats();
    assert_eq!(s.detections, 0);
    assert_eq!(s.roi_scans, 0, "no ROI is ever armed on a blank stream");
}
