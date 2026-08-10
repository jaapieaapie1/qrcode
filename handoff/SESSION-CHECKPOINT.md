# Session Checkpoint — 2026-08-10
*Read this before reading anything else. If it covers current state, skip BUILD-LOG.*

---

## Where We Stopped

Full stack built, reviewed, committed (through afa3d30). All six crates +
tests-integration green: `cargo test --workspace` 29 suites pass, clippy clean,
fmt clean. Cross-target verified: no_std trio on thumbv7em + riscv32imc (ESP32-C3
class), all crates on wasm32. Goal from Project Owner satisfied; awaiting next
direction.

---

## What Was Decided This Session

- Six-crate layout + tests-integration; waves: core → (encode|decode|render|detect parallel) → (video|integration) → detect-fix
- N3 penalty: score-once per pattern (ISO Table 11 NOTE 3, verified from spec text; Nayuki per-side deviates)
- Detect: 2D 5×5 alignment template match (scanline method unreliable); axis_scale rotation correction; nested-phantom triple rejection
- Video: lending FrameSource (`next_frame(&mut self) -> Option<Frame<'_>>`), no capture dep, qr-detect decode feature pinned
- Multiscan is core detect behavior (Vec<Detection>), covered in tests-integration/tests/multiscan.rs
- Micro QR: NOT built; if ordered → `micro` feature, parallel types, default-on scan side (KG-8)

## Still Open

- KG-8 Micro QR decision (Project Owner)
- KG-4 Kanji SJIS→UTF-8, structured append reassembly
- KG-6 encoder mixed-mode segmentation, Kanji/ECI encode
- KG-7/KG-9 detect: full alignment grid for severe perspective; estimator bias 35–40° for v20+
- No git remote / CI. CI could enforce the 4 verified targets.

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
