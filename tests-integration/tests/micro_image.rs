//! Suite 7 — Micro QR through the image pipeline: `encode_micro` ->
//! `qr-render` -> `image::GrayImage` -> `qr-detect::detect_micro` ->
//! `qr-decode::decode_micro`.
//!
//! Micro symbols carry a single finder and a 2-module quiet zone (ISO 6.3.6),
//! so they give the detector far less to lock onto than a QR triple does. This
//! suite pins the honest envelope: pixels per module, rotation, noise, and
//! co-existence with QR symbols in the same frame.
//!
//! It also carries the cross-crate regression for the ISO 7.5.1 misdecode
//! caps ([`v1_l_three_codeword_errors_never_misdecode`]) — the same guarantee
//! M1 relies on, checked at the level where it matters: a real image.

mod common;

use common::{
    Content, Gray, add_noise, compose, corrupt_codeword, data_module_order, micro_content_kinds,
    micro_label, micro_max_chars, micro_mode, pad, payload, render_gray, rotate, rotate_quarters,
    ver,
};
use qr_core::micro::{MicroSymbolInfo, MicroVersion, micro_symbols};
use qr_core::{BitMatrix, EcLevel};
use qr_detect::{detect_all, detect_micro};
use qr_encode::{MicroOptions, Options, encode, encode_micro};

/// The quiet zone a micro symbol requires (ISO 6.3.6): 2 modules, against the
/// 4 a QR symbol wants.
const MICRO_QUIET: u32 = 2;

/// Asserts two matrices agree module for module.
fn assert_same_matrix(got: &BitMatrix, want: &BitMatrix, label: &str) {
    assert_eq!(got.size(), want.size(), "{label}: sampled dimension");
    let mismatches = (0..want.size())
        .flat_map(|y| (0..want.size()).map(move |x| (x, y)))
        .filter(|&(x, y)| got.get(x, y) != want.get(x, y))
        .count();
    assert_eq!(
        mismatches, 0,
        "{label}: {mismatches} sampled module mismatches"
    );
}

/// Capacity-filling payload in the widest mode the version supports, plus the
/// encoded symbol.
fn fixture(info: &MicroSymbolInfo, mask: Option<u8>) -> (Vec<u8>, BitMatrix) {
    let kind = *micro_content_kinds(info.version)
        .last()
        .expect("every micro version supports numeric");
    let len = micro_max_chars(micro_mode(kind), info);
    let data = payload(kind, len);
    let m = encode_micro(
        &data,
        MicroOptions {
            ec: info.ec,
            version: Some(info.version),
            mask,
        },
    )
    .expect("encode micro fixture");
    (data, m)
}

/// Detects exactly one micro symbol in `g` and asserts it decodes to `data`.
fn assert_micro_decodes(g: &Gray, want_matrix: &BitMatrix, data: &[u8], label: &str) {
    let dets = detect_micro(&g.to_image());
    assert_eq!(dets.len(), 1, "{label}: expected exactly one micro symbol");
    let det = &dets[0];
    assert_same_matrix(&det.matrix, want_matrix, label);
    let decoded = det
        .decoded
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: detected but did not decode"));
    assert_eq!(decoded.bytes, data, "{label}: payload bytes");
    assert_eq!(
        decoded.errors_corrected, 0,
        "{label}: clean render, no repair"
    );
}

// ---------------------------------------------------------------------------
// Scale sweep
// ---------------------------------------------------------------------------

/// Every symbol, every content kind it supports, at 8 px per module: the
/// baseline that must never regress.
#[test]
fn image_path_all_symbols_all_modes() {
    for info in micro_symbols() {
        for &kind in micro_content_kinds(info.version) {
            let len = micro_max_chars(micro_mode(kind), info);
            let data = payload(kind, len);
            let m = encode_micro(
                &data,
                MicroOptions {
                    ec: info.ec,
                    version: Some(info.version),
                    mask: None,
                },
            )
            .expect("encode");
            let g = render_gray(&m, 8, MICRO_QUIET);
            assert_micro_decodes(
                &g,
                &m,
                &data,
                &format!("{} {} @8px", micro_label(info), kind.label()),
            );
        }
    }
}

