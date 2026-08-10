# Architect Brief — QR Workspace

Project: full QR code stack from scratch in Rust. Cargo workspace at repo root, crates in `crates/`.
Reference: `ISO_IEC_18004_2024(en).pdf` in repo root (read specific pages only when a table/algorithm is needed — it is 16MB, never read whole).

Global constraints (all steps):
- Zero external deps except: `image` (only qr-detect, qr-render), video I/O interface deps (only qr-video, wave 3). Everything else hand-rolled — including Reed-Solomon, GF(256), BCH, binarization, perspective transform.
- `qr-core`, `qr-encode`, `qr-decode`: `#![no_std]` + `extern crate alloc`. `cargo check` must pass; no hidden std usage.
- Workspace edition 2024, resolver 3. Crate skeletons already exist and compile.
- Coordinate convention: `(x, y)` = (column, row), origin top-left. Document on every public API taking coordinates.
- Tests colocated (`#[cfg(test)]` + `tests/` dir). Tests may use std.
- Do not touch crates outside your step. Root Cargo.toml is owned by Arch — if you need a workspace dep added, note it in REVIEW-REQUEST, do not edit root.
- No `unsafe`. No panics on malformed input in library paths — return `Result`/`Option`.

---

## Step 1 — qr-core (foundation, blocks everything)

Public API surface (exact names negotiable, semantics not):
- `Version(u8)` newtype, 1..=40. `size() = 17 + 4*version`. Alignment pattern coordinate table (ISO Annex E). Total codeword count per version.
- `EcLevel { L, M, Q, H }`.
- `Mode { Numeric, Alphanumeric, Byte, Kanji, Eci }` + char-count-indicator bit lengths per version group.
- `BitMatrix`: square bit grid, packed storage, `get/set(x, y)`, `size()`. This type crosses every crate boundary — keep lean.
- ECC block structure table: per (version, ec_level) → (ec codewords per block, group1 count, group1 data len, group2 count, group2 data len). Full 40×4 from ISO Table 9. Test invariant: data+ec sums to total codewords per version.
- GF(256), polynomial 0x11D: exp/log tables (const-built), add/mul/div/pow/inv.
- Reed-Solomon: `rs_encode(data, ec_len)` (generator derived, not hardcoded); `rs_correct(codewords, ec_len) -> Result<errors_fixed, Error>` — syndromes, Berlekamp-Massey, Chien, Forney. Erasures not required v1.
- Format info: 15-bit BCH(15,5) encode + decode-with-correction (≤3 bit errors), mask 0x5412. 32-entry table lookup acceptable for decode.
- Version info: 18-bit BCH(18,6) encode + decode-with-correction, versions ≥7.
- Masking: the 8 mask predicates `mask_bit(mask_id, x, y) -> bool` (shared encode+decode).
- Function-pattern map: `is_function_module(version, x, y)` — finders+separators, timing, alignment, format/version areas, dark module. Single source of truth for encode AND decode.

Tests (mandatory):
- RS known vector (ISO Annex I example, version 1-M "01234567"); corrupt ≤ floor(ec/2) bytes → corrected; more → clean error.
- Format/version BCH: all valid words roundtrip; 1–3 bit flips corrected.
- Table invariants v1–v40 × 4 levels.
- GF: a·inv(a)=1 ∀ a≠0.

### Definition of Done
- [ ] `cargo test -p qr-core` green
- [ ] `cargo check -p qr-core` green (no_std respected)
- [ ] All API surface above present + doc comments

## Step 2a — qr-encode (after core)
Segmentation (numeric/alnum/byte auto-select; greedy mixed-mode acceptable v1 — Known Gap if simplified), bit stream + terminator + pad bytes (0xEC/0x11), version auto-select (smallest fitting), block split + RS + interleave, zigzag placement, all 8 masks + ISO penalty N1–N4, format/version info placement. API: `encode(&[u8], Options { ec, version: Option, mask: Option, mode: Option }) -> Result<BitMatrix>`. Kanji optional v1 (Known Gap if skipped). ECI not in encoder v1.
Tests: Annex I vector → exact matrix if feasible; penalty unit tests; smoke encode v1–v40.
DoD: tests green, no_std check green.

