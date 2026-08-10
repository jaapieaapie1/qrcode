# Review Feedback — Step 3a qr-video

**Verdict: Step 3a CLEAR**
Reviewer: Richard-V
Date: 2026-07-23
Counts: 0 BLOCKER · 0 MAJOR · 6 MINOR

Verified: `cargo test -p qr-video` → 7 passed. `cargo clippy -p qr-video --all-targets -- -D warnings` → clean. No `unsafe`. Deps = qr-core + qr-detect only. Scope honored (only `crates/qr-video/` touched; no root Cargo.toml edit).

---

## BLOCKER
None.

## MAJOR
None.

## MINOR

### M1 — Identity flips silently when a code's decode success toggles across frames
`detector.rs:444` `identity_key` keys on `fnv1a(payload_bytes)` when decode succeeds, else `hash_matrix(matrix)`. These two hashes share one `u64` key domain. A real code that decodes on some frames and fails on others (noise, motion blur) therefore changes identity mid-stream → the old track times out to `Lost` and the same physical code re-emits `FirstSeen`. The *mechanism* is documented on `Observation.key` (`lib.rs`/`detector.rs:91`), but the *consequence* (spurious FirstSeen/Lost churn on intermittent decode, and matrix-hash instability on a non-pixel-stable code) is not. Per the review rule this is acceptable-if-documented → flag MINOR. Fix: add one line to the `Observation.key` / `identity_key` doc stating that identity is stable only while decode outcome is stable, and that a decode↔undecode transition (or sub-module drift in the matrix-hash fallback) re-issues FirstSeen. No code change required.

### M2 — Decode-feature coupling: pin it, don't mirror it (Bob's flagged question)
`payload_of` (`detector.rs:432`) unconditionally reads `det.decoded`, which in qr-detect is `#[cfg(feature = "decode")]` (`qr-detect/src/lib.rs:45`). Today qr-video compiles only because qr-detect's `default = ["decode"]` is pulled in. If any workspace member ever builds qr-detect with `default-features = false` and nothing re-enables decode, feature unification turns it off and qr-video fails to compile.
Ruling on the feature question: **do NOT add a mirrored optional `decode` feature on qr-video.** A mirror implies qr-video can build standalone-without-decode, which it cannot — `payload_of` reads `det.decoded` unconditionally, so a Cargo.toml-only mirror still fails to compile under `--no-default-features`; making it truly optional would require `#[cfg(feature="decode")]`-gating `payload_of` plus a fallback code path and a new test. That is not cheap and is out of scope for 3a.
Cheap + clean fix instead — pin the feature qr-video's code actually requires so it is robust to any downstream `--no-default-features`:
```toml
# crates/qr-video/Cargo.toml, line 9
qr-detect = { workspace = true, features = ["decode"] }
```
One line, no code change, guarantees `decoded` exists whenever qr-video is in the build. Prescribe this.

### M3 — No test for a code at the image border
Review focus called this out. All fixtures sit inset (offset 20/30 + quiet zone) so the ROI padded-bbox clamping (`detector.rs:367-377`) and crop-edge remap (`crop` + `detect_in_roi`) are never exercised against `x=0` / right / bottom edges. I traced the clamping and it is sound (`roi.x ≤ width` by construction, so `crop`'s clamped `x0` equals the `ox` used for remap → no off-by-one), but there is no test proving it. Add one fixture with the tile flush against a frame edge, assert it still detects and corners map back correctly.

### M4 — No test for the periodic full-scan refresh or ROI abandonment
`roi_config()` sets `full_scan_interval = 0`, so the refresh-cadence branch (`detector.rs:218`) and the `roi_miss_limit` abandonment branch (`scan`, `detector.rs:220-221`) are never asserted. Both are load-bearing (codes appearing outside the box; stale-ROI recovery). Add a test that a full scan fires on the interval boundary, and one where an ROI is dropped after `roi_miss_limit` consecutive misses.

### M5 — Redundant work / dead branch (perf + clarity)
- `emit_lost` (`detector.rs:309-313`) recomputes `payload_of` + `identity_key` for every detection already keyed in `record_detections`, cloning payload bytes an extra time per detection. Small payloads → negligible, but it is duplicated work in the per-frame path. Consider threading the keys computed in `record_detections` through to `emit_lost`.
- `Track.lost` and the `t.lost` check in `record_detections` (`detector.rs:266`) are effectively dead: `emit_lost` sets `lost = true` and removes the track in the same pass (`lost_keys`), so no track ever persists with `lost = true`. Harmless but confusing; drop the field or add a comment on why it stays.

### M6 — u32 multiply in crop capacity/indexing
`crop` (`detector.rs:421,423`) uses `(w * h) as usize` and `(y * frame.width) as usize` in u32 arithmetic, while `Frame::is_well_formed` deliberately widens to u64 to avoid overflow. Only a concern above ~4 Gpx (unreachable for real frames), but for consistency with the well-formed check, widen these to `usize`/`u64` before multiplying.

---

## Cleared
Reviewed `Cargo.toml`, `lib.rs`, `detector.rs`, `tests/video.rs`. ROI crop/remap coordinate math is sound (origin used for remap provably equals the clamped crop origin; no edge off-by-ones). Full-scan fallback on ROI miss never drops a frame. Timestamp handling is clean under backwards/non-monotonic time (saturating_sub → no spurious FirstSeen/Lost). All temporal logic is ms-based and internally consistent. State machine FirstSeen→Repeated→Lost→FirstSeen verified correct, codes tracked independently. Borrow design (lending `next_frame`) is sound, zero-copy on the full-frame hot path, ROI crop copy acceptable per brief. No unsafe, clippy clean, deps limited to qr-core + qr-detect. All six findings are MINOR — none blocks the step.
