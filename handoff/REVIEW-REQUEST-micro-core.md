# Review Request — Step 5a: qr-core Micro QR foundation

Builder: Bob-M
Step: Wave 4, Step 5a (blocks 5b–5d)
Scope: `crates/qr-core/` only.
**Ready for Review: YES**

---

## 1. Files changed

| File | Lines | What |
|---|---|---|
| `crates/qr-core/src/micro.rs` | **new**, 734 | All Micro QR primitives. The whole module is behind `#[cfg(feature = "micro")]` via lib.rs. |
| `crates/qr-core/tests/micro.rs` | **new**, 707 | 14 integration tests. File-level `#![cfg(feature = "micro")]` so it compiles to zero tests with the feature off. |
| `crates/qr-core/Cargo.toml` | +10 | New `[features]` block: `default = ["micro"]`, `micro = []`. |
| `crates/qr-core/src/lib.rs` | +11 | `#[cfg(feature = "micro")] pub mod micro;` + a `# Features` doc section. No change to any existing `pub use`. |
| `crates/qr-core/src/bch.rs` | +5 / ~1 | `bch_remainder` private → `pub(crate)`; new `#[cfg(feature = "micro")] pub(crate) const FORMAT_GEN_POLY`. |
| `handoff/ARCHITECT-BRIEF.md` | +25 | Builder Plan section (per BUILDER.md). |

`Cargo.lock` untouched. No other crate touched.

### The only edit to pre-existing code

`bch.rs`: `fn bch_remainder` → `pub(crate) fn bch_remainder`, plus a `pub(crate)` alias for the
generator polynomial. **Rationale:** ISO Annex C.2 specifies *the same* `(15, 5)` BCH code with *the
same* generator `G(x) = x¹⁰+x⁸+x⁵+x⁴+x²+x+1` for both symbologies — only the XOR mask differs
(`0x5412` vs `0x4445`). Duplicating the divider in `micro.rs` would have been the alternative and is
strictly worse. `pub(crate)` does not alter the public API surface. The const is feature-gated so it
cannot become dead code when `micro` is off.

---

## 2. Public API added (all under `qr_core::micro`)

```rust
// Constants
pub const MICRO_FORMAT_MASK: u16;                             // 0x4445
pub const MICRO_MASK_TO_QR: [u8; 4];                          // [1, 4, 6, 7]
pub const MICRO_FORMAT_INFO_POSITIONS: [(u32, u32); 15];      // indexed by bit number, 0 = LSB

// Types (all parallel — no existing enum gained a variant)
pub enum MicroVersion { M1, M2, M3, M4 }
    ALL, number, from_number, size, from_size, total_codewords,
    data_modules, mode_indicator_bits, terminator_bits, ec_levels
pub enum MicroEcLevel { Detection, L, M, Q }
    ALL, ec_level() -> Option<EcLevel>, from_ec_level(EcLevel) -> Option<Self>
pub struct MicroSymbolInfo { version, ec, symbol_number, total_codewords,
                             data_codewords, ec_codewords, data_bits,
                             error_correction_capacity, misdecode_protection }
    final_data_codeword_bits(), has_half_final_data_codeword()
pub enum MicroMode { Numeric, Alphanumeric, Byte, Kanji }
    ALL, is_available, indicator -> Option<(value, bits)>, char_count_bits
pub struct MicroDataModules;  // Iterator<Item = (u32, u32)>, allocation-free

// Functions
pub fn micro_symbol_info(MicroVersion, MicroEcLevel) -> Option<MicroSymbolInfo>
pub fn micro_symbol_info_from_number(u8) -> Option<MicroSymbolInfo>
pub fn micro_symbols() -> &'static [MicroSymbolInfo; 8]
pub fn micro_mask_bit(mask_id: u8, x: u32, y: u32) -> bool
pub fn micro_format_info_encode(MicroVersion, MicroEcLevel, mask: u8) -> Option<u16>
pub fn micro_format_info_decode(raw: u16) -> Option<(MicroSymbolInfo, u8)>
pub fn micro_format_info_read(&BitMatrix) -> u16
pub fn micro_format_info_write(&mut BitMatrix, format: u16)
pub fn micro_is_function_module(MicroVersion, x: u32, y: u32) -> bool
pub fn micro_data_modules(MicroVersion) -> MicroDataModules
```

