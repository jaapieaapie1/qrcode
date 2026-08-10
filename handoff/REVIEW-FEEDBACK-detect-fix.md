# Review Feedback — Step 4: `qr-detect` bug fixes (CI-1, CI-2)

Reviewer: Richard-F
Date: 2026-08-10
Scope reviewed: `crates/qr-detect/` only. `tests-integration/` neither read, edited, nor run.

**Verdict: Step 4 CLEAR**
**Ready for Builder: YES**

BLOCKER: 0 — MAJOR: 1 — MINOR: 5

---

## Verification performed

| Command | Result |
|---|---|
| `cargo test -p qr-detect` | 15 + 21 passed, 0 failed |
| `cargo test -p qr-detect --no-default-features` | 15 + 20 passed, 0 failed |
| `cargo clippy -p qr-detect --all-targets -- -D warnings` | clean (forced rebuild, not a cache hit) |
| `cargo clippy -p qr-detect --all-targets --no-default-features -- -D warnings` | clean |

`git diff HEAD --numstat -- crates/qr-detect`: `finder.rs 13/3`, `grouping.rs 171/5`,
`tests/detect.rs 80/0`. **Zero deletions in the test file** — the "no existing test
modified or deleted" claim is verified mechanically, not taken on trust. The five
deletions in `src` are exactly the ones the request lists. The pre-existing
`ratio_accepts_clean_pattern` assertion (`== 4.0`) still holds under the new
estimator without being touched — no assertion was weakened to fit the change.

I did not run `tests-integration`. The integration-pin claims in the request are
Arch's to confirm.

---

## Must Fix

None.

---

## Should Fix

### MAJOR-1 — `finder.rs:26-34` — the offset-invariance argument is wrong in the mixed regime, and KG-10's stated cause is a misdiagnosis

The doc comment claims the four single-module runs "are differences of two such
chords, so the offset cancels and they stay correct." That is only true when
**both** chords are in the same regime.

The horizontal chord of a square of half-width `a` rotated by θ is
`2a/max(|cosθ|,|sinθ|)` — constant — but only while `|h| <= a·(|cosθ|-|sinθ|)`,
where `h` is the scanline's offset from the centre. Past that the scanline is
cutting corners and the chord tapers linearly in `h`. **The threshold scales with
`a`**, so the core (`a = 1.5m`) enters the taper well before the outer ring
(`a = 3.5m`) does. In that mixed band the `h` terms do not cancel and the estimate
inflates toward `(1 + cot θ)/2` times the truth.

I simulated this: rasterised rotated finder patterns, ran the real
`matches_ratio` shape test, averaged over seven sub-pixel centre offsets, applied
`axis_scale`, compared against the true module.

| module px | 0 deg | 20 deg | 40 deg | 45 deg |
|---|---|---|---|---|
| new (4-run mean) | 0.0% | +0.0 .. +1.5% | **+5.6 .. +8.2%** | -5.0 .. +1.0% |
| old (`total/7`) | 0.0% | -0.0 .. +0.3% | -3.0 .. -6.9% | **-12.4 .. -14.4%** |

Two things follow.

1. **The change is still the right call and I am not blocking it.** `total/7`
   collapses at 45 degrees exactly as the request says, and that is the regime the
   old code could not reach at all. Net envelope is strictly better and the tests
   prove it.

2. **KG-10 is misattributed.** The request calls the residual v20/v40 failure at
   35-40 degrees "a precision floor, not the systematic bias." It is a systematic
   bias — the new estimator's, peaking at 35-40 degrees and vanishing at both 0
   and 45. That is the signature of Bob's own envelope table: v20/v40 fail at
   35-40 but **45 passes**. A precision floor does not switch off at 45 degrees.
   A +7% module overestimate costs `0.07·(D-7)` modules of dimension — 6.3 at
   v20, 12 at v40 — which is precisely the observed miss.

   Fix: reword KG-10 to name the real cause (the mixed-regime bias in
   `matches_ratio`, worst near 40 degrees). Do not write "needs the module
   measured to ~1%" — the error is not noise and averaging more scanlines will
   not remove it. The cheap lead is to suppress or down-weight scanlines whose
   core run has already entered the corner taper (detectable from `runs[2]`
   relative to `total`), so the surviving lines are the ones where both
   estimators agree. That is a Step 5 item, not a Step 4 blocker.

---

