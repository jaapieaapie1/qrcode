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

### Step 5a — qr-core micro foundation — BUILT, AWAITING REVIEW
*Date: 2026-08-10 — Bob-M*

Files: `crates/qr-core/src/micro.rs` (new, 734), `crates/qr-core/tests/micro.rs` (new, 707),
`Cargo.toml` (+`micro` feature, default-on), `lib.rs` (+gated `pub mod micro`),
`bch.rs` (`bch_remainder` → `pub(crate)` so micro reuses the shared BCH(15,5); ISO Annex C.2
specifies the same generator for both symbologies).

API: `MicroVersion`, `MicroEcLevel`, `MicroSymbolInfo`, `MicroMode`, `micro_mask_bit`,
`micro_format_info_{encode,decode,read,write}`, `MICRO_FORMAT_INFO_POSITIONS`,
`micro_is_function_module`, `micro_data_modules()`. All parallel types — no existing
signature or enum changed. Detail + ISO citations per table: handoff/REVIEW-REQUEST-micro-core.md.

Key decisions:
- `MicroEcLevel { Detection, L, M, Q }` is a NEW parallel enum, not `EcLevel`. ISO Table 13
  symbol 0 is "Error detection only" — M1 has no EC level, and H never applies. Deviates from
  the brief's "allowed EcLevel subsets" wording; flagged for Arch ack.
- `micro_data_modules()` is an allocation-free iterator and the single source of truth for
  encode+decode placement (the `is_function_module` pattern). Takes only the version: module
  order does not vary with EC level; only the bit-stream index of the M3 half codeword does.
- Micro masks forward to `mask_bit` via `MICRO_MASK_TO_QR = [1,4,6,7]` — ISO Table 10 lists
  both references against one condition column, so no formula is duplicated.
- Micro mask-selection score (ISO 7.8.3.2 edge count) deliberately NOT in core — it belongs in
  qr-encode (5b), by symmetry with N1–N4.

Verification: every table read page-targeted from the ISO PDF (Tables 1, 2, 3, 7, 9, 10, 13, C.1;
clauses 5.3.2.2–5.3.8, 7.4.10, 7.4.11, 7.5.1, 7.6, 7.7.3, 7.9.2, Annex C). Figures 4 and 26
rasterised and read. **Annex I.3 / Figure I.4 rendered at 600 dpi and decoded to a 13×13 grid;
the M2-L example is reproduced from core primitives with 0 mismatches across all 169 modules**,
which pins placement order, mask mapping, format-info value and format-info bit positions to
printed ISO output.

Tests: 14 new. `cargo test -p qr-core` = 40 pass (micro on) / 26 pass (micro off, file cfg'd out).
`cargo check --workspace` + `clippy -D warnings` green in both configs. no_std proven by
`cargo check -p qr-core --target {thumbv7em-none-eabihf,riscv32imc-unknown-none-elf}` in both configs.

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
- **KG-8** — PARTIALLY CLOSED by Step 5a: qr-core micro foundation built behind the default-on `micro` feature (tables, format info, 4 masks, function map, placement iterator). Encode/decode/detect remain unsupported until Steps 5b–5d. Original entry: Micro QR (M1–M4) unsupported across all crates. ISO 18004:2024 includes it (sizes 9+2*M, single finder, own format-info/EC/mode tables; M1 numeric-only). Cleanly additive to current design; detect needs a single-finder algorithm. If built: `micro` cargo feature, parallel types (no shared Symbol enum). Awaiting Project Owner decision — logged 2026-07-23
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
