# Review Feedback — Step 5e: micro integration suite

**Reviewer:** Richard-MI
**Date:** 2026-08-10
**Verdict:** **Step 5e CLEAR**
**Ready for Builder:** YES
**Counts:** 0 BLOCKER · 1 MAJOR · 8 MINOR

Scope reviewed: `tests-integration/tests/common/mod.rs` (+139), `micro_roundtrip.rs`,
`micro_image.rs`. Product crates are committed and were read only where a claim
had to be checked against them. This is a test-only review.

---

## Verification performed

```
cargo test   -p tests-integration                        81 passed, 0 failed, 0 ignored
cargo clippy -p tests-integration --all-targets -D warnings   clean
cargo fmt --all --check                                       clean
```

Per-suite, against the REVIEW-REQUEST table:

| suite | tests | wall | claimed |
|---|---:|---:|---:|
| `full_roundtrip` | 11 | 1.01 s | — |
| `fuzz_seeded` | 5 | 6.47 s | — |
| `matrix_roundtrip` | 11 | 17.65 s | 17.6 s |
| `micro_image` | 12 | 0.35 s | 0.36 s |
| `micro_roundtrip` | 15 | 0.02 s | 0.02 s |
| `multiscan` | 3 | 0.15 s | — |
| `video_session` | 9 | 0.18 s | — |
| `warped` | 15 | 0.23 s | — |

81 = 11+5+11+12+15+3+9+15. Counts and runtimes are accurate as stated. No
`#[ignore]`, no skipped binary.

Empirical probes were run in a temporary test file in `tests-integration/tests/`
which was deleted afterwards; `git status` on `tests-integration/` is back to
exactly the three files under review. Nothing of Bob's was edited.

### The 344 count — derived independently from the ISO PDF, Bob is right

Extracted Table 7 from `ISO_IEC_18004_2024(en).pdf` (printed p. 30) and summed
the micro rows over the content kinds the sweep actually drives (`ALL_CONTENT`
has four members, so byte is counted twice — binary and UTF-8; Kanji is not
tested and is a documented crate-level gap):

| symbol | data bits | num | alnum | byte | tested sum |
|---|---:|---:|---:|---:|---:|
| M1 | 20 | 5 | — | — | 5 |
| M2-L | 40 | 10 | 6 | — | 16 |
| M2-M | 32 | 8 | 5 | — | 13 |
| M3-L | 84 | 23 | 14 | 9 | 55 |
| M3-M | 68 | 18 | 11 | 7 | 43 |
| M4-L | 128 | 35 | 21 | 15 | 86 |
| M4-M | 112 | 30 | 18 | 13 | 74 |
| M4-Q | 80 | 21 | 13 | 9 | 52 |

Total **344**. I also re-derived every one of those capacities from the bit
budget (Table 7 data bits, Table 2 mode-indicator widths 0/1/2/3, Table 3 char
count widths) and all sixteen figures agree with the printed table. The pinned
count is correct and it is a genuine outside witness for `qr-core`'s
`data_bits` table. See MINOR-2 for the limit of that witness.

Table 9 (printed p. 34) also checked against `qr-core`'s `MICRO_SYMBOLS`:
`(c,k,r)` = M1 (5,3,0) p2 · M2-L (10,5,1) p3 · M2-M (10,4,2) p2 · M3-L (17,11,2)
p2 · M3-M (17,9,4) p0 · M4-L (24,16,3) p2 · M4-M (24,14,5) p0 · M4-Q (24,10,7)
p0. Exact match. v1-L is (26,19,2) p3, so the r-cap regression's "3 errors
exceed r = 2" is correct.

Module counts also reconcile: 8×8 finder+separator block + timing row/column +
15 format modules gives 85/89/93/97 function modules for M1–M4, leaving
36/80/132/192 — which equals data codewords × 8 (less 4 for the M1/M3 half) plus
EC codewords × 8 from Table 9. `codeword_ranges_tile_the_module_stream` and the
`mine.len() == data_modules()` assertion therefore do cross-check two
independently-verifiable ISO tables against each other. Real value, narrow scope.

### Envelope claims — probed, all honest

- **Scale floor 2 px/module.** Probed 1–3 px for all eight symbols: 1 px detects
  nothing for all eight; 2 and 3 px decode all eight byte-exactly. Bob's claim is
  exactly right and the sweep starts in the right place.
- **Quiet zone 2.** Probed 0–4: M1 needs at least 1 module of quiet zone, M2–M4
  detect at 0. `MICRO_QUIET = 2` (ISO 6.3.6) sits safely above the floor.
