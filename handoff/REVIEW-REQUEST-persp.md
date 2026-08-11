# REVIEW-REQUEST — Step 10: Micro QR perspective refinement

**Ready for Review: YES** — round 2, Richard's `REVIEW-FEEDBACK-persp.md`
applied (2 MAJOR + 6 MINOR; see "Round 2" at the end for what changed and the
envelope delta).
**Scope:** `crates/qr-detect/` only. `benchmarks/` untouched (sibling Bob owns it).
**BUILD-LOG.md deliberately not touched** — a sibling Bob is active in the same
session and BUILD-LOG is a shared file. Arch to fold in the three entries
Richard listed (KG-13 movement, quiet-zone asymmetry, `square_axes` known gap).

---

## The problem

Micro QR sampling built its grid->image map from three measured anchors (finder
centre, and the outer end of each timing walk) and *assumed* the fourth corner:
the parallelogram completion. That is an affine map, and a micro symbol has no
alignment pattern to correct it with.

The Project Owner's report: an M1 on a hand-held phone screen at 10-20 degrees
of tilt was **located but never decoded** — corners drawn, grid wrong. A full QR
symbol at the same angle decoded, because the QR path solves a real homography
from three finders.

Reproduced, and it is exactly the affine assumption. Measured on the M1 golden
vector under a pinhole tilt, pre-change:

| camera distance (symbol spans) | first tilt angle that fails |
|--------------------------------|-----------------------------|
| 2 | 10 deg |
| 3 | 10-15 deg |
| 4 | 15 deg |
| 6 | 20-25 deg |

---

## Design

Four stages, in `crates/qr-detect/src/micro.rs`.

### 1. Keep every timing boundary (`walk_timing`)

The timing walk already measured the position of every module boundary along
row 0 and column 0 to a fiftieth of a module; it counted them and threw them
away. It now returns the vector. Entry `i` is the boundary at grid `7 + i`, the
last entry is the symbol's outer edge. `walk_dim` / `walk_edge` recover what the
old two-tuple return carried.

### 2. Trigger: does the affine map explain the walks? (`timing_residuals`)

For each measured boundary, the candidate map's own image of that grid line is
intersected with the ray the walk travelled, and the two positions compared.
Intersecting rather than comparing against `map(k, 0.5)` matters: the ray is the
*initial* estimate's row and drifts across the true one under tilt, which would
otherwise enter the residual as a bias exactly where the signal is.

If the worst residual is within `AFFINE_RESIDUAL_TOLERANCE_PX` (0.45 px), the
affine map is used and nothing else runs — **the untilted fast path is
unchanged**. The tolerance is in pixels because that is where its noise floor
comes from (a binarized edge is located to about half a pixel regardless of
module size). Measured: worst upright residual over the whole suite, 3 px to
20 px modules, is 0.26 px.

### 3. Refinement, two searches (`refine_bottom_right`)

Both search the **bottom-right corner** and build the full four-point homography
via the existing `PerspectiveTransform::quad_to_quad`, with `src` naming the
symbol's actual far corner `(dim, dim)`.

**A. Timing fit** — minimises the sum of squared timing residuals.
Coarse-to-fine, `FIT_STAGES(4) x FIT_LATTICE(7)^2 = 196` candidates, starting at
half a symbol span and narrowing threefold per stage; `search` clamps the result
to that half span, so the bound in the doc is the bound in the code (MAJOR-1).
**Zero image reads** — it
scores against measurements already taken. Unconditionally safe (timing patterns
are function patterns) but weakly conditioned: both walks project onto a ray, and
a projection cannot see where the grid lines sit *across* that ray. Measured, it
lands within about a module of the corner and no closer. Enough to bring the
corner into reach, not enough to sample by.

**B. Probe search** — minimises `grid_disagreement` over the quadrant furthest
from the finder: for each module, the centre is compared with four points a third
of a module away. A grid on the module centres agrees everywhere; a grid half a
module out straddles the boundary and reads the neighbour. Content-agnostic, so
it works in the data area, which is where a corner error shows. `PROBE_STAGES(3)
x PROBE_LATTICE(5)^2 = 75` lattice candidates (76 evaluations — the incumbent is
scored once before the first stage) over a **clamped** total excursion of 1.6
modules per axis. Deliberately tight, because a score that knows nothing about
what the symbol contains would otherwise wander into the quiet zone; without the
clamp the per-stage recentring made the reachable set 2.8 modules, which on an
11-module M1 reaches the quiet zone the bound exists to stay out of (MAJOR-1).

