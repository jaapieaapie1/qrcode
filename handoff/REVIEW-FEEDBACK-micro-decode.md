# Review Feedback — Step 5c: qr-decode micro

Reviewer: Richard-MD
Date: 2026-08-10
Ready for Builder: YES

**Verdict: Step 5c CLEAR**

BLOCKER: 0 · MAJOR: 0 · MINOR: 4 · Escalate: 2

Judged standalone. qr-encode and qr-detect were not read; qr-core micro taken as
trusted (committed). No files touched outside this feedback file.

---

## Gates — all green, re-run by me, not taken on report

| Gate | Result |
| --- | --- |
| `cargo test -p qr-decode` | 7 + 28 pass |
| `cargo test -p qr-decode --no-default-features` | 7 pass, micro suite compiles to 0 tests |
| `cargo check --workspace` | green |
| `cargo clippy -p qr-decode --all-targets -- -D warnings` | clean |
| `cargo clippy -p qr-decode --all-targets --no-default-features -- -D warnings` | clean |
| `cargo fmt --check -p qr-decode` | clean |
| `cargo test -p tests-integration` | green (segment refactor regression-free) |

`#![no_std]`, `#![forbid(unsafe_code)]` intact. No `unsafe`, no `unwrap`/`expect`/
indexing panic in any library path (see Robustness below).

---

## Must Fix

None.

---

## Verified — the six focus areas

### 1. Pipeline correctness — correct

- **Size → version.** `MicroVersion::from_size` gates first; `InvalidSize` for
  anything but 11/13/15/17. `wrong_sizes_are_rejected` covers 1/10/12/14/16/18/21/25,
  including the QR side of the boundary (21, 25).
- **Symbol-number vs size.** `micro.rs:118` rejects `info.version != version` with
  `VersionInfo` **before** anything downstream touches `info`. Clean error, no
  panic, no partial work. `symbol_number_must_agree_with_the_size` writes an M4-L
  format word into a 13×13 M2 grid and pins `Err(VersionInfo)`. The check is
  symmetric, so the M4-format-in-11×11 case the brief asked about takes the same
  path. Correct: with one format copy there is no second copy to vote against, and
  this is the only structural guard available.
- **Unmask.** `read_block` XORs `micro_mask_bit(mask, x, y)` only against modules
  yielded by `micro_data_modules(version)` — the core iterator is the sole source
  of the data region, so the mask cannot reach a function module. Single source of
  truth preserved; nothing from 5a duplicated.
- **Extraction.** Via the core iterator, with `codeword_bits()` giving 4 bits at
  `index + 1 == data_codewords` when `has_half_final_data_codeword()` and 8
  elsewhere. Module count reconciles exactly: M1 `8+8+4+8+8 = 36`, M3 `132`, both
  matching `MicroVersion::data_modules()`. EC codewords correctly stay 8-bit after
  the half codeword.
- **`with_bit_len` at the 4-bit boundary.** `bits.min(data.len() * 8)` cannot
  exceed the slice; `read()` guards `pos + n > self.bits`. `data_bit_stream`
  shifts the trailing nibble up by 4 so an MSB-first walk of `data_bits` bits is
  gapless. Checked both layouts: M1 20 bits over 3 bytes, M3-L 84 over 11, M3-M 68
  over 9. Bit 83 readable, bit 84 not. No off-by-one.
  `full_capacity_payload_needs_no_terminator` (M1, exactly 20 bits) and
  `m3_numeric_fills_the_half_codeword_exactly` (68) pin both ends.
- **Terminator.** The "numeric indicator + zero character count" unification is
  arithmetically exact, and I checked all four versions against ISO Tables 2 and 3
  rather than taking the claim: M1 `0+3=3`, M2 `1+4=5`, M3 `2+5=7`, M4 `3+6=9` —
  matching `terminator_bits()` 3/5/7/9. The two `remaining() < …` guards handle the
  abbreviated terminator and the no-terminator full-capacity case. Pad codewords
  are unreachable because a non-full payload always carries a terminator first.
  Three shapes pinned by test.
- **Termination.** Every loop iteration consumes at least 4 bits (`count == 0` on
  Numeric breaks; a zero-count non-numeric segment still consumes its header). No
  infinite loop reachable.

### 2. RS gating — correct, and the caps are right

I did not take the `r` values on trust. Page-targeted read of the PDF, Table 9
(p. 34) and the ISO 7.5.1 `p` list on the same page:

| Symbol | ISO `(c, k, r)` | ISO `p` | Core |
| --- | --- | --- | --- |
| M1 | (5,3,0) | 2 | 5/3/2, r=0, p=2 ✓ |
| M2-L | (10,5,1) | 3 | ec 5, r=1, p=3 ✓ |
| M2-M | (10,4,2) | 2 | ec 6, r=2, p=2 ✓ |
| M3-L | (17,11,2) | 2 | ec 6, r=2, p=2 ✓ |
| M3-M | (17,9,4) | 0 | ec 8, r=4, p=0 ✓ |
| M4-L | (24,16,3) | 2 | ec 8, r=3, p=2 ✓ |
| M4-M | (24,14,5) | 0 | ec 10, r=5, p=0 ✓ |
| M4-Q | (24,10,7) | 0 | ec 14, r=7, p=0 ✓ |

