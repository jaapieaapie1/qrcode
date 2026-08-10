# Review Feedback — Step 3b: `tests-integration`

**Reviewer:** Richard-I
**Date:** 2026-08-10
**Verdict:** **Step 3b CLEAR**
**Ready for Builder:** YES
**Counts:** 0 BLOCKER · 1 MAJOR · 6 MINOR

Scope reviewed: `tests-integration/` only. `qr-detect` source was not read or
judged; CI-1/CI-2 are accepted findings.

---

## Verification performed

The working tree's `qr-detect` is mid-fix and does not compile, so I verified
against a detached `git worktree` at HEAD (`79bc7db`) in scratch, then removed it.
Nothing in the working tree was touched.

```
cargo test  -p tests-integration      50 passed, 2 ignored, 0 failed   (27.4 s wall)
cargo clippy -p tests-integration --all-targets -- -D warnings   clean
```

Per-suite, matching the REVIEW-REQUEST table within noise:

| suite | tests | wall | claimed |
|---|---:|---:|---:|
| `full_roundtrip` | 11 | 1.05 s | 1.1 s |
| `matrix_roundtrip` | 11 | 18.29 s | 18.5 s |
| `warped` | 14 + 2 ign | 0.20 s | 0.2 s |
| `video_session` | 9 | 0.20 s | 0.2 s |
| `fuzz_seeded` | 5 | 5.80 s | 5.8 s |

Counts and runtimes are accurate as stated.

### Annex I values — verified against the PDF, Bob is right

Extracted `ISO_IEC_18004_2024(en).pdf` §I.2.3 (printed p. 89) directly. The
published 26-codeword stream ends:

```
10100101 00100100 11010100 11000001 11101101 00110110 11000111 10000111 00101100 01010101
```

= `[0xA5,0x24,0xD4,0xC1,0xED,0x36,0xC7,0x87,0x2C,0x55]`. `ANNEX_I_EC` transcribes
this exactly. `ANNEX_I_DATA` also matches I.2.2 byte for byte
(`00010000 00100000 00001100 01010110 01100001 10000000` + 10 pad codewords).
The ten values quoted in the Step 3b dispatch are wrong; Bob's correction stands.
KG-3 is genuinely closed. Note in passing that the standard contradicts itself —
I.2.5 says mask `010`, I.2.6 says `011`; the test pins `010`, which agrees with
I.2.5 and Figure I.2. Correct call.

### Bug-pin logic — verified empirically, not just by reading

While the concurrent `qr-detect` fix was linked in, I ran the already-built
`warped` binary:

- `cargo test --ignored` → **both** `rotated_30_degrees` and
  `single_rotated_symbol_yields_single_candidate` **pass**.
- `rotation_dimension_underestimate_is_still_present` → **fails loudly** with
  `"CI-1 appears fixed: sampled dimension 37 now matches the true 37 — drop this
  test and un-ignore rotated_30_degrees"`.

That is exactly the post-fix contract Bob specified. The CI-1 pin is correctly
encoded and cannot be fixed silently. See MAJOR-1 for what it does *not* cover.

---

## Must Fix (BLOCKER)

None.

---

## Should Fix (MAJOR)

### MAJOR-1 — CI-2's `#[ignore]` has no "bug still present" companion

`tests-integration/tests/warped.rs:93` — `single_rotated_symbol_yields_single_candidate`
is `#[ignore]`d with no counterpart that fails once CI-2 is fixed. CI-1 has one
(`rotation_dimension_underestimate_is_still_present`, line 110); CI-2 has none.

I confirmed this is not theoretical: with the in-flight `qr-detect` fix linked,
the CI-2 test already passes under `--ignored`, while nothing in the suite fails
to force the un-ignore. The CI-1 companion's failure message names only
`rotated_30_degrees`, so a fixer following it verbatim leaves the CI-2 test
skipped forever and the phantom-candidate coverage is silently lost.

Fix: add a companion asserting the phantom is still emitted — e.g. at the
documented repro (v7-M, 10 px/module, 200 px pad, 20°) assert
`qr_detect::detect(...).len() > 1` with a message pointing at
`single_rotated_symbol_yields_single_candidate`. Alternatively, extend the CI-1
companion's message to name both ignored tests. This wants to land in the same
commit as the `qr-detect` fix, not after it.

