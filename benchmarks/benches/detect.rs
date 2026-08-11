//! qr-detect over synthetic 720p frames.
//!
//! Every frame is 1280x720 luma, built once before the timed loop from a real
//! encoded symbol via qr-render. The four cases that matter operationally:
//!
//! * **happy path** — one v5 symbol, axis-aligned, decodes.
//! * **rotated** — the same frame turned 30 degrees, so the sampler runs a real
//!   perspective transform instead of an axis-aligned one.
//! * **miss path** — a symbol-free frame. This is the *worst* case, not the
//!   cheapest: finding nothing is exactly what makes `detect_luma` run its
//!   second, differently-binarized retry pass over the whole frame.
//! * **micro / mixed** — the M4 and `detect_all_luma` pipelines.
//!
//! `binarize` is a private module, so there is no public binarize-only bench.
//! The primary pass is measured instead through `detect_luma_single_pass`,
//! which is `detect_luma` minus the retry; the difference between the two on
//! the miss-path frame is the retry's cost.

mod common;

use benchmarks::{Unit, bench_units, group};
use common::{FRAME_H, FRAME_W, blank_frame, cluttered_frame, qr_fixture, rotate};
use qr_core::EcLevel;
use qr_detect::{detect_luma, detect_luma_single_pass};
use std::hint::black_box;

/// Module size of the rendered symbols, in pixels. 8px/module puts a v5 symbol
/// (37 modules + 8 quiet) at 360px on a 720-line frame — a plausible hand-held
/// framing, and well clear of the sampler's resolution floor.
const SCALE: u32 = 8;

/// Frames are only ever pixels; the throughput column is megapixels per second.
const FRAME_PIXELS: u64 = (FRAME_W * FRAME_H) as u64;

fn decoded_count(dets: &[qr_detect::Detection]) -> usize {
    dets.iter().filter(|d| d.decoded.is_some()).count()
}

