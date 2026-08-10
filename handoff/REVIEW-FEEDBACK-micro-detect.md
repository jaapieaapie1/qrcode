# Review Feedback — Step 5d (qr-detect micro)
Date: 2026-08-10
Reviewer: Richard-MT
Ready for Builder: YES

**Verdict: Step 5d CLEAR** (round 2).

Round 1 raised 1 BLOCKER, 1 MAJOR, 4 MINOR. Bob's round 2 closes both blocking
items; I re-verified each independently rather than taking the round-2 note at
its word. See **Round 2 verification** at the bottom. The four MINORs stand as
logged — none blocks, MINOR-1 and MINOR-2 are for Arch's Known Gaps.

The round-1 body below is left intact as the record of what was found and why.

Scope reviewed: `crates/qr-detect/` only (`micro.rs`, `tests/micro_detect.rs`,
`lib.rs`, `grouping.rs`, `binarize.rs`, `Cargo.toml`). qr-encode's in-flight
state ignored per dispatch.

The micro algorithm itself is good work — the geometry is sound, the failure
paths close, and the test suite is honest about its envelope. The block is not
about `micro.rs`. It is about the one line Bob changed in a stage that every
symbology and qr-video sit on top of.

---

## Verification performed

| check | result |
|---|---|
| `cargo test -p qr-detect` | 19 lib + 21 detect + 21 micro, green |
| `cargo test -p qr-detect --no-default-features` | 16 lib + 20 detect, green (micro compiled out) |
| `cargo test -p qr-detect --no-default-features --features micro` | green |
| `cargo test -p qr-detect --no-default-features --features decode` | green |
| `cargo test -p qr-detect --all-features` | green |
| `cargo clippy -p qr-detect --all-targets -- -D warnings` (all five configs) | clean |
| `cargo fmt --check -p qr-detect` | clean |
| `cargo test -p qr-video` | 7 passed, green |
| `cargo test -p tests-integration` | 54 passed across 6 binaries, green |

`tests-integration` is fully green — nothing attributable to qr-detect, and
nothing attributable to qr-encode reached it either.

---

## Must Fix

> **Round 2: both items below are CLOSED.** Verification at the bottom of this
> file. Retained as the record.

### BLOCKER-1 — `binarize.rs:84` — adaptive window `/16 → /8` is a shared-stage regression with no coverage in the direction it costs

**The latent bug is real and I verified it independently.** I replicated
`adaptive()` byte-for-byte outside the crate and ran Bob's own fixture
(168 px frame, 24 px solid block):

```
radius = min/16  ->  16 px of the 24x24 core hollowed out
radius = min/8   ->   0 px
```

16 px is exactly the 4x4 hole Bob reported. The mechanism is exactly as
described: the pixel-dark rule is `value*count*100 < sum*88`, so a black pixel
only flips light when the local mean is *exactly* zero — i.e. when the window is
fully contained in the block. The condition is `2*(min/8)+1 > 3*module`, which
for a symbol of `S` modules with a `Q`-module quiet zone reduces to
`S + 2Q > 12`. A legal M1 with ISO's 2-module quiet zone is `11 + 4 = 15`. It
clears, with 25% headroom. So the change is correct and the new radius is the
right order of magnitude. `binarize::tests::a_solid_block_does_not_hollow_out`
is a genuine regression lock — it does fail on the old radius.

**But the rationale overstates the QR case, and that matters because Arch will
rule on it.** The review request says "the same arithmetic hits a QR v1 filling
a 500 px frame". Under the old `/16` the condition is `S + 2Q > 24`. QR v1 with
the spec-mandated 4-module quiet zone is `21 + 8 = 29`. It clears. The old
radius only broke a QR v1 framed with a quiet zone under ~1.5 modules, which is
sub-spec. This is *not* a pre-existing bug for spec-compliant QR. It is a
micro-only bug. That does not make the change wrong; it does mean the change is
being made *for micro*, at the expense of the QR/video path, and should be
argued on those terms.

