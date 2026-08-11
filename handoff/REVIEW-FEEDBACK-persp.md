# Review Feedback — Step 10: Micro QR perspective refinement

Date: 2026-08-11
Reviewer: Richard-P
Scope reviewed: `git diff HEAD -- crates/qr-detect` (`src/micro.rs` +658/-, `tests/micro_detect.rs` +403/-). Nothing else.

**Verdict: Step 10 CLEAR.**

Ready for Builder: YES — conditional on the two MAJOR items below being corrected
inline before commit. Both are wrong statements in comments, not wrong code. No
re-review needed.

Counts: **0 BLOCKER, 2 MAJOR, 8 MINOR.**

Stray files: none. `crates/qr-detect/tests/` holds `detect.rs` and
`micro_detect.rs` only; the `zz_scratch.rs` noted earlier is gone. Nothing
deleted, nothing touched but this file.

---

## What I did

I did not take Bob's numbers on trust and I did not `git stash` — a sibling Bob
was live in the same working tree when I started, and stashing it would have
pulled the floor out from under him. Instead I built the "before" detector
outside the repo: `git show HEAD:crates/qr-detect/src/*.rs` into a standalone
`qr-detect-old` package pointed at the unchanged `qr-core` / `qr-decode`, and a
scratch crate depending on both old and new, carrying a verbatim copy of the new
`micro_detect.rs` so both sides run identical fixtures and identical helpers.
Every "measured" figure below is mine, from that harness. The repo was never
modified.

---

## Must Fix — MAJOR

### M1. The search excursion in the code is not the excursion in the doc comments

`micro.rs:99-113` (`FIT_RANGE`, `FIT_STAGES`), `micro.rs:136-146` (`PROBE_RANGE`),
and the `refine_bottom_right` doc.

`search` recentres on its own incumbent at every stage and only *then* narrows,
so the total distance the returned point may sit from `start` is the **sum** of
the stage radii, not the first radius:

- probe search: `1.6 + 0.8 + 0.4 = ±2.8 modules` per axis (≈3.96 diagonal),
  not the "range of 1.6 modules" the constant claims and not the "range of
  barely more than a module" `refine_bottom_right` claims. That is 1.75x the
  stated bound, and on an M1 (11 modules) ±2.8 modules puts the far corner
  comfortably inside the quiet zone — the exact place the doc says the bound
  exists to keep it out of.
- timing fit: `0.5 * (1 + 1/3 + 1/9 + 1/27) = ±0.741 of a span`, not ±0.5.

This is the load-bearing sentence of the whole safety argument for a
content-agnostic image score, and it is the one thing you asked me to push on.
The *behaviour* is fine — I could not make it produce a wrong answer (see
"Evidence" below) — but the number a future Bob will read before touching
`PROBE_STAGES` is wrong by a factor of nearly two.

Fix: either state the real bound on the constants (`PROBE_RANGE` is a *stage*
half-width; total excursion is `PROBE_RANGE * (2 - 2^(1-PROBE_STAGES))`), or
clamp the returned point to `PROBE_RANGE` of `start` in `search` and keep the
doc as written. I prefer the clamp — the doc is then true by construction and
cannot rot when someone adds a stage.

### M2. "A tighter per-step test than before" is false for the gap band

`micro.rs:~703-730` (`walk_timing`) and REVIEW-REQUEST §5.

```rust
if !(0.55 * scale..=1.45 * scale).contains(&gap) || !(0.5..=2.0).contains(&gap) {
```

It is tighter *relative to the tracked scale*. In absolute terms it is strictly
wider: the EMA `scale = 0.5 * (scale + gap)` converges to 2.0 from above-unity
gaps and to 0.5 from below, so the accepted absolute band converges to
**[0.5, 2.0]** against the old fixed **[0.55, 1.45]**. Three consecutive gaps at
the band ceiling take `scale` from 1.0 to 1.394 (1.0 -> 1.225 -> 1.338 -> 1.394),
after which a **2.0-module gap is accepted**. An M4 walk has up to 10 gaps, so
that state is reachable.

