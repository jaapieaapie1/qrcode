# Review Feedback — Step 5b: qr-encode Micro QR encoder

Reviewer: Richard-ME
Date: 2026-08-10
Scope reviewed: `crates/qr-encode/` only (`src/micro.rs`, `tests/micro.rs`, `Cargo.toml`,
`src/lib.rs`). `crates/qr-detect` ignored entirely per instruction, including its
in-flight clippy failure.

**Verdict: Step 5b CLEAR**
**Ready for Builder: YES**
BLOCKER: 0 — MAJOR: 0 — MINOR: 4

---

## Must Fix (BLOCKER)

None.

---

## Should Fix (MAJOR)

None.

---

## Should Fix (MINOR)

- **`crates/qr-encode/tests/micro.rs:193-195` — stale comment contradicts the Arch
  ruling and the code.** The comment reads "it is carried in the high nibble for the
  Reed-Solomon step (7.4.11)". That is the pre-ruling zero-*suffixed* description. The
  implementation, the sibling test at line 219, the module docs at `src/micro.rs:204-214`
  and the committed decode side all say zero-*prefixed* (low nibble, high nibble clear).
  A future reader could "correct" the code to match this comment. Fix: rewrite those
  three lines to say the 4 bits reach Reed-Solomon in the low nibble as a value in
  `0..=15`, and that only those 4 bits are written into the symbol.

- **`crates/qr-encode/tests/micro.rs:67-84` — the test's `evaluation_score` is a
  verbatim copy of `src/micro.rs:354-367`, not an independent transcription.** Its doc
  comment claims it is "transcribed from the clause independently of the encoder". It
  is character-for-character the same loop bounds, the same exclusion set and the same
  comparison. It can catch an argmax or tie-break regression; it cannot catch a wrong
  edge or wrong exclusion definition, because both sides would move together. Fix:
  either drop the "independently" claim, or pin one hand-computed `(S1, S2, Es)` triple
  from a known matrix so the edge definition itself is asserted.

- **REVIEW-REQUEST §6 overstates the Figure I.4 evidence for the edge definition.**
  The request says reproducing Figure I.4 "is the real proof that the edge definition
  and the min×16 + max, highest-wins formula are right, because getting either wrong
  changes the chosen mask". I tested that claim directly (see Verification 1 below).
  Highest-wins *is* proved by Figure I.4. The **edge definition is not**: I recomputed
  the four mask scores for the printed symbol under five different edge/exclusion
  readings and all five select mask `01`. No fix needed in code — the definition is
  correct, I verified it against the clause text — but the claim should not be carried
  forward into BUILD-LOG or the 5e brief as if it were covered by a test.

- **No micro equivalent of the QR path's `char_count_never_overflows_indicator`.** The
  character count is written with `push_bits(data.len() as u32, count_bits)`, which
  truncates silently if the length ever exceeded the field. I hand-checked all nine
  (version, mode) pairs and the Table 7 data-bit capacity check in `resolve` is strictly
  tighter than the count field in every one, so it is unreachable today. It is an
  unasserted invariant on a silent-corruption path. Fix: one loop asserting
  `capacity < 2^count_bits` for every (version, mode).

---

## Verification performed

### 1. ISO 7.8.3.2 mask evaluation — read from the PDF, clause and figure

Read printed p.50 (7.8.3.2) and p.51 (Figure 24 key) directly. Clause text:

> "count the number of dark modules in the right and lower edges of the symbol
> (excluding the final module of the timing pattern) … S1 is the number of dark modules
> in the right side edge; S2 is the number of dark modules in the lower side edge …
> The data mask pattern which results in the highest score shall be selected."

- **Formula**: `Es = min(S1,S2)*16 + max(S1,S2)`, highest wins. `src/micro.rs:366`
  implements exactly this. **Correct.**
- **Orientation**: `s1` reads column `size-1` (right edge), `s2` reads row `size-1`
  (lower edge) — `src/micro.rs:359,362`. Matches the clause. Note for the record: the
  formula is **symmetric in S1 and S2** (`min*16+max` either way), so a swapped
  implementation is mathematically indistinguishable, not merely lucky. The swap
  concern is provably immaterial here.
- **Exclusion**: the two timing patterns are row 0 and column 0. Their final modules
  are `(size-1, 0)` on the right edge and `(0, size-1)` on the lower edge. The loop
  `for i in 1..size` excludes exactly those two and no others; the shared corner
  `(size-1, size-1)` counts in both. **Correct** per the clause.
