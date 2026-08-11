# Review Feedback — Step 11b: encode mask-selection performance
Date: 2026-08-11
Reviewer: Richard-OE
Ready for Builder: YES

**Verdict: Step 11b CLEAR** — 0 BLOCKER, 0 MAJOR, 4 MINOR.

Scope confirmed clean: only `crates/qr-encode/src/{lib.rs,matrix.rs,tests.rs}` modified,
`packed.rs` + `packed/tests.rs` new. `qr-core` untouched, `micro.rs` untouched,
no manifest change, no new deps, `#![forbid(unsafe_code)]` still in force at
`lib.rs:56` and no `unsafe` token anywhere in the new files. Concurrent qr-detect
work ignored.

---

## Must Fix

None.

---

## Should Fix

- **[crates/qr-encode/src/packed/tests.rs:175]** — Word-boundary size list is
  `[21, 25, 45, 64, 65, 77, 128, 129, 141, 177]`. It covers *on* and *one past*
  the boundary but not *one before*: **63 and 127 are absent**, so the
  "last word holds 63 real bits" shape is never differentially tested. I added
  both temporarily and re-ran — **both pass**, so this is a coverage gap, not a
  latent bug. Fix: make the array
  `[21, 25, 45, 63, 64, 65, 77, 127, 128, 129, 141, 177]`. One line.

- **[crates/qr-encode/src/packed.rs:546-557]** — When `forced.is_some()`,
  `grids = 3`, so after the four `split_at_mut` calls `transposed` is a
  **zero-length slice**. Safe today because `penalty_score` is never reached on
  the forced path, but the comment at 546-548 explains the grid *count* without
  saying that one of the four bindings is empty in that mode. A future edit that
  scores on the forced path fails with an opaque slice-index panic, not a clear
  error. Fix: one clause in the comment, or
  `debug_assert!(forced.is_none() || transposed.is_empty());` next to the split.

- **[benchmarks/README.md:155-198]** — Doc drift. That file is tracked and still
  states the pre-optimization figures as current: `42.08 µs` v1-M mask auto,
  `9.81 ms` v40-L mask auto, "**92.4 %** of `encode()`" at both ends, and the
  "24x what `decode()` does" comparison at line 198. All four are now wrong by
  4.6x-16x. Outside the letter of the brief (`crates/qr-encode/` only), but it is
  the document this step's baseline was quoted from, and leaving it will mislead
  the next person who reads it. Fix: regenerate, or add a dated "superseded by
  Step 11b" banner at the top.

## Nits

- **[crates/qr-encode/src/packed.rs:378-381]** — `History::push` gates the
  implicit-light-border addition on an `empty` flag; the reference
  (`matrix.rs:282`) gates it on `runs[0] == 0`. These agree only *incidentally*:
  after the first push `runs[0] >= line_len >= 1`, and every later run is at
  least one module long, so `runs[0]` can never return to 0. Your version is the
  more robust of the two — worth one comment line saying the divergence is
  deliberate, so nobody "fixes" it back to the reference form.

---

## Escalate to Architect

- **Bob's open question 1 (where `matrix::reference` lives).** Preference call,
  not a code call. My read: leave it. The module is `#[cfg(test)]`, so it costs
  the shipped crate nothing, and the header comment at `matrix.rs:66-70` already
  states why it exists and that it must not be deleted. Moving it is churn.

- **Bob's open question 3 (`u64` words on riscv32).** Accept. The selftest runs
  all four suites in the same wall time and the heap moved down; a `usize`-wide
  variant doubles the shift/mask cases for no host gain. Revisit only if the ESP
  path becomes latency-bound.

- **Bob's surprise 3 — resolved, with the culprit named.** See "Heap" below. It
  is a pre-existing regression from commit `7f75a0b`, not this step. Somebody
  should decide whether +48 KB of peak heap on the ESP target is acceptable, or
  whether the segmentation DP needs a bounded-scratch rework. That is an Arch
  call, and it is a separate step.

---

## What I verified, and how

### 1. Differential-test adequacy

- **Ran the suite.** All 8 `packed::tests::*` pass (27.2 s debug).
- **Smallest symbol (21):** already covered — size 21 is in the random-grid list,
  and `build_symbol_matches_reference_symbol` covers v1 (size 21) x 4 EC x
  (auto + 8 forced) against the reference's finished `BitMatrix`. Nothing to add.
- **Mask-exemption x word boundary:** genuinely covered, not by the random grids
  (those never involve masks) but by `build_symbol_matches_reference_symbol`.
  v12 is size 65, so x=64 sits alone in the second word *and is a function
  module* (top-right finder rows 0-7, plus the format bit at x=size-1); v40 is
  177 = 2x64+49, whose final partial word carries finder, alignment, version-info
  and format modules. Both are compared module-for-module across all 8 masks and
  4 EC levels. 1440 finished-symbol comparisons in total.
