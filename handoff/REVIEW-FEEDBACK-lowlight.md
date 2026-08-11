# Review Feedback — Step 12a, low-light / glare robustness
Date: 2026-08-11
Reviewer: Richard-LL
Scope reviewed: `git diff HEAD -- crates/qr-detect` (binarize.rs, lib.rs). Nothing else.

**Verdict: Step 12a CLEAR**
Ready for Builder: YES (no Must Fix in the code)

Counts: **0 BLOCKER, 1 MAJOR, 7 MINOR.**
The MAJOR is a wrong number in this handoff document, not in the code. It must be
corrected before anyone lifts that rule into user-facing guidance.

---

## How I verified

I did not take the envelope tables on trust. I built an independent
reimplementation of the degradation model from the prose in §1 of the review
request — my own PRNG, my own seeds, my own illumination functions — and ran it
against a clean `git worktree` at HEAD and against the working tree. Everything
below is my measurement, not Bob's, unless it says otherwise. Harness and raw
output live in the session scratchpad; nothing was added to the tree, and I
modified no file under `crates/`.

Where I needed to test a counterfactual (retry at the wide margin; smoothing
ungated; a median instead of a quartile) I did it on a *copy* of the tree in
scratchpad. `git status` on the real repo is unchanged from when I started.

---

## What reproduced

### 1. The envelope (§5) — reproduced, closely

| grid | Bob | mine (independent seeds) |
|---|---:|---:|
| no-glare, before | 469 / 2160 | **460 / 2160** |
| no-glare, after | 1227 / 2160 | **1226 / 2160** |
| cells improved | 291 | 309 |
| cells regressed | 1 | **0** |

Full 2160-cell grid both sides. The `v5 8px` slice matches his printed table
cell-for-cell on five of six rows, differing by one seed on two cells. This is
as close as two independent implementations of the same model get. The claimed
envelope is real.

### 2. The one regression — acceptable, and structurally bounded

Characterised, and the conclusion is stronger than "one seed of one cell".

* In my run the named cell (`M2 4px linear, delta 60, sigma 12`) reads 1/3
  **both** before and after — it is a coin-flip cell in either build, which is
  exactly what "marginal" means.
* Across all 2160 cells with a different seed stream, **zero** regressions.
* The mechanism confines it. The primary rung is byte-identical arithmetic:
  `22/25` reduces from `88/100`, and `can_be_dark`'s `lo*25 < hi*22` is the same
  predicate as the old `lo*100 < hi*88`. A frame the primary decoded returns
  before the retry is ever reached. So a regression is only reachable on frames
  where the primary already failed **and** the old retry happened to succeed —
  it can never take away a frame that used to decode on the first pass. That is
  a bounded class, and the grid says it is a very small one.

Acceptable. Do not chase it; raising the retry margin toward 97 to suppress it
would trade a measured 257-cell win for a 24-cell one (below).

### 3. Margin layering (88 then 96) — the non-ordering is real

I built a variant with `NARROW_MARGIN = 22` (retry keeps the wide margin,
denoise gate untouched) and ran the whole grid:

| retry margin | grid |
|---|---:|
| 88 (wide) | 644 / 2160 |
| **96 (narrow, shipped)** | **1226 / 2160** |

Per cell, narrow better on **257**, wide better on **24**. In the region Bob
calls out — `delta 60`, `sigma >= 8`, 108 cells — wide wins **16**, narrow wins
**29**, tie 63. His 57/108 figure is at module-error level and mine is at decode
level, so the numbers differ; the **finding** is the same and it is the one that
matters: **the two margins do not order.** Layering rather than replacing is the
right call, and since the primary is untouched the wide margin is always tried
first. Confirmed.

`fits_u32`: **code unchanged** (verified against the diff — the hunk touches the
doc comment only, the function body has no `+`/`-` lines). The bound is
`255 * count * MARGIN_DEN`, which is the binding term at *any* margin because
every margin is `<= MARGIN_DEN`; `24/25` therefore routes identically to
`22/25`, and `accumulator_routing_boundaries` is untouched arithmetic. Claim
confirmed.

Pinned tests: I checked the diff hunk boundaries directly. The only hunk inside
`mod tests` before the new block is `the_two_accumulator_widths_agree` (arity
update, now sweeping **both** margins — strictly widened). `a_solid_block_does_not_hollow_out`,
`a_solid_block_at_worn_print_contrast_does_not_hollow_out`,
`adaptive_survives_a_vignette_to_the_documented_depth`,
`the_retry_window_holds_a_vignette_the_primary_loses`,
`a_frame_too_flat_to_threshold_is_settled_at_any_radius` and
`accumulator_routing_boundaries` have no diff hunks at all, and
`crates/qr-detect/tests/` is untouched in its entirety. All green. Claim
confirmed.