**Operational note for Arch (not a defect):** once the `qr-detect` fix lands,
`cargo test -p tests-integration` goes RED on
`rotation_dimension_underestimate_is_still_present` until `warped.rs` is edited.
That is by design, but the two changes must be sequenced together or CI will
break on a green fix.

---

## Should Fix (MINOR)

### MINOR-1 — the "independent zigzag" is a paraphrase, not an independent derivation

`tests-integration/tests/common/mod.rs:187-229` vs
`crates/qr-decode/src/extract.rs:98-127`. Same loop skeleton throughout
(`while right >= 1`, `if right == 6 { right = 5 }`, inner `vert` × `j` loops,
`is_function_module` guard, `mask_bit` un-mask, `right -= 2`). The only genuinely
different expression is the up/down rule (`upward` toggle vs `(right + 1) & 2`).
It also shares `is_function_module` and `mask_bit` with the code under test.

This does not invalidate the test — its ground truth is the ISO codeword stream,
not the decoder, so KG-2 *is* closed — but the REVIEW-REQUEST's "independently
written" framing oversells it. Consequence worth recording: `mask_bit` is applied
by the encoder and removed by the walk, so it **cancels exactly**. The Annex I
test therefore validates *module placement* against ISO, but **not the mask
pattern itself** — a wrong `mask_bit` would be invisible here. Mask correctness
still rests on `qr-core`'s own tests. Worth one line in the test's doc comment so
nobody over-reads the guarantee later.

### MINOR-2 — "capacity-filled" is asserted nowhere

`common/mod.rs:155` `max_chars` is the tests' own capacity model, and no test
checks a symbol is actually full. If `max_chars` ever under-computes, every sweep
in Suites 1, 2 and 5 silently weakens to short payloads and still passes green.

I spot-checked the model against the ISO capacity tables at three points across
the range — v1-M numeric = 34, v1-H alphanumeric = 10, v40-L byte = 2953 — and it
is exact today. Cheap guard: in `sweep`, assert `encode(&payload(kind, len + 1), opts)`
fails, at one version per EC level.

### MINOR-3 — silent-skip guards are the "silently sample" mechanism

`matrix_roundtrip.rs:38` (`if len == 0 { continue }`), `:105`
(`if data.is_empty() { continue }`), `full_roundtrip.rs:38` (`if len == 0 { return }`),
`fuzz_seeded.rs:22` (`if cap == 0 { return }`). None of these fire today —
`max_chars` is non-zero for every version/EC/mode pair — so the v1–40 × 4 EC
sweep is genuinely exhaustive, not sampled. But these are exactly the constructs
that would turn an exhaustive sweep into a silent no-op after an unrelated change.
Prefer `assert!(len > 0, ...)`.

### MINOR-4 — cadence tests' doc comments describe coverage that does not exist

`video_session.rs:257` claims the cadence refresh means "a second code appearing
outside the tracked box is still found", and `:284-285` claims "with the cadence
disabled a code that appears outside the ROI is missed on that frame (and only
recovered by the miss-fallback), whereas the refresh frame sees it directly".
Neither test contains a second code or a code outside the ROI — both drive a
single stationary code at `(60, 40)` and assert scan counters only.

The counters do exercise the branch, so KG-5's letter is met and I am not
reopening it. But the prose promises behavioural coverage that is not there, and
a future maintainer will trust it. Either trim the comments to what is asserted,
or add the second-code case they describe.

### MINOR-5 — downscale aliases rather than blurs (honest, but not camera-like)

`common/mod.rs:397` `rescale` inverse-maps with bilinear sampling and no area
prefilter, so a 0.25 factor reads a 2×2 neighbourhood out of every 4×4 source
block. That is decimation with aliasing, not optical blur. Checked deliberately
because accidental smoothing would make detection artificially easy — this errs
the *other* way, so the fixtures are harsh, not soft. No change required; noting
that "2.5 px/module" here is a harder input than a real camera would produce, and
a future area-averaged variant would be a different (also useful) test.

