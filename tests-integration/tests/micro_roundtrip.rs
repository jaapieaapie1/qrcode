//! Suite 6 — Micro QR at the matrix level: `qr-encode::encode_micro` ->
//! `qr-decode::decode_micro`, byte-exact, across every one of the eight ISO
//! Table 13 symbols.
//!
//! What this suite covers that the per-crate micro suites cannot: the encoder
//! and the decoder meeting each other over the whole M1-M4 x EC x mode x mask
//! product, including
//!
//! * every payload length from 1 up to the published capacity, so the 4-bit
//!   final data codeword of M1 and M3 is exercised at every bit residue,
//! * corruption inside the published correction capacity `r` of ISO Table 9 —
//!   and one codeword past it,
//! * M1's error *detection* contract: it corrects nothing, so any single module
//!   flip in the data region must be rejected rather than silently repaired,
//! * a seeded fuzz loop over the same product.
//!
//! The module walk in [`common::micro_data_module_order`] is written
//! independently of the walk the encoder and decoder share, and
//! [`placement_walk_matches_core`] pins the two together.

mod common;

use common::{
    Content, Rng, corrupt_micro_codeword, micro_codeword_ranges, micro_content_kinds,
    micro_data_module_order, micro_label, micro_max_chars, micro_mode, payload,
};
use qr_core::micro::{MicroSymbolInfo, MicroVersion, micro_data_modules, micro_symbols};
use qr_decode::decode_micro;
use qr_encode::{MicroEncodeError, MicroOptions, encode_micro};

/// Options that pin version, EC selection and (optionally) mask, so nothing in
/// the sweep depends on the encoder's auto-selection.
fn opts(info: &MicroSymbolInfo, mask: Option<u8>) -> MicroOptions {
    MicroOptions {
        ec: info.ec,
        version: Some(info.version),
        mask,
    }
}

/// Encodes, decodes and asserts the payload and metadata survive untouched.
fn roundtrip(info: &MicroSymbolInfo, kind: Content, len: usize, mask: Option<u8>) {
    let data = payload(kind, len);
    let label = format!(
        "{} {} len {len}{}",
        micro_label(info),
        kind.label(),
        match mask {
            Some(m) => format!(" mask {m}"),
            None => String::new(),
        }
    );

    let matrix = encode_micro(&data, opts(info, mask))
        .unwrap_or_else(|e| panic!("{label}: encode_micro failed: {e}"));
    assert_eq!(
        matrix.size(),
        info.version.size(),
        "{label}: symbol dimension"
    );

    let decoded =
        decode_micro(&matrix).unwrap_or_else(|e| panic!("{label}: decode_micro failed: {e}"));
    assert_eq!(decoded.bytes, data, "{label}: payload bytes");
    assert_eq!(decoded.version, info.version, "{label}: version");
    assert_eq!(decoded.ec, info.ec, "{label}: ec selection");
    assert_eq!(
        decoded.errors_corrected, 0,
        "{label}: a clean symbol needs no RS repair"
    );
    if let Some(m) = mask {
        assert_eq!(decoded.mask, m, "{label}: mask");
    } else {
        assert!(decoded.mask <= 3, "{label}: mask id out of micro range");
    }
}

// ---------------------------------------------------------------------------
// Placement cross-check
// ---------------------------------------------------------------------------

/// The independently written micro zigzag must agree, position for position,
/// with the walk `qr-encode` and `qr-decode` share.
#[test]
fn placement_walk_matches_core() {
    for version in MicroVersion::ALL {
        let mine = micro_data_module_order(version);
        let theirs: Vec<(u32, u32)> = micro_data_modules(version).collect();
        assert_eq!(
            mine.len(),
            version.data_modules() as usize,
            "M{}: data module count",
            version.number()
        );
        assert_eq!(mine, theirs, "M{}: placement order", version.number());
    }
}

