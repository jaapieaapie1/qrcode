//! Stateful across-frame detection: ROI tracking, temporal de-duplication, and
//! a full-scan cadence, layered on the stateless [`qr_detect`] pipeline.

use std::collections::HashMap;

use qr_core::BitMatrix;
use qr_detect::{Detection, Point, detect_luma};

use crate::{EventKind, Frame, FrameSource};

/// Tuning knobs for [`VideoDetector`]. [`Config::default`] is a sensible
/// starting point; construct with struct-update syntax to override individual
/// fields.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Force a full-frame scan every `full_scan_interval` frames even while an
    /// ROI is active, so codes that appear outside the tracked box are still
    /// found. `0` disables periodic refresh (ROI stays until it misses).
    pub full_scan_interval: u64,
    /// ROI padding as a fraction of the tracked box's larger side. `0.5` pads
    /// by half the box size on every edge, which tolerates codes drifting
    /// between frames.
    pub roi_padding_frac: f64,
    /// Minimum ROI padding in pixels, applied on top of `roi_padding_frac` so a
    /// small box still keeps enough quiet zone to detect.
    pub roi_padding_min_px: u32,
    /// Consecutive ROI misses tolerated before the ROI is dropped and scanning
    /// reverts to full-frame. Each miss already triggers a same-frame
    /// full-frame fallback; this bounds how long a stale ROI is retried.
    pub roi_miss_limit: u32,
    /// A payload seen again within this many milliseconds of its last sighting
    /// is [`EventKind::Repeated`] (suppressed by default) rather than a fresh
    /// [`EventKind::FirstSeen`].
    pub debounce_millis: u64,
    /// A tracked payload not seen for at least this many milliseconds is
    /// reported [`EventKind::Lost`] and dropped, so a later re-appearance is a
    /// fresh `FirstSeen`.
    pub lost_after_millis: u64,
    /// When `true`, [`EventKind::Repeated`] observations are included in the
    /// output. Default `false` (repeats are only counted in [`Stats`]).
    pub emit_repeated: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            full_scan_interval: 30,
            roi_padding_frac: 0.5,
            roi_padding_min_px: 16,
            roi_miss_limit: 3,
            debounce_millis: 750,
            lost_after_millis: 500,
            emit_repeated: false,
        }
    }
}

/// Running counters, exposed for observability and as test hooks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Frames handed to [`VideoDetector::process_frame`].
    pub frames: u64,
    /// Frames whose primary scan covered the whole image.
    pub full_scans: u64,
    /// Frames whose primary scan was restricted to the ROI.
    pub roi_scans: u64,
    /// ROI scans that found at least one symbol.
    pub roi_hits: u64,
    /// ROI scans that found nothing (each triggers a full-frame fallback).
    pub roi_misses: u64,
    /// Full-frame scans that ran as a fallback after an ROI miss.
    pub roi_fallbacks: u64,
    /// Total symbols detected across all frames (before de-duplication).
    pub detections: u64,
    /// `FirstSeen` observations emitted.
    pub first_seen: u64,
    /// `Repeated` observations counted (whether or not emitted).
    pub repeated: u64,
    /// `Lost` observations emitted.
    pub lost: u64,
}

/// A de-duplicated detection event for one frame.
///
/// Corners are in full-frame image space, `(x, y)` = (column, row), even when
/// the symbol was found via an ROI scan (they are mapped back).
#[derive(Clone, Debug)]
pub struct Observation {
    /// Which kind of event this is.
    pub kind: EventKind,
    /// Stable identity key for the payload: a hash of the decoded bytes when
    /// decoding succeeded, otherwise a hash of the sampled module grid. Equal
    /// keys denote the same tracked code.
    pub key: u64,
    /// Decoded payload bytes, when `qr-decode` succeeded for this code.
    pub bytes: Option<Vec<u8>>,
    /// Decoded payload as UTF-8 text, when available.
    pub text: Option<String>,
    /// Symbol corners (top-left, top-right, bottom-right, bottom-left). `None`
    /// for [`EventKind::Lost`].
    pub corners: Option<[Point; 4]>,
    /// Timestamp of the frame this event belongs to.
    pub timestamp_millis: u64,
    /// `true` when the underlying detection came from an ROI scan rather than a
    /// full-frame scan. Meaningless for [`EventKind::Lost`] (always `false`).
    pub via_roi: bool,
}