- **Word boundaries 63/127:** missing (see Should Fix). Added temporarily,
  re-ran, passed, reverted.
- **Extra stress I ran (temporary, reverted — tree is unchanged):** for sizes
  63/64/65/66/127/128/129/130/177, differential `penalty_score` vs
  `reference::penalty_score` over (a) all-light and all-dark grids, (b) **scaled
  1:1:3:1:1 finder patterns at every scale n=1..5 and every horizontal offset**,
  which sweeps a scaled finder across the x=64 seam one module at a time, and
  (c) solid runs of length 1..12 starting at every x in [56, 72). **All
  bit-identical.** This is the case I most expected to break; it does not.
- **Micro path untouched:** `micro.rs` not in the diff, 15 micro tests pass in
  both feature configs, `encode_micro` bench unmoved (5.08 µs median). The
  edge-count scorer (ISO 7.8.3.2) is not reachable from `packed`.

### 2. Transpose correctness — padding cannot leak

Three independent reasons a phantom run at a line end is impossible:

1. **Source padding is provably zero.** `build_candidate` (packed.rs:311) is
   `base ^ (pattern & !funcs)`. For x >= size, `base` was only ever written
   through `set_bit` at valid coordinates, and `mask_pattern_rows` (285-294) only
   sets x < size — so `pattern` is 0 there and the XOR term vanishes regardless
   of `!funcs`. Padding reads as light.
2. **Bottom padding is explicitly zero-filled**, packed.rs:347-352: block rows
   with `y >= size` get `0` before `transpose64`, and write-back is guarded by
   `if x < size`. So bits `y >= size` of a transposed column word are zero.
3. **A second guard in the walk**: `line_penalties` masks
   `changes &= low_mask(size - base)` on the final partial word (packed.rs:433-435),
   so even a nonzero pad bit could not emit a transition at x >= size.

Also checked for staleness between the eight candidates: the `(bi, bj)` loops
write **every** element of `transposed` on every call, and `mask_pattern_rows`
starts with `table.fill(0)`, and `build_candidate` fully overwrites `candidate`.
No buffer carries state across mask iterations.

Empirically confirmed at 177 = 2x64+49 and at 63/65/127/129 above, plus
`transpose_matches_naive_for_every_version` (all 40 versions, every module) and
`transpose64_matches_naive` (64 blocks x 4096 bits).

### 3. Run-walk equivalence

Line-by-line against `matrix::reference::line_run_penalty` (237-257) and
`finder_penalty` (305-332):

- **Cross-word runs count once.** `carry = v >> 63` (packed.rs:430) feeds bit 0
  of the next word's transition mask, so `changes` bit 0 is
  `v_0 ^ prev_63` — no transition where the colour is continuous. Verified
  directly by the straddling-run sweep in (1c).
- **Leading run.** At `wi == 0`, `carry = line[0] & 1` makes `changes` bit 0
  identically zero, suppressing the phantom transition at x=0. The
  `if dark { hist.push(0); n3 += count()*40 }` prologue (packed.rs:420-425)
  reproduces exactly what the reference does at `i == 0` when `at(0)` is dark
  (matrix.rs:312-321), zero-length light run and all.
- **Trailing run.** `if dark { push(len); push(size) } else { push(len + size) }`
  then one `count()` (packed.rs:452-463) is a transcription of matrix.rs:325-330.
- **N3 scaled semantics preserved.** `History::count` (packed.rs:393-402) is
  `FinderHistory::count` (matrix.rs:294-301) with identical predicates: core
  `n, n, 3n, n, n` and the **4n light border tested on both sides**
  (`lead || trail`), for any n. Bob correctly rejected the brief's 11-bit window,
  which cannot express the scaled form. No overflow risk: n <= 177, so n*4 <= 708.
- **N1 arithmetic.** `len - 2` == reference's `3 + (run_len - 5)`.
- **N2.** `!(t^t1) & !(t^b) & !(t1^b1)` is all-four-equal; `limit_mask(w, size-1)`
  correctly restricts to top-left x < size-1; cross-word neighbour via
  `(t >> 1) | (tn << 63)` with `tn = 0` on the last word (padding is light).
- **N4.** Identical expression to matrix.rs:229-231, and `count_ones()` over the
  packed rows cannot overcount because padding is zero.

### 4. Tie-break — preserved

`if score < best_score` over masks ascending 0..7 (packed.rs:575) keeps the
incumbent on equality; the reference's
`Some((bs, _, _)) if *bs <= score => {}` (matrix.rs:352) does the same. Identical
by inspection. `annex_i_selected_mask_is_two` passes (ISO Annex I still selects
mask 2), and `build_symbol_matches_reference_symbol` compares the *finished
matrix* for 40 versions x 4 EC x 9 mask configs = 1440 cases, which would surface
any tie resolved differently. I did not construct a synthetic tie; forcing equal
scores requires crafting a payload and the inspection is conclusive.

