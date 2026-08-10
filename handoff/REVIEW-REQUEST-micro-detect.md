# Review Request — Step 5d qr-detect micro

Builder: Bob-MT
Ready for Review: YES (round 2 — BLOCKER-1 and MAJOR-1 addressed, see
"Round 2" at the bottom for what changed)
Scope: `crates/qr-detect/` only. No other crate, no root `Cargo.toml`, no shared
handoff file touched (BUILD-LOG left to Arch — three Bobs were writing
concurrently; Known Gaps to fold in are listed at the bottom).

## What this is

Single-finder Micro QR (M1–M4) detection, behind the default-on `micro` feature.
Parallel API only: `MicroDetection`, `detect_micro`, `detect_micro_luma`,
`detect_all`, `detect_all_luma`. **No existing public type or signature
changed** — `Detection`, `detect`, `detect_luma`, `read_one`, `read_one_luma`
are byte-for-byte what they were.

## Files

- `crates/qr-detect/src/micro.rs` — **new**, 525 lines, `micro`-gated. The whole
  algorithm.
- `crates/qr-detect/tests/micro_detect.rs` — **new**, 695 lines, whole file
  `#![cfg(feature = "micro")]`. 22 end-to-end tests.
- `crates/qr-detect/src/lib.rs` — micro API (`MicroDetection` 145–163,
  `detect_micro*` 165–181, `detect_all*` 183–206, `collect_micro` 208–220,
  `build_micro_detection` 222–237) plus a refactor of the shared pipeline front
  into `scan` (81–93) and `collect_qr` (95–103) so `detect_all` binarizes once.
- `crates/qr-detect/src/grouping.rs:206–219` — `group` now also returns a
  per-finder "consumed by a triple" flag vector. Same algorithm, same order; the
  three internal tests were updated to destructure.
- `crates/qr-detect/src/binarize.rs:83–84` + tests at 205–321 — **shared-stage
  change, please look hardest here**: adaptive window radius `min(w,h)/16` →
  `min(w,h)/8`. Rationale below.
- `crates/qr-detect/Cargo.toml` — feature wiring.

## Algorithm (micro.rs)

1. **Square geometry** (`square_axes`, 113). 720 rays (0.5°) from the finder
   centre; each measures the radius of the outer dark ring. Radius is minimal on
   the square's axes and maximal at its corners, so minimising the 4-ray sum over
   θ ∈ [0°, 90°) gives the rotation mod 90, and the maximum-radius *point* in
   each diagonal sector is a corner. Opposite edges are averaged →
   two edge vectors of length `7 · module`. This also yields a
   **rotation-invariant** module size; `Finder::module` cannot be used, it is
   inflated by rotation (the `axis_scale` story from Step 2d).
2. **Orientation** (`detect_from_finder`, 67). Mod-90 leaves four frames; the
   symbol is in exactly one quadrant. Each frame is tried, times five angular
   tweaks (0, ±0.5°, ±1°) that absorb corner-localisation error — over 17 modules
   half a degree is already an eighth of a module of drift. First frame scoring
   zero function-pattern mismatches wins; otherwise the best under the tolerance.
3. **Dimension** (`walk_timing`, 350). A ray down the centre of row 0 (and of
   column 0), starting at grid 6.5 inside the finder ring. Transitions are
   counted, not module centres sampled — so an error in the module estimate
   shifts every transition but does not change how many there are, which makes
   the walk self-calibrating. Expected: finder edge at 7, one transition per
   module from 8, symbol edge at `dim`. Gaps must stay within 0.55–1.45 modules.
   Both walks must agree, and `dim` must be 11/13/15/17.
4. **Sampling** (`try_frame`, 277). Correspondences: finder centre ↔ (3.5, 3.5),
   and the two *measured* timing endpoints ↔ (dim, 0.5) and (0.5, dim). The
   fourth is the parallelogram completion in both spaces, so `quad_to_quad`
   reproduces exactly the affine map through those three. A micro symbol has no
   alignment pattern — there is no fourth independent measurement, so true
   perspective cannot be pinned down. Existing `PerspectiveTransform` reused
   unchanged.