### 4. Adoption gate

The refined map is adopted only if:
- its quiet-zone ring (`quiet_zone_dark`, the modules half a module outside the
  symbol) has at most `MAX_QUIET_ZONE_DARK` (2) dark modules, **and**
- it beats the affine map on `(function_mismatches, grid_disagreement)` —
  strictly beats: the affine map is first in the candidate list and `min_by_key`
  keeps the first minimum, so a tie leaves the incumbent in place (MINOR N1).

`MAX_FUNCTION_MISMATCHES` is unchanged at 1, and is still the accept/reject gate.
The quiet-zone check gates the **refinement only**: the affine map behaves
exactly as before, which is what keeps the retry-pass regression tests
(`a_degraded_micro_sample_does_not_suppress_the_retry`) meaningful.

### 5. Timing-walk gap band, now relative

`walk_timing`'s gap acceptance band was a fixed `0.55..=1.45` in units of the
finder's module. Under tilt the far end of a timing pattern is stretched or
compressed by up to a factor of two relative to the finder, so the walk was
ending early and under-counting the dimension — this, not the sampling, is what
capped M4 at 12%. The band now tracks the spacing actually observed (an EMA over
the gaps seen so far) with an absolute `0.5..2.0` bound on how far it may wander.

Round 1 claimed this was "a tighter per-step test than before". That is only true
*against its own drifting reference*, where it is the same +/-45% the fixed band
allowed. **In absolute terms it is strictly wider**, and both the source comment
and this line now say so: the EMA
converges on whatever the gaps are doing, so the accepted absolute band converges
towards the outer clamp, `[0.5, 2.0)` against the old fixed `[0.55, 1.45]`.
Measured by Richard over ~810 damaged-timing warps, that widening is +2/-1
smaller-crop detections, every one of them KG-13-class and already logged. The
upper clamp is now **exclusive** of 2.0, because a two-module gap is exactly one
bridged boundary and `timing_residuals` maps entry `i` to grid `7 + i` — walking
through one would silently renumber every measurement the refinement fits to
(MAJOR-2). It bought M4 12% -> 14% and M2/M3 17% -> 23%.

### What was tried and rejected (measured, documented in the source)

The brief proposed scoring candidates on function-pattern agreement. **It does
not work, and the reason is structural**: every function module of a micro symbol
lies in row 0, column 0 or the finder block — all clustered at the corner the
search is *not* moving. Measured on a 17-module symbol under a 15% keystone,
function-pattern agreement is already perfect with the corner still half of its
error away, at which point 90 of the 172 data modules still sample the wrong
cell. It is a sound gate; it is not a usable objective. This is recorded in the
doc comment on `refine_bottom_right` so it is not re-attempted.

The brief also suggested seeding the corner from the timing walks' end
directions. That is what search A is, generalised: rather than extrapolating the
last few spacings (3 noisy measurements), it fits all 10-22 of them at once by
least squares. Extrapolating from the far end alone was measured at 0.5-1.5
modules of error against 0.32-0.45 for the full fit, so the fit was kept.

---

## Envelope, before and after

Measured over **120 configurations**: 4 versions x 3 module sizes (8, 12, 20 px)
x 2 masks x 5 keystone directions (top, bottom, left, right, diagonal). A
configuration counts only if every module is recovered **exactly**.

| keystone | recovered before | recovered after |
|----------|------------------|-----------------|
|  0% | 120 | 120 |
|  2% | 116 | 120 |
|  3% | 111 | 120 |
|  4% |  93 | 120 |
|  5% |  62 | **120** |
|  6% |  40 | 118 |
|  8% |  20 | 117 |
| 10% |  14 | **111** |
| 12% |   1 | 104 |
| 13% |   0 |  77 |
| 15% |   0 |  48 |
| 20% |   0 |  33 |

Per-version median first failure: M1 9% -> 14%, M2 7% -> 17%, M3 5% -> 15%,
M4 4% -> 14%.

