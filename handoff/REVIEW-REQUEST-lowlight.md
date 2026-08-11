# Review Request — Step 12a, low-light / poor-illumination robustness

Date: 2026-08-11
Builder: Bob-LL
Scope: `crates/qr-detect/` only (binarize.rs, lib.rs). No other crate touched.
**Ready for Review: YES**

Includes Arch's mid-task scope addendum: specular glare / reflections.

---

## 1. The degradation model

Built as a standalone harness outside the repo (scratchpad only — nothing added
to the tree, no new dependency, no fixture files). It path-depends on the
workspace crates and drives the real public API, so every "decode" below is
`detect_luma` / `detect_micro_luma` returning the exact payload the symbol was
encoded from — not a detection count, not a module-error proxy.

Frames are 1280x720 luma. A real encoded symbol is rendered through
`qr-encode` + `qr-render` at a 4-module quiet zone and blitted at (60, 60) —
off-centre on purpose, so the radial illumination model actually falls across
the symbol rather than grazing it.

```
luma(x,y) = I(x,y) * (BG - delta * is_dark) + glare(x,y) + N(0, sigma)
```

| axis | values | stands for |
|---|---|---|
| `BG` | 220 | paper white under indoor light |
| `delta` | 60, 40, 25, 15 | print contrast, in luma below the paper. 60 is a decent print, 15 is a faded thermal receipt |
| `sigma` | 0, 4, 8, 12, 16 | zero-mean Gaussian sensor noise, approximating rising ISO |
| `I(x,y)` | flat / linear / radial | `1`; a diagonal ramp to 0.4 across the frame; a vignette `1 - 0.75 r^2` from the frame centre, which puts the symbol corner at 0.41 |
| glare | 4 blob sizes x 3 placements | additive `255 * exp(-2 (r/R)^2)`, saturating at the core |
| symbol | QR v2, v5; Micro M2, M4 | |
| scale | 4, 6, 8 px/module | |

Contrast and illumination are varied **independently** and multiplicatively,
which is the axis `vignetted_module_errors` already established in this file and
the one that matters: a `delta` of 25 under a 0.41 vignette is 10 luma of
signal, and no fixture that paints luma 0 on luma 255 exercises it.

Everything is deterministic: a xorshift PRNG seeded per (cell, repetition), 3
seeds per cell, Box-Muller for the normal. No clock, no image files, no network.

Grid sizes: **2160** decode attempts on the no-glare grid (4 symbols x 3 scales x
3 illumination x 4 contrast x 5 noise x 3 seeds), **1152** on the glare grid,
plus a finder-proximity sweep and a (window radius x margin) sweep of 2 x 7
settings over the whole grid measured at module-error level.

---

## 2. Baseline envelope — decode success, BEFORE

Each cell is successes out of 3 seeds. Column groups are `delta` 60 / 40 / 25 /
15; within each group, `sigma` 0 4 8 12 16.

