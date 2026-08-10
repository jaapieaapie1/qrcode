# Review Request — Step 5c: qr-decode micro

**Builder:** Bob-MD
**Scope:** `crates/qr-decode/` only. No other crate touched.
**Ready for Review: YES**

---

## What was built

`decode_micro(&BitMatrix) -> Result<DecodedMicro, DecodeError>` behind a default-on
`micro` cargo feature, parallel to the existing `decode` path. Existing public API is
byte-for-byte unchanged: no new variants on `DecodeError`, no changed signatures, no
change to `Decoded`.

Pipeline: side length → `MicroVersion` → single format-info strip read + BCH correct →
symbol number cross-checked against the size → unmask + extract via the core placement
iterator → Reed-Solomon (correct, or *detect only* for M1) → micro bit-stream parse →
payload bytes + best-effort UTF-8.

Everything table-shaped comes from `qr_core::micro`. Nothing from Step 5a is duplicated:
no version tables, no mask predicates, no format-info codec, no placement walk, no
function-module map.

---

## Files changed

| File | Lines | What / why |
| --- | --- | --- |
| `crates/qr-decode/Cargo.toml` | 7–17 (new `[features]`) | `default = ["micro"]`, `micro = ["qr-core/micro"]`. Pure forward — nothing sets `default-features = false` on a sibling, so feature unification stays additive. |
| `crates/qr-decode/src/lib.rs` | 16–21 (docs), 35–36 (`pub mod micro`), 46–47 (re-export) | Gates and re-exports `decode_micro` / `DecodedMicro` at the crate root so they sit beside `decode` / `Decoded`; documents the feature. |
| `crates/qr-decode/src/bits.rs` | 7–37 | `BitReader` gained an explicit `bits` limit plus a `micro`-gated `with_bit_len`. Micro data streams end on a 4-bit boundary (M1 = 20 bits, M3-L = 84), so the trailing nibble of the backing slice must stay unreadable. `new()` behaves exactly as before. |
| `crates/qr-decode/src/segment.rs` | 16–96 (new helpers), 118–139 (call sites) | Extracted `push_numeric` / `push_alphanumeric` / `push_byte` / `push_kanji` as `pub(crate)` helpers and made the QR parser call them. Micro reuses them verbatim — the *data* encodings are identical across the two symbologies; only the mode and character-count indicators differ. No behaviour change to the QR parser. |
| `crates/qr-decode/src/micro.rs` | 1–283 (new) | The micro decoder. |
| `crates/qr-decode/tests/micro.rs` | 1–787 (new) | 28 tests, whole file `#![cfg(feature = "micro")]`. |

### `src/micro.rs` map

- `DecodedMicro` (48–72) — `bytes`, `text`, `version: MicroVersion`, `ec: MicroEcLevel`,
  `mask`, `errors_corrected`.
- `decode_micro` (74–105) — mirror handling: straight decode, then transpose-and-retry,
  reusing the crate-private `transpose` from `lib.rs` (not duplicated).
- `decode_micro_oriented` (107–137) — the pipeline, including the symbol-number vs size
  cross-check.
- `read_block` (139–176) — walks `micro_data_modules`, un-masks with `micro_mask_bit`,
  packs into the single RS block honouring the 4-bit final data codeword.
- `correct_block` (178–225) — the RS gate (see below).
- `data_bit_stream` (221–235) — repacks the corrected codewords into a contiguous
  MSB-first bit stream.
- `parse_micro_segments` (237–283) — the M-specific bit stream.

---

## The three things the brief called out

### 1. M1 is detection-only, and never corrected

`MicroEcLevel::Detection` / `error_correction_capacity == 0` takes a separate branch that
**never calls `rs_correct`**. It recomputes the two EC codewords with `rs_encode` over the
received data and compares — for a systematic code that is exactly the "are all syndromes
zero?" test — and returns `BlockCorrection` on any mismatch. `errors_corrected` is always
`0` for M1.

`m1_rejects_any_damage_without_correcting` proves it exhaustively: it flips **every one of
the 36 data modules in turn** and asserts all 36 are rejected. With `rs_correct` those
would be single-codeword errors sitting inside `ec_len / 2 = 1`, so a naive implementation
would silently "recover" a different payload on all of them.

### 2. Format-info decode success is not a validity gate

Documented at the top of `src/micro.rs`. The guards, in order:

1. **Symbol-number vs size.** `info.version != version` → `DecodeError::VersionInfo`.
   With one format copy and no second copy to vote against, this is the only structural
   check available; it removes 6 of the 8 symbol numbers for any given size.
2. **Reed-Solomon.** The real gate.

`random_grids_are_rejected_cleanly` feeds 1 200 pseudorandom grids (300 per version)
through `decode_micro` and asserts every one is an `Err` — *and* asserts that well over
100 of them got **past** the format information, so the test would fail if the BCH decode
were silently doing the rejecting instead of RS. `blank_and_solid_grids_are_rejected`
covers the all-light word the core docs single out (it decodes to M4-L mask 1).

### 3. Misdecode protection is enforced (a deliberate addition)

`rs_correct` allows up to `d / 2` corrections. ISO 7.5.1 Formula 3 (`e + 2t ≤ d − p`)
publishes a lower capacity `r` for symbols with `p > 0`: the surplus codewords must
*detect*, not correct. `correct_block` therefore rejects any block that needed more than
`info.error_correction_capacity`.