- **Rotation 0–45°.** Probed all eight symbols at 10 px/module with 60 px pad, 5°
  steps: **all eight pass every angle**, including M1. No hidden failure behind
  the sampling. See MAJOR-1 — the sampling is honest but its stated reason is not.
- **Mild noise / quarter turns.** Probed: `errors_corrected == 0` and the sampled
  matrix identical to the encoder's, for all eight symbols in both cases. See
  MINOR-3.

---

## Must Fix (BLOCKER)

None.

---

## Should Fix (MAJOR)

### MAJOR-1 — `rotation_sweep_0_to_45_degrees` omits M1 on a false premise

`micro_image.rs:163-168`. The doc comment says the sweep runs "on the smallest
and the largest symbol" and the loop is `[MicroVersion::M2, MicroVersion::M4]`.
M2 is not the smallest — **M1 is** (11×11 against M2's 13×13). The one symbol
excluded from the arbitrary-angle sweep is the one with `r = 0`, no correction
capacity at all, and therefore the least tolerant of any resampling error in the
whole symbology. It is the single most valuable rotation case and it is the one
left out, with a justification that is factually wrong.

This is not theoretical and it is not expensive. I ran the identical sweep over
all eight symbols: every one passes every angle from 0° to 45°. The cost is
fractions of a second in a 0.35 s suite.

Also note `.find(|i| i.version == version)` takes the *first* info for that
version, so only M2-L and M4-L are covered — M2-M, M4-M and M4-Q are never
rotated off a quarter turn either. `fixture()` picks the widest content kind, so
the payloads differ per EC level and the cases are not redundant.

Fix: drive the sweep over `micro_symbols()` in full, and delete or correct the
"smallest and largest" sentence. If runtime ever argues against eight, keep M1
and M4-Q (the extremes of size and redundancy) and say plainly that the middle
is sampled.

---

## Should Fix (MINOR)

### MINOR-1 — the "independent zigzag" is a paraphrase, and its ISO ground truth lives elsewhere

`common/mod.rs:340-364` vs `qr-core/src/micro.rs:662-740`. The loop skeleton is
restated (explicit `while col >= 1` / `for k` / `for dx` against the core's
iterator state machine), but it is the same algorithm, and the doc comment is
near-verbatim from the core's own ("the vertical timing pattern sits in column 0
rather than column 6, so there is no mid-symbol column skip, and column 0 is
never part of a two-module-wide pair"). More to the point, it calls
`micro_is_function_module` — the very map that decides which modules are skipped.
A misplaced function module would be invisible to `placement_walk_matches_core`,
because both sides ask the same oracle.

This does **not** leave a hole, and I checked before saying so: ISO Annex I.3 /
Figure I.4 (printed pp. 91-93) gives a complete worked M2-L example, and
`crates/qr-encode/tests/micro.rs:98-190` pins the finished symbol module-for-module
against the printed bitmap plus both codeword rows `[0x40,0x18,0xAC,0xC3,0x00]` /
`[0x86,0x0D,0x22,0xAE,0x30]`. That is proper outside ground truth for the function
map, the walk, the mask and the format bits — for M2. So the genuinely new thing
`placement_walk_matches_core` contributes is self-consistency on M1, M3 and M4,
plus the cardinality check against 36/80/132/192.

That is worth having. It is not "an outside witness" for the shared walk, and the
REVIEW-REQUEST should not say it is. One line in the test's doc comment naming
`iso_annex_i3_reproduces_figure_i4` as where the ISO ground truth actually sits
would stop a future maintainer over-reading this.

### MINOR-2 — the capacity model is independent of `qr-encode`, not of `qr-core`, and 344 pins only the sum

`common/mod.rs:315-338`. `micro_max_chars` reads `info.data_bits`,
`mode.indicator(version)` and `mode.char_count_bits(version)` straight out of
`qr-core`. Bob's wording ("independent of `qr-encode`'s private helpers") is
accurate; the REVIEW-REQUEST's "reproduces every ISO Table 7 figure" is the part
that needs care — the arithmetic reproduces Table 7 *given* `qr-core`'s tables.

The `assert_eq!(cases, 344)` is what turns it into a real check, and I verified
344 against the PDF above, so it does catch a corrupted `data_bits` row. But it
pins the **total**, not the sixteen individual capacities: one capacity going up
by one and another going down by one passes green. `one_past_capacity_is_refused`
narrows this from the encoder's side, which is a good pairing. If you want it
airtight, pin the eight per-symbol subtotals (5/16/13/55/43/86/74/52) rather than
one grand total — same cost, no compensating-error hole.

### MINOR-3 — two image tests use the weak assertion where the strong one is free

`micro_image.rs:153-161` `assert_rotated_decodes` checks bytes only, and it is
used by `rotation_quarter_turns` (:190) and `mild_noise` (:213). Its doc comment
justifies that with resampling, which is right for
`rotation_sweep_0_to_45_degrees` and wrong for the other two: quarter turns are
resampling-free by construction, and the mild-noise case is claimed in the
REVIEW-REQUEST to "not disturb any micro symbol" — a claim the test does not
assert.

Measured, all eight symbols, both tests: sampled matrix **identical** to the
encoder's and `errors_corrected == 0` in every case. So `assert_micro_decodes`
can be dropped in as-is and the "no disturbance" claim becomes contractual instead
of prose. As it stands, RS could start silently mopping up flipped modules under
mild noise and nothing would go red.

### MINOR-4 — `seeded_fuzz_auto_version`'s floor is 2.4× below the real rate

`micro_roundtrip.rs:530-533` asserts `encoded > 50` of 300 draws. Measured with
the shipped seed: **123**. An encoder regression that rejected half of what it
accepts today would still pass. Raise the floor to something near the observed
value (`>= 110`) or pin it exactly at 123 — the seed is fixed, so the number is
deterministic and a change to it is information, not noise.

### MINOR-5 — two tests go vacuous if `has_half_final_data_codeword` regresses

`micro_roundtrip.rs:181-183` and `:352-354` both `continue` on
`!info.has_half_final_data_codeword()`. If that predicate ever returned false for
every symbol, both tests pass green having asserted nothing, and the M1/M3 4-bit
codeword — the most delicate part of the micro bit packing — loses its dedicated
coverage silently. Same construct the Step 3b review flagged as MINOR-3.

Cheap guard: after the loop, assert the number of symbols actually exercised
(2 for `half_codeword_corruption_costs_one_symbol`, 3 for
`half_codeword_versions_odd_numeric_lengths`).

### MINOR-6 — `two_micros_in_one_image` does not pin the two versions apart

`micro_image.rs:292-333`. The REVIEW-REQUEST says the 7-char alnum payloads "land
on M3-L and M3-M", which is what makes the test interesting — two *different*
micro sizes coexisting. The test uses auto version and never asserts what it got,
so if selection changed and both landed on the same version it would still pass
and the mixed-size coverage would quietly vanish. Add
`assert_eq!(d.version, MicroVersion::M3)` per detection, or pick payloads that
force two different versions and assert both.

### MINOR-7 — the 250-case product is not counted the way the 344 sweep is

`micro_roundtrip.rs:132-150`. `every_length_up_to_capacity` pins its case count
and is the stronger test for it; `all_symbols_all_modes_all_masks` does not. The
`cap > 0` assertion at :136 covers the worst case, so this is a consistency nit
rather than a hole — but the same two-line counter would make the 25-combo product
tamper-evident too.

### MINOR-8 — `m1_rejects_any_single_module_flip` is exhaustive in position, single in payload

`micro_roundtrip.rs:417-444`. All 36 data modules, one flip each — genuinely
exhaustive over the data region, and the right test to have. Worth recording that
it runs against exactly one fixture (capacity-filled 5-digit numeric, mask 0), so
a defect that only misdecodes at a shorter payload or a different mask is out of
its reach. Wrapping the body in a loop over lengths 1..=5 and masks 0..4 is 180
decodes on an 11×11 symbol — free at this suite's runtime. Not required; noted.

---

## Ruling on the p = 0 question (Bob's open question 1)

**Keep the strong `Err` assertion exactly as written. Do not weaken it.** Two
reasons, both measured rather than argued.

**It is deterministic by fixture, not fragile.** The shipped test has no
randomness: fixed payload from `payload(kind, cap)`, mask forced to 0, and the
first `r + 1` full-width codewords corrupted by total bit inversion. It will pass
or fail identically on every run. I then attacked the fragility question directly:
600 randomized trials per correctable symbol — random payload length across the
whole capacity range, random subset of `r + 1` full codewords via Fisher-Yates —
**4 200 trials, 4 200 hard errors, zero misdecodes**, including all three `p = 0`
symbols (M3-M r=4, M4-M r=5, M4-Q r=7). I also re-ran the shipped corruption
pattern across every payload length 0..=cap for all seven symbols: not one decoded.
The assertion is not sitting on a lucky fixture; it is sitting on a wide plateau.

**The proposed weakening is vacuous, and slightly backwards.** "Must not return
the original payload" cannot fail. A bounded-distance Reed-Solomon decoder with
`t = r` cannot reach the true codeword when `r + 1` symbols are in error — the
true codeword is at distance `r + 1`, outside the decoding sphere. My probe
confirms it: `correct = 0` in all 4 200 trials. Swapping the strong form for that
one would replace a real assertion with a tautology.

There is also a framing point worth stating plainly, because it affects any future
attempt to relax this: returning the *original* payload after `r + 1` errors would
be a *good* outcome, not the failure mode. The thing `p` exists to prevent is a
confident *wrong* payload. So the only honest weaker form would be "may misdecode,
just don't crash", which is not a test.

What I would add is one sentence, not a code change: in the doc comment at
`micro_roundtrip.rs:375-378`, record that for M3-M, M4-M and M4-Q this is a
fixture-specific canary rather than an ISO guarantee (`p = 0` reserves nothing),
and that if it ever fires after an unrelated change to `payload()` or the capacity
model, the correct response is to investigate the new fixture — not to soften the
assertion. That keeps the canary and stops the next person from filing it as
flaky.

---

## Escalate to Architect

1. **BUILD-LOG Step 5e row.** Bob left it out per the write restriction on his
   dispatch, correctly. It needs adding, along with the honest-envelope numbers
   from the REVIEW-REQUEST (scale floor 2 px/module, rotation 0–45°, quiet zone 2)
   — those are the kind of thing that gets lost between sessions.
2. **Bob's open question 2 (`qr_core::micro` imported unconditionally in
   `common/mod.rs:17-19`).** Agreed with Bob, and it is a Step 6 CI decision, not
   a code decision. If the CI matrix grows a micro-less configuration for the test
   harness, `common/mod.rs:276-410` plus both suites need `#[cfg]` gating or a
   `micro` feature on `tests-integration`. Worth writing into the Step 6 brief now
   while it is cheap, rather than discovering it when the matrix goes red.
