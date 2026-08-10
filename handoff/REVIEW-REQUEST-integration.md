# Review Request — Step 3b: `tests-integration`

**Builder:** Bob-I
**Ready for Review:** YES
**Scope:** `tests-integration/` only. No crate under test was modified.

---

## Result

```
cargo test  -p tests-integration                          50 passed, 2 ignored, 0 failed  (26.6 s wall)
cargo clippy -p tests-integration --all-targets -- -D warnings    clean
```

Two `#[ignore]`d tests, both pinning genuine `qr-detect` defects found by this
work (**CI-1**, **CI-2** below). Nothing else is skipped.

---

## Files

| File | Lines | What |
|---|---:|---|
| `tests-integration/src/lib.rs` | 15 | Doc-only; the crate exists solely to host `tests/`. |
| `tests-integration/tests/common/mod.rs` | 555 | Shared helpers: capacity arithmetic, independent zigzag walk, `Gray` buffer + warps, xorshift64* PRNG. |
| `tests-integration/tests/full_roundtrip.rs` | 198 | Suite 1 — encode → render → `GrayImage` → detect → decode. |
| `tests-integration/tests/matrix_roundtrip.rs` | 309 | Suite 2 — encode → decode sweep, ISO Annex I pins, RS corruption. |
| `tests-integration/tests/warped.rs` | 270 | Suite 3 — rotation / downscale / perspective / noise. |
| `tests-integration/tests/video_session.rs` | 357 | Suite 4 — multi-frame session, ROI, cadence, borders. |
| `tests-integration/tests/fuzz_seeded.rs` | 109 | Suite 5 — deterministic pseudo-fuzz. |

`tests/common/mod.rs` carries `#![allow(dead_code)]`: each test binary compiles
it separately and uses a different subset.

---

## Suite 1 — full stack roundtrip (`full_roundtrip.rs`, 11 tests, 1.1 s)

`encode(payload)` → `qr_render::render_image` → `DynamicImage::to_luma8()` →
`qr_detect::read_one` → `Detection::decoded`, asserting byte-exact payload plus
version / EC / mask readback, `errors_corrected == 0`, **and** that the
detector's sampled `BitMatrix` equals the encoder's module-for-module.

- Full cross product for v1, v2, v5, v7 × {L,M,Q,H} × {numeric, alphanumeric,
  byte-binary, UTF-8} — 64 cases, one test per version so they run in parallel.
- Sampled for the big ones: v10 (all four EC levels), v20 (L, Q), v40 (L, H).
- All 8 forced masks at v3-M through the image path.
- Every payload fills its symbol **to capacity** (densest modules = hardest
  sampling case).
- Binary payloads always contain `0x00` and `0xFF`; UTF-8 payloads mix 2-, 3-
  and 4-byte code points and are asserted on both `bytes` and `text`.
- Two different codes composited into one image — both found, both decoded to
  their own payload.

## Suite 2 — matrix-level roundtrip (`matrix_roundtrip.rs`, 11 tests, 18.5 s)

Exhaustive and imaging-free.

- **v1–v40 × {L,M,Q,H}, capacity-filled** — one test per content kind (4 × 160
  = 640 encode/decode pairs), asserting payload, version, EC and
  `errors_corrected == 0`.
- **Short payloads across v1–v40 × all EC** — exercises the terminator +
  `0xEC`/`0x11` pad path the capacity sweep never reaches. Content kind rotates
  across the grid rather than multiplying it (the sweep already covers all four
  kinds at every version; a 4× cross product here cost 64 s in a debug build for
  no extra signal).
- **All 8 masks × {L,M,Q,H} at v1, v3, v7, v14, v27, v40.**

### KG-3 closed — ISO Annex I EC codewords pinned

`annex_i_ec_codewords_match_published_values` pins `rs_encode` output to the
literal values published in **ISO/IEC 18004:2024, Annex I.2.3** (p. 89–90).

