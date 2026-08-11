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

- No git remote — push to GitHub to activate CI
- KG-4 (Kanji SJIS→UTF-8, structured append), KG-6 (mixed-mode/Kanji/ECI encode), KG-7 (full alignment grid), KG-9 (estimator bias 35–40°), KG-11 (two-pass binarize), KG-12 (M1/M3 nibble interop vs external decoder), KG-13 (bridged-timing crop), KG-14 (hardcoded micro map in detect)
- MicroOptions has no forced-mode field (accepted)

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
