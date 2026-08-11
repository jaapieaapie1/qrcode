//! Browser demo glue: two `wasm-bindgen` entry points over the QR workspace.
//!
//! [`scan`] takes a raw luma plane (the browser computes it from a canvas
//! `ImageData`) and returns detection results as a JSON string. [`generate`]
//! takes text and returns an SVG document, or a readable `ERROR: ...` message.
//!
//! JSON is written by hand with [`format!`] rather than pulling in a
//! serialisation framework: the two shapes here are fixed and tiny, and the
//! demo's whole point is that the stack stays dependency-light.

use qr_core::micro::MicroEcLevel;
use qr_core::{BitMatrix, EcLevel};
use qr_encode::{MicroOptions, encode_micro};
use wasm_bindgen::prelude::wasm_bindgen;

/// Module scale used for both symbologies. SVG is resolution-independent, so
/// this only sets the coordinate system the page then scales with CSS.
const SVG_SCALE: u32 = 8;
/// Quiet zone for QR, in modules (ISO recommends 4).
const QR_QUIET_ZONE: u32 = 4;
/// Quiet zone for Micro QR, in modules (ISO recommends 2).
const MICRO_QUIET_ZONE: u32 = 2;

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

/// Detects every QR and Micro QR symbol in a luma plane.
///
/// `luma` must be `width * height` bytes, row-major, one byte per pixel. The
/// return value is a JSON array; each element is
///
/// ```json
/// {
///   "kind": "qr" | "micro",
///   "text": string | null,
///   "bytes_hex": string | null,
///   "corners": [[x, y], [x, y], [x, y], [x, y]],
///   "version": number | null,
///   "error": string | null
/// }
/// ```
///
/// Symbols that were located but could not be decoded are included, with
/// `error` set and `text`/`bytes_hex` null, so the overlay can show "found but
/// unreadable" rather than silently dropping them. `corners` are in image
/// pixel coordinates, ordered top-left, top-right, bottom-right, bottom-left.
///
/// Never fails: a dimension mismatch or an empty frame yields `[]`.
#[wasm_bindgen]
#[must_use]
pub fn scan(luma: &[u8], width: u32, height: u32) -> String {
    let (qr, micro) = qr_detect::detect_all_luma(luma, width, height);

    let mut out = String::from("[");
    let mut first = true;

    for d in &qr {
        push_separator(&mut out, &mut first);
        let (text, bytes, version, error) = match &d.decoded {
            Some(dec) => (
                dec.text.as_deref(),
                Some(dec.bytes.as_slice()),
                Some(u32::from(dec.version.number())),
                None,
            ),
            None => (None, None, qr_version_from_size(d.matrix.size()), Some(())),
        };
        push_detection(
            &mut out, "qr", text, bytes, &d.corners, version, error, &d.matrix,
        );
    }

    for d in &micro {
        push_separator(&mut out, &mut first);
        let (text, bytes, version, error) = match &d.decoded {
            Some(dec) => (
                dec.text.as_deref(),
                Some(dec.bytes.as_slice()),
                Some(u32::from(dec.version.number())),
                None,
            ),
            None => (
                None,
                None,
                micro_version_from_size(d.matrix.size()),
                Some(()),
            ),
        };
        push_detection(
            &mut out, "micro", text, bytes, &d.corners, version, error, &d.matrix,
        );
    }

    out.push(']');
    out
}

fn push_separator(out: &mut String, first: &mut bool) {
    if *first {
        *first = false;
    } else {
        out.push(',');
    }
}

/// Writes one detection object. `error` is `Some(())` when decoding failed;
/// the message itself is built here so both symbologies word it identically.
#[allow(clippy::too_many_arguments)]
fn push_detection(
    out: &mut String,
    kind: &str,
    text: Option<&str>,
    bytes: Option<&[u8]>,
    corners: &[qr_detect::Point; 4],
    version: Option<u32>,
    error: Option<()>,
    matrix: &BitMatrix,
) {
    out.push_str("{\"kind\":\"");
    out.push_str(kind);
    out.push_str("\",\"text\":");
    match text {
        Some(t) => push_json_string(out, t),
        None => out.push_str("null"),
    }
    out.push_str(",\"bytes_hex\":");
    match bytes {
        Some(b) => {
            out.push('"');
            for byte in b {
                out.push_str(&format!("{byte:02x}"));
            }
            out.push('"');
        }
        None => out.push_str("null"),
    }
    out.push_str(",\"corners\":[");
    for (i, p) in corners.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("[{:.1},{:.1}]", p.x, p.y));
    }
    out.push_str("],\"version\":");
    match version {
        Some(v) => out.push_str(&format!("{v}")),
        None => out.push_str("null"),
    }
    out.push_str(",\"error\":");
    match error {
        Some(()) => push_json_string(
            out,
            &format!(
                "located but not decodable ({}x{} modules sampled)",
                matrix.size(),
                matrix.size()
            ),
        ),
        None => out.push_str("null"),
    }
    out.push('}');
}

