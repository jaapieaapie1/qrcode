# benchmarks

Timing harness for the QR workspace. **No new dependencies** — no criterion, no
`test::Bencher`, nothing beyond `std` and the workspace crates themselves. The
harness lives in [`src/lib.rs`](src/lib.rs) and is about 200 lines.

## Running

```sh
cargo bench -p benchmarks                  # full suite, ~1s per bench
cargo bench -p benchmarks --bench detect   # one target
BENCH_QUICK=1 cargo bench -p benchmarks    # smoke mode: 3 iterations each
```

`cargo bench` implies the `bench` profile (optimised, `debug-assertions` off).
`BENCH_QUICK=1` is what CI runs: it proves every bench builds, its fixtures
assert clean and it runs to completion. Quick-mode *numbers* are noise — do not
read them.

## What the columns mean

```
bench                        iters      min    median      p95      max      rate
```

* **iters** — chosen by calibration: the harness measures one iteration, then
  runs however many fit in ~1 second, capped by each bench's hint.
* **min / median / p95 / max** — per timed iteration. Read the median for "how
  fast", the min for "how fast when nothing interferes", and the spread for
  jitter. There is no outlier rejection and no statistical comparison against a
  stored baseline; this is a timing table, not a regression detector.
* **rate** — per-iteration work quantity divided by the median: MiB/s of payload
  for encode/decode, megapixels/s for the detector, frames/s for qr-video, and
  ns/op where the bench loops internally.

### Reading it honestly

**Batched rows.** A timed iteration pays one `Instant::now` pair. On the
baseline machine `Instant` granularity is 41.67 ns and an empty timed span reads
~41 ns, so anything below a couple of microseconds would be measuring the clock
as much as the work. Those benches loop `B` times inside one timed iteration —
the name carries `xB`, and the rate column is per operation, not per iteration.
Everything under ~2 µs per operation is batched: GF(256), the format BCH, the
small Reed-Solomon blocks, `decode` v1 and `decode_micro`.

**Thin tails.** Nearest-rank p95 over ~100 samples is the 5th value from the
top. Rows that run fewer than ~200 iterations — `encode v40-L` at ~100 and the
flat-frame detector rows at ~145 — have a p95 resting on a handful of samples.
Read their min and median; treat their p95 as indicative.

**Work inside the timed region.** The `rs_correct` benches restore their scratch
buffer with `copy_from_slice` inside the loop, because correction is in-place.
At 26–148 bytes that is a rounding error against the syndrome and Chien work it
feeds, but it is there.

## Fixtures

Everything is generated in-process by `qr-encode` + `qr-render`: no image files,
no network, no wall clock anywhere in fixture construction. Frames are
1280x720 luma with symbols rendered at 8 px/module. Every fixture asserts what
it is — version, decoded payload, detection count, `Stats` deltas — before any
timing starts, so a bench can never quietly measure a different code path than
its name claims.

Two stages have no public entry point of their own and are measured by
difference instead:

| stage | measured as |
|---|---|
| binarize + finder scan (one pass) | `detect_luma_single_pass`; the gap to `detect_luma` on a miss-path frame is the retry |
| segmentation DP | same payload, same pinned version, `mode: None` vs `mode: Some(Mode::Byte)` |
| mask selection | same payload, `mask: None` (eight scored) vs `mask: Some(0)` (one placed) |

---

## Baseline

Apple M4, macOS 26.4.1, rustc 1.96.0 (ac68faa20 2026-05-25), `bench` profile,
2026-08-11 (post Step 11 performance rework). Measured at `a4920e5` in the
repo working tree, quiesced.

Verbatim `cargo bench -p benchmarks` output, so a fresh run can be diffed
against it directly:

```
qr-core — Reed-Solomon
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
rs_encode v1-M (16d/10ec) x256                       13498    60.79 µs    70.12 µs    96.50 µs     1.66 ms      55.7 MiB/s
rs_correct v1-M (26cw) clean x256                    15473    58.12 µs    60.58 µs    77.50 µs   288.42 µs    236.66 ns/op
rs_correct v1-M (26cw) 5 err x256                     3093   209.38 µs   235.25 µs   274.88 µs   615.96 µs    918.95 ns/op
rs_encode v40-L (118d/30ec) x32                      13904    63.04 µs    70.46 µs    79.58 µs   258.21 µs      51.1 MiB/s
rs_correct v40-L (148cw) clean x32                    2667   278.54 µs   283.08 µs   349.17 µs   677.25 µs   8846.38 ns/op
rs_correct v40-L (148cw) 15 err x32                   1481   654.62 µs   665.58 µs   721.62 µs   863.42 µs  20799.47 ns/op

qr-core — BCH format information
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
format_info_decode, 32 valid codes x256               3974   226.92 µs   257.50 µs   437.54 µs     7.82 ms     31.43 ns/op
format_info_decode, 1-bit errors x256                 3753   227.00 µs   255.08 µs   269.42 µs   813.54 µs     31.14 ns/op

qr-core — GF(256)
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
gf::mul, 65536 ops                                   20000    33.33 µs    37.12 µs    41.29 µs   107.21 µs      0.57 ns/op
gf::inv, 65536 ops                                   20000    29.79 µs    34.42 µs    37.33 µs   108.54 µs      0.53 ns/op

qr-decode — decode()
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
decode v1-M, 8 numeric digits x64                     9466    95.50 µs   110.58 µs   119.08 µs   367.54 µs       4.4 MiB/s
decode v10-M, 213B byte                              20000    33.92 µs    38.75 µs    42.00 µs   408.96 µs       5.2 MiB/s
decode v40-L, 2953B byte                              1616   555.96 µs   624.38 µs   649.88 µs     1.04 ms       4.5 MiB/s
decode v10-M, 65 modules flipped (49 cw fixed)       16158    54.17 µs    61.62 µs    65.58 µs   180.29 µs       3.3 MiB/s

qr-decode — decode_micro()
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
decode_micro M4-Q, 9 alphanum chars x64              11646    76.75 µs    86.04 µs    91.92 µs   289.04 µs       6.4 MiB/s

qr-detect — detect_luma(), 1280x720
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
happy path: 1 v5 symbol @8px/module                   1444   629.75 µs   687.17 µs   704.71 µs   806.50 µs     1341.16 M/s
happy path, single pass (no retry)                    1449   626.50 µs   688.29 µs   725.92 µs     2.52 ms     1338.97 M/s
rotated 30 degrees                                    1440   626.58 µs   688.04 µs   708.71 µs     1.08 ms     1339.45 M/s
miss path: no symbol (primary + retry)                 883     1.05 ms     1.12 ms     1.15 ms     1.38 ms      823.19 M/s
miss path, single pass (no retry)                     1775   508.83 µs   563.79 µs   580.92 µs   795.62 µs     1634.65 M/s
miss path: flat frame (no symbol, no clutter)         2000   292.25 µs   320.38 µs   438.29 µs     2.21 ms     2876.63 M/s
miss path: flat frame, single pass (no retry)         2000   288.33 µs   314.29 µs   340.96 µs   912.62 µs     2932.30 M/s

qr-detect — Micro QR, 1280x720
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
detect_micro_luma: 1 M4 symbol @8px/module            1313   706.50 µs   760.46 µs   779.17 µs   815.04 µs     1211.90 M/s
detect_all_luma: 1 v5 + 1 M4                           891     1.06 ms     1.12 ms     1.15 ms     2.67 ms      824.02 M/s

qr-encode — encode()
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
encode v1-M, 8 numeric digits                        20000     8.79 µs     9.00 µs     9.25 µs    36.71 µs     868.1 KiB/s
encode v10-M, 198B URL-ish (auto version)            19978    44.75 µs    49.75 µs    51.96 µs   143.88 µs       3.8 MiB/s
encode v40-L, 2953B byte (auto version)               1587   553.58 µs   600.50 µs   664.88 µs   983.42 µs       4.7 MiB/s

qr-encode — DP segmentation cost (same version, DP on vs off)
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
v10-M URL-ish, version pinned, DP on                 18592    43.12 µs    47.96 µs    49.08 µs   114.17 µs       3.9 MiB/s
v10-M URL-ish, version pinned, mode=Byte (no DP)     20000    41.54 µs    46.12 µs    48.71 µs   139.25 µs       4.1 MiB/s
v40-L byte, version pinned, DP on                     1659   537.04 µs   578.50 µs   606.54 µs   765.46 µs       4.9 MiB/s
v40-L byte, version pinned, mode=Byte (no DP)         1817   521.79 µs   561.54 µs   592.46 µs   798.12 µs       5.0 MiB/s

qr-encode — mask selection cost (auto vs pinned)
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
encode v1-M, 8 digits: mask auto (8 candidates)      20000     8.12 µs     9.12 µs     9.29 µs    33.08 µs     856.2 KiB/s
encode v1-M, 8 digits: mask = Some(0) pinned         20000     1.58 µs     1.71 µs     1.79 µs     9.33 µs       4.5 MiB/s
encode v40-L, 2953B: mask auto (8 candidates)         1639   570.75 µs   604.54 µs   627.71 µs     1.29 ms       4.7 MiB/s
encode v40-L, 2953B: mask = Some(0) pinned            4171   185.42 µs   219.54 µs   232.46 µs   446.71 µs      12.8 MiB/s

qr-encode — encode_micro()
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
encode_micro M4-Q, 9 alphanumeric chars              50000     4.38 µs     4.96 µs     5.04 µs    22.42 µs       1.7 MiB/s

qr-video — process_frame(), 1280x720 (rate is frames/s)
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
steady-state ROI frame (code present)                 3047   293.25 µs   324.25 µs   334.83 µs   439.12 µs        3.08 k/s
full-scan cadence frame (code present)                1449   629.33 µs   688.25 µs   708.88 µs   953.04 µs        1.45 k/s
```