> **The values in the build dispatch were wrong.** The brief gave
> `[42,159,74,221,244,169,239,150,138,70]`. I checked the PDF: the standard
> lists the 26-codeword bit stream, whose last ten codewords are
> `10100101 00100100 11010100 11000001 11101101 00110110 11000111 10000111
> 00101100 01010101` = `[0xA5,0x24,0xD4,0xC1,0xED,0x36,0xC7,0x87,0x2C,0x55]` =
> `[165,36,212,193,237,54,199,135,44,85]`. Per the brief's own instruction I
> used the PDF's values. They agree with `rs_encode` and with the existing
> `qr-core` / `qr-encode` unit tests, so nothing in the stack was wrong — only
> the dispatch note.

### KG-2 closed — placement geometry validated independently

`annex_i_symbol_placement_matches_published_stream` encodes `"01234567"` at
v1-M, then walks the finished symbol with an **independently written** zigzag
(`common::data_module_order`, from ISO 7.7.3), un-masks with mask 2 and compares
the recovered 26 codewords against the Annex I stream (16 data + 10 EC). This
validates the encoder's module placement against the standard rather than
against the decoder's own reader — the circularity KG-2 flagged.

Incidentally confirms `qr-encode`'s `((right + 1) & 2) == 0` upward-column rule
is equivalent to plain alternation including across the `x == 6` timing-column
skip.

### KG-1 closed — multi-block RS corruption

`multi_block_corruption_is_corrected_v5_q` — v5-Q has **4 blocks** (2×15 + 2×16
data, 18 EC each, t = 9). The test flips all 8 modules of each of the first
`4 × 3 = 12` interleaved codewords, asserts (via `common::data_block_indices`,
an independent model of ISO 7.6 interleaving) that the damage spans ≥ 2 blocks
with ≤ t errors each, then requires `bytes == payload` and
`errors_corrected == 12`.

Plus:
- `ec_region_corruption_is_corrected_v5_q` — same but in the EC region.
- `beyond_correction_capacity_fails_cleanly_v5_q` — `t + 1` errors all aimed at
  block 0; must return `BlockCorrection` or `Malformed`, never a wrong payload,
  never a panic.

## Suite 3 — warped detection (`warped.rs`, 14 passed / 2 ignored, 0.2 s)

Warp helpers are written locally: `qr-detect`'s live in its own `tests/` dir and
are not reachable across crates. `qr-detect` was **not** modified.

Passing: rotation 20° / 90° / 180° / 270°; downscale to ~2.5 px/module (M and H)
and a non-integer ~2.8 px/module ratio; perspective keystone at auto-version and
at v7 (alignment-refinement path); bell-shaped noise at amplitude 45 and 70;
rotate + downscale + noise combined; keystone + noise on v5.

Ignored: `rotated_30_degrees` (CI-1) and
`single_rotated_symbol_yields_single_candidate` (CI-2).

## Suite 4 — video session (`video_session.rs`, 9 tests, 0.2 s)

Frames are real rendered symbols composited onto a white canvas, driven through
a `FrameSource` implementation.

- `session_enter_move_leave_reenter` — 20 frames at 100 ms: visible 0–5
  (drifting right), empty 6–12, visible again 13–19. Asserts the emitted event
  sequence is exactly `FirstSeen, Lost, FirstSeen`, with the Lost landing at
  t = 1000 ms (first frame ≥ 500 ms after the last sighting), matching keys,
  corners present on sightings and absent on Lost, and `Stats` showing
  `detections == 13`, `first_seen == 2`, `lost == 1`, `roi_scans > 0`,
  `roi_hits > 0`.
- `roi_sourced_observation_reports_full_frame_corners` — an ROI-sourced
  `Observation` sets `via_roi` and its corners are mapped back to full-frame
  coordinates.

### KG-5 closed — border positions

- `code_at_top_left_border` — code flush at `(0,0)`; ROI padding would go
  negative, must clamp to `(0,0)`, stay inside the frame, and the next frame's
  ROI scan must still hit.
