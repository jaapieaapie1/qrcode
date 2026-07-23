# Review Feedback — Step 1 (qr-core)

**Verdict: Step 1 CLEAR**

Date: 2026-07-23
Reviewer: Richard

Counts: 0 BLOCKER · 0 MAJOR · 2 MINOR

---

## Verification run

- `cargo test -p qr-core` → 26 passed, 0 failed (gf 5, bch 7, reed_solomon 6, tables 4, matrix_mask 4).
- `cargo clippy -p qr-core --tests` → 0 warnings.
- `#![forbid(unsafe_code)]` present; no `unsafe`, no `use std`/`std::` anywhere in `src/` (only the guarded `#[cfg(test)] extern crate std`). Zero external deps in Cargo.toml. no_std discipline holds.

## Math correctness — verified by hand + tests

- **GF(256):** PRIM `0x11D` correct. exp table doubled to 512 so `mul` (max log sum 508), `div` (max 509), `inv` (max 255) never index out of range. `exp(i)` reduces `i%255`. All within bounds. `add/mul/div/inv` axioms tested exhaustively over the field.
- **Reed-Solomon decode chain:** generator roots α^0..α^(ec-1) (fcr=0). Syndromes, Berlekamp-Massey (textbook, XOR subtraction, correct discrepancy/branch logic), Chien (locator α^(n-1-i), root test Λ(X⁻¹)=0), Forney (`e = X·Ω(X⁻¹)/Λ'(X⁻¹)`, correct for b=0). Post-correction syndrome re-verification guarantees no silent mis-decode. `positions.len()!=err_count` and `den==0` both fail closed.
- **RS test adequacy (focus item 4):** `correct_up_to_capacity` at num=5 corrupts index 17 — an EC byte — so errors in BOTH data and EC regions are exercised; `roundtrip_various_lengths` also lands errors in the EC tail. ISO Annex I v1-M vector (data→EC = A5 24 D4 ... 55) and gen-poly-10 exponents both asserted exactly.
- **BCH:** FORMAT_GEN `0x537` = x¹⁰+x⁸+x⁵+x⁴+x²+x+1, FORMAT_MASK `0x5412`, VERSION_GEN `0x1F25` = x¹²+x¹¹+x¹⁰+x⁹+x⁸+x⁵+x²+1 — all correct. Known value M/mask101 → `0x40CE` matches ISO Annex C. Min-distance-7 and 1–3 bitflip correction tested for all words.
- **Mask predicates (focus item 1):** all 8 match ISO Table 10 exactly, with i=row(y), j=col(x). Mask 4 uses floor(i/2)+floor(j/3); masks 5–7 correct.
- **Block table spot checks:** v1-M `(10,1,16,0,0)`→26 ✓; v7-Q `(18, 2×14, 4×15)`→data 88 + ec 108 = 196 ✓; v40-H `(30, 20×15, 61×16)`→data 1276 + ec 2430 = 3706 ✓. All 160 entries cross-checked by `block_sums_match_total_codewords`.
- **Alignment (Annex E) spot checks:** v2 [6,18], v7 [6,22,38], v32 [6,34,60,86,112,138], v39 [6,26,54,82,110,138,166], v40 [6,30,58,86,114,142,170] — all correct.
- **Function map:** finders+separators (8×8 ×3), timing (row/col 6), format L-bands + dark module, version blocks (≥7), 5×5 alignment with the 3 finder-colliding centers skipped. Cross-validated for ALL 40 versions by `function_map_matches_data_region_size` (non-function count == 8·total_codewords + remainder_bits) — this single test transitively validates the function map, the total-codeword table, and the alignment table against each other.
- **API soundness:** BitMatrix OOB reads→false, OOB writes→ignored, no panic; (x,y)=(col,row) convention consistent across BitMatrix, mask, and function map and asserted in tests. `Version::new` rejects 0/41. No panicking paths on malformed input.

---

## MINOR (log as known gap — do not block)

- **[gf.rs:108] `pow` can overflow-panic on 32-bit no_std targets.** `(LOG[a] as usize) * (n as usize)` overflows `usize` when `n` is large on a 32-bit target (debug panic), contradicting the module's "total and panic-free" claim. Harmless today (only test caller, small n; QR never needs large exponents). If a fix is wanted it is one line: reduce `n % 255` before multiplying (valid since α has order 255). Log as known gap.
- **[reed_solomon.rs:128] `rs_correct` doc wording.** Comment says the buffer "is left unchanged from the point of failure," but the Forney loop mutates `codewords[pos]` before the final syndrome re-verify can return `Err`, so the buffer may be partially modified on failure. Behavior is fine and the "treat Err as unrecoverable, don't read the buffer" contract is stated — only the wording is slightly inaccurate. Bob already flagged this as an open question; confirm the qr-decode caller honors it in Step 2.

## Cleared

qr-core is feature-complete and correct per the Step 1 brief: GF(256), full RS decode chain, BCH format/version, BitMatrix, the 40×4 block table, total-codeword/alignment/remainder tables, 8 masks, and the function-pattern map — no_std, no unsafe, zero deps, 26 tests green, clippy clean. Nothing blocks. The two MINOR items are latent/cosmetic and can be logged.