A 2.0-module gap is precisely one bridged boundary. That matters beyond phantom
safety: `timing_residuals` maps entry `i` to grid `7 + i`, so swallowing a
bridged boundary silently corrupts every measurement the refinement then fits
to. The `0.5..=2.0` clamp is also **inclusive** at exactly the value that does
this.

Measured, KG-13 surface, one damaged timing module x {M3, M4} x k=9..size x
{6, 10, 14} px/module x 5 tilts x {0, 6, 12}% keystone (~810 warps):

| | crop detections reported |
|---|---|
| before | 21 |
| after | 22 (2 new, 1 gone) |

So the practical widening is 2 warps in 810, and every one of them is a
KG-13-class smaller-crop already logged. Keep the change — it is what buys
M2/M3 17%->23% and M4 12%->14%. But:

Fix: (a) correct the claim in the source comment and in the request — it is
tighter per step against a *drifting* reference and looser in absolute terms;
(b) make the upper clamp exclusive of 2.0, or drop it to ~1.8, so a
single-bridged boundary can never be walked through; (c) note under **KG-13** in
BUILD-LOG that the relative band moves it by +2/-1 over this fixture set, so the
next person to touch it knows which way it went.

---

## Should Fix — MINOR

- **N1. `micro.rs:~468` — tie-break adopts the refinement.** `vec![refined,
  affine]` with `min_by_key`, which returns the *first* minimum, so an exact tie
  on `(function_mismatches, grid_disagreement)` adopts the searched map over the
  settled one. Ties are not exotic: you document that rotated 3-5 px symbols
  enter refinement without needing it, and those tie at (0, 0) routinely. Swap
  to `vec![affine, refined]` — one word — so the incumbent keeps the frame when
  nothing has been proven against it.

- **N2. `micro.rs:~112` — `FIT_STAGES` doc.** "after four the lattice is
  `FIT_RANGE / 27` of a span" mixes the last stage's *radius* (R/27) with its
  *step* (R/81). The "tenth of a module on an M4" figure that follows is the
  step. Say which.

- **N3. Read-ceiling arithmetic is short.** Search B is **76** `grid_disagreement`
  evaluations, not 75 — the incumbent is scored once before the loop. And the
  adoption gate runs `grid_disagreement` on up to 2 candidates, another
  `2 x 5 x 81 = 810` reads on an M4, which the "worst-case bound per refined
  frame" list omits entirely. Everything else checks out: 196 fit candidates and
  0 image reads (verified — `timing_residuals` touches no `bin`), `75 x 5 x
  ceil(dim/2)^2` = 30,375 (M4) / 13,500 (M1) exactly as stated, 2 x 82 = 164
  function reads, 4 x 17 = 68 ring reads, 289 sample reads.

- **N4. The 20-refinement ceiling is real.** Verified in code, not prose:
  `detect_from_finder` is `for (u0,v0) in frames` (4) x `for tweak in
  ANGLE_TWEAKS` (5), and `note_refinement()` fires at most once per `try_frame`.
  Ceiling 20, structurally. Your "measured 1 per finder" I cannot check from
  outside — `REFINEMENTS` is `#[cfg(test)]` and crate-internal — so it stays your
  claim, not a verified one. That is fine; the ceiling is the number that
  matters and it holds.

- **N5. `micro.rs:~866-881` — `REFINEMENTS` is a process-global counter.**
  `the_refinement_runs_when_tilted_and_not_when_upright` snapshots it and asserts
  equality across a loop, in a parallel harness. It is safe **today** — I checked
  every `#[cfg(test)]` module in the crate; `binarize.rs`, `finder.rs`,
  `geometry.rs`, `grouping.rs` and the other three tests in `micro.rs` never
  reach `detect_from_finder`, and `lib.rs` has no unit tests at all. The first
  unit test anyone adds that does will make this flaky, silently and
  intermittently. One line in the test doc saying so would be cheap insurance.