```
v2   4px flat    | 3 3 3 3 0 | 3 0 0 0 0 | 3 0 0 0 0 | 3 0 0 0 0 |
v2   4px linear  | 3 3 3 3 0 | 3 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
v2   4px radial  | 3 3 1 0 0 | 3 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
v2   6px flat    | 3 3 3 0 0 | 3 0 0 0 0 | 3 0 0 0 0 | 3 0 0 0 0 |
v2   6px linear  | 3 3 3 0 0 | 3 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
v2   6px radial  | 3 2 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
v2   8px flat    | 3 3 0 0 0 | 0 0 0 0 0 | 3 0 0 0 0 | 3 0 0 0 0 |
v2   8px linear  | 3 3 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
v2   8px radial  | 3 1 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
v5   4px flat    | 3 3 3 2 0 | 3 0 0 0 0 | 3 0 0 0 0 | 3 0 0 0 0 |
v5   4px linear  | 3 3 2 1 0 | 3 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
v5   4px radial  | 3 3 0 0 0 | 3 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
v5   6px flat    | 3 3 3 0 0 | 0 0 0 0 0 | 3 0 0 0 0 | 3 0 0 0 0 |
v5   6px linear  | 3 3 1 0 0 | 0 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
v5   6px radial  | 3 3 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
v5   8px flat    | 3 3 0 0 0 | 0 0 0 0 0 | 3 0 0 0 0 | 3 0 0 0 0 |
v5   8px linear  | 3 3 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
v5   8px radial  | 3 2 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
M2   4px flat    | 3 3 3 3 0 | 3 3 0 0 0 | 3 0 0 0 0 | 3 0 0 0 0 |
M2   4px linear  | 3 3 3 3 0 | 3 3 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
M2   4px radial  | 3 3 0 0 0 | 3 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
M2   6px flat    | 3 3 3 3 0 | 3 2 0 0 0 | 3 0 0 0 0 | 3 0 0 0 0 |
M2   6px linear  | 3 3 3 1 0 | 3 2 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
M2   6px radial  | 3 3 0 0 0 | 3 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
M2   8px flat    | 3 3 3 1 0 | 3 2 0 0 0 | 3 0 0 0 0 | 3 0 0 0 0 |
M2   8px linear  | 3 3 3 2 0 | 3 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
M2   8px radial  | 3 3 0 0 0 | 3 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
M4   4px flat    | 3 3 3 3 1 | 3 3 0 0 0 | 3 0 0 0 0 | 3 0 0 0 0 |
M4   4px linear  | 3 3 3 3 1 | 3 3 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
M4   4px radial  | 3 3 0 0 0 | 3 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
M4   6px flat    | 3 3 2 2 0 | 3 2 0 0 0 | 3 0 0 0 0 | 3 0 0 0 0 |
M4   6px linear  | 3 3 2 1 0 | 3 1 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
M4   6px radial  | 3 2 0 0 0 | 3 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
M4   8px flat    | 3 3 3 0 0 | 3 1 0 0 0 | 3 0 0 0 0 | 3 0 0 0 0 |
M4   8px linear  | 3 3 3 0 0 | 3 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
M4   8px radial  | 3 3 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 | 0 0 0 0 0 |
```

**469 of 2160 (21.7%).**

Two things in that table should stop a reader, and both turned out to be the
mechanism rather than noise.

* It is **not monotonic in contrast.** `v2 8px flat` decodes at `delta` 25 and
  15 but fails at 40. Lower contrast working where higher contrast fails is not
  a gradual envelope running out; it is two different code paths.
* The whole `delta <= 40` region collapses the moment illumination stops being
  flat, even at `sigma = 0` — a noiseless frame with a thin print under a gentle
  ramp.

---

## 3. Mechanism — instrumented, not guessed

The harness re-implements the Bradley pass so it can score **module-centre
binarization errors** for any (window divisor, margin) pair, next to whether the
real pipeline decoded. That separates "the binarization lost the symbol" from
"the binarization was fine and something downstream lost it".

Extract, `v5 @ 8px`, errors out of 1369 module centres (668 = every dark module
read light, i.e. a blank):

| illum | delta | sigma | decoded | err @88 | err @94 | err @97 | err @100 |
|---|---:|---:|:--:|---:|---:|---:|---:|
| flat | 60 | 0 | Y | 0 | 0 | 0 | 0 |
| flat | 60 | 8 | . | 26 | 0 | 0 | 5 |
| flat | 40 | 0 | **.** | **227** | **0** | **0** | 0 |
| flat | 25 | 0 | Y | 668 | 0 | 0 | 0 |
| flat | 15 | 0 | Y | 668 | 668 | 0 | 0 |
| radial | 40 | 0 | . | 353 | 0 | 0 | 0 |
| radial | 25 | 0 | . | 668 | 182 | 0 | 0 |
| radial | 15 | 0 | . | 668 | 668 | 155 | 0 |

### 3a. The dominant mechanism: the margin is *relative*, and it has a dead band

`value * count * 100 < sum * 88` asks a pixel to be 12% below its **local mean**.
In a data region about half dark, the local mean is itself `B - d/2`, so a dark
module at `B - d` clears the test only when `d > 0.47 B`. Against a 220
background that is `d > 47` — so an ordinary worn or dim print at `d = 40`
does not produce a **degraded** binarization. It produces a **blank one**, and
there is nothing downstream for error correction to work on. Confirmed above:
`err @88 = 227` where `err @94 = 0`.

That also explains the non-monotonicity, which is the load-bearing detail:

* `d = 25` (11% of the background) fails `can_be_dark` outright, so the adaptive
  pass is skipped and the **Otsu fallback** runs. On a flat frame a global
  threshold is exactly right, so it decodes — by accident of the short-circuit.
* `d = 40` (18%) passes `can_be_dark`, runs the adaptive pass, marks *some*
  stray pixels dark — enough that the "no dark pixel at all" degenerate guard
  does not fire — and the useless binarization is kept. It fails.

