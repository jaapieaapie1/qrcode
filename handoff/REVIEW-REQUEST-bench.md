# Review Request — Step 9: benchmarks

**Ready for Review: YES** (round 2)
Builder: Bob-BM · 2026-08-11

---

## Round 2 — response to `handoff/REVIEW-FEEDBACK-bench.md`

Richard was right on all three MAJORs. Two of them were me writing down a
mechanism I had not verified, which is the worst kind of thing to leave in a
file someone else will act on. All three fixed, plus five of the seven MINORs.

### MAJOR 1 — false rationale for the dev-dep convention break — FIXED

He proved with `cargo tree -e features --workspace --no-default-features` that
`micro` is already on for qr-core/qr-encode/qr-decode/qr-detect at HEAD, via
`tests-integration` → `qr-video` → `qr-detect`, and that `benchmarks` cannot
close that hole — my own `qr-video = { workspace = true }` line re-opens it
regardless. The four `default-features = false` lines bought nothing and the
comment justifying them said the opposite of what cargo does.

Took his first option: all six sibling deps are back to `workspace = true` and
the comment is gone. `benchmarks/Cargo.toml:16-22`. The crate keeps its own
`micro` feature (it gates the micro bench blocks), and its comment now states
plainly that the forwarding is a no-op in-workspace.

My round-1 line "I verified both lanes still behave" was true and irrelevant —
both lanes pass, which is not the same as the lane doing its job. Withdrawn.

### MAJOR 2 — flat-frame root cause guessed wrong — FIXED

I wrote "the finder scanner then walks a lot of spurious runs." Wrong, and it
would have sent whoever picks up the Known Gap to the wrong file.

The real mechanism is `crates/qr-detect/src/binarize.rs:90-96`: on a uniform
frame the Bradley pass yields `dark_count == 0`, so the completed adaptive
result — integral image plus 921 600 window evaluations — is discarded and a
full Otsu pass runs over the whole frame on top of it. Two binarizations instead
of one; `detect_luma` then finds nothing and retries, doing both again. The
1.40 ms single-pass delta is exactly one Otsu pass over 921 600 px. The binary
image that comes out is uniform, so the finder scan on a flat frame is the
*cheapest* it ever gets — the opposite of what I wrote.

Corrected in `benchmarks/README.md` (with the offending snippet quoted) and in
the KG-17 entry in `handoff/BUILD-LOG.md`, both attributed to Richard.

### MAJOR 3 — the headline encoder finding was inferred, never measured — FIXED

Fair hit. `Options::mask` was sitting right there and I had already used the
identical technique for the DP two groups above. Added a
`qr-encode — mask selection cost (auto vs pinned)` group,
`benchmarks/benches/encode.rs:143-174`. My numbers match his:

| | median | pinned | selection's share |
|---|---:|---:|---:|
| v1-M, 8 digits | 42.08 µs | 3.21 µs | **92.4 %** |
| v40-L, 2953 B | 9.81 ms | 744.50 µs | **92.4 %** |

The README headline now leads with the measured split instead of the inference,
and the prose points at the table rather than carrying the claim itself.

### MINORs

| # | Action |
|---|---|
| 4 — table not verbatim | **Fixed.** README's baseline is now a fenced block of literal harness output, diffable against a fresh `cargo bench` run. |
| 5 — `xB` truncated off the `rs_correct` rows | **Fixed.** Dropped `(= floor(ec/2))` from the label (it is in the doc comment); the `x256`/`x32` tags now fit. Same for the v40 mask row. |
| 6 — `decode v1-M` and `decode_micro` sit at 32-41 ticks | **Fixed.** Both batched x64, matching the qr-core pattern. Per-symbol figures given under the table (1.74 µs and 1.38 µs). Timer granularity documented in "Reading it honestly". |
| 7 — thin p95 tails on sub-200-iteration rows | **Documented, not changed.** Raising `iters_hint` cannot help: those rows are iteration-bound by the ~1 s target, not by the ceiling. README names the specific rows and says to read min/median on them. |
| 8 — unasserted fixtures | **Fixed.** `detect.rs:58-95` now asserts flat-frame emptiness, both `detect_luma_single_pass` outcomes (decodes the happy frame, finds nothing in both miss frames), and payload equality on the rotated frame. |
| 9 — "DP is free" leans on the weaker row | **Fixed.** README states the conclusion off `min` (548.08 vs 542.17 µs, ~1 %) and says explicitly that the v40 row cannot show a DP cost by construction — all-`pseudo_bytes` ≥ 0x80 leaves the DP no alternative segmentation to weigh. |
| 10 — work inside the timed loop | **Noted in README**, no change, per his own "no change needed". |