/// Axis-aligned region of interest in full-frame pixel coordinates.
#[derive(Clone, Copy, Debug)]
struct Roi {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// Per-payload tracking state.
struct Track {
    last_seen: u64,
    bytes: Option<Vec<u8>>,
    text: Option<String>,
}

/// Stateful multi-frame QR detector.
///
/// Feed frames with [`process_frame`](VideoDetector::process_frame), or drive a
/// [`FrameSource`] end-to-end with [`run`](VideoDetector::run). State carried
/// across frames: the active ROI, the miss counter, the frame index (for the
/// full-scan cadence), and the per-payload tracking table.
pub struct VideoDetector {
    cfg: Config,
    frame_index: u64,
    active_roi: Option<Roi>,
    consecutive_misses: u32,
    tracks: HashMap<u64, Track>,
    stats: Stats,
}

impl VideoDetector {
    /// Creates a detector with the given configuration.
    pub fn new(cfg: Config) -> VideoDetector {
        VideoDetector {
            cfg,
            frame_index: 0,
            active_roi: None,
            consecutive_misses: 0,
            tracks: HashMap::new(),
            stats: Stats::default(),
        }
    }

    /// Creates a detector with [`Config::default`].
    pub fn with_defaults() -> VideoDetector {
        VideoDetector::new(Config::default())
    }

    /// Returns the running [`Stats`] snapshot.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Returns the active ROI in `(x, y, w, h)` full-frame pixels, if any. Used
    /// mainly by tests to confirm ROI tracking is engaged.
    pub fn active_roi(&self) -> Option<(u32, u32, u32, u32)> {
        self.active_roi.map(|r| (r.x, r.y, r.w, r.h))
    }

    /// Processes one frame and returns the de-duplicated events for it.
    ///
    /// A malformed frame (luma length not matching `width * height`, or a
    /// zero-sized frame) is treated as empty: it advances timers and may emit
    /// [`EventKind::Lost`] events, but detects nothing.
    pub fn process_frame(&mut self, frame: Frame<'_>) -> Vec<Observation> {
        self.stats.frames += 1;
        self.frame_index += 1;
        let now = frame.timestamp_millis;

        let valid = frame.is_well_formed() && frame.width > 0 && frame.height > 0;
        let (detections, via_roi) = if valid {
            self.scan(&frame)
        } else {
            (Vec::new(), false)
        };

        self.stats.detections += detections.len() as u64;

        let mut out = Vec::new();
        let seen_keys = self.record_detections(&detections, via_roi, now, &mut out);
        self.update_roi(&detections, frame.width, frame.height);
        self.emit_lost(&seen_keys, now, &mut out);
        out
    }

    /// Drives a [`FrameSource`] to exhaustion, invoking `sink` for every
    /// observation in order. Convenience over a manual `process_frame` loop;
    /// handles the lending borrow of the source internally.
    pub fn run<S, F>(&mut self, source: &mut S, mut sink: F)
    where
        S: FrameSource,
        F: FnMut(Observation),
    {
        while let Some(frame) = source.next_frame() {
            for obs in self.process_frame(frame) {
                sink(obs);
            }
        }
    }

    // --- scanning ---------------------------------------------------------