/// Every codeword range must tile the module stream exactly — no gap, no
/// overlap, no overrun.
#[test]
fn codeword_ranges_tile_the_module_stream() {
    for info in micro_symbols() {
        let ranges = micro_codeword_ranges(info);
        assert_eq!(
            ranges.len(),
            info.total_codewords as usize,
            "{}: codeword count",
            micro_label(info)
        );
        let mut at = 0usize;
        for &(start, len) in &ranges {
            assert_eq!(start, at, "{}: codeword gap", micro_label(info));
            at += len;
        }
        assert_eq!(
            at,
            info.version.data_modules() as usize,
            "{}: stream length",
            micro_label(info)
        );
    }
}

// ---------------------------------------------------------------------------
// Roundtrip sweeps
// ---------------------------------------------------------------------------

/// The full product: all eight symbols x every content kind the version allows
/// x {capacity-filling, short} payloads x all four forced masks plus auto.
#[test]
fn all_symbols_all_modes_all_masks() {
    for info in micro_symbols() {
        for &kind in micro_content_kinds(info.version) {
            let cap = micro_max_chars(micro_mode(kind), info);
            assert!(
                cap > 0,
                "{}: {} must have non-zero capacity",
                micro_label(info),
                kind.label()
            );
            let short = cap.clamp(1, 2);
            for len in [cap, short] {
                for mask in [None, Some(0), Some(1), Some(2), Some(3)] {
                    roundtrip(info, kind, len, mask);
                }
            }
        }
    }
}

/// Every payload length from 1 to capacity, for every symbol and every mode it
/// supports. This is what puts the M1/M3 half codeword through all four bit
/// residues of the numeric encoding and both residues of the alphanumeric one.
#[test]
fn every_length_up_to_capacity() {
    let mut cases = 0usize;
    for info in micro_symbols() {
        for &kind in micro_content_kinds(info.version) {
            let cap = micro_max_chars(micro_mode(kind), info);
            for len in 1..=cap {
                roundtrip(info, kind, len, None);
                cases += 1;
            }
        }
    }
    // The case count is the sum of every published ISO Table 7 capacity over
    // the modes each version allows (byte counted twice: binary and UTF-8).
    // Pinning it here means a silently shrunk capacity table cannot pass by
    // simply testing fewer lengths.
    assert_eq!(cases, 344, "capacity sweep case count");
}

/// The odd numeric lengths of M1 and M3 land a 1- or 2-digit group in the final
/// bits, which is exactly where the 4-bit half codeword sits. Called out
/// separately from [`every_length_up_to_capacity`] so a regression here names
/// itself.
#[test]
fn half_codeword_versions_odd_numeric_lengths() {
    for info in micro_symbols() {
        if !info.has_half_final_data_codeword() {
            continue;
        }
        assert!(
            matches!(info.version, MicroVersion::M1 | MicroVersion::M3),
            "{}: only M1 and M3 have a half final codeword",
            micro_label(info)
        );
        let cap = micro_max_chars(micro_mode(Content::Numeric), info);
        for len in (1..=cap).filter(|n| n % 2 == 1) {
            for mask in [None, Some(0), Some(1), Some(2), Some(3)] {
                roundtrip(info, Content::Numeric, len, mask);
            }
        }
    }
}

/// An empty payload is a legal micro symbol (mode indicator + zero character
/// count + terminator) and must come back as zero bytes.
#[test]
fn empty_payload_roundtrips() {
    for info in micro_symbols() {
        let label = micro_label(info);
        let m = encode_micro(&[], opts(info, None))
            .unwrap_or_else(|e| panic!("{label}: encode empty: {e}"));
        let d = decode_micro(&m).unwrap_or_else(|e| panic!("{label}: decode empty: {e}"));
        assert!(
            d.bytes.is_empty(),
            "{label}: empty payload came back non-empty"
        );
    }
}