In degrees, on the M1 golden under a real pinhole projection (the Owner's
scenario), first tilt angle that fails:

| camera distance | before | after |
|-----------------|--------|-------|
| 2 symbol spans  | 10 deg | 30-35 deg |
| 3 symbol spans  | 10-15 deg | 35-40 deg |
| 4 symbol spans  | 15 deg | 30-40 deg |
| 6 symbol spans  | 20-25 deg | 40 deg |

**Honest statement of the new envelope: every configuration exact to a 5%
keystone (was 1%); 111 of 120 at 10%; the useful edge is 12-14% and it is ragged
past that.** Which configuration falls over first past the middle of the envelope
depends on where module centres land, which is why the tests pin counts rather
than individual cases out there. The `Right` keystone (far edge compressed) is
consistently the weakest direction; M4 is the weakest version.

Past the envelope the detector mostly fails closed (no detection), and when it
does sample a wrong grid the codeword stream does not decode, so with `decode` on
nothing wrong is reported. The one *new* failure mode found — a smaller grid
fitted inside a larger symbol, an M4 read as 13 modules at a 16% diagonal
keystone — is what `quiet_zone_dark` exists to reject, and it is rejected.

---

## Cost

Upright cost is unchanged by construction: the trigger is one pass over the
10-22 boundaries the walks already measured (a few hundred flops, no image
reads), and the fast path then does exactly what it did before, except that it
scores the 68-82 function modules before sampling the full 121-289 rather than
after — which makes a *failing* frame cheaper than it was.

End-to-end, release build, 348x348 frame, M4 at 12 px/module, 200 iterations:

| fixture | before | after |
|---------|--------|-------|
| upright | 0.519 ms | 0.519 ms — **unchanged** |
| 10% keystone | 0.433 ms (wrong grid) | 0.496 ms (exact) |

Round 1 reported 0.519 -> 0.472 on upright. Richard could not reproduce an
improvement (0.866 -> 0.866 on his machine, unchanged to three decimals) and he
is right that "unchanged" is both true and the stronger claim; the 0.047 was
noise on a figure dominated by binarization and the finder scan. The +15% on a
refined frame reproduces on both machines.

**Worst-case bound per refined frame:**
- search A: 196 candidate homographies, **0 image reads**
- search B: **76** `grid_disagreement` evaluations (75 lattice + the incumbent,
  scored once before the first stage) x 5 reads x ceil(dim/2)^2 modules =
  **<= 30,780 reads** (M4; 13,680 for M1)
- adoption: 2 function scorings (<=164 reads), **2 further `grid_disagreement`
  evaluations (<= 810 reads on an M4)** — omitted from round 1 — 1 quiet-zone
  ring (<=68), 1 full sample (<=289)

**How many frames can refine:** `detect_from_finder` tries 4 quadrant frames x 5
angle tweaks, so 20 is the hard ceiling, but three of the four frames die inside
`walk_timing` and the loop returns on the first frame scoring 0 mismatches.
Measured on a 10% keystone, all four versions: **1 refinement per finder**.

Rotated symbols at 3-5 px modules can enter refinement without needing it —
resampling makes their edges ragged enough to overshoot the 0.45 px trigger.
Round 1 called that "cost, not risk". Richard swept it (4 versions x 0-90 deg in
2.5 deg steps x 7 module sizes, 1036 configurations per side) and the accurate
statement is **cost plus a little churn at the bottom of the size envelope**: net
exact recoveries improve at every module size (3 px 63->81, 4 px 102->129, 5 px
123->140, 6 px 137->146, and 8/12/20 px all reach 148/148), but at 3 px about 20
configurations and at 4 px two are lost that the old code kept. At 5 px and above
the new failure set is a strict subset of the old. Both affected sizes are below
the documented rotation envelope (10 px, and 5 px for
`rotation_at_a_small_module_size`). The tie-break now favouring the affine map
(MINOR N1) is aimed squarely at this surface.

---

## Files changed

### `crates/qr-detect/src/micro.rs` (+~590 lines)

