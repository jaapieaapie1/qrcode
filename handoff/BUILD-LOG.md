# Build Log
*Owned by Architect. Updated by Builder after each step.*

---

## Current Status

**Active step:** 1 — qr-core foundation (Bob building)
**Last cleared:** scaffold — 2026-07-23
**Pending deploy:** NO

---

## Step History

### Step 0 — Workspace scaffold — COMPLETE
*Date: 2026-07-23*

Files changed:
- `Cargo.toml` — workspace, resolver 3, edition 2024, 6 members
- `crates/{qr-core,qr-encode,qr-decode,qr-detect,qr-video,qr-render}/` — skeletons, compile green
- git initialized, scaffold committed

### Step 1 — qr-core — COMPLETE (pending review)
*Date: 2026-07-23*
Bob (Opus) built per brief. `cargo test -p qr-core` = 26 passing, `cargo check`
green (no_std), clippy clean.

Files created under `crates/qr-core/src/`: gf.rs, reed_solomon.rs, bch.rs,
bitmatrix.rs, blocks.rs, version.rs, ec.rs, mode.rs, mask.rs, function.rs;
lib.rs wired. Tests under `crates/qr-core/tests/`: gf, reed_solomon, bch,
tables, matrix_mask.

Key decisions:
- alloc used for BitMatrix + RS buffers (rest is alloc-free). #![forbid(unsafe_code)].
- RS: roots α^0.., fcr=0; rs_correct re-verifies syndromes → no silent mis-decode.
- BCH decode: brute-force min-distance (≤3 bit correction).
- Function-map correctness cross-checked geometrically against total-codeword +
  remainder tables for all 40 versions (independent verification).

See handoff/REVIEW-REQUEST.md for full detail. Ready for Richard.

---

## Known Gaps

- (none yet)

---

## Architecture Decisions
*Locked decisions that cannot be changed without breaking the system.*

- Workspace layout: crates/ dir, 6 crates: qr-core (no_std foundation), qr-encode (no_std+alloc), qr-decode (no_std+alloc), qr-detect (std+image), qr-video (std, trait-based FrameSource, no hard capture dep), qr-render (std+image) — 2026-07-23
- All shared QR domain logic (GF256, RS, BCH, tables, masks, function-pattern map, BitMatrix) lives in qr-core; encode/decode must not duplicate it — 2026-07-23
- Coordinate convention (x, y) = (column, row), origin top-left — 2026-07-23
- Zero external deps except `image` (detect/render) and gated video I/O — 2026-07-23
- Build waves: 1=core, 2=encode+decode+render+detect parallel, 3=video+integration — 2026-07-23
