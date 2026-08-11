# REVIEW REQUEST — Step 13: `no_std` qr-detect for ESP32-class targets

**Ready for Review: YES**
Builder: Bob-NSD · 2026-08-11

---

## Verdict up front

Feasible, done, and cheaper than expected.

* `qr-detect` gains a default-on `std` feature. Without it the crate is
  `#![no_std]` + `alloc`, the `image` dependency disappears, and the whole
  pipeline — binarize, finder scan, grouping, sampler, micro, decode — compiles
  and **runs** on `riscv32imc-unknown-none-elf`.
* Peak heap for a QVGA (320x240) frame end to end on the emulated ESP32-C3:
  **163,544 of 196,608 bytes (83%)**, QEMU `ALL PASS`.
* Hand-rolled float math is worst-case **1 ulp** (2.2204e-16 relative) on
  `sqrt`, **bit-exact** on `ceil`/`round`, **2.2204e-16 absolute** on `sin_cos`
  — against a brief target of 1e-9 relative / 1e-7 absolute, so six to nine
  orders of margin.
* Detection results are identical on both float paths: the entire existing
  corpus (90 qr-detect tests, plus the cross-crate warp/fuzz/multiscan/video
  suites) passes twice — once as a real `no_std` build, once with the whole
  workspace forced onto the hand-rolled arithmetic.
* Benchmarks unmoved: happy-path median 679.42 µs against a README baseline of
  687.17 µs (−1.1%); every detector and video row inside ±2%.

Nothing in the brief was left undone. One item was deliberately declined and
logged as a Known Gap: bit-packing `Binary`. Reasoning and measured numbers
below.

---

## Files changed

| file | lines | what and why |
|---|---|---|
| `crates/qr-detect/Cargo.toml` | 8-45 | `image` becomes optional; new default-on `std` feature owns it; `[lints.rust]` declares the `qr_detect_soft_float` verification cfg. |
| `crates/qr-detect/src/mathf.rs` | new, 541 | The four float routines `core` lacks, in two implementations behind one interface, plus the differential tests against `std`. |
| `crates/qr-detect/src/lib.rs` | 124-154, 165, 187, 333, 397, 457 | `no_std` attribute + `extern crate alloc`; `mathf` module; the four `GrayImage` entry points gated on `std`; crate docs gain a `# no_std` section. |
| `crates/qr-detect/src/binarize.rs` | 30-32 | `alloc::vec` / `alloc::vec::Vec` imports (no prelude under `no_std`). |
| `crates/qr-detect/src/finder.rs` | 8-10 | Same. |
| `crates/qr-detect/src/grouping.rs` | 9-13, 67 | Same, plus `snap_dimension` routes its `round` through `mathf`. |
| `crates/qr-detect/src/geometry.rs` | 28 | `Point::distance` routes its `sqrt` through `mathf`. |
| `crates/qr-detect/src/sampler.rs` | 10, 59, 98, 104, 130, 144, 171, 183 | Eight `round`/`sqrt`/`ceil` call sites routed through `mathf`. |
| `crates/qr-detect/src/micro.rs` | 39-45, 208, 246, 299-300, 899, 957, 974 | Seven `round`/`sqrt`/`sin_cos` call sites routed through `mathf`; `alloc` imports. |
| `esp-selftest/Cargo.toml` | 12-22 | Adds `qr-detect` with `default-features = false, features = ["decode","micro"]`. |
| `esp-selftest/src/runtime.rs` | 331-339 | New `heap_used()` — the bump pointer, so a suite can break its own peak down. |
| `esp-selftest/src/main.rs` | 1-32, 530-660, 663, 707-717 | Suite 5: a synthetic QVGA frame carrying one QR v2 and one Micro QR M3, through `detect_all_luma`, both decoded, memory split printed. |
| `.github/workflows/ci.yml` | 33-49, 55-56, 92-102, 195-198 | Two new test lanes, one clippy lane, three cross-target checks, two extra QEMU log assertions. |

---

## 1. Feature matrix

