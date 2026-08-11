# Review Feedback — Step 11a, detect performance

Date: 2026-08-11
Reviewer: Richard-OD
Ready for Builder: YES
**Verdict: Step 11a CLEAR**

BLOCKER: 0 — MAJOR: 0 — MINOR: 6

Scope reviewed: `git diff HEAD -- crates/qr-detect` (binarize.rs, finder.rs,
lib.rs; 510 insertions, 78 deletions). A sibling Builder's in-flight
`crates/qr-encode` work was ignored; it did not break any build or suite I ran.

---

## What I verified, and how

I did not take the bit-identical claim on trust. I built a differential harness
outside the repo (scratchpad only, nothing added to the tree): the pre-change
`binarize.rs` from `git show HEAD:` and the post-change file compiled side by
side as two modules, plus a textbook Bradley reference written from the
definition. Results, all seeds fixed, zero failures:

| check | coverage | failures |
|---|---|---:|
| `binarize()` old vs new, full `dark` buffer | 1 440 frames — 15 sizes (1×1, 5×3, 23×23, 23×40, 24×24, 25×24, 24×25, 26×31, 33×27, 40×24, 64×37, 65×64, 96×71, 128×96, 201×149) × 8 content models (uniform, noise, narrow-band, gradient+marks, bimodal, single speck, vignette, two-level at the 88/100 edge) × 6 reps × div 8 and 16 | **0** |
| `adaptive()` old vs new, fallbacks bypassed | 1 056 runs (same matrix, ≥24 px only) | **0** |
| `adaptive_at::<u32>` vs `::<u64>` | 2 192 runs, every radius 1..=(min−1)/2 on every size | **0** |
| `adaptive_at::<u64>` vs naive Bradley reference | 7 180 runs, w 5..40 × h ∈ {3,5,9,17,40} × **every** legal radius — this is the one that hits `w == 2*radius+1` exactly, where the interior/edge slices are at their tightest | **0** |
| `can_be_dark` soundness | 9 808 frames rejected by the guard; for each, old `adaptive` confirmed to mark nothing dark, at both radii | **0** |
| 88/100 boundary sweep | every (lo,hi) pair straddling `lo*100 >= hi*88` for hi 1..=255, on a 64×64 three-phase frame: guard sound, and `binarize()` output identical old vs new either side of the edge | **0** |
| `fits_u32` overflow proof | 12 frame shapes × 2 divisors, each `true` result re-checked in `u128` against all four products (`w*h*255`, `count*255*22`, `255*count*25`, `count*25`) | **0** |
| `prefix_sum` vs naive | lengths 0..80, incl. every remainder class of `PREFIX_BLOCK = 8` | **0** |
| finder run extraction, old per-pixel walk vs new `run_length` walk | 39 800 random rows (lengths 1..200, three densities) + explicit boundaries: run ending exactly at 32, row of exactly 32, 31, 33, 64, single-element runs, row shorter than a block | **0** |

Line-by-line the algebra also holds:

* **Bradley constant.** `100/88 → 25/22` is exact (100 = 4·25, 88 = 4·22), the
  comparison stays strict `<`, so no rounding or truncation moves. There is no
  division anywhere in the new path — the old integral version had none either —
  so there is no division-order hazard to get wrong.
* **Window geometry.** Interior `x ∈ [radius, w−radius)` uses
  `prefix[x+radius+1] − prefix[x−radius]`, count `rows*(2r+1)`. Left edge
  `x = i < radius` uses `prefix[i+radius+1] − 0`, count `rows*(i+r+1)`. Right
  edge `x = w−radius+i` uses `total − prefix[w−2r+i]`, count `rows*(2r−i)`.
  All three match the old clamped `x0/x1/y0/y1` count exactly, including at
  `x = w−1` (count `r+1` columns). `rows = y1−y0+1` is unchanged verbatim.
* **`can_be_dark`.** Dark requires `value*count*100 < sum*88`; `sum <= count*max`
  and `value >= min` give `value*count*100 >= min*count*100 >= max*count*88 >= sum*88`
  whenever `min*100 >= max*88`. The guard's `>=` is the exact negation of the
  test's `<` — no `+1` bias, no off-by-one. The boundary sweep above walks both
  sides of the edge for every (min,max) pair that can straddle it.
* **All-dark arm deletion.** The proof in the comment is right: for the frame's
  brightest pixel `m`, `m*count*100 < sum*88 <= count*m*88 <= count*m*100` is
  false for `m*count > 0`, and for `m = 0` reduces to `0 < 0`. The brightest
  pixel is never dark, so `dark_count == total` was unreachable.
