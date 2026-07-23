# Review Feedback — Step 2b: qr-decode

**Verdict: Step 2b CLEAR**
Reviewer: Richard-D · Date: 2026-07-23
BLOCKER: 0 · MAJOR: 0 · MINOR: 3

Reviewed `crates/qr-decode/` (lib.rs, bits.rs, extract.rs, segment.rs, tests/decode.rs, Cargo.toml).
qr-core API treated as trusted; verified only the signatures qr-decode calls against it.

## Verification (all green)
- `cargo check -p qr-decode` — clean, no_std respected.
- `cargo test -p qr-decode` — 7 passed, 0 failed.
- `cargo clippy -p qr-decode --all-targets -- -D warnings` — clean on forced rebuild (not stale cache).
- No `unsafe` (`#![forbid(unsafe_code)]`); zero `unwrap/expect/panic!` in `src/`. Zero deps beyond qr-core.

## Correctness — verified
- **Zig-zag walk** matches nayuki convention exactly: `right` from `size-1` step −2, `right==6→5` timing skip, `upward = (right+1)&2==0`, `is_function_module` skip, `bit < total_bits` guard. extract.rs:99-129.
- **Format info both copies** (extract.rs:20-58) matches ISO/nayuki placement bit-for-bit: copy1 col-8/row-8 L-shape with the `(8,7)/(8,8)/(7,8)` bit-6/7/8 break, copy2 `(size-1-i,8)` + `(8,size-15+i)`. `format_info_decode` (≤3-bit BCH correct), first copy that decodes wins.
- **Version info v≥7** (extract.rs:62-78): both copies (`(size-11+i%3, i/3)` and transpose), used only to *reject* a geometry/self-description mismatch; size stays authoritative. Matches brief.
- **De-interleave math** (extract.rs:133-186): `max_data = max(g1,g2)`, column-wise data fill skipping short group-1 blocks past their length, EC block-major. Inverse of the encoder's interleave; proven by the clean v5-Q (unequal groups) and v7-M (4-block) tests decoding exact payloads.
- **rs_correct Err handling**: any `Err` → `DecodeError::BlockCorrection`, whole `data` buffer discarded (extract.rs:181). Core contract honored.
- **Segment bit exactness** (segment.rs): numeric 10/7/4 with `>999/>99/>9` guards; alnum 11/6 with `>=45*45/>=45` guards; byte 8; Kanji 13 + correct SJIS reconstruction; ECI varint 1/2/3-byte by leading bits with clean reject on invalid prefix; char-count via `Mode::char_count_bits(version)`; terminator `0b0000` and `remaining()<4` padding both break; pad bytes (0xEC/0x11) never reached because a terminator always precedes them in well-formed streams.
- **BitReader** (bits.rs): MSB-first, bounds-checked (`pos + n > len*8 → None`), `n>32 → None`. All call sites read ≤16 bits.
- **Robustness**: every read is `?`-propagated to `None → Malformed`; truncated streams, oversized counts, and ECI overflow all yield a clean error, no panic. Mirror retry (lib.rs:91-99) transposes on any failure and preserves the original error if the retry also fails.
- **ISO Annex I vector independently validated**: hardcoded data codewords `0x10 0x20 0x0C 0x56 0x61 0x80 0xEC 0x11…` are the genuine ISO 1-M "01234567" values; the numeric parser is exercised against the real ISO answer (zig-zag is bypassed as a validator since the fixture is its inverse, but the parser + RS + format roundtrip are genuine).

## Should Fix (MINOR — none block)
- **MINOR-1 tests/decode.rs:222-246** — The corruption test (`corrects_corrupted_codewords`) runs on v1-M, a **single-block** symbol; the brief asked corruption to hit multiple blocks. Multi-block symbols (v5-Q, v7-M) are only tested clean. Error correction on an *interleaved multi-block* symbol is therefore untested. Risk is low (de-interleave proven by clean multi-block tests, rs_correct unit-tested in core), but recommend one test that flips modules spanning ≥2 blocks of the v5-Q fixture and asserts recovery + `errors_corrected > 0`.
- **MINOR-2 tests/decode.rs (whole fixture builder)** — Fixtures are the literal inverse of the reader, so zig-zag/format/version *placement geometry* is not independently validated against ISO — only segment parse, RS, and format roundtrip are. Placement matches the nayuki convention (verified by inspection here), and true interop is deferred to Step 3b per accepted gap. Awareness only; the Step-3b roundtrip vs qr-encode is the real check — ensure qr-encode uses the same bit ordering.
- **MINOR-3 tests/decode.rs:198-206** — Annex I EC codewords are derived via `rs_encode` rather than pinned to ISO's published 10 EC bytes (`A5 24 D4 C1 ED 36 C7 87 2C 55`). A shared encode/decode bug in qr-core RS would hide. Cheap hardening: assert the EC bytes equal the ISO values. qr-core RS is separately reviewed, so low priority.

## Accepted gaps (not flagged, per brief)
Kanji raw SJIS bytes only; structured-append parsed-and-skipped (not reassembled); roundtrip vs qr-encode deferred to Step 3b.

## Cleared
Reviewed the full qr-decode crate: pipeline correctness (format/version/zig-zag/de-interleave/RS/segment), robustness against malformed input, no_std/no-unsafe/zero-dep discipline, and test adequacy. All DoD checks pass. No blocking or major issues; three minor test-hardening notes.