3. **Kanji.** ISO Table 7 lists Kanji capacities for M3 and M4; `qr-encode` does
   not implement it and says so. The integration suite therefore cannot cover it
   and correctly does not pretend to. Confirming this stays a logged Known Gap
   rather than drifting into "tested".

---

## Cleared

Reviewed both new suites plus the `common/mod.rs` micro section against the
Step 5e brief (`handoff/ARCHITECT-BRIEF.md:104-106`) and the 5f r-cap commit
(`6174b10`).

- **All brief items present.** Roundtrip sweep M1–M4 × levels × content
  (`all_symbols_all_modes_all_masks`, 250 cases over all 25 valid symbol/mode
  combos); image path encode→render→`detect_micro`→`decode_micro`
  (`image_path_all_symbols_all_modes`, plus scale and mask sweeps); mixed QR +
  micro frame (`qr_and_micro_in_one_image`); fuzz extension (two seeded loops,
  700 draws); rotation (0–45° sweep plus quarter turns on all eight). The r-cap
  integration regression (`v1_l_three_codeword_errors_never_misdecode`) is present
  and correct — v1-L is (26,19,2) per Table 9, three errors do exceed r = 2, and
  the test pins detection-succeeds / matrix-matches / `decoded.is_none()` with a
  clean-symbol control through the identical path.