So the band between roughly 12% and 21% of relative contrast is a hole that the
existing fallback structure steps over. Adding an illumination gradient closes
the accidental Otsu escape as well, which is why the `delta <= 40` region dies
completely on the linear and radial rows.

### 3b. The second mechanism: noise lands entirely on one side of the comparison

The window mean is taken over thousands of pixels, so it is already noise-free.
The pixel's own noise is therefore not attenuated at all by the comparison — the
whole of it moves one side. Measured at the retry settings, mean module errors
over the grid:

| delta | sigma | no filter | median-3 | 2x2 box | **3x3 box** |
|---:|---:|---:|---:|---:|---:|
| 60 | 8 | 4.3 | 0.6 | 0.1 | **0.0** |
| 60 | 12 | 20.5 | 4.9 | 0.9 | **0.0** |
| 60 | 16 | 45.1 | 14.2 | 4.2 | **0.3** |
| 40 | 8 | 23.2 | 5.6 | 0.8 | **0.2** |
| 40 | 12 | 59.8 | 21.6 | 7.5 | **1.4** |
| 25 | 8 | 83.7 | 47.6 | 28.1 | **12.4** |

Cells binarizing perfectly at `delta 60, sigma 12`: 0 of 36 unfiltered, **35 of
36** with the symmetric 3x3 box. The symmetric box beats both of the kernels the
brief suggested at every scale down to 4 px/module — a 2x2 box shifts module
centres by half a pixel, and a horizontal median-of-3 only attacks one axis.

### 3c. Glare — two mechanisms, as Arch predicted, and only one is ours

Split by whether the blob has driven a module's light *and* dark levels to the
top of the range (`core`, physically unrecoverable) or not (`out`, threshold
behaviour). `v5 @ 8px`, out of 1369:

| placement | blob (modules) | decoded | core | err @88 total / **outside** | err @96 total / **outside** |
|---|---:|:--:|---:|---:|---:|
| interior | 2 | Y | 1 | 1 / **1** | 1 / **1** |
| interior | 8 | Y | 21 | 32 / **21** | 22 / **11** |
| interior | 15 | . | 89 | 88 / **50** | 62 / **24** |
| finder | 2 | **.** | 1 | 5 / **4** | 1 / **0** |
| finder | 8 | . | 21 | 30 / **21** | 27 / **18** |
| outside | 15 | Y | 32 | 38 / **25** | 31 / **18** |

1. **Saturated core — physics.** The envelope is a straight RS-budget question
   and the table in §5 shows where each version dies: v5 survives an 8-module
   blob on its interior and dies at 15; v2 dies at 8; M2/M4 die at 2-4. Nothing
   in a binarizer changes that.
2. **Near-blob threshold shift — ours, and the narrow margin already fixes most
   of it.** Non-core errors roughly halve at the narrow margin (21 -> 11,
   25 -> 18, 4 -> 0).
3. **Glare on a finder kills detection outright**, at every blob size, even the
   2-module one where the narrow margin drives non-core errors to **zero**. One
   saturated module inside the 3-module finder core breaks the `1:1:3:1:1` run
   and the vertical cross-check, and no thresholding rule puts it back.
   Quantified in §5.

### 3d. Arch's clamp candidate: measured, and it is a loss

Arch suggested clamping near-255 pixels out of the window mean. I measured the
**ceiling** on that idea — an *oracle* cap set to exactly the fixture's unlit
background level, which no real estimator could beat — over the whole glare
grid. Non-core module errors, `err @88` outside the core, without cap vs with:

| case | no cap | oracle cap |
|---|---:|---:|
| v2 interior, blob 15 | 53 | **68** |
| v5 interior, blob 15 | 50 | **74** |
| v5 outside, blob 15 | 25 | **36** |
| v2 outside, blob 15 | 28 | **36** |

It is **never better and often much worse**, at both margins. The reason is
straightforward once measured: capping the bright pixels lowers the local mean
across the whole window, which makes genuinely dark modules *harder* to classify,
and that costs more than the false-darks near the highlight it was meant to
prevent. Rejected on the measurement, not on the cost — the cost question never
had to be asked.

---

## 4. Levers chosen

### Lever 1 — the Bradley margin becomes a parameter, and the retry rung narrows it to 96%

A `(window divisor, margin)` sweep over the whole grid, scoring cells that
binarize perfectly and mean module error:

| div | margin | perfect (of 720) | <=2% err | mean err |
|---:|---:|---:|---:|---:|
| 8 | 88 | 100 | 142 | 164.9 |
| 8 | 94 | 178 | 251 | 92.5 |
| 8 | **96** | **196** | 265 | 68.5 |
| 8 | 97 | 193 | 266 | 63.0 |
| 8 | 100 | 153 | 197 | 71.9 |
| 16 | 88 | 58 | 92 | 181.9 |
| 16 | **96** | **189** | 263 | 73.5 |
| 16 | 97 | 211 | 286 | 63.2 |

Two findings decide the shape:

* **96-97 is a plateau**, and it falls away on both sides. 92 is still losing
  thin-contrast symbols; 100 (no margin at all) has nothing left to reject noise
  with and drops back below both.
* **88 and 96 do not order.** Of the 108 cells at full contrast with
  `sigma >= 8`, the *wide* margin beats the narrow one on **57**. A wide margin
  is noise protection; a narrow margin is contrast sensitivity. This is exactly
  the shape of the window-radius conflict this crate already documents, and it
  gets the same resolution: **layer it, do not replace it.** The primary margin
  is untouched at 88.

96 rather than 97 is deliberate and buys something concrete: `96/100` reduces to
`24/25`, the same denominator `88/100 -> 22/25` already uses. So `fits_u32`'s
overflow bound, the accumulator-width routing, and the pinned
`accumulator_routing_boundaries` test are all **unchanged arithmetic**. The
plateau makes that free.

### Lever 2 — a gated 3x3 box smoothing, on the retry rung only

Chosen on §3b. Two things about the gate are load-bearing:

* **It is necessary, not a nicety.** Smoothing unconditionally on the retry rung
  **breaks `a_deep_vignette_is_recovered_by_the_retry_pass`** — that fixture
  renders at 3 px/module, where a 3x3 box spans a whole module. I hit that
  failure and it is what forced the gate. (Reported as a positive: the pinned
  test caught exactly the case it exists for.)
* **The statistic is a lower quartile, not a median.** The gate reads
  `|2 p[x] - p[x-1] - p[x+1]|` over every 16th row. The second difference
  annihilates anything locally linear, so illumination gradients and vignettes
  contribute nothing — this measures *noise*, not image energy. A **quantile**
  is what separates noise from module edges, since an edge is a large
  contribution from a minority of pixels. I started at the median and it is not
  safe: at `s` px/module the fraction of pixels interior to a run is at worst
  `(s-2)/s`, one third at 3 px/module, so a median needs more quiet pixels than
  a dense 3 px frame has. The lower quartile needs a quarter and clears it.

  Measured lower quartiles: **0** on every noise-free frame at every pitch from
  3 px up, including the strict alternating checkerboard; 4 at `sigma` 4, 8 at
  `sigma` 8, 15 at `sigma` 16. The trigger sits at 3, in the gap.

### Lever 3 — one deliberate gap, documented and pinned

The retry now changes the margin, and `can_be_dark`'s short-circuit depends on
the margin — so `is_radius_invariant` skipping the retry is no longer a proof on
that arm. It is still the right call, and the bound itself is the argument: a
frame that fails `can_be_dark` at 88% has under 12% of *global* dynamic range,
so illumination varies by at most 12% end to end and nothing reflects under 88%
of anything else. That is a globally flat scene, and on a globally flat scene the
Otsu fallback it already returns is not an approximation of the adaptive answer —
it is the better one. Pinned by
`a_thin_contrast_symbol_on_a_flat_frame_goes_through_otsu`, which asserts the
fallback resolves every module centre rather than merely that it runs.

### What I did not do

* **No third pass.** The brief offered one, and I did not need it: stacking all
  three settings onto the existing retry rung buys the envelope below while a
  miss still costs exactly two passes. A third pass would have put the miss-path
  benchmark ~50% over its 5% budget for a gain the second rung already captures.
* **No highlight clamp** — §3d.
* **No change to the primary rung.** Every happy-path frame walks byte-for-byte
  the same code it did before.

---

## 5. Envelope AFTER

### No-glare grid