**And it does cost the QR/video path.** I measured it. Same replicated
`adaptive()`, 1280x720 frame, 25-module symbol at 3 px/module, 45% print
contrast (worn print), radial vignette, symbol in the frame corner. Module
errors out of 625:

| vignette depth | errors at `/16` | errors at `/8` |
|---|---|---|
| 0.70 | 0 | 0 |
| 0.80 | 0 | 23 |
| 0.85 | 0 | 71 |
| 0.90 | 0 | 109 |
| 0.95 | 1 | 140 |

Also, on a linear illumination ramp at 1920x1080 the clamped-window border band
(width = `radius`, now 135 px per edge instead of 67) starts producing spurious
dark pixels at ramp 0.10 luma/px under `/8` versus 0.20 under `/16` — the new
radius halves the tolerable illumination gradient inside a band that is now
12.5% of the short side on each edge instead of 6.25%.

To be fair to the change: for a symbol away from the frame border on a *linear*
gradient, window width costs nothing — the local mean tracks a linear ramp
exactly at any radius. I confirmed 0 errors at every radius/ramp combination
mid-frame. The cost is confined to (a) the clamped border band and (b) the case
where the window is much wider than the symbol and the illumination is curved.
Both are the qr-video case.

**Existing coverage is thin, precisely.** `binarize::tests::adaptive_handles_gradient_with_marks`
is the only gradient fixture: 64x64, background ramp 80..144, marks at luma **0**.
That is a 100% contrast mark against a 64-luma ramp — it passes at any radius and
constrains nothing. At 64 px the radius moves 4 → 8 and the test cannot tell.
There is no fixture anywhere in the workspace that constrains adaptive
thresholding under illumination that the new window cannot track. KG-12 as
drafted understates this: it is not that no fixture "goes beyond the existing
64 px unit test" — it is that the 64 px test does not test gradient tolerance at
all.

**What I am not asking for.** I looked for a free fix and there isn't one. A
fixed absolute cap on the radius fails: a micro symbol filling a 720p frame has
48 px modules, needing a 145 px window, so any cap tight enough to help vignette
is too tight for fullscreen micro. Pixelwise union of the two thresholds
inherits the vignette failure; intersection reinstates the hollowing. The honest
options are (a) accept `/8` and its cost, or (b) two candidate binarizations
driven through the finder scan. That is a robustness trade-off affecting a
shipping downstream, which is an Architect call, not a Builder one.

**To clear this:**
1. Escalate to Arch for a ruling on (a) vs (b) — see Escalate section. Do not
   land a shared-stage threshold change on Builder judgement alone.
2. Whichever way it goes, add a binarize test that locks the trade-off *in the
   direction it costs* — a fixture with a small low-contrast symbol under
   illumination the window cannot track, asserting the behaviour that was
   chosen. Right now the suite locks only the direction that improved.
3. Correct the KG-12 wording and the "pre-existing latent bug ... QR v1 filling
   a 500 px frame" claim to the `S + 2Q > 24` / `S + 2Q > 12` arithmetic above.

---

## Should Fix

### MAJOR-1 — `micro.rs:63` — `MAX_FUNCTION_MISMATCHES = 2` sits exactly on the boundary that admits a phantom; and no test ever reaches it

Bob asked for a ruling between 0, 2 and 4. **It should be 1.** Here is why, and
it is not a taste argument.

Consider the timing walk overshooting by one module pair: a dark speck in the
quiet zone exactly one module wide, starting one module past the symbol edge,
appearing in both the row and column walk. `walk_timing` accepts two extra
transitions (both gaps land inside `0.55..=1.45`), `last` increases by 2 and
stays even, so `dim' = dim + 2` — which for dim 11/13/15 is *another valid micro
size*, `dim_row == dim_col` holds, and `MicroVersion::from_size` accepts. Now
score it in `function_mismatches`:

- the 8x8 finder/separator block: the measured edge point still maps at the true
  module scale, so the finder samples correctly → **0 mismatches**
- `for k in 8..size`, row 0 and column 0: `k < dim` are real timing modules →
  correct. `k = dim` is odd (all micro sizes are odd) so `k % 2 == 0` is false,
  expected light, quiet zone is light → correct. `k = dim + 1` is even, expected
  **dark**, quiet zone is light → **1 mismatch per axis**

Total: **exactly 2**. `mismatches > MAX_FUNCTION_MISMATCHES` is false at 2, so
the phantom is accepted and a wrong-dimension grid is returned. At
`MAX_FUNCTION_MISMATCHES = 1` it is rejected. The current value is not "generous
but not phantom-friendly" — it is one unit too generous, and the failure it
admits is structural rather than random.

Against 0: 1 still absorbs a single binarisation casualty on a function module,
which is the stated purpose of the tolerance. Against 4: 4 admits the same
phantom plus a second module pair of slack, and would also widen the perspective
envelope past the point where the affine map is trustworthy — the 4% fail-closed
number in the honest envelope is *produced by* this threshold.

Second half of the finding: **the threshold is untested.** All four negative
tests — `isolated_finder_pattern_is_not_a_micro_symbol`,
`finder_with_broken_timing_is_rejected`, `a_qr_symbol_alone_yields_no_micro`,
`blank_image_yields_nothing` — die in `walk_timing` before `function_mismatches`
is ever consulted. Bob's own note confirms it: every positive test lands on 0.
So the constant has no test in either direction. Whatever value is chosen needs
a negative fixture that actually reaches the validator — a finder-shaped square
with plausible-but-wrong surroundings that survives the timing walk. The
overshoot case above is the natural fixture: render a valid M1/M2 and place a
one-module dark speck in the quiet zone on both timing axes.

---

## Minor

### MINOR-1 — `micro.rs:378–393` — a bridged timing module can truncate the walk into a valid smaller grid, scoring 0 mismatches

The mirror image of MAJOR-1, and this one no mismatch budget can catch. If a
timing module is lost to binarisation so that two transitions merge, the gap
exceeds 1.45, the walk breaks early, and `dim` lands on a *smaller* valid size —
e.g. an M4 truncated at grid 11 reports M1. The resulting grid is a self-
consistent crop of the real symbol: the finder is correct, `k` in `8..11` are all
real timing modules, so `function_mismatches` returns 0 and the crop is reported.

It needs the bridge to occur symmetrically in both the row and column walk
(otherwise `dim_row != dim_col` rejects), so it is hard to reach, and
`decode_micro` will reject the crop because the format information lands
elsewhere. But it means the honest envelope's "fails closed — no detection,
never a wrong grid" is not unconditional. I looked for a cheap guard and there
isn't one: breaking on an out-of-range gap is also the *normal* termination when
the walk runs into a distant speck in the quiet zone, so it is not a usable
discriminator, and the module scale of a crop is self-consistent so no geometric
cross-check separates them.

Action: soften the envelope wording and log it. Suggested KG-13: "detect micro:
a timing module lost to binarisation in both walks at matching offsets can
truncate the dimension walk to a smaller valid size and yield a self-consistent
crop that passes function-pattern validation. Caught downstream by
`decode_micro` (format information lands off-place), not by detection."

### MINOR-2 — `micro.rs:423` — the function-pattern map is hardcoded, and only a `debug_assert!` ties it to qr-core

`function_mismatches` re-derives the finder/separator/timing layout by hand
(`x == 0 || x == 6 || y == 0 || y == 6 || ((2..=4).contains(&x) && ...)`) rather
than deriving it from `micro_is_function_module`. That is defensible — the core
predicate answers "is this a function module", not "what value must it hold" —
but the only link between the two is
`debug_assert!(micro_is_function_module(version, k, 0))`, which compiles out in
release. If qr-core's micro function map is ever corrected, this silently drifts
in exactly the builds that ship. Either make it a real check or ask qr-core for
an expected-value helper so there is one source of truth. Not blocking; log it.