/// 2 to 10 pixels per module for every symbol.
///
/// Two is the measured floor for an integer-scaled, noise-free render: at 1
/// px/module the detector finds nothing, which is the honest envelope rather
/// than a bug — an M1 symbol plus its quiet zone is 15 px square at that scale.
#[test]
fn image_path_scale_sweep_2_to_10() {
    for info in micro_symbols() {
        let (data, m) = fixture(info, None);
        for scale in 2..=10u32 {
            let g = render_gray(&m, scale, MICRO_QUIET);
            assert_micro_decodes(
                &g,
                &m,
                &data,
                &format!("{} @{scale}px/module", micro_label(info)),
            );
        }
    }
}

/// All four micro data masks driven through the image path, so a mask-specific
/// sampling or format-read bug cannot hide behind auto-selection.
#[test]
fn image_path_all_four_masks() {
    for info in micro_symbols() {
        for mask in 0..4u8 {
            let (data, m) = fixture(info, Some(mask));
            let g = render_gray(&m, 8, MICRO_QUIET);
            assert_micro_decodes(&g, &m, &data, &format!("{} mask {mask}", micro_label(info)));
        }
    }
}

// ---------------------------------------------------------------------------
// Rotation
// ---------------------------------------------------------------------------

/// Detects one micro symbol in a rotated frame and asserts the payload; the
/// sampled matrix is not compared, because resampling a rotated raster is
/// allowed to differ from the ideal grid as long as the payload survives.
fn assert_rotated_decodes(g: &Gray, data: &[u8], label: &str) {
    let dets = detect_micro(&g.to_image());
    assert_eq!(dets.len(), 1, "{label}: expected exactly one micro symbol");
    let decoded = dets[0]
        .decoded
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: detected but did not decode"));
    assert_eq!(decoded.bytes, data, "{label}: payload bytes");
}

/// 0 to 45 degrees in 5 degree steps, on every symbol — M1 especially, being
/// the smallest (11×11) with no error correction (`r = 0`), is the
/// highest-value rotation case. Beyond 45 degrees the symbol is better
/// described by the next quarter turn, which [`rotation_quarter_turns`]
/// covers exactly.
#[test]
fn rotation_sweep_0_to_45_degrees() {
    for info in micro_symbols() {
        let (data, m) = fixture(info, None);
        let base = pad(&render_gray(&m, 10, MICRO_QUIET), 60);
        for step in 0..=9u32 {
            let deg = (step * 5) as f64;
            let g = rotate(&base, deg.to_radians());
            assert_rotated_decodes(
                &g,
                &data,
                &format!("{} rotated {deg}deg", micro_label(info)),
            );
        }
    }
}