All eight exact. `ec = 2r + p` holds for every row, and Formula (3) `2t ≤ d − p`
reproduces every `r`. The Table 9 footnote **a** — *"The error correction capacity
is less than half the number of error correction codewords to reduce the
probability of misdecodes"* — is printed against M1, M2-L, M2-M, M3-L and M4-L.
So the `fixed > error_correction_capacity` cap is **ISO-mandated, not drift**. Bob
called it "a deliberate addition"; it is in fact required conformance, and I am
recording it as such.

**M1 detection-only** is correctly implemented: `error_correction_capacity == 0`
takes a branch that recomputes the EC codewords with `rs_encode` and compares.
`rs_correct` is never reachable for M1. `errors_corrected` is hard `0`.
`m1_rejects_any_damage_without_correcting` flips all 36 data modules one at a time
and asserts 36 rejections — a naive `rs_correct` would have "recovered" every one
of them inside `ec/2 = 1`. This is the right test and it is exhaustive.

The half-codeword high-nibble guard (`micro.rs:214`) is a sound corollary: a true
4-bit codeword can never carry a high nibble, so one after correction is a
mis-correction. Only reachable on the `rs_correct` path (the M1 branch leaves the
block untouched), which is correct.

### 3. Mirror retry — no double-accept

`decode_micro` returns the straight-orientation result whenever it is `Ok`; the
transpose is attempted only on failure, and its error is discarded in favour of
the original (`map_err(|_| first)`). A symmetric matrix therefore cannot be
accepted twice — the straight decode wins deterministically, and a matrix that
decodes both ways yields the same payload by construction. Byte-identical in shape
to `decode` at `lib.rs:105`, so the two paths cannot drift. `mirrored_symbols_decode`
covers all 8 version/level pairs, `mirrored_figure_i4_decodes` covers ISO ground
truth. Error precedence matches the QR path.

### 4. Refactor safety — QR behaviour unchanged

I diffed the four extracted helpers against the code they replaced, line by line,
rather than resting on the 7 green tests. All four are verbatim moves:

- `push_numeric` — same 10/7/4-bit split, same `>999`/`>99`/`>9` range rejects.
- `push_alphanumeric` — same 11/6-bit split, same `>= 45*45` and `>= 45` rejects.
- `push_byte` — unchanged.
- `push_kanji` — **checked specifically, as asked.** `((v / 0xC0) << 8) | (v % 0xC0)`
  then `+= if sjis < 0x1F00 { 0x8140 } else { 0xC140 }` is character-for-character
  the original. The `< 0x1F00` threshold, the `0xC0` radix and the byte push order
  are all preserved. No semantic change.

Call sites now read the count then delegate; the only structural change is `count`
losing its `mut` where the helper owns the loop. `tests-integration` green and
`m4_kanji_roundtrip` exercises `push_kanji` through the new path with one character
from each SJIS range, which is better coverage than the QR suite had.

`BitReader::new` is unchanged in behaviour (`bits = data.len() * 8`), and
`with_bit_len` is `micro`-gated so the non-micro build carries no dead code.

### 5. Robustness — clean

- **No panic paths.** Character counts are bounded by their indicator width (≤ 63),
  so no unbounded allocation. Over-long counts exhaust the reader and surface as
  `Malformed` (`overlong_character_count_is_malformed`). `MicroMode::ALL.get()` is
  fallible-indexed, so M4 indicators 4–7 give `Malformed`, not a panic
  (`unknown_mode_indicator_is_malformed`). `1 << (width - 1 - bit)` cannot underflow
  because `bit` resets at `width`. `BitMatrix::get` returns `false` out of bounds
  and the iterator is version-matched anyway. `block[data_len - 1]` is only reached
  under `has_half_final_data_codeword()`, where `data_len ≥ 3`.
- **Garbage.** `random_grids_are_rejected_cleanly` (1 200 grids, 300 per version)
  and `blank_and_solid_grids_are_rejected` both pass, and the former asserts
  `format_survived > 100` — so it genuinely proves RS is the gate rather than BCH
  silently doing the rejecting. That assertion is the difference between a real
  test and a comfortable one, and I am glad it is there.
- **Feature discipline.** `micro = ["qr-core/micro"]`, `default = ["micro"]`.
  Nothing sets `default-features = false` on a sibling; unification stays additive.
  Test file is `#![cfg(feature = "micro")]` at file scope, so
  `--no-default-features` compiles it out entirely rather than leaving a husk.
  Both configs clippy-clean.
- **no_std.** `alloc` only. No `std` leak.

### 6. Test adequacy — the fixture is genuinely pinned

This was the question worth asking, and the answer holds up. There are **two**
independent anchors, not one:

1. `fixture_builder_reproduces_figure_i4` runs the *same* `fixture()` entry point
   every derived test uses and compares module-for-module against the hand
   transcribed Figure I.4. Nothing bypasses it.