```
v2   4px flat    | 3 3 3 3 0 | 3 3 3 3 3 | 3 3 2 0 0 | 3 0 0 0 0 |
v2   4px linear  | 3 3 3 3 0 | 3 3 3 3 3 | 3 3 3 1 0 | 0 0 0 0 0 |
v2   4px radial  | 3 3 1 1 0 | 3 3 3 3 1 | 3 1 0 0 0 | 0 0 0 0 0 |
v2   6px flat    | 3 3 3 3 3 | 3 3 3 3 3 | 3 3 3 1 0 | 3 0 0 0 0 |
v2   6px linear  | 3 3 3 3 3 | 3 3 3 3 3 | 3 3 3 0 0 | 0 0 0 0 0 |
v2   6px radial  | 3 3 3 3 3 | 3 3 3 3 1 | 3 3 0 0 0 | 0 0 0 0 0 |
v2   8px flat    | 3 3 3 3 3 | 3 3 3 3 3 | 3 3 3 0 0 | 3 0 0 0 0 |
v2   8px linear  | 3 3 3 3 3 | 3 3 3 3 3 | 3 3 0 0 0 | 0 0 0 0 0 |
v2   8px radial  | 3 3 3 3 3 | 3 3 3 3 0 | 3 3 0 0 0 | 0 0 0 0 0 |
v5   4px flat    | 3 3 3 2 0 | 3 3 3 3 3 | 3 3 3 0 0 | 3 0 0 0 0 |
v5   4px linear  | 3 3 2 1 1 | 3 3 3 3 3 | 3 3 2 0 0 | 0 0 0 0 0 |
v5   4px radial  | 3 3 2 1 1 | 3 3 3 2 0 | 3 1 0 0 0 | 0 0 0 0 0 |
v5   6px flat    | 3 3 3 3 3 | 3 3 3 3 3 | 3 3 3 0 0 | 3 0 0 0 0 |
v5   6px linear  | 3 3 3 3 3 | 3 3 3 3 3 | 3 3 3 0 0 | 0 0 0 0 0 |
v5   6px radial  | 3 3 3 3 3 | 3 3 3 3 0 | 3 3 0 0 0 | 0 0 0 0 0 |
v5   8px flat    | 3 3 3 3 3 | 3 3 3 3 3 | 3 3 3 0 0 | 3 0 0 0 0 |
v5   8px linear  | 3 3 3 3 3 | 3 3 3 3 3 | 3 3 3 0 0 | 0 0 0 0 0 |
v5   8px radial  | 3 3 3 3 3 | 3 3 3 3 0 | 3 3 0 0 0 | 0 0 0 0 0 |
M2   4px flat    | 3 3 3 3 0 | 3 3 0 0 0 | 3 2 1 0 0 | 3 0 0 0 0 |
M2   4px linear  | 3 3 3 2 0 | 3 3 1 0 0 | 3 2 0 0 0 | 3 0 0 0 0 |
M2   4px radial  | 3 3 0 0 0 | 3 1 0 0 0 | 3 1 0 0 0 | 3 0 0 0 0 |
M2   6px flat    | 3 3 3 3 1 | 3 3 3 2 3 | 3 3 3 2 0 | 3 0 0 0 0 |
M2   6px linear  | 3 3 3 1 1 | 3 2 2 2 2 | 3 3 3 2 0 | 3 0 0 0 0 |
M2   6px radial  | 3 3 1 1 1 | 3 3 2 1 0 | 3 2 0 0 0 | 0 0 0 0 0 |
M2   8px flat    | 3 3 3 3 3 | 3 3 3 3 3 | 3 3 3 2 0 | 3 0 0 0 0 |
M2   8px linear  | 3 3 3 3 3 | 3 3 3 3 3 | 3 3 3 2 0 | 0 0 0 0 0 |
M2   8px radial  | 3 3 3 3 3 | 3 3 3 2 2 | 3 3 0 0 0 | 0 0 0 0 0 |
M4   4px flat    | 3 3 3 3 1 | 3 3 1 0 0 | 3 2 1 0 0 | 3 0 0 0 0 |
M4   4px linear  | 3 3 3 3 1 | 3 3 1 0 0 | 3 1 0 0 0 | 3 0 0 0 0 |
M4   4px radial  | 3 3 0 0 0 | 3 3 0 0 0 | 3 1 0 0 0 | 3 0 0 0 0 |
M4   6px flat    | 3 3 2 2 2 | 3 3 2 3 3 | 3 3 3 1 0 | 3 0 0 0 0 |
M4   6px linear  | 3 3 2 2 1 | 3 3 2 3 1 | 3 3 2 0 0 | 0 0 0 0 0 |
M4   6px radial  | 3 3 2 1 1 | 3 3 2 2 0 | 3 3 0 0 0 | 0 0 0 0 0 |
M4   8px flat    | 3 3 3 3 3 | 3 3 3 3 3 | 3 3 3 1 0 | 3 0 0 0 0 |
M4   8px linear  | 3 3 3 3 3 | 3 3 3 3 3 | 3 3 3 1 0 | 0 0 0 0 0 |
M4   8px radial  | 3 3 3 3 2 | 3 3 3 3 1 | 3 3 0 0 0 | 0 0 0 0 0 |
```

