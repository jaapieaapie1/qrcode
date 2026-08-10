//! Suite 1 — the whole stack: `qr-encode` -> `qr-render` -> `image::GrayImage`
//! -> `qr-detect` -> `qr-decode`, asserting byte-exact payload recovery.
//!
//! Coverage is a matrix sweep over versions x EC levels x content kinds. The
//! small versions get the full cross product; the large ones are sampled, since
//! each case rasterises and re-detects a several-hundred-pixel image in an
//! unoptimised build. All eight data masks are exercised through the image path
//! at version 3-M.
//!
//! Every payload fills its symbol to capacity, which maximises module density
//! and is the hardest case for the sampler.

mod common;

use common::{ALL_CONTENT, ALL_EC, Content, ec_label, max_chars, payload, render_gray, ver};
use qr_core::{BitMatrix, EcLevel};
use qr_encode::{Options, encode};

/// Asserts two matrices agree module-for-module.
fn assert_same_matrix(got: &BitMatrix, want: &BitMatrix, label: &str) {
    assert_eq!(got.size(), want.size(), "{label}: sampled dimension");
    let mut mismatches = 0usize;
    for y in 0..want.size() {
        for x in 0..want.size() {
            if got.get(x, y) != want.get(x, y) {
                mismatches += 1;
            }
        }
    }
    assert_eq!(
        mismatches, 0,
        "{label}: {mismatches} sampled module mismatches"
    );
}

/// Encodes a capacity-filling payload, renders it, detects it and decodes it,
/// asserting the payload and the symbol metadata survive the whole trip.
fn roundtrip(v: u8, ec: EcLevel, kind: Content, scale: u32, mask: Option<u8>) {
    let version = ver(v);
    let len = max_chars(kind.mode(), version, ec);
    if len == 0 {
        return;
    }
    let data = payload(kind, len);
    let label = format!(
        "v{v}-{} {} (len {len}, scale {scale}{})",
        ec_label(ec),
        kind.label(),
        match mask {
            Some(m) => format!(", mask {m}"),
            None => String::new(),
        }
    );

    let mut opts = Options::new(ec);
    opts.version = Some(version);
    opts.mask = mask;
    let matrix = encode(&data, opts).unwrap_or_else(|e| panic!("{label}: encode failed: {e}"));

    let gray = render_gray(&matrix, scale, 4);
    let detection = qr_detect::read_one(&gray.to_image())
        .unwrap_or_else(|| panic!("{label}: detector found no symbol"));

    assert_same_matrix(&detection.matrix, &matrix, &label);

    let decoded = detection
        .decoded
        .unwrap_or_else(|| panic!("{label}: detected but did not decode"));
    assert_eq!(decoded.bytes, data, "{label}: payload mismatch");
    assert_eq!(decoded.version.number(), v, "{label}: version");
    assert_eq!(decoded.ec, ec, "{label}: ec level");
    if let Some(m) = mask {
        assert_eq!(decoded.mask, m, "{label}: mask");
    }
    assert_eq!(
        decoded.errors_corrected, 0,
        "{label}: clean render needs no RS repair"
    );
}

/// Full EC x content cross product for one version.
fn cross_product(v: u8, scale: u32) {
    for ec in ALL_EC {
        for kind in ALL_CONTENT {
            roundtrip(v, ec, kind, scale, None);
        }
    }
}

// --- small versions: full cross product, one test per version --------------

#[test]
fn cross_product_v1() {
    cross_product(1, 8);
}

#[test]
fn cross_product_v2() {
    cross_product(2, 8);
}

#[test]
fn cross_product_v5() {
    cross_product(5, 8);
}

#[test]
fn cross_product_v7() {
    cross_product(7, 6);
}

// --- large versions: sampled ------------------------------------------------

#[test]
fn sampled_v10() {
    roundtrip(10, EcLevel::L, Content::Numeric, 6, None);
    roundtrip(10, EcLevel::M, Content::Binary, 6, None);
    roundtrip(10, EcLevel::Q, Content::Utf8, 6, None);
    roundtrip(10, EcLevel::H, Content::Alphanumeric, 6, None);
}

#[test]
fn sampled_v20() {
    roundtrip(20, EcLevel::L, Content::Binary, 5, None);
    roundtrip(20, EcLevel::Q, Content::Alphanumeric, 5, None);
}

#[test]
fn sampled_v40() {
    roundtrip(40, EcLevel::L, Content::Numeric, 4, None);
    roundtrip(40, EcLevel::H, Content::Binary, 4, None);
}

// --- masks ------------------------------------------------------------------

/// All eight data masks driven through the full image pipeline at v3-M.
#[test]
fn all_masks_v3_m() {
    for mask in 0..8u8 {
        roundtrip(3, EcLevel::M, Content::Binary, 8, Some(mask));
    }
}

/// The byte-mode payloads deliberately carry `0x00` and `0xFF`; make sure they
/// really do survive the image path rather than being trimmed somewhere.
#[test]
fn binary_extremes_survive_image_path() {
    let data: Vec<u8> = vec![0x00, 0xFF, 0x00, 0x00, 0xFF, 0xFF, 0x7F, 0x80, 0x01, 0xFE];
    let m = encode(&data, Options::new(EcLevel::Q)).expect("encode binary extremes");
    let gray = render_gray(&m, 8, 4);
    let det = qr_detect::read_one(&gray.to_image()).expect("detect binary extremes");
    let d = det.decoded.expect("decode binary extremes");
    assert_eq!(d.bytes, data);
    assert_eq!(d.text, None, "0x00/0xFF payload is not valid UTF-8 text");
}

/// A multibyte UTF-8 payload must come back both as identical bytes and as the
/// same string.
#[test]
fn utf8_text_survives_image_path() {
    let text = "héllo wörld 日本語 🎉 ok";
    let m = encode(text.as_bytes(), Options::new(EcLevel::M)).expect("encode utf8");
    let gray = render_gray(&m, 8, 4);
    let det = qr_detect::read_one(&gray.to_image()).expect("detect utf8");
    let d = det.decoded.expect("decode utf8");
    assert_eq!(d.bytes, text.as_bytes());
    assert_eq!(d.text.as_deref(), Some(text));
}

/// Two different codes side by side in one image must both be found and
/// decoded to their own payloads.
#[test]
fn two_codes_in_one_image() {
    let a = encode(b"FIRST-CODE-123", Options::new(EcLevel::M)).expect("encode a");
    let b = encode(b"SECOND-CODE-456", Options::new(EcLevel::M)).expect("encode b");
    let ga = render_gray(&a, 8, 4);
    let gb = render_gray(&b, 8, 4);

    let gap = 48u32;
    let w = ga.w + gap + gb.w;
    let h = ga.h.max(gb.h);
    let mut canvas = common::Gray::new(w, h, 255);
    for y in 0..ga.h {
        for x in 0..ga.w {
            canvas.set(x, y, ga.get(x, y));
        }
    }
    for y in 0..gb.h {
        for x in 0..gb.w {
            canvas.set(ga.w + gap + x, y, gb.get(x, y));
        }
    }

    let dets = qr_detect::detect(&canvas.to_image());
    assert_eq!(dets.len(), 2, "expected two detections");
    let mut payloads: Vec<Vec<u8>> = dets
        .into_iter()
        .map(|d| d.decoded.expect("both codes must decode").bytes)
        .collect();
    payloads.sort();
    let mut want = vec![b"FIRST-CODE-123".to_vec(), b"SECOND-CODE-456".to_vec()];
    want.sort();
    assert_eq!(payloads, want);
}
