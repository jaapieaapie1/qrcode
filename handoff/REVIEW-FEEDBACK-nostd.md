# Review Feedback — Step 13: `no_std` qr-detect

Date: 2026-08-11 · Reviewer: Richard-NSD
Verdict: **Step 13 CLEAR**
Counts: **0 BLOCKER · 1 MAJOR · 4 MINOR**

Everything Bob claimed, I reproduced. Every number in the review request that I
could check independently came back the same or better. The work is sound. One
thing he wrote down is not true, and it happens to be the thing protecting the
lane the whole no_std claim rests on. Fix it before the next step; it does not
block this one.

---

## BLOCKER

None.

---

## MAJOR

### MAJOR-1 — `crates/qr-detect/Cargo.toml:10-17` — the check-cfg typo guard does not exist

The manifest comment, and REVIEW-REQUEST §1, both say:

> `[lints.rust]` in the manifest declares it to `check-cfg` so a typo is still a warning.

It is not. Verified two ways:

```
RUSTFLAGS='--cfg qr_detect_soft_floats' cargo check -p qr-detect   # clean, silent, no warning
rustc --crate-type=lib --check-cfg 'cfg(qr_detect_soft_float)' \
      --cfg qr_detect_soft_floats l.rs                            # clean, silent, no warning
```

`unexpected_cfgs` / `--check-cfg` lints `cfg(...)` **predicates written in source**.
It does not lint `--cfg` **flags on the command line**. Declaring
`cfg(qr_detect_soft_float)` in `[lints.rust]` buys you exactly one thing — the
`#[cfg(qr_detect_soft_float)]` in `mathf.rs` stops being reported as unexpected.
It buys nothing at all on the invocation side.

Why this matters more than a wrong comment: a misspelled flag in the CI lane
produces a **green run that exercises nothing**. The workspace builds on `std`
floats, 39 targets pass, and the equivalence claim quietly stops being tested.
That is precisely the silent no-op Bob designed free-functions-over-a-trait to
prevent — he closed the door and left the window open. The reasoning in
`mathf.rs:26-31` is right, and it applies to the invocation too: the switch has
to be honest, and right now nothing makes it so.

Fix, two parts:

1. Correct the comment in `Cargo.toml` and §1 of the request. Say what
   `check-cfg` actually does (suppresses the source-side warning) and say
   plainly that the flag name is unguarded.
2. Give the CI lane a positive liveness assertion. A `#[cfg(qr_detect_soft_float)]`
   test in `mathf::tests` — named so it is greppable — plus a `grep -q` on the
   lane's own output for that test name. A typo then makes the grep fail
   instead of making the lane a no-op. (A test alone is not enough: under a
   typo the test simply is not compiled, and a run of zero tests still exits 0.)

**Note for the record:** the lane as currently written *is* live. I proved it —
see Ruling 4. This is about the next person to touch that line, not this diff.

---

## MINOR

### MINOR-1 — `crates/qr-detect/src/mathf.rs:48-51` — module doc contradicts the code

> "Outside the stated trig domain the Cody-Waite reduction runs out of the extra
> bits it splits `pi/2` into and **the result degrades smoothly**"

It does not degrade smoothly. `reduce` returns `None` past `REDUCTION_LIMIT` and
`sin_cos` returns `(NAN, NAN)`. That is the right behaviour, it is what
`reduce`'s own doc at `:275-278` says, it is what `trig_rejects_what_it_cannot_reduce`
asserts, and it is what REVIEW-REQUEST §2 and open question 3 argue for. Only
the module header says otherwise — and the module header is the thing a reader
lands on first. Rewrite it to say NaN.

### MINOR-2 — `handoff/BUILD-LOG.md` KG-22 — the QVGA bit-packing figure does not follow from the model

KG-22 says packing cuts QVGA `156,164 → 86,564 B` (45%). By Bob's own model:

```
76,800  frame buffer
 9,600  Binary::dark packed to bits (320*240/8)
 2,564  colsum + prefix, 4 * (2w + 1)
-------
88,964 B   -> a 43% cut, not 45%
```

The VGA figure in the same entry (`619,524 → 350,724`) is exact by that same
model — I checked it to the byte. So it is a slip on one line, not a wrong
model. Same 2,400 B in REVIEW-REQUEST §4 ("156 KB → 87 KB", "45%"). KG-22's
whole value is that a future step gets sized against an honest number; correct
it.

