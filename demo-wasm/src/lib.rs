//! Browser demo glue: `wasm-bindgen` entry points over the QR workspace.
//!
//! Three surfaces:
//!
//! * [`scan`] — stateless, one image in, all detections out. Used for the
//!   file-input path, where there is no stream and no state to carry.
//! * [`Scanner`] — stateful, for the camera. Wraps [`qr_video::VideoDetector`],
//!   which adds ROI tracking, a full-scan cadence and temporal de-duplication
//!   across frames, and adds a Micro QR pass on top (qr-video is QR-only).
//! * [`generate`] — text in, SVG document out, or a readable `ERROR: ...`.
//!
//! JSON is written by hand with [`format!`] rather than pulling in a
//! serialisation framework: the shapes here are fixed and tiny, and the demo's
//! whole point is that the stack stays dependency-light.

use std::collections::HashMap;

use qr_core::micro::MicroEcLevel;
use qr_core::{BitMatrix, EcLevel};
use qr_encode::{MicroOptions, encode_micro};
use qr_video::{Config, EventKind, Frame, VideoDetector};
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
        Some(b) => push_hex_string(out, b),
        None => out.push_str("null"),
    }
    out.push_str(",\"corners\":");
    push_corners(out, corners);
    out.push_str(",\"version\":");
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

/// Appends `bytes` as a quoted lowercase hex string.
fn push_hex_string(out: &mut String, bytes: &[u8]) {
    out.push('"');
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out.push('"');
}

/// Appends the four corners as `[[x,y],[x,y],[x,y],[x,y]]`, one decimal place —
/// the overlay is drawn in CSS pixels, so more precision is noise.
fn push_corners(out: &mut String, corners: &[qr_detect::Point; 4]) {
    out.push('[');
    for (i, p) in corners.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("[{:.1},{:.1}]", p.x, p.y));
    }
    out.push(']');
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
// Stateful camera scanning
// ---------------------------------------------------------------------------

/// Run the Micro QR pass on every Nth frame.
///
/// [`qr_video::VideoDetector`] covers QR only, so micro symbols need a separate
/// full-frame pass here — it cannot ride the ROI fast path. Every third frame
/// (5/s at the page's 15 fps cadence) keeps a micro symbol responsive while
/// leaving the ROI-tracked QR path the common case. The page holds overlay
/// boxes for a short time-to-live, so the lower rate does not make the box
/// flicker.
const MICRO_SCAN_INTERVAL: u64 = 3;

/// A stateful scanner over a stream of camera frames.
///
/// Holds one [`VideoDetector`] for QR — which contributes region-of-interest
/// tracking after a hit, a periodic full-frame refresh, and de-duplication of a
/// payload that stays in view — plus a small tracking table of its own for the
/// Micro QR pass, so both symbologies report the same `first`/`repeat`/`lost`
/// event vocabulary.
///
/// Create once per page, [`Scanner::reset`] when the camera restarts.
#[wasm_bindgen]
pub struct Scanner {
    qr: VideoDetector,
    /// Micro payload identity -> timestamp of its last sighting.
    micro_tracks: HashMap<u64, u64>,
    frame_index: u64,
    cfg: Config,
    /// Reused destination for the RGBA -> luma conversion, so a 30 fps stream
    /// does not allocate a megabyte per frame.
    luma: Vec<u8>,
}

impl Default for Scanner {
    fn default() -> Scanner {
        Scanner::new()
    }
}