| what | where | why |
|------|-------|-----|
| module doc, step 5 | 30-33 | the refinement is now part of the documented pipeline |
| `FIRST_TIMING_BOUNDARY`, `AFFINE_RESIDUAL_TOLERANCE_PX`, `FIT_*`, `PROBE_*`, `MAX_QUIET_ZONE_DARK` | 74-143 | the new constants, each with its measurement |
| `try_frame` | 354-447 | fast path preserved verbatim; refinement + adoption gate added after it |
| `sample_symbol` | 449-473 | the full-sample loop, lifted out of `try_frame` so both paths share it |
| `TimingFit` | 475-497 | a walk reduced to (ray, boundaries) for the fit |
| `timing_residuals` | 499-551 | the objective, and why it intersects rather than compares points |
| `refine_bottom_right` | 553-631 | the two searches, and why function-pattern scoring cannot drive them |
| `search` | 633-670 | shared coarse-to-fine lattice, monotone by construction |
| `grid_disagreement` | 672-701 | the probe score |
| `walk_timing` | 703-771 | returns every boundary; relative gap band |
| `walk_dim`, `walk_edge` | 773-782 | what the old two-tuple return carried |
| `function_modules` | 784-810 | the function map, iterator form, shared by the scorer |
| `function_mismatches` | 812-831 | now scores a *transform* against the image, not a sampled matrix |
| `quiet_zone_dark` | 833-858 | the refinement's "is this still the same symbol" check |
| `sample_at` | 860-864 | one rounding rule, was duplicated three times |
| `note_refinement` / `REFINEMENTS` | 866-881 | test-only counter; no-op in release |
| `the_refinement_runs_when_tilted_and_not_when_upright` | 996-1078 | the fast-path assertion |

### `crates/qr-detect/tests/micro_detect.rs` (+~400 lines)

| what | why |
|------|-----|
| `Tilt` enum + `keystone()` | five tilt directions; they are not equivalent, the finder sits in one corner |
| `tilted()` | now takes a direction |
| `envelope_matrix()` | the 40-configuration subset the envelope test runs over |
| `perspective_envelope` | **replaces `mild_perspective`**, whose doc comment asserted the 4% cap was structural. Exact to 5% over all 40; count floors at 10% and 12%. |
| `tilt_3d()` | pinhole projection; distance in pixels, callers pass multiples of the symbol span |
| `golden_m1()` / `M1_GOLDEN_12345` | the decodable M1 vector, from `tests-integration/tests/micro_golden.rs` |
| `off_axis_scenes()` | 16 scenes: 4 camera distances x 4 tilt axes |
| `a_micro_symbol_on_a_screen_photographed_off_axis_decodes` | **the Owner's scenario.** Fails on the pre-change detector at 10 deg (2 spans) and 15 deg (4 spans). |
| `..._is_sampled_exactly` | the `--no-default-features --features micro` half, asserting the grid instead |
| `tilting_does_not_resurrect_a_rejected_candidate` | every negative (bare finder, broken timing, nested triple, noise) x 5 tilts x 5 keystones |
| `a_tilted_qr_symbol_yields_no_micro` | 3 QR versions x 5 tilts x 4 keystones, no phantom micros |

**One existing test was rewritten:** `mild_perspective` -> `perspective_envelope`.
Its body asserted a 4% envelope and its doc comment explained at length why a
micro symbol could not do better. Both are now false. Everything it asserted is
still asserted (all four versions exact at 4%), inside a much wider matrix.

---

## Test evidence

| command | result |
|---------|--------|
| `cargo test -p qr-detect` | 23 lib + 24 detect + 28 micro_detect, green |
| `cargo test -p qr-detect --no-default-features` | 19 + 23, green |
| `cargo test -p qr-detect --no-default-features --features decode` | 19 + 24, green |
| `cargo test -p qr-detect --no-default-features --features micro` | 23 + 23 + 26, green |
| `cargo test -p qr-detect --all-features` | 23 + 24 + 28, green |
| `cargo test -p qr-video` | 7, green |
| `cargo test -p tests-integration` | 84 across 10 suites, green |
| `cargo test --workspace` | 39 suites, green |
| `cargo clippy -p qr-detect --all-targets -- -D warnings` | clean |
| `cargo clippy -p qr-detect --all-targets --no-default-features -- -D warnings` | clean |
| `cargo fmt --all -- --check` | clean |