### Escalation forwarded to Arch, untouched

Richard's finding that `cargo test --workspace --no-default-features` provides
**zero** micro-off coverage — and has not for some time — is a product/CI
decision, not mine to take. Same for whether the flat-frame binarize fix becomes
its own step against `qr-detect` rather than staying a Known Gap. Both are in
his feedback under "Escalate to Architect"; I have not acted on either. KG-17
records the mechanism either way.

---

## What was built

A new workspace member `benchmarks/` with a hand-rolled timing harness and five
bench targets. **Zero new dependencies** — no criterion, no `test::Bencher`,
nothing outside `std` and the workspace crates.

## Files

| File | What / why |
|---|---|
| `Cargo.toml` | Adds `"benchmarks"` to `members`. The only root edit. |
| `benchmarks/Cargo.toml` | Dev-deps on the six qr crates + `image` at `workspace = true`; `autobenches = false`; five `[[bench]]` entries with `harness = false`; a `micro` feature gating this crate's micro benches. |
| `benchmarks/src/lib.rs` | The harness: `group`, `bench`, `bench_units`, `Unit`, `quick`. Calibration, warmup, per-iteration timing, percentiles, aligned table output, duration/rate formatting. |
| `benchmarks/benches/common/mod.rs` | Shared fixture builders: luma conversion, 720p frame construction, blitting, bilinear rotation, deterministic pseudo-random payloads, capacity-filling symbol fixtures. |
| `benchmarks/benches/core.rs` | Reed-Solomon encode/correct at v1-M and v40-L block shapes (clean and at `floor(ec/2)` errors), format-info BCH decode (exact and 1-bit-error paths), GF(256) mul/inv. |
| `benchmarks/benches/encode.rs` | `encode()` at v1 numeric / v10 URL-ish / v40 max byte; DP-on vs DP-off at pinned versions; **mask auto vs `mask: Some(0)`**; `encode_micro` M4-Q. |
| `benchmarks/benches/decode.rs` | `decode()` on clean v1 (x64) / v10 / v40, on a maximally-damaged-but-correctable v10, plus `decode_micro` M4 (x64). |
| `benchmarks/benches/detect.rs` | `detect_luma` happy / rotated 30° / miss (cluttered) / miss (flat), each with a `detect_luma_single_pass` counterpart, plus `detect_micro_luma` and `detect_all_luma`. |
| `benchmarks/benches/video.rs` | `process_frame` steady-state ROI vs full-scan cadence, 720p. |
| `benchmarks/README.md` | Run instructions, column semantics, how to read the table honestly, fixture policy, verbatim baseline, findings. |
| `.github/workflows/ci.yml` | New `bench-smoke` job: `BENCH_QUICK=1 cargo bench -p benchmarks`. |

---

## Design decisions worth your attention

**1. Calibration and batching.** Full mode aims each bench at ~1 s: measure a
growing batch until it exceeds 20 ms, derive per-iteration cost, run
`min(1s / cost, iters_hint)` iterations. `BENCH_QUICK=1` skips calibration and
runs three.

Every timed iteration pays one `Instant::now` pair (~41 ns on this machine, with
41.67 ns granularity — Richard measured both). Anything under ~2 µs per
operation loops `B` times inside one timed iteration and carries `xB` in its
name: GF mul/inv, format BCH, small RS blocks, `decode` v1, `decode_micro`.

**2. What could not be isolated through the public API.** `qr_detect::binarize`
is private, so there is no binarize-only bench; the primary pass is measured
through `detect_luma_single_pass` instead, and the gap to `detect_luma` on a
miss-path frame is exactly the retry. Segmentation and mask selection have no
public entry points either, but `Options` exposes a lever for each — `mode` and
`mask` — so both are measured by difference at a pinned version.

**3. Fixtures assert what they are.** Every fixture is built in-process from
`qr-encode` + `qr-render` — no files, no network, no clock — and asserts its
identity before timing starts: version by matrix size; decoded payload equality
on both the upright and rotated detect frames; emptiness on both miss-path
frames; both `detect_luma_single_pass` outcomes; and `Stats` deltas after the
video benches (`roi_misses` and `full_scans` unchanged for the ROI bench, every
frame full-scanned for the cadence bench).

The v10 URL fixture searches for the payload tail length that lands on version
10 rather than hard-coding one, and the byte fixtures are computed from
`ec_blocks()` capacity. If a capacity table is ever corrected, these fail loudly
instead of quietly benchmarking a neighbouring version.