### 4. The denoise gate — both load-bearing claims verified

* **The gate is necessary.** I removed it (unconditional `denoise` on the retry
  rung) and `a_deep_vignette_is_recovered_by_the_retry_pass` **FAILS**
  ("expected exactly one detection"). Exactly as reported.
* **Quartile over median is justified.** With `NOISE_QUANTILE = 2`,
  `the_noise_estimate_separates_sensor_noise_from_texture` **FAILS** on
  "3px modules, delta 156, alternating true" — the estimator calls a clean
  3 px/module checkerboard noisy, which is precisely the failure the quartile is
  there to avoid. Worth stating precisely for the record: the pinned
  *deep-vignette* fixture itself still passes under a median, so the quartile is
  a **safety margin** rather than a rescue of that specific test. §4's wording
  ("a median needs more quiet pixels than a dense 3 px frame has") is accurate;
  the fixture that actually falls over is Bob's own new synthetic checkerboard,
  and it is good that he wrote it.
* **Estimator math** checks out. `|2p[x] - p[x-1] - p[x+1]|` annihilates any
  locally linear field, so vignette and ramp contribute nothing; for white noise
  of s.d. `s` the second difference has variance `6 s^2` and the lower quartile
  of `|.|` is `sqrt(6) * s * erf^-1(0.25) * sqrt(2)` ~ `0.79 s`, which matches
  the measured 4 / 8 / 11 / 15 at sigma 4 / 8 / 12 / 16.
  `below * 4 < n` is a correct strict lower-quartile test, and ties resolving to
  "quiet" is the safe direction.
* **Cost when not firing.** `noise_reaches` is called only from
  `binarize_retry`, so the happy path never touches it — verified by reading the
  call graph, and by the benchmark (happy path *down* 0.2%). It reads one row in
  sixteen, 57 510 samples at 720p. The measured price shows up only on the
  cluttered miss path: see §6 below, where it is larger than reported but still
  well inside budget.

### 5. Glare (§5) — geometry reproduced, one number wrong

My proximity sweep reproduces his table **exactly** for every row I ran
(v2/v5/M4/M2 at blob 2/4/8), and — importantly — is **identical before and
after**, confirming this is a geometry limit the rework does not move.

```
              offset  0  1  2  3  4  5  6  7  8 10 12   (modules)
v2 / v5  blob 2 |     .  .  Y  Y  Y  Y  Y  Y  Y  Y  Y
v2 / v5  blob 4 |     .  .  .  Y  Y  Y  Y  Y  Y  Y  Y
v2 / v5  blob 8 |     .  .  .  .  .  .  Y  Y  Y  ...
```

Glare-on-finder total loss confirmed as physics: 0/4 at **every** blob size, at
every placement offset 0, before and after. One saturated module inside the
3-module finder core breaks the `1:1:3:1:1` run and no thresholding rule puts it
back. Correctly stated as unfixable.

Glare grid slice (3 placements x 4 sizes x 4 symbols x 2 sigmas, 192 attempts):
**92 -> 97, zero regressions.** Same direction and same zero-regression result as
his 344 -> 469 of 1152. The per-version death points also match: v5 survives an
8-module interior blob and dies at 15; v2 dies at 8 before (and now survives it);
M2 dies at 2.

**But the stated rule does not fit his own data — see MAJOR-1.**

### 6. Benchmarks — every row inside 5%, miss path larger than reported

Apple M4, `bench` profile, medians. "before" is the clean worktree at HEAD, run
in the same session.

| bench | README | before | after | Δ vs before |
|---|---:|---:|---:|---:|
| happy path: 1 v5 @8px | 687.17 µs | 689.17 µs | 687.75 µs | −0.2% |
| happy path, single pass | 688.29 µs | 689.21 µs | 685.62 µs | −0.5% |
| rotated 30° | 688.04 µs | 688.12 µs | 690.88 µs | +0.4% |
| **miss: no symbol (primary + retry)** | 1.12 ms | 1.12 ms | **1.15 ms** | **+2.7%** |
| miss, single pass | 563.79 µs | 563.62 µs | 562.17 µs | −0.3% |
| miss: flat frame | 320.38 µs | 319.83 µs | 316.67 µs | −1.0% |
| miss: flat frame, single pass | 314.29 µs | 314.79 µs | 316.42 µs | +0.5% |
| detect_micro_luma: 1 M4 | 760.46 µs | 768.62 µs | 760.29 µs | −1.1% |
| detect_all_luma: 1 v5 + 1 M4 | 1.12 ms | 1.24 ms¹ | 1.15 ms | — |
| video: steady-state ROI | 324.25 µs | — | 325.92 µs | +0.5% vs README |
| video: full-scan cadence | 688.25 µs | — | 699.88 µs | +1.7% vs README² |