**469 -> 1227 of 2160 (21.7% -> 56.8%).** 218 cells go from total failure to
total success. 291 cells improve, **1 regresses**.

Where it came from, out of 108 per cell:

| | sigma 0 | sigma 4 | sigma 8 | sigma 12 | sigma 16 |
|---|---|---|---|---|---|
| **delta 60** | 108 -> 108 | 103 -> 108 | 56 -> **93** | 31 -> **86** | 2 -> **64** |
| **delta 40** | 75 -> **108** | 22 -> **105** | 0 -> **88** | 0 -> **83** | 0 -> **62** |
| **delta 25** | 36 -> **108** | 0 -> **94** | 0 -> **56** | 0 -> 13 | 0 -> 0 |
| **delta 15** | 36 -> 51 | 0 -> 0 | 0 -> 0 | 0 -> 0 | 0 -> 0 |

| illumination | | | symbol | |
|---|---|---|---|---|
| flat | 231 -> 468 / 720 | | v2 | 103 -> 327 / 540 |
| linear | 149 -> 424 / 720 | | v5 | 92 -> 324 / 540 |
| radial | 89 -> 335 / 720 | | M2 | 142 -> 289 / 540 |
| | | | M4 | 132 -> 287 / 540 |

The `delta 15` row is unmoved and I am not claiming it: 15 luma on a 220
background is under 7% contrast, and under a vignette that is 6 luma of signal
against the quantiser. It is past what any threshold recovers.

**The one regression:** `M2 4px linear, delta 60, sigma 12` goes 3/3 -> 2/3. One
seed of one cell, in the region where the wide margin genuinely wins (§4) and
where the primary pass was already carrying it. Reported rather than tuned away.

### Glare grid

**344 -> 469 of 1152. 46 cells improve, zero regress.**

| placement | blob 2 | blob 4 | blob 8 | blob 15 |
|---|---|---|---|---|
| **interior** | 55 -> 71 | 37 -> 58 | 23 -> 25 | 0 -> 12 |
| **finder** | 0 -> 0 | 0 -> 0 | 0 -> 0 | 0 -> 0 |
| **outside** | 80 -> 95 | 81 -> 95 | 52 -> 71 | 16 -> 42 |

(of 96 each; blob diameter in modules)

Where each version dies to an interior blob, at 8 px/module — the RS-budget
question Arch asked for:

| symbol | modules | survives blob | dies at | saturated core at death |
|---|---:|---:|---:|---:|
| v5-M | 1369 | 8 | 15 | 89 modules |
| v2-M | 625 | 4 | 8 | 21 modules |
| M4-M | 289 | 2 | 4 | 5 modules |
| M2-L | 169 | — | 2 | 1 module |

**A blob on a finder is a total loss at every size, before and after.** Even the
2-module blob, where the narrow margin drives non-core errors to zero, loses the
symbol on one saturated module inside the finder core. How close a blob can sit,
walked diagonally out from the finder centre (`Y` = decoded, 8 px/module):

```
sym  blob | offset 0  1  2  3  4  5  6  7  8 10 12   (modules)
v2      2 |        .  .  Y  Y  Y  Y  Y  Y  Y  Y  Y
v2      4 |        .  .  .  Y  Y  Y  Y  Y  Y  Y  Y
v2      8 |        .  .  .  .  .  .  Y  Y  Y  Y  .
v5      2 |        .  .  Y  Y  Y  Y  Y  Y  Y  Y  Y
v5      4 |        .  .  .  Y  Y  Y  Y  Y  Y  Y  Y
v5      8 |        .  .  .  .  .  .  Y  Y  Y  Y  Y
M4      2 |        .  .  Y  .  .  .  Y  Y  Y  Y  Y
M4      4 |        .  .  .  .  .  .  .  Y  Y  Y  Y
M4      8 |        .  .  .  .  .  .  .  .  .  .  .
M2      4 |        .  .  .  .  .  .  .  .  .  .  .
```