Coordinates are `(x, y)` = (column, row), origin top-left, documented on every coordinate-taking item.
No `unsafe`. No panics on malformed input — out-of-range inputs return `None` / `false`.

---

## 3. ISO citations — every table and constant, per source

The PDF was read page-targeted. Where a value lives in a *figure* rather than text, the page was
rasterised at 150–600 dpi and read visually or decoded programmatically; that is noted below.

| Item in code | ISO/IEC 18004:2024 source | Value verified |
|---|---|---|
| `MicroVersion::size` | **5.3.2.2** (p.13) + **Table 1** col A (p.17) | 11 / 13 / 15 / 17, i.e. `9 + 2n` |
| `MicroVersion::total_codewords` | **Table 1** col E (p.17) | 5 / 10 / 17 / 24 |
| `MicroVersion::data_modules` | **Table 1** col D (p.17), `D = A² − B − C` | 36 / 80 / 132 / 192 |
| function-pattern module count (test only) | **Table 1** col B (p.17) | 70 / 74 / 78 / 82 |
| format-info module count | **Table 1** col C (p.17) | 15 for all four |
| remainder bits = 0 | **Table 1** last col (p.17) | 0 for all four — codeword stream fills the symbol exactly |
| `MicroVersion::mode_indicator_bits` | **Table 2** (p.21) | 0 / 1 / 2 / 3 |
| `MicroMode::indicator` values | **Table 2** (p.21) | M2: N=0, A=1. M3: N=00, A=01, B=10, K=11. M4: N=000, A=001, B=010, K=011 |
| mode availability (the "NA" cells) | **Table 2** (p.21), **7.4.5 / 7.4.6 / 7.4.7** (p.19) | M1 numeric only; byte and Kanji absent in M1 and M2 |
| `MicroVersion::terminator_bits` | **Table 2** last row (p.21) + **7.4.10** (p.29) | 3 / 5 / 7 / 9 zero bits |
| `MicroMode::char_count_bits` | **Table 3** (p.21) | M1: 3. M2: 4/3. M3: 5/4/4/3. M4: 6/5/5/4 |
| `MicroSymbolInfo::data_codewords`, `data_bits` | **Table 7** (p.30) | (3,20) (5,40) (4,32) (11,84) (9,68) (16,128) (14,112) (10,80) |
| `total_codewords`, `ec_codewords`, `error_correction_capacity` (`r`) | **Table 9** (p.34), blocks `(c,k,r)` | M1 (5,3,0); M2-L (10,5,1); M2-M (10,4,2); M3-L (17,11,2); M3-M (17,9,4); M4-L (24,16,3); M4-M (24,14,5); M4-Q (24,10,7) |
| `misdecode_protection` (`p`) | **7.5.1** (p.34), the `p =` bullet list | p=3: M2-L. p=2: M1, M2-M, M3-L, M4-L. p=0: rest |
| single RS block per symbol | note under **7.6** (p.40) | "Micro QR code symbols consist of a single block" |
| level H unavailable | **7.5.1** (p.33) | "error correction level H is not available in micro QR code symbols" |
| M1 = detection only | **Table 9** (p.34) `(5,3,0)`; **Table 13** (p.53) | correction capacity r = 0 |
| 4-bit final data codeword (M1/M3) | **7.4.11** (p.29) | "All codewords are 8 bits in length, except for the final data symbol character in micro QR code versions M1 and M3 symbols, which is 4 bits" |
| M1/M3 pad half-codeword = `0000` | **7.4.11** (p.29) | relevant to 5b, recorded in doc comments |
| the 4 micro data masks | **Table 10** (p.46) | micro `00`→QR `001`; `01`→`100`; `10`→`110`; `11`→`111` |
| `MICRO_FORMAT_MASK` | **7.9.2** (p.53) and **Annex C.2** (p.74) | `100010001000101` = `0x4445` |
| BCH generator shared with QR | **Annex C.2** (p.74) | `G(x) = x¹⁰+x⁸+x⁵+x⁴+x²+x+1` — identical for both symbologies |
| 3-bit symbol number + 2-bit mask | **7.9.2** (p.52–53) | first three data bits = symbol number, fourth and fifth = mask reference |
| symbol-number ↔ (version, ec) table | **Table 13** (p.53) | 0=M1-detect, 1=M2-L, 2=M2-M, 3=M3-L, 4=M3-M, 5=M4-L, 6=M4-M, 7=M4-Q |
| all 32 format-info words | **Table C.1** (p.75), micro column | transcribed into the test as `TABLE_C1_MICRO`; independently re-derived by BCH computation and matched 32/32 |
| ≤3-bit correction limit | **Annex C.3** (p.74) | "Hamming distance … is 7, which enables up to 3 bit errors to be corrected" |
| `MICRO_FORMAT_INFO_POSITIONS` | **Figure 26** (p.53) — *rasterised at 150 dpi and read* | bits 0–7 down column 8, rows 1→8; bits 8–14 along row 8, columns 7→1 |
| single finder, top-left | **5.3.3.2** (p.14), **Figure 4** (p.7) — *rasterised* | one 7×7 finder, ratio 1:1:3:1:1 |
| separator | **5.3.4** (p.15) | one module wide, row 7 and column 7 |
| timing patterns | **5.3.5** (p.15) | "horizontal timing pattern runs across **row 0** … the vertical timing pattern similarly runs down **column 0**", edge to edge |
| no alignment patterns | **5.3.6** (p.15) | "present only in QR code symbols of version 2 or larger" |
| quiet zone 2X (informational) | **5.3.8** (p.15) | noted in module docs; rendering is not this step |
| placement rules | **7.7.3** (p.44–45) | "Exactly the same principles apply to a micro QR code symbol" + the 2-wide alternating-column bit-stream method of p.44 |
| half codeword is a 2×2 block | **7.7.3** (p.45) | "D3 in a version M1 symbol, D11 in a version M3-L symbol and D9 in a version M3-M symbol is a 2 × 2 square 4-module block" |
| end-to-end worked example | **Annex I.3** (p.91–92) + **Figure I.4** (p.93) — *rasterised at 600 dpi and decoded to a module grid* | see §5 |

