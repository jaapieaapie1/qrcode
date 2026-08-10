# Review Request — Step 3a qr-video

**Builder:** Bob-V
**Ready for Review:** YES
**Scope lock honored:** only `crates/qr-video/` touched. No root `Cargo.toml` edit. No other crate touched.

## What this is

Stateful, capture-agnostic multi-frame QR detection layered on the committed
`qr-detect` pipeline. Adds ROI tracking, temporal de-dup/debounce, and a
full-scan cadence. No capture backend; zero new dependencies (only `qr-core` +
`qr-detect`, both already in `[workspace.dependencies]`).

## Files

- `crates/qr-video/Cargo.toml` — added `qr-core` + `qr-detect` workspace deps (lines 7-9). No new external deps.
- `crates/qr-video/src/lib.rs` (new, ~120 lines) — public surface: `Frame<'a>`, `FrameSource` trait, `EventKind`, re-exports `Point`, and re-exports detector types.
- `crates/qr-video/src/detector.rs` (new, ~430 lines) — `Config`, `Stats`, `Observation`, `VideoDetector`; ROI scan/fallback/refresh logic, dedup + lost tracking, crop + coordinate mapping, FNV-1a identity hashing.
- `crates/qr-video/tests/video.rs` (new, ~380 lines) — synthetic fixtures (cribbed from `qr-detect/tests/detect.rs`), a `MovingQrSource` implementing `FrameSource`, and 7 tests.

## API summary

- `Frame<'a> { luma: &'a [u8], width, height, timestamp_millis: u64 }` — borrowed luma, caller-supplied ms timestamp (no `std::time` in the API, stays embeddable-ish). `Frame::new`, `is_well_formed()`.
- `trait FrameSource { fn next_frame(&mut self) -> Option<Frame<'_>> }`.
- `VideoDetector`: `new(Config)`, `with_defaults()`, `process_frame(Frame) -> Vec<Observation>`, `run(&mut source, sink)` to drive a `FrameSource`, `stats() -> Stats`, `active_roi() -> Option<(x,y,w,h)>`.
- `Observation { kind: EventKind, key: u64, bytes: Option<Vec<u8>>, text: Option<String>, corners: Option<[Point;4]>, timestamp_millis, via_roi }`.
- `EventKind { FirstSeen, Repeated, Lost }`.
- `Config { full_scan_interval, roi_padding_frac, roi_padding_min_px, roi_miss_limit, debounce_millis, lost_after_millis, emit_repeated }` + `Default`.
- `Stats` — 10 counters (frames, full_scans, roi_scans, roi_hits, roi_misses, roi_fallbacks, detections, first_seen, repeated, lost).

## Key decisions

**Borrow design — lending method, no GAT.** `next_frame(&mut self) -> Option<Frame<'_>>` ties the frame's lifetime to `&mut self`, so the source owns one reusable buffer and each pulled frame invalidates the previous one. This is exactly a camera / demux loop: one live frame, zero per-frame allocation. I deliberately did **not** implement `Iterator` (its item can't borrow `self`, forcing an owned buffer per frame) nor a GAT `LendingIterator` trait (more machinery than a single elided-lifetime method needs). No `unsafe`. Rationale is documented on the trait itself. Tradeoff — you process each frame before pulling the next — is what streaming wants. `MovingQrSource` in the tests demonstrates the pattern (owns a canvas, blits, returns a borrow).

**Event model.** `process_frame` returns `Vec<Observation>` tagged with `EventKind`. `FirstSeen` on a new/expired/re-appeared payload; `Repeated` within the debounce window (suppressed from output unless `Config::emit_repeated`, but always counted in `Stats`); `Lost` when a tracked payload goes unseen past `lost_after_millis`, after which it is dropped so a return is a fresh `FirstSeen`.

**Identity key.** Decoded payload bytes hashed (FNV-1a) when `qr-decode` succeeds; otherwise the sampled module grid is hashed. This is what makes the tests work at all: the hand-drawn fixtures are not valid QR payloads so `decode` returns `None` (same as `qr-detect`'s own tests), and the matrix-hash fallback still gives stable cross-frame identity. In production, real codes decode and dedup on payload.

**ROI scan path.** After any hit the padded union bbox of detection corners is armed as the ROI. Next frame: if an ROI is active, not at a refresh boundary, and under the miss limit -> crop the luma to the ROI, run `detect_luma` on the sub-buffer, map corners back to full-frame. On an ROI miss the same frame falls back to a full scan (frame is never dropped just because the box drifted), miss counter increments, and after `roi_miss_limit` consecutive misses the ROI is abandoned. `full_scan_interval` forces a periodic full-frame refresh so codes appearing outside the box are still found. `via_roi` on each `Observation` (and the `roi_scans`/`roi_hits` stats) expose which path ran — the requested test hook.

## Test results

`cargo test -p qr-video` — **7 passed, 0 failed.**
`cargo clippy -p qr-video --all-targets -- -D warnings` — **clean.**

Coverage vs DoD:
- ROI engages: `roi_tracking_engages_after_first_hit` — frame 0 full-scan hit arms ROI; frame 1 takes ROI path (`roi_scans==1`, `roi_hits==1`) and its observation is `via_roi==true`.
- Dedup debounce across frames: `dedup_debounces_repeated_payload` — 8 frames -> 1 `FirstSeen`, 7 counted repeats, none emitted (default).
- Lost then refound: `lost_then_refound_is_reported` — exact sequence `FirstSeen, Lost, FirstSeen`.
- Moving code ≥10 frames: `moving_code_tracked_across_frames` — 12 frames, one identity, `first_seen==1`, ROI carries ≥10 of 12 frames.
- Extras: two-codes-two-identities, malformed-frame-graceful, blank-stream-nothing.

## Notes / open questions

- **No workspace dep needed.** I stayed within `qr-core` + `qr-detect`; no root `Cargo.toml` change required. If Arch later wants an optional real capture adapter (e.g. `v4l`/`nokhwa`) it should be a separate feature-gated dep proposed then — not added here (brief 3a: propose in review first).
- **`qr-render` unused.** The brief offered rendering fixtures via `qr-render`; I hand-drew tiles instead (as `qr-detect` does), avoiding even a dev-dep and any coupling to a crate not in `[workspace.dependencies]`.
- **Decode-feature coupling.** `qr-video` relies on `qr-detect`'s default `decode` feature (so `Detection::decoded` exists). If detection is ever built `--no-default-features`, `payload_of` would not compile. Acceptable given decode is on by default; flag if you want it feature-mirrored.
- No `BUILD-LOG.md` edit — dispatch said write only my own handoff file while the qr-encode review runs concurrently.