### MINOR-3 — `crates/qr-detect/src/lib.rs:124-140` — `--no-default-features` changed meaning, undocumented

Before this step, `qr-detect --no-default-features` dropped `decode` and `micro`
and **kept** `image` and the four `GrayImage` entry points. It now removes them
too. Nothing in the repo relied on the old shape (I checked: the workspace
`--no-default-features` clippy and test lanes are both green), and the crate is
0.1.0 and unpublished, so there is nothing to fix in code. But the `# no_std`
section documents what turning `std` *off* gives you and never says that the
old bare `--no-default-features` incantation now means something different. One
sentence.

### MINOR-4 — `esp-selftest/src/main.rs:558-566` — say what suite 5 does not prove

The frame is axis-aligned, noise-free, dead-flat 235/25, and exactly 6 px per
module. I checked the geometry and it is legitimate, not accidentally trivial:
QR v2 at 25 modules × 6 = 150 px at (24, 44); micro M3 at 15 × 6 = 90 px at
(206, 76); quiet zones 24 px left, 44 px top, 46 px bottom, 32 px between the
two symbols, 24 px right — every one of them ≥ 4 modules, and the buffer index
maxes at 61,933 of 76,800 so nothing writes out of bounds. The adaptive
threshold has real work to do. That is a proper on-target smoke test.

It is not an envelope test, and `ALL PASS` will read like one to whoever comes
next. The rotation sweep, perspective envelope and low-light ladder run on the
host — twice — and that is the right split. Say so in the suite's doc comment:
this proves the pipeline *executes* on the target and both groupers fire, and
the corpus proves it is *correct*.

---

## Rulings

**1. `--cfg` rather than a cargo feature for soft-float — UPHELD.**
Bob's reasoning is correct and I would have insisted on it if he had not. A
cargo feature is unified across the graph; a feature that changes results means
any sibling crate switching it on silently moves a consumer's arithmetic
underneath them. Features must be additive. A `--cfg` can only be set by
whoever invokes the compiler, which is exactly the right blast radius. The
discoverability cost he worries about in open question 1 is real but small —
and it is bought back by fixing MAJOR-1, which is what actually makes the switch
trustworthy.

**2. `sin_cos` returning NaN beyond `2^20` — ACCEPTED as written. No
`debug_assert`, no clamp.**
I audited all three call sites rather than taking the claim:

* `micro.rs:210` — `phi = i as f64 * TAU / RAYS`, `RAYS = 720`, `i < 720`. So
  `|phi| < 2π`. Compile-time bounded.
* `micro.rs:248` — same expression over `idx`, also bounded by `RAYS`.
* `micro.rs:976` (`rot`) — one caller, `micro.rs:170`, with
  `angle = tweak.to_radians()` where `ANGLE_TWEAKS = [0.0, -0.5, 0.5, -1.0, 1.0]`
  **degrees**. Bounded by a `const` array.

Not one argument is data-derived. There is no path from pixel data, frame size,
or a hostile image to an angle of any magnitude — the arguments are constant
expressions in every case. The domain cannot be approached, let alone exceeded,
so a `debug_assert` would assert something the type system of the call graph
already guarantees and a clamp would be dead code. Loud NaN over a quiet lie is
the correct call, and the existing `is_finite` guards in `try_frame` are belt
and braces on top. Fix the contradicting doc (MINOR-1) and this is done.

**3. `sqrt` at 1 ulp, Dekker two-product declined — UPHELD.**
Correct trade. The load-bearing claim was never "the floats agree", it is "the
detections agree", and that is verified twice against the full corpus. A pixel
coordinate carries about 1e-3 of meaning; 2.2204e-16 is nine orders below the
noise floor of the thing being measured. Twenty lines of Dekker to move a miss
rate that no downstream stage can observe is work for its own sake.

**4. Free functions rather than an extension trait — UPHELD, and verified
rather than believed.**
This is the claim that decides whether lane (b) means anything, so I did not
take it on the argument. Release asm for `qr-detect`, aarch64:

```
default std build       : 4 hardware sqrt (fsqrt d0/d1, fsqrt.2d v2/v5)
--cfg qr_detect_soft_float: 0
```