- `code_at_bottom_right_border` — mirror case, ROI clamps to `(w, h)`.
- `code_partially_off_frame_is_handled` — half a symbol: no event, no stale ROI,
  no panic.

### KG-5 closed — full-scan refresh cadence

- `full_scan_refresh_follows_configured_cadence` — `full_scan_interval = 3` over
  9 stationary frames: exactly 4 full scans (frame 1 + cadence frames 3/6/9) and
  5 ROI scans, 0 misses.
- `full_scan_refresh_zero_disables_periodic_rescan` — `full_scan_interval = 0`:
  1 full scan, 5 ROI scans.
- `roi_miss_triggers_full_frame_fallback` — the code jumps 380 px between
  frames: 1 ROI miss, 1 same-frame full-frame fallback, no lost detection, and
  de-duplication still emits a single `FirstSeen`.
- `malformed_frame_is_survivable` — truncated luma buffer is treated as empty,
  still drives the Lost timer, drops the ROI, does not panic.

## Suite 5 — seeded pseudo-fuzz (`fuzz_seeded.rs`, 5 tests, 5.8 s)

xorshift64* seeded four ways × 50 iterations = **200 cases**. Each picks a
random version (1–40), EC level, content kind and length (1..=capacity); the
binary kind uses fully random bytes. Failures print the seed-derived iteration
index, version, EC, kind and length, and re-run identically. Plus
`fuzz_degenerate_payloads`: empty payload and single bytes `0x00 / 0x0A / 0x7F /
0xFF` at every EC level.

---

## Cross-crate bugs found

