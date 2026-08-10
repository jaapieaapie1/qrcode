@.claude/skills/token-optimization.md

## Project
From-scratch QR code stack in Rust. Cargo workspace, crates/: qr-core
(no_std foundation: GF256, RS, BCH, tables, masks, BitMatrix), qr-encode
(no_std+alloc, ESP-target), qr-decode (no_std+alloc), qr-detect (std+image,
multi-code), qr-video (frame-stream tracking), qr-render (std+image, PNG/SVG/
ASCII). Only external dep: image (+transitives). tests-integration holds
cross-crate roundtrip/warp/fuzz/multiscan/video suites. ISO 18004:2024 PDF in
repo root. State lives in handoff/ (BUILD-LOG, SESSION-CHECKPOINT).

## Three Man Team
Available agents: Arch (Architect), Bob (Builder), Richard (Reviewer)
