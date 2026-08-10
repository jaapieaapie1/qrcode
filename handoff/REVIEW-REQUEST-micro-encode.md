# Review Request — Step 5b: qr-encode Micro QR encoder

Builder: Bob-ME
Step: Wave 4, Step 5b (after 5a)
Scope: `crates/qr-encode/` only. No other crate, no shared handoff file touched.
**Ready for Review: YES**

---

## 1. Files changed

| File | Lines | What |
|---|---|---|
| `crates/qr-encode/src/micro.rs` | **new**, 432 | The whole micro encoder. Entirely behind `#[cfg(feature = "micro")]` via lib.rs. |
| `crates/qr-encode/tests/micro.rs` | **new**, 626 | 15 integration tests. File-level `#![cfg(feature = "micro")]`, so it compiles to zero tests with the feature off. |
| `crates/qr-encode/Cargo.toml` | +9 | New `[features]` block: `default = ["micro"]`, `micro = ["qr-core/micro"]`. |
| `crates/qr-encode/src/lib.rs` | +14 | `#[cfg(feature = "micro")] pub mod micro;`, three feature-gated re-exports, a `# Features` doc section. |

Nothing else. `Cargo.lock` untouched by me. **Zero edits to existing code paths** — no
existing function, type, signature or test was modified; the only changes to `lib.rs` are
additive lines.

I did not add a Builder Plan section to `handoff/ARCHITECT-BRIEF.md` as BUILDER.md asks,
because two sibling Bobs are writing to that file concurrently on 5c and 5d. The plan is
§2 below instead. This work followed a direct build dispatch.

---

## 2. Public API added

```rust
// crate root re-exports, feature-gated, parallel to encode/Options/EncodeError
pub use micro::{MicroEncodeError, MicroOptions, encode_micro};

pub mod micro {
    pub enum MicroEncodeError { DataTooLarge, UnsupportedMode, UnsupportedEcLevel, InvalidMask }
        // + Display + core::error::Error, matching EncodeError

    pub struct MicroOptions {
        pub ec: MicroEcLevel,                 // required
        pub version: Option<MicroVersion>,    // None = smallest fitting
        pub mask: Option<u8>,                 // None = highest 7.8.3.2 score
    }
    impl MicroOptions { pub fn new(ec: MicroEcLevel) -> Self }

    pub fn encode_micro(data: &[u8], options: MicroOptions) -> Result<BitMatrix, MicroEncodeError>
}
```

`MicroOptions` has exactly the three fields the brief specifies — no `mode` field, unlike
the QR `Options`. Mode is always derived, because micro mode availability is a function of
the version and a caller-forced mode would need a fourth error variant for no gain the
brief asked for.

`MicroEncodeError` is a parallel type rather than new variants on `EncodeError`, per the
Wave 4 lock. Its ranges genuinely differ (mask `0..=3` not `0..=7`, and an EC selection
that only some versions offer), so reusing `EncodeError`'s docs would have made them wrong.

Everything is `no_std` + `alloc`, no `unsafe`, no new dependency.

---

## 3. What is reused vs. what is new

**Taken from `qr_core::micro` (5a), duplicated nowhere:** `MicroVersion` (size,
`data_modules`, `mode_indicator_bits`, `terminator_bits`), `MicroEcLevel`,
`micro_symbol_info` / `micro_symbols` (Table 7/9/13 capacities), `MicroMode`
(`is_available`, `indicator`, `char_count_bits`), `micro_mask_bit`, `micro_data_modules`
(the placement iterator), `micro_format_info_encode`, `micro_format_info_write`,
`micro_is_function_module` (tests). Reed-Solomon is `qr_core::reed_solomon::rs_encode`
unchanged — micro symbols are a single block (ISO 7.6 note), so no block-splitting or
interleaving path is involved at all.

**Taken from the existing qr-encode internals:** `bits::BitBuffer` and
`segment::{mode_supports, payload_bits, write_payload}`. The per-mode payload encoding is
byte-for-byte the same in both symbologies (ISO 7.4.4–7.4.7); only the indicator width,
the count width and the availability differ, and all three come from core.

**New here, and only here:** mode/version resolution under micro availability rules, the
micro bit-stream assembly and padding, the half-codeword handling, the micro function
pattern drawing, and the ISO 7.8.3.2 mask evaluation score.

---

## 4. ISO citations for the encoder-side decisions

Every clause below was read from the PDF at repo root this session, page-targeted — none
from memory.