    /// Runs the primary scan for a frame, choosing ROI vs full-frame per the
    /// cadence, with an automatic full-frame fallback on an ROI miss. Returns
    /// detections in full-frame coordinates plus whether they came from the ROI
    /// path (`true`) or a full-frame scan (`false`).
    fn scan(&mut self, frame: &Frame<'_>) -> (Vec<Detection>, bool) {
        let refresh = self.cfg.full_scan_interval != 0
            && self.frame_index.is_multiple_of(self.cfg.full_scan_interval);
        let use_roi = !refresh
            && self.consecutive_misses < self.cfg.roi_miss_limit
            && self.active_roi.is_some();

        if use_roi {
            let roi = self.active_roi.expect("checked is_some above");
            self.stats.roi_scans += 1;
            let dets = detect_in_roi(frame, roi);
            if dets.is_empty() {
                self.stats.roi_misses += 1;
                self.consecutive_misses += 1;
                // Same-frame fallback: don't drop the frame just because the
                // ROI moved off the code.
                self.stats.full_scans += 1;
                self.stats.roi_fallbacks += 1;
                (detect_full(frame), false)
            } else {
                self.stats.roi_hits += 1;
                self.consecutive_misses = 0;
                (dets, true)
            }
        } else {
            self.stats.full_scans += 1;
            self.consecutive_misses = 0;
            (detect_full(frame), false)
        }
    }

    // --- de-duplication ---------------------------------------------------

    /// Turns raw detections into `FirstSeen`/`Repeated` observations and
    /// updates the tracking table. `via_roi` marks whether the batch came from
    /// an ROI scan. Returns the identity keys seen this frame, in order.
    fn record_detections(
        &mut self,
        dets: &[Detection],
        via_roi: bool,
        now: u64,
        out: &mut Vec<Observation>,
    ) -> Vec<u64> {
        let mut seen_keys = Vec::with_capacity(dets.len());
        for det in dets {
            let (bytes, text) = payload_of(det);
            let key = identity_key(det, bytes.as_deref());
            seen_keys.push(key);

            let is_new = match self.tracks.get(&key) {
                None => true,
                Some(t) => now.saturating_sub(t.last_seen) > self.cfg.debounce_millis,
            };

            self.tracks.insert(
                key,
                Track {
                    last_seen: now,
                    bytes: bytes.clone(),
                    text: text.clone(),
                },
            );

            if is_new {
                self.stats.first_seen += 1;
                out.push(Observation {
                    kind: EventKind::FirstSeen,
                    key,
                    bytes,
                    text,
                    corners: Some(det.corners),
                    timestamp_millis: now,
                    via_roi,
                });
            } else {
                self.stats.repeated += 1;
                if self.cfg.emit_repeated {
                    out.push(Observation {
                        kind: EventKind::Repeated,
                        key,
                        bytes,
                        text,
                        corners: Some(det.corners),
                        timestamp_millis: now,
                        via_roi,
                    });
                }
            }
        }
        seen_keys
    }

    /// Emits `Lost` for tracked codes absent from this frame past the timeout,
    /// and drops them so a re-appearance is a fresh `FirstSeen`.
    /// `seen` holds the identity keys observed this frame.
    fn emit_lost(&mut self, seen: &[u64], now: u64, out: &mut Vec<Observation>) {
        let mut lost_keys = Vec::new();
        for (key, track) in self.tracks.iter() {
            if seen.contains(key) {
                continue;
            }
            if now.saturating_sub(track.last_seen) >= self.cfg.lost_after_millis {
                self.stats.lost += 1;
                out.push(Observation {
                    kind: EventKind::Lost,
                    key: *key,
                    bytes: track.bytes.clone(),
                    text: track.text.clone(),
                    corners: None,
                    timestamp_millis: now,
                    via_roi: false,
                });
                lost_keys.push(*key);
            }
        }
        for k in lost_keys {
            self.tracks.remove(&k);
        }
    }

    // --- ROI maintenance --------------------------------------------------