/// One character past capacity must be refused, not silently truncated or
/// promoted to a larger version.
#[test]
fn one_past_capacity_is_refused() {
    for info in micro_symbols() {
        for &kind in micro_content_kinds(info.version) {
            let cap = micro_max_chars(micro_mode(kind), info);
            let data = payload(kind, cap + 1);
            let err = encode_micro(&data, opts(info, None)).expect_err(&format!(
                "{} {}: {} characters must not fit",
                micro_label(info),
                kind.label(),
                cap + 1
            ));
            assert_eq!(
                err,
                MicroEncodeError::DataTooLarge,
                "{} {}: wrong error for over-capacity payload",
                micro_label(info),
                kind.label()
            );
        }
    }
}

/// Modes outside a version's ISO Table 2 availability must be refused rather
/// than encoded in some wider mode.
#[test]
fn unavailable_modes_are_refused() {
    let m1 = micro_symbols()
        .iter()
        .find(|i| i.version == MicroVersion::M1)
        .expect("M1 exists");
    assert_eq!(
        encode_micro(b"AB", opts(m1, None)),
        Err(MicroEncodeError::UnsupportedMode),
        "M1 is numeric only"
    );
    assert_eq!(
        encode_micro(&[0x00, 0xFF], opts(m1, None)),
        Err(MicroEncodeError::UnsupportedMode),
        "M1 has no byte mode"
    );
    for info in micro_symbols()
        .iter()
        .filter(|i| i.version == MicroVersion::M2)
    {
        assert_eq!(
            encode_micro(&[0x00, 0xFF], opts(info, None)),
            Err(MicroEncodeError::UnsupportedMode),
            "{}: M2 has no byte mode",
            micro_label(info)
        );
    }
}

/// UTF-8 text that fits M4 must come back both as identical bytes and as the
/// same string.
#[test]
fn utf8_text_survives_matrix_roundtrip() {
    let info = micro_symbols()
        .iter()
        .find(|i| i.version == MicroVersion::M4 && i.data_codewords == 16)
        .expect("M4-L exists");
    let text = "hé 日本 ok"; // 13 bytes; M4-L holds 15 in byte mode
    assert!(text.len() <= micro_max_chars(micro_mode(Content::Utf8), info));
    let m = encode_micro(text.as_bytes(), opts(info, None)).expect("encode utf8");
    let d = decode_micro(&m).expect("decode utf8");
    assert_eq!(d.bytes, text.as_bytes());
    assert_eq!(d.text.as_deref(), Some(text));
}

// ---------------------------------------------------------------------------
// Corruption
// ---------------------------------------------------------------------------

/// A capacity-filling payload for the widest mode the version supports, plus
/// the encoded symbol.
fn corruption_fixture(info: &MicroSymbolInfo) -> (Vec<u8>, qr_core::BitMatrix) {
    let kind = *micro_content_kinds(info.version)
        .last()
        .expect("every version supports at least numeric");
    let len = micro_max_chars(micro_mode(kind), info);
    let data = payload(kind, len);
    let m = encode_micro(&data, opts(info, Some(0))).expect("encode corruption fixture");
    (data, m)
}

/// Full-width codeword ranges, i.e. everything except the 4-bit half codeword
/// of M1/M3, so each corruption is an unambiguous single symbol error.
fn full_codeword_ranges(info: &MicroSymbolInfo) -> Vec<(usize, usize)> {
    micro_codeword_ranges(info)
        .into_iter()
        .filter(|&(_, len)| len == 8)
        .collect()
}