| Decision in code | ISO/IEC 18004:2024 source | What it says |
|---|---|---|
| mask score is **not** N1–N4 | **7.8.3.2** (p.50) | "count the number of dark modules in the right and lower edges of the symbol (excluding the final module of the timing pattern)"; `Es = S1×16+S2` if `S1 ≤ S2` else `S2×16+S1`; "The data mask pattern which results in the **highest** score shall be selected." |
| edge definition | **7.8.3.2** + **Figure 24** (p.51) | right edge = column `size-1` rows `1..=size-1` (excludes `(size-1, 0)`, the last module of the row-0 timing pattern); lower edge = row `size-1` columns `1..=size-1` (excludes `(0, size-1)`). The shared corner counts in both, as in the worked figure. |
| terminator omitted/abbreviated | **7.4.10** (p.29) | "shall be omitted if the data bit stream completely fills the capacity … or abbreviated if the remaining capacity … is less than the required bit length" |
| padding bits, then pad codewords | **7.4.11** (p.29) | zero padding bits to the codeword boundary, then `11101100`/`00010001` alternately to fill Table 7 capacity |
| M1/M3 final data codeword is 4 bits | **7.4.11** (p.29) | "All codewords are 8 bits in length, except for the final data symbol character in micro QR code versions M1 and M3 symbols, which is 4 bits" |
| M1/M3 half pad codeword is `0000` | **7.4.11** (p.29) | "The pad codeword used in the final data symbol character position … shall be represented as 0000" |
| mode availability / indicators / terminator lengths | **Table 2** (p.21) | via core |
| character count widths | **Table 3** (p.21) | via core |
| data capacities (the boundary test) | **Table 7** (p.30) | M1 5/—/—; M2-L 10/6/—; M2-M 8/5/—; M3-L 23/14/9; M3-M 18/11/7; M4-L 35/21/15; M4-M 30/18/13; M4-Q 21/13/9 (numeric/alnum/byte) |
| single RS block | **7.6** note (p.40) | "Micro QR code symbols consist of a single block" |
| finder, separator, timing geometry | **5.3.3.2, 5.3.4, 5.3.5** (p.14–15) | one 7×7 finder top-left, one-module separator, timing along row 0 and column 0 edge to edge |
| no alignment / version info / dark module | **5.3.6** (p.15) | alignment patterns are "present only in QR code symbols of version 2 or larger" |
| worked example | **Annex I.3** (p.91–92) + **Figure I.4** (p.93) | reproduced exactly — see §6 |

---

## 5. Design decisions

1. **Automatic version selection re-selects the mode per version.** Mode availability
   widens with version (ISO Table 2), so `b"ab"` has *no* legal mode in M1 or M2 and byte
   mode in M3. The loop therefore evaluates `(version, best available mode)` pairs in
   M1→M4 order and takes the first that fits, rather than picking a mode once and then
   hunting for a version. `b"AB"` lands in M2 and `b"ab"` in M3 for this reason, even
   though both are two characters.

2. **`UnsupportedMode` vs `DataTooLarge` is decided by whether any candidate version had a
   usable mode at all**, so `encode_micro(b"a", ec = Detection)` reports
   `UnsupportedMode` (byte mode does not exist in M1) rather than the misleading
   `DataTooLarge`.

3. **The M1/M3 half codeword enters Reed-Solomon zero-*prefixed*: it is a value in
   `0..=15`, high nibble clear.** MSB-first packing of a bit stream whose length is
   `data_bits` (20, 84 or 68 — all `≡ 4 mod 8`) leaves those 4 bits in the *high* nibble of
   the final byte, so `build_data_codewords` shifts the last byte down by four before the
   RS step. Only the 4 bits are written into the symbol and their module order is
   unchanged by the prefixing, which is what makes `data_bits + 8 × ec_codewords` equal
   `MicroVersion::data_modules` exactly for all eight symbols. **This was originally built
   the other way round and changed on an Arch ruling — see §8.1 for the resolution and the
   evidence that the emitted EC modules moved with it.**

4. **The mask is chosen on the symbol *before* the format information is written.** Sound
   because the format modules occupy column 8 and row 8, and the smallest symbol is 11
   modules across, so they never lie on either scored edge. Documented at the scoring
   function.

5. **Ties in the mask score go to the lowest mask id.** ISO does not specify a tie-break.
   The test asserts the rule rather than leaving it implicit, so a future refactor cannot
   silently change output.

6. **An empty payload encodes as a zero-length numeric segment**, which is legal in every
   version and keeps `encode_micro(b"", …)` from being a special error. Note the QR path's
   `segment::select_mode` maps empty to *byte* mode; micro cannot do that (byte does not
   exist in M1/M2) so `micro::select_mode` is a separate function, not a wrapper.

7. **Kanji is never selected.** ISO offers it for M3/M4 but this crate does not implement
   Kanji (existing crate-level Known Gap); `segment::mode_supports(Mode::Kanji, _)` is
   `false`, so such payloads fall through to byte mode. `qr_mode()` still maps
   `MicroMode::Kanji → Mode::Kanji` rather than being partial, so the day Kanji lands the
   micro path inherits it for free with no dead branch today.