`default = ["std", "decode", "micro"]`. `std = ["dep:image"]`.

| features | `image` dep | `GrayImage` API | `_luma` API | float math | verified by |
|---|---|---|---|---|---|
| `std,decode,micro` (default) | yes | yes | yes | `std` | `cargo test --workspace` |
| `std,decode` | yes | yes (no micro) | yes | `std` | `cargo test -p qr-detect --no-default-features --features std,decode` |
| `std` | yes | yes (no micro/decode) | yes | `std` | ditto |
| `decode,micro` | **no** | **no** | yes | `mathf::soft` | `cargo test -p qr-detect --no-default-features --features decode,micro` |
| `micro` | no | no | yes | `mathf::soft` | clippy, both bare-metal targets |
| *(none)* | no | no | yes (QR only) | `mathf::soft` | clippy, both bare-metal targets |

**Additivity holds.** `std` only ever *adds* the `image`-typed conveniences —
`detect`, `read_one`, `detect_micro`, `detect_all` — which were always one-line
forwards to their `_luma` counterparts. Nothing is removed by enabling it, and
turning it on for a consumer that did not ask cannot break that consumer.

**Existing consumers are untouched.** `qr-video`, `demo-wasm`, `tests-integration`
and `benchmarks` all take `qr-detect` with default features on; `std` is in the
default set, so their dependency edges resolve to exactly the crate they resolved
to before. **Zero edits outside `crates/qr-detect/`, `esp-selftest/` and `ci.yml`.**
In particular `qr-video` needed no feature forward: it already uses only the
`_luma` entry points and it stays a `std` crate itself.

`qr_detect_soft_float` is a `--cfg`, **not** a cargo feature. It forces the
hand-rolled math on a `std` build. A cargo feature would have been wrong: features
must be additive and this one changes results (by at most an ulp), so a sibling
crate switching it on would silently change a consumer's arithmetic. As a cfg it
can only be set by whoever is invoking the compiler. `[lints.rust]` in the
manifest declares it to `check-cfg` so a typo is still a warning.

---

## 2. `mathf` — accuracy

`core` already carries `abs`, `min`, `max`, `clamp`, `signum`, `copysign`,
`to_radians` and the classification predicates (verified by compiling each
against `thumbv7em-none-eabihf`, not assumed). What it does not carry, and what
this pipeline actually forms, is exactly four functions. Grepping the production
paths — `atan2`, `hypot`, `powf`, `ln`, `floor` do **not** appear outside test
fixtures, which only ever build with `std` — the set is:

| function | call sites | implementation |
|---|---|---|
| `sqrt` | `geometry:28`, `sampler:98` ×2, `micro:957` | bit-trick exponent-halving seed + 5 Newton steps |
| `round` | `grouping:67`, `sampler` ×7, `micro` ×3 | significand masking via `trunc`, halves away from zero |
| `ceil` | `sampler:104` | ditto |
| `sin_cos` | `micro:208`, `micro:246`, `micro:974` | 3-term Cody-Waite reduction + fdlibm kernel polynomials |

`sin_cos` and everything behind it is gated on `feature = "micro"` — nothing
else in the crate forms an angle — so a micro-off build carries no trig at all
and stays clippy-clean under `-D warnings`.

### Measured against `std`

Every function is differential-tested against the `std` method it replaces, over
dense sweeps plus each function's edge cases. `mathf::tests`, and the numbers are
asserted, not merely printed:

| function | domain swept | points | worst observed | asserted bound | target from brief |
|---|---|---|---|---|---|
| `sqrt` | `[MIN_POSITIVE, MAX/4]` geometric ×1.05, one octave dense, `0..1e8` dense | ~440 k | **1 ulp**, rel. **2.2204e-16** | `<= 1 ulp`, rel `< 2.3e-16` | rel `< 1e-9` |
| `sqrt` edges | `±0`, `MIN_POSITIVE`, `MIN_POSITIVE/2`, `5e-324`, `1e-310`, `MAX`, `±inf`, NaN, negatives | 16 | exact on the specials, `<= 1 ulp` on the subnormals | as stated | — |
| `ceil` | `±200` at 1e-3, then ×1.5 to `MAX` with `±0.5` probes | ~402 k | **bit-exact** | `to_bits()` equality | — |
| `round` | same | ~402 k | **bit-exact** | `to_bits()` equality | — |
| `sin_cos` | `±200 rad` at 5e-4, then ×1.001 to `2^20` with 4 probes each | ~856 k | **2.2204e-16 absolute** (= 1 ulp of 1.0, at x = −198.706) | `< 1e-7`, then `< 3e-16` | abs `< 1e-7` |

