# Review Request — Step 4: `qr-detect` bug fixes (CI-1, CI-2)

**Builder:** Bob-F
**Ready for Review:** YES
**Scope:** `crates/qr-detect/` only. `tests-integration/` was **not** touched — only run.

---

## Result

```
cargo test  -p qr-detect                                          15 + 21 passed, 0 failed
cargo test  -p qr-detect --no-default-features                    15 + 20 passed, 0 failed
cargo clippy -p qr-detect --all-targets -- -D warnings            clean
cargo clippy -p qr-detect --all-targets --no-default-features -- -D warnings   clean
cargo test  --workspace                                           all green except the
                                                                  designed CI-1 tripwire
```

Both `#[ignore]`d integration pins now pass; the companion tripwire fails on
purpose. Details under **Integration pins** below.

---

## CI-1 — dimension estimate was not rotation-corrected

### Root cause

`grouping::score_triple` mixed two quantities measured in different frames:

- `leg_top` / `leg_left` — finder centre-to-centre **Euclidean** distances,
  rotation-invariant.
- `module` — the mean of `Finder.module`, which `finder.rs` derives from
  **horizontal and vertical scanline run lengths**, i.e. measured against the
  *image* axes.

A horizontal ray from the centre of a finder rotated by `θ` leaves each of the
concentric square rings at `a / max(|cos θ|, |sin θ|)` instead of `a`, so every
run — and hence the module estimate — is stretched by `1 / max(|cos θ|, |sin θ|)`.
The vertical profile stretches by exactly the same factor. `leg / module + 7`
therefore undershot the true dimension by `(dimension - 7) · (1 - cos θ)`.
`snap_dimension` absorbs roughly two modules of that, then silently snaps to a
*smaller* valid version: `detect()` reports success, the matrix has the wrong
side length, and decode fails. The `leg_err > 0.20` guard cannot catch it — the
legs themselves are fine.

There was a **second, independent** error source that only showed up once the
first was corrected: `matches_ratio` estimated the module as `total / 7`. A
scanline crossing a finder off-centre clips the 3-module core (its chord through
the rings shortens with the offset) while leaving the four single-module runs
intact — those are *differences* of two chords, so the offset cancels. Under
rotation the clipping is severe, worst at 45 degrees where the scanline runs
corner-to-corner, and `total / 7` under-reported the module by >10%.

### Approach chosen

**1. Correct the module to the symbol's own frame, analytically.**
`grouping::axis_scale(u, v)` multiplies the axis-aligned module estimate by
`max(|u.x|, |u.y|)`, where `u` is the unit vector along the top-left ->
top-right leg (and likewise `v` for the left leg; both are averaged). For a pure
rotation `max(|u.x|, |u.y|) == max(|cos θ|, |sin θ|)`, so this undoes the stretch
**exactly** and folds at 45 degrees — matching the observed failure envelope to
the degree. It is identity for an upright symbol, and bounded in `(0.707, 1.0]`.

**2. Make the underlying measurement offset-invariant.** `matches_ratio` now
returns `(runs[0] + runs[1] + runs[3] + runs[4]) / 4.0`. `total / 7` is kept as
the reference for the `1:1:3:1:1` *shape* test, which is what the ratio is
stated against; only the returned *estimate* changed.

### Rationale — why not the alternatives

- **Timing-pattern module counting (ISO-sanctioned).** Exact in principle, but
  the timing row sits 3 modules off the finder-centre line, so placing the scan
  needs a module estimate first — circular — and it is the most damage-sensitive
  structure in the symbol. Kept in reserve; not needed, since the corrected
  estimate is no longer ambiguous (see the envelope table).
- **ZXing's `sizeOfBlackWhiteBlackRunBothWays` — re-walk the finder along the
  leg direction.** Correct, but it re-walks the image for every candidate triple
  and its accuracy is only `±1 px` over a 7-module span (~1.4% at 10 px/module,
  ~1.3 modules of dimension error at v20). The analytic correction reuses the
  existing multi-scanline, vote-averaged measurement, which is already far more
  precise, and adds no image traversal at all.
- **Joint solve over candidate dimensions using leg disagreement.** Does not
  work: `leg_top/(D-7)` and `leg_left/(D-7)` share the divisor, so their
  disagreement is independent of `D` and carries no information about it.

### Measured envelope (synthetic fixtures, `--test detect`, angle folded to 0..=45)

Last angle at which `detect()` returns exactly one candidate with the correct
dimension, swept in 5-degree steps:

| version | v1 | v5 | v10 | v20 | v40 |
|---|---|---|---|---|---|
| before | 30 | 20 | 15 | *never* | *never* |
| after  | **45** | **45** | **45** | **30** (45 also passes) | **30** |

v1/v5/v10 are now correct across the whole `0..=45` fold, i.e. at every angle.
The residual v20/v40 limit at 35-40 degrees is a precision floor, not the
systematic bias: it needs the module measured to ~1% for a 170-module span.
Logged as a Known Gap suggestion below.

---

## CI-2 — phantom extra candidate under rotation

### Root cause (confirmed by instrumenting `group()`)

Not a duplicate triple — `group()` already blocks a finder index from being
reused. The v7-M @ 20 degrees repro produced **10** finder clusters: the three
genuine ones (22 votes each) plus seven weak ones (3-7 votes) thrown up by
alignment patterns and dense data runs, which clear the `1:1:3:1:1` test on a
handful of scanlines under rotation. Three of the weak ones sat at roughly a
right angle and scored as a second, smaller symbol *entirely inside the real
one*:

```
accept dim=45 tl=(360.95,231.07) tr=(718.14,361.09) bl=(230.84,588.23) score=0.050 idx=[0,3,9]
accept dim=21 tl=(627.75,424.00) tr=(622.67,561.33) bl=(479.83,430.50) score=0.906 idx=[6,7,8]
```

It survived CI-1's fix (the corrected estimate is right for both — the phantom
*is* a plausible small symbol in isolation), so it needed its own fix.

### Approach chosen

Reject a triple whose three finder centres all fall inside an already-accepted
candidate's symbol area. Two real symbols never overlap — a quiet zone separates
them — so a fully nested triple cannot be a second symbol.