¹ that before-row was unstable (max 91.54 ms — machine hiccup); disregarded.
² measured in a visibly noisier run whose happy-path row also read +3%.

Every row is inside the 5% budget, so the DoD holds. The miss path is
**+2.0 to +2.7%** across two runs, not the +0.9% reported — three times the
stated figure. It is still the right side of the budget and it is the *only*
row that moved, and it moved by roughly the cost of reading 57 000 pixels, which
is what the gate is. Reported as MINOR-1, not as a block: the conclusion
("every row inside 5%") survives; the number does not.

Happy path untouched by construction — verified in the code, not just in the
benchmark.

### 7. Suites — all green, all counts as claimed

| check | result |
|---|---|
| `cargo test -p qr-detect` | 30 + 24 + 28 ok |
| `--no-default-features` | 26 + 23 ok |
| `--no-default-features --features decode` | 26 + 24 ok |
| `--no-default-features --features micro` | 30 + 23 + 26 ok |
| `--all-features` | 30 + 24 + 28 ok |
| `cargo test -p qr-video` | 7 ok |
| `cargo test -p tests-integration` | 84 ok |
| `cargo clippy -p qr-detect --all-targets` (x3 configs) | clean |
| `cargo clippy --workspace --all-targets` | clean |
| `cargo fmt -p qr-detect -- --check` | clean |
| `mod binarize` still private; no public API change | confirmed |
| no `unsafe`, no dependency change | confirmed |

---

## MAJOR

**MAJOR-1 — the glare finder-clearance rule is wrong at d = 8, in the unsafe
direction.** (`handoff/REVIEW-REQUEST-lowlight.md` §5, "The rule for QR")

`d/2 + 1` gives 5 modules for an 8-module blob. Both his table and mine show the
first decode at offset **6**. The rule under-predicts the clearance a user needs,
and it is explicitly earmarked for user-facing guidance — telling a customer 5
modules is enough when 6 is not is the kind of number that comes back as a
support ticket.

Measured minimum clearance from the finder centre, diagonal, 8 px/module:

| blob d | `d/2 + 1` says | measured (v2, v5) |
|---:|---:|---:|
| 2 | 2 | 2 ✓ |
| 4 | 3 | 3 ✓ |
| 8 | **5** | **6** ✗ |

The prose gloss — "its saturated core must not touch the finder's 3x3 core" —
implies `d/2 + 1.5`, which fits d = 8 (5.5 -> 6) and d = 2, but over-predicts
d = 4. Neither closed form fits all three points.

**Fix:** either state the measured clearances as a small table rather than a
formula, or use `ceil(d/2 + 1.5)` and note it is conservative at d = 4. Do not
ship `d/2 + 1`. Nothing in `crates/` depends on this, so it does not block the
code — but it must not reach guidance as written.

---

## MINOR

1. **`REVIEW-REQUEST-lowlight.md` §6** — miss-path delta reported as +0.9%;
   measured +2.0 to +2.7% over two runs against both the README and the
   worktree baseline. Conclusion unchanged (inside 5%); correct the figure so
   the next step's baseline is honest.

2. **`binarize.rs:159`** (`NOISE_ROW_STRIDE` doc) — "The statistic is a *median*
   over hundreds of thousands of samples". It is a lower quartile. Stale from the
   first implementation.

3. **`binarize.rs:274`** (`denoise` doc) — "every noise-free fixture measured has
   a *median* of 0". Same stale word; the constant docs directly above it say
   quartile. Two contradictory descriptions of the same statistic in one file.

4. **`binarize.rs:602`** (`adaptive_at`) — comment still reads "callers guarantee
   it via the `window_div >= 8` / min-side rules". The sibling comment in
   `adaptive` was correctly updated to `>= 3`; this one was missed. Left as-is it
   states a precondition the code no longer has.

5. **`binarize.rs:276-278`** (`denoise` doc) — "The one-pixel border keeps its
   original value" is only true of the left and right **columns**. Rows 0 and
   h-1 come from `tmp` and are horizontally smoothed (1D), because the vertical
   pass starts from `out = tmp.clone()`. Harmless in behaviour, wrong in the
   doc, and the asymmetry is not obvious from reading the loops.