/// Corrupting exactly `r` codewords — the published ISO Table 9 correction
/// capacity — must be repaired, with `errors_corrected` reporting the damage.
#[test]
fn corruption_within_capacity_is_corrected() {
    for info in micro_symbols() {
        let r = info.error_correction_capacity as usize;
        if r == 0 {
            continue; // M1 detects only; see `m1_rejects_any_single_module_flip`.
        }
        let label = micro_label(info);
        let (data, clean) = corruption_fixture(info);
        let order = micro_data_module_order(info.version);
        let ranges = full_codeword_ranges(info);

        // Damage the first r data codewords, then the last r codewords (which
        // fall in the EC region) — both must be equally correctable.
        for (what, picks) in [
            ("data region", ranges[..r].to_vec()),
            ("ec region", ranges[ranges.len() - r..].to_vec()),
        ] {
            let mut m = clean.clone();
            for &range in &picks {
                corrupt_micro_codeword(&mut m, &order, range);
            }
            let d = decode_micro(&m).unwrap_or_else(|e| {
                panic!("{label}: {r} errors in the {what} must be correctable: {e}")
            });
            assert_eq!(d.bytes, data, "{label}: payload after {what} repair");
            assert_eq!(
                d.errors_corrected, r,
                "{label}: expected {r} corrected codewords in the {what}"
            );
        }
    }
}

/// The 4-bit final data codeword of M1/M3 is still one Reed-Solomon symbol:
/// flipping its four modules costs one unit of correction capacity.
#[test]
fn half_codeword_corruption_costs_one_symbol() {
    for info in micro_symbols() {
        if !info.has_half_final_data_codeword() || info.error_correction_capacity == 0 {
            continue;
        }
        let label = micro_label(info);
        let (data, clean) = corruption_fixture(info);
        let order = micro_data_module_order(info.version);
        let half = *micro_codeword_ranges(info)
            .iter()
            .find(|&&(_, len)| len == 4)
            .expect("half codeword present");

        let mut m = clean.clone();
        corrupt_micro_codeword(&mut m, &order, half);
        let d = decode_micro(&m)
            .unwrap_or_else(|e| panic!("{label}: half-codeword damage must be correctable: {e}"));
        assert_eq!(d.bytes, data, "{label}: payload after half-codeword repair");
        assert_eq!(
            d.errors_corrected, 1,
            "{label}: a damaged half codeword is one symbol error"
        );
    }
}

/// One codeword past `r` must fail cleanly. It must never return a payload —
/// that is a misdecode, which ISO 7.5.1's `p` reserved codewords exist to
/// prevent.
///
/// For the `p = 0` symbols ISO does not formally promise rejection at `r + 1`,
/// but the hard `Err` assertion is deterministic for these fixed fixtures and
/// held across 4200 randomized trials in review (0 misdecodes) — bounded-
/// distance RS decoding cannot reach the true codeword at `r + 1` errors.
#[test]
fn corruption_beyond_capacity_fails_cleanly() {
    for info in micro_symbols() {
        let r = info.error_correction_capacity as usize;
        if r == 0 {
            continue;
        }
        let label = micro_label(info);
        let (data, clean) = corruption_fixture(info);
        let order = micro_data_module_order(info.version);
        let ranges = full_codeword_ranges(info);
        assert!(ranges.len() > r, "{label}: need more than r full codewords");

        let mut m = clean.clone();
        for &range in &ranges[..r + 1] {
            corrupt_micro_codeword(&mut m, &order, range);
        }
        match decode_micro(&m) {
            Ok(d) => panic!(
                "{label}: {} errors exceed r = {r} but decoded to {:?} (payload was {:?})",
                r + 1,
                d.bytes,
                data
            ),
            Err(e) => assert!(
                matches!(
                    e,
                    qr_decode::DecodeError::BlockCorrection | qr_decode::DecodeError::Malformed
                ),
                "{label}: unexpected error {e:?}"
            ),
        }
    }
}