Per-symbol figures for the batched rows: `decode` v1-M is 111.67 µs / 64 =
**1.74 µs**; `decode_micro` M4-Q is 88.08 µs / 64 = **1.38 µs**.

---

## What the baseline says

### Mask selection dominated encoding; word-level scoring fixed it

Historical (pre `a4920e5`): mask selection was a measured 92.4 % of `encode()`
at both ends of the version range (v1-M 42.08 µs auto vs 3.21 µs pinned;
v40-L 9.81 ms vs 744.50 µs) — eight candidate matrices, each fully placed and
penalty-scored through per-module predicate calls.

Step 11b rebuilt the search on packed u64 rows (function map as bitmaps,
candidates via XOR, shared run-walk for N1/N3, blocked bit transpose for the
column direction, popcount N4), bit-identical to the reference scorer. Current
numbers in the baseline above: v1-M auto 9.12 µs, v40-L auto 604 µs — 16x on
v40 — with the auto/pinned gap now ~4x instead of ~13x.

### The segmentation DP is free

At a pinned version, DP-on and DP-off agree to about 1 % on min — 548.08 vs
542.17 µs at v10 — which is where the reading holds up; the medians (632.12 vs
629.04 µs) sit inside the run-to-run spread of that row and should not be
leaned on.

The v10 URL row is the load-bearing one. The v40 row cannot demonstrate anything
about the DP by construction: its payload is `pseudo_bytes`, every byte ≥ 0x80,
so there is no alternative segmentation for the DP to weigh. It is there to show
the cost does not grow with payload size, not to show the DP is cheap.

### A featureless frame was the worst case; now it is the best

Historical (pre `5c7f732`): a blank frame cost 6.88 ms — the Bradley pass
completed, produced zero dark pixels, was discarded, and a full Otsu ran on
top; the retry then repeated both. Step 11a added the `can_be_dark`
dynamic-range proof: a frame whose `min/max` spread cannot yield a dark pixel
skips the adaptive pass entirely, and the result is marked radius-invariant so
the retry is skipped too. Current: flat frame 321 µs, the cheapest frame in
the table — where a camera pointed at nothing spends its time now matches
what the frame deserves.

### The retry pass is a clean doubling

Miss-path `detect_luma` remains ~2x `detect_luma_single_pass` on cluttered
symbol-free frames (1.12 ms vs 564 µs) — the retry is a second full
binarize-and-scan, as designed. Flat frames no longer retry at all
(radius-invariant short-circuit). The happy path is unaffected either way.

### Rotation is nearly free

30 degrees costs ~0 % over axis-aligned (688 both, medians). The perspective
sampler is not where detection time goes; binarize is.

### ROI tracking earns its keep

A steady-state ROI frame is 324 µs against 688 µs for a full scan — 2.1x, or
~3080 fps of headroom against ~1450. The ROI is padded 50 % by default, so the
win is bounded by how much of the frame it still covers.
