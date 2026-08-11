# Session Checkpoint — 2026-08-11
*Read this before reading anything else. If it covers current state, skip BUILD-LOG.*

---

## Where We Stopped

Through 5b3f093, all CI green (7 jobs). This session shipped: Micro QR
perspective refinement (envelope 1%→10% keystone, ~30deg hand-held);
benchmarks crate + baseline; perf rework (detect 3x to 687us/720p, flat
frames 21x, encode v40 16x to 604us, all bit-identical by differential
proof); demo camera path (stateful qr-video Scanner in wasm, 1080p
constraints, 15fps, wasm-side luma, PNG download/copy, instant-regen
generator); low-light + glare hardening (envelope 21.7%→56.8%, glare rule:
blob needs d/2+2 modules clearance from finder — on-finder glare is
physics); no_std qr-detect (std feature default-on, hand-rolled 1-ulp
mathf, QVGA detect+decode on emulated riscv32 in 163.5KB heap).

## Next Up (Project Owner intent)

ESP32-S3-CAM (OV3660) gate reader: camera → detect_luma → validate payload
against gRPC server → GPIO high if valid. Decisions pending: firmware
scaffold now vs when board arrives; network design (recommended: protobuf
over HTTPS POST via prost + esp-idf mbedtls — real gRPC/h2 too heavy);
whether server can grow a unary HTTP endpoint. esp-idf std shell
recommended over bare-metal (WiFi/TLS). Xtensa toolchain (espup) needed —
not in CI; optional xtensa check job via esp-rs/xtensa-toolchain action.
Sensor tip: YUV422 Y plane == luma, feed directly.

## Still Open

- KG-4 Kanji text, KG-7 alignment grid, KG-9 estimator bias, KG-13/14/15
  minor detect items, KG-18/19 micro detect asymmetries, KG-20 ESP heap
  note, KG-21 ROI ladder exclusion (ruled), KG-22 Binary bit-packing
  (43% memory cut, needs own step vs 5% bench gate)
- Demo: binarized-view toggle needs a qr-detect debug export; camera-path
  QR version label needs qr_video::Observation.version
- LICENSE files before public; GitHub gc for purged ISO PDF
- Cloudflare Pages deploy step armed, awaiting repo secrets

## Resume Prompt

You are Arch on this project.
Read SESSION-CHECKPOINT.md, then ARCHITECT.md.
Confirm where we stopped and what the next action is. Then wait.

## Version Check
version_notified: v1.3.0
