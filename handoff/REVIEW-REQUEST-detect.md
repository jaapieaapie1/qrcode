# Review Request — Step 2d qr-detect

Builder: Bob-T
Ready for Review: YES
Scope: `crates/qr-detect/` only. No other crate, no root Cargo.toml touched.

## What this is
Full QR detection pipeline: grayscale image -> located + sampled QR symbols
(BitMatrix), optionally decoded. All image math hand-rolled; the `image` crate is
used only for `GrayImage` buffer types.

## Files
- `crates/qr-detect/Cargo.toml` — deps `qr-core`, `image` (workspace), optional
  `qr-decode`. Feature `decode` (default ON) wires the decode call.
- `crates/qr-detect/src/lib.rs` — public API + pipeline orchestration.
- `crates/qr-detect/src/geometry.rs` — `Point` + `PerspectiveTransform`
  (square-to-quad / adjoint / compose; hand-rolled). 3 unit tests.
- `crates/qr-detect/src/binarize.rs` — Bradley adaptive threshold via integral
  image + Otsu global fallback (small images / degenerate results). 2 unit tests.
- `crates/qr-detect/src/finder.rs` — `1:1:3:1:1` scanline scan, vertical
  cross-check, center clustering, vote filter. 2 unit tests.
- `crates/qr-detect/src/grouping.rs` — triple selection (right-angle vertex =
  TL, handedness for TR/BL), dimension estimate + snap, greedy multi-code
  selection. 3 unit tests.
- `crates/qr-detect/src/sampler.rs` — alignment refine (v>=2) via 5x5 template
  match, grid->image homography, module sampling into `BitMatrix`.
- `crates/qr-detect/tests/detect.rs` — 13 end-to-end tests over self-generated
  fixtures (see below).

## Public API
- `detect(&GrayImage) -> Vec<Detection>` — all symbols in an image.
- `detect_luma(&[u8], w, h) -> Vec<Detection>` — the `&[u8]`+dims entry.
- `read_one(&GrayImage) -> Option<Detection>` / `read_one_luma(..)` — best/first
  (prefers a decoded one when the feature is on).
- `Detection { corners: [Point;4], matrix: BitMatrix, decoded: Option<Decoded> }`
  — `decoded` field present only with the `decode` feature. `corners` ordered
  TL, TR, BR, BL in `(x,y)`=(col,row), origin top-left, y down.
- Re-exports `Point`, `PerspectiveTransform`.

## Pipeline (per brief Step 2d)
binarize (adaptive window + Otsu fallback) -> finder scan `1:1:3:1:1` (H scan +
vertical cross-check) -> center clustering -> triple selection (right angle +
size similarity + leg equality + dimension consistency) -> module size +
orientation -> alignment refine (v>=2) -> homography -> grid sample -> BitMatrix
-> decode. Multi-code supported.

## Key decisions
- Perspective handling uses a single grid->image homography anchored by the 3
  finder centers plus one refined bottom-right alignment pattern (v>=2). For v1
  (no alignment) the 4th corner is a parallelogram estimate (affine-exact for
  rotate/scale/shear; adequate given no alignment pattern exists).
- Alignment refine is done by **2D template matching** (5x5 dark-border /
  light-gap / dark-center), scoring 25 cells with local basis vectors taken from
  the provisional transform — NOT by isolated `1:1:1:1:1` run scanning. Reason:
  the alignment pattern's outer border routinely merges with adjacent data
  modules, so run isolation fails ~50% of rows; template matching is robust to
  that and follows the perspective. Center then sub-pixel-refined on the central
  dark blob.
- Finder clusters with fewer than 3 scanline hits are dropped (real finders get
  many hits; random-data flukes get 1-2). Prevents spurious extra symbols.
- `decode` feature: qr-decode landed during this build with the exact brief
  signature `decode(&BitMatrix) -> Result<Decoded, _>`, so the feature is
  default-ON and verified to compile + run. `--no-default-features` builds
  detection standalone (also tested + clippy-clean). Per dispatch, the call sits
  behind `build_detection` so the crate compiles either way.

## Tests
`cargo test -p qr-detect`: 10 unit + 13 integration = 23 pass, 0 fail.
Also green with `--no-default-features`. `cargo clippy -p qr-detect --all-targets`
clean in both configs.

Integration fixtures are generated entirely in test code (no qr-encode /
qr-render dependency — those are concurrent; roundtrip is Step 3b): a synthetic
module grid (3 finders + separators + timing + BR alignment for v>=2 +
deterministic pseudo-random data) is drawn into a grayscale buffer, then warped
with hand-rolled transforms, and the sampled BitMatrix is compared
module-for-module. Cases: upright v1/v2/v7; small/large module scale; rotation
~7deg and ~20deg; perspective (keystone) v2 and v7; bounded noise; two codes in
one image; blank image; decode-wiring smoke test.

## Uncertainties / notes for Richard
- Fixtures verify **geometry** (raw sampled modules == source modules). They are
  not valid QR payloads, so the decode smoke test asserts `decoded == None`
  (graceful, no panic). A real encode->detect->decode roundtrip is Step 3b.
- Very high versions under *severe* perspective rely on a single BR alignment
  pattern; extreme warps could still drift (real decoders use the full interior
  alignment grid). Tested warps (v7 keystone) pass exactly; pathological warps
  are out of scope for v1. Flagging as a possible future enhancement, not a gap.
- I added a short Builder Plan note under `handoff/ARCHITECT-BRIEF.md` per
  BUILDER.md; no other shared handoff file was modified.