## Step 2b — qr-decode (after core)
Input `BitMatrix`. Mirror handling: retry transposed on failure. Format info from both copies, version info ≥7 both copies, unmask via core predicates, zigzag extraction skipping function modules via core map, deinterleave, RS-correct per block (count corrected errors), parse segments: numeric/alnum/byte/kanji/ECI (designator parsed; ECI 26 → UTF-8; else raw bytes), FNC1 tolerated/skipped. API: `decode(&BitMatrix) -> Result<Decoded { bytes, text: Option<String>, version, ec, mask, errors_corrected }>`.
Tests: known matrices, corrupted-codeword recovery, garbage → clean error.
DoD: tests green, no_std check green.

## Step 2c — qr-render (after core)
`render_image(&BitMatrix, &Opts) -> image::RgbaImage` (scale, quiet zone default 4, fg/bg); `render_svg -> String` (no deps); `render_ascii -> String`. std + workspace `image` dep.
Tests: dimensions, quiet zone, pixel spot checks, SVG string asserts.
DoD: tests green.

## Step 2d — qr-detect (after core + 2b; may stub decode call until 2b merges — integrate via `qr_decode::decode`)
Pipeline: gray buffer (`image::GrayImage` in, plus `&[u8]`+dims entry) → binarize (adaptive window; Otsu fallback) → finder candidates 1:1:3:1:1 scanline (horizontal + vertical cross-check) → cluster centers → triple selection (right angle + size similarity) → module size + orientation → alignment refine (v≥2) → homography → sample grid → `BitMatrix` → decode. API: `detect(&GrayImage) -> Vec<Detection { corners, matrix, decoded }>` + `read_one`. Multi-code per image.
Tests: synthetic — use qr-encode + qr-render as dev-deps, apply rotation/scale/perspective/noise in-test.
DoD: tests green incl. warped-image cases.

## Step 3a — qr-video (after 2d)
No hard capture dep. `trait FrameSource { fn next_frame(&mut self) -> Option<Frame> }`, Frame = luma + dims + timestamp. ROI tracking from last detection, temporal dedup/debounce of repeated payloads, fast-path early exit. Capture adapter only behind feature flag — propose dep in REVIEW-REQUEST first.

## Step 3b — integration tests (after all)
Workspace member `tests-integration` (dev-only): encode→render→rasterize→detect→decode roundtrip across versions/levels/masks/content (binary + UTF-8); seeded fuzz loop.

---

# Wave 4 — Micro QR (Steps 5a–5e). ISO 18004:2024 Micro QR symbology.

Locked design decisions (Architect + Project Owner):
- Cargo feature `micro`, DEFAULT-ON, one polarity everywhere: qr-core defines it; qr-encode/qr-decode/qr-detect forward it (`micro = ["qr-core/micro"]` etc.). Opt-out via `default-features = false`. Nothing in the dependency chain may set `default-features = false` on a sibling (feature additivity — qr-video keeps its explicit `features = ["decode"]` pin).
- PARALLEL TYPES, not shared enums: `MicroVersion` (M1–M4), `encode_micro()`, `decode_micro()`, `DecodedMicro` (or clearly-named equivalents). Existing public signatures MUST NOT change; no new variants on existing public enums. qr-render needs zero changes (BitMatrix in, pixels out). qr-video inherits detection via qr-detect.
- All micro items live in `micro`-gated modules; crates must build cleanly with `--no-default-features` (minus micro) AND with it on. Both configs in every DoD.
- Reference: ISO PDF at repo root. Micro spec spread through the main clauses (symbol sizes 11/13/15/17; single finder; timing row 0 / col 0; format info BCH(15,5) XOR mask 0x4445 with 3-bit symbol number + 2-bit data mask; 4 micro data-mask patterns; modes M1 numeric-only no indicator, M2 1-bit, M3 2-bit, M4 3-bit indicators; short terminators; M1/M3 final 4-bit half codeword; EC: M1 error-detection-only 2 ec cw, M2/M3 L+M, M4 L/M/Q). VERIFY every table against the PDF — do not trust memory; page-targeted reads.

