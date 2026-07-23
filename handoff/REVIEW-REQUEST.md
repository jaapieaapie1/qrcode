# Review Request — Step 1 (qr-core)
*Written by Builder (Bob). Read by Reviewer (Richard).*

Ready for Review: YES

---

## What Was Built

The complete `qr-core` foundation crate: `#![no_std]` + `extern crate alloc`,
`#![forbid(unsafe_code)]`, zero external dependencies. Provides GF(256)
arithmetic (poly 0x11D, compile-time tables), Reed-Solomon encode + full decode
(syndromes → Berlekamp-Massey → Chien → Forney), BCH format(15,5)/version(18,6)
encode + decode-with-correction, the `BitMatrix` module grid, `Version` /
`EcLevel` / `Mode` metadata with the full ISO Table 9 block structure (40×4) and
total-codeword / alignment tables, the 8 mask predicates, and the
`is_function_module` map. `cargo test -p qr-core` = 26 passing, `cargo check -p
qr-core` green, `cargo clippy` zero warnings.

## Files Changed

All under `crates/qr-core/`. No other crate or the root Cargo.toml was touched.

| File | Lines | Change |
|---|---|---|
| `src/lib.rs` | 1-77 | Crate root: no_std/alloc/no-unsafe attrs, module wiring, public re-exports, `Error` enum (+`Display`/`core::error::Error`). |
| `src/gf.rs` | 1-140 | GF(256) with compile-time exp/log tables (0x11D); `add/mul/div/inv/pow/exp/log`. |
| `src/reed_solomon.rs` | 1-260 | `generator_poly`, `rs_encode`, `rs_correct` (BM + Chien + Forney), in-place correction returning error count; verifies zero syndromes before success. |
| `src/bch.rs` | 1-120 | Format (0x537, mask 0x5412) and version (0x1F25) BCH encode + brute-force min-distance decode (≤3 bit correction). |
| `src/bitmatrix.rs` | 1-110 | Bit-packed square grid, `get/set/toggle/size/new`; OOB reads false, OOB writes ignored (no panic). |
| `src/blocks.rs` | 1-180 | `EcBlocks` + full ISO Table 9 `[40][4]` as `ec_blocks(version, ec)`, with codeword-count helpers. |
| `src/version.rs` | 1-165 | `Version(1..=40)` newtype: `size/total_codewords/remainder_bits/alignment_pattern_positions`; total-codeword + Annex E alignment tables. |
| `src/ec.rs` | 1-75 | `EcLevel {L,M,Q,H}` + `index`/`format_bits` (M=00,L=01,H=10,Q=11). |
| `src/mode.rs` | 1-70 | `Mode` + 4-bit indicators + char-count-indicator bit lengths per version group. |
| `src/mask.rs` | 1-30 | The 8 `mask_bit(mask_id, x, y)` predicates (ISO Table 10). |
| `src/function.rs` | 1-95 | `is_function_module(version, x, y)` — finders/separators, timing, format, version-info (≥7), dark module, alignment (skips 3 finder-colliding centers). |
| `tests/gf.rs` | new | a·inv(a)=1 ∀a≠0, exp/log inverse, mul/div roundtrip, pow, identities. |
| `tests/reed_solomon.rs` | new | ISO Annex I v1-M "01234567" known EC vector + gen-poly-10 exponents; correct ≤⌊ec/2⌋ → fixed; more → clean error; multi-length roundtrips. |
| `tests/bch.rs` | new | Format/version roundtrip all words, 1–3 bitflip correction, min-distance-7, known 0x40CE value. |
| `tests/tables.rs` | new | Block-sum == total_codewords (160 entries) + geometric: function-map data-module count == 8·codewords+remainder (all 40). |
| `tests/matrix_mask.rs` | new | BitMatrix get/set/toggle/OOB, mask non-degeneracy + checkerboard. |

## Design Decisions (made without explicit instruction)

- **`alloc` is used**, not avoided. `BitMatrix` (dynamic size to v40 = 177²) and
  the RS working buffers need heap; a fully alloc-free core would force fixed
  max-size arrays on every caller. GF/BCH/mask/function/tables are alloc-free.
- **RS convention**: first codeword = highest-degree term; generator roots
  α^0..α^(ec_len-1) (QR convention, fcr=0). Forney uses `e = X·Ω(X⁻¹)/Λ'(X⁻¹)`.
  `rs_correct` re-verifies syndromes are zero after correction and returns
  `TooManyErrors` if not — so a mis-decode never silently returns Ok.
- **BCH decode is brute-force** min-Hamming-distance over the 32 (format) / 34
  (version) valid words — well within budget and matches the brief's "table
  lookup acceptable".
- **`EcLevel::format_bits`** encodes the ISO mapping M=00,L=01,H=10,Q=11 (this is
  the format-info bit order, distinct from the L,M,Q,H enum/index order).
- **Function-map validation is independent**: the geometric test counts
  non-function modules straight from `is_function_module` and asserts it equals
  `8·total_codewords + remainder_bits`. This cross-checks the function map, the
  total-codeword table and (indirectly) the block table against each other — a
  single transcription error in any of the three would fail it. It passes for
  all 40 versions.

## Open Questions / To Double-Check

- **Erasure decoding not implemented** (brief: "Erasures not required v1"). RS
  handles errors-only.
- **Coordinate/skip logic for alignment patterns**: I skip the three grid
  corners (TL/TR/BL) that collide with finders. Validated only indirectly via
  the module-count test; a dedicated encode/decode roundtrip in Step 2 will
  exercise exact placement.
- **`rs_correct` on hard failure**: the buffer may be left partially modified
  when it returns `Err` (documented). Callers must treat `Err` as an
  unrecoverable block and not read the buffer — confirm that matches how
  qr-decode will use it.
- **Root Cargo.toml**: no workspace dep additions were needed for qr-core.

## Known Gaps Logged

None new. qr-core is feature-complete per the Step 1 brief.

## Test / Check Results

- `cargo test -p qr-core` → 26 passed, 0 failed (gf 5, reed_solomon 6, bch 7,
  tables 4, matrix_mask 4; doctests 0).
- `cargo check -p qr-core` → green (no_std respected).
- `cargo clippy -p qr-core --tests` → 0 warnings.
