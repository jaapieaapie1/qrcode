# Review Request — Step 2b: qr-decode

**Builder:** Bob-D
**Ready for Review:** YES
**Scope:** `crates/qr-decode/` only. Root `Cargo.toml` untouched (dep uses existing `qr-core` workspace entry).

## Files

| File | What |
| --- | --- |
| `crates/qr-decode/Cargo.toml` | Added `qr-core = { workspace = true }`. |
| `crates/qr-decode/src/lib.rs` (~185 lines) | Public API: `decode`, `Decoded`, `DecodeError`. Orchestration + mirror retry + `version_from_size` + `transpose`. |
| `crates/qr-decode/src/bits.rs` (~46 lines) | `BitReader`: MSB-first fixed-width bit field reader over a byte slice. |
| `crates/qr-decode/src/extract.rs` (~200 lines) | `read_format_info` / `read_version_info` (both copies, BCH-corrected), zig-zag codeword extraction + un-mask, de-interleave, per-block `rs_correct`. |
| `crates/qr-decode/src/segment.rs` (~155 lines) | `parse_segments`: numeric / alphanumeric / byte / Kanji / ECI / FNC1 / structured-append. |
| `crates/qr-decode/tests/decode.rs` (~330 lines) | Fixture builder (inverse of the reader) + test-side byte-mode encoder; 7 tests. |

## API

```rust
pub fn decode(matrix: &BitMatrix) -> Result<Decoded, DecodeError>;

pub struct Decoded {
    pub bytes: Vec<u8>,
    pub text: Option<String>,   // best-effort UTF-8 of bytes
    pub version: Version,
    pub ec: EcLevel,
    pub mask: u8,
    pub errors_corrected: usize,
}

pub enum DecodeError { InvalidSize, FormatInfo, VersionInfo, BlockCorrection, Malformed }
```

Matches the brief's `decode(&BitMatrix) -> Result<Decoded{...}>` signature exactly.

## Pipeline (each stage → the qr-core primitive it uses)

1. `version_from_size(size)` — size is authoritative (`17 + 4·v`).
2. `read_format_info` — reads both format copies, `qr_core::bch::format_info_decode` (≤3-bit correction), first copy that decodes wins.
3. Version info (v≥7) — `read_version_info` reads both copies via `version_info_decode`; used only to **reject** a matrix whose self-described version contradicts its geometry. Size stays authoritative when the field is unreadable.
4. Zig-zag extraction — standard two-column walk from bottom-right, skips function modules via `qr_core::is_function_module`, un-masks each module via `qr_core::mask_bit`.
5. De-interleave — block layout from `qr_core::ec_blocks` (group1/group2), inverse of the encoder's column interleave.
6. Per-block `qr_core::reed_solomon::rs_correct` — sums corrected-codeword count; **any** `Err` (unrecoverable block) → `DecodeError::BlockCorrection` (buffer discarded, per core contract).
7. `parse_segments` — MSB-first `BitReader` over the concatenated corrected data.

Mirror handling: `decode` runs `decode_oriented`; on any error it retries on the transposed matrix and returns the mirrored result, else surfaces the original error.

## Key decisions

- **Placement convention:** ISO/nayuki format/version bit placement and the `right==6 → 5` zig-zag column skip. Bit ordering (bit 0 = LSB of the value from `format_info_encode` / `version_info_encode`) matches qr-core's encoders, so it will line up with qr-encode's placement in Step 3.
- **Size is authoritative for version**, not the version-info field — the input is an exact-size `BitMatrix`, so geometry is definitive; version info is a redundant integrity check only.
- **`text` is best-effort** `String::from_utf8(bytes)`: covers numeric/alphanumeric (ASCII), byte-mode UTF-8, and ECI 26. Non-UTF-8 (e.g. Shift-JIS Kanji) → `text = None`, `bytes` still holds the raw payload (Kanji emitted as its two Shift-JIS bytes). Per brief: "ECI 26 → UTF-8; else raw bytes."
- **ECI/FNC1/structured-append** parsed for length (to keep bit alignment) and skipped; they add no payload bytes.
- **no_std + alloc**, zero deps beyond `qr-core`. Uses stable edition-2024 let-chains in `decode_oriented`.

## Tests (all green)

`decodes_iso_annex_i_vector` (ISO Annex I v1-M "01234567", exact Annex I data codewords, EC via `rs_encode`, mask 2 → bytes/text/version/ec/mask/errors all asserted) · `corrects_corrupted_codewords` (6 flipped modules → recovered, `errors_corrected > 0`) · `rejects_uncorrectable_matrix` (90 flips → `BlockCorrection`) · `rejects_invalid_size` (size 20 → `InvalidSize`) · `decodes_mirrored_symbol` (transposed fixture → recovered via retry) · `decodes_multiblock_byte_symbol` (v5-Q, group1+group2 unequal blocks → de-interleave path) · `decodes_version_info_symbol` (v7-M, version-info read + 4-block interleave).

Fixtures are hand-built from qr-core primitives (no dependency on the concurrently-built qr-encode); the fixture builder is the literal inverse of the reader.

## DoD

- `cargo test -p qr-decode` — 7 passed, 0 failed.
- `cargo check -p qr-decode` — clean (no_std respected).
- `cargo clippy -p qr-decode --all-targets -- -D warnings` — clean.

## Uncertainties / notes for Richard

- **Roundtrip vs qr-encode deferred to Step 3b** (per brief). Correctness here rests on the fixture builder matching the reader; if qr-encode diverges from the ISO/nayuki placement convention, Step 3 will catch it. Worth a cross-check that qr-encode's format/version/zig-zag placement uses the same bit ordering.
- **Kanji → text:** no Shift-JIS→UTF-8 table (out of scope, would be a large table); Kanji bytes land in `bytes` only. Flag if `text` for Kanji is required.
- **Structured-append** is skipped, not reassembled (multi-symbol join is not in the brief). Single-symbol payload of an SA symbol still decodes.