## Step 5a — qr-core micro foundation (blocks 5b–5d)
In `crates/qr-core/src/micro.rs` (feature-gated), reusing gf/rs/bitmatrix untouched:
- `MicroVersion` M1–M4: size 9+2*n, symbol capacity tables (total/data/ec codewords per version×level incl. M1 detection-only), allowed `EcLevel` subsets, mode availability, char-count widths, terminator lengths.
- Micro format info: BCH(15,5) reuse, XOR mask 0x4445, symbol-number encoding table ((version, ec) ↔ 3-bit symbol number), placement coordinates around the single finder; encode + decode-with-correction.
- The 4 micro mask predicates (ISO micro table — NOT the 8 QR masks).
- `micro_is_function_module(mv, x, y)`: finder+separator top-left, timing full row 0 + col 0, format area.
- Codeword placement order helper if cleanest here (2-wide zigzag, no col-6 skip, respects half codewords) — or leave to encode/decode symmetry via a shared placement iterator in core (preferred: single source of truth like `is_function_module` was).
Tests: table invariants vs PDF values, format info roundtrip + 1–3 bitflips, mask predicate spot checks vs ISO figure, function map counts (data module count == 8*codewords - 4*half adjustments per version).
DoD: both feature configs green, clippy clean, no_std kept.

## Step 5b — qr-encode micro (after 5a)
`encode_micro(&[u8], MicroOptions { version: Option<MicroVersion>, ec, mask: Option }) -> Result<BitMatrix>`: mode auto-select respecting per-M availability, smallest-fitting version auto-select, bit stream with M-specific indicators/counts/terminator, M1/M3 half-codeword handling, RS via core, placement via core iterator, 4-mask evaluation with the ISO micro mask-selection score (edge-count method — NOT QR N1–N4), format info placement.
Tests: ISO Annex I micro example if present (verify in PDF; there is an M2 "01234567" example — reproduce exactly), capacity boundary per version×level, all 4 masks forced, auto-select correctness.
DoD: both configs green, clippy, no_std.

## Step 5c — qr-decode micro (after 5a)
`decode_micro(&BitMatrix) -> Result<DecodedMicro>`: size → MicroVersion, format info read+correct → symbol number+mask, unmask, extract via core placement iterator, RS correct (M1: detection-only — verify 2 ec codewords as checksum, no correction attempt beyond?  ISO: M1 has error detection capability only — attempt correction with 2 ec cw is 1-error… follow ISO: treat as detection, reject on nonzero syndrome), parse M-specific bitstream, half-codeword handling. Mirror retry like QR path.
Tests: fixtures from core primitives (independent of 5b), corruption, garbage.
DoD: both configs green, clippy, no_std.

## Step 5d — qr-detect micro (after 5a; decode integration after 5c lands)
Micro detection: single finder pattern (existing finder scan already yields candidates), orientation from the two timing patterns radiating along the symbol edges adjacent to the finder, dimension from timing-run module count (11/13/15/17 snap), corner/homography from finder square + timing endpoints, sample → BitMatrix → decode_micro. Integrate into `detect`: micro candidates are finder clusters NOT consumed by any QR triple; `Detection` gains NOTHING publicly — add parallel `MicroDetection` + `detect_micro()` and a combined convenience if clean. Rotation support to ±45° like QR path where feasible; document envelope honestly.
Tests: synthetic micro symbols (hand fixtures or via 5b as dev-dep once landed), rotation sweep, QR+micro coexisting in one image without cross-stealing finders.
DoD: both configs green, clippy.