**Not implemented here, deliberately** (all cited so 5b/5c/5d inherit the references):

- Micro mask-selection score (`S1`/`S2` edge counting, `Es = min×16 + max`, **highest** wins) —
  **7.8.3.2** (p.50) + **Figure 24** (p.51). This is qr-encode's job per the brief (Step 5b), exactly as
  the QR N1–N4 penalty lives in qr-encode and not in qr-core. I wrote it, then removed it to hold scope.
- ECI / FNC1 / structured append are unavailable in micro — **7.4.3** (p.18), **7.4.9** (p.20), **8.1** (p.55).

---

## 4. Design decisions

1. **`MicroEcLevel` is a new parallel enum, not `EcLevel`.** The brief said "allowed `EcLevel`
   subsets", but ISO Table 13 symbol 0 is *"Error detection only"* — M1 has no error correction level
   at all, and level H never applies. Adding a `Detection` variant to the shared `EcLevel` is
   forbidden by the Wave 4 lock ("no new variants on existing public enums"), and `Option<EcLevel>`
   read badly at every call site. `MicroEcLevel { Detection, L, M, Q }` with
   `ec_level()`/`from_ec_level()` bridges is the result. **This is a deviation from the brief's
   wording — flagging for Arch.**

2. **`micro_data_modules()` is the single source of truth for placement**, as mandated. It is an
   allocation-free `Iterator`, so it costs nothing on the no_std targets and both the encoder (5b)
   and decoder (5c) walk the identical order. It takes only `MicroVersion` — module *order* does not
   depend on the EC level. (ISO Figure 11 shows two M3 layouts "which differ only in the codeword
   placement according to the error correction level"; that difference is purely *where in the bit
   stream* the half codeword falls — D11 for M3-L, D9 for M3-M — not which module comes next. Verified:
   both fall on a 2×2 block under one shared order.)