### MINOR-3 — `micro.rs:57` — `ANGLE_TWEAKS`: brute force is the right call, no change

Bob's open question 3. Keep it. The early return on `mismatches == 0` means the
common case costs one attempt, not twenty, and every failing attempt dies in
`walk_timing` on the first sample or the `transitions[0] ≈ 7.0` check. Deriving
the tweak from a measured residual would add a feedback loop with its own
failure modes to save work that is not on any hot path. Not a defect.

### MINOR-4 — test helper duplication: keep it duplicated, no `tests/common/mod.rs`

Ruling below. Recorded here so it is not re-litigated.

---

## Rulings on Bob's open questions

**1. The binarize radius — right call, wrong place, and not Bob's call to make.**
Right call in that the arithmetic is correct, the latent bug is real, and micro
does not work without it. Wrong place in that it is a behaviour change to the
one stage shared by every symbology and by qr-video, it carries a measured cost
(BLOCKER-1), and it landed inside a micro feature step on Builder judgement. It
does not need its own commit — it needs an Architect ruling and a test in the
direction it costs. See BLOCKER-1.

**2. `MAX_FUNCTION_MISMATCHES` — set it to 1, and give it a test.** Not 0, not 2,
not 4. The margin at 2 is exactly the width of the `dim + 2` overshoot phantom;
1 closes it while still absorbing one casualty. See MAJOR-1 for the arithmetic.

**3. Test helper duplication — no, do not extract `tests/common/mod.rs`.** The
duplicated helpers are pure geometry (render/rotate/pad/perspective/noise) with
no shared state and essentially no drift risk; they are the cheapest kind of
duplication to carry. Extraction buys ~120 lines and costs coupling between two
suites that test different symbologies with different fixture generators, in a
crate where each integration test file being self-contained has been the
standing pattern since Step 2d. The place where cross-suite helper sharing
actually pays is Step 5e's `tests-integration` micro suite, which spans crates.
Revisit if a third file in this crate wants them. For now: keep the copy.

---

## Reviewed and passed

Everything below was checked against the review request's claims and holds.

**Ray-scan square geometry (`square_axes`, 113).** The claimed profile is
correct: from the finder centre a ray crosses dark core → gap → outer ring →
outside, so `changes.get(2)` is the outer edge. `max_r = finder.module * 6.0`
covers the 45° corner at `3.5·√2 ≈ 4.95` modules with margin, and the comment's
premise (`finder.module` is inflated by rotation, never deflated) is what makes
both that bound and the `min_run` speck threshold conservative rather than
optimistic. The averaging of opposite corner pairs, the 1.35 aspect guard, the
0.25 orthogonality guard and the `cross2` handedness fix-up are all correct. The
`min_run` filter is the right call and the request is honest about why.

**Orientation frames — a symbol at exactly 45° does not alias.** The four-ray
sum minimisation over `t ∈ [0, RAYS/4)` recovers the square's axes modulo 90°;
at 45° the square is still a square, so the minimisation is well-posed and the
corner sectors (±22.5° centred on `axis_idx + RAYS/8`) land the corners at sector
centres, the best case rather than the worst. Whichever of the four equivalent
axis choices the minimisation returns, all four frames are then tried and each is
validated independently, so a wrong frame cannot be *selected* — it can only be
*accepted*, and acceptance requires timing patterns in a direction where there
are none. `frames_are_right_handed` locks the handedness of all four. Bob's
`awkward_angles` and the 2.5° sweep to 45° confirm it empirically.