**The rule for QR: a blob of diameter `d` modules must sit at least `d/2 + 2`
modules from a finder centre** — i.e. its saturated core must not touch the
finder's 3x3 core. Micro is materially worse: a single finder has no redundancy
and the blob lands in the data area at the same time, so M2/M4 lose everything
from a 4-module blob at any offset tested. This is a geometry limit, essentially
unchanged by the rework (the after table differs from the before by two cells on
micro), and it belongs in user-facing guidance, not in the binarizer.

---

## 6. Benchmarks

Apple M4, `bench` profile, medians. "before" is a clean `git worktree` at HEAD,
run in the same session on the same quiesced machine, because the README
baseline predates a sibling's commits. Every row re-run twice for stability.

| bench | README | before (worktree) | after | Δ vs before |
|---|---:|---:|---:|---:|
| happy path: 1 v5 @8px | 687.17 µs | 687.50 µs¹ | **685.12 µs** | −0.3% |
| happy path, single pass | 688.29 µs | 687.50 µs | **686.50 µs** | −0.1% |
| rotated 30° | 688.04 µs | 685.79 µs | **687.38 µs** | +0.2% |
| **miss: no symbol (primary + retry)** | 1.12 ms | 1.12 ms | **1.13 ms** | **+0.9%** |
| miss, single pass | 563.79 µs | 561.79 µs | **563.17 µs** | +0.2% |
| miss: flat frame | 320.38 µs | 315.88 µs | **312.12 µs** | −1.2% |
| miss: flat frame, single pass | 314.29 µs | 314.58 µs | **321.21 µs** | +2.1% |
| detect_micro_luma: 1 M4 | 760.46 µs | 758.17 µs | **757.29 µs** | −0.1% |
| detect_all_luma: 1 v5 + 1 M4 | 1.12 ms | 1.11 ms | **1.11 ms** | 0.0% |
| video: steady-state ROI | 324.25 µs | — | **324.08 µs** | −0.1% |
| video: full-scan cadence | 688.25 µs | — | **687.96 µs** | 0.0% |

¹ The worktree's first happy-path sample read 708.67 µs against 687.50 µs for
the identical work on the very next row; I took the settled figure rather than
the lucky/unlucky one, which is the correction Richard asked for last round.

**Every row inside 5%.** The happy path is untouched by construction — it never
reaches the retry rung. The miss path pays only the noise estimate, which reads
one row in sixteen.

### One benchmarking finding worth keeping

The first version of the noise estimate read a **median from a 256-bin
histogram** and cost the miss path **110 µs — a tenth of it — for a statistic
that touches 57 000 pixels.** That is the exact pathology this file already
documents at `HISTOGRAM_LANES`: on a smooth frame nearly every sample lands in
the same bin and the loop serialises on store-to-load forwarding. Rewriting it
as "count the samples below the threshold and compare against `n/4`" answers the
same question with one accumulator and no memory-dependent indexing, and the
same benchmark moves by 2%. The doc comment on `noise_reaches` records this so
nobody reintroduces the histogram to "get the actual number out".

---

## 7. Files changed

### `crates/qr-detect/src/binarize.rs` (+606 / −60)