3. **Micro masks forward to `mask_bit`** via `MICRO_MASK_TO_QR`, because ISO Table 10 lists the micro
   references against the *same* condition column as the QR references. No duplicated formulae.

4. **`micro_format_info_read`/`_write` are included** even though the brief only said "placement
   coordinates". Two callers deriving bit order from a coordinate array independently is exactly how
   encode and decode drift apart; one write and one read function makes that impossible.

5. **`micro_is_function_module` includes the format-information area**, matching the existing
   `is_function_module`, which also treats the format band as reserved. The test asserts columns B
   and C of ISO Table 1 *separately* so the two are not conflated.

6. **Feature polarity** exactly as locked: `default = ["micro"]`, `micro = []`, opt out with
   `--no-default-features`. Nothing sets `default-features = false` on a sibling. Siblings do not yet
   forward the feature — that lands with 5b/5c/5d (`micro = ["qr-core/micro"]`).

---

## 5. The strongest verification: ISO Annex I.3 / Figure I.4

`iso_annex_i3_reproduces_figure_i4` rebuilds the standard's own worked example — `"01234567"` in an
M2-L symbol — from core primitives and compares it **module for module** against the printed final
symbol.

How the reference grid was obtained: page 93 rendered to PGM at 600 dpi, the symbol's bounding box
located by dark-pixel profile (637 × 633 px), divided into a 13 × 13 grid and each cell sampled at its
centre. That grid is pasted into the test as `FIGURE_I4`.

The test then checks, in order:

- `rs_encode([0x40,0x18,0xAC,0xC3,0x00], 5)` → `[0x86,0x0D,0x22,0xAE,0x30]`, byte-identical to the
  codewords printed in **I.3.3** — confirming the shared Reed-Solomon path needs no micro-specific work.
- placement of all 10 codewords through `micro_data_modules`, MSB first,
- XOR with `micro_mask_bit(1, …)` — **I.3.5** selects micro mask reference `01`,
- `micro_format_info_encode(M2, L, 1)` → `0x5099`, matching the `101000010011001` printed in **I.3.6**,
- written at `MICRO_FORMAT_INFO_POSITIONS`,
- **all 169 modules compared against Figure I.4 — 0 mismatches**,
- then read back: `micro_format_info_decode(micro_format_info_read(&m))` recovers M2-L + mask 1, and
  re-walking the placement iterator with the same mask returns the original 10 codewords (the exact
  property Step 5c will depend on).

This single test pins the placement order, the mask mapping, the format-info value *and* the format-info
bit positions against printed ISO output rather than against my own reading of the prose.

---

## 6. Test results

14 new tests in `crates/qr-core/tests/micro.rs`:

| Test | Pins |
|---|---|
| `version_sizes_match_iso_5_3_2_2` | sizes, `from_size`/`from_number` round-trips, rejection of QR sizes |
| `table_1_module_accounting` | Table 1 cols B, C, D, E — counted from the function map, not restated |
| `table_13_symbol_numbers` | all 8 triples; every unlisted (version, ec) pair rejected; H rejected |
| `table_7_and_table_9_capacities` | `c = k + ec`; `data_bits` vs codeword widths incl. the half codeword; stream fills `data_modules` exactly; `2t + p ≤ d` (7.5.1 Formula 3) |
| `table_2_mode_indicators_and_terminators` | indicator values/widths, NA cells, terminator lengths, indicator fits its width |
| `table_3_character_count_indicator_widths` | full 4 × 4 grid incl. NA |
| `table_10_micro_data_masks` | 1600 coordinates × 4 masks against the formulae transcribed verbatim from Table 10 |
| `annex_c1_micro_format_information_table` | all 32 words vs Table C.1; distinctness; non-zero; range rejection |
| `annex_c3_format_information_error_correction` | every valid word × every 1-, 2- and 3-bit corruption → **15 968 corruptions, all corrected** |
| `figure_26_format_information_positions` | bit→coordinate map; all 15 reported reserved; read/write inverse |
| `micro_function_map_regions` | every module of every version against the three regions; out-of-range is `false`; M4 centre is data (no alignment pattern) |
| `placement_iterator_covers_every_data_module_once` | count == Table 1 col D; bijection with the non-function modules; starts bottom-right |
| `half_codeword_forms_a_two_by_two_block` | D3 / D11 / D9 are 2×2 squares (7.7.3) |
| `iso_annex_i3_reproduces_figure_i4` | §5 above |

### DoD sweep — commands run and observed results

| Command | Result |
|---|---|
| `cargo fmt -p qr-core -- --check` | **clean** (KG-10 discipline held this time) |
| `cargo test -p qr-core` (micro ON) | **ok** — 40 tests, 0 failed (14 micro + 26 pre-existing) |
| `cargo test -p qr-core --no-default-features` (micro OFF) | **ok** — 26 tests, 0 failed; `tests/micro.rs` compiles to 0 tests |
| `cargo check --workspace --all-targets` | **ok** |
| `cargo check --workspace --no-default-features --all-targets` | **ok** |
| `cargo check -p qr-core --no-default-features --all-targets` | **ok** |
| `cargo clippy -p qr-core --all-targets -- -D warnings` | **clean** |
| `cargo clippy -p qr-core --no-default-features --all-targets -- -D warnings` | **clean** |
| `cargo clippy --workspace --all-targets -- -D warnings` | **clean** |
| `cargo clippy --workspace --no-default-features --all-targets -- -D warnings` | **clean** |
| `cargo check -p qr-core --target thumbv7em-none-eabihf` (both configs) | **ok** — no_std proven, not merely asserted |
| `cargo check -p qr-core --target riscv32imc-unknown-none-elf` (both configs) | **ok** |
| negative check: `qr_core::micro::…` with `--no-default-features` | **`error[E0433]: cannot find 'micro' in 'qr_core'`** — gating confirmed |

One `#[allow]` was added: `clippy::manual_is_multiple_of` on `table_10_micro_data_masks`, with a
comment. The mask conditions are transcribed in ISO's own `mod` notation on purpose — rewriting them
as `is_multiple_of` would make the test textually identical to `mask.rs` and destroy its independence.

---

## 7. Open questions / for Richard

1. **`MicroEcLevel` vs the brief's "allowed `EcLevel` subsets"** — decision 4.1 above. Needs an Arch
   ack, since 5b/5c/5d will take it in their signatures.
2. **Micro mask-selection score is deliberately absent from core.** If Arch would rather it be shared
   (5d might want it to rank candidate reads), say so and I will move it in — it is ~15 lines and I
   have the 7.8.3.2 / Figure 24 references ready. Left in qr-encode for now, by symmetry with N1–N4.
3. **`bch.rs` visibility widening** is the one edit to existing code. Called out for explicit approval.
4. **M1 decode semantics** are recorded but not enforced here: ISO gives M1 `r = 0`, so 5c must treat
   its 2 EC codewords as *detection* (reject on non-zero syndrome), not attempt correction.
   `MicroSymbolInfo::error_correction_capacity == 0` is the flag 5c should branch on.
5. **Not in scope, no Known Gap opened:** micro segmentation, encoding, decoding and detection are
   Steps 5b–5d. Nothing in 5a is a partial implementation of them.
