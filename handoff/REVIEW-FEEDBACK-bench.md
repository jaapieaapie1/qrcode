# Review Feedback — Step 9: benchmarks
Date: 2026-08-11
Reviewer: Richard-BM
Ready for Builder: NO

**Verdict: Step 9 BLOCKED** — 0 BLOCKER, 3 MAJOR, 7 MINOR.

Nothing here is a redesign. The harness is sound, the guards are green, and the
numbers reproduce. What blocks is three written claims that are not true, in
files that will be read by whoever picks this up next. I do not sign off on a
manifest comment that says the opposite of what cargo does.

## How I verified

Everything below was measured in an isolated `git worktree` at clean HEAD
(`ec4085e`) with `benchmarks/` and the root `Cargo.toml` copied in. The sibling's
in-flight `crates/qr-detect` edits were **not** present in that tree, so no
result here is contaminated by them — and equally, I have not tested this against
those edits. Same machine class as Bob's baseline (Apple M4, macOS).

---

## Must Fix

### MAJOR 1 — `benchmarks/Cargo.toml:16-31`: the stated rationale for the convention break is false

The manifest comment and REVIEW-REQUEST §1 both claim that depending on the
siblings at their defaults "would re-enable `micro` workspace-wide through
feature unification and quietly hollow out CI's `--no-default-features` lane."

That lane is *already* hollow, at HEAD, without `benchmarks`. Proof:

```
# in the worktree, root Cargo.toml at HEAD (no "benchmarks" member)
$ cargo tree -e features --workspace --no-default-features | grep 'feature "micro"'
qr-core feature "micro"
qr-decode feature "micro"
qr-detect feature "micro"
qr-encode feature "micro"
```

And with `benchmarks` added as a member, the resolved feature set is **byte-for-byte
identical**:

```
$ diff <(features at HEAD) <(features with benchmarks)
IDENTICAL FEATURE SET
```

The enabling edge is `tests-integration` → `qr-video` (`workspace = true`) →
`qr-detect` (`workspace = true`, defaults = `["decode","micro"]`) →
`qr-core/micro`. `cargo tree -e features -i qr-core --workspace --no-default-features`
shows that chain explicitly.

`benchmarks` cannot close it and does not: it takes `qr-video = { workspace = true }`
and `qr-render = { workspace = true }` on line 32-33, which re-enables the exact
same chain through this crate too. So the four `default-features = false` lines
buy **nothing**. They are not harmful, but the comment justifying them is wrong
and the convention break is unearned.

Fix — pick one and say so in the manifest:
- Revert lines 26-31 to `workspace = true` and delete the comment; or
- Keep them and rewrite the comment to say what is actually true: the lane is
  already unified `micro`-on via `qr-video`/`tests-integration`, these lines
  are defensive only, and closing the hole properly needs the workspace-level
  polarity flip described in Open Question 2.

Also correct REVIEW-REQUEST §1's "I verified both lanes still behave" — both
lanes *pass*, which is not the same as the lane doing its job.

### MAJOR 2 — `benchmarks/README.md:149-153` (and REVIEW-REQUEST "Surprises"): the flat-frame root cause is guessed wrong

The observation is right and I reproduced it (see below). The mechanism written
down is not. Bob's guess: "whatever threshold it settles on splits sensor-uniform
pixels arbitrarily, and the finder scanner then walks a lot of spurious runs."

The actual mechanism is in `crates/qr-detect/src/binarize.rs:90-96`:

```rust
let dark_count = bin.dark.iter().filter(|&&d| d).count();
if dark_count == 0 || dark_count == total {
    return Some(otsu(luma, width, height));
}
```

On a uniform frame the Bradley pass produces `dark_count == 0`, so the entire
adaptive result — integral image plus 921 600 window evaluations — is **discarded**
and a full Otsu pass (histogram + threshold application + a second `Vec<bool>`)
runs over the whole frame on top of it. Two binarizations instead of one. Then
`detect_luma` finds nothing and retries, doing it all again.

The arithmetic matches exactly: flat single-pass 3.40 ms vs cluttered single-pass
2.00 ms; the 1.40 ms delta is one Otsu pass over 921 600 px. It is emphatically
*not* extra finder scanning — the resulting binary image is all-white, so the
finder scan on a flat frame is the cheapest it ever gets.