2. `annex_i3_data_codewords` pins `[0x40, 0x18, 0xAC, 0xC3, 0x00]` against ISO
   Annex I.3.2. I hand-derived that stream myself — indicator `0`, count `1000`,
   `012`→10 bits, `345`→10 bits, `67`→7 bits, 5-bit terminator, 3 pad bits — and
   got the same five bytes. So the builder is anchored to ISO independently of the
   decoder, and the ASCII transcription is anchored independently of the builder.
   Neither can drift without the other catching it.

`build_symbol` also asserts `placed == version.data_modules()` and
`index == block.len()`, so a fixture cannot quietly under-fill.

---

## Should Fix — MINOR

- **MINOR — `tests/micro.rs`, corruption coverage is uneven across levels.** M3-M
  (`r = 4`) and M4-M (`r = 5`) have **no** corruption test at all, and M2-M and
  M4-Q have a correctable case but no uncorrectable one. Full matrix today: M1 ✓/✓,
  M2-L ✓/✓, M2-M ✓/✗, M3-L ✓/✓, M3-M ✗/✗, M4-L ✓/✓, M4-M ✗/✗, M4-Q ✓/✗. The
  mechanism is level-independent and well covered elsewhere, so this does not block —
  but the missing two are precisely the `p = 0` levels where the cap equals `d/2`
  and the *generic* bound is the live one, which is the arm least exercised by the
  existing tests. Add an `r`-and-`r+1` pair for M3-M and M4-M, and an `r+1` case for
  M2-M and M4-Q, to close the table.

- **MINOR — `src/micro.rs:270`, zero-count segments in non-numeric modes are
  silently accepted.** `count == 0` short-circuits to the terminator break only for
  `MicroMode::Numeric`. An alphanumeric/byte/kanji segment with `count == 0`
  consumes its header, emits nothing and continues. It cannot arise from a
  conforming symbol, it terminates, and it cannot panic — but it is a stream shape
  ISO does not define, and accepting it quietly is a small hole in an otherwise
  strict parser. Recommend rejecting it as `Malformed`, or documenting the
  tolerance deliberately.

- **MINOR — `src/micro.rs:161-174`, a short placement iterator zero-fills in
  silence.** If `micro_data_modules` ever yielded fewer modules than the block
  needs, `read_block` would return a partly-zero block and let RS reject it
  downstream. That is safe today and the RS gate catches it, but the test-side
  `build_symbol` asserts its module count and the decode side does not. A
  post-walk `index == total` check costs nothing and turns a future core
  regression into a named error instead of a confusing `BlockCorrection`.

- **MINOR — `src/micro.rs:102`, mirror retry runs on `InvalidSize`.** A matrix of
  the wrong size is transposed and re-walked, allocating a whole `BitMatrix` for a
  decode that is guaranteed to fail at the same first line. Cheap to skip. I note
  it only because it is a wasted allocation on the hot rejection path in a
  `no_std` crate; it does exactly mirror `decode` at `lib.rs:105`, and consistency
  with the QR path is a fair reason to leave it alone. Builder's call.

---

## Escalate to Architect

1. **The misdecode-protection cap is missing from the QR path, and that is a real
   conformance gap — not merely an inconsistency.** Bob flagged this and was right
   to. ISO Table 9 footnote **a** marks versions 1-L, 1-M, 1-Q, 1-H, 2-L and 3-L as
   having `r` below `ec/2`, exactly as the micro symbols do. `extract.rs:176` uses
   the generic `d/2` bound with no cap, so a version 1-L symbol (published `r = 2`,
   `p = 3`, `d/2 = 3`) will today accept a 3-error "correction" that ISO requires be
   rejected as a probable misdecode. Out of scope for 5c and correctly left
   untouched, but it should become its own step. This is the one finding here with
   customer-visible consequences — a wrong payload returned as a confident success.

2. **Half-codeword nibble convention vs Step 5b.** Not re-litigated per your
   ruling; recording that HIS side is internally consistent and ISO-grounded.
   `read_block` packs the 4 bits into the low nibble, RS runs over that byte,
   `data_bit_stream` shifts them back up, and the high-nibble guard enforces the
   invariant after correction. That is the ISO Annex N.1 zero-prefix convention
   (*"prefixed with 0000"*) applied consistently end to end. If Bob-E chose the
   high nibble, 5b moves — Bob-MD's side is the one with the citation behind it.
   Resolution at 5e as directed.

Noted, not escalated: `DecodeError` reuse — accepted per your pre-ruling, and the
per-variant micro semantics are documented on `decode_micro` where a reader will
find them. Correct call.

---

## Cleared

The full 5c pipeline — size gating, single-strip format read with the
symbol-number cross-check, unmasking and extraction through the core iterator, the
bit-length-limited reader at both half-codeword layouts, M1 detection-only
rejection, the ISO-mandated `r` cap, mirror retry, and the shared-helper refactor
of `segment.rs` — reviewed against the ISO PDF and found correct. The EC table was
verified module by module against Table 9 and the 7.5.1 `p` list, not taken on
trust. Nothing here ships broken. Four MINORs, none blocking. Good work, Bob.