- **N6. `micro.rs:~427` — new rejection path on the fast path.**
  `timing_residuals(&affine, ...)?` returns `None` and kills the frame if any
  candidate maps a grid line parallel to the walk ray (`denom.abs() < 1e-9`).
  The old code had no such exit before sampling. I believe it is unreachable for
  any quad a real finder produces, and the threshold is tight enough that it
  cannot fire on noise, but it is a new `None` on a path documented as
  "unchanged". Worth a word in the fast-path comment.

- **N7. Test-file arithmetic.** `tilting_does_not_resurrect_a_rejected_candidate`
  says "which is 20 warps per fixture"; it is 5 tilts x 5 shrinks = **25** (the
  request has this right, the source comment does not). It also says "every
  negative in this file is re-run" — it is four hand-built fixtures, not every
  negative in the file.

- **N8. The 120-config table in `perspective_envelope`'s doc is not
  reproducible as written.** It does not say which two masks or which seed. I
  reproduced it independently with masks 0 and 1, seed 1234, and got numbers
  that track but do not match (below). State the mask indices and seed, or say
  the table is indicative.

---

## Evidence

### 1. Fast-path integrity — CLEAR

Structurally the fast path is unchanged. `function_mismatches` moved from
reading a sampled `BitMatrix` to reading the transform directly, but through the
same `sample_at` rounding at the same `map(x+0.5, y+0.5)` points, so it is the
same count on the same pixels; `sample_symbol` is the old loop lifted verbatim.

**Can a frame that used to decode now fail?** Not through the reject gate: the
affine map is *always* a candidate and `min_by_key` picks the lower
`function_mismatches`, so the winner's mismatch count is `<=` the affine map's
and `mismatches > MAX_FUNCTION_MISMATCHES` can only fire where it fired before.
The residual risk is adoption of a refined map that scores tied-or-better and
samples worse. I swept the acknowledged surface — rotation, where refinement
fires spuriously — 4 versions x 0-90 deg in 2.5 deg steps x 7 module sizes, 1036
configs per side:

| px/module | before exact | after exact | new-only failures |
|-----------|--------------|-------------|-------------------|
| 3  |  63/148 |  81/148 | ~20 |
| 4  | 102/148 | 129/148 | 2 (M2@30, M2@72) |
| 5  | 123/148 | 140/148 | 0 (new failures are a strict subset of old) |
| 6  | 137/148 | 146/148 | 0 (strict subset) |
| 8  | 146/148 | **148/148** | 0 |
| 12 | 148/148 | **148/148** | 0 |
| 20 | 148/148 | **148/148** | 0 |

Net strongly positive at every size. The churn is real but confined to 3-4
px/module, below the documented rotation envelope (10 px, and 5 px for
`rotation_at_a_small_module_size`), and at 5 px and above the new failure set is
a strict subset of the old. Your "that is cost, not risk" is very nearly right —
it is cost plus a little churn at the bottom of the size envelope. Say so.

**Upright-never-refines is non-vacuous.** The counter test drives 16 real
fixtures (4 versions x 4 module sizes) through `detect_from_finder`, asserts
each detects and every module is exact, *then* asserts the counter did not move
— it cannot pass by the loop being empty or the detector being dead. The second
half shears an M3 and asserts the counter *does* move, so it cannot pass by the
refinement being unreachable. Both halves are load-bearing and both are real.
Accepted.

### 2. Search soundness — CLEAR, with M1

- Stage-A objective is correct. A homography maps lines to lines, so taking the
  candidate's images of the two endpoints of grid line `k` and intersecting that
  segment with the walk ray is exact, not an approximation. The intersection
  algebra (`t = cross2(p0 - origin, edge) / cross2(dir, edge)`) is right, and
  `TimingFit::new` builds `origin`/`dir` from the same `grid_point(..., t, 0.5)`
  the walk sampled along, so `predicted` and `measured` are in the same units —
  modules. `affine_worst * module` converts to pixels for the 0.45 gate.
  Consistent.