| lines | change |
|---|---|
| 1-28 | Module doc: the two comparison parameters, and smoothing as a third, gated degradation |
| 54-75 | `is_radius_invariant` doc: the margin arm is now a judgement, with the pointer to the argument |
| 116-155 | `MARGIN_DEN` (25), `PRIMARY_MARGIN` (22 = 88%), `NARROW_MARGIN` (24 = 96%), each carrying the measurement that set it |
| 157-191 | `NOISE_ROW_STRIDE`, `NOISE_QUANTILE`, `DENOISE_TRIGGER` |
| 193-247 | `noise_reaches` — the gate statistic, plus why it is a quantile and why it is a comparison rather than a histogram |
| 249-310 | `denoise` — separable symmetric 3x3 box, 1-px border preserved |
| 312-322 | `binarize` — unchanged 4-arg signature, now delegating at `PRIMARY_MARGIN` |
| 324-340 | `binarize_retry` — the one place all three retry settings are stated |
| 342-400 | `binarize_with` — the margin parameter, with `debug_assert`s for both preconditions (this also closes Richard's MINOR #1 from Step 11a: `window_div >= 3` is now asserted, not just commented) |
| 410-443 | `can_be_dark(luma, margin)` — parameterised, with the globally-flat-scene argument for the retry skip |
| 465-503 | `fits_u32` doc: why it is margin-independent. **Code unchanged**, so the routing and its pinned test are untouched arithmetic |
| 505-520, 600-620 | `adaptive` / `adaptive_at` take the margin; `t_num` from the parameter, `t_den` still `MARGIN_DEN` |
| 806-816 | `the_two_accumulator_widths_agree` now sweeps both margins (mechanical arity update, strictly widened) |
| 1046-1124 | Test helpers: `noisy`, `texture_module`, `textured_frame` (the degradation model in miniature), `texture_errors` |
| 1125-1166 | **New** `the_noise_estimate_separates_sensor_noise_from_texture` |
| 1167-1201 | **New** `smoothing_recovers_module_centres_that_noise_flips` |
| 1202-1250 | **New** `the_narrow_margin_sees_a_print_the_primary_margin_cannot` |
| 1251-1275 | **New** `a_thin_contrast_symbol_on_a_flat_frame_goes_through_otsu` |

### `crates/qr-detect/src/lib.rs` (+64 / −40)

| lines | change |
|---|---|
| 14-42 | Crate doc: the retry rung's three settings and why a miss still costs two passes |
| 72-86 | The retry-skip section gains the margin caveat |
| 229-248 | **New** `enum Pass { Primary, Retry }` and `binarize_for` — the ladder as one named decision instead of a loose `window_div` threaded through four signatures |
| 250 | `type Pass<T>` renamed `Outcome<T>` (it is a pass's *result*; the name was needed for the enum) |
| 265, 281, 406, 447 | `detect_qr_at` / `scan` / `detect_micro_at` / `detect_all_at` take a `Pass` |

**No `unsafe`. No dependency or `Cargo.toml` change. No public API change** —
everything added is inside the private `binarize` module.

---

## 8. DoD

| check | result |
|---|---|
| `cargo test -p qr-detect` | 30 + 24 + 28 pass |
| `... --no-default-features` | 26 + 23 pass |
| `... --no-default-features --features decode` | 26 + 24 pass |
| `... --no-default-features --features micro` | 30 + 23 + 26 pass |
| `... --all-features` | 30 + 24 + 28 pass |
| `cargo test -p qr-video` | 7 pass |
| `cargo test -p tests-integration` | 84 pass |
| `cargo clippy -p qr-detect --all-targets` | clean |
| `... --no-default-features --all-targets` | clean |
| `... --all-features --all-targets` | clean |
| `cargo fmt -p qr-detect -- --check` | clean |
| `cargo bench -p benchmarks --bench detect` | every row within 5% (§6) |
| pinned tests modified | **none** |

Every pinned test named in the brief — hollowing, vignette envelope, worn print,
retry semantics, flat-frame short-circuit, the rotation and perspective suites —
is byte-for-byte unchanged and green. The only test text I touched is
`the_two_accumulator_widths_agree`, where adding a parameter to a private
function forced the call site; it now sweeps both margins, which strictly widens
what it pins.

---

## 9. Open questions for Richard / Arch

1. **`benchmarks/README.md` is now more stale, not less.** It still describes
   the retry as "a second full binarize-and-scan" differing only in radius. I
   left `benchmarks/` alone per the brief, and Richard already escalated its
   ownership after Step 11a. The retry-rung paragraph and the detect baseline
   both want refreshing in whichever step picks that up.

2. **`detect_luma_single_pass` gets none of this**, by design — it is the
   primary rung only, and `qr-video`'s ROI path uses it. So a tracked symbol in
   a dim or noisy scene is scanned at the *old* envelope between full-scan
   cadence frames. That may be exactly right (a tracked ROI is a crop, and the
   crate argues the retry is the wrong tool inside one), but the Project Owner's
   complaint was about live camera scanning, so it is worth a decision rather
   than an inheritance.

3. **Micro symbols are the weak half of the after table** (M2/M4 at 287-289 of
   540 against v2/v5 at 324-327) and they collapse fastest under glare. Single-
   finder anchoring plus a small RS budget is the cause; nothing in binarization
   moves it. Flagging in case a micro-specific step is wanted.

4. **The one regressed cell** (§5). I can suppress it by raising the retry
   margin toward 97, but that trades measurably worse behaviour elsewhere on the
   grid. Left as-is; say the word if you want it chased.

5. **The degradation harness lives in scratchpad, not the repo.** It is ~600
   lines and generates every fixture in-process from the workspace crates, so it
   would drop into `tests-integration` cleanly if the envelope is worth pinning
   as a suite rather than as a table in this document. Not done: the brief
   scoped me to `crates/qr-detect/`, and a 2160-frame grid is a minute of CI.
