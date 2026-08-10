# Review Feedback — Step 2a: qr-encode

**Verdict: Step 2a CLEAR**

Date: 2026-08-10
Reviewer: Richard-E
Ready for Builder: YES

Counts: **0 BLOCKER, 0 MAJOR, 5 MINOR**

Gates (all green): `cargo test -p qr-encode` 21 passed (10 unit + 11 integration) ·
`cargo check -p qr-encode` · `cargo clippy -p qr-encode --all-targets -- -D warnings` ·
`cargo fmt --check`.

---

## Ruling on the N3 interpretation (Bob's flagged uncertainty)

**Bob is correct. Score-once is the conformant reading. Adopt it; close the uncertainty.**

Verified independently against the ISO text (`ISO_IEC_18004_2024(en).pdf`, 7.8.3.1, Table 11
and NOTE 3, p.49–50) — not by deferring to the Annex I result:

- **Table 11**, row 3: feature `1:1:3:1:1 ratio (dark:light:dark:light:dark) pattern in a row
  or column, preceded or followed by light area 4 modules wide`; **Evaluation condition:
  "Existence of the pattern"**; Points: `N3` (flat 40).
  The scoring unit is the *existence* of the pattern, not the number of qualifying sides.
- **NOTE 3**: "If the light area of more than four module width exists **after or before** a
  1:1:3:1:1 ratio pattern, **the imposed penalty is 40 points**." Disjunctive condition,
  singular penalty. A per-side reading would have to render this as "40 points per side",
  which the text does not say.

Corroboration (independent of the textual argument): ISO Annex I I.2.5 states "The data masking
pattern selected is referenced **010**" for the v1-M "01234567" symbol. Bob's implementation
reproduces mask 2; the both-sides double-count selects mask 0. Two independent lines of
evidence agree. The Nayuki both-sides count is the deviation here, not the reference.

**N3 window scanning — no missed or double-counted patterns (n=1).** I extracted
`FinderHistory`/`finder_penalty` into a standalone harness and probed the cases that could
plausibly break it. All correct:

| Case (n=1) | Result | Expected |
|---|---|---|
| 4-light before, dark tail | 40 | 40 |
| 4-light after only, short (1-module) light before | 40 | 40 |
| Pattern flush at START of line (quiet-zone lead) | 40 | 40 |
| Pattern flush at END of line (quiet-zone trail) | 40 | 40 |
| 4-light on **both** sides | **40** | 40 (not 80) — score-once holds |
| Two distinct patterns in one line | **80** | 80 (40 each) — per-occurrence, correct |

The `runs[0] == 0` empty-history sentinel is sound: the only zero-length run ever pushed is
the leading one at index 0, and every later push is `>= 1`. The end-of-line flush (dark run,
then trailing light merged with the border) counts the final window exactly once. Overlapping
patterns cannot collide because each distinct core terminates at a distinct light-run push, so
`count()` is evaluated once per candidate window.

**N4 dark-ratio step math — exact.** `k = ceil(|20·dark − 10·total| / total) − 1`, ×10.
Verified against NOTE 4 including boundary inclusivity: 50%→0, 45%→0, 44%→10, 40%→10,
39%→20, 55%→0. Matches "0 points if between 45% and 55%, 10 points if between 40% and 60%".
No overflow: max `dark·20` = 626 580, well inside u32.

---

## Must Fix

None.

---

## Should Fix (MINOR — none block the step)

- **MINOR-1 — `matrix.rs:229–236` — N3 opposite-side guard suppresses scaled (n≥2) patterns.**
  `lead` requires `r[0] >= 4n && r[6] >= n`; `trail` requires `r[6] >= 4n && r[0] >= n`. The
  trailing `>= n` term on the *opposite* side has no basis in ISO — the standard conditions on
  one side only. Confirmed miss: a scaled n=2 pattern with an 8-module light area **after** but
  only a 1-module light run **before** scores 0 where ISO's disjunction says 40.
  Harmless at n=1 (where `>= n` degenerates to `>= 1`, i.e. "history is populated" — which is
  in fact why it was needed), and n=1 is the only width that occurs in practice. Recommend
  replacing the `>= n` guards with an explicit history-populated check (e.g. a `pushes: u8`
  counter, or `r[6] > 0`) so the ISO disjunction is expressed exactly.