- `search` is monotone: `stage_score` is seeded from the incumbent and only
  strict improvement (`s < stage_score`) displaces it, so the result never
  scores worse than `start`. Odd lattices additionally re-test the incumbent
  point itself. `PartialOrd` on `f64` handles NaN safely by never improving, and
  `timing_residuals` cannot return a NaN `sse` because it rejects non-finite
  `predicted` first.
- Candidate counts as stated: 4 x 7^2 = 196, 3 x 5^2 = 75. Resolutions as
  stated: fit step R/81 = 0.105 modules on an M4, probe step 0.2 modules.
- **Stage-B content-agnosticism, probed as asked.** I built M4 fixtures with the
  far quadrant overwritten — solid dark, solid light, checker at module pitch,
  vertical stripes, horizontal stripes — the patterns most likely to give
  `grid_disagreement` either no signal (solid) or a periodic false minimum
  (stripes at module pitch), x 3 module sizes x 5 tilts x 4 keystones:

  | | wrong-grid detections, before | after |
  |---|---|---|
  | solid dark BR quadrant, 4% | 13/15 | **8/15** |
  | solid dark BR quadrant, 8% | 15/15 | **6/15** |
  | checker at module pitch, 8% | 15/15 | **0/15** |
  | vertical stripes, 8% | 15/15 | **0/15** |
  | horizontal stripes, 8% | 15/15 | **3/15** |

  The new code produces **fewer** wrong grids than the old one on every single
  adversarial row. Solid dark is the worst case as predicted — the probe score
  goes flat and the search correctly refuses to move (ties do not displace the
  incumbent), leaving the timing fit's ~1-module error — but it degrades to
  no-decode, not to a wrong answer.
- **Nothing ever answers wrongly.** 520 pinhole scenes on the decodable M1
  golden (5 distances x 13 angles x 8 tilt axes): 298 decoded correctly,
  12 located-not-decoded, 210 not located, **0 wrong payloads**. It fails
  closed.

### 3. Phantom safety — CLEAR

`tilting_does_not_resurrect_a_rejected_candidate` (100 warps) and
`a_tilted_qr_symbol_yields_no_micro` (60 warps) both green in all five configs.

I also built the thing you did not: a bare finder with separator **and both
timing patterns correct**, empty data area, 4 sizes x 2 scales x 5 tilts x 5
keystones. It is reported as a micro symbol — 122/200 before, 155/200 after. It
should be, and the ring check cannot and must not stop it: that shape has
perfect function patterns and a genuinely clean quiet zone, which is to say it
is a structurally valid Micro QR symbol carrying all-light data. It does not
decode (format info is all-light, BCH rejects), so with `decode` on nothing
reaches a caller. This is pre-existing — it detects upright under the old code
too — and the new code only extends it to tilt, which is the point of the step.
Not a finding. Recorded so nobody mistakes it for one later.

### 4. Gap band and the upright path — CLEAR, with M2

For the first gap `scale == 1.0`, so `0.55..=1.45` intersected with `0.5..=2.0`
is bit-identical to the old band: an upright walk's first step is untouched, and
a walk whose gaps stay near 1.0 keeps `scale` near 1.0 and behaves as before.
That is why the 15 rotation/upright micro tests and `tests-integration`'s
`micro_image` (12) and `micro_golden` (3) are all green.

Whether a relative band can bridge a gap the fixed band rejected: **yes**, see
M2 — the answer to your question 3 is that your instinct to ask was right and
your justification for it was not. Measured effect is 2 extra KG-13-class crops
in 810 damaged-timing warps. KG-13 is widened, marginally, and should say so.

### 5. Envelope honesty — CLEAR

I did not stash. I reproduced the **full** 120-configuration sweep on both
detectors (4 versions x 3 module sizes {8,12,20} x 2 masks {0,1} x 5 keystone
directions), which is more than the slice asked for and cost the same:

| keystone | your before | mine before | your after | mine after |
|----------|-------------|-------------|------------|------------|
|  0% | 120 | 120 | 120 | 120 |
|  2% | 116 | 117 | 120 | 120 |
|  3% | 111 | 112 | 120 | 120 |
|  4% |  93 |  95 | 120 | 120 |
|  5% |  62 |  61 | 120 | 120 |
|  6% |  40 |  40 | 118 | 118 |
|  8% |  20 |  24 | 117 | 117 |
| 10% |  14 |  17 | 111 | 113 |
| 12% |   1 |   1 | 104 | 104 |
| 13% |   0 |   0 |  77 |  80 |
| 15% |   0 |   0 |  49 |  48 |
| 20% |   0 |   0 |  33 |  33 |

Independent reproduction, within 0-4 configurations everywhere (mask/seed
choice, see N8). **The table is honest, including the shape of the raggedness.**
The headline claim — every configuration exact to 5% where it was 1-4% before —
reproduces exactly.

The M4 x `Right` slice you called the weakest corner: exact 6/6 through 10%,
5/6 at 6% (the raggedness), 2/6 at 12%, 0/6 at 14%. "Bottoms out around 12-14%"
is right; strictly the first failure is at 6%, and 12% is where it collapses.

M1 on a screen, pinhole, decoded payload asserted:

| | 2 spans before | 2 spans after | 4 spans before | 4 spans after |
|---|---|---|---|---|
| first angle with any failure | **10 deg** | **30 deg** (3/4) | **15 deg** | **30 deg** (3/4) |
| first angle with total failure | 20 deg | 35 deg | 35 deg | 45 deg |

Your "before" column reproduces exactly — 10 deg at 2 spans, 15 deg at 4 spans,
which is the Owner's report. Your "after" column for 2 spans (30-35 deg)
reproduces. Your "after" for 4 spans reads 40-45 deg; on my axis set one of four
scenes already drops at 30 deg, so that row is a notch optimistic. Not worth a
finding on its own — you flagged the edge as ragged — but the row should read
30-40. What actually matters is unarguable: **10 and 15 degrees are 4/4 at every
camera distance, where before they were 1/4 and 2/4.** The Owner's bug is fixed.

### 6. Cost — CLEAR

Release build, 348x348, M4 at 12 px/module, 200 iterations x 5 reps, best-of:

| fixture | before | after |
|---------|--------|-------|
| upright | 0.866 ms | 0.866 ms (exact) |
| 10% keystone | 0.863 ms (wrong grid) | 0.999 ms (exact) |

My absolute numbers are higher than yours (different machine/profile); the
ratios are what I am checking. **Upright is unchanged to three decimal places** —
I do not reproduce the 0.519 -> 0.472 improvement you reported, so drop that
claim and say "unchanged", which is both true and the stronger statement. The
+15% on a refined frame reproduces exactly (yours 0.433 -> 0.496, mine 0.863 ->
0.999). Paying 15% on a frame that previously returned a wrong grid, to return
the right one, needs no defending.

### 7. Suites — all green, all run by me

| command | result |
|---------|--------|
| `cargo test -p qr-detect` | 23 + 24 + 28, green |
| `cargo test -p qr-detect --no-default-features` | 19 + 23, green |
| `cargo test -p qr-detect --no-default-features --features decode` | 19 + 24, green |
| `cargo test -p qr-detect --no-default-features --features micro` | 23 + 23 + 26, green |
| `cargo test -p qr-detect --all-features` | 23 + 24 + 28, green |
| `cargo test -p qr-video` | 7, green |
| `cargo test -p tests-integration` | 84 across 10 suites, green (incl. `micro_image` 12, `micro_golden` 3, `warped` 15, `video_session` 9) |
| `cargo clippy -p qr-detect --all-targets -- -D warnings` | clean |
| `cargo clippy -p qr-detect --all-targets --no-default-features -- -D warnings` | clean |
| `cargo fmt --all -- --check` | clean |

Every count matches what you reported.

---

## Rulings on your five open questions

**1. `grid_disagreement` reads the image with no idea what it should contain —
is the gating enough?**
**Yes, with M1 fixed.** The gating holds under everything I could throw at it:
300 adversarial-content warps where the new code produces fewer wrong grids than
the old on every row, 520 pinhole scenes with zero wrong payloads, 160 negative
warps clean. But your containment argument rests on a bound the code does not
have (±2.8 modules, not 1.6). Correct the bound or clamp the search — the
conclusion survives either way, the justification does not survive as written.