**Timing walk — the self-calibrating claim holds, with the two holes above.** I
checked whether the snap can accept 15 when the truth is 17. From module-estimate
error alone: no, and the bound is tight. The `|transitions[0] - 7.0| ≤ 0.6` check
constrains the estimate to within 8.6% of truth; at that worst case the per-module
gaps are 0.914–1.086, comfortably inside `0.55..=1.45`, so estimate error cannot
break the walk early. `END = 20.5` clears a 17-module symbol at the worst estimate
(18.5). The `last % 2 == 1` drop-back is correct — the symbol edge is always a
dark→light change at an even index because every micro size is odd — and cannot
underflow, since odd implies `last ≥ 1`. `dim = 7 + last` then goes through
`MicroVersion::from_size`. The two genuine holes are a spurious transition pair
(MAJOR-1) and a bridged module (MINOR-1); neither comes from the estimate.

**Affine map from 3 anchors — fails closed, and the parallelogram completion is
exact.** `src[2] = (dim - 3.0, dim - 3.0)` is the correct completion of
`(3.5,3.5)`, `(dim,0.5)`, `(0.5,dim)`, and all four coordinates are exactly
representable in f64, so `x0 - x1 + x2 - x3` is exactly `0.0` and
`square_to_quad` takes its affine branch for the source quad — the map really is
the affine map through the three measured points, as claimed. The destination
quad may miss zero by an ulp and take the general branch, with negligible
`a13`/`a23`. Degenerate input cannot panic: a zero denominator yields NaN, and
`p.x.round() as i64` on NaN saturates to 0 in Rust rather than trapping, so the
sample reads out of bounds → light → validation rejects. The 4% envelope is
produced by the mismatch budget, which is a further reason not to raise it.

**Function-pattern validation placement.** Inside `try_frame`, before the corner
computation and before any `Some` is returned. Nothing escapes unvalidated, and
the early return at `mismatches == 0` cannot skip it.

**Feature wiring — micro decoding is active iff `micro` + `decode`.** Verified
against the real manifests: qr-core `default = ["micro"]`, `micro = []`;
qr-decode `default = ["micro"]`, `micro = ["qr-core/micro"]`; qr-detect
`micro = ["qr-core/micro", "qr-decode?/micro"]`. The weak dependency is the right
tool and the reasoning in the request is correct — `--features micro` alone does
not pull qr-decode in, and `micro + decode` cannot reach a `qr_decode::micro`
that was compiled out. `build_micro_detection` has exactly two arms,
`all(micro, decode)` and `all(micro, not(decode))`, so there is no cfg gap.
All five configurations build, test and clippy clean (table above).
`--no-default-features` compiles `micro_detect.rs` out cleanly via the file-level
`#![cfg]`. qr-video's `features = ["decode"]` pin does not set
`default-features = false`, so additivity is preserved and the brief's rule holds.

**Finder-consumption handshake — no behaviour change on the QR path.** `group`'s
`Vec<QrCandidate>` return is identical: `used` was only hoisted above the `n < 3`
early return so it can be returned, and nothing reads it in between. Marking is
unchanged — only accepted triples set it, and a triple rejected by the nesting
rule correctly leaves its finders available. The three internal tests were
updated by destructuring only. `two_qr_symbols_in_one_image` and the multi-QR
grouping test still pass, so multi-QR grouping is intact, and
`a_qr_symbol_alone_yields_no_micro` covers v1/v2/v7 for the phantom direction.

**No panics, no unsafe, API discipline held.** No `unsafe`, no `unwrap`, no
`expect`, no `panic!` anywhere in `micro.rs`. Every fallible path is `Option`.
Array indexing is bounded: `radius[idx]`/`valid[idx]` go through
`rem_euclid(RAYS as i32)`, `transitions[i - 1]` has `i ≥ 1`, `changes.get(2)` is
checked. Float-to-int casts are saturating, not UB. On the API: `Detection`,
`detect`, `detect_luma`, `read_one`, `read_one_luma` are untouched;
`MicroDetection` is a genuinely parallel type carrying no QR version or EC
concept; and it matches `Detection`'s (absent) derives, so the two are
consistent. Brief compliance for Step 5d is complete — single-finder anchor,
orientation from timing, 11/13/15/17 snap, sample → `decode_micro`, micro
candidates restricted to finders no triple claimed, nothing added to `Detection`.