Zero. Every hardware square root vanishes and the object grows 14,737 → 16,341
lines — the Newton iteration is genuinely what gets compiled. Under a trait the
inherent `f64::sqrt` would have shadowed it, the count would not have moved, and
the lane would have been theatre. Bob called this correctly and the comment at
`mathf.rs:26-31` should stay exactly where it is.

**5. Bit-packing `Binary` declined, logged as KG-22 — UPHELD.**
Right call and right scoping. `Binary::row(y) -> &[bool]` is a deliberate
affordance for the one stage that touches every pixel, and unwinding it against
a 5% benchmark gate is a design question with a real answer that this step is
not the place to find. The numbers justify the deferral on their own. Correct
the QVGA figure per MINOR-2.

---

## What I verified

Not "as claimed" — reproduced.

| claim | result |
|---|---|
| no_std corpus lane, 90 tests | **38 + 24 + 28 = 90, all ok.** Exact. |
| soft-float workspace lane, 39 targets | **39 targets, all ok.** Also green under CI's exact `RUSTFLAGS='-D warnings --cfg qr_detect_soft_float'`. |
| genuinely `#![no_std]` | **Yes.** All three cross-target checks pass — thumbv7em ×2, riscv32imc ×1. Those targets have no `std` to leak into. No `std::` path anywhere in `crates/qr-detect/src`. |
| QEMU ALL PASS + heap | **Reproduced byte-for-byte.** `ALL PASS`, exit 0, `heap peak 163544 of 196608`, `320x240 luma frame 76800 B + pipeline 85792 B`. |
| memory formula `w·h + 4(2w+1)` | **Exact against the code.** `binarize.rs:609-611` allocates `colsum: w`, `prefix: w+1` (u32 via `fits_u32`), `dark: w*h` bools. QVGA 79,364 ✓ VGA 312,324 ✓. |
| retry doubling | **Correct.** `binarize_retry` holds `denoise()`'s `Vec<u8>` (w·h) live across `binarize_with`; the primary `Binary` never escapes `detect_qr_at`, so it is already dropped. Peak = 2wh + 4(2w+1) → 156,164 ✓ / 619,524 ✓. |
| VGA does not fit a C3 | **Correct.** 620 KB happy / 927 KB retry against a C3's ~400 KB SRAM. |
| std path byte-identical | **Confirmed.** Normalized release asm, HEAD vs branch: identical 14,737 lines; the only deltas are panic-location line numbers. No arithmetic instruction moved. |
| bench within noise of README | **686.83 µs happy-path median vs 687.17 µs baseline — −0.05%.** Every detect and micro row within ±1.5%. Bob's 679.42 µs and my 686.83 µs are both noise around the same number. |
| consumers unchanged | **Empty.** `git diff HEAD --stat` touches only `crates/qr-detect/`, `esp-selftest/`, `.github/`, `handoff/`. qr-video, demo-wasm, benchmarks, tests-integration: zero lines. |
| dead-code fix sound, not suppressed | **Sound.** Real `#[cfg(feature = "micro")]` gates on the `sin_cos` dispatch entry, the `pub(crate) use trig::sin_cos` re-export, and `mod trig` as a unit. Grepped the whole diff: **no `#[allow(...)]` added anywhere**. The code is not compiled, not silenced. |
| CI RUSTFLAGS does not clobber | **Correct.** Step-level `env` replaces the workflow-level `RUSTFLAGS: -D warnings` wholesale, and the step re-states `-D warnings` alongside the cfg. Nothing is lost. I ran the exact combined string: clean. |
| fmt / clippy / DoD lanes | `cargo fmt --all --check` clean; `clippy --workspace --all-targets` clean; `--no-default-features` clean; the new detector lane clean; `cargo test --workspace` ok; `--no-default-features` ok; wasm32 check ok; esp-selftest fmt + clippy clean. |

---

## Cleared

Reviewed the whole `no_std` conversion of `qr-detect` — feature matrix, the
`mathf` substitution and its differential tests, both equivalence lanes, the
memory audit, esp-selftest suite 5, the CI additions and the benchmark
regression. It holds. The bare-metal build is not merely checked, it is executed
against the full 90-test corpus, and the soft-float path is provably the one
that runs. Fix MAJOR-1 before Step 14 and clean up the four MINORs; none of them
change a result.