- **Discriminating power of Figure I.4** — I reconstructed the printed M2-L symbol from
  the test fixture, rebuilt the M2 function map (89 function modules, 80 data modules,
  matching 40 data bits + 5 EC codewords), remasked with each of the four micro masks
  and scored under five edge readings:

  | reading | scores (masks 0-3) | selected |
  |---|---|---|
  | as implemented (`1..size` both edges) | 72, 136, 136, 68 | 1 |
  | whole edges, nothing excluded | 89, 153, 153, 85 | 1 |
  | exclude the corner instead | 72, 136, 136, 68 | 1 |
  | exclusion on right edge only | 72, 136, 136, 68 | 1 |
  | exclusion on lower edge only | 72, 136, 136, 68 | 1 |

  All five pick mask `01`. So Figure I.4 does not test the exclusion rule (MINOR
  above). It **does** test the direction: lowest-wins would pick mask 3 (68), and it
  **does** support decision §5 — masks 1 and 2 tie at 136 and ISO itself selects `01`,
  so the lowest-id tie-break is ISO-consistent, not merely an arbitrary house rule.
  That is worth recording; ISO leaves the tie unspecified.
- **Mask chosen before format information is written**: sound. Format modules occupy
  column 8 and row 8; the smallest symbol is 11 across, so `size-1 >= 10 > 8` and no
  format module lies on either scored edge. Also confirmed the scored edges contain no
  function modules at all once row 0 / column 0 are excluded.

### 2. Bit stream — every field checked against the PDF

Read printed p.21 (Tables 2 and 3), p.29 (7.4.10, 7.4.11), p.30 (Table 7), p.41 (7.6
note).

- **Mode indicator widths**, Table 2: M1 = 0, M2 = 1, M3 = 2, M4 = 3 bits. Sourced from
  `qr_core::micro` (committed, trusted); verified end-to-end by my hand computations
  below, which reproduce the encoder's pinned bytes exactly.
- **Character count widths**, Table 3: M1 numeric 3; M2 numeric 4 / alnum 3; M3 numeric
  5 / alnum 4 / byte 4; M4 numeric 6 / alnum 5 / byte 5. Confirmed against the printed
  table.
- **Terminator lengths**, Table 2: M1 000 (3), M2 00000 (5), M3 0000000 (7), M4
  000000000 (9). `src/micro.rs:235-241` uses
  `min(terminator_bits, data_bits - buf.len())`, which is exactly 7.4.10's "omitted if
  the data bit stream completely fills the capacity … or abbreviated if the remaining
  capacity … is less than the required bit length". Both branches are exercised: the
  M1 `"12345"` case omits it entirely (20 bits exactly), the 23-digit M3-L case omits
  it, and `"1234"` in M1 lands on exactly 3 bits remaining so it is written in full.
- **Pad codewords for micro**: 7.4.11 states the `11101100` / `00010001` alternating
  rule for the whole document, with no micro carve-out; the only micro-specific
  sentence is the `0000` half pad. `src/micro.rs:249` starts with `0xEC` and alternates
  — **correct**, and it is the same rule as QR, not a micro-specific sequence.
- **M1/M3 half codeword padding when data is short**: 7.4.11 — "The pad codeword used
  in the final data symbol character position in micro QR code versions M1 and M3
  symbols shall be represented as 0000". `src/micro.rs:258-260` zero-fills to
  `data_bits`, which produces both the padding-bit rule and the `0000` half pad from
  one loop. The `buf.len() <= full_codeword_bits` guard at line 243 is correct in every
  case I could construct: below the boundary it byte-aligns then pads codewords; at the
  boundary both loops no-op; above the boundary (payload running into the half
  codeword) it correctly skips straight to the trailing zero fill.
- **Version auto-select with mode-availability interaction**: `resolve`
  (`src/micro.rs:177-201`) iterates M1→M4 and re-selects the mode per version, which is
  required because availability widens with version. I checked the ordering assumption
  that a more compact mode is never more expensive: numeric beats alnum beats byte at
  every micro version even accounting for the differing count widths (worst case, 1
  digit in M3: numeric 2+5+4 = 11 bits, byte 2+4+8 = 14). So most-compact-available is
  also smallest. The `UnsupportedMode` vs `DataTooLarge` split via `mode_available` is
  right and gives the honest error for `b"a"` at `Detection`.
- **Table 7 capacities**: the test's 8-row table matches the printed table exactly
  (M1 5/—/—; M2-L 10/6/—; M2-M 8/5/—; M3-L 23/14/9; M3-M 18/11/7; M4-L 35/21/15;
  M4-M 30/18/13; M4-Q 21/13/9).
