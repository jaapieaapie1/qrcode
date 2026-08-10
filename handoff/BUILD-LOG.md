# Build Log
*Owned by Architect. Updated by Builder after each step.*

---

## Current Status

**Active step:** 3b — tests-integration (Bob-I building; final step)
**Last cleared:** Waves 1+2+3a — all six crates committed, reviewed CLEAR: qr-core f8f74ab, qr-render 0ea46ed, qr-decode 6d8c31c, qr-detect 96b5bd4, qr-video 898ee1c, qr-encode 62c19b7
**Pending deploy:** NO (local git only; no remote configured)

---

## Step History

### Step 0 — Workspace scaffold — COMPLETE
*Date: 2026-07-23*

Files changed:
- `Cargo.toml` — workspace, resolver 3, edition 2024, 6 members
- `crates/{qr-core,qr-encode,qr-decode,qr-detect,qr-video,qr-render}/` — skeletons, compile green
- git initialized, scaffold committed

### Wave 2 — encode/decode/render/detect — IN PROGRESS
*Date: 2026-07-23*
Four Bobs (Opus) parallel: Step 2a encode, 2b decode, 2c render, 2d detect.
Per-crate review requests: handoff/REVIEW-REQUEST-{encode,decode,render,detect}.md.
Detect codes against decode's brief-defined API; decode call feature-gatable if sibling lags.

### Step 1 — qr-core — COMPLETE, REVIEWED CLEAR, COMMITTED (f8f74ab)
*Date: 2026-07-23*
Bob (Opus) built per brief. `cargo test -p qr-core` = 26 passing, `cargo check`
green (no_std), clippy clean.

Files created under `crates/qr-core/src/`: gf.rs, reed_solomon.rs, bch.rs,
bitmatrix.rs, blocks.rs, version.rs, ec.rs, mode.rs, mask.rs, function.rs;
lib.rs wired. Tests under `crates/qr-core/tests/`: gf, reed_solomon, bch,
tables, matrix_mask.

Key decisions:
- alloc used for BitMatrix + RS buffers (rest is alloc-free). #![forbid(unsafe_code)].
- RS: roots α^0.., fcr=0; rs_correct re-verifies syndromes → no silent mis-decode.
- BCH decode: brute-force min-distance (≤3 bit correction).
- Function-map correctness cross-checked geometrically against total-codeword +
  remainder tables for all 40 versions (independent verification).

See handoff/REVIEW-REQUEST.md for full detail. Ready for Richard.

---

## Known Gaps

- **KG-1** — CLOSED by Step 3b (v5-Q 4-block corruption test, errors_corrected == 12)
- **KG-2** — CLOSED by Step 3b (independent zigzag walk pinned to ISO Annex I stream)
- **KG-3** — CLOSED by Step 3b (Annex I EC bytes pinned to PDF values, verified by Richard-I against §I.2.3)
- **KG-4** — decode: Kanji exposed as raw Shift-JIS bytes, no SJIS→UTF-8 table; structured append parsed-and-skipped, not reassembled — logged 2026-07-23
- **KG-5** — CLOSED by Step 3b (border clamping + full_scan_interval cadence + ROI-miss fallback tests)
- **KG-6** — encode: single-mode segmentation only (no greedy mixed-mode), no Kanji/ECI encoder-side — logged 2026-07-23
- **KG-7** — detect: severe perspective on high versions uses single BR alignment anchor, not full interior alignment grid — logged 2026-07-23
- **KG-8** — Micro QR (M1–M4) unsupported across all crates. ISO 18004:2024 includes it (sizes 9+2*M, single finder, own format-info/EC/mode tables; M1 numeric-only). Cleanly additive to current design; detect needs a single-finder algorithm. If built: `micro` cargo feature, parallel types (no shared Symbol enum). Awaiting Project Owner decision — logged 2026-07-23
- **KG-9** — detect: rotation envelope for large versions at steep angles (v20/v40 fail 35–40°, pass 45°) is bounded by the module estimator's own +6–8% high bias in the mixed chord-taper regime (Richard-F measured; NOT a raster precision floor). Next lever: filter corner-tapered scanlines before averaging — logged 2026-07-23
- **KG-10** — qr-detect not rustfmt-clean prior to Step 4; fmt applied in Step 4 commit — logged 2026-07-23

---

## Architecture Decisions
*Locked decisions that cannot be changed without breaking the system.*

- Workspace layout: crates/ dir, 6 crates: qr-core (no_std foundation), qr-encode (no_std+alloc), qr-decode (no_std+alloc), qr-detect (std+image), qr-video (std, trait-based FrameSource, no hard capture dep), qr-render (std+image) — 2026-07-23
- All shared QR domain logic (GF256, RS, BCH, tables, masks, function-pattern map, BitMatrix) lives in qr-core; encode/decode must not duplicate it — 2026-07-23
- Coordinate convention (x, y) = (column, row), origin top-left — 2026-07-23
- Zero external deps except `image` (detect/render) and gated video I/O — 2026-07-23
- Build waves: 1=core, 2=encode+decode+render+detect parallel, 3=video+integration — 2026-07-23
