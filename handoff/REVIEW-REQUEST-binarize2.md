# Review Request — KG-11, layered retry (round 3)

**Ready for Review: YES**
**Builder:** Bob-BIN
**Round:** 3. Round 1 (per-pixel two-scale binarization) was blocked and
abandoned. Round 2 (layered retry, candidate-count trigger) was blocked on
BLOCKER-2. This round changes the *trigger* only — the layered design, the
binarization revert and every round-2 test survive unchanged.

**Scope:** `crates/qr-detect/` plus two lines in `crates/qr-video/src/detector.rs`.
No new dependency, no `unsafe`.

---

## BLOCKER-2 — fixed, and Richard was right about the mechanism

The round-2 design rested on "where `/16` is needed, `/8` returns nothing rather
than something degraded". That is true on the QR path and false on the micro
path, for exactly the reason he gives: QR needs three finders in a valid
arrangement and degrades to *no candidate*, while micro needs one finder plus
timing — and near a vignetted corner the finder survives while the timing and
data area rot. The pipeline samples a wrong grid, reports a detection, and the
count-based trigger reads that as success.

I reproduced it independently on a decodable fixture (below): **28 wrong modules
out of an M4's 289**, slightly worse than the 23 he measured, on frames the retry
resolves to zero errors and a correct payload.

### The new trigger

Per Arch's ruling, feature-graded, gating the predicate rather than the pipeline:

| build | trigger | rationale |
|---|---|---|
| `decode` on | no candidate **decoded** | a sample that does not decode is not evidence the binarization worked |
| `decode` off | no candidate **found** (unchanged) | no decode outcome exists to key on; these builds decode downstream |

When both passes have run, the returned pass is whichever ranks higher on
`(usable, found)`, ties to the primary.

**One refinement on Arch's ruling worth flagging.** The ruling said "more
decoded results; tie → primary". I made the tie-break `(usable, found)`
lexicographic rather than `usable` alone. Straight `usable` with tie-to-primary
would *lose* results in one case: primary finds 0 candidates, retry finds 1 that
does not decode — a 0-0 tie on `usable`, so the primary's empty vector wins and
the retry's detection is thrown away. That is a regression against round 2, which
returned it. The second element fixes it and changes nothing else: with `decode`
off, `usable == found`, so the rule collapses to exactly the round-2 semantics.
Say if you would rather have the literal ruling.

`detect_all_luma` sums usable across both symbologies, so one degraded micro
sample can no longer suppress the QR side — ruling item 3.

---

## Measurements

### The bypass, reproduced and closed

M4-Q golden symbol (payload `"12345"`) at 8 px/module, vignetted 1280x720 frame,
0.55 print contrast. Sweeping depth in 0.001 steps:

| depth | primary alone | public `detect_micro_luma` |
|---|---|---|
| 0.914 | 1 det, 11 errors, decodes | same (11 errors still decode — EC absorbs them) |
| **0.916** | 1 det, **28 errors, no decode** | **1 det, 0 errors, decodes** |
| 0.919-0.920 | 1 det, 11 errors, decodes | same |
| **0.921, 0.922** | 1 det, **28 errors, no decode** | **1 det, 0 errors, decodes** |
| 0.923-0.925 | 0 det | 1 det, 0 errors, decodes |
| **0.926, 0.927** | 1 det, **28 errors, no decode** | **1 det, 0 errors, decodes** |
| 0.928+ | 0 det | 1 det, 0 errors, decodes |

Five bypass points in this 15-step window, every one now recovered exactly. Note
the 11-error rows: those decode, error correction earns its keep, and the retry
correctly does *not* fire. The trigger distinguishes "degraded but readable" from
"degraded and lost", which a module-error threshold could not have done.

### Micro retry envelope (MINOR-4)

Measured with Richard's methodology — a crate copy with `PRIMARY_WINDOW_DIV = 16`
to give a true retry-radius-alone control, since the in-tree entry points cannot
produce one. Deepest vignette depth at which the M4 golden symbol is recovered
**exactly and decoded**:

| px/module | primary alone (`/8`) | retry alone (`/16`) | public entry point |
|---|---|---|---|
| 4 | 0.825 | >= 0.999 | >= 0.999 |
| 6 | 0.898 | >= 0.999 | >= 0.999 |
| 8 | 0.904 | >= 0.999 | >= 0.999 |
| 10 | 0.911 | >= 0.999 | >= 0.999 |
| 12 | 0.930 | >= 0.999 | >= 0.999 |