The trig test asserts the loose bound *and* the real one, so a regression that
merely stays inside the contract still fails the build.

### Domain, stated honestly

* `sqrt` — the whole non-negative range including subnormals (scaled into the
  normal range by an exact power of two first). Negatives and NaN return NaN;
  `±0.0` returns itself with its sign; `+inf` returns `+inf`.
* `ceil` / `round` — everything, `±0` and non-finite included, bit-for-bit.
* `sin_cos` — `|x| <= 2^20` (≈1.05e6). Past that the three-way `pi/2` split runs
  out of the bits it reserves and the reduction sheds precision, so it returns
  NaN rather than a quiet lie. The pipeline's angles all live in `[-2pi, 2pi]`,
  six orders of magnitude inside the limit; `micro.rs:208` and `:246` form
  `i * TAU / RAYS` and `:974` a tweak of a few degrees.

Subnormal *arguments to trig* are not a case: they reduce to themselves and the
polynomial returns them unchanged.

### Why `sqrt` is 1 ulp and not 0

Plain `f64` Newton is not correctly rounded — about 25% of arguments land one ulp
off, and reformulating the iteration (`y + (x/y - y)/2`, or a final correction
step) does not change that, measured over 4 M points. Getting the last bit needs
an *exact* residual, which without FMA means a Dekker two-product; that was
prototyped and gets the miss rate to 4e-5 but still is not exact near the
subnormal boundary. Since it cannot be made a guarantee either way, the
equivalence has to be verified empirically regardless — so the simple form wins
and the verification below is what carries the claim.

### Why free functions and not an extension trait

A trait would have kept `x.sqrt()` at every call site. It would also have been
useless: inherent methods win method resolution, so on a `std` build `f64`'s own
`sqrt` shadows any trait method of the same name, and `--cfg qr_detect_soft_float`
would have tested nothing at all while appearing to work. Free functions cannot
be shadowed, so the switch is honest. This is written down at the top of
`mathf.rs` because it is exactly the kind of thing someone will try to "clean up".

---

## 3. Detection equivalence — the claim that actually matters

Floats agreeing to an ulp is not the claim; *detections* agreeing is. Two
independent runs, both green:

**(a) The real `no_std` build, natively.**
`cargo test -p qr-detect --no-default-features --features decode,micro` compiles
the library as `#![no_std]` with no `image` and the hand-rolled math, then links
it into a host test harness. **90 tests pass** — 38 unit, 24 `tests/detect.rs`,
28 `tests/micro_detect.rs` — the same 90 that pass under default features. This
turned out better than the brief anticipated: the bare-metal configuration is not
merely *checked* natively, it is *executed* natively, against the full corpus,
including the rotation sweep, perspective envelope, low-light ladder and
retry-pass fixtures.

**(b) The whole workspace on the hand-rolled path.**
`RUSTFLAGS='--cfg qr_detect_soft_float' cargo test --workspace` — 39 test targets,
all ok. That puts `tests-integration`'s warp / fuzz / multiscan suites and
`qr-video`'s tracking suite on the bare-metal arithmetic, which the crate-local
run in (a) cannot reach.

Both are now CI lanes.

### Regression guard on the hardening you told me not to break