8. **Feature polarity exactly as locked:** `default = ["micro"]`, `micro =
   ["qr-core/micro"]`. Nothing sets `default-features = false` on a sibling. The
   `# Features` doc section deliberately writes the micro names *unlinked*, because
   rustdoc emits four broken-intra-doc-link warnings under `--no-default-features`
   otherwise; there is a comment saying so.

---

## 6. Strongest verification: ISO Annex I.3 / Figure I.4, end to end

`iso_annex_i3_reproduces_figure_i4` calls

```rust
encode_micro(b"01234567", MicroOptions { ec: MicroEcLevel::L, version: None, mask: None })
```

— **fully automatic**, nothing forced — and compares all 169 modules against the symbol
printed on page 93. It passes with **0 mismatches**.

That single call independently confirms, in one shot:

- numeric mode selected over alphanumeric (I.3.1),
- version M2 selected (M1 does not offer level L),
- the I.3.2 bit stream, including the 4-bit character count and the 5-bit terminator, and
  the three padding bits — the recovered data codewords are exactly the printed
  `01000000 00011000 10101100 11000011 00000000`,
- the I.3.3 error correction codewords `10000110 00001101 00100010 10101110 00110000`,
- placement through the core iterator (I.3.4),
- **the 7.8.3.2 evaluation score picks mask reference `01`** (I.3.5) — this is the real
  proof that the edge definition and the `min×16 + max`, highest-wins formula are right,
  because getting either wrong changes the chosen mask and every masked module with it,
- format information `101000010011001` = `0x5099` (I.3.6),
- final symbol construction (I.3.7).

The test then also forces version M2, and forces mask 1, and asserts both produce the
identical matrix — so the forced paths and the automatic paths agree.

---

## 7. Test results

15 tests in `crates/qr-encode/tests/micro.rs`:

| Test | Pins |
|---|---|
| `iso_annex_i3_reproduces_figure_i4` | §6 above |
| `m1_bit_stream_and_half_codeword` | M1 `"1234"` → data codewords `[0x83, 0xDA, 0x00]` derived in the comment from Tables 2/3/7 + 7.4.10/7.4.11; EC matches `rs_encode` |
| `half_codeword_enters_reed_solomon_zero_prefixed` | the §8.1 cross-crate contract: M1 `"12345"` → `[0xA3, 0xDA, 0x0D]` with EC pinned literally as `[0x14, 0x60]`, asserted different from the zero-suffixed reading's `[0x6E, 0xC7]` |
| `m3_half_codeword_is_a_pad_codeword` | M3-L `b"hi"` → `[0x89,0xA1,0xA4,0x00,0xEC,0x11,0xEC,0x11,0xEC,0x11,0x00]`; the alternating pad codewords and the `0000` half pad |
| `payload_running_into_the_half_codeword_is_bit_padded` | 23 digits in M3-L fills the symbol exactly: terminator omitted (7.4.10) and the half codeword carries payload, not pad |
| `capacity_boundaries_match_table_7` | all 8 symbols × 3 modes: exact Table 7 capacity encodes, capacity+1 is `DataTooLarge`, and unavailable modes are `UnsupportedMode` |
| `all_four_masks_can_be_forced` | all 8 symbols × 4 masks: format info reads back the right symbol number and mask, and unmasking with it yields a stream whose EC codewords re-derive |
| `masks_produce_distinct_symbols` | the 4 masks are genuinely different symbols |
| `out_of_range_mask_is_rejected` | 4, 7, 8, 255 → `InvalidMask` |
| `automatic_mask_maximises_the_iso_7_8_3_2_score` | 8 symbols × 5 payloads: the chosen mask is the argmax of a score function transcribed independently from the clause into the test, and ties break to the lowest id |
| `automatic_version_selection_takes_the_smallest_fit` | numeric steps M2→M3→M4 at 10/23/35; 36 digits `DataTooLarge`; `Detection` pins M1; `Q` pins M4; `MicroOptions::new` equals the explicit form |
| `automatic_mode_selection_respects_availability` | alnum needs M2, byte needs M3, binary needs M3, numeric preferred for digits, empty payload fits M1 |
| `unavailable_modes_and_levels_are_rejected` | 8 invalid (version, ec) pairs from Table 13 → `UnsupportedEcLevel`; 3 unavailable-mode cases → `UnsupportedMode` |
| `function_patterns_are_correct_for_every_version` | finder ring/core, separator, timing parity per version; and the placement iterator is a bijection onto the non-function modules |
| `every_symbol_number_round_trips_through_the_format_information` | all 8 Table 13 symbol numbers |

### DoD sweep — commands run and observed results