**2. `MAX_QUIET_ZONE_DARK` gating the refinement only.**
**Keep it as it is.** You made the right call and for the right reason. The
affine map cannot move the far corner, so it has nothing to lie about that the
function-pattern budget does not already catch; the refinement can, so it is the
one that owes evidence. Putting the ring on both paths to buy symmetry, at the
cost of making `a_degraded_micro_sample_does_not_suppress_the_retry` vacuous,
trades a settled regression test for a tidier diagram. No. Log the asymmetry and
your measurement (zero degraded primaries over scales 4-14, depths 0.70-0.995)
in BUILD-LOG so the next person does not re-litigate it from scratch.

**3. The relative timing gap band.**
**Keep the change, fix the claim — see M2.** It is not a tighter test; it is
tighter against a drifting reference and looser in absolute terms, converging to
[0.5, 2.0] against the old [0.55, 1.45]. Measured cost is 2 extra KG-13-class
crops in 810 warps, which is a fair price for M4 12%->14% and M2/M3 17%->23%.
Make the upper clamp exclusive of 2.0 so a single bridged boundary can never be
walked through — that is the value that corrupts `timing_residuals`' index-to-
grid mapping, and it is currently accepted on the nose.

**4. Envelope tests assert counts, not every case, at 10% and 12%.**
**Correct call, no change.** Pinning individual configurations at the ragged
edge pins noise, and noise-pinned tests get deleted by the third person who
trips over them. Your floors have the right amount of slack: 33/40 and 30/40
against measured equivalents of ~37.7/40 and ~34.7/40, so roughly 4-5
configurations of headroom — enough to survive a rounding change, tight enough
to catch a real regression. The exact assertions still cover 0-5% over all 40,
which is where the guarantee actually lives.

**5. M4 weakest, `Right` weakest, `square_axes` next.**
**Confirmed and agreed, and out of scope for Step 10.** My slice: M4 x `Right`
is 6/6 through 10%, 2/6 at 12%, 0/6 at 14%. Your diagnosis is the right one — a
foreshortened finder normalised to a common module size is a first-order error
that no amount of far-corner search can undo, because it poisons `module` and
therefore the ray scale every measurement is expressed in. That is a separate
step with its own brief. Log it as a known gap; do not start it here.

---

## Escalate to Architect

- **BUILD-LOG is not touched by this change** and needs three entries folded in
  by Arch: (a) **KG-13** moves by +2/-1 over the damaged-timing fixture set as a
  result of the relative gap band; (b) the quiet-zone asymmetry from ruling 2,
  with Bob's measurement, so it is not re-argued; (c) a new known gap for
  `square_axes` / foreshortened finders from ruling 5, since that is where the
  next 10 points of envelope are.
- **Detection without `decode` reports structurally-valid-but-empty symbols more
  often now** (122/200 -> 155/200 on the bare-finder-plus-timing shape, see §3).
  This is correct behaviour and pre-existing, but the `micro`-without-`decode`
  consumer now sees more of it. If any downstream treats a `MicroDetection` as
  "there is a symbol here" without checking `decoded`, that is a product
  question, not a code one.

---

## Cleared

Reviewed the full `crates/qr-detect` diff — the residual trigger, both search
stages, the adoption gate, the quiet-zone ring, the gap-band change and all 403
lines of new test — against an independently rebuilt pre-change detector.
The Owner's bug is genuinely fixed (10 and 15 degrees go from 1/4 and 2/4 to 4/4
at every camera distance), the envelope table reproduces to within 0-4 of 120
configurations, upright cost is unchanged to three decimal places, the reject
gate cannot be stricter than it was, no adversarial fixture produced a wrong
answer in 520 scenes, and all five feature configurations, both clippy passes
and fmt are clean. Two comments state bounds the code does not hold to. Fix
those and ship it.
