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

## Builder Plan
*Builder adds their plan here before building. Architect reviews and approves.*

Architect approval: [ ] Approved / [ ] Redirect