| Command | Result |
|---|---|
| `cargo fmt -p qr-encode -- --check` | **clean** |
| `cargo test -p qr-encode` (micro ON) | **ok** — 36 tests, 0 failed (10 unit + 15 micro + 11 roundtrip) |
| `cargo test -p qr-encode --no-default-features` | **ok** — 21 tests, 0 failed; `tests/micro.rs` compiles to 0 tests |
| `cargo clippy -p qr-encode --all-targets -- -D warnings` | **clean** |
| `cargo clippy -p qr-encode --no-default-features --all-targets -- -D warnings` | **clean** |
| `cargo check --workspace --all-targets` | **ok** |
| `cargo check --workspace --no-default-features --all-targets` | **ok** |
| `cargo check -p qr-encode --target thumbv7em-none-eabihf` (both configs) | **ok** — no_std proven |
| `cargo check -p qr-encode --target riscv32imc-unknown-none-elf` (both configs) | **ok** |
| `cargo doc -p qr-encode --no-deps` (both configs) | **0 warnings** |
| negative check: `qr_encode::micro::encode_micro` with `default-features = false` | **`error[E0433]: cannot find 'micro' in 'qr_encode'`** — gating confirmed |

**Observed, not mine:** `cargo clippy --workspace --all-targets -- -D warnings` currently
fails in `crates/qr-detect/src/lib.rs:88` (`clippy::type_complexity` on
`Option<(Binary, Vec<Finder>, Vec<QrCandidate>, Vec<bool>)>`). That file is Bob-MD's
in-flight Step 5d work, uncommitted at the time of writing. Per scope lock I did not touch
it and I did not write to the shared BUILD-LOG. `cargo clippy` on qr-core, qr-encode,
qr-decode and qr-render individually is clean in both feature configs.

---

## 8. Open questions / for Richard

1. **RESOLVED — M1/M3 half-codeword nibble position.** I raised this as a cross-crate
   contract that ISO does not state for micro, having first built it zero-*suffixed* (4
   bits in the high nibble of the RS byte). **Arch ruled zero-*prefixed*: the 4-bit final
   data codeword enters Reed-Solomon as a value in `0..=15`, high nibble clear**, on the
   ISO Annex N.1 precedent ("The first 4-bit data codeword is prefixed with 0000 to make
   its length 8 bits for generating the error correction codewords"), codeword-as-value
   semantics, and the already-committed decode side (`bcafb32`), which is exhaustively
   tested on that convention. **The encoder now implements the ruling.**

   What changed: one shift in `build_data_codewords` (the packed final byte is shifted
   down by four before the RS call) and the matching 4-bit read in `build_module_stream`.
   **Module placement of the 4 data bits is unchanged** — the same four bits land in the
   same modules in the same order, because the zero prefix is never written into the
   symbol. Only the RS input byte, and therefore the EC codewords, differ.

   Evidence that the EC modules really moved, and that the tests would catch a regression
   either way: the new test `half_codeword_enters_reed_solomon_zero_prefixed` uses M1
   `"12345"`, whose half codeword is `1101` — unlike `"1234"`, whose `0000` half codeword
   is the same byte under both readings and so cannot discriminate. It pins the data
   codewords as `[0xA3, 0xDA, 0x0D]` and the error correction codewords **literally** as
   `[0x14, 0x60]`, and asserts they differ from `rs_encode([0xA3, 0xDA, 0xD0], 2)` =
   `[0x6E, 0xC7]`, which is what the zero-suffixed reading produced before the change.

   The ISO Annex I.3 / Figure I.4 reproduction (§6) is an M2 symbol, so it has no half
   codeword and still passes with 0 mismatches — the ruling did not disturb the one piece
   of printed ISO ground truth available.

2. **`place_data` indexes `stream[n / 8]` and would panic if the stream were shorter than
   the version's data-module count.** It cannot be: the length is
   `data_bits + 8 × ec_codewords == data_modules` for all eight symbols and the test
   asserts it for every one. I chose the indexing over a silent `unwrap_or(0)` because a
   broken internal invariant should be loud, but say the word and I will restructure it to
   be total.

3. **`MicroOptions` has no `mode` field**, unlike the QR `Options` (decision §2). If Arch
   wants forced-mode parity across the two APIs it is a small additive change plus one
   error variant — but it is not in the brief and I did not add it speculatively.

4. **RESOLVED — empty payloads are accepted** as a zero-length numeric segment
   (decision 5.6). I flagged the risk of an encode/decode asymmetry; Arch confirmed the
   committed decode side tolerates a zero character count. No change needed.

5. **Not in scope, no Known Gap opened:** micro detection (5d), micro decoding (5c), the
   micro integration suite (5e), Kanji, and mixed-mode segmentation (already an existing
   crate-level Known Gap for the QR path, now equally true for micro).