`frame_coords` solves the accepted candidate's affine frame properly (`p - tl =
a·u + b·v`, via the 2x2 determinant) rather than projecting onto near-orthogonal
axes. Bounds are `-margin ..= 1 + margin` with `margin = 3.5 / (dimension - 7)`,
because the frame runs `0..=1` between finder *centres* and the symbol extends
3.5 modules past them on every side — that exact margin, not a magic number, is
what catches a phantom finder sitting in the symbol's outer band (one real case
landed at `b = 1.0021`).

Requiring **all three** centres to be nested, rather than any one, keeps the rule
conservative: only wholly-nested triples are dropped, so two adjacent or
touching symbols are unaffected (pinned by
`adjacent_symbols_are_not_treated_as_nested` and the pre-existing
`groups_two_symbols` / `two_codes_in_one_image`).

Rejected alternative: validating sampled format info. It is a stronger filter but
`decode` is an optional feature, so the check would have to be duplicated or the
`--no-default-features` build would lose it; and it would not distinguish a
phantom from a genuinely damaged symbol.

---

## Files changed

| File | Lines | What |
|---|---|---|
| `crates/qr-detect/src/finder.rs` | 22-34 (doc), 48 | `matches_ratio` returns the mean of the four single-module runs instead of `total / 7`; the shape test is unchanged. |
| `crates/qr-detect/src/finder.rs` | 99, 102 | `vertical_check` returns `matches_ratio`'s estimate instead of recomputing `total / 7`. |
| `crates/qr-detect/src/grouping.rs` | 78-97 | New `axis_scale`, with the derivation of the `1 / max(\|cos θ\|, \|sin θ\|)` stretch in the doc comment. |
| `crates/qr-detect/src/grouping.rs` | 117-123 | `u` / `v` hoisted to unit axis vectors; the right-angle `cos` is now their dot product (was `v1`/`v2`, unnormalised + divided). Behaviour identical. |
| `crates/qr-detect/src/grouping.rs` | 135-138 | **The CI-1 fix**: the averaged finder module is scaled by `axis_scale(u, v)` before the dimension is derived from it. `expected_leg` / `leg_err` below inherit the corrected value. |
| `crates/qr-detect/src/grouping.rs` | 169-204 | New `frame_coords` + `is_nested_in`. |
| `crates/qr-detect/src/grouping.rs` | 230-232 | **The CI-2 fix**: `group()` skips a triple nested inside an already-accepted candidate. |
| `crates/qr-detect/src/grouping.rs` | 296-396 | Four new unit tests (see below). |
| `crates/qr-detect/tests/detect.rs` | 175-185 | New `pad` helper — rotation needs `(sqrt(2)-1)/2` of the side on each edge to avoid clipping. |
| `crates/qr-detect/tests/detect.rs` | 352-420 | The rotation matrix: helper + 8 tests. |

No existing test was modified or deleted. No existing assertion encoded the buggy
behaviour, so nothing needed justifying away.

`cargo fmt --check` reports three pre-existing diffs in `qr-detect`
(`binarize.rs:95`, `geometry.rs:109`, `sampler.rs:42`, plus `grouping.rs`'s
untouched `leg_err` and `sort_by`) — the crate was already not fmt-clean under
this toolchain. Everything I added is fmt-clean; I deliberately did not reformat
lines I did not otherwise change.

---

## Test evidence

### New unit tests (`grouping.rs`)

- `axis_scale_is_identity_when_upright` — an upright symbol must be untouched.
- `axis_scale_undoes_rotation_stretch` — 0 / 30 / 45 degrees against the closed
  form, plus 30 == 60 to pin the fold.
- `rotated_symbol_keeps_its_true_dimension` — a synthetic v10 triple rotated 30
  degrees with the module pre-stretched by `1 / cos 30`: must still snap to 57.
  This is the CI-1 mechanism in isolation, no imaging involved.
- `nested_phantom_triple_is_rejected` — the CI-2 shape: a real v7 triple plus
  three interior "finders" 16 modules apart. Asserts one candidate, and that the
  same phantom triple *alone* is still a valid candidate — proving the rejection
  is the nesting rule, not the geometry.
- `adjacent_symbols_are_not_treated_as_nested` — two v1 symbols with a 4-module
  quiet zone between them; neither may swallow the other.

### New rotation tests (`tests/detect.rs`) — the requested failure matrix

Each renders a synthetic symbol, pads it so no corner clips, rotates, and
requires exactly one detection matching **module-for-module**. Every one of these
failed before the fix.

| test | previously |
|---|---|
| `rotated_v5_25deg` | failed (envelope was 23 deg) |
| `rotated_v5_30deg` | failed |
| `rotated_v7_20deg` | failed (envelope was 20 deg, marginal) |
| `rotated_v10_17deg` | failed (envelope was 17 deg, marginal) |
| `rotated_v20_15deg` | failed (envelope was 12 deg) |
| `rotated_v10_40deg` | failed — the fold point is the worst case, not the ends |
| `rotated_v5_45deg` | failed — needed the `matches_ratio` fix too |
| `rotated_v5_60deg` | failed — pins that past 45 deg the factor folds back |

### Integration pins (`tests-integration/tests/warped.rs`, run only, not edited)

```
cargo test -p tests-integration --test warped -- --include-ignored
  rotated_30_degrees ............................... ok   (was #[ignore], CI-1)
  single_rotated_symbol_yields_single_candidate .... ok   (was #[ignore], CI-2)
  rotation_dimension_underestimate_is_still_present  FAILED  <-- expected
  ... 13 others ................................... ok
```

The failure is the tripwire behaving exactly as Bob-I designed it:

```
CI-1 appears fixed: sampled dimension 37 now matches the true 37
  — drop this test and un-ignore rotated_30_degrees
```

**Arch:** the two `#[ignore]` attributes can come off, and
`rotation_dimension_underestimate_is_still_present` should be deleted. Both are
in `tests-integration`, outside my write scope.

`cargo test --workspace` is otherwise fully green — including all 11
`full_roundtrip`, 11 `matrix_roundtrip`, 9 `video_session` and 5 `fuzz_seeded`
tests, and `qr-video`'s own suite.

---

## Known Gaps — proposed updates (BUILD-LOG.md not edited; another agent has it open)

```
- KG-8 — CLOSED. Rotation correction landed (grouping::axis_scale +
         finder::matches_ratio). v1/v5/v10 now correct at every angle;
         v20/v40 correct to 30 deg.
- KG-9 — CLOSED. Nested phantom triples rejected (grouping::is_nested_in).
- KG-10 — NEW (low): at v20+ and >=35 deg of rotation the module estimate is
          within ~1% but the dimension needs better than that over a 170-module
          span, so the snap can still miss by one version. The ISO timing-pattern
          count is the escalation path if a real camera case ever needs it.
```

---

## Open questions for Arch

1. **KG-10 / timing-pattern cross-check.** I stopped at the point where the
   systematic bias is gone and only measurement precision limits v20+ past 35
   degrees. Adding a timing-pattern module count to arbitrate between adjacent
   candidate dimensions would close it, at the cost of a second image traversal
   and a new damage-sensitivity. Worth doing now, or is 30 degrees at v40 enough?
2. **Weak finder clusters.** The v7 repro produced seven spurious finder
   clusters against three real ones. The nesting rule handles the consequence;
   raising `finder::cluster`'s `n >= 3` vote floor, or scoring clusters
   relatively, would handle the cause. Out of this step's scope — say the word.