- **Assertions are byte-exact throughout.** Every roundtrip asserts
  `decoded.bytes == data`, never a bare `is_ok()`; the matrix-level suite adds
  version, EC and mask readback plus `errors_corrected == 0` on clean symbols. The
  image suite compares the detector's sampled `BitMatrix` module-for-module against
  the encoder's wherever resampling permits.
- **`errors_corrected` is pinned exactly, not merely non-zero.** `== r` in
  `corruption_within_capacity_is_corrected` (both data region and EC region), `== 1`
  in `half_codeword_corruption_costs_one_symbol`, `== 0` on every clean path.
- **The 344-case sweep is exhaustive and correctly counted.** Verified against ISO
  Table 7 extracted from the PDF; the derivation is reproduced above.
- **M1 detection coverage is exhaustive.** All 36 data modules, one flip each,
  every one rejected — the right test for an `r = 0` symbol, where a silent repair
  is a misdecode. See MINOR-8 for its one-fixture scope.
- **Mixed-image cross-interference is asserted in both directions.** `detect_all`
  returning exactly 1 QR and exactly 1 micro rules out both the QR triple
  swallowing the micro finder and the micro path claiming one of the QR's three.
- **No silent skips that fire today.** Every `continue` guard was checked: the
  `r == 0` skips reach only M1 and are covered by the dedicated M1 test; the
  `has_half` skips reach the right three symbols. See MINOR-5 for the construct
  risk, not a present gap.
- **Runtime and envelope claims are accurate.** Measured independently; the
  2 px/module floor and the 0–45° rotation range both hold, and the sampling that
  was done is sampling, not concealment.

One MAJOR, and it is a cheap one: put M1 back in the rotation sweep and fix the
sentence that says M2 is the smallest symbol. Nothing here ships broken.