---

## Escalate to Architect

**The binarize window radius is a robustness trade-off, not a code decision.**
Micro detection requires a window wider than the 3-module finder core, which for
a tightly framed micro symbol means `min(w,h)/8`. The QR/video path wants the
narrowest window that still tracks illumination, which for large camera frames
means `/16` or narrower. Both are correct for their own input. The measured
numbers are in BLOCKER-1: `/8` is required for micro and costs roughly half the
tolerable vignette depth for a small low-contrast symbol at a frame corner.

Two options, and I cannot pick between them at the code level because the choice
is about which input the library is expected to handle well:

- **(a) Accept `/8` as the single global radius.** One line, already written,
  everything green today. Cost: qr-video's worst-case envelope narrows, and we
  will not find out from the test suite when it bites.
- **(b) Two candidate binarizations driven through the finder scan** — `/16` for
  locality, `/8` for large solid features, union the finder results. Keeps both
  envelopes. Cost: roughly doubles binarize + finder-scan work, and it is real
  work in a stage that is currently simple, so it wants its own step rather than
  being bolted onto 5d.

I checked the cheap middle grounds and they do not exist: an absolute radius cap
breaks fullscreen micro (48 px modules need a 145 px window), a pixelwise union
of the two thresholds inherits the vignette failure, and an intersection
reinstates the hollowing.

My recommendation, for what it is worth, is (a) now with the two-way coverage
test from BLOCKER-1 and KG-12 rewritten honestly, and (b) logged as the lever if
qr-video ever reports a field failure. But that is Arch's call to make, on the
record, not something to carry in as a one-line diff inside a micro feature step.

---

# Round 2 verification

Date: 2026-08-10. Scope: BLOCKER-1 and MAJOR-1 only, per Arch's dispatch.
Both closed. **Step 5d CLEAR.**

## BLOCKER-1 — CLOSED. The vignette fixture is honest.

Arch asked me to judge the fixture, not to insist it match my numbers. It is
honest, and I say that having reproduced it rather than read it.

I re-implemented `vignetted_module_errors` outside the crate against the same
replicated `adaptive()` I used in round 1, with Bob's exact parameters
(1280x720, 25 modules at 3 px inset to (40,40), dark reflectance 0.55, radial
`1 - depth*r^2`), and swept the divisor:

```
fixture: 1280x720, 25 mods @3px at (40,40), dark reflectance 0.55, dark density 292/625
  depth     /16      /8      /6      /4
   0.50       0       0       0      10
   0.70       0       0       6     246
   0.85       0       0     100     323
   0.86       0       0     102     325
   0.88       0       5     121     328
   0.90       0      13     139     331
   0.95       0      32     173     333
```

The `/16` and `/8` columns reproduce the comment's table **exactly** — 0/0/0 at
0.50/0.70/0.85, 13 at 0.90, 32 at 0.95, and `/16` clean throughout. The table in
the source comment is a measurement, not a recollection.

**Why his knee (0.88) differs from mine (0.80), and why that is fine.** Different
fixture, and the differences pull in both directions: his print contrast is
*lower* than mine (0.55 reflectance vs my 0.45, so harder) while his dark-module
density is *lower* (292/625 ≈ 47% vs my 67%, so easier), and his symbol is inset
40 px rather than 8 px. His parameters are the more realistic ones on every axis
— real data modules run near 50% density, and a 40 px inset is a plausible
framing where an 8 px one is not. A fixture is not dishonest for being less
adversarial than mine; it is dishonest if it hides the cost or pins a boundary it
cannot hold. This one does neither: it states plainly that `/16` held everywhere
and `/8` does not past 0.85, and it names the two-pass binarize as the fix that
would keep both envelopes.

