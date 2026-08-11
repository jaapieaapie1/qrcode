# Review Request — Step 11b: encode mask-selection performance

**Author:** Bob-OE
**Scope:** `crates/qr-encode/` only. `qr-core` read, not touched.
**Ready for Review: YES**

---

## What the baseline said

`benchmarks/README.md`: mask selection is 92.4 % of `encode()` at both ends of the
version range — v1-M 42.08 µs vs 3.21 µs pinned, v40-L 9.81 ms vs 744.50 µs pinned.
Eight candidate matrices, each fully re-placed and penalty-scored.

Profiling the shape of the old code, the 92 % splits three ways per candidate:

1. `build_unmasked()` ran **eight times**, and `place_data` calls
   `qr_core::is_function_module` once per module — a predicate that walks the
   whole alignment-coordinate grid (7x7 = 49 centres at v40). 31 329 modules x 49
   checks, eight times over.
2. `apply_mask` did the same predicate walk again, plus `mask_bit` and a
   read-modify-write `toggle` per module.
3. `penalty_score` walked every module ~5 more times through closure-indirected
   `BitMatrix::get` (bounds test + `/8` + `%8` per access).

So the fix is not only "score faster" — it is "stop asking a 49-branch predicate
31 329 times per candidate".

---

## Approach

New module `crates/qr-encode/src/packed.rs`. The symbol lives as packed rows:
`ceil(size / 64)` `u64` words per row, bit `x` of row `y` at word `x / 64`,
bit `x % 64` from the LSB end, bits at `x >= size` pinned to zero so padding
reads as light modules and no scan needs a bounds test.

**Placement / masking**

* The function-module map is built **once**, structurally, as region fills
  (~1 600 set modules at v40 instead of 31 329 predicate calls). `place_data`
  then skips function modules with a bit test.
* The eight ISO masks are periodic in `y` with periods 1, 2, 3, 4 and 6, all of
  which divide 12. Twelve packed rows therefore describe a whole mask at any
  symbol size. A candidate is
  `data_row XOR (mask_row AND NOT function_row)` — three word ops per word, no
  per-module predicate anywhere.
* Version info and the fixed dark module are mask-independent, so they go into
  the shared base grid once instead of once per candidate.

**Scoring** (`penalty_score`)

* **N4** — `count_ones()` over the rows, then the unchanged ISO integer formula.
* **N2** — per row pair, `!(t ^ t>>1) & !(t ^ b) & !(t>>1 ^ b>>1)` masked to
  `x < size - 1`; `count_ones() * 3`.
* **N1 + N3** — *one* run walk per line, sharing the runs. Transitions come out
  of the packed row as `v ^ (v << 1)` and are walked with `trailing_zeros`, so
  the cost is one step per **run** (~size/2) rather than per module, and the
  cross-word carry keeps it exact. N3 keeps the reference's seven-run history
  verbatim, in registers rather than a `copy_within`.
* **Column direction** — a 64x64 blocked bit transpose (recursive halving,
  `transpose64`), then the *same* line walk on the transposed rows. Measured
  against the alternative of bit-sliced column walks: the transpose wins because
  it lets one exact, well-tested line routine serve both directions, and at v40
  it is ~10 k word ops against ~640 k for the run walks it feeds.

**What did *not* work / was rejected**

The brief suggested N3 via 11-bit shifted-window comparison. That is **not**
ISO-conformant here: `finder_penalty` implements the *scaled* 1:1:3:1:1 pattern
(any `n`, with an `4n` light border), which an 11-bit window cannot express.
Scores had to be bit-identical, so N3 stayed run-based. Likewise N1 could have
been a pure bitwise `E & E>>1 & E>>2 & E>>3` popcount, but since the run walk for
N3 is unavoidable, folding N1 into it is strictly cheaper than a second pass.

**Tie-break** — unchanged: `score < best_score` keeps the incumbent on a tie, so
the lowest mask id still wins.

---

## Before / after

Apple M4, `bench` profile, `cargo bench -p benchmarks --bench encode`, run in an
isolated `git worktree` at clean HEAD (`9e3b445`) so the concurrent `qr-detect`
work could not perturb it. Medians.

### Headline

| bench | before | after | change |
|---|---:|---:|---:|
| `encode` v1-M, 8 digits, mask auto | 42.00 µs | **9.08 µs** | **4.6x** |
| `encode` v40-L, 2953 B, mask auto | 9.86 ms | **615.83 µs** | **16.0x** |