### MINOR-1 — `finder.rs:26-34` — the doc should say what it actually proves
Rewrite the offset-invariance sentence to state the condition ("...so long as
both chords are in the same regime; the core enters its corner taper before the
outer ring does, and in that band the cancellation fails"). As written the comment
will convince the next reader that a known error source does not exist.

### MINOR-2 — `grouping.rs:94` — range is stated open, should be closed
`axis_scale` is documented as "in `(0.707, 1.0]`". It attains `1/sqrt(2)`
exactly at 45 degrees; the interval is `[1/sqrt(2), 1.0]`. The 45-degree unit
test asserts equality with `0.5f64.sqrt()`, so the code and the comment disagree
about their own test.

### MINOR-3 — `grouping.rs:176` — `f64::EPSILON` is the wrong degeneracy scale
`det` is in px^2; `f64::EPSILON` is 2.2e-16. A frame with `det = 1e-9` sails
through and yields garbage coordinates. It fails *safe* here (garbage lands
outside the bounds, so nothing is rejected) and `score_triple`'s `|cos| <= 0.30`
plus `leg >= 1.0` gates already guarantee `det >= 0.95`, so this is unreachable
today. Still the wrong constant to leave in a geometry routine — use a scale-
relative threshold (e.g. `1e-9 * leg_top * leg_left`) or drop the check and
document why it cannot fire.

### MINOR-4 — `grouping.rs:230` — the nesting rule is order-dependent, and what makes the order safe is undocumented
`is_nested_in` is only tested against **already accepted** candidates. If a
phantom ever outscored its container, the phantom would be accepted first, the
real symbol would then not be nested inside it, and both would come back — CI-2
again, reversed. What actually prevents this is the `1.0 / min_votes` term in
`score`: the real triple had 22 votes (0.045) against the phantom's 3-7
(0.14-0.33). That term is now load-bearing for CI-2's fix and nothing says so.
Add a line to `group()` noting the dependency, so nobody retunes the score
function and silently reopens CI-2.

### MINOR-5 — no test pins module sizes below 4 px
`scaled_small_module` renders at 4 px/module; nothing exercises 2-3 px. I
checked this concern directly rather than leave it open — see the ruling on the
tiny-module edge case below — and there is no regression, but the guard rail is
missing. Worth one test at 2-3 px/module upright when convenient.

---

## Points examined and cleared

**CI-1 — `axis_scale` math is exactly right for pure rotation.** Derivation
checked independently: a ray `p = (t,0)` from the centre maps to
`(t cosθ, -t sinθ)` in the symbol frame; it leaves `max(|x'|,|y'|) = a` at
`t = a/max(|cosθ|,|sinθ|)`. Identical for the vertical ray. All rings are
concentric and co-oriented, so every run scales by the same factor. `u` is the
unit vector `(cosθ, sinθ)`, so `max(|u.x|,|u.y|)` is `max(|cosθ|,|sinθ|)`
exactly. The cancellation is exact, not approximate, and the 45-degree fold is
real rather than an artefact.

**Shear / perspective — degrades gracefully, cannot blow up.** `u` and `v` are
normalised before use, so `max(|.x|,|.y|)` is confined to `[1/sqrt(2), 1]` no
matter how non-orthonormal the frame is: the worst possible mis-scaling is 41%,
bounded, sign-stable, never zero, never NaN, never negative. And it cannot even
reach the worst case, because `score_triple` has already rejected anything with
`|cos| > 0.30` — roughly 17.5 degrees of shear — before `axis_scale` is called.
Using one global `(u,v)` for three finders that have locally different scales
under perspective is an approximation, but a bounded one, and the pre-existing
`perspective_warp_v2` / `perspective_warp_v7` tests still pass. Cleared.

**Tiny-module regression risk — measured, none found.** My first instinct was
that the 4-run mean must be noisier: `total` is a single outer-edge-to-outer-edge
span, so its two interior quantisation errors cancel, while the 4-run mean
depends on four edges. The simulation says otherwise. Swept 2.0, 2.5, 3.0, 3.5,
4.0, 5.0 px modules across seven sub-pixel centre offsets at 0 degrees: the new
estimator is **exactly 0.0% error in every single case**, while `total/7` shows
up to +0.5% from outer-edge inclusion. The run structure is symmetric for an
upright pattern, so the interior errors cancel in the 4-run mean too. No
regression for small upright codes. Cleared.

**CI-2 — the affine solve is correct.** `p - tl = a·u + b·v` with `u = tr-tl`,
`v = bl-tl`; `det = ux·vy - uy·vx`; Cramer gives `a = (dx·vy - dy·vx)/det`,
`b = (ux·dy - uy·dx)/det`. Both match the code. Correct.

**The `3.5/(dimension-7)` margin is justified, not a magic number.** One frame
unit spans `dimension - 7` modules; the symbol extends 3.5 modules past the
finder centres on each side. Correct by construction.

**A dense sheet of small codes cannot be wrongly rejected.** I worked the
arithmetic rather than trust the assertion. Take symbols A and B, non-overlapping.
B's nearest finder centre sits at least 3.5 modules beyond B's own edge, which is
at least at A's edge — so at minimum 3.5 modules outside A's boundary even with
**zero** quiet zone between them, and 11.5 modules out with the ISO-mandated 4
each. The bound is `1 + 3.5/(dim-7)`, i.e. exactly A's edge. B's centres always
land outside it, and the rule needs **all three** inside. Safe with margin, and
safe in the degenerate case too: if A's own dimension were mis-snapped as low as
21, the margin widens to 0.25 frame units (3.5 modules of a 14-module frame) and
an adjacent B still sits at 1.5 or beyond. Cleared — the conservatism is real,
not asserted.

**A phantom with one centre outside does slip through.** By construction, and
correctly so — the all-three rule is what makes the adjacency safety above hold.
Note the observed phantom landed at `b = 1.0021` against a bound of 1.092; had it
landed at 1.10 the rule would not have fired. The nesting rule treats the
symptom, and it is the right symptom to treat at this step, but it is thin at the
boundary. The cause is the weak-cluster problem — see the ruling below.

**Not consuming `used[]` on rejection is correct.** A nested triple is skipped
with `continue` before its indices are marked, so those finders stay available to
other triples. Right behaviour.

---

## Rulings on Bob's open questions

**Q1 — KG-10 / timing-pattern cross-check. Ruling: NO, do not add it now, and
the question is built on a wrong premise.**

The residual 35-40 degree failure at v20/v40 is not a precision floor. It is the
new estimator's own systematic bias in the mixed chord regime — measured above at
+5.6 to +8.2% at 40 degrees, dead-on for the observed dimension miss, and
consistent with 45 degrees passing while 40 fails. A second image traversal,
a new damage-sensitivity surface, and a dependency on the most fragile structure
in the symbol are a heavy price to pay for a problem whose actual cause is a
scanline-selection detail in code you already own. Reject the corner-tapered
scanlines first and re-measure; only if the bias survives that does the
timing-pattern count earn its cost. Update the KG-10 wording accordingly — as
written it will send the next engineer down the expensive path.

Whether 30 degrees at v40 is *good enough to ship* is Arch's call, not mine.

**Q2 — raise the `finder::cluster` vote floor. Ruling: NO, and not in this step
either way.**

Out of scope for Step 4 — you were right to leave it. On the merits: raising
`n >= 3` is a blunt instrument that would cut real finders before it cut
phantoms. In my simulation a genuine finder at a 2 px module yields only about
four or five horizontal scanline hits at 20-45 degrees. A floor of 4 or 5 starts
dropping legitimate low-resolution and heavily-rotated codes — the exact
population this step just spent its effort rescuing. If the cause is worth
attacking, attack it relatively (score clusters against the strongest cluster in
the image, or against the median) rather than with a fixed absolute floor. Log it
as a Known Gap; do not implement it on this step's ticket.

---

## Escalate to Architect

- **KG-10 wording and priority.** The cause is misdiagnosed in the request (see
  MAJOR-1). Arch should decide whether the corner-taper scanline filter goes into
  Step 5 or waits for a real camera case, but KG-10's text must be corrected
  either way so the wrong fix is not queued.
- **Integration pins.** Bob reports both `#[ignore]`s can come off and
  `rotation_dimension_underestimate_is_still_present` should be deleted. Outside
  Bob's write scope and outside mine — I neither ran nor read `tests-integration`.
  Arch owns it.
- **`cargo fmt`.** The crate was already not fmt-clean before this step
  (`binarize.rs`, `geometry.rs`, `sampler.rs`, plus untouched lines in
  `grouping.rs`). Bob correctly did not reformat lines he did not otherwise
  change. Someone should schedule a one-shot fmt pass on its own commit so it
  never contaminates a review diff again.

---

## Cleared

Both bugs are genuinely fixed at the mechanism level, not papered over: the
rotation correction is analytically exact for pure rotation and provably bounded
otherwise, and the nesting rejection cannot swallow a real adjacent symbol. All
36 tests pass in both feature configurations, clippy is clean in both, no
existing test was modified or deleted, and no regression appears for small
modules under direct measurement. The one MAJOR is a wrong explanation attached
to correct code — it costs nothing today and would cost the next engineer a week.

Step 4 CLEAR.