6. **`binarize.rs:502`** (`fits_u32`) — the bound hardcodes `255 * 25` where the
   `25` is `MARGIN_DEN`. The doc argues correctness *from* `MARGIN_DEN`, and the
   `debug_assert!(margin <= MARGIN_DEN)` is in a different function. If anyone
   ever changes `MARGIN_DEN`, the overflow proof breaks silently in release.
   Use `MARGIN_DEN as usize` in the expression.

7. **`binarize.rs:292, 301`** (`denoise`) — allocates two full frames
   (`luma.to_vec()` then `tmp.clone()`). The second is avoidable; only the four
   borders of `out` need seeding from `tmp`. Retry path only, so this is
   1.8 MB of transient allocation on a miss, not a hot-path cost. Note it, do
   not necessarily fix it.

**Observation, not a finding** — Lever 3's argument (`can_be_dark` doc, ~line
428) is sound for the case it pins but is broader than what it measures. A frame
in the band `0.88 <= lo/hi < 0.96` skips a retry whose margin *would* have run
the adaptive pass. Bob argues such a frame is globally flat so Otsu is the better
answer, and `a_thin_contrast_symbol_on_a_flat_frame_goes_through_otsu` pins that
— but only at `vignette = 0.0`. The uncovered sub-case is a narrow-range frame
that is *not* uniformly illuminated (say a 10% gradient over a 2% print). It is
argued away rather than measured. I am not asking for a fix: at 2% contrast the
symbol is far past the `delta 15` row that his own envelope shows is dead at any
setting, so the band contains no recoverable symbols. He labelled it a judgement
and it is the right judgement. Recording it so the next person does not have to
rediscover the gap.

---

## Ruling on his open question 2 — `detect_luma_single_pass` / the qr-video ROI path

**Inheritance is acceptable. Document it; do not give the ROI path the ladder.**

I read `crates/qr-video/src/detector.rs` before ruling. Three facts decide it:

1. **No frame is lost.** `Detector::scan` falls back to `detect_full` — which is
   `detect_luma`, the whole new ladder — on the **same frame**, the moment an ROI
   scan comes back empty (detector.rs:227-234). A dim frame the ROI path cannot
   hold is not dropped; it is rescanned at the new envelope immediately.
2. **It self-corrects.** After `roi_miss_limit` (3) consecutive misses the ROI
   path is abandoned and every frame is a full-ladder scan until a hit reseeds
   the ROI. So sustained low light does not degrade to the old envelope at all —
   it degrades to full-ladder scanning within three frames, ~100 ms at 30 fps.
3. **Applying the ladder inside a crop would be actively wrong.**
   `RETRY_WINDOW_DIV` is a fraction of the *smaller side of the buffer it is
   given* — on a tight ROI crop that radius collapses toward the 4 px floor, and
   a finder core hollows out. That is the exact failure this crate already
   documents as the reason the wide window is primary. Worse, a crop under 24 px
   short-circuits to Otsu regardless. The crate's existing comment at
   `detect_in_roi` ("Single-pass deliberately") is right for a reason that
   predates this step and still holds.

The cost of inheritance is a latency transient, not a capability loss: during at
most three frames of a lighting change, each frame pays ROI (~324 µs) + full
(~688 µs) ~ 1.0 ms instead of 688 µs. That is inside any 30 fps budget and it is
self-limiting. Doing the opposite makes the worst case *worse* — ROI primary +
ROI retry + full-frame ladder.

**Concrete ask, and it is small:** a paragraph in `qr-video`'s `detect_in_roi`
doc recording that the ROI path deliberately runs the pre-12a envelope, that the
same-frame fallback and the 3-miss limit are what make that safe, and that the
retry's window radius is meaningless inside a crop. That belongs to whichever
step owns `qr-video` — Bob was scoped to `crates/qr-detect/` and correctly did
not touch it. **Log it to BUILD-LOG, do not reopen 12a for it.**

---

## Cleared

The three-lever change to `crates/qr-detect` — parameterised Bradley margin with
the retry rung at 96%, a quartile-gated symmetric 3x3 box denoise on that same
rung, and the `Pass` enum that names the ladder — is correct, measured, and
honestly reported. I reproduced the envelope independently (460 -> 1226 against
his 469 -> 1227), reproduced the glare geometry exactly, confirmed both
counterfactuals that justify the gate by breaking them on a copy of the tree,
confirmed the primary rung is byte-identical arithmetic, and confirmed `fits_u32`
and every pinned test are untouched and green across all eight suite
configurations plus clippy and fmt. The one regression is a coin-flip cell and
the mechanism bounds the class it belongs to. The one number that did not
reproduce is the glare clearance rule, and it lives in this handoff document
rather than in the code.

**Step 12a CLEAR.**