Targets were v1-M under 10 µs and v40-L under 2 ms. Both met.

### Full encode suite

| bench | before | after | change |
|---|---:|---:|---:|
| encode v1-M, 8 numeric digits | 42.12 µs | 9.46 µs | 4.5x |
| encode v10-M, 198 B URL-ish (auto version) | 585.46 µs | 50.38 µs | 11.6x |
| encode v40-L, 2953 B byte (auto version) | 9.91 ms | 597.46 µs | 16.6x |
| v10-M URL-ish, version pinned, DP on | 573.00 µs | 48.04 µs | 11.9x |
| v10-M URL-ish, version pinned, mode=Byte | 570.83 µs | 46.21 µs | 12.4x |
| v40-L byte, version pinned, DP on | 9.77 ms | 582.00 µs | 16.8x |
| v40-L byte, version pinned, mode=Byte | 9.74 ms | 562.29 µs | 17.3x |
| v1-M, 8 digits: mask auto (8 candidates) | 42.00 µs | 9.08 µs | 4.6x |
| v1-M, 8 digits: mask = Some(0) pinned | 3.25 µs | 1.71 µs | 1.9x |
| v40-L, 2953 B: mask auto (8 candidates) | 9.86 ms | 615.83 µs | 16.0x |
| v40-L, 2953 B: mask = Some(0) pinned | 757.21 µs | 219.92 µs | 3.4x |
| encode_micro M4-Q, 9 alphanumeric | 5.08 µs | 5.00 µs | — (untouched) |

The pinned rows moved too because `place_data` no longer calls
`is_function_module` per module; that is the same fix, not a separate one.

### Mask selection's share of `encode()`

| | before | after |
|---|---:|---:|
| v1-M, 8 digits | 92.4 % (38.75 µs) | 81.2 % (7.37 µs) — **5.3x cheaper** |
| v40-L, 2953 B | 92.4 % (9.10 ms) | 64.3 % (395.9 µs) — **23x cheaper** |

Selection is still the largest single line item, but at v40 it is now smaller
than everything else in `encode()` combined.

### ESP32-class heap

`esp-selftest` under QEMU, measured on this machine:

| | heap peak |
|---|---:|
| clean HEAD (`9e3b445`) | 123 640 / 196 608 B |
| this change | **112 840 / 196 608 B** |

**Note for Arch:** `handoff/REVIEW-REQUEST-esp-selftest.md` records 75 348 B. That
figure no longer reproduces at clean HEAD on this machine (123 640 B) — something
between that session and now raised it, and it is not this change. Worth a
separate look; I did not chase it, it is outside this step.

The number moved *down* because all working storage is now a **single** `Vec`,
allocated after the output matrix and dropped before returning. That ordering is
deliberate: the selftest's bump allocator only reclaims the most recent live
block, so scratch has to be the top block at drop time. My first cut allocated
six separate buffers and pushed the peak to 177 580 B — worth knowing if anyone
adds a buffer here later. There is a comment on `build_symbol` saying so.

---

## Files changed

| file | lines | what |
|---|---|---|
| `crates/qr-encode/src/packed.rs` | 1–592 (new) | The whole packed path: grid helpers (37–90), function map (92–161), function patterns (163–206), zigzag placement (208–239), version/format info (241–279), mask pattern table (281–295), candidate build (297–314), scorer (316–518), `build_symbol` entry point (520–589). |
| `crates/qr-encode/src/packed/tests.rs` | 1–272 (new) | Old-vs-new equivalence tests (below). |
| `crates/qr-encode/src/matrix.rs` | 1–419 | `interleave` unchanged, moved to the top (16–68). Everything else — placement, masking, format/version info, `penalty_score`, `penalty_components`, `line_run_penalty`, `finder_penalty`, `FinderHistory`, `choose_mask` — moved **verbatim** into `#[cfg(test)] pub(crate) mod reference` (71–419). Not deleted; it is the executable spec the new code is checked against. |
| `crates/qr-encode/src/lib.rs` | 63–66, 296–299 | Declares `mod packed`; `encode()`'s placement/mask tail replaced by one `packed::build_symbol(version, ec, &interleaved, options.mask)` call. Public API, `Options`, errors, segmentation, capacity logic all untouched. |
| `crates/qr-encode/src/tests.rs` | 107, 161, 213–234 | Three call sites re-pointed at `matrix::reference::*`. No test logic changed. |