/// Exact quarter turns are resampling-free, so every symbol must survive all
/// four orientations.
#[test]
fn rotation_quarter_turns() {
    for info in micro_symbols() {
        let (data, m) = fixture(info, None);
        let base = render_gray(&m, 8, MICRO_QUIET);
        for quarters in 0..4u32 {
            let g = rotate_quarters(&base, quarters);
            assert_rotated_decodes(
                &g,
                &data,
                &format!("{} rotated {}deg", micro_label(info), quarters * 90),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Noise
// ---------------------------------------------------------------------------

/// Mild sensor-style noise — the same amplitude the QR suite calls "visibly
/// degrades the image but stays below the binarisation threshold margin" — must
/// not disturb any micro symbol, including detection-only M1.
#[test]
fn mild_noise() {
    for info in micro_symbols() {
        let (data, m) = fixture(info, None);
        let mut g = render_gray(&m, 10, MICRO_QUIET);
        add_noise(
            &mut g,
            45,
            0x51CE_A5A5_u64.wrapping_mul(info.symbol_number as u64 + 1),
        );
        assert_rotated_decodes(&g, &data, &format!("{} + noise a=45", micro_label(info)));
    }
}

/// Heavier noise on the most redundant micro symbol, where Reed-Solomon is
/// allowed to mop up whatever modules flip.
#[test]
fn heavy_noise_m4_q() {
    let info = micro_symbols()
        .iter()
        .find(|i| i.version == MicroVersion::M4 && i.ec == qr_core::micro::MicroEcLevel::Q)
        .expect("M4-Q exists");
    let (data, m) = fixture(info, None);
    let mut g = render_gray(&m, 10, MICRO_QUIET);
    add_noise(&mut g, 70, 0x1357_9BDF);
    assert_rotated_decodes(&g, &data, "M4-Q + noise a=70");
}

// ---------------------------------------------------------------------------
// Mixed frames
// ---------------------------------------------------------------------------

/// A QR symbol and a micro symbol in one image: `detect_all` must return one of
/// each, with no cross-interference — the QR triple must not swallow the micro
/// finder, and the micro path must not claim any of the QR's three.
#[test]
fn qr_and_micro_in_one_image() {
    let qr_payload = b"QR-SIDE-PAYLOAD-42";
    let micro_payload = b"MICRO99";

    let qr = encode(qr_payload, Options::new(EcLevel::M)).expect("encode qr");
    let micro = encode_micro(
        micro_payload,
        MicroOptions::new(qr_core::micro::MicroEcLevel::L),
    )
    .expect("encode micro");

    let gq = render_gray(&qr, 8, 4);
    let gm = render_gray(&micro, 8, MICRO_QUIET);

    let gap = 64u32;
    let w = gq.w + gap + gm.w + 2 * gap;
    let h = gq.h.max(gm.h) + 2 * gap;
    let mut canvas = compose(w, h, &gq, gap as i64, gap as i64);
    let placed = compose(w, h, &gm, (gap + gq.w + gap) as i64, gap as i64);
    for y in 0..h {
        for x in 0..w {
            if placed.get(x, y) != 255 {
                canvas.set(x, y, placed.get(x, y));
            }
        }
    }

    let (qrs, micros) = detect_all(&canvas.to_image());
    assert_eq!(qrs.len(), 1, "expected exactly one QR symbol");
    assert_eq!(micros.len(), 1, "expected exactly one micro symbol");
    assert_eq!(
        qrs[0].decoded.as_ref().expect("qr decodes").bytes,
        qr_payload,
        "QR payload"
    );
    assert_eq!(
        micros[0].decoded.as_ref().expect("micro decodes").bytes,
        micro_payload,
        "micro payload"
    );
}

/// Two micro symbols side by side must both be found and must decode to their
/// own payloads.
#[test]
fn two_micros_in_one_image() {
    let a_payload = b"ALPHA-1";
    let b_payload = b"BETA-22";
    let a = encode_micro(
        a_payload,
        MicroOptions::new(qr_core::micro::MicroEcLevel::L),
    )
    .expect("encode a");
    let b = encode_micro(
        b_payload,
        MicroOptions::new(qr_core::micro::MicroEcLevel::M),
    )
    .expect("encode b");

    let ga = render_gray(&a, 8, MICRO_QUIET);
    let gb = render_gray(&b, 8, MICRO_QUIET);

    let gap = 64u32;
    let w = gap + ga.w + gap + gb.w + gap;
    let h = gap + ga.h.max(gb.h) + gap;
    let mut canvas = compose(w, h, &ga, gap as i64, gap as i64);
    let placed = compose(w, h, &gb, (gap + ga.w + gap) as i64, gap as i64);
    for y in 0..h {
        for x in 0..w {
            if placed.get(x, y) != 255 {
                canvas.set(x, y, placed.get(x, y));
            }
        }
    }

    let dets = detect_micro(&canvas.to_image());
    assert_eq!(dets.len(), 2, "expected two micro symbols");
    let mut payloads: Vec<Vec<u8>> = dets
        .into_iter()
        .map(|d| d.decoded.expect("both micros must decode").bytes)
        .collect();
    payloads.sort();
    let mut want = vec![a_payload.to_vec(), b_payload.to_vec()];
    want.sort();
    assert_eq!(payloads, want);
}

// ---------------------------------------------------------------------------
// Misdecode-cap regression (ISO 7.5.1)
// ---------------------------------------------------------------------------

/// Version 1-L reserves `p = 3` of its 7 error correction codewords for
/// detection, so its published correction capacity is `r = 2`, not the
/// `floor(7 / 2) = 3` that raw Reed-Solomon would solve.
///
/// Through the full image path: three corrupted codewords must still be
/// *detected* as a symbol (the finder patterns are untouched) and must yield a
/// clean decode failure. Returning a confidently wrong payload here is the
/// failure this test exists to prevent.
#[test]
fn v1_l_three_codeword_errors_never_misdecode() {
    let version = ver(1);
    let data = b"MISDECODE-CHECK-1";
    let mut m = encode(data, {
        let mut o = Options::new(EcLevel::L);
        o.version = Some(version);
        o.mask = Some(0);
        o
    })
    .expect("encode v1-L");
    let clean = m.clone();

    let order = data_module_order(version);
    for i in 0..3 {
        corrupt_codeword(&mut m, &order, i);
    }

    let g = render_gray(&m, 8, 4);
    let dets = qr_detect::detect(&g.to_image());
    assert_eq!(dets.len(), 1, "the damaged symbol must still be detected");
    assert_same_matrix(&dets[0].matrix, &m, "v1-L damaged");
    assert!(
        dets[0].decoded.is_none(),
        "v1-L with 3 codeword errors exceeds ISO r = 2 and must not decode, got {:?}",
        dets[0].decoded.as_ref().map(|d| d.bytes.clone())
    );

    // Sanity: the undamaged symbol does decode through the same path, so the
    // assertion above is about the corruption and not about the pipeline.
    let g_clean = render_gray(&clean, 8, 4);
    let clean_dets = qr_detect::detect(&g_clean.to_image());
    assert_eq!(clean_dets.len(), 1);
    assert_eq!(
        clean_dets[0]
            .decoded
            .as_ref()
            .expect("clean v1-L decodes")
            .bytes,
        data
    );
}

/// The micro counterpart of the cap: M1 corrects nothing, so a single damaged
/// module in the data region must survive detection but fail decoding.
#[test]
fn m1_single_module_damage_detects_but_does_not_decode() {
    let info = micro_symbols()
        .iter()
        .find(|i| i.version == MicroVersion::M1)
        .expect("M1 exists");
    let (_, clean) = fixture(info, Some(0));
    let mut m = clean.clone();
    // Bottom-right corner is always a data module in every micro version.
    let last = m.size() - 1;
    m.set(last, last, !m.get(last, last));

    let g = render_gray(&m, 8, MICRO_QUIET);
    let dets = detect_micro(&g.to_image());
    assert_eq!(dets.len(), 1, "the damaged M1 must still be detected");
    assert_same_matrix(&dets[0].matrix, &m, "M1 damaged");
    assert!(
        dets[0].decoded.is_none(),
        "M1 is detection-only; a flipped module must not decode, got {:?}",
        dets[0].decoded.as_ref().map(|d| d.bytes.clone())
    );
}

/// Numeric-only content on M1 through the image path, at the smallest symbol
/// there is (11 x 11 modules plus a 2-module quiet zone).
#[test]
fn m1_smallest_symbol_image_path() {
    let info = micro_symbols()
        .iter()
        .find(|i| i.version == MicroVersion::M1)
        .expect("M1 exists");
    for len in 1..=micro_max_chars(micro_mode(Content::Numeric), info) {
        let data = payload(Content::Numeric, len);
        let m = encode_micro(
            &data,
            MicroOptions {
                ec: info.ec,
                version: Some(MicroVersion::M1),
                mask: None,
            },
        )
        .expect("encode M1");
        let g = render_gray(&m, 8, MICRO_QUIET);
        assert_micro_decodes(&g, &m, &data, &format!("M1 numeric len {len}"));
    }
}