    /// Recomputes the active ROI as the padded union of this frame's detection
    /// corners. Clears it when the frame produced nothing.
    fn update_roi(&mut self, dets: &[Detection], width: u32, height: u32) {
        if dets.is_empty() {
            self.active_roi = None;
            return;
        }
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for det in dets {
            for c in det.corners {
                min_x = min_x.min(c.x);
                min_y = min_y.min(c.y);
                max_x = max_x.max(c.x);
                max_y = max_y.max(c.y);
            }
        }

        let box_w = (max_x - min_x).max(0.0);
        let box_h = (max_y - min_y).max(0.0);
        let pad = (box_w.max(box_h) * self.cfg.roi_padding_frac)
            .max(self.cfg.roi_padding_min_px as f64);

        let x0 = (min_x - pad).floor().max(0.0) as u32;
        let y0 = (min_y - pad).floor().max(0.0) as u32;
        let x1 = ((max_x + pad).ceil() as i64).clamp(0, width as i64) as u32;
        let y1 = ((max_y + pad).ceil() as i64).clamp(0, height as i64) as u32;

        self.active_roi = Some(Roi {
            x: x0,
            y: y0,
            w: x1.saturating_sub(x0).max(1),
            h: y1.saturating_sub(y0).max(1),
        });
    }
}

// -------------------------------------------------------------------------
// Free helpers
// -------------------------------------------------------------------------

/// Runs full-frame detection.
fn detect_full(frame: &Frame<'_>) -> Vec<Detection> {
    detect_luma(frame.luma, frame.width, frame.height)
}

/// Runs detection inside `roi` by copying the sub-window into a contiguous
/// buffer, then maps the resulting corners back into full-frame coordinates.
fn detect_in_roi(frame: &Frame<'_>, roi: Roi) -> Vec<Detection> {
    let (sub, w, h) = crop(frame, roi);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let mut dets = detect_luma(&sub, w, h);
    let ox = roi.x as f64;
    let oy = roi.y as f64;
    for det in &mut dets {
        for c in &mut det.corners {
            c.x += ox;
            c.y += oy;
        }
    }
    dets
}

/// Copies the `roi` sub-window out of `frame` into a fresh row-major buffer.
/// Clamps the ROI to the frame bounds.
fn crop(frame: &Frame<'_>, roi: Roi) -> (Vec<u8>, u32, u32) {
    let x0 = roi.x.min(frame.width);
    let y0 = roi.y.min(frame.height);
    let x1 = (roi.x + roi.w).min(frame.width);
    let y1 = (roi.y + roi.h).min(frame.height);
    let w = x1.saturating_sub(x0);
    let h = y1.saturating_sub(y0);
    if w == 0 || h == 0 {
        return (Vec::new(), 0, 0);
    }
    let mut buf = Vec::with_capacity(w as usize * h as usize);
    for y in y0..y1 {
        let row = y as usize * frame.width as usize;
        let start = row + x0 as usize;
        let end = row + x1 as usize;
        buf.extend_from_slice(&frame.luma[start..end]);
    }
    (buf, w, h)
}

/// Extracts the decoded payload from a detection, when present.
fn payload_of(det: &Detection) -> (Option<Vec<u8>>, Option<String>) {
    if let Some(d) = det.decoded.as_ref() {
        (Some(d.bytes.clone()), d.text.clone())
    } else {
        (None, None)
    }
}

/// Computes a stable identity key for a detection: a hash of the decoded bytes
/// when available, otherwise a hash of the sampled module grid. Two frames
/// showing the same physical code therefore share a key even when decoding is
/// unavailable.
///
/// Caveat: the two hash domains differ, so a code that decodes on some frames
/// but not others flips identity, producing a spurious `Lost`/`FirstSeen`
/// pair. Accepted trade-off — a module-grid hash of a *successfully decoded*
/// frame would tie identity to sampling jitter instead.
fn identity_key(det: &Detection, bytes: Option<&[u8]>) -> u64 {
    match bytes {
        Some(b) => fnv1a(b),
        None => hash_matrix(&det.matrix),
    }
}

/// FNV-1a 64-bit hash of the module grid (size-prefixed).
fn hash_matrix(m: &BitMatrix) -> u64 {
    let size = m.size();
    let mut h = fnv1a(&size.to_le_bytes());
    let mut byte = 0u8;
    let mut bit = 0u8;
    for y in 0..size {
        for x in 0..size {
            byte |= (m.get(x, y) as u8) << bit;
            bit += 1;
            if bit == 8 {
                h = fnv1a_step(h, byte);
                byte = 0;
                bit = 0;
            }
        }
    }
    if bit != 0 {
        h = fnv1a_step(h, byte);
    }
    h
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h = fnv1a_step(h, b);
    }
    h
}

#[inline]
fn fnv1a_step(h: u64, b: u8) -> u64 {
    (h ^ b as u64).wrapping_mul(FNV_PRIME)
}
