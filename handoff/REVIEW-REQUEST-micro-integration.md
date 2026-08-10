# Review Request — Step 5e, micro integration suite

**Builder:** Bob-MI
**Scope:** Wave 4 Step 5e only — `tests-integration/` micro suites. No product crate touched.
**Ready for Review:** YES

---

## Files changed

| File | Lines | What |
|---|---|---|
| `tests-integration/tests/common/mod.rs` | +139 (new section at 276–410, import at 17–19) | Micro helper section: labels, per-version content availability, independent capacity arithmetic, independent micro zigzag walk, codeword bit ranges, codeword corruption |
| `tests-integration/tests/micro_roundtrip.rs` | 534 (new) | Suite 6 — matrix-level `encode_micro` ↔ `decode_micro` |
| `tests-integration/tests/micro_image.rs` | 437 (new) | Suite 7 — image path `encode_micro` → render → `detect_micro`/`detect_all` → decode |

`handoff/BUILD-LOG.md` deliberately **not** touched — the dispatch restricted writes to `tests-integration/` plus this file. Arch may want the Step 5e row added there.

---

## Helpers added to `common/mod.rs` (276–410)

One sentence each:

- `micro_label(info)` — `M1-detect` / `M2-L` / `M4-Q` for assertion messages.
- `micro_content_kinds(version)` — the content kinds ISO Table 2 allows per version (M1 numeric; M2 +alnum; M3/M4 +byte), so the sweeps never ask a version for a mode it does not have.
- `micro_mode(kind)` — content kind → the `MicroMode` the encoder is expected to land on.
- `micro_max_chars(mode, info)` — capacity arithmetic written from ISO 7.4.11 (indicator + char count + payload, terminator truncated at capacity, capped by the char-count indicator's own range). **Independent of `qr-encode`'s private helpers**, and it reproduces every ISO Table 7 figure — see `every_length_up_to_capacity`'s pinned case count below.
- `micro_data_module_order(version)` — the two-module-wide zigzag, written independently of `qr_core::micro::micro_data_modules` (which encoder and decoder share). `placement_walk_matches_core` pins the two together, so the shared walk now has an outside witness.
- `micro_codeword_ranges(info)` — `(bit offset, bit length)` per codeword, 4 bits wide for the final data codeword of M1/M3.
- `corrupt_micro_codeword(m, order, range)` — flips every module of one codeword.

---

## Suite 6 — `micro_roundtrip.rs` (15 tests, 0.02 s)

| Test | Coverage |
|---|---|
| `placement_walk_matches_core` | Independent zigzag == shared core walk, all 4 versions, position for position; module counts 36/80/132/192. |
| `codeword_ranges_tile_the_module_stream` | Codeword ranges tile the stream exactly for all 8 symbols — no gap, overlap or overrun. |
| `all_symbols_all_modes_all_masks` | All 8 symbols × every allowed content kind (25 combos) × {capacity-fill, short} × {auto, mask 0–3} = **250 roundtrips**, byte-exact payload + version + ec + mask + `errors_corrected == 0`. |
| `every_length_up_to_capacity` | Every length 1..=cap for every (symbol, mode) = **344 roundtrips**; the count is asserted, so a silently shrunk capacity table cannot pass by testing fewer lengths. |
| `half_codeword_versions_odd_numeric_lengths` | M1 and M3-L/M, odd numeric lengths only, all 5 mask settings — the 4-bit final codeword at every bit residue, named separately so a regression identifies itself. |
| `empty_payload_roundtrips` | Zero-length payload on all 8 symbols. |
| `one_past_capacity_is_refused` | cap+1 characters ⇒ `MicroEncodeError::DataTooLarge`, all 25 combos (capacity pinned from the other side). |
| `unavailable_modes_are_refused` | Alnum/byte on M1, byte on M2-L/M ⇒ `UnsupportedMode`. |
| `utf8_text_survives_matrix_roundtrip` | Multibyte text on M4-L: bytes **and** `text` identical. |
| `corruption_within_capacity_is_corrected` | Per correctable symbol: exactly `r` (ISO Table 9) corrupted codewords in the data region, then in the EC region ⇒ correct payload with `errors_corrected == r`. |
| `half_codeword_corruption_costs_one_symbol` | Damaging M3's 4-bit final codeword is exactly one RS symbol error. |
| `corruption_beyond_capacity_fails_cleanly` | `r + 1` corrupted codewords ⇒ `BlockCorrection`/`Malformed`, never a payload. Holds for all 7 correctable symbols, including the `p = 0` ones (M3-M, M4-M, M4-Q) where ISO does not guarantee it. |
| `m1_rejects_any_single_module_flip` | **All 36** M1 data modules, one flip each ⇒ error every time. M1 is detection-only (`r = 0`); a silent "repair" would be a misdecode. |
| `seeded_fuzz_matrix_roundtrip` | 400 seeded draws over (symbol, mode, length 0..=cap, mask incl. auto). |
| `seeded_fuzz_auto_version` | 300 seeded draws with **encoder-chosen** version; asserts the decoder agrees with whatever it picked, and that every rejection is `DataTooLarge`/`UnsupportedMode`. |

## Suite 7 — `micro_image.rs` (12 tests, 0.36 s)

Quiet zone 2 modules throughout (ISO 6.3.6), `render_gray` → `detect_micro`.

| Test | Coverage |
|---|---|
| `image_path_all_symbols_all_modes` | All 8 symbols × every allowed content kind at 8 px/module; sampled matrix compared module-for-module against the encoder's, then payload. |
| `image_path_scale_sweep_2_to_10` | All 8 symbols × 2–10 px/module (72 cases). |
| `image_path_all_four_masks` | All 8 symbols × forced masks 0–3 through the image path. |
| `rotation_sweep_0_to_45_degrees` | M2-L and M4-L, 0–45° in 5° steps, 10 px/module, 60 px pad. |
| `rotation_quarter_turns` | All 8 symbols × 0/90/180/270°, resampling-free. |
| `mild_noise` | All 8 symbols, amplitude 45 (the QR suite's calibration) — including detection-only M1, which tolerates zero flipped modules. |
| `heavy_noise_m4_q` | M4-Q at amplitude 70. |
| `qr_and_micro_in_one_image` | One v-auto QR + one micro in one frame via `detect_all`: exactly 1 + 1, both payloads correct. No finder cross-stealing in either direction. |
| `two_micros_in_one_image` | Two auto-version micros (7-char alnum payloads land on M3-L and M3-M) coexisting, both decoded to their own payloads. |
| `v1_l_three_codeword_errors_never_misdecode` | **r-cap regression (6174b10) at integration level:** v1-L, 3 corrupted codewords, full image path ⇒ detection succeeds and the sampled matrix matches, but `decoded` is `None`. Paired with a clean-symbol control through the identical path. |
| `m1_single_module_damage_detects_but_does_not_decode` | The micro counterpart: one flipped module ⇒ detected, matrix matches, `decoded` is `None`. |
| `m1_smallest_symbol_image_path` | Every M1 numeric length 1–5 through the image path (11×11 modules). |

---

## Bugs found

**None.** No product-crate defect surfaced; no `#[ignore]` pins were needed. Both suites were green on essentially the first run — the two failures I hit were mine (an over-long UTF-8 fixture, a `wrapping_mul` on an ambiguous integer literal).

Two independent cross-checks that could have caught a table or placement error and did not fire:

- The independently written zigzag matches `micro_data_modules` exactly for all four versions.
- The independently written capacity arithmetic reproduces ISO Table 7 exactly — the pinned 344-case total was computed by hand from the published capacities before the test was run, and matched first time.

## Envelope measured (documented honestly, not asserted as a promise)

- **Scale floor: 2 px/module** for an integer-scaled, noise-free render with a 2-module quiet zone. I probed 1 px/module explicitly: the detector finds nothing (an M1 symbol plus quiet zone is 15 px square at that scale). The sweep therefore starts at 2 and the doc comment says why.
- **Rotation:** 0–45° in 5° steps passes for M2 and M4 at 10 px/module with a 60 px pad; exact quarter turns pass for all 8 symbols. I did not probe past 45° — beyond that the symbol is better described by the next quarter turn.

## Runtimes (debug profile, macOS)

| Target | Wall |
|---|---|
| `micro_roundtrip` (15 tests) | 0.02 s |
| `micro_image` (12 tests) | 0.36 s |
| `cargo test -p tests-integration` (81 tests, 9 binaries) | 28.5 s incl. compile; `matrix_roundtrip` at 17.6 s dominates |
| `cargo test --workspace` | all green |

The micro suites are ~1 % of integration-suite runtime; the sweeps are sampled where the cost would explode (rotation on two versions, not eight) and exhaustive where it is free (matrix-level lengths and masks).

## DoD

- [x] `cargo test -p tests-integration` — 81 passed, 0 failed, **0 ignored**
- [x] `cargo test --workspace` — all green
- [x] `cargo clippy --workspace --all-targets -- -D warnings` — clean
- [x] `cargo fmt --all --check` — clean
- [x] `cargo check --workspace --no-default-features --tests` — clean (unchanged behaviour: `tests-integration`'s dependency edges request default features, so the micro helpers still resolve)

## Open questions for Richard / Arch

1. `corruption_beyond_capacity_fails_cleanly` asserts a hard `Err` for the three symbols where ISO reserves no misdecode-protection codewords (`p = 0`: M3-M, M4-M, M4-Q). It holds for these specific deterministic corruptions, but ISO only guarantees detection where `p > 0`. If you would rather not pin behaviour the standard does not promise, say so and I will split those three into a weaker assertion (must not return the original payload) — but I would rather keep the strong form as a canary.
2. `common/mod.rs` now imports `qr_core::micro` unconditionally. `tests-integration` has no feature flags of its own and nothing in the workspace turns micro off for it, so this is fine today. If Step 6 CI ever adds a genuinely micro-less configuration for the test harness, these two suites and the helper section need `#[cfg]` gating or a `micro` feature on `tests-integration`.
3. Step 5e's DoD line in the brief is "full workspace green", which is met. `handoff/BUILD-LOG.md` still needs its Step 5e row — left to Arch per the write restriction on this dispatch.