## Step 5e — integration micro suite (after 5b–5d)
tests-integration: micro roundtrip sweep M1–M4 × levels × content, encode→render→detect_micro→decode_micro image path, mixed image (QR + micro), fuzz extension, rotation cases.
DoD: full workspace green.

## Step 6 — CI (Arch-owned)
GitHub Actions: test matrix (stable; macOS+ubuntu), clippy -D warnings, fmt --check, cross-target checks (thumbv7em-none-eabihf, riscv32imc-unknown-none-elf for no_std trio; wasm32-unknown-unknown all crates), both feature configs.

---

## Builder Plan
*Builder adds their plan here before building. Architect reviews and approves.*

### Bob-T — Step 2d qr-detect (building now, per direct dispatch)
Modules in crates/qr-detect/src/: geometry (Point + ZXing-style PerspectiveTransform, hand-rolled),
binarize (Bradley adaptive-window via integral image + Otsu fallback), finder (1:1:3:1:1 scanline +
vertical cross-check + center clustering), grouping (triple selection, right-angle/orientation,
dimension estimate), sampler (alignment refine v>=2, homography, grid sample -> BitMatrix),
lib (detect/read_one orchestration). API: `detect(&GrayImage) -> Vec<Detection{corners,matrix,decoded}>`,
`detect_luma(&[u8],w,h)`, `read_one`. Multi-code supported. Tests: synthetic fixtures drawn in test
code (finders/alignment/deterministic data), rendered to GrayImage, warped (rotate/scale/perspective/
noise) with hand-rolled transforms, sampled BitMatrix compared module-for-module. NOT depending on
qr-encode/qr-render (concurrent). qr-decode call gated behind `decode` cargo feature (see note in
REVIEW-REQUEST-detect.md). This plan follows a direct build dispatch; proceeding without blocking.

Architect approval: [x] Approved (retroactive — matches Step 2d brief; 2D template match for alignment is a sound deviation) / [ ] Redirect

### Bob-M — Step 5a qr-core micro foundation (building now, per direct dispatch)
Single new file `crates/qr-core/src/micro.rs`, gated `#[cfg(feature = "micro")]` from lib.rs; new
`micro` feature, `default = ["micro"]`. Zero edits to existing modules — micro reuses `bch`
(remainder helper made `pub(crate)`), `mask_bit`, `gf`, `reed_solomon`, `BitMatrix`, `EcLevel` as-is.

API: `MicroVersion {M1..M4}` (size/number/from_size roundtrips), `MicroEcLevel {Detection,L,M,Q}`
(PARALLEL type — M1's "error detection only" is not an `EcLevel`, and adding a variant to `EcLevel`
is forbidden; converts to/from `EcLevel`), `MicroSymbolInfo` (symbol number, total/data/ec codewords,
data bits, correction capacity r, misdecode protection p, half-codeword flag), `MicroMode`
(+ indicator value/width, char-count width, availability per version), `micro_mask_bit`,
`micro_format_info_encode/decode`, `MICRO_FORMAT_INFO_POSITIONS`, `micro_is_function_module`,
and `micro_data_modules()` — an allocation-free `Iterator<Item=(u32,u32)>` that is the single source
of truth for encode+decode placement (like `is_function_module`).

Deviation flagged for Arch: brief said "allowed `EcLevel` subsets"; M1 has no EC level at all
(ISO Table 13 symbol 0 = "Error detection only"), so a parallel `MicroEcLevel` is used instead.

Verification already completed against the ISO PDF before writing code (see REVIEW-REQUEST-micro-core.md
for clause citations): Tables 1, 2, 3, 7, 9, 10, 13, C.1, Figures 4, 11, 26, and the full Annex I.3/I.4
M2-L worked example — Figure I.4 was rasterised at 600 dpi and decoded to a ground-truth 13x13 grid,
which the placement iterator + mask 01 + format info reproduce with 0 mismatches on all 80 data modules.

This plan follows a direct build dispatch; proceeding without blocking.

Architect approval: [ ] Approved / [ ] Redirect
