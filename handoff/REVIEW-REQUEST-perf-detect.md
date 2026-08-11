# REVIEW-REQUEST — Step 11a, detect performance

**Ready for Review: YES**
Builder: Bob-OD. Scope: `crates/qr-detect/` only. No new deps, no `unsafe`,
no changes to `benchmarks/`.

---

## Numbers

Apple M4, `bench` profile, `cargo bench -p benchmarks --bench detect`. "Before"
is a run of the same command at the start of this session on this machine — it
reproduces `benchmarks/README.md`'s published baseline within a few percent, and
is used here rather than the README figures so both columns come off one
machine in one sitting. Medians.

| bench | before | after | change |
|---|---:|---:|---:|
| happy path: 1 v5 symbol @8px/module | 2.09 ms | **687 µs** | −67 % |
| happy path, single pass (no retry) | 2.09 ms | **705 µs** | −66 % |
| rotated 30 degrees | 2.11 ms | **705 µs** | −67 % |
| miss path: no symbol (primary + retry) | 3.99 ms | **1.13 ms** | −72 % |
| miss path, single pass (no retry) | 1.99 ms | **566 µs** | −72 % |
| miss path: flat frame (no symbol, no clutter) | 6.53 ms | **318 µs** | −95 % |
| miss path: flat frame, single pass (no retry) | 3.27 ms | **319 µs** | −90 % |
| detect_micro_luma: 1 M4 symbol | 2.14 ms | **761 µs** | −64 % |
| detect_all_luma: 1 v5 + 1 M4 | 2.52 ms | **1.12 ms** | −56 % |

Both targets met: flat frame 318 µs against the 1 ms target, happy path 687 µs
against 2.13 ms. Nothing regressed.

`cargo bench -p benchmarks --bench video` falls out of the same change (not
re-measured before, so the left column is the README baseline):

| bench | README baseline | after |
|---|---:|---:|
| steady-state ROI frame | 902.71 µs (1.11 k/s) | **332.92 µs (3.00 k/s)** |
| full-scan cadence frame | 2.15 ms (464 /s) | **704.62 µs (1.42 k/s)** |

### Where the time went

From a temporary `#[cfg(test)]` probe (since deleted), 720p, primary radius, µs
per whole-frame pass. **Measured** rows were timed directly; **inferred** rows
are arithmetic on end-to-end bench deltas and should be read as
order-of-magnitude only.

| stage | before | after | provenance |
|---|---:|---:|---|
| `find_finders`, flat frame | 472 | 116 | both measured |
| `find_finders`, textured frame | 507 | 187 | both measured |
| Otsu histogram, flat frame | 1 141 | 179 | both measured (1 lane vs 4) |
| `binarize`, flat frame | ~3 100 | 249 | before inferred, after measured |
| `binarize`, textured frame | ~1 900 | 421 | before inferred, after measured |
| ├ column slide (`add_row` + `sub_row`) | — | 77 | measured |
| ├ row prefix sum | — | 223 | measured |
| ├ threshold, interior | — | 76 | measured |
| ├ threshold, edge columns | 138 | ~45 | before measured, after inferred |
| ├ integral image build | — | 0 (removed) | — |

The two numbers worth carrying forward: the prefix sum is now the largest single
item in the adaptive pass, and on a flat frame the whole cost is one Otsu
histogram plus one finder scan.


---

## Changes

### `crates/qr-detect/src/binarize.rs`

1. **`can_be_dark` (new, binarize.rs:162-169) + the two early returns
   in `binarize` (binarize.rs:114-129).** A pixel is dark when `value * count * 100 < sum * 88`; since
   `sum <= count * max` and `value >= min`, a frame with `min * 100 >= max * 88`
   cannot produce one. Two bytes of dynamic range decide it, so a blank or
   near-blank frame goes straight to the Otsu fallback instead of building an
   integral image and evaluating 921 600 windows to be told the same thing.
   **The fallback itself is untouched** — this is the exact case that used to
   reach Otsu via `dark_count == 0`, so the output is bit-identical; only the
   wasted adaptive pass is gone. The `< 12 %` contrast frames Otsu rescues are
   still rescued.