This matters because Bob is logging it as a Known Gap for someone else to chase,
and the note as written points them at the finder scanner. Correct the mechanism
in README:149-153 and in the Known Gap entry.

### MAJOR 3 — `benchmarks/README.md:139-143`: the headline encoder finding is asserted, never measured — and `Options::mask` makes measuring it trivial

"Encoding is mask selection … Anyone optimising the encoder should start and end
there" is the single most actionable sentence in the document. It is backed only
by inference (encode 42.7 µs, RS inside it 274 ns, decode 1.7 µs). No bench row
measures mask selection. A regression in `matrix.rs:269-285` would show up only
as a blended `encode()` number with no way to attribute it.

`qr_encode::Options` has `pub mask: Option<u8>` (`crates/qr-encode/src/lib.rs:123`).
Bob already used exactly this technique for the DP (`mode: Some(Mode::Byte)`), so
the pattern is established in the same file. I wrote the two-line bench and ran it:

| bench | median |
|---|---:|
| v1-M 8 digits: mask auto (8 candidates) | 42.00 µs |
| v1-M 8 digits: `mask = Some(0)` forced | **3.12 µs** |
| v40-L 2953B: mask auto (8 candidates) | 9.81 ms |
| v40-L 2953B: `mask = Some(0)` forced | **703.25 µs** |

Mask selection is **92.6 %** of `encode()` at v1 and **92.8 %** at v40. The claim
is true and if anything understated. Add these two rows to `encode.rs` under a
`qr-encode — mask selection cost (auto vs pinned)` group and let the table carry
the finding instead of the prose.

---

## Should Fix

### MINOR 4 — `benchmarks/README.md:114-133`: the baseline table is not verbatim harness output

The harness prints `438.08 M/s` and `1.13 k/s`; the README shows `433.35 Mpx/s`
and `1117 fps`. The harness truncates names at `NAME_W = 48` — real output is
`rs_correct v1-M (26cw) 5 err (= floor(ec/2)) x2~`; README:72 shows an
untruncated, hand-shortened `rs_correct v1-M (26cw) 5 err x256`. The numbers are
right, but a "baseline" that cannot be diffed against a fresh `cargo bench` run
is worth much less than one that can. Either paste the raw output or state that
the table is transcribed.

### MINOR 5 — `benchmarks/src/lib.rs:47` `NAME_W = 48` truncates the `xB` tag off the rows that need it

`src/lib.rs:36-39` and README:36-39 both promise "the name carries `xB`". The two
`rs_correct` rows are exactly the ones where it is cut:
`... 5 err (= floor(ec/2)) x2~` and `... 15 err (= floor(ec/2))~`. The reader
cannot tell what the `ns/op` column is per. Shorten the labels (drop
`(= floor(ec/2))`, it is already in the doc comment) or widen the column.

### MINOR 6 — timer granularity: two rows sit close enough to the clock to matter

Measured on this machine: `Instant` granularity is **41.67 ns**, and an empty
timed span reads a median of **41 ns** (200 k samples). So every sample carries a
fixed ~41 ns and quantises to ~42 ns steps.

The ns/op claims are all correctly batched and clock-clean — GF at 65 536 ops per
timed iteration (36.79 µs ≈ 883 ticks), BCH at 8 192 ops (257 µs), RS at 32-256
ops. `gf::mul = 0.56 ns/op` is **not** a per-op timing artifact. That part of the
design is right.

But two unbatched rows are close in:
- `decode_micro M4-Q` 1.33 µs = 32 ticks → ~3.1 % of the reading is clock;
  min/median/p95 (1.25/1.33/1.42 µs) are 3-4 ticks apart, so that spread is
  mostly quantisation, not jitter.
- `decode v1-M` 1.71 µs = 41 ticks → ~2.4 %.

Either batch those two like the core benches, or add one line to the README
saying rows under ~2 µs carry a ~3 % clock overhead and should not be compared at
finer resolution.

### MINOR 7 — p95 on the sub-110-iteration rows rests on ≤ 8 tail samples

