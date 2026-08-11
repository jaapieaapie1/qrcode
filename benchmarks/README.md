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
2026-08-11. Measured in an isolated `git worktree` at clean HEAD (`ec4085e`)
plus `benchmarks/`.

Verbatim `cargo bench -p benchmarks` output, so a fresh run can be diffed
against it directly:

```
qr-core — Reed-Solomon
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
rs_encode v1-M (16d/10ec) x256                       13744    67.75 µs    70.17 µs    92.08 µs   271.33 µs      55.7 MiB/s
rs_correct v1-M (26cw) clean x256                    15222    65.08 µs    65.42 µs    68.08 µs   205.17 µs    255.54 ns/op
rs_correct v1-M (26cw) 5 err x256                     4214   232.29 µs   237.58 µs   246.33 µs   632.79 µs    928.06 ns/op
rs_encode v40-L (118d/30ec) x32                      13970    70.21 µs    71.25 µs    74.38 µs   306.79 µs      50.5 MiB/s
rs_correct v40-L (148cw) clean x32                    3188   310.67 µs   312.92 µs   319.33 µs   534.96 µs   9778.66 ns/op
rs_correct v40-L (148cw) 15 err x32                   1348   731.54 µs   742.67 µs   756.96 µs   930.29 µs  23208.34 ns/op

qr-core — BCH format information
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
format_info_decode, 32 valid codes x256               3897   254.00 µs   254.58 µs   261.58 µs   950.17 µs     31.08 ns/op
format_info_decode, 1-bit errors x256                 3911   254.04 µs   257.08 µs   264.58 µs   828.88 µs     31.38 ns/op

qr-core — GF(256)
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
gf::mul, 65536 ops                                   20000    37.17 µs    37.71 µs    41.75 µs   156.04 µs      0.58 ns/op
gf::inv, 65536 ops                                   20000    33.33 µs    34.92 µs    36.88 µs    97.83 µs      0.53 ns/op


qr-decode — decode()
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
decode v1-M, 8 numeric digits x64                     8151   105.17 µs   111.67 µs   119.79 µs   237.88 µs       4.4 MiB/s
decode v10-M, 213B byte                              20000    37.96 µs    39.67 µs    76.29 µs   184.04 µs       5.1 MiB/s
decode v40-L, 2953B byte                              1562   618.83 µs   629.88 µs   640.79 µs   767.50 µs       4.5 MiB/s
decode v10-M, 65 modules flipped (49 cw fixed)       15779    60.88 µs    62.71 µs    65.12 µs   147.54 µs       3.2 MiB/s

qr-decode — decode_micro()
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
decode_micro M4-Q, 9 alphanum chars x64              11511    85.33 µs    88.08 µs    95.46 µs   396.21 µs       6.2 MiB/s


qr-detect — detect_luma(), 1280x720
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
happy path: 1 v5 symbol @8px/module                    455     2.10 ms     2.13 ms     2.21 ms     3.58 ms      432.29 M/s
happy path, single pass (no retry)                     457     2.10 ms     2.13 ms     2.19 ms     2.64 ms      433.25 M/s
rotated 30 degrees                                     465     2.12 ms     2.15 ms     2.28 ms     5.72 ms      428.49 M/s
miss path: no symbol (primary + retry)                 244     4.02 ms     4.07 ms     4.15 ms     4.29 ms      226.71 M/s
miss path, single pass (no retry)                      489     2.00 ms     2.04 ms     2.10 ms     5.05 ms      452.19 M/s
miss path: flat frame (no symbol, no clutter)          145     6.83 ms     6.88 ms     7.00 ms     7.32 ms      133.98 M/s
miss path: flat frame, single pass (no retry)          291     3.41 ms     3.44 ms     3.55 ms     7.00 ms      267.62 M/s

qr-detect — Micro QR, 1280x720
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
detect_micro_luma: 1 M4 symbol @8px/module             457     2.14 ms     2.19 ms     2.72 ms     5.42 ms      421.01 M/s
detect_all_luma: 1 v5 + 1 M4                           385     2.54 ms     2.58 ms     2.71 ms     2.87 ms      356.96 M/s


qr-encode — encode()
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
encode v1-M, 8 numeric digits                        20000    40.92 µs    42.42 µs    45.08 µs   354.50 µs     184.2 KiB/s
encode v10-M, 198B URL-ish (auto version)             1540   533.38 µs   610.71 µs     1.51 ms    27.84 ms     316.6 KiB/s
encode v40-L, 2953B byte (auto version)                 92     9.70 ms    10.21 ms    12.62 ms    16.30 ms     282.3 KiB/s

qr-encode — DP segmentation cost (same version, DP on vs off)
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
v10-M URL-ish, version pinned, DP on                  1518   548.08 µs   632.12 µs   903.00 µs    10.03 ms     305.9 KiB/s
v10-M URL-ish, version pinned, mode=Byte (no DP)      1651   542.17 µs   629.04 µs   852.08 µs     1.73 ms     307.4 KiB/s
v40-L byte, version pinned, DP on                       97     9.66 ms     9.83 ms    11.27 ms    26.40 ms     293.4 KiB/s
v40-L byte, version pinned, mode=Byte (no DP)          101     9.65 ms     9.85 ms    12.02 ms    14.72 ms     292.8 KiB/s

qr-encode — mask selection cost (auto vs pinned)
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
encode v1-M, 8 digits: mask auto (8 candidates)      20000    41.21 µs    42.08 µs    71.71 µs     1.57 ms     185.6 KiB/s
encode v1-M, 8 digits: mask = Some(0) pinned         20000     3.04 µs     3.21 µs     3.33 µs    10.04 µs       2.4 MiB/s
encode v40-L, 2953B: mask auto (8 candidates)          102     9.59 ms     9.81 ms    10.07 ms    10.21 ms     293.8 KiB/s
encode v40-L, 2953B: mask = Some(0) pinned            1320   710.67 µs   744.50 µs     1.06 ms     3.27 ms       3.8 MiB/s

qr-encode — encode_micro()
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
encode_micro M4-Q, 9 alphanumeric chars              50000     4.79 µs     5.08 µs     5.21 µs    69.46 µs       1.7 MiB/s


qr-video — process_frame(), 1280x720 (rate is frames/s)
bench                                                iters         min      median         p95         max            rate
--------------------------------------------------------------------------------------------------------------------------
steady-state ROI frame (code present)                 1108   883.25 µs   902.71 µs     1.02 ms     1.81 ms        1.11 k/s
full-scan cadence frame (code present)                 471     2.11 ms     2.15 ms     2.64 ms     6.00 ms        464.3 /s
```