Requirement-by-requirement:

1. **Perspective sweep** — `perspective_envelope`, plus the 120-configuration
   table above.
2. **Owner's scenario** — `a_micro_symbol_on_a_screen_photographed_off_axis_*`,
   16 scenes at 10/15 deg asserted exactly, a floor of 14/16 at 25 deg. Verified
   to fail on the pre-change detector.
3. **Upright regression** — every pre-existing micro test green unchanged (bar
   the rewrite noted above), and the fast path asserted directly by counter in
   `the_refinement_runs_when_tilted_and_not_when_upright`: 16 upright fixtures
   (4 versions x 4 module sizes), zero refinements, every module exact. The
   second half of that test drives a sheared symbol through and asserts the
   counter *does* move, so the first half cannot pass by the refinement having
   become unreachable.
4. **Phantom safety** — `tilting_does_not_resurrect_a_rejected_candidate` (4
   fixtures x 25 warps) and `a_tilted_qr_symbol_yields_no_micro` (3 versions x 20
   warps), plus every pre-existing negative still green.
5. **Rotation sweep** — `rotation_sweep_is_within_the_documented_envelope` and
   the three other rotation tests unchanged and green.

---

## Open questions / things Richard should push on

1. **`grid_disagreement` reads the image with no idea what it should contain.**
   It is bounded (1.6 modules), it runs after a search that cannot be wrong about
   what it is looking at, and nothing it produces is adopted without clearing the
   function-pattern budget *and* the quiet-zone ring. I believe that is enough,
   and the 220 negative warps say so, but it is the part of this change with the
   most freedom and it is where I would look first.
2. **`MAX_QUIET_ZONE_DARK = 2` is gating the refinement only,** deliberately, to
   keep the affine map's settled retry-pass behaviour. If the reviewer prefers it
   on both paths, note that it makes
   `a_degraded_micro_sample_does_not_suppress_the_retry` vacuous — I measured
   that fixture producing zero degraded primary samples with the gate on both
   paths, over scales 4-14 and depths 0.70-0.995. That test would need a new
   fixture.
3. **The relative timing gap band** is the one change outside the refinement
   proper. It widens what a walk will follow, which is a phantom-safety surface.
   All negatives are green and the per-step test is tighter than the old fixed
   band, but a second opinion is worth having.
4. **Envelope tests assert counts, not every case, at 10% and 12%.** Deliberate —
   pinning individual configurations out there pins noise — but it is a weaker
   form of assertion than the rest of this file uses.
5. **M4 is the weakest version and `Right` (far edge compressed) the weakest
   direction.** Both bottom out around 12-14%. If more is wanted, the next thing
   to attack is `square_axes`, which normalises the finder's two axes to a common
   module size and so mismeasures a foreshortened finder.

   *Richard's rulings on all five: 1 yes-with-MAJOR-1, 2 keep as is, 3 keep the
   change and fix the claim, 4 correct call no change, 5 confirmed and out of
   scope. All applied below.*

---

## Round 2 — what changed after review

`REVIEW-FEEDBACK-persp.md`: 0 BLOCKER, 2 MAJOR, 8 MINOR, verdict CLEAR
conditional on the MAJORs. Everything below is in `crates/qr-detect/` only.

### MAJOR-1 — the search excursion in the code was not the one in the doc

`search` recentres on its own incumbent each stage *then* narrows, so the
reachable set was the sum of the stage radii: **±2.8 modules for the probe search
(not 1.6) and ±0.741 of a span for the timing fit (not 0.5)**. The behaviour was
fine — Richard could not make it produce a wrong answer — but the containment
argument for a content-agnostic image score rests on that bound, and on an
11-module M1 ±2.8 modules reaches the quiet zone the bound exists to exclude.

Arch's ruling was to **clamp**, so the doc is true by construction and stays true
if someone adds a stage. `search` now clamps every candidate to `radius` of
`start` on both axes, and its doc explains why the clamp is not redundant.

**Envelope delta from the clamp (full 120-config sweep, re-run):**

