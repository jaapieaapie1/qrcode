//! qr-video: the two costs a frame can have.
//!
//! `VideoDetector` is stateful, so both benches warm the detector into the
//! state they mean to measure before the timed loop starts, and both assert
//! afterwards that the detector really stayed in that state — an ROI bench that
//! silently fell back to full scans would just be a slower copy of the other
//! bench.
//!
//! The frame is stationary across iterations. That is the point: steady-state
//! ROI tracking is what a phone pointed at a poster actually does, and it is
//! the case the ROI machinery exists to make cheap.
//!
//! The rate column here is frames per second.

mod common;

use benchmarks::{Unit, bench_units, group};
use common::{FRAME_H, FRAME_W, frame_with_at};
use qr_core::EcLevel;
use qr_video::{Config, Frame, VideoDetector};
use std::hint::black_box;

/// 8px/module, matching the detector benches.
const SCALE: u32 = 8;

/// Frame interval at 30fps, in milliseconds. Timestamps are synthesised from a
/// counter, never from the wall clock, so a run is reproducible.
const FRAME_INTERVAL_MS: u64 = 33;

fn main() {
    let (v5, _) = common::qr_fixture(5, EcLevel::M, 0x5eed_5eed);
    let frame = frame_with_at(&v5, SCALE, 400, 180);

    group("qr-video — process_frame(), 1280x720 (rate is frames/s)");

    // --- steady-state ROI tracking -----------------------------------------
    //
    // `full_scan_interval: 0` disables the periodic full-frame refresh, so once
    // the first frame has established an ROI every later frame takes the ROI
    // path — which is exactly the steady state being measured.
    let mut roi = VideoDetector::new(Config {
        full_scan_interval: 0,
        ..Config::default()
    });
    let mut clock = 0u64;
    roi.process_frame(Frame::new(&frame, FRAME_W, FRAME_H, clock));
    assert!(
        roi.active_roi().is_some(),
        "first frame must establish an ROI"
    );

    let before = roi.stats();
    bench_units(
        "steady-state ROI frame (code present)",
        5_000,
        Unit::Items(1),
        || {
            clock += FRAME_INTERVAL_MS;
            roi.process_frame(black_box(Frame::new(&frame, FRAME_W, FRAME_H, clock)))
        },
    );
    let after = roi.stats();
    assert_eq!(
        after.roi_misses, before.roi_misses,
        "ROI must not have missed: the bench would be measuring the fallback"
    );
    assert_eq!(
        after.full_scans, before.full_scans,
        "no frame in the ROI bench may have full-scanned"
    );
    assert!(after.roi_hits > before.roi_hits, "ROI must have hit");

    // --- full-scan cadence frame -------------------------------------------
    //
    // `full_scan_interval: 1` makes every frame a refresh frame, which is what
    // the periodic cadence costs when it comes round.
    let mut full = VideoDetector::new(Config {
        full_scan_interval: 1,
        ..Config::default()
    });
    let mut clock = 0u64;
    let before = full.stats();
    bench_units(
        "full-scan cadence frame (code present)",
        2_000,
        Unit::Items(1),
        || {
            clock += FRAME_INTERVAL_MS;
            full.process_frame(black_box(Frame::new(&frame, FRAME_W, FRAME_H, clock)))
        },
    );
    let after = full.stats();
    assert_eq!(
        after.roi_scans, before.roi_scans,
        "no frame in the full-scan bench may have taken the ROI path"
    );
    assert_eq!(
        after.full_scans - before.full_scans,
        after.frames - before.frames,
        "every frame in the full-scan bench must have full-scanned"
    );

    println!();
}