- **Single RS block**: confirmed at printed p.40, "NOTE Micro QR code symbols consist
  of a single block", and 7.6 a) "(or a single block for micro QR code symbols)". No
  interleaving path is correct.

### 3. Half codeword — pinned bytes recomputed by hand, independently

M1 `"12345"`, mode indicator 0 bits, count 5 in 3 bits = `101`, `"123"` → 10 bits
`0001111011`, `"45"` → 7 bits `0101101`. Total 3+10+7 = 20 = the M1 data capacity, so
the terminator is omitted.

```
101 0001111011 0101101
-> 10100011 11011010 1101
-> 0xA3, 0xDA, half = 0b1101
```

Zero-prefixed the third codeword is `0x0D`. **`[0xA3, 0xDA, 0x0D]` confirmed.**

Reed-Solomon by hand, GF(256) mod `0x11D`, generator for 2 EC codewords
`g(x) = x^2 + 3x + 2`:

```
[A3, DA, 0D, 0, 0]
A3*2 = 0x5B, A3*3 = 0xF8  ->  [_, 0x22, 0x56, 0, 0]
22*2 = 0x44, 22*3 = 0x66  ->  [_, _, 0x30, 0x44, 0]
30*2 = 0x60, 30*3 = 0x50  ->  remainder 0x14, 0x60
```

**`[0x14, 0x60]` confirmed.** I also recomputed the zero-*suffixed* alternative,
`rs_encode([0xA3, 0xDA, 0xD0], 2)` = `[0x6E, 0xC7]`, matching the value the test asserts
it must differ from. The discriminating test is genuinely discriminating.

I hand-checked the other two pinned streams as well:

- M1 `"1234"`: `100 0001111011 0100 000` → `[0x83, 0xDA, 0x00]`. Confirmed. (As Bob
  notes, this one cannot discriminate the nibble convention.)
- M3-L `b"hi"`: `10 0010 01101000 01101001` + 7-bit terminator + 3 padding bits = 32
  bits → `0x89, 0xA1, 0xA4, 0x00`, then 6 pad codewords `EC 11 EC 11 EC 11` to reach 80
  bits, then the `0000` half pad. `[0x89,0xA1,0xA4,0x00,0xEC,0x11,0xEC,0x11,0xEC,0x11,0x00]`,
  11 codewords as Table 7 requires. Confirmed.

### 4. Roundtrip coverage against qr-decode

**The 11 tests in `tests/roundtrip.rs` are the pre-existing QR tests. None of them is a
micro test — `tests/roundtrip.rs` contains no reference to micro at all, and
`tests/micro.rs` never touches `qr-decode`.** So M1, the half-codeword crate-crossing
case, and the 8 version×EC symbols have **no** encode→decode coverage in this step. Every
micro assertion is made against the test file's own `extract_bits` / `to_codewords`
helpers, which mirror the encoder's conventions; the sole independent ground truth is
Figure I.4, which is M2-L and therefore has no half codeword.

This is **not** a Step 5b defect: the brief assigns the encode↔decode sweep to Step 5e,
and Step 5c is explicitly required to be tested "independent of 5b". I am not raising it
as a finding against Bob-ME. But I did not want the contract left unverified, so I
verified it out of band:

- Read `crates/qr-decode/src/micro.rs:139-146, 152-158, 212-216, 225-236`. The decoder
  packs the 4-bit final codeword into the **low** nibble and rejects a corrected value
  with a non-zero high nibble. Same convention as the encoder. The contract holds by
  inspection.
- Built a throwaway harness outside the repo (scratchpad, nothing added to the
  workspace) depending on qr-core + qr-encode + qr-decode and ran
  `encode_micro` → `decode_micro` over all 8 symbols × 5 mask settings (auto + 0-3) ×
  16 payloads, plus the fully-automatic version-selection path across all 4 EC
  selections. Payloads included the empty payload, M1 half-codeword cases `"1234"` and
  `"12345"`, the M3 pad-half case `b"hi"`, the M3 exact-fill 23-digit case, binary
  `[0x00, 0xFF, 0x80]`, and the M4 35-digit boundary.

  **466 roundtrips, 0 failures** — payload bytes, version, EC selection and mask all
  recovered exactly. The half-codeword convention is confirmed live across the crate
  boundary, not just by inspection.