2. **`Binary::radius_invariant` + `is_radius_invariant()` (binarize.rs:42-57).** Set when
   the adaptive pass was skipped for a reason that holds at *every* radius —
   the size floor, or `can_be_dark`. Deliberately *not* set when Otsu runs
   because the adaptive pass came back empty: another radius genuinely might
   not, and suppressing the retry there would lose symbols.

3. **`dark_count == 0 || dark_count == total` → `!dark.iter().any(...)`
   (binarize.rs:144).** The all-dark arm is provably unreachable (the frame's brightest
   pixel can never be below `0.88 ×` its own window mean — proof is in the
   comment), and `any` short-circuits on the first dark pixel instead of
   counting all 921 600.

4. **`adaptive` / `adaptive_at` (binarize.rs:228-249, 303-398): the integral image is gone.** The
   window sum is split vertically and horizontally: `colsum[x]` holds the
   window's rows in column `x` and slides down the frame one row at a time, and
   a per-row prefix over it turns any horizontal span into one subtraction.
   State is two `w`-element buffers that stay in cache instead of a
   `(w+1) × (h+1)` `u64` array — 7.4 MB of traffic per pass at 720p. Every loop
   is a flat walk over slices; no per-pixel bounds check, no data-dependent
   indexing.

5. **`Acc` trait + `fits_u32` (binarize.rs:171-226).** The threshold comparison only
   vectorizes at a width the target has vector multiply for, which is `u32`. The
   accumulator is therefore generic, and `fits_u32` proves at runtime which
   instantiation is safe: 32 bits covers everything up to 4K, `u64` takes over
   beyond that. Both are the same code, so they agree bit for bit —
   `tests::the_two_accumulator_widths_agree` pins that at three radii, since no
   fixture in the workspace is big enough to reach the wide path on its own.
   The Bradley constants are divided by four (`100/88` → `25/22`, exact) purely
   to buy accumulator headroom.

6. **Row interior vs edges split (binarize.rs:352-395).** The interior — every column where
   the window does not touch a frame edge — has a constant pixel `count`, so it
   is one loop over three slices. The `2 × radius` edge columns get two separate
   loops with the clamp already resolved rather than re-deriving `min`/
   `saturating_sub` bounds per pixel; they were costing as much as the whole
   interior.

7. **`prefix_sum` + `PREFIX_BLOCK` (binarize.rs:262-301).** The prefix is the one
   loop-carried dependency left. Blocked so consecutive blocks are independent
   and only the carry fold stays on the critical path.

8. **`histogram` + `HISTOGRAM_LANES` (binarize.rs:400-436).** `hist[p] += 1` serialises on
   store-to-load forwarding whenever neighbouring pixels share a value, which is
   *every* pixel of the frame that reaches Otsu most often. Four interleaved
   tallies: 1 141 µs → 179 µs on a uniform 720p frame. Four rather than eight or
   sixteen is measured, not assumed — the sweep is in the doc comment.

9. **`Binary::row()` (binarize.rs:59-69).** Row slice accessor for the finder scan.

### `crates/qr-detect/src/finder.rs`

10. **`find_finders` run extraction (finder.rs:145-175) and
    `run_length` (finder.rs:122-141).** This is the only stage that touches every pixel after binarization, and it
    was reading them one bounds-checked `Binary::get(i64, i64)` at a time
    through a nested `while`. Now it walks the row as a slice and finds each
    run's end with a blocked `any` the backend can vectorize, so a long uniform
    stretch is skipped a block at a time. The `runs` buffer is hoisted out of
    the row loop (it was a fresh `Vec` per row). Same runs, same order, same
    output. 472 → 116 µs on a flat frame.

### `crates/qr-detect/src/lib.rs`

11. **The retry is skipped when the primary pass was radius-invariant**
    (`detect_luma`, `detect_micro_luma`, `detect_all_luma`; helper
    `settled_by_binarization`, type alias `Pass<T>`). Provably lossless: the
    second pass would rebuild the same `Binary` and rescan it. The `None` arm
    folds in the other dead retry — a luma buffer whose length does not match
    its dimensions is rejected identically at both radii. Crate docs gained a
    "When the retry is skipped outright" section.

    A genuinely flat frame now costs **one** cheap pass (one min/max sweep, one
    Otsu, one finder scan) rather than four binarizations.