#[wasm_bindgen]
impl Scanner {
    /// Creates a scanner with qr-video's default tuning, except that repeated
    /// sightings are emitted: the page needs fresh corners every frame to draw
    /// the overlay, and distinguishes "new payload" from "still in view" by the
    /// event name rather than by whether an event arrived at all.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> Scanner {
        let cfg = Config {
            emit_repeated: true,
            ..Config::default()
        };
        Scanner {
            qr: VideoDetector::new(cfg),
            micro_tracks: HashMap::new(),
            frame_index: 0,
            cfg,
            luma: Vec::new(),
        }
    }

    /// Drops all cross-frame state. Call when the camera stops, so a payload
    /// still tracked from the previous session is reported fresh next time.
    pub fn reset(&mut self) {
        self.qr = VideoDetector::new(self.cfg);
        self.micro_tracks.clear();
        self.frame_index = 0;
    }

    /// Processes one frame given as packed RGBA (canvas `ImageData` order) and
    /// returns its events as a JSON array.
    ///
    /// `rgba` must be `width * height * 4` bytes. The RGBA -> luma reduction
    /// happens here rather than in JavaScript even though it quadruples the
    /// bytes crossing the wasm boundary: measured on a 1440x810 frame (node
    /// 22, release build), the JS pixel loop costs 1.96 ms while the wider copy
    /// plus this loop costs 0.99 ms — the boundary copy is a `memcpy` and the
    /// pixel loop is not.
    ///
    /// `timestamp_millis` is `performance.now()`: only monotonicity matters.
    #[must_use]
    pub fn scan_frame_rgba(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
        timestamp_millis: f64,
    ) -> String {
        let mut luma = std::mem::take(&mut self.luma);
        rgba_to_luma(rgba, &mut luma);
        let out = self.process(&luma, width, height, timestamp_millis);
        self.luma = luma;
        out
    }

    /// `(x, y, width, height)` of the region the next frame will be scanned in,
    /// or `null` when the next scan covers the whole frame. Exposed so the page
    /// can say whether the fast path is engaged.
    #[must_use]
    pub fn roi(&self) -> String {
        match self.qr.active_roi() {
            Some((x, y, w, h)) => format!("[{x},{y},{w},{h}]"),
            None => "null".to_string(),
        }
    }
}

impl Scanner {
    /// Body of [`Scanner::scan_frame_rgba`], over the reduced luma plane.
    fn process(&mut self, luma: &[u8], width: u32, height: u32, timestamp_millis: f64) -> String {
        let now = clamp_timestamp(timestamp_millis);
        self.frame_index += 1;
        let well_formed = luma.len() as u64 == u64::from(width) * u64::from(height);

        let mut out = String::from("[");
        let mut first = true;

        for obs in self.qr.process_frame(Frame::new(luma, width, height, now)) {
            push_separator(&mut out, &mut first);
            push_event(
                &mut out,
                "qr",
                event_name(obs.kind),
                obs.key,
                obs.text.as_deref(),
                obs.bytes.as_deref(),
                obs.corners.as_ref(),
                // qr-video's `Observation` carries the payload but not the
                // symbol version, so the camera path cannot label it. The
                // file-input path, which goes through `scan`, still can.
                None,
                obs.bytes.is_none() && obs.kind != EventKind::Lost,
                obs.via_roi,
            );
        }

        // `VideoDetector` treats a malformed frame as empty; the micro pass is
        // called directly, so it gets the same guard here.
        if well_formed && self.frame_index.is_multiple_of(MICRO_SCAN_INTERVAL) {
            self.scan_micro(luma, width, height, now, &mut out, &mut first);
        }

        out.push(']');
        out
    }

    /// Full-frame Micro QR pass with its own de-duplication, mirroring the
    /// event vocabulary qr-video produces for QR.
    fn scan_micro(
        &mut self,
        luma: &[u8],
        width: u32,
        height: u32,
        now: u64,
        out: &mut String,
        first: &mut bool,
    ) {
        let dets = qr_detect::detect_micro_luma(luma, width, height);
        let mut seen = Vec::with_capacity(dets.len());

        for d in &dets {
            let (text, bytes) = match &d.decoded {
                Some(dec) => (dec.text.as_deref(), Some(dec.bytes.as_slice())),
                None => (None, None),
            };
            let key = match bytes {
                Some(b) => fnv1a(b),
                None => hash_matrix(&d.matrix),
            };
            seen.push(key);

            let previous = self.micro_tracks.insert(key, now);
            let fresh =
                previous.is_none_or(|last| now.saturating_sub(last) > self.cfg.debounce_millis);

            push_separator(out, first);
            push_event(
                out,
                "micro",
                if fresh { "first" } else { "repeat" },
                key,
                text,
                bytes,
                Some(&d.corners),
                micro_version_from_size(d.matrix.size()),
                bytes.is_none(),
                false,
            );
        }

        // Retire tracks that have gone quiet, so the table stays bounded and a
        // code shown again later is reported as a fresh sighting.
        let lost: Vec<u64> = self
            .micro_tracks
            .iter()
            .filter(|(key, last)| {
                !seen.contains(key) && now.saturating_sub(**last) >= self.cfg.lost_after_millis
            })
            .map(|(key, _)| *key)
            .collect();
        for key in lost {
            self.micro_tracks.remove(&key);
            push_separator(out, first);
            push_event(
                out, "micro", "lost", key, None, None, None, None, false, false,
            );
        }
    }
}