fn main() {
    let (v5, payload) = qr_fixture(5, EcLevel::M, 0x5eed_5eed);
    let happy = common::frame_with(&v5, SCALE);
    let rotated = rotate(&happy, 30.0);
    let empty = cluttered_frame();
    let flat = blank_frame();

    // Fixture sanity, so a bench can never quietly measure the wrong path.
    let happy_dets = detect_luma(&happy, FRAME_W, FRAME_H);
    assert_eq!(
        decoded_count(&happy_dets),
        1,
        "happy frame must decode once"
    );
    assert_eq!(
        happy_dets[0].decoded.as_ref().expect("decoded").bytes,
        payload,
        "happy frame must decode to the payload it was built from"
    );
    let rotated_dets = detect_luma(&rotated, FRAME_W, FRAME_H);
    assert_eq!(
        decoded_count(&rotated_dets),
        1,
        "rotated frame must decode once"
    );
    assert_eq!(
        rotated_dets[0].decoded.as_ref().expect("decoded").bytes,
        payload,
        "rotated frame must decode to the same payload as the upright one"
    );
    assert!(
        detect_luma(&empty, FRAME_W, FRAME_H).is_empty(),
        "cluttered miss-path frame must find nothing"
    );
    // The flat frame carries the 6.92 ms headline, so its emptiness is a claim
    // that has to hold, not an assumption.
    assert!(
        detect_luma(&flat, FRAME_W, FRAME_H).is_empty(),
        "flat miss-path frame must find nothing"
    );

    // Three benches run on `detect_luma_single_pass`. If it ever early-returned,
    // they would silently measure a stub, so pin both of its outcomes here.
    assert_eq!(
        decoded_count(&detect_luma_single_pass(&happy, FRAME_W, FRAME_H)),
        1,
        "single pass must decode the happy frame on its own"
    );
    for (label, frame) in [("cluttered", &empty), ("flat", &flat)] {
        assert!(
            detect_luma_single_pass(frame, FRAME_W, FRAME_H).is_empty(),
            "single pass must find nothing in the {label} miss-path frame"
        );
    }

    group("qr-detect — detect_luma(), 1280x720");

    bench_units(
        "happy path: 1 v5 symbol @8px/module",
        2_000,
        Unit::Items(FRAME_PIXELS),
        || detect_luma(black_box(&happy), FRAME_W, FRAME_H),
    );
    bench_units(
        "happy path, single pass (no retry)",
        2_000,
        Unit::Items(FRAME_PIXELS),
        || detect_luma_single_pass(black_box(&happy), FRAME_W, FRAME_H),
    );
    bench_units(
        "rotated 30 degrees",
        2_000,
        Unit::Items(FRAME_PIXELS),
        || detect_luma(black_box(&rotated), FRAME_W, FRAME_H),
    );
    bench_units(
        "miss path: no symbol (primary + retry)",
        2_000,
        Unit::Items(FRAME_PIXELS),
        || detect_luma(black_box(&empty), FRAME_W, FRAME_H),
    );
    bench_units(
        "miss path, single pass (no retry)",
        2_000,
        Unit::Items(FRAME_PIXELS),
        || detect_luma_single_pass(black_box(&empty), FRAME_W, FRAME_H),
    );
    bench_units(
        "miss path: flat frame (no symbol, no clutter)",
        2_000,
        Unit::Items(FRAME_PIXELS),
        || detect_luma(black_box(&flat), FRAME_W, FRAME_H),
    );
    bench_units(
        "miss path: flat frame, single pass (no retry)",
        2_000,
        Unit::Items(FRAME_PIXELS),
        || detect_luma_single_pass(black_box(&flat), FRAME_W, FRAME_H),
    );

    #[cfg(feature = "micro")]
    {
        use qr_core::micro::{MicroEcLevel, MicroVersion};
        use qr_detect::{detect_all_luma, detect_micro_luma};
        use qr_encode::MicroOptions;

        let m4 = qr_encode::encode_micro(
            b"HELLO M4Q",
            MicroOptions {
                version: Some(MicroVersion::M4),
                ..MicroOptions::new(MicroEcLevel::Q)
            },
        )
        .expect("M4-Q fixture must encode");
        assert_eq!(m4.size(), 17);

        let micro_frame = common::frame_with(&m4, SCALE);

        // Mixed frame: the v5 on the left, the M4 on the right, both clear of
        // each other's quiet zone.
        let mut mixed = blank_frame();
        let (qr_tile, qw, qh) = common::render_luma(&v5, SCALE);
        let (m_tile, mw, mh) = common::render_luma(&m4, SCALE);
        common::blit(&mut mixed, &qr_tile, qw, qh, 120, (FRAME_H - qh) / 2);
        common::blit(&mut mixed, &m_tile, mw, mh, 820, (FRAME_H - mh) / 2);

        assert_eq!(
            detect_micro_luma(&micro_frame, FRAME_W, FRAME_H)
                .iter()
                .filter(|d| d.decoded.is_some())
                .count(),
            1,
            "micro frame must decode once"
        );
        let (qrs, micros) = detect_all_luma(&mixed, FRAME_W, FRAME_H);
        assert_eq!(
            (
                decoded_count(&qrs),
                micros.iter().filter(|d| d.decoded.is_some()).count()
            ),
            (1, 1),
            "mixed frame must decode one QR and one micro"
        );

        group("qr-detect — Micro QR, 1280x720");

        bench_units(
            "detect_micro_luma: 1 M4 symbol @8px/module",
            2_000,
            Unit::Items(FRAME_PIXELS),
            || detect_micro_luma(black_box(&micro_frame), FRAME_W, FRAME_H),
        );
        bench_units(
            "detect_all_luma: 1 v5 + 1 M4",
            2_000,
            Unit::Items(FRAME_PIXELS),
            || detect_all_luma(black_box(&mixed), FRAME_W, FRAME_H),
        );
    }

    println!();
}