`encode v40-L` runs 99-103 iterations, `detect_luma` flat runs 144-146. Nearest-rank
p95 over 100 samples is the 5th-from-top value. The README disclaims statistics
generally, but these specific rows should be marked, or their `iters_hint` raised.

### MINOR 8 — `benchmarks/benches/detect.rs:46-67`: two fixtures are unasserted

Correctness assertions being *outside* the timed loop is the right call and is
done correctly everywhere. But the coverage has holes:
- the `flat` frame (`detect.rs:44`) is never asserted symbol-free, yet two benches
  run on it — including the 6.92 ms headline;
- no `detect_luma_single_pass` result is asserted anywhere. If single-pass ever
  early-returned, three benches would silently measure a stub;
- `rotated` asserts detection count but not payload equality, unlike `happy`.

Three more `assert_eq!`s before `group()`.

### MINOR 9 — `benchmarks/README.md:145-147`: "the DP is free" leans on the weaker of the two rows

At v40 the payload is `pseudo_bytes` — every byte ≥ 0x80 (`common/mod.rs:177`), so
the DP has no alternative segmentation to weigh at all. That row cannot show a DP
cost by construction. The v10 URL row is the load-bearing one, and there my run
gave 646.33 µs (DP on) vs 596.29 µs (no DP) on median — 8 % — though 568.04 vs
567.04 µs on **min**, which is where the "within noise" reading holds up. State
the conclusion off `min`, not `median`, or say both.

### MINOR 10 — work inside the timed loop

`core.rs:105` `scratch.copy_from_slice(src)` and the `.expect()` calls in
`decode.rs:73/81` and `core.rs:109` are inside the timed region. Bob documents the
`copy_from_slice` and it is a rounding error at 26-148 bytes. Noting for
completeness only; no change needed.

---

## Escalate to Architect

**The `--no-default-features` CI test lane provides zero micro-off coverage, and
has not for some time.** Proven above: `cargo test --workspace --no-default-features`
compiles `qr-core`, `qr-encode`, `qr-decode` and `qr-detect` with `micro` **on**,
because `tests-integration` and `qr-video` take their siblings at defaults. Every
`#[cfg(not(feature = "micro"))]` path in the workspace is therefore untested by
that lane.

The only job that genuinely exercises micro-off is `cross-targets`:
`cargo check -p qr-core -p qr-encode -p qr-decode --no-default-features --target
thumbv7em-none-eabihf` — a `check`, on three crates, with no tests run.

This predates Step 9 and is not Bob's to fix unasked. It is a product/CI decision:
either flip the workspace-dependency polarity (`default-features = false` in
`[workspace.dependencies]` plus explicit opt-in everywhere, per Bob's Open
Question 2), or add a `cargo test -p qr-core -p qr-encode -p qr-decode -p qr-detect
--no-default-features` job that resolves features over only those packages. Your
call.