**For Arch:** Step 5e must still land these as real tests in-tree. My harness is
throwaway and is not committed.

### 5. Feature discipline, no_std, unsafe, panics

- `default = ["micro"]`, `micro = ["qr-core/micro"]`. Polarity matches the Wave 4 lock.
  Nothing in `crates/qr-encode/Cargo.toml` or the workspace `Cargo.toml` sets
  `default-features = false` on a sibling (the only such line in the workspace is on the
  third-party `image` crate). Feature unification stays additive.
- `#![no_std]` and `#![forbid(unsafe_code)]` unchanged in `lib.rs`; `micro.rs` adds no
  `std`, no `unsafe`, no new dependency, and only `alloc::vec::Vec`.
- `tests/micro.rs` is file-gated with `#![cfg(feature = "micro")]` and compiles to 0
  tests with the feature off — observed.
- Panic audit of `src/micro.rs`: no `unwrap`, `expect`, `panic!`, slicing or arithmetic
  that can overflow. The two index sites are `pad[i % 2]` (trivially in range) and
  `stream[n / 8]` in `place_data`. On Bob's open question 2 — I checked the invariant
  holds for all eight symbols: `data_bits + 8 * ec_codewords == data_modules` gives
  36/80/80/132/132/192/192/192, and `build_module_stream` produces
  `ceil(data_modules / 8)` bytes, so the maximum index `(data_modules - 1) / 8` is
  always in range. It is asserted per-symbol by `all_four_masks_can_be_forced` and
  `m1_bit_stream_and_half_codeword`. **Leave the indexing as it is** — a loud failure
  on a broken internal invariant is the right call and `unwrap_or(0)` would silently
  emit a corrupt symbol.
- Clean error paths confirmed by test and by reading: over-capacity → `DataTooLarge`;
  forced version too small → `DataTooLarge`; non-numeric into M1 → `UnsupportedMode`;
  mask 4/7/8/255 → `InvalidMask`; all 8 invalid (version, EC) pairs from Table 13 →
  `UnsupportedEcLevel` (and that list is exhaustive: 16 pairs, 8 valid, 8 rejected).
  The character count can never overflow its field — see the MINOR above.

### 6. Commands run

| Command | Result |
|---|---|
| `cargo test -p qr-encode` | ok — 36 passed, 0 failed (10 unit + 15 micro + 11 roundtrip) |
| `cargo test -p qr-encode --no-default-features` | ok — 21 passed, 0 failed; `tests/micro.rs` = 0 tests |
| `cargo clippy -p qr-encode --all-targets -- -D warnings` | clean |
| `cargo clippy -p qr-encode --no-default-features --all-targets -- -D warnings` | clean |
| `cargo check -p qr-encode --no-default-features` | ok |
| `cargo fmt -p qr-encode -- --check` | clean |
| out-of-tree encode→decode harness, 466 micro roundtrips | 0 failures |

`crates/qr-detect` was not built, not read and not judged, per instruction.

---

## Escalate to Architect

Nothing requiring a ruling. Three things for the record:

1. **The lowest-id mask tie-break is ISO-consistent, not arbitrary.** In the Annex I.3
   symbol, micro masks `01` and `10` both score 136 and ISO selects `01`. Decision §5
   is now backed by the standard's own worked example. Worth putting in BUILD-LOG.
2. **Figure I.4 does not cover the 7.8.3.2 edge/exclusion definition** (five different
   readings all reproduce it). If that claim is carried into BUILD-LOG or the 5e brief
   as test coverage, it will be wrong. The definition itself is correct — I checked it
   against the clause.
3. **Step 5e must add the micro encode→decode sweep.** It does not exist anywhere in
   the workspace today. I verified it passes (466 cases, 0 failures) but only in a
   throwaway harness outside the repo.

Bob's open question 3 (`MicroOptions` has no `mode` field) is Arch's call and was
pre-ruled an accepted gap; I have nothing to add.

---

## Cleared

The Micro QR encoder in `crates/qr-encode/src/micro.rs` was reviewed against ISO/IEC
18004:2024 clauses 7.4.10, 7.4.11, 7.6, 7.8.3.2 and Tables 2, 3, 7 read from the PDF
this session, with the M1 and M3 bit streams and their Reed-Solomon codewords
recomputed by hand and the cross-crate half-codeword contract exercised live against
the committed qr-decode. Spec compliance, error paths, feature gating, `no_std` and
panic-freedom all pass. No drift from the Step 5b brief and no existing code path
touched. Four minor test-and-comment items, none blocking.