`rotate` (`:359`) and `perspective` (`:414`) are correct inverse-mapped bilinear
warps — the rotation matrix at `:368` is the proper inverse, the homography at
`:434` is solved dst→src with partial pivoting, so neither can produce holes.
`add_noise` (`:490`) is fully deterministic: seeded xorshift64*, row-major, sum of
four uniforms. Fixture honesty is sound.

### MINOR-6 — `Content::Utf8` degenerates to ASCII at small lengths

`common/mod.rs:125` `utf8_payload` breaks before the first 2-byte code point when
`n` is small and tops up with `b'x'`, so `payload(Content::Utf8, 3)` is `"eox"` —
pure ASCII. `short_payloads_pad_correctly` and low-`len` fuzz iterations labelled
"utf8" therefore carry no multibyte content. Harmless (the capacity-filled sweeps
and `utf8_text_survives_image_path` cover the real case), but the label overstates
what those particular cases exercise.

---

## Escalate to Architect

1. **MAJOR-1 sequencing.** The CI-2 companion and the `qr-detect` fix need to land
   together. Arch should decide who owns that edit — it is in Bob-I's file but
   triggered by Bob-D's work.
2. **Bob's open question 1 (CI-1 severity).** Agreed with Bob: a v20 symbol
   failing past 12° of rotation is a shipping defect for the camera path, not a
   Known Gap. Already being fixed concurrently — Arch should confirm KG-8 is
   opened as a Step 4 item rather than logged and left.
3. **Bob's open question 2 (`errors_corrected` semantics).** Support the doc note.
   The suite now pins `errors_corrected == 12` and `== 8` as exact values; that
   makes the "count of corrupted codeword positions across all blocks" reading
   contractual whether or not it is documented. Better to write it down.
4. **Bob's open question 3 (runtime budget).** 27.4 s wall, of which
   `matrix_roundtrip` is 18.3 s (67 %) — that is 800+ debug-build encode/decode
   pairs already split across four parallel test functions, so it is inherent, not
   waste. No test dominates unnecessarily. Recommend leaving it as-is and adding
   `--release` to CI if it ever becomes a problem; do **not** hide the v1–40
   sweeps behind `--ignored`, that is the strongest coverage in the crate.

---

## Cleared

Reviewed all five suites plus `common/mod.rs` against the Step 3b brief
(`handoff/ARCHITECT-BRIEF.md:66-67`) and the four Known Gaps.

- All five suites present; brief's required coverage (encode→render→rasterize→
  detect→decode across versions/levels/masks/content incl. binary and UTF-8, plus
  a seeded fuzz loop) is fully met and exceeded.
- Roundtrips assert byte-exact payload equality throughout — `decoded.bytes == data`
  in every suite, never a bare `is_ok()`. Suite 1 additionally asserts the
  detector's sampled `BitMatrix` equals the encoder's module-for-module, plus
  version/EC/mask readback and `errors_corrected == 0`.
- The v1–40 × {L,M,Q,H} sweep is genuinely exhaustive (640 pairs per content kind,
  four kinds), not sampled. Confirmed by reading the loop bounds and by confirming
  the skip guards never fire.
- **KG-1 closed** — v5-Q, 4 RS blocks, damage proven to span ≥ 2 blocks with ≤ t
  each via an independent ISO 7.6 interleaving model, exact `errors_corrected`
  pinned, plus EC-region and beyond-capacity cases.
- **KG-2 closed** — placement geometry validated against the ISO Annex I codeword
  stream rather than the decoder's reader. See MINOR-1 for the precise scope of
  the guarantee.
- **KG-3 closed** — EC codewords pinned to the values I read out of the ISO PDF
  myself.
- **KG-5 closed** — border clamping at both corners, partial-symbol handling,
  cadence 3 and cadence 0, ROI-miss fallback, malformed-frame survival. See
  MINOR-4 for a doc/coverage mismatch that does not reopen the gap.

Nothing here ships broken. Fix MAJOR-1 alongside the `qr-detect` change and the
minors at leisure.