- **MINOR-2 — `matrix.rs:232–234` — scaled-vs-absolute light-area width is an unflagged
  interpretation.** The code requires the light margin to be `4n` wide. Table 11 says the
  pattern is a *ratio* but the light area is "**4 modules** wide" (absolute), and NOTE 3
  likewise says "four module width". So a n=2 pattern followed by a 4-module light area scores
  0 here, 40 under a strict-absolute reading. Genuinely ambiguous in the standard, coincident
  at n=1, and affects mask *selection* only — never symbol validity. Worth a code comment
  recording the choice rather than a behaviour change.

- **MINOR-3 — `matrix.rs:291` — `expect()` in a library path.** `best.expect("eight masks
  always yield a candidate")` is provably unreachable (the loop always runs 8 iterations), but
  it is the crate's only panic site under a `no panics on malformed input` brief. Restructure to
  seed `best` from mask 0 outside the loop, or fold with an unconditional first assignment, so
  the type system carries the invariant.

- **MINOR-4 — `bits.rs:52` — `push_bits` silently truncates.** Documented as "`count` must be
  `<= 32`; higher bits of `value` are ignored", but `count > 32` shifts `value: u32` by ≥32,
  which is a panic in debug and a no-op-ish wrap in release. No caller can reach it (max count
  is 16), so this is documentation-only hardening — a `debug_assert!(count <= 32)` would pin it.

- **MINOR-5 — `segment.rs:130,135` — `alnum_value(...).unwrap_or(0)` masks a contract
  violation.** Unreachable because `write_payload` is only called after `mode_supports`/
  `select_mode` validation, but silently encoding a non-alphanumeric byte as `0` would corrupt
  a payload rather than fail loudly if that invariant is ever broken. Same for the `- b'0'`
  arithmetic in the numeric arm (wraps on non-digit input rather than erroring). Consider making
  the validated-charset invariant explicit at the function boundary.

---

## Verified correct (no action)

Checked directly, all pass:

- **Mode auto-select boundaries.** Numeric ⊂ alnum ⊂ byte precedence correct; the 45-character
  alnum set matches ISO Table 5 exactly (`0-9 A-Z SP $ % * + - . / :` — lowercase correctly
  excluded, so `"abc"` falls to byte). Empty input → byte, encodes and decodes.
- **Char-count indicator widths.** Exhaustively proved over all 40 versions × 4 EC levels × 3
  modes that the maximum character count admitted by the capacity check always fits the
  indicator width — the truncation in `write_char_count` is unreachable by construction.
- **Terminator + bit padding + pad bytes.** Terminator clamped to remaining capacity
  (`min(4, cap - len)`), zero-fill to byte boundary, then alternating `0xEC`/`0x11`. Exact match
  to the ISO Annex I 16 data codewords.
- **Version auto-select, off-by-one at exact capacity.** `bits <= capacity` is the correct
  comparison. Tested payloads sized to *exactly* fill v1/v2/v7/v20/v40 at L and H: all encode,
  all roundtrip, and auto-selection never overshoots to the next version. One byte over →
  clean `DataTooLarge`, no panic.
- **Block split + interleave order.** Data interleaved column-wise across group1+group2, then EC
  column-wise across all blocks. Matches ISO Annex I EC codewords; v5-H (2-group, 4-block)
  round-trips through an independent deinterleave + `rs_correct` with 0 corrections.
- **Zigzag placement + col-6 skip.** Column pairs walk right→left; `right == 6` remaps to 5 so
  the timing column is excluded and pairs run (20,19)…(8,7),(5,4),(3,2),(1,0). Direction
  `((right+1) & 2) == 0` alternates correctly *and continues alternating across the skip*, which
  is the standard's behaviour. Now verified against an independent decoder, not just self-consistently.