Nothing was touched in the retry ladder, the low-light rungs or the perspective
refinement — the diff in those files is import lines and float call sites, no
control flow. The evidence is that the fixtures that pin that behaviour still
pass, on both float paths:
`a_degraded_micro_sample_does_not_suppress_the_retry`,
`a_degraded_micro_sample_does_not_suppress_the_retry_in_detect_all`,
`the_retry_window_holds_a_vignette_the_primary_loses`,
`adaptive_survives_a_vignette_to_the_documented_depth`,
`the_narrow_margin_sees_a_print_the_primary_margin_cannot`,
`smoothing_recovers_module_centres_that_noise_flips`,
`the_noise_estimate_separates_sensor_noise_from_texture`,
`a_frame_too_flat_to_threshold_is_settled_at_any_radius`,
`the_refinement_runs_when_tilted_and_not_when_upright`,
`perspective_envelope`, `rotation_sweep_is_within_the_documented_envelope`,
`a_micro_symbol_on_a_screen_photographed_off_axis_decodes`.

---

## 4. Memory audit

Measured, not modelled: a tracking `GlobalAlloc` wrapped around the host
allocator, running `detect_all_luma` over rendered frames. `pipeline_peak` is
peak live bytes *above* the caller's frame buffer.

| case | frame | pipeline peak | total | note |
|---|---|---|---|---|
| QVGA 320x240, symbol found on the primary pass | 76,800 | **79,364** | **156,164** | the normal path |
| QVGA 320x240, blank frame (radius-invariant, settles at one pass) | 76,800 | 76,896 | 153,696 | cheapest |
| QVGA 320x240, noisy frame that drives the retry ladder | 76,800 | **156,164** | **232,964** | worst case |
| VGA 640x480, symbol found on the primary pass | 307,200 | **312,324** | **619,524** | |
| VGA 640x480, noisy frame that drives the retry ladder | 307,200 | 619,524 | 926,724 | worst case |

The model is exact — the measurements match it to the byte:

```
primary pass:  pipeline_peak = w*h            (Binary::dark, Vec<bool>, 1 B/px)
                             + 4 * (2w + 1)   (column sums + prefix sums, u32)
retry pass:    + w*h                          (the denoised luma copy, held
                                               while the second Binary is built)
```

Everything else — `Vec<Finder>`, `Vec<QrCandidate>`, the sampled `BitMatrix`,
the decoder's block buffers — is under 3 KB combined at any frame size, because
none of it scales with pixels.

The accumulator width is chosen by `binarize::fits_u32`; both QVGA and VGA stay
on `u32`, so the `u64` arm only appears at frame sizes far past anything an
ESP32 will hold anyway.

### What this means for a real target

* **QVGA fits an ESP32-C3 comfortably.** 156 KB working set against ~400 KB of
  DRAM, and the emulated run below proves it inside a 192 KB arena with 33 KB to
  spare — *including* the frame buffer on the heap, which a real driver would
  keep in a static DMA region instead.
* **A noisy QVGA frame is the number to size against**, not the happy path:
  233 KB. Still inside a C3.
* **VGA does not fit a C3.** 620 KB on the happy path, 927 KB on the retry path.
  It needs an ESP32-S3 with PSRAM, or the bit-packing below.

### Bit-packing `Binary`: assessed, declined, logged

`Binary::dark` is `Vec<bool>` at 1 byte per pixel. Packed to bits it would be
9,600 B at QVGA and 38,400 B at VGA — cutting the *total* working set 43% at
QVGA (156 KB → 87 KB) and 43% at VGA (620 KB → 351 KB). That is a real prize,
and at VGA it is close to the difference between fitting an S3's 512 KB SRAM
and not.

It is not trivial, and the brief's bar was trivial. `Binary::row(y) -> &[bool]`
exists for one reason, documented at its definition: the finder scan is the only
stage that reads every pixel, and handing it a contiguous slice saves it a bounds
check and an `i64 -> usize` conversion per pixel. A packed representation cannot
return that slice. Every caller would move to per-pixel bit extraction in the
hottest loop in the crate, against a benchmark baseline this step is required to
hold within 5%. That is a performance-versus-footprint trade with a real design
question inside it (pack and re-widen a row at a time? keep the finder scan on
bytes and pack only for the sampler?), and it deserves its own step rather than a
corner of this one.