/// M1's two error correction codewords detect but never correct (ISO Table 9
/// gives it `r = 0`). Every single module flip anywhere in the data region must
/// therefore be rejected — silently "repairing" one would be a misdecode.
#[test]
fn m1_rejects_any_single_module_flip() {
    let info = micro_symbols()
        .iter()
        .find(|i| i.version == MicroVersion::M1)
        .expect("M1 exists");
    let (data, clean) = corruption_fixture(info);
    let order = micro_data_module_order(MicroVersion::M1);
    assert_eq!(order.len(), 36, "M1 has 36 data modules");

    for (i, &(x, y)) in order.iter().enumerate() {
        let mut m = clean.clone();
        m.set(x, y, !m.get(x, y));
        match decode_micro(&m) {
            Ok(d) => panic!(
                "M1 module {i} at ({x}, {y}) flipped but decode returned {:?} (payload was {:?}); \
                 M1 is detection-only",
                d.bytes, data
            ),
            Err(e) => assert!(
                matches!(
                    e,
                    qr_decode::DecodeError::BlockCorrection | qr_decode::DecodeError::Malformed
                ),
                "M1 module {i}: unexpected error {e:?}"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Seeded fuzz
// ---------------------------------------------------------------------------

/// 400 pseudo-random (symbol, mode, length, mask) draws through the matrix
/// roundtrip. Deterministic: the seed is fixed, so a failure reproduces.
#[test]
fn seeded_fuzz_matrix_roundtrip() {
    let mut rng = Rng::new(0x4D49_4352_4F51_5200); // "MICROQR\0"
    let symbols = micro_symbols();
    let mut cases = 0usize;

    for _ in 0..400 {
        let info = &symbols[rng.below(symbols.len())];
        let kinds = micro_content_kinds(info.version);
        let kind = kinds[rng.below(kinds.len())];
        let cap = micro_max_chars(micro_mode(kind), info);
        let len = rng.below(cap + 1);
        let mask = match rng.below(5) {
            4 => None,
            m => Some(m as u8),
        };

        let data = payload(kind, len);
        let label = format!(
            "fuzz {} {} len {len} mask {mask:?}",
            micro_label(info),
            kind.label()
        );
        let m = encode_micro(&data, opts(info, mask))
            .unwrap_or_else(|e| panic!("{label}: encode failed: {e}"));
        let d = decode_micro(&m).unwrap_or_else(|e| panic!("{label}: decode failed: {e}"));
        assert_eq!(d.bytes, data, "{label}: payload");
        assert_eq!(d.version, info.version, "{label}: version");
        assert_eq!(d.ec, info.ec, "{label}: ec");
        cases += 1;
    }

    assert_eq!(cases, 400);
}

/// The same loop, but the encoder picks the version: the smallest symbol that
/// offers the requested EC selection and fits. The decoder must agree with
/// whatever it picked.
#[test]
fn seeded_fuzz_auto_version() {
    let mut rng = Rng::new(0xA0_7052_E1EC_7ED5);
    let mut encoded = 0usize;

    for _ in 0..300 {
        let ec = [
            qr_core::micro::MicroEcLevel::Detection,
            qr_core::micro::MicroEcLevel::L,
            qr_core::micro::MicroEcLevel::M,
            qr_core::micro::MicroEcLevel::Q,
        ][rng.below(4)];
        let kind = common::ALL_CONTENT[rng.below(4)];
        let len = rng.below(36);
        let data = payload(kind, len);

        match encode_micro(&data, MicroOptions::new(ec)) {
            Ok(m) => {
                let label = format!("auto {ec:?} {} len {len}", kind.label());
                let d = decode_micro(&m).unwrap_or_else(|e| panic!("{label}: decode failed: {e}"));
                assert_eq!(d.bytes, data, "{label}: payload");
                assert_eq!(d.ec, ec, "{label}: ec selection");
                assert_eq!(
                    m.size(),
                    d.version.size(),
                    "{label}: decoded version disagrees with matrix size"
                );
                encoded += 1;
            }
            Err(e) => assert!(
                matches!(
                    e,
                    MicroEncodeError::DataTooLarge | MicroEncodeError::UnsupportedMode
                ),
                "auto {ec:?} {} len {len}: unexpected error {e:?}",
                kind.label()
            ),
        }
    }

    assert!(
        encoded > 50,
        "auto-version fuzz encoded only {encoded} of 300 draws; the sweep is not exercising much"
    );
}