**4. The corrupted-decode fixture is genuinely worst-case.** `worst_correctable`
flips evenly-spread data modules and walks the flip count down from the
theoretical ceiling until the symbol decodes again. For v10-M that settles at 65
flipped modules repairing 49 codewords — the most damage this symbol survives.

---

## Baseline

Apple M4, macOS 26.4.1, rustc 1.96.0, `bench` profile, 2026-08-11, measured in
an isolated worktree at clean HEAD (`ec4085e`) plus `benchmarks/`. Verbatim
harness output in `benchmarks/README.md`. Headline medians:

| | median |
|---|---:|
| `encode()` v1-M, 8 digits | 42.42 µs |
| `encode()` v1-M, `mask: Some(0)` | 3.21 µs |
| `encode()` v40-L, 2953 B | 10.21 ms |
| `encode()` v40-L, `mask: Some(0)` | 744.50 µs |
| `encode_micro` M4-Q | 5.08 µs |
| `decode()` v1-M | 1.74 µs (batched x64) |
| `decode()` v40-L | 629.88 µs |
| `decode_micro` M4-Q | 1.38 µs (batched x64) |
| `detect_luma` 720p, happy path | 2.13 ms |
| `detect_luma` 720p, miss (cluttered) | 4.07 ms |
| `detect_luma` 720p, miss (flat) | **6.88 ms** |
| `process_frame` steady-state ROI | 902.71 µs |
| `process_frame` full-scan cadence | 2.15 ms |
| `gf::mul` | 0.58 ns/op |
| `rs_correct` v40-L at 15 errors | 23.21 µs/op |

---

## Findings

**Encoding is 92.4 % mask selection, at both ends of the version range.** Now
measured: v1 42.08 µs auto vs 3.21 µs pinned, v40 9.81 ms vs 744.50 µs.
Everything else combined — segmentation, padding, Reed-Solomon, interleaving,
placing the one mask that gets kept — is the remaining 8 %. The RS inside a v1
encode is 256 ns, 0.6 % of the total. This is also why `encode()` costs 24x
`decode()` on the same symbol: the decoder reads the chosen mask out of the
format information instead of scoring eight candidates.

**A featureless frame binarizes twice.** 6.88 ms flat vs 4.07 ms cluttered vs
2.13 ms happy. `binarize.rs:90-96` discards the completed Bradley pass when
`dark_count == 0` and runs a full Otsu over the frame on top; `detect_luma` then
retries and does both again. Fix is small and local. KG-17.

**The segmentation DP costs ~1 %** — 548.08 vs 542.17 µs on min at v10, medians
inside the noise. The v40 row cannot show a DP cost by construction and is not
evidence either way.

**The retry pass is a clean 2.0x doubling** on both symbol-free frames, and free
on the happy path (a usable primary short-circuits it).

**Rotation costs 1 %.** 30 degrees is 2.15 vs 2.13 ms — the perspective sampler
is not where detection time goes.

**ROI tracking gives 2.4x**, not more: 902.71 µs vs 2.15 ms. Bounded by
`roi_padding_frac: 0.5`, which keeps a good fraction of the frame in the box.

---

## Guards run

All run in an isolated worktree at clean HEAD (`ec4085e`) plus `benchmarks/` and
the one-line root `Cargo.toml` edit — same method Richard used, and for the same
reason (see caveat).

| Check | Result |
|---|---|
| `cargo test --workspace` | green — 39 test targets, 0 failures |
| `cargo test --workspace --no-default-features` | green — 39 test targets, 0 failures |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo clippy --workspace --all-targets --no-default-features -- -D warnings` | clean |
| `cargo fmt --all --check` | clean |
| `cargo bench -p benchmarks` | full suite runs, all fixture assertions pass (output is the README baseline) |
| `BENCH_QUICK=1 cargo bench -p benchmarks` | all 5 targets, 34 rows, under a second (the CI gate) |

**Caveat.** In the live working tree a sibling has in-flight edits to
`crates/qr-detect/src/micro.rs` plus an untracked
`crates/qr-detect/tests/zz_scratch.rs`; the scratch test currently fails to
compile and is the sole source of fmt diffs there. Per Arch I have not touched
`crates/qr-detect`, and I did not park their files to run my guards — hence the
worktree. Nothing above is contaminated by their work, and nothing above clears
it. Nothing in `benchmarks/` depends on it.

---

## Open questions

1. **`iters_hint` values.** Chosen so no bench runs longer than ~1 s and none
   runs so few iterations that the percentiles are meaningless. A judgement
   call, not derived from anything.

2. Both round-1 open questions are now yours via Richard's escalation section:
   the `--no-default-features` lane's lack of micro-off coverage, and whether
   the flat-frame binarize fix gets its own step.