---

## How "bit-identical" is established

`packed/tests.rs`, all against `matrix::reference`, seeded xorshift64* (no deps):

| test | coverage |
|---|---|
| `transpose64_matches_naive` | 64 random 64x64 blocks, every one of 4 096 bits. |
| `transpose_matches_naive_for_every_version` | all 40 versions, every module. |
| `mask_patterns_repeat_every_twelve_rows` | all 8 masks, 200x200 coordinates — proves the 12-row period assumption. |
| `function_map_matches_is_function_module` | all 40 versions, every module, against `qr_core::is_function_module`. |
| `packed_placement_matches_reference` | all 40 versions, every module, function patterns + zigzag data + version info. |
| `packed_score_matches_reference_on_random_grids` | 10 sizes (incl. 64/65 and 128/129 word boundaries) x 24 rounds x 4 densities — deliberately far outside real-symbol shapes: long runs, dense 2x2 fields, scaled finder patterns. |
| `packed_score_components_match_reference_on_symbols` | all 40 versions x all 8 masks, real symbols. |
| `build_symbol_matches_reference_symbol` | all 40 versions x 4 EC levels x (auto + 8 forced masks) — compares the finished `BitMatrix` for equality, so it covers the selection *and* the tie-break. |

Plus every pre-existing test unchanged, including `annex_i_selected_mask_is_two`
(ISO Annex I still selects mask 2) and `forced_mask_is_honoured_and_verifiable`.

---

## Definition of done

| check | result |
|---|---|
| `cargo test -p qr-encode` | 35 + 15 + 11 + 12 pass, 0 fail |
| `cargo test -p qr-encode --no-default-features` | 35 + 11 + 12 pass, 0 fail |
| `cargo test -p tests-integration` | **84** pass, 0 fail |
| `cargo clippy -p qr-encode --all-targets` | clean |
| `cargo clippy -p qr-encode --no-default-features --all-targets` | clean |
| `cargo fmt -p qr-encode` | clean |
| `cargo check -p qr-encode --target thumbv7em-none-eabihf` | ok (also `--no-default-features`) |
| `cargo check -p qr-encode --target riscv32imc-unknown-none-elf` | ok |
| `cargo check -p qr-encode --target wasm32-unknown-unknown` | ok |
| `esp-selftest` build + QEMU riscv32 | `ALL PASS`, exit 0, 4/4 suites, heap 112 840 B |
| new deps | none |
| `unsafe` | none (`#![forbid(unsafe_code)]` still in force) |
| `qr-core` edits | none |

---

## Open questions / things worth a second pair of eyes

1. **`matrix.rs` is now almost entirely `#[cfg(test)]`.** That is what the brief
   asked for (keep the reference, do not delete it) and it is genuinely load-
   bearing for the equivalence tests. But it means `interleave` is the only
   thing the shipped crate uses out of that file. If you would rather the
   reference lived under `packed/reference.rs` next to the tests that consume
   it, say so and I will move it — no behaviour change either way.

2. **The 12-row mask period** is asserted by a test over a 200x200 coordinate
   range rather than proved from the algebra in the doc comment. The algebra is
   simple (periods 2, 2, 1, 3, 4, 6, 6, 6 all divide 12) but the test is what
   actually guards it if `qr_core::mask_bit` were ever edited. Flag if you want
   the reasoning spelled out per mask in the comment.

3. **`u64` words on a 32-bit core.** riscv32 splits every word op in two, so the
   ESP target gets a smaller win than the host. It still beats the per-module
   path comfortably (the selftest runs the same suites in the same wall time),
   and going `usize`-wide would double the shift/mask cases for no host gain. I
   chose not to. Happy to revisit if the ESP32 path ever becomes latency-bound.

4. **The stale 75 348 B heap figure** in `REVIEW-REQUEST-esp-selftest.md`
   (see above). Not mine, but somebody should find out what raised it.

5. `micro.rs` is untouched. `encode_micro` is 5 µs and its evaluation score is a
   different rule (ISO 7.8.3.2, edge darkness, 4 masks, 17x17 max) — out of
   scope for this step and not worth the packed machinery.