Also for you: **flat-frame detector cost** (Bob's Open Question 1). With the real
mechanism now known (MAJOR 2), the fix is small and local —
`binarize.rs:90-96` currently pays for a full adaptive pass before throwing it
away. Recommend its own step against `qr-detect`, not a Known Gap.

---

## Reproduced numbers

Isolated worktree, clean HEAD + `benchmarks/`, Apple M4, `bench` profile.

**Spot-check (a) — blank frame vs cluttered frame.** Confirmed, direction and
magnitude both.

| bench | Bob | mine |
|---|---:|---:|
| happy path | 2.13 ms | 2.10 ms |
| miss: cluttered (primary + retry) | 4.22 ms | 4.05 ms |
| miss: **flat** (primary + retry) | 6.92 ms | **6.82 ms** |
| miss: cluttered, single pass | 2.11 ms | 2.00 ms |
| miss: **flat**, single pass | 3.45 ms | **3.40 ms** |

Flat is 3.2x the happy path and 1.68x the cluttered frame; single-pass flat is
1.70x single-pass cluttered. The retry is a clean 2.0x doubling on both. The
finding stands; the explanation does not (MAJOR 2).

**Spot-check (b) — encode dominated by mask selection.** Confirmed, and
quantified for the first time.

| | median |
|---|---:|
| `encode()` v1-M, auto mask | 42.00 µs |
| `encode()` v1-M, `mask = Some(0)` | 3.12 µs → **92.6 % is mask selection** |
| `encode()` v40-L, auto mask | 9.81 ms |
| `encode()` v40-L, `mask = Some(0)` | 703.25 µs → **92.8 % is mask selection** |

Supporting rows from my run: `rs_encode v1-M x256` = 68.67 µs → 268 ns per block,
0.6 % of a v1 encode, matching Bob's 274 ns. `encode v1-M` 42.00 µs vs
`decode v1-M` 1.71 µs = 24.6x, matching his 25x.

---

## Guards (my run, clean worktree)

| Check | Result |
|---|---|
| `cargo test --workspace` | green — 26 test binaries, 0 failures |
| `cargo test --workspace --no-default-features` | green — 0 failures |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo clippy --workspace --all-targets --no-default-features -- -D warnings` | clean |
| `cargo fmt --all --check` | clean |
| `cargo bench -p benchmarks` (all 5 targets, full mode) | all fixture assertions pass |
| `BENCH_QUICK=1 cargo bench -p benchmarks` | all 5 targets run, **0.45 s** wall |

No `qr-detect` failures appeared, because the sibling's in-flight `micro.rs`
edits and `zz_scratch.rs` were not in the worktree. Nothing in this review is
attributable to them, and nothing in this review clears them.

---

## Cleared

Reviewed and passing, no action:

- **`black_box` placement.** Input is black-boxed at every call site
  (`&happy`, `&payload`, `&mut scratch`, `Frame::new(...)`, GF operands) and the
  output by `run()` at `src/lib.rs:125`. The GF benches additionally carry an XOR
  accumulator chain returned from the closure, so neither the loop body nor the
  loop itself can be eliminated. No hoisting risk: every non-GF closure
  allocates, and `Instant::now` is an opaque call the optimiser cannot move pure
  work across in either direction. This is done right.
- **Assertions outside the timed loop.** Detector and encoder fixtures are
  validated once before `group()`; video validates via `Stats` deltas after the
  bench returns. That is the correct trade — neither pollutes timing, and neither
  bench can silently fall onto a different code path.
- **Fixture determinism.** No files, no network, no wall clock. `pseudo_bytes` is
  a seeded xorshift; video timestamps come from a counter, not `Instant`. Same
  seeds produce the same fixtures across targets.
- **Video ROI bench is genuinely steady-state.** One priming frame plus an
  `active_roi().is_some()` assertion establishes the track before `bench_units`;
  `full_scan_interval: 0` disables the periodic refresh; `before` is captured
  *before* the call, so the post-hoc `roi_misses` / `full_scans` / `roi_hits`
  deltas cover the calibration and warmup iterations as well as the timed ones.
  The counterpart bench asserts `full_scans - before == frames - before`. This is
  the most carefully built bench in the set.
- **Calibration loop.** 4x batch growth to a 20 ms floor; `iters_hint` correctly
  used as a ceiling in both the loop exit (`n >= iters_hint`) and the final
  `clamp(1, iters_hint)`; a zero per-iteration estimate is handled by
  `checked_div` falling back to the hint rather than dividing by zero. Warmup of
  `(iters/10).max(3)` on top of ~1.33x the calibration batch is adequate for
  every bench here.
- **`corrupt` / `worst_correctable`** (`decode.rs:20-49`). `stride = len/flips`
  with `flips > 0` asserted and `flips <= len` asserted; max index
  `(flips-1) * floor(len/flips) < len`, so no out-of-range and no repeated toggle
  of the same module. `errors_corrected > 0` is asserted on the result. The
  fixture is what it claims.
- **CI `bench-smoke`** (`.github/workflows/ci.yml:45-60`). `BENCH_QUICK=1` is
  wired, all five targets execute (verified), nothing is thresholded on timing,
  and the comment explains why. 0.45 s of runtime. Correctly scoped as a
  build-and-run gate.
- **Scope.** No drift. Root `Cargo.toml` gains one line; the only other file
  touched outside `benchmarks/` is the CI workflow, both in scope.