/// QR version from a sampled grid side length: `size = 4 * version + 17`.
/// Returns `None` when the grid is not a legal QR size, which a failed
/// detection can produce.
fn qr_version_from_size(size: u32) -> Option<u32> {
    if !(21..=177).contains(&size) || !(size - 17).is_multiple_of(4) {
        return None;
    }
    Some((size - 17) / 4)
}

/// Micro version from a sampled grid side length: `size = 2 * version + 9`.
fn micro_version_from_size(size: u32) -> Option<u32> {
    if !(11..=17).contains(&size) || size.is_multiple_of(2) {
        return None;
    }
    Some((size - 9) / 2)
}

/// Appends `s` as a quoted, escaped JSON string.
fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

/// Encodes `text` and returns a standalone SVG document.
///
/// `ec` is `"L"`, `"M"`, `"Q"` or `"H"` for QR; Micro QR additionally accepts
/// `"D"` (the M1-only error *detection* selection) and rejects `"H"`, which no
/// micro version offers. When `micro` is true the smallest micro version that
/// fits the payload at that selection is used; otherwise the smallest QR
/// version. Error correction is never silently downgraded — an impossible
/// combination is reported rather than quietly re-encoded.
///
/// Any failure returns a human-readable message prefixed with `ERROR:` instead
/// of an SVG, so the page can display it verbatim.
#[wasm_bindgen]
#[must_use]
pub fn generate(text: &str, ec: &str, micro: bool) -> String {
    let data = text.as_bytes();
    if data.is_empty() {
        return "ERROR: nothing to encode — type some text first.".to_string();
    }

    let matrix = if micro {
        match micro_ec(ec) {
            Ok(level) => match encode_micro(data, MicroOptions::new(level)) {
                Ok(m) => m,
                Err(e) => return format!("ERROR: {e}"),
            },
            Err(msg) => return msg,
        }
    } else {
        match qr_ec(ec) {
            Ok(level) => match qr_encode::encode(data, qr_encode::Options::new(level)) {
                Ok(m) => m,
                Err(e) => return format!("ERROR: {e}"),
            },
            Err(msg) => return msg,
        }
    };

    let quiet = if micro {
        MICRO_QUIET_ZONE
    } else {
        QR_QUIET_ZONE
    };
    let opts = qr_render::Options::default()
        .scale(SVG_SCALE)
        .quiet_zone(quiet);
    qr_render::render_svg(&matrix, &opts)
}

/// Parses a QR error-correction level. The `Err` case is the finished message.
fn qr_ec(ec: &str) -> Result<EcLevel, String> {
    match ec {
        "L" | "l" => Ok(EcLevel::L),
        "M" | "m" => Ok(EcLevel::M),
        "Q" | "q" => Ok(EcLevel::Q),
        "H" | "h" => Ok(EcLevel::H),
        "D" | "d" => Err(
            "ERROR: error-detection-only is a Micro QR (M1) selection; QR uses L, M, Q or H."
                .to_string(),
        ),
        other => Err(format!(
            "ERROR: unknown error correction level {other:?} — use L, M, Q or H."
        )),
    }
}

/// Parses a Micro QR error-correction selection. The `Err` case is the
/// finished message.
fn micro_ec(ec: &str) -> Result<MicroEcLevel, String> {
    match ec {
        "D" | "d" => Ok(MicroEcLevel::Detection),
        "L" | "l" => Ok(MicroEcLevel::L),
        "M" | "m" => Ok(MicroEcLevel::M),
        "Q" | "q" => Ok(MicroEcLevel::Q),
        "H" | "h" => Err(
            "ERROR: Micro QR has no level H — M4 tops out at Q. Pick Q, or uncheck Micro."
                .to_string(),
        ),
        other => Err(format!(
            "ERROR: unknown error correction level {other:?} — Micro QR uses D, L, M or Q."
        )),
    }
}