**Logged as KG-22.**

---

## 5. Proof on emulated hardware — esp-selftest suite 5

`esp-selftest` now links `qr-detect` with `default-features = false,
features = ["decode","micro"]` — the exact configuration the CI cross-target job
checks — and runs a fifth suite:

1. Encode a QR v2 (`"esp32 detect v2"`, level M) and a Micro QR M3
   (`"m3-ok"`, level L) with `qr-encode`, on the target.
2. Allocate a 320x240 luma buffer and draw both symbols into it at 6 px/module
   with a 12-line module-to-pixel blitter in the firmware — light 235, dark 25,
   deliberately not 255/0 so the adaptive threshold has real work. No `qr-render`
   dependency: that crate is `std` + `image`.
3. `qr_detect::detect_all_luma` over the frame.
4. Assert exactly one QR and exactly one micro detection, both decoded, both
   payloads and both versions exact.

The two symbols share a frame on purpose. `detect_all_luma` binarizes and scans
for finders once and feeds both groupers, so one frame exercises the QR triple
path *and* the single-finder micro path — including the rule that a finder
claimed by a triple never yields a phantom micro — at the cost of one QVGA
buffer. That matters here: a bump allocator cannot reclaim a frame mid-suite, so
a second frame would double the peak this suite exists to measure.

### QEMU transcript

```
qr-selftest: riscv32imc-unknown-none-elf on qemu virt
             qr-core + qr-encode + qr-decode + qr-detect, no_std + alloc

[1/5] qr roundtrip, v1-v10 sampled.......... PASS (29 symbols)
[2/5] micro qr, all 8 symbols............... PASS (8 symbols)
[3/5] reed-solomon recovery................. PASS (3 cases)
[4/5] iso 7.5.1 misdecode protection........ PASS (8 cases)
[5/5] detection from a qvga luma frame......
        320x240 luma frame 76800 B + pipeline 85792 B
        PASS (2 symbols)

heap peak 163544 of 196608 bytes (largest single suite)
ALL PASS
```

QEMU exit status 0.