* **`radius_invariant`.** Set only on the two frame-only conditions (size floor,
  `can_be_dark`). Never set by `adaptive()`, and never set on the
  "adaptive came back empty" Otsu — `otsu(..., false)` on that arm, correctly.
  A vignette has wide dynamic range by construction, so it can never trip the
  guard; confirmed empirically, not just by argument —
  `the_retry_window_holds_a_vignette_the_primary_loses`,
  `adaptive_survives_a_vignette_to_the_documented_depth`,
  `a_deep_vignette_is_recovered_by_the_retry_pass`,
  `a_shallow_vignette_is_handled_by_the_primary_pass`,
  `a_degraded_micro_sample_does_not_suppress_the_retry` (+ the `detect_all`
  variant) all pass, and the deep-vignette one can only pass *because* the retry
  still engages.
* **Retry skip is lossless.** `better_pass` and the `detect_all` ranking both
  return the primary on a tie, and a radius-invariant `Binary` is rebuilt
  bit-identically at the other radius, so the early return produces exactly what
  the comparison would have. The `None` arm is radius-independent by inspection
  (`scan` returns `None` only when `binarize` does, i.e. bad dimensions).
* **`run_length`.** Always returns ≥1 (`rest[0] == color` by construction), so
  the outer walk always advances. `chunks_exact` remainder and
  `into_remainder()` are used correctly on both iterators.
* **`fits_u32` thresholds.** `w*h <= u32::MAX/255` = 16 843 009 px and
  `count <= u32::MAX/(255*25)` = 673 720 px. 4K (3840×2160, r = 270,
  count = 292 681) routes to `u32`; 8K (7680×4320) routes to `u64` on the first
  condition; a large square (4104×4104, r = 513) routes to `u64` on the second.
  Bob's "up to 4K" is accurate.

---

## Suites, lint, format

| check | result |
|---|---|
| `cargo test -p qr-detect` | 25 + 24 + 28 pass |
| `cargo test -p qr-detect --no-default-features` | 21 + 23 pass |
| `... --no-default-features --features decode` | 21 + 24 pass |
| `... --no-default-features --features micro` | 25 + 23 + 26 pass |
| `cargo test -p qr-detect --all-features` | 25 + 24 + 28 pass |
| `cargo test -p qr-video` | 7 pass |
| `cargo test -p tests-integration` | 84 pass (11+5+11+3+12+15+3+9+15) |
| `cargo clippy -p qr-detect --all-targets` | clean |
| `cargo clippy -p qr-detect --no-default-features --all-targets` | clean |
| `cargo clippy -p qr-detect --all-features --all-targets` | clean |
| `cargo fmt -p qr-detect -- --check` | clean |
| `unsafe` in `crates/qr-detect/src` | none |
| dependency / Cargo.toml changes | none |

Every DoD line Bob claimed reproduces exactly.

---

## Reproduced numbers

Apple M4, `bench` profile, medians. "before" is a clean `git worktree` at HEAD
(no stash, working tree untouched), run back to back with "after" on an
otherwise idle machine.

`cargo bench -p benchmarks --bench detect`:

| bench | before (mine) | after (mine) | Bob's after | Δ |
|---|---:|---:|---:|---:|
| happy path: 1 v5 @8px/module | 2.09 ms | **705.62 µs** | 687 µs | −66 % |
| happy path, single pass | 2.09 ms | **706.12 µs** | 705 µs | −66 % |
| rotated 30° | 2.11 ms | **704.92 µs** | 705 µs | −67 % |
| miss: no symbol (primary + retry) | 4.00 ms | **1.15 ms** | 1.13 ms | −71 % |
| miss, single pass | 1.99 ms | **577.38 µs** | 566 µs | −71 % |
| miss: flat frame | 6.79 ms | **328.50 µs** | 318 µs | −95 % |
| miss: flat frame, single pass | 3.39 ms | **331.58 µs** | 319 µs | −90 % |
| detect_micro_luma: 1 M4 | 2.14 ms | **779.88 µs** | 761 µs | −64 % |
| detect_all_luma: 1 v5 + 1 M4 | 2.53 ms | **1.13 ms** | 1.12 ms | −55 % |

`cargo bench -p benchmarks --bench video`: steady-state ROI **324.92 µs**
(3.08 k/s), full-scan cadence **689.42 µs** (1.45 k/s) — both slightly better
than Bob's figures.

My "before" column lands on Bob's to within 0.5 % on seven of nine rows (flat
frame 6.79 vs his 6.53), so his baseline was honestly measured, not borrowed.
Both targets are met with room: flat frame 329 µs against 1 ms, happy path
706 µs against 2.13 ms.

---

## Must Fix

None.

---

## Should Fix (MINOR)