`>= 0.999` is the sweep ceiling, not a cliff. The public column now matches the
retry-alone control at every scale, which is the property the trigger fix buys:
before it, the public column was capped at the primary's value wherever a
degraded sample intervened.

**And the control confirms `/16` is not a viable primary** — the same crate copy
fails 3 unit tests, 13 micro integration tests and 2 QR integration tests, all
hollowing. That is the whole justification for layering rather than switching.

### QR envelope, restated with its scope (MINOR-4)

Round 2's table was version-2 QR at 3 px/module and was presented without
qualification. It is **QR-only** and it is unchanged by this round:

| depth | primary finds | with retry | exact |
|---|---|---|---|
| 0.85 | 1 | 1 | yes |
| 0.90 | 0 | 1 | yes |
| 0.93-0.97 | 0 | 1 | yes |

At 4 px/module the primary holds to 0.93, at 6 px/module the whole range. Richard
independently found 39 QR bypass points, every one bounded at 1 module error
against a retry-alone 0 — inside every EC budget, so no payload was ever at risk
on that path. The micro table above is the one that moved.

### Cost

| case | single pass | with retry | ratio |
|---|---|---|---|
| hit — decodes on primary | 2.09 ms | 2.11 ms | 1.01x |
| **detected but not decoded** | 2.54 ms | 4.02 ms | **1.58x** |
| miss — nothing found | 3.46 ms | 6.94 ms | 2.01x |

**The middle row is new in this round and is the behaviour change to be aware
of.** Under the round-2 trigger a frame that detected-but-did-not-decode paid one
pass; it now pays two. That is the point — those are exactly the frames a retry
can rescue — but it moves cost onto a case that previously looked like a hit. For
qr-video it means an unreadable code parked in front of the camera makes every
*full* scan cost ~1.6x indefinitely. Mitigating factors: full scans are already
cadence-limited, and the ROI path is single-pass so the per-frame hot path is
untouched. Flagging it rather than burying it; if the cadence cost is unwanted,
the lever is qr-video's full-scan interval, not this trigger.

---

## qr-video re-check (DoD item)

`detect_full` uses `detect_luma`, so it inherits the new trigger; qr-video
enables `qr-detect/decode`, so it gets the decode-aware version. `detect_in_roi`
remains `detect_luma_single_pass`. All 7 qr-video tests pass.