**Heap peak: 163,544 of 196,608 bytes — 83.2% of the 192 KiB arena, 33,064 bytes
spare.** Suite 5 is now the largest suite by a wide margin (the previous peak was
encode's ~113 KB, KG-20), so the run-level figure is suite 5's figure.

The firmware's 85,792 B pipeline figure is larger than the host's 79,364 B for
the same frame because a bump allocator reclaims only in reverse order: it
includes both symbols' `BitMatrix`es, the micro sampler's transients, the
decoders' buffers, and every `Vec` growth that a real allocator would have
recycled. 6,428 B of non-reclaimed churn on a 320x240 frame is a fair
representation of what a simple embedded allocator costs.

---

## 6. CI

`cross-targets` gains three checks:

```yaml
cargo check -p qr-detect --no-default-features --features decode,micro --target thumbv7em-none-eabihf
cargo check -p qr-detect --no-default-features --features decode,micro --target riscv32imc-unknown-none-elf
cargo check -p qr-detect --no-default-features                          --target thumbv7em-none-eabihf
```

The third is the micro-off / decode-off shape, so a feature combination cannot
rot into a dead-code or missing-import failure unnoticed — it caught exactly that
during this step (the trig block was dead without `micro`).

`test` gains the two equivalence lanes from §3. `lint` gains
`cargo clippy -p qr-detect --all-targets --no-default-features --features
decode,micro -- -D warnings`.

`esp32-emulation` needed no structural change — it already builds and runs the
firmware — but its log assertions were tightened from `grep -q '^ALL PASS'` to
also require `320x240 luma frame` and `^heap peak`, so a suite that stopped being
*run* cannot pass by simply not failing.

---

## 7. Benchmarks

Same machine as the README baseline (Apple M4, rustc 1.96.0), `bench` profile,
quiesced. Medians against `benchmarks/README.md`:

| bench | baseline | now | delta |
|---|---|---|---|
| happy path: 1 v5 symbol @8px/module | 687.17 µs | 679.42 µs | **−1.1%** |
| happy path, single pass (no retry) | 688.29 µs | 685.00 µs | −0.5% |
| rotated 30 degrees | 688.04 µs | 683.75 µs | −0.6% |
| miss path: no symbol (primary + retry) | 1.12 ms | 1.14 ms | +1.8% |
| miss path, single pass (no retry) | 563.79 µs | 561.54 µs | −0.4% |
| miss path: flat frame | 320.38 µs | 321.96 µs | +0.5% |
| miss path: flat frame, single pass | 314.29 µs | 316.08 µs | +0.6% |
| detect_micro_luma: 1 M4 symbol | 760.46 µs | 760.17 µs | −0.04% |
| detect_all_luma: 1 v5 + 1 M4 | 1.12 ms | 1.12 ms | 0.0% |
| qr-video steady-state ROI frame | 324.25 µs | 322.96 µs | −0.4% |
| qr-video full-scan cadence frame | 688.25 µs | 681.38 µs | −1.0% |

Every row inside ±2%, well within the 5% gate, and the spread is symmetric —
this is run-to-run noise, not drift. Expected: on a `std` build every `mathf`
call is an `#[inline]` forward to the same `std` method the code called before,
so the emitted instructions are identical.

---

## 8. Definition of Done

| requirement | status |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo test --workspace` | ok |
| `cargo test --workspace --no-default-features` | ok |
| `cargo test -p qr-core --no-default-features` | ok |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo clippy --workspace --all-targets --no-default-features -- -D warnings` | clean |
| `cargo test -p qr-detect --no-default-features --features decode,micro` | ok, 90 tests |
| `cargo clippy -p qr-detect --all-targets --no-default-features --features decode,micro -- -D warnings` | clean |
| `RUSTFLAGS='--cfg qr_detect_soft_float' cargo test --workspace` | ok, 39 targets |
| `cargo check -p qr-detect --no-default-features --features decode,micro --target thumbv7em-none-eabihf` | ok |
| `cargo check -p qr-detect --no-default-features --features decode,micro --target riscv32imc-unknown-none-elf` | ok |
| `cargo check -p qr-detect --no-default-features --target {thumbv7em,riscv32imc}` | ok |
| `cargo check … --target wasm32-unknown-unknown` (all crates) | ok |
| esp-selftest `cargo fmt --check` + `cargo clippy --release -- -D warnings` | clean |
| QEMU `ALL PASS`, exit 0, with suite 5 | yes |
| benchmarks within 5% of README | yes, ±2% |
| zero new dependencies | yes |
| zero `unsafe` added | yes (esp-selftest's pre-existing runtime `unsafe` untouched except the new safe `heap_used`) |
| root `Cargo.toml` untouched | yes |

---

## Open questions for Richard

1. **`qr_detect_soft_float` as a `--cfg` rather than a feature.** I am confident
   this is right (features must be additive; this one changes results), but it
   does mean the escape hatch is invisible to `cargo tree` and discoverable only
   from the manifest's `check-cfg` line and `mathf.rs`. If you would rather it
   were a documented feature with a loud name, say so — it is a small change.

2. **`sqrt` at 1 ulp.** I declined the Dekker two-product correction. It is 20
   lines and would take the off-by-one-ulp rate from ~25% to ~4e-5 without
   reaching a guarantee, so the corpus verification is load-bearing either way.
   The prototype numbers are in §2 if you want to re-litigate.

3. **`REDUCTION_LIMIT` returns NaN rather than degrading.** `sin_cos(|x| > 2^20)`
   is NaN. `std` would return a poor answer instead. The pipeline never gets
   within six orders of magnitude, and NaN propagating into `try_frame` is
   rejected by the existing `is_finite` guards. I prefer the loud failure; flag
   it if you disagree.

4. **Bit-packing `Binary` (KG-22).** Declined per §4 with the numbers. If the
   roadmap has VGA-on-an-S3 in it, this becomes the highest-value single change
   available in the crate, and it wants its own step against the finder scan's
   benchmark rows.