- **Mask XOR restricted to non-function modules** via core `is_function_module` — single source
  of truth shared with decode, as the brief requires.
- **Format info: both copies + dark module.** Copy 1 (L around top-left finder), copy 2 (split
  top-right/bottom-left), dark module at `(8, size-8)`. No overlap between the copy-2 loops and
  the dark module. Read back correctly by the independent decoder across all 8 masks × 4 levels.
- **Version info v≥7, both blocks, orientation.** `(size-11+i%3, i/3)` top-right and its
  transpose bottom-left. Verified at v7/v8/v10/v14 through `qr_decode`, which *rejects* a symbol
  whose version-info field contradicts its geometry — so a mis-oriented block would fail loudly.
- **Options handling.** Forced version too small → `DataTooLarge`. Forced mask honoured and
  read back for all 8. Forced mask > 7 → `InvalidMask`. Forced mode incompatible with data
  (`Numeric`/`Alphanumeric` on `"abc"`, `Kanji`, `Eci`) → `ModeMismatch`, error not panic.
- **N1** `3 + (run-5)`, matching NOTE 1's "seven consecutive → 5, not 12" no-double-count rule.
  **N2** counts every 2×2 window at 3 points, matching NOTE 2's 3×3 → 4 windows × 3 = 12.
- **Mask tie-break.** Strict `<` replacement over ascending mask ids ⇒ lowest id wins ties.
- **no_std / deps / safety.** No `std::` outside `#[cfg(test)]`; `#![forbid(unsafe_code)]`;
  zero dependencies beyond `qr-core`. `cargo check` green. Core primitives reused throughout —
  no reimplementation of RS, BCH, mask predicates or the function-module map, as briefed.

---

## Reviewer-added tests (per Arch's cross-check authorisation)

Added `crates/qr-encode/tests/roundtrip.rs` (11 tests) and a `[dev-dependencies]`
`qr-decode = { workspace = true }` in `crates/qr-encode/Cargo.toml`. **No root `Cargo.toml`
change** — `qr-decode` was already in `workspace.dependencies`. No source file modified;
no other crate touched.

Rationale — this closes the gap flagged in the review brief. Bob's in-crate tests extract
codewords with `extract_interleaved`, a *copy of the encoder's own zigzag walk*. A uniformly
shifted, transposed or mis-stepped placement would corrupt the symbol yet still pass those
tests, because writer and reader share the defect. Routing finished symbols through the
independently-implemented `qr_decode::decode` removes that shared-mode blind spot. It also
recruits decode's version-info cross-check as a free oracle for version-block orientation.

Coverage: byte payload × 4 EC levels (auto mask); v5-H multi-block interleave near capacity;
all 8 forced masks; Annex I numeric with mask-2 assertion; v7/8/10/14 version-info symbols;
empty payload; exhaustive char-count-indicator bound; exact-capacity fit and no-overshoot;
one-byte-over clean error; incompatible forced modes; invalid mask.

Answering the brief's question directly: **the v1–40 smoke test does more than assert
absence of panic** — it checks matrix size and decodes format info back through the core BCH
path. But it is *not* sufficient to catch a shifted zigzag, since it never reads the payload
back. The new roundtrip tests are what close that hole.

---

## Escalate to Architect

None. Both open interpretation points (MINOR-1, MINOR-2) are confined to scaled n≥2 finder
patterns, affect mask selection only, and cannot render an invalid or undecodable symbol.

---

## Accepted gaps (non-blocking, per brief)

Single-mode segmentation (no greedy mixed-mode); no Kanji, no ECI encoder-side. Both are
documented in the crate docs and `REVIEW-REQUEST-encode.md`.

## Cleared

Full ISO/IEC 18004 encode data path in `crates/qr-encode/` — mode selection, bit-stream
assembly, version selection, RS block split and interleave, zigzag placement, all 8 masks with
ISO 7.8.3 N1–N4 penalty scoring, and format/version information placement — reviewed against
the ISO text and verified end-to-end through an independent decoder. Correct and ready to commit.