**No behaviour surprise, but one worth stating:** qr-video's own fixtures are
synthetic grids that deliberately never decode (`tests/video.rs`: "the synthetic
grid is not a valid QR payload, so `qr-decode` returns `None`"). So under the new
trigger *every* full scan in that suite now runs both passes. Results are
identical — the tie-break returns the primary — but the suite is exercising the
double-pass path throughout, and its timings are ~1.6x. Real payloads decode on
the primary and never reach it.

---

## Files changed

Round 2's diff stands; this round adds the trigger and its tests.

### `crates/qr-detect/src/binarize.rs`

Unchanged from round 2 (a genuine revert to the shipped `/8` body with the radius
promoted to a parameter, plus `PRIMARY_WINDOW_DIV` / `RETRY_WINDOW_DIV` and the
contrast fixture), except:

| lines | change |
|---|---|
| 356–387 | `the_retry_window_holds_a_vignette_the_primary_loses` now pins the **crossover** — both radii asserted at depth 0.90, the first depth where they disagree — instead of two points either side of it (MINOR-5). Keeps the guard assertion and adds a pointer to the end-to-end pins. |

### `crates/qr-detect/src/lib.rs`

| lines | change |
|---|---|
| 28–54 | **New** crate-doc section: what "usable" means, why it is not "found something", the no-`decode` limitation stated as a limitation, and the `(usable, found)` ranking. |
| 135–142 | `detect_luma` triggers on `usable_qr` rather than `is_empty`, then ranks. |
| 144–177 | **New** `usable_qr`, cfg-gated on `decode`; **new** `better_pass` ranking helper. |
| 296–327 | `detect_micro_luma` likewise; **new** `usable_micro`, cfg-gated. Doc comment explains why the micro path is the one that needed this. |
| 355–390 | `detect_all_luma` triggers on the sum of usable across both symbologies and ranks on combined totals. |

The pipeline itself is untouched — `scan`, `detect_qr_at`, `detect_micro_at`,
`detect_all_at`, `collect_qr`, `collect_micro` and both `*_single_pass` entry
points are exactly as reviewed in round 2. Only the predicate moved.

### `crates/qr-detect/tests/detect.rs`

Unchanged from round 2.

### `crates/qr-detect/tests/micro_detect.rs`

**New** `M4_GOLDEN_12345` fixture and `golden_m4()`; **new** `vignetted_micro_frame`;
**new** tests `a_degraded_micro_sample_does_not_suppress_the_retry`,
`a_degraded_micro_sample_does_not_suppress_the_retry_in_detect_all` (both
`cfg(feature = "decode")`) and `without_decode_the_trigger_stays_a_candidate_count`
(`cfg(not(feature = "decode"))`).

### `crates/qr-video/src/detector.rs`

Unchanged from round 2 (import + `detect_in_roi` calling the single-pass entry,
plus doc paragraphs on both helpers).

---

## Testing notes

**The decodable fixture.** The BLOCKER-2 regression cannot be written against
`micro_fixture`: its data is pseudo-random, so *neither* pass decodes it, the
trigger has nothing to key on, and the test would pass vacuously against the
broken code. It needs a symbol that genuinely decodes. Rather than take a
dependency on qr-encode (which qr-detect's suites deliberately avoid, and which
was mid-rewrite in another Bob's tree while I worked), I lifted the M4-Q
`"12345"` vector verbatim from `tests-integration/tests/micro_golden.rs` — a grid
generated by **segno 1.6.6** and read back by **zxing-cpp 3.1.1**, neither of
which shares code with ours. Provenance is recorded in the fixture's doc comment.

**Why the tests sweep a band instead of pinning a depth.** The exact depths where
the primary degrades this way are an artifact of where module centres fall under
the vignette; they will move if sampling ever changes. So the tests assert the
*rule* over 0.914..=0.928 — for every depth where the primary produces a
non-decoding detection, the public entry point must decode — and then assert that
at least 3 such depths were found. Without that count the loop could go silently
vacuous, which is the exact failure mode that let round 2 through review.

**Feature coverage.** `micro_detect.rs` is `#![cfg(feature = "micro")]`, so the
no-`decode` semantics test only runs in a **`--no-default-features --features
micro`** build. That combination is not in the standard three, so I ran it
explicitly; without it the `cfg(not(feature = "decode"))` branch of the trigger
would ship untested.

## Test status

| suite | result |
|---|---|
| `cargo test -p qr-detect` (default) | 22 + 24 + 25 pass |
| `cargo test -p qr-detect --no-default-features` | 19 + 23 pass |
| `cargo test -p qr-detect --all-features` | 22 + 24 + 25 pass |
| `cargo test -p qr-detect --no-default-features --features micro` | 22 + 23 + 23 pass |
| `cargo test -p qr-video` | 7 pass |
| `cargo test -p tests-integration` | 84 pass — warped 15, multiscan 3, micro_image 12, video_session 9, full_roundtrip 11, matrix_roundtrip 11, micro_roundtrip 15, fuzz_seeded 5, micro_golden 3 |
| clippy `-D warnings`: qr-detect default / no-default / micro-no-decode, qr-video | clean |
| `cargo fmt -p qr-detect -p qr-video --check` | clean |

qr-encode compiles again in the working tree, so **tests-integration ran in-tree
this round** — round 2's HEAD-worktree workaround was not needed.

---

## Open questions

1. **The `(usable, found)` tie-break** refines Arch's literal ruling to avoid
   discarding a retry-only undecodable detection. Flagged above; revert to plain
   `usable` if the literal rule is preferred.
2. **The no-`decode` build keeps the weaker guarantee**, documented in the crate
   docs and pinned by a test. The alternative would be exposing enough for a
   caller to re-run the second radius — which `*_single_pass` already is. Reads
   right to me; confirming.
3. **The detected-but-not-decoded frame now costs 1.6x** (cost table above). This
   is inherent to a decode-aware trigger and is the intended behaviour, but it is
   a genuine change to qr-video's full-scan cost profile. Confirming that is
   accepted.
4. **Multi-code limitation** unchanged from round 2 and still documented rather
   than solved: one symbol decoding at the primary radius suppresses the retry a
   second, deeply vignetted symbol would need.
