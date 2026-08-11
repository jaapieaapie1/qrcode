# Session Checkpoint — 2026-08-10
*Read this before reading anything else. If it covers current state, skip BUILD-LOG.*

---

## Where We Stopped

Wave 4 (Micro QR M1–M4) + CI complete and committed through 2497114. Full stack:
QR v1–40 AND Micro M1–M4 encode/decode/detect/render/video, all from scratch,
all reviewed. `cargo test --workspace` green in both feature configs (108
integration + per-crate suites); clippy/fmt clean; cross-targets green
(thumbv7em, riscv32imc, wasm32). CI workflow at .github/workflows/ci.yml —
activates on first push (NO REMOTE CONFIGURED YET).

---

## What Was Decided This Session (Wave 4)

- `micro` feature default-on everywhere, forwarded (`micro = ["qr-core/micro"]`); parallel types (MicroVersion, MicroEcLevel incl. Detection, encode_micro/decode_micro/detect_micro/detect_all)
- M1/M3 half codeword: HIGH nibble / LEFT-JUSTIFIED into RS (KG-12 interop verification REFUTED the earlier low-nibble ruling — zxing-cpp + segno agree bit-exact; Annex N.1 describes a LEADING partial codeword padding left, ours is trailing). Golden external vectors pinned in tests-integration/tests/micro_golden.rs
- ISO 7.5.1 misdecode caps enforced on BOTH QR and micro paths (Step 5f; behavior change for 1-L/1-M/2-L — release-note material)
- Binarize adaptive radius /16→/8 (micro-only bug fix); vignette cost pinned at depth 0.85; two-pass binarize = KG-11
- MAX_FUNCTION_MISMATCHES = 1, bracketed by test
- Micro detection envelope: rotation exact 0–45° all versions, 2 px/module floor, ~4% perspective (affine-only, fails closed)

## Still Open

- KG-4 (Kanji SJIS→UTF-8 decode text, structured append), KG-7 (full alignment grid for severe perspective), KG-9 (estimator bias 35–40° v20+), KG-13 (bridged-timing crop), KG-14 (hardcoded micro map), KG-15 (qr-video decodable fixture), KG-16 (multi-code vignette retry limit, accepted)
- MicroOptions has no forced-mode field (accepted); Options not non_exhaustive (revisit at crates.io publish)
- No LICENSE files despite MIT OR Apache-2.0 in Cargo.toml — add before going public
- GitHub server-side gc pending for full ISO-PDF purge (history rewritten 2026-08-11) — ask GitHub Support before flipping public

CLOSED this session: KG-6 (DP segmentation + opt-in Kanji + ECI, 7f75a0b), KG-11 (layered binarize retry, 627c69e), KG-12 (nibble convention REFUTED + fixed both sides + golden vectors, 0c36cdc). Repo: github.com/jaapieaapie1/qrcode (private), CI green.

---

## Resume Prompt

Copy and paste this to resume:

---

You are Arch on this project.
Read SESSION-CHECKPOINT.md, then ARCHITECT.md.
Confirm where we stopped and what the next action is. Then wait.

---

## Version Check
version_notified: v1.3.0