5. **Validation** (`function_mismatches`, 403). The sampled grid must contain a
   micro symbol's finder, separator and both timing patterns: ≤ 1 mismatch out
   of 68–82 modules. Format information is excluded (symbol-dependent; that is
   `decode_micro`'s job, with error correction). **This is the discriminator**
   that stops finder-shaped noise being reported.

`ray_transitions` (238) is shared by steps 1 and 3 and carries the one
non-obvious rule: colour runs shorter than a third of a module are ignored. A
rotated symbol is resampled, so its edges binarize into a ragged staircase and a
shallow-angle ray picks up one- and two-pixel specks. Without the filter the
square measurement came out **6° and 17% off** at 10° rotation and the rotation
envelope was full of holes; with it, every version is exact across the sweep.
That was the single biggest find of this step.

## Feature wiring — what I chose and why

```toml
default = ["decode", "micro"]
decode  = ["dep:qr-decode"]
micro   = ["qr-core/micro", "qr-decode?/micro"]
```

There is deliberately **no third `micro-decode` feature**. A detected micro
symbol is decoded exactly when `micro` and `decode` are both on, which is the
same rule the QR path already follows, and `MicroDetection::decoded` is gated on
`decode` just like `Detection::decoded`.

The weak `qr-decode?/micro` is what makes that safe in both directions: it turns
qr-decode's own micro module on **if and only if** `decode` has already pulled
qr-decode in, so `--features micro` alone never drags the optional dependency
into the build, and `micro` + `decode` can never reach for a `qr_decode::micro`
that was compiled out. Relying on default-feature unification instead would work
today but would break silently the moment anything in the graph set
`default-features = false` on qr-decode.

qr-decode's `micro` module landed while I was building, so this is wired live,
not stubbed: `detected_symbols_are_decoded` (tests 645) renders ISO Figure I.4,
detects it and gets `"01234567"` back out of `decode_micro`.

Verified green: default, `--no-default-features`, `--no-default-features
--features micro`, `--no-default-features --features decode`, `--all-features`.

## The binarize change (the one thing outside the micro module)

`radius = min(w,h)/16` → `/8`. Bradley's rule of thumb is a window of an eighth
of the image *width*, i.e. this radius — but that assumes features much smaller
than the window. The largest solid feature in any symbol is the finder's 3×3
core, and a micro symbol is small enough that a normally framed shot puts the
core at a fifth of the image width. Where the window fits inside the core the
local mean *is* the core's colour, the centre pixel is not below it, and it flips
to light. The finder scan's vertical cross-check then walks a broken dark band
and the symbol is lost — **zero finders found**, which is exactly what happened
before this change: an M2 at 8 px modules with a 4-module quiet zone (168 px)
punched a 4×4 px hole in its own finder.

**Correction to round 1 (Richard, BLOCKER-1).** I wrote that this was a
pre-existing bug that "hits a QR v1 filling a 500 px frame". That overstates it
and the arithmetic says so. The window must exceed the core:
`2·radius + 1 > 3·module`, which for an `S`-module symbol with a `Q`-module quiet
zone reduces to `S + 2Q > 24` under the old `/16` and `S + 2Q > 12` under `/8`.
A spec-compliant QR v1 with its mandated 4-module quiet zone is `21 + 8 = 29`
and clears the old threshold; only a QR framed with a quiet zone under ~1.5
modules — sub-spec — ever broke. **This is a micro-only bug**: a legal M1 with
ISO's 2-module quiet zone is `11 + 4 = 15`, which fails the old threshold and
clears the new one with 25% headroom. The change is made *for* micro, and it is
argued on that basis, not on a QR regression that does not exist.

**It costs the QR/video path, and that cost is now pinned in the suite.** Per
Arch's ruling `/8` stays (the middle grounds do not exist: an absolute radius cap
breaks fullscreen micro, a pixelwise union inherits the vignette failure, an
intersection reinstates the hollowing; two-pass binarize is logged as its own
future step). The suite now locks the trade-off in **both** directions:

- `binarize::tests::a_solid_block_does_not_hollow_out` (205) — the direction the
  change fixed. Fails on `/16`, passes on `/8`. Verified both ways.
- `binarize::tests::adaptive_survives_a_vignette_to_the_documented_depth`
  (fixture 232–292, test 294–321) — **new**, the direction the change costs. A 25-module symbol at 3 px/module
  at 45% print contrast, in the corner of a 1280×720 frame, under a radial
  vignette. Module errors out of 625 on this fixture:

  | vignette depth | 0.50 | 0.70 | 0.85 | 0.90 | 0.95 |
  |---|---|---|---|---|---|
  | `/8` (shipping) | 0 | 0 | 0 | 13 | 32 |
  | `/16` (old) | 0 | 0 | 0 | 0 | 0 |

  The test pins **depth 0.85** — frame corner at 15% of centre illumination —
  as the envelope that must hold. The comment states the measured cost in the
  same table and names two-pass binarize as the fix that keeps both envelopes.
  My knee sits deeper than the 0.80 Richard measured; his profile and symbol
  placement differ from mine, and I pinned what my own fixture actually
  survives rather than a number I could not reproduce.

## Honest envelope

| condition | result |
|---|---|
| upright, M1–M4, all 4 masks | exact |
| module size | 3 px to 20 px exact; ISO's 2-module quiet zone is enough |
| rotation | **exact at every 2.5° from 0° to 45°, all four versions**, and the frame search is 90°-symmetric so that covers the circle (spot-checked at 50/90/137/180/225/310°). Also exact at 5 px modules under rotation, and at 7.5/12.3/22.5/33.7/41.1°. |
| perspective | **tilt up to 4%** of the symbol width (M1 reaches 6%, M2–M4 stop at 4%). Beyond that it fails closed. This is the price of having no alignment pattern: the map is affine and all the tolerance is the half-module sampling margin. |
| noise | ±40 luma amplitude exact |
| coexistence | QR + micro in one image: both detected, neither steals the other's finders. Two micro symbols in one image: both. A QR alone yields zero micro detections at v1/v2/v7. |
| rejection | a bare finder pattern, and a symbol with its horizontal timing filled solid, both yield nothing; two damaged function modules are rejected by the validator |

Rotation is a *stronger* envelope than the QR path's (KG-9), because the micro
module size comes from ray-measured square corners rather than from
scanline-derived runs.

One honesty correction to round 1: "never a wrong grid" was too strong. Richard's
MINOR-1 is right — a timing module bridged by binarisation at matching offsets in
*both* walks truncates the walk to a smaller valid size and yields a
self-consistent crop that scores 0 mismatches. It needs the damage to be
symmetric, and `decode_micro` rejects the crop because the format information
lands off-place, but detection alone does not catch it. Arch is logging it.

## Tests

22 in `tests/micro_detect.rs`, 3 unit tests in `micro.rs`, 2 new in
`binarize.rs`. Fixtures are built here from qr-core micro primitives (placement
iterator + the four masks + format-info writer), so nothing depends on
qr-encode; one fixture is ISO Figure I.4 transcribed module for module, which is
also the decode test. Generated fixtures carry pseudo-random data rather than an
RS-valid stream — detection never inspects codewords, and inventing an RS layout
here would duplicate and possibly contradict the encoder. The decode test asserts
both directions of that: Figure I.4 decodes to its payload, a synthetic fixture
returns `None` rather than a wrong answer.