Both are in `qr-detect` and both are **left unfixed** per the dispatch (crate is
committed and reviewed, and outside Step 3b's write scope). Each is pinned by an
`#[ignore]`d test carrying the full analysis in its doc comment.

### CI-1 — dimension estimate is not rotation-corrected (`qr-detect`, medium)

`crates/qr-detect/src/grouping.rs:117`, in `score_triple`:

```rust
let raw_dim = ((leg_top + leg_left) / 2.0) / module + 7.0;
let dimension = snap_dimension(raw_dim)?;
```

`leg_top` / `leg_left` are finder centre-to-centre Euclidean distances and are
rotation-invariant. `module`, however, is the mean of `Finder.module`, which
`finder.rs` measures as **horizontal and vertical scanline run lengths**. For a
symbol rotated by θ those runs are stretched by `1 / cos θ`, so `raw_dim`
undershoots the true dimension by `(dimension - 7) * (1 - cos θ)` modules.
`snap_dimension` tolerates roughly 2 modules of error, after which the detector
**silently returns a smaller valid version** — `detect()` reports success, the
matrix has the wrong side length, and `decode` then fails.

The `leg_err > 0.20` guard below does not catch it: at 30° the leg error is only
~7 %.

Measured breakdown angle (rendered at 10 px/module, θ folded into 0..=45°):

| version | 1 | 2 | 3 | 4 | 5 | 7 | 10 | 15 | 20 |
|---|---|---|---|---|---|---|---|---|---|
| last good θ | 34° | 30° | 27° | 25° | 23° | 20° | 17° | 14° | 12° |

Every entry matches `(dimension - 7) * (1 - cos θ) ≈ 2` to the degree, which
confirms the mechanism. Note the envelope **shrinks as the version grows** — a
v20 code is unreadable past 12° of rotation, which is well inside normal
hand-held camera use.

The angle folds at 45°, as predicted: a v4 symbol fails identically at 30° and
at 60°, and works again near 90°.

Suggested fix (Arch/Bob's call, not done here): measure module size along the
finder-to-finder direction — e.g. ZXing's `sizeOfBlackWhiteBlackRunBothWays`
walked between finder centres — or cross-check against the timing pattern,
instead of reusing the axis-aligned scanline run width.

Pinned by:
- `warped.rs::rotated_30_degrees` — `#[ignore]`d, the case that should pass.
- `warped.rs::rotation_dimension_underestimate_is_still_present` — **runs and
  passes today**; it asserts the underestimate is still present and fails loudly
  with a "drop this test and un-ignore" message once CI-1 is fixed, so the fix
  cannot land silently.

### CI-2 — phantom extra candidate under rotation (`qr-detect`, low)

`grouping::group` can emit a second candidate from the same three finder
patterns, snapped to a smaller dimension, so `detect()` returns an extra
non-decodable `Detection`.

Repro (v7-M, 10 px/module, 200 px pad): 20° → `sizes = [45, 21]`;
22° → `sizes = [41, 29]`.

`read_one` is immune — it prefers a detection that decoded — but any consumer
counting detections over-counts, including `qr_video::Stats::detections`. Likely
the same root cause as CI-1 (a mis-snapped dimension makes an overlapping triple
look plausible), so a CI-1 fix may close this too.

Pinned by `warped.rs::single_rotated_symbol_yields_single_candidate`
(`#[ignore]`d).

### Not a bug — Annex I EC values in the dispatch

See KG-3 above: the ten literals in the build dispatch do not match ISO/IEC
18004:2024 Annex I.2.3. The code was right; the note was wrong. Recorded here so
the wrong values do not propagate into another brief.

---

## Proposed Known Gaps updates (BUILD-LOG.md — not edited, per scope lock)

```
- KG-1 — CLOSED by tests-integration/tests/matrix_roundtrip.rs (v5-Q, 4 blocks)
- KG-2 — CLOSED by matrix_roundtrip.rs::annex_i_symbol_placement_matches_published_stream
- KG-3 — CLOSED by matrix_roundtrip.rs::annex_i_ec_codewords_match_published_values
         (pinned to the ISO PDF, not to the values quoted in the Step 3b brief)
- KG-5 — CLOSED by tests-integration/tests/video_session.rs (border + cadence + ROI abandonment)
- KG-8 — NEW: detect — dimension estimate not rotation-corrected; symbols fail
         past ~34 deg (v1) falling to ~12 deg (v20). See CI-1, REVIEW-REQUEST-integration.md
- KG-9 — NEW: detect — phantom extra candidate from the same finder triple under
         rotation; detect() over-reports. See CI-2
```

KG-4 (Kanji / structured append), KG-6 (mixed-mode segmentation, Kanji/ECI
encode) and KG-7 (interior alignment grid) are unchanged — all are missing
features rather than defects, and the integration suites deliberately stay
inside the implemented surface.

---

## Runtimes (debug build, `cargo test -p tests-integration`, 10-core, parallel)

| suite | wall |
|---|---:|
| `full_roundtrip` | 1.1 s |
| `matrix_roundtrip` | 18.5 s |
| `warped` | 0.2 s |
| `video_session` | 0.2 s |
| `fuzz_seeded` | 5.8 s |
| **total** | **26.6 s** |

`matrix_roundtrip` dominates; it is 800+ encode/decode pairs in an unoptimised
build and is already split across five parallel test functions. `cargo test
--release` would cut it substantially if the suite ever moves to a slower CI
box.

---

## Open questions for Arch

1. **CI-1 severity.** A v20 code failing past 12° of rotation is, in my view, a
   shipping blocker for the camera path, not a Known Gap. Want me to open it as
   a Step 4 fix in `qr-detect`, or does it stay logged?
2. **`errors_corrected` semantics.** I asserted it equals the number of
   corrupted codewords (12 for the KG-1 case) and it does — i.e. it counts error
   *positions* across all blocks. Worth a doc note on `Decoded::errors_corrected`
   so the meaning is contractual rather than incidental?
3. **Runtime budget.** 26.6 s debug is fine locally. If integration is meant to
   run on every commit, say the word and I will move the v1–v40 sweeps behind
   `#[ignore]` + an explicit `--ignored` CI job, or drop them to
   `--release`-only.