### 5. Heap discipline

Reproduced on this machine, `esp-selftest` release under `qemu-system-riscv32`:

```
[1/4] qr roundtrip, v1-v10 sampled....... PASS (29 symbols)
[2/4] micro qr, all 8 symbols............ PASS (8 symbols)
[3/4] reed-solomon recovery.............. PASS (3 cases)
[4/4] iso 7.5.1 misdecode protection..... PASS (8 cases)
heap peak 112840 of 196608 bytes
ALL PASS      (exit 0, 4/4 suites)
```

Matches Bob's 112 840 B exactly. The ordering claim in the `build_symbol` comment
holds structurally: `BitMatrix::new` (551) precedes the single `vec!` (552),
`vec![0u64; n]` allocates once with no growth realloc, and the explicit
`drop(scratch)` (587) frees before `out` is returned — so scratch is the most
recent live block at drop time, which is what the bump allocator needs. Accepted.

### 6. Suites — all reproduced

| check | result |
|---|---|
| `cargo test -p qr-encode` | 35 + 15 + 11 + 12 = **73** pass, 0 fail |
| `cargo test -p qr-encode --no-default-features` | 35 + 11 + 12 = **58** pass, 0 fail |
| `cargo test -p tests-integration` | 11+5+11+3+12+15+3+9+15 = **84** pass, 0 fail |
| `cargo clippy -p qr-encode --all-targets` | clean (no warnings) |
| `cargo clippy -p qr-encode --no-default-features --all-targets` | clean |
| `cargo fmt -p qr-encode -- --check` | clean |
| `cargo check --target thumbv7em-none-eabihf` (both feature configs) | ok |
| `cargo check --target riscv32imc-unknown-none-elf` | ok |
| `cargo check --target wasm32-unknown-unknown` | ok |

### Bench — direction reproduced

`cargo bench -p benchmarks --bench encode`, this machine, medians:

| bench | Bob claims | I measured | baseline |
|---|---:|---:|---:|
| v1-M, 8 digits, mask auto | 9.08 µs | **9.08 µs** | 42.00 µs |
| v40-L, 2953 B, mask auto | 615.83 µs | **604.42 µs** | 9.86 ms |
| v40-L, 2953 B, mask = Some(0) | 219.92 µs | **219.25 µs** | 757.21 µs |
| encode v1-M end to end | 9.46 µs | **9.17 µs** | 42.12 µs |
| encode v40-L end to end | 597.46 µs | **633.96 µs** | 9.91 ms |
| encode_micro M4-Q | 5.00 µs | **5.08 µs** | 5.08 µs (untouched) |

**16.3x at v40 auto-mask, 4.6x at v1.** Claim stands. Micro unchanged, as stated.

### 7. Surprise 3 — attribution confirmed, and pinned to a commit

Bob's direction is right, and I found the culprit. Four measurements, same
machine, same QEMU, `esp-selftest` unchanged since it was introduced
(`git log -- esp-selftest/src/main.rs` shows exactly one commit, `4f1d5b2`):

| commit | heap peak |
|---|---:|
| `4f1d5b2` esp-selftest introduced | **75 348 B** — reproduces the documented figure |
| `7f75a0b` optimal mixed-mode segmentation, Kanji, ECI | **123 640 B** |
| `c0c896e` current clean HEAD | 123 640 B |
| this change | **112 840 B** |

The **+48 292 B** regression landed in `7f75a0b` (the segmentation DP), five
commits before this step. `handoff/REVIEW-REQUEST-esp-selftest.md`'s 75 348 B is
correct *for its own commit* and simply was never re-measured. Nothing to do with
Step 11b, which moves the number **down** 10 800 B. Handing this to Arch as a
separate item.

---

## Cleared

The word-packed placement, masking and ISO 7.8.3 scoring path in
`crates/qr-encode/src/packed.rs` was reviewed against the retained
`matrix::reference` implementation and re-verified independently on the four
places this kind of rework usually breaks — u64-boundary run stitching, transpose
pad bits, N3's scaled-pattern border test, and the mask tie-break. It is
bit-identical on everything I could throw at it, including a scaled-finder sweep
across the word seam that the committed tests do not perform. All 12 definition-of-done
checks reproduce, the 84-test integration suite is green, the ESP selftest is ALL
PASS at 112 840 B, and the v1/v40 benchmark direction is confirmed. Four MINOR
items, none blocking.

**Step 11b is clear.**

*(Note: my temporary test additions were reverted; `crates/qr-encode` is exactly
as Bob left it. I touched only this feedback file.)*