**No gratuitous luma copy found.** `detect` hands `img.as_raw()` straight
through and every stage borrows it; the only allocations per pass are the
`Vec<bool>` output and the two `w`-element scratch buffers. (`qr-video`'s ROI
path does copy, but that is a real crop and out of scope here.)

---

## Tests

Two added, both in `binarize.rs`:

* `the_two_accumulator_widths_agree` — `u32` and `u64` instantiations compared
  bit for bit over a vignetted worn-print fixture at three radii, with a guard
  that the fixture does not binarize to one flat colour (which would let two
  broken implementations agree).
* `a_frame_too_flat_to_threshold_is_settled_at_any_radius` — pins both halves of
  the retry skip: the flag the pipeline reads, and the claim it makes (both
  radii produce identical output). Run at a 9 % spread as well as a perfectly
  uniform frame, so it stands for a near-blank camera frame rather than only the
  synthetic one. Also asserts the flag is *not* set on a frame with real dark
  content.

Every pre-existing fixture is untouched and green — specifically the ones the
brief called out: `a_solid_block_does_not_hollow_out`,
`a_solid_block_at_worn_print_contrast_does_not_hollow_out`,
`adaptive_survives_a_vignette_to_the_documented_depth`,
`the_retry_window_holds_a_vignette_the_primary_loses`, and the end-to-end
retry-path pins `detect::a_deep_vignette_is_recovered_by_the_retry_pass` and
`micro_detect::a_degraded_micro_sample_does_not_suppress_the_retry`.

## DoD

| check | result |
|---|---|
| `qr-detect` default | 25 + 24 + 28 pass |
| `qr-detect --no-default-features` | 21 + 23 pass |
| `qr-detect --no-default-features --features decode` | 21 + 24 pass |
| `qr-detect --no-default-features --features micro` | 25 + 23 + 26 pass |
| `qr-detect --all-features` | 25 + 24 + 28 pass |
| `qr-video` | 7 pass |
| `tests-integration` | 84 pass |
| `cargo clippy -p qr-detect --all-targets` | clean |
| `cargo clippy -p qr-detect --no-default-features --all-targets` | clean |
| `cargo fmt -p qr-detect -- --check` | clean |
| `unsafe` | none |
| new dependencies | none |

---

## Open questions / uncertainties

1. **Scope.** Item 10 (`finder.rs`) is past the two items in the brief. I did it
   because the measurement said to: after binarize shrank, the finder scan was
   40 % of the happy path and 66 % of the flat frame, and the fix is the same
   "walk slices, not bounds-checked indices" the brief asks for in item 2. It is
   semantics-preserving. Say the word and it comes out — the two binarize items
   alone land the happy path at ~1.14 ms and the flat frame at ~715 µs, which
   still meets both targets.

2. **The all-dark arm I deleted** (change 3) rests on a proof in a comment, not
   a test — no fixture can cover "for all frames". If you would rather keep the
   branch, it costs a full `count` pass on every frame instead of a
   short-circuiting `any`.

3. **`fits_u32`'s wide path is only reached above ~8.3 Mpx** (8K falls back,
   4K does not). The equivalence test drives `adaptive_at::<u64>` directly
   rather than through a 33-megapixel fixture, so the *dispatch* at that size is
   untested; only the code it dispatches to is. A 33 Mpx fixture would be ~33 MB
   and seconds of test time, which seemed the wrong trade.

4. **`benchmarks/README.md` is now stale** for the detect and video sections —
   its "A featureless frame is the detector's worst case" analysis describes
   behaviour that no longer exists. I did not touch `benchmarks/`, per the
   brief. Someone should refresh it.

5. **What is left, if anyone wants more.** The prefix sum (223 µs) is the
   largest single item in the adaptive pass and is throughput-bound at ~1 add
   per element; a two-pass variant measured *worse*, so it stays as is. After
   that the pipeline is roughly binarize 440 µs / finder scan 190 µs / grouping
   + sample + decode 70 µs on the happy path. `Binary` is still one `bool` per
   pixel (921 600 bytes); a bitset would fit in L2 and speed the finder scan,
   but it touches every consumer of `Binary::get` and is a change of a different
   size.