| keystone | as reviewed | clamped | delta |
|----------|-------------|---------|-------|
| 0-10% | (unchanged) | (unchanged) | **0** |
| 11% | 103 | 101 | -2 |
| 12-14% | (unchanged) | (unchanged) | 0 |
| 15% | 49 | 48 | -1 |
| 16% | 48 | 47 | -1 |
| 17-21% | (unchanged) | (unchanged) | 0 |
| 22% | 28 | 27 | -1 |

**Zero cost at 10%**, which was Arch's revert threshold (>2 configurations), and
zero anywhere at or below 10%. The five configurations lost are all past 11%, in
the ragged region the envelope table already describes as ragged. The clamp
stands. The published table changes in one row: **15% goes 49 -> 48**.

### MAJOR-2 — "tighter than before" was false for the gap band

Corrected in the source comment and in §5 above: tighter per step against a
drifting reference, strictly wider in absolute terms, converging on `[0.5, 2.0)`
against the old fixed `[0.55, 1.45]`. The upper clamp is now **exclusive** of
2.0 (`0.5..2.0`), which was the substantive half: a gap of exactly two modules is
one bridged boundary, and `timing_residuals` maps entry `i` to grid `7 + i`, so
walking through one renumbers every measurement the refinement then fits to. It
was accepted on the nose before. Richard's measured KG-13 movement (+2/-1 over
~810 damaged-timing warps) is recorded in the comment for whoever touches it next.

### Minors applied

| # | change |
|---|--------|
| N1 | tie-break: `vec![affine, refined]`, so `min_by_key`'s first-minimum rule leaves the incumbent in place on a tie. Ties are routine on the rotated-small-module surface where refinement fires spuriously. |
| N2 | `FIT_STAGES` doc no longer mixes the last stage's radius (`R/27`) with its step (`R/81`); the 0.105-modules-on-an-M4 figure is named as the step. |
| N3 | read ceiling corrected: search B is **76** `grid_disagreement` evaluations not 75, and the adoption gate's own 2 evaluations (<=810 reads on an M4) were missing entirely. |
| N5 | the counter test's doc now says outright that a second unit test reaching `detect_from_finder` makes it intermittently flaky, and what to do instead. |
| N6 | the new `?` on the fast path (`timing_residuals` giving up on a grid line parallel to the ray) is now called out in the fast-path comment, with why it cannot fire. |
| N7 | `tilting_does_not_resurrect_a_rejected_candidate` doc: 25 warps per fixture not 20, and "four hand-built negatives" not "every negative in this file". |
| N8 | the 120-config table now states masks 0 and 2 and seed 1234, so it is reproducible. |

N4 needed no change (Richard verified the 20-refinement ceiling structurally and
accepted the measured 1-per-finder as a claim rather than a verified figure).

### Corrections to round 1's own claims

- **Upright cost is "unchanged", not improved.** The 0.519 -> 0.472 was noise;
  Richard measured 0.866 -> 0.866. §Cost now says unchanged, which is the
  stronger statement anyway.
- **Rotation churn.** "Cost, not risk" was slightly too clean. Net exact
  recoveries improve at every module size, but 3 px loses ~20 configurations and
  4 px loses 2 that the old code kept; at 5 px and above the new failure set is a
  strict subset of the old. §Cost now says so.
- **The 4-symbol-span degrees row read 40-45 deg; it is 30-40 deg** on Richard's
  wider axis set. Corrected in the table.
- **The 15% row of the envelope table is 48, not 49**, after the clamp.

### Not a finding, recorded so it is not mistaken for one later

Richard built a bare finder **with both timing patterns correct** and an empty
data area. It is reported as a micro symbol (122/200 before, 155/200 after),
which is correct: that shape has perfect function patterns and a clean quiet
zone, i.e. it *is* a structurally valid Micro QR carrying all-light data. It does
not decode (all-light format info, BCH rejects). Pre-existing — it detects
upright under the old code too — and the new code only extends it to tilt. He
escalated the `micro`-without-`decode` consumer question to Arch as a product
matter.

### Verification after the round-2 changes

All five feature configurations, `qr-video`, `tests-integration`, both clippy
passes and fmt re-run green — counts identical to the table above. The 120-config
sweep was re-run on the final code; the only change from the reviewed version is
the -5 configurations across 11%/15%/16%/22% listed under MAJOR-1.