The render/rotate/pad/noise/perspective helpers are a deliberate copy of the ones
in `detect.rs` — each integration test file stays self-contained. Say the word if
you would rather have a `tests/common/mod.rs`.

## DoD

Round 2, all re-run from clean:

| check | result |
|---|---|
| `cargo test -p qr-detect` | 20 lib + 21 detect + 22 micro, green |
| `cargo test -p qr-detect --no-default-features` | 17 lib + 20 detect, green (micro compiled out) |
| `cargo test -p qr-detect --no-default-features --features micro` | 20 + 20 + 21, green |
| `cargo test -p qr-detect --no-default-features --features decode` | 17 + 21, green |
| `cargo test -p qr-detect --all-features` | 20 + 21 + 22, green |
| `cargo clippy -p qr-detect --all-targets -- -D warnings` | clean, default and `--no-default-features` |
| `cargo test -p qr-video` | 7 passed, green |
| `cargo test -p tests-integration` | 54 passed across 6 binaries, green |
| `cargo check --workspace` | green |
| `cargo fmt --check` | clean |

The micro suite is 21 under `--features micro` and 22 with `decode` as well —
the extra one is `detected_symbols_are_decoded`, which is `#[cfg(feature =
"decode")]`.

## Open questions

All three from round 1 are ruled on and applied: `/8` stays with two-way
coverage (BLOCKER-1), `MAX_FUNCTION_MISMATCHES` is 1 with a test that reaches the
validator (MAJOR-1), and the duplicated test helpers stay duplicated (MINOR-4).
Nothing open from my side.

## For BUILD-LOG (Arch to fold in)

- **KG-11** — detect micro: perspective tolerance is ~4% tilt because a micro
  symbol has no alignment pattern to supply a fourth correspondence; the grid map
  is affine. Fails closed. Lever if it ever matters: iterate the map against the
  timing patterns after the first sample.
- **KG-12** — detect: `binarize`'s adaptive window is `min(w,h)/8` so that a
  finder's 3-module core cannot hollow out in a tightly framed shot
  (`S + 2Q > 12` rather than `> 24`; a legal M1 at `11 + 4 = 15` needs it, a
  spec QR v1 at `21 + 8 = 29` never did). The cost is measured and deliberate: a
  wider window tracks curved illumination less closely, so a small low-contrast
  symbol at a frame corner loses modules under a vignette deeper than 0.85 where
  `/16` held. Pinned in both directions by
  `a_solid_block_does_not_hollow_out` and
  `adaptive_survives_a_vignette_to_the_documented_depth`. The fix that keeps both
  envelopes is two candidate binarizations driven through the finder scan — its
  own step, not a threshold tweak.

## Round 2 — what changed since the first submission

- `binarize.rs` — added `adaptive_survives_a_vignette_to_the_documented_depth`
  (232–321), the coverage in the costing direction, pinning vignette depth 0.85
  with the measured `/8` vs `/16` table in the comment. The radius itself is
  unchanged per Arch's ruling.
- `micro.rs:63` — `MAX_FUNCTION_MISMATCHES` 2 → 1, with the overshoot-phantom
  arithmetic in the doc comment as the reason the budget is tight.
- `tests/micro_detect.rs:663–695` — `the_function_pattern_budget_is_one_module`,
  the first test that reaches the validator at all. Corruption is confined to
  two corners of the finder's 3×3 core turned light — the only function modules
  that can be damaged without disturbing the ray scan or the timing walk
  (darkening a separator module extends the finder ring into it and inflates
  that ray; holing the ring invalidates it). One casualty: detected, grid exact.
  Two: rejected. **Verified two-way** — with the constant put back to 2 the
  two-casualty case is accepted and this test fails, so it locks the value and
  not just the code path.
- Review request: the "pre-existing QR bug" claim corrected to the
  `S + 2Q > 24` / `> 12` arithmetic, KG-12 rewritten, the envelope's "never a
  wrong grid" softened per MINOR-1.