Per-symbol figures for the batched rows: `decode` v1-M is 111.67 µs / 64 =
**1.74 µs**; `decode_micro` M4-Q is 88.08 µs / 64 = **1.38 µs**.

---

## What the baseline says

### Encoding is 92 % mask selection — measured, not inferred

| | median | pinned to one mask | selection's share |
|---|---:|---:|---:|
| `encode()` v1-M, 8 digits | 42.08 µs | 3.21 µs | **92.4 %** |
| `encode()` v40-L, 2953 B | 9.81 ms | 744.50 µs | **92.4 %** |

Eight candidate matrices, each fully placed and penalty-scored, are more than
nine tenths of `encode()` at both ends of the version range. Everything else
combined — segmentation, padding, Reed-Solomon, interleaving, placement of the
one mask that gets kept — is the remaining 8 %. The RS inside a v1 encode is
256 ns, 0.6 % of the total.

That is the whole optimisation story for the encoder. It is also why `encode()`
costs 24x what `decode()` does on the same v1 symbol (42.08 µs vs 1.74 µs): the
decoder never scores a mask, it reads which one was used out of the format
information.

### The segmentation DP is free

At a pinned version, DP-on and DP-off agree to about 1 % on min — 548.08 vs
542.17 µs at v10 — which is where the reading holds up; the medians (632.12 vs
629.04 µs) sit inside the run-to-run spread of that row and should not be
leaned on.

The v10 URL row is the load-bearing one. The v40 row cannot demonstrate anything
about the DP by construction: its payload is `pseudo_bytes`, every byte ≥ 0x80,
so there is no alternative segmentation for the DP to weigh. It is there to show
the cost does not grow with payload size, not to show the DP is cheap.

### A featureless frame is the detector's worst case, not its best

A blank 720p frame takes 6.88 ms — 3.2x the happy path (2.13 ms) and 1.7x the
*cluttered* symbol-free frame (4.07 ms). Single-pass: 3.44 ms flat against
2.04 ms cluttered.

The mechanism is `crates/qr-detect/src/binarize.rs:90-96`:

```rust
let dark_count = bin.dark.iter().filter(|&&d| d).count();
if dark_count == 0 || dark_count == total {
    return Some(otsu(luma, width, height));
}
```

On a uniform frame the Bradley pass yields `dark_count == 0`, so the completed
adaptive result — integral image plus 921 600 window evaluations — is thrown
away and a full Otsu pass (histogram, threshold application, a second
`Vec<bool>`) runs over the whole frame on top of it. **Two binarizations instead
of one.** The 1.40 ms single-pass delta is exactly one Otsu pass over 921 600
pixels. Then `detect_luma` finds nothing and retries, doing the whole thing
again — hence 6.88 ms.

It is *not* extra finder scanning. The binary image that comes out is uniform,
so the finder scan on a flat frame is the cheapest it ever gets.

### The retry pass is a clean doubling

Miss-path `detect_luma` is almost exactly 2x `detect_luma_single_pass` on both
symbol-free frames (4.07 vs 2.04 ms; 6.88 vs 3.44 ms) — the retry is a second
full binarize-and-scan, as designed. It costs nothing on the happy path (2.13 ms
either way), because a usable primary result short-circuits it.

### Rotation is nearly free

30 degrees costs 1 % over axis-aligned (2.15 vs 2.13 ms). The perspective
sampler is not where detection time goes; binarize is.

### ROI tracking earns its keep

A steady-state ROI frame is 902.71 µs against 2.15 ms for a full scan — 2.4x, or
1110 fps of headroom against 464. The ROI is padded 50 % by default, so the win
is bounded by how much of the frame it still covers.