**The pin is placed correctly — inside the envelope, not on the cliff.** I probed
the gap: the true `/8` knee on this fixture is 0.88 (5 errors), so asserting 0 at
0.85 leaves real headroom rather than sitting one rounding step from red. That is
the right side of the line to pin on.

**Two-way is genuine, at the pair level, which is what I asked for.** The radius
is now bracketed from both directions and I confirmed each bracket is live:

- too narrow — at `/16`, `a_solid_block_does_not_hollow_out` fails (round 1:
  16 px of the 24x24 core hollows, verified independently then)
- too wide — at `/6` the 0.85 case produces **100** module errors and
  `adaptive_survives_a_vignette_to_the_documented_depth` fails; at `/4`, 323

So `/8` is now held in place by a test on each side. That is exactly the coverage
whose absence was the blocker. The radius itself is unchanged per Arch's ruling,
which is the outcome I recommended.

KG-12 as rewritten is accurate: the `S + 2Q > 12` / `> 24` arithmetic is correct,
the "spec QR v1 at 21 + 8 = 29 never did" correction is the one I asked for, and
the cost is stated rather than buried.

## MAJOR-1 — CLOSED. The constant is pinned from both sides.

`micro.rs:68` is `MAX_FUNCTION_MISMATCHES: u32 = 1`, and
`the_function_pattern_budget_is_one_module` is the first test in the suite that
reaches the validator at all.

Bob claims it locks the value and not just the code path. I did not take that on
trust — I edited the constant and ran it, both ways, then restored the file:

| constant | result |
|---|---|
| `= 0` | **FAILS** at `micro_detect.rs:340` — "one casualty: expected exactly one micro detection" |
| `= 1` (shipped) | passes; whole suite green |
| `= 2` | **FAILS** at `micro_detect.rs:691` — "two damaged function modules must be rejected by the validator" |

In both failing runs it was the *only* test to fail (21 passed, 1 failed). So the
test brackets the constant from both sides and pins it to exactly 1 — stronger
than the one-way lock I asked for. `micro.rs` restored byte-for-byte from backup
afterwards; the constant reads 1 and the suite is green.

The fixture reasoning is sound too. Corrupting core corners (2,2) and (4,4) to
light gives exactly 1 and 2 mismatches in `function_mismatches`, and the choice
of *which* modules is well argued: making a core corner light merges it into the
gap ring and leaves every `boundary_radius` where it was, so the ray scan and the
timing walk are undisturbed and the verdict really is the validator's. The
one-casualty case passing `assert_micro_recovers` against the *corrupted* matrix
is the proof that the corruption reaches the validator instead of tripping an
earlier stage — that is the right way to demonstrate it, and it is the argument I
would have asked for if he had not made it himself.

The overshoot-phantom arithmetic that motivated the change is now in the doc
comment, so the next person to reach for a looser budget will see why it is tight.

## Spot-run

| check | result |
|---|---|
| `cargo test -p qr-detect` | 20 lib + 21 detect + 22 micro, green |
| `cargo test -p qr-detect --no-default-features` | 17 lib + 20 detect, green |
| `cargo fmt --check -p qr-detect` | clean |

Counts match Bob's round-2 DoD exactly (lib 19→20 for the vignette test, micro
21→22 for the budget test). Nothing else in the crate moved.

## Still open — none blocking

MINOR-1 (truncated-walk crop), MINOR-2 (hardcoded function map behind a
`debug_assert!`), MINOR-3 (`ANGLE_TWEAKS`, no change wanted) and MINOR-4 (helper
duplication, ruled keep) stand as written. MINOR-1 and MINOR-2 are Known Gap
material for Arch; Bob has already softened the envelope's "never a wrong grid"
wording per MINOR-1. None of them blocks Step 5d.

**Step 5d CLEAR.** Good work on the round 2 — both fixes are the real thing,
tested in the direction that costs, and Bob argued the fixture design instead of
asserting it. That is the standard.