1. **MINOR — `binarize.rs:320-395`. `adaptive_at` gained an unguarded
   precondition.** The old clamped-per-pixel loop coped with any radius; the new
   slice arithmetic requires `w >= 2*radius + 1` and panics on a slice range
   otherwise. Demonstrated: on a 64×64 frame, `binarize(.., window_div = 1)` and
   `window_div = 2` panic where the old code returned `Some(866)` / `Some(858)`.
   Unreachable today — `mod binarize` is private, the only call sites pass
   `PRIMARY_WINDOW_DIV` / `RETRY_WINDOW_DIV`, and for any `window_div >= 3` with
   sides ≥ 24 the invariant holds — so this is a latent trap, not a live bug.
   Fix: promote the comment at `binarize.rs:230-232` into a
   `debug_assert!(w >= 2 * radius + 1)` at the top of `adaptive_at`, and state
   the bound on `binarize`'s `window_div` doc line. Two lines, buys the next
   person who changes a divisor a failing test instead of a panic in the field.

2. **MINOR — `binarize.rs:505-531`. `the_two_accumulator_widths_agree` tests
   equivalence, not headroom.** At 320×240 with radius ≤ 30 the products are
   four orders of magnitude below `u32::MAX`, so the test would pass even if
   `fits_u32`'s constants were wrong by a factor of 100. Nothing tests
   `fits_u32` itself. Fix: add three assertions —
   `assert!(fits_u32(3840, 2160, 270))`, `assert!(!fits_u32(7680, 4320, 540))`,
   and one on the second condition (`assert!(!fits_u32(4104, 4104, 513))`). No
   fixture needed; it is pure arithmetic. I verified all three by hand and in
   `u128`, but the check belongs in the suite, not in this review.

3. **MINOR — perf table is 2-3 % optimistic across the board.** Happy path
   reproduces at 705.62 µs, not 687 µs; flat frame 328.50, not 318; micro
   779.88, not 761. The tell is in Bob's own table: 687 µs for the happy path
   against 705 µs for happy-path-single-pass, which is the same work — 687 is a
   lucky sample, not a median of a settled run. Not a correctness issue and it
   changes no conclusion (both targets still met by 3× and 1.5×), but the
   numbers that go into `benchmarks/README.md` should be the reproducible ones.
   Recommend the table be restated at my figures, or re-measured.

4. **MINOR — the stage-attribution table (`REVIEW-REQUEST`, "Where the time
   went") is not reproducible.** The probe it came from was deleted, and three
   of its rows are explicitly arithmetic on end-to-end deltas. The end-to-end
   deltas it explains are sound, so I am not asking for the probe back — but the
   table should not be copied into `BUILD-LOG` as measurement. Label the
   inferred rows or drop them.

5. **MINOR — `can_be_dark` adds a full-frame min/max pass to every call**,
   including the textured frames it can never help. It is paid for many times
   over (textured single-pass miss went 1.99 ms → 577 µs), so this is an
   observation, not an objection: if anyone revisits, the min/max folds into the
   first `add_row` for free. Log it, do not act on it now.

6. **MINOR — `benchmarks/README.md` is stale.** Bob flagged this himself
   (open question 4) and correctly left `benchmarks/` alone per the brief. Its
   "a featureless frame is the detector's worst case" analysis is now false —
   the flat frame is the *fastest* case at 329 µs, 2× faster than the happy path.
   Needs an owner. See Escalate.

---

## Escalate to Architect

1. **Scope: the `finder.rs` change (item 10) is past the brief.** Bob declared
   it and offered to withdraw it. My finding on the code: it is
   semantics-preserving — I differentially tested the old per-pixel walk against
   the new blocked one on 39 800 random rows plus the block-boundary cases, zero
   divergence, and the `runs` buffer hoist is safe (`clear()` at the top of every
   row). My recommendation is to **keep it**: without it the happy path lands at
   ~1.14 ms instead of 706 µs, and the risk is now measured rather than argued.
   But it is scope, and scope is your call, not mine.

2. **Who refreshes `benchmarks/README.md`?** The detect and video sections
   describe behaviour that no longer exists, and correcting them means editing a
   directory the brief put out of bounds. Needs to be assigned as its own step
   before the stale analysis gets quoted somewhere.

---

## Cleared

Reviewed the full `crates/qr-detect` diff: the sliding-column-sum rewrite of
`binarize`, the `can_be_dark` short-circuit and `radius_invariant` retry skip,
the `Acc`/`fits_u32` accumulator split, the blocked `prefix_sum`, the interleaved
`histogram`, the `run_length` finder scan, and the pipeline changes in `lib.rs`.
The bit-identical claim holds under differential test against the deleted
integral-image implementation and against an independent Bradley reference; the
`can_be_dark` bound is exactly the negation of the dark test with no off-by-one;
the retry skip is lossless and does not touch the vignette rescue path;
`fits_u32` is sound with 4K on the narrow path and 8K on the wide. All eleven
DoD checks reproduce, and the two headline benchmarks reproduce within 3 % of
the claim. Six MINOR items, none blocking. **Step 11a CLEAR.**
