# Review Request — Step 2a: qr-encode

Builder: Bob-E
Ready for Review: YES

## Scope
Full ISO/IEC 18004 encode data path in `crates/qr-encode/`. Depends only on `qr-core`
(added `qr-core = { workspace = true }` to `crates/qr-encode/Cargo.toml`). `no_std` +
`alloc`, no other deps. Touched only the qr-encode crate.

## Files
- `src/bits.rs` (65) — `BitBuffer`: MSB-first bit accumulator packed into `Vec<u8>`.
- `src/segment.rs` (145) — mode selection + per-mode bit encoding (numeric / alphanumeric
  / byte); capacity math (`total_bits`, `payload_bits`, char-count via core `Mode`).
- `src/matrix.rs` (402) — function-pattern drawing, zigzag data placement, mask XOR,
  format/version info placement, RS block split + interleave (via core `rs_encode`), ISO
  7.8.3 penalty scoring, mask selection.
- `src/lib.rs` (224) — public API: `encode`, `Options`, `EncodeError`. Pipeline: resolve
  mode → resolve smallest-fitting version → build padded data codewords → interleave →
  place → mask-select (or forced) → format/version info.
- `src/tests.rs` (265) — see Tests.

## Public API
- `encode(data: &[u8], options: Options) -> Result<BitMatrix, EncodeError>`
- `Options { ec: EcLevel, version: Option<Version>, mask: Option<u8>, mode: Option<Mode> }`
  + `Options::new(ec)` for full auto.
- `EncodeError { DataTooLarge, ModeMismatch, InvalidVersion, InvalidMask }` (Display + Error).
- Coordinates are `(x, y)` = (col, row), origin top-left, documented on placement fns.

## Key decisions
- **Reuse, no duplication**: uses core `ec_blocks`, `rs_encode`, `format_info_encode`,
  `version_info_encode`, `mask_bit`, `is_function_module`, `BitMatrix`, `Version`, `Mode`,
  `EcLevel`. No core logic reimplemented.
- **Placement/format/version geometry** follows the standard zigzag + Nayuki-style
  format/version bit maps. Verified against ISO Figure I.2 (see Tests) — geometry is
  pixel-exact including the format-info L around the top-left finder.
- **Penalty N3 (subtle)**: ISO 7.8.3 NOTE 3 scores a 1:1:3:1:1 pattern **40 points once**
  for its *existence* when a 4-wide light area is present before OR after — NOT 40 per side.
  An initial both-sides (Nayuki-style) count double-counted and wrongly selected mask 0 for
  the Annex I symbol; corrected to score-once, which reproduces ISO's mask 010 (2). N4 uses
  the exact ISO integer `k` form. N1/N2 standard.
- **Mask tie-break**: lowest mask id wins ties (standard).
- **Padding**: 0xEC/0x11 alternating; terminator clamped to remaining capacity.

## Tests (10, all green; clippy clean; fmt clean; no_std — no `std::` in non-test source)
- `annex_i_data_codewords` — bitstream/pad exactly matches ISO Annex I 16 data codewords.
- `annex_i_interleave_produces_reference_ec` — RS EC matches ISO's 10 EC codewords.
- `annex_i_full_symbol_roundtrip` — encode → read format via core `format_info_decode`
  (=> M) → unmask → zigzag-extract → data+EC match ISO. (core primitives verify the matrix.)
- `annex_i_selected_mask_is_two` — auto mask-selection reproduces ISO's mask 010 (2).
- `forced_mask_is_honoured_and_verifiable` — all 8 forced masks round-trip through format.
- `byte_mode_roundtrips_through_ec_correction` — v5-H multi-block: deinterleave + core
  `rs_correct` → 0 errors on a clean symbol (exercises 2-group interleave).
- `n1_run_penalty_counts_long_runs`, `n3_finder_penalty_detects_pattern` — penalty units.
- `smoke_encode_all_versions` — v1–v40 byte encode, size + format sane.
- `too_large_payload_is_rejected` — clean error, no panic.

Note: the ISO Annex I full-matrix vector was validated two ways — exact data/EC codewords +
mask, and geometry cross-checked against the standard's Figure I.2 rendering.

## Known Gaps (per brief — allowed for v1)
- Single-mode segmentation only (whole payload one mode). No greedy mixed-mode.
- No Kanji, no ECI in encoder.

## Uncertainties for Richard
- N3 "score-once vs per-side" — I went with the literal ISO NOTE 3 reading, corroborated by
  the Annex I mask-2 result. Worth a second look since implementations differ here.
- `penalty_score` / `penalty_components` are `pub` within the (private) `matrix` module for
  test access; not part of the crate's external API. Fine to keep or tighten.

## Workspace note
No root Cargo.toml change needed — `qr-core` already in `workspace.dependencies`.