This bites on M2-L (`r=1, d/2=2`), M2-M, M3-L (`r=2, d/2=3`) and M4-L (`r=3, d/2=4`) —
i.e. on half the micro symbol set, where `p` is 2–3 out of only 5–8 EC codewords, so the
misdecode risk is material. `misdecode_protection_caps_correction_below_the_reed_solomon_bound`
pins it on M4-L and M3-L; `m2_rejects_two_damaged_codewords` pins it on M2-L.

**Note for Richard:** the existing QR path (`extract.rs`) does *not* apply this cap — it
uses the generic `d / 2`. I did not touch it (out of scope for 5c). Flagging the
inconsistency; if you want it unified, that is an Arch call and a separate step.

---

## Decisions worth a second opinion

1. **`DecodeError` is reused rather than a new error type.** Adding variants to a public
   enum is forbidden by the Wave 4 rules, and every micro failure maps cleanly onto an
   existing variant. `decode_micro`'s doc comment spells out the micro reading of each one
   (`VersionInfo` = "symbol number contradicts the size"). The alternative — a parallel
   `MicroDecodeError` — is more literally "parallel types", but adds a public type whose
   variants would be identical. Happy to flip if you prefer the parallel type.

2. **The M1/M3 half codeword is zero-extended into the LOW nibble for RS.** ISO 7.4.11
   says the final data codeword is 4 bits and the pad in that position is `0000`, but does
   not state how the 4 bits enter the GF(256) polynomial. The only place ISO makes the
   convention explicit is Annex N.1 for model 1 QR: *"The first 4-bit data codeword is
   prefixed with 0000 to make its length 8 bits for generating the error correction
   codewords."* I followed that: value `0..=15`, high nibble zero. `data_bit_stream` then
   shifts it back up so the bit reader sees a contiguous stream.

   **This must match Step 5b.** If Bob-E chose the high nibble, one of us has to move.
   It is a one-line change on either side. Worth an explicit cross-check before 5e.

   I added a corollary guard: after correction, a half codeword with a non-zero high
   nibble is a mis-correction (the true value can never have one) and is rejected as
   `BlockCorrection`.

3. **The terminator is detected as "numeric mode indicator + zero character count".** The
   micro terminator is `2n + 1` zero bits for version Mn, which is *exactly* a numeric mode
   indicator (n−1 bits) followed by a zero character count (Table 3: 3/4/5/6 bits) — the
   arithmetic works out for all four versions. So one uniform rule handles the terminator,
   the abbreviated terminator (fewer bits remain than a header needs → stop) and the
   no-terminator full-capacity case. This is what keeps pad codewords `0xEC`/`0x11` from
   being parsed as segment headers. `padding_after_the_terminator_is_not_parsed`,
   `full_capacity_payload_needs_no_terminator` and `empty_payload_decodes_to_nothing`
   cover the three shapes.

---

## Tests — 28, all green

Ground truth is ISO Figure I.4 (the printed M2-L `"01234567"` symbol), transcribed as an
ASCII grid. **No dependency on `qr-encode`.** The fixture builder in the test file is
assembled from `qr-core` primitives and is itself pinned against Figure I.4 module-for-
module before any derived fixture is trusted.

- `iso_figure_i4_decodes` — the printed ISO symbol decodes to `"01234567"`, M2, L, mask 1,
  0 corrections.
- `fixture_builder_reproduces_figure_i4` — licenses every other fixture.
- `annex_i3_data_codewords` — `[0x40, 0x18, 0xAC, 0xC3, 0x00]`, the Annex I.3.2 codewords.
- Happy path: M1 numeric, M2 alphanumeric, M3 byte and numeric (both half-codeword
  layouts, `data_bits` 84 and 68), M4 byte × 3 levels × 4 masks, M4 Kanji (Shift-JIS,
  both SJIS ranges), and `every_symbol_number_roundtrips` over all 8 symbols.
- Correction: M2-L r=1, M2-M r=2, M3-L r=2, M4-Q r=7 (the deepest micro correction).
- Rejection: M1 exhaustive single-flip sweep; the `r` cap on M4-L/M3-L/M2-L; 1 200 random
  grids; blank and solid grids; every invalid side length; wrong symbol number for the
  size; unknown mode indicator; overlong character count.
- Mirror: transposed Figure I.4, plus a transposed fixture for all 8 version/level pairs.

---

## DoD

| Gate | Result |
| --- | --- |
| `cargo test -p qr-decode` | 7 + 28 pass |
| `cargo test -p qr-decode --no-default-features` | 7 pass, micro suite compiled out |
| `cargo check --workspace` | green |
| `cargo clippy -p qr-decode --all-targets -- -D warnings` | clean |
| `cargo clippy -p qr-decode --all-targets --no-default-features -- -D warnings` | clean |
| `cargo fmt --check -p qr-decode` | clean |
| `cargo check -p qr-decode --target thumbv7em-none-eabihf` | green, both feature configs |
| `cargo check -p qr-decode --target riscv32imc-unknown-none-elf` | green, both feature configs |
| `cargo test -p tests-integration` | 54 pass — the `segment.rs` refactor is regression-free |

`#![no_std]` and `#![forbid(unsafe_code)]` untouched.

---

## Open questions

1. Half-codeword nibble convention vs Step 5b (decision 2 above) — needs a cross-check
   with Bob-E before Step 5e.
2. `DecodeError` reuse vs a parallel `MicroDecodeError` (decision 1).
3. The misdecode-protection cap now differs between the micro path (enforced) and the QR
   path (not enforced). Arch call whether to unify.