/// `performance.now()` as milliseconds since the stream started. Negative,
/// non-finite or absurd values collapse to something a monotonic counter can
/// still use rather than panicking on the cast.
fn clamp_timestamp(millis: f64) -> u64 {
    if millis.is_finite() && millis > 0.0 {
        millis as u64
    } else {
        0
    }
}

fn event_name(kind: EventKind) -> &'static str {
    match kind {
        EventKind::FirstSeen => "first",
        EventKind::Repeated => "repeat",
        EventKind::Lost => "lost",
    }
}

/// Writes one event object. `corners` is absent for `lost`; `unreadable` marks
/// a symbol that was located but did not decode.
#[allow(clippy::too_many_arguments)]
fn push_event(
    out: &mut String,
    kind: &str,
    event: &str,
    key: u64,
    text: Option<&str>,
    bytes: Option<&[u8]>,
    corners: Option<&[qr_detect::Point; 4]>,
    version: Option<u32>,
    unreadable: bool,
    via_roi: bool,
) {
    out.push_str("{\"kind\":\"");
    out.push_str(kind);
    out.push_str("\",\"event\":\"");
    out.push_str(event);
    // u64 as a string: JSON numbers lose the low bits of a 64-bit hash.
    out.push_str("\",\"key\":\"");
    out.push_str(&format!("{key:016x}"));
    out.push_str("\",\"text\":");
    match text {
        Some(t) => push_json_string(out, t),
        None => out.push_str("null"),
    }
    out.push_str(",\"bytes_hex\":");
    match bytes {
        Some(b) => push_hex_string(out, b),
        None => out.push_str("null"),
    }
    out.push_str(",\"corners\":");
    match corners {
        Some(c) => push_corners(out, c),
        None => out.push_str("null"),
    }
    out.push_str(",\"version\":");
    match version {
        Some(v) => out.push_str(&format!("{v}")),
        None => out.push_str("null"),
    }
    out.push_str(",\"unreadable\":");
    out.push_str(if unreadable { "true" } else { "false" });
    out.push_str(",\"via_roi\":");
    out.push_str(if via_roi { "true" } else { "false" });
    out.push('}');
}

/// Packed RGBA to 8-bit luma, integer approximation of
/// `0.299R + 0.587G + 0.114B`. Trailing bytes of a short buffer are ignored.
fn rgba_to_luma(rgba: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(rgba.len() / 4);
    for px in rgba.chunks_exact(4) {
        let y = 77 * u32::from(px[0]) + 150 * u32::from(px[1]) + 29 * u32::from(px[2]);
        out.push((y >> 8) as u8);
    }
}

/// FNV-1a 64-bit, matching the identity hash qr-video uses so both symbologies
/// key their tracking tables the same way. (The prime is FNV's 64-bit
/// 0x100_0000_01b3; keys are still qualified by symbology on the JS side, so
/// equal payloads in a QR and a Micro never share an overlay entry.)
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// FNV-1a over a module grid, size-prefixed. Used as the identity of a symbol
/// that was located but not decoded, so it is still tracked across frames.
fn hash_matrix(m: &BitMatrix) -> u64 {
    let size = m.size();
    let mut bits = Vec::with_capacity((size * size) as usize);
    for y in 0..size {
        for x in 0..size {
            bits.push(u8::from(m.get(x, y)));
        }
    }
    let mut h = fnv1a(&size.to_le_bytes());
    h ^= fnv1a(&bits);
    h
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
                Err(e) => {
                    // The generic encoder error is technically right but
                    // unhelpful for the two selections with narrow payload
                    // rules; explain the actual constraint instead.
                    let digits_only = data.iter().all(u8::is_ascii_digit);
                    if level == qr_core::micro::MicroEcLevel::Detection && !digits_only {
                        return "ERROR: D (detect only) means Micro M1, and M1 \
                                carries digits only - use 0-9, or pick another \
                                error correction level."
                            .to_string();
                    }
                    return format!("ERROR: {e}");
                }
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
