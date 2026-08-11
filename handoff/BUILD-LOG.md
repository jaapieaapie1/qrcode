# Build Log
*Owned by Architect. Updated by Builder after each step.*

---

## Current Status

**Active step:** Step 9 (benchmarks) BUILT, awaiting review — see `handoff/REVIEW-REQUEST-bench.md`. Step 7 COMPLETE (esp-selftest committed, esp32-emulation CI job green on GitHub in 54s: run 31439384048).
**Last cleared:** Waves 1–3 complete + Step 4 detect fix (afa3d30) + CI (f13fa3a)
**Pending deploy:** NO — remote github.com/jaapieaapie1/qrcode (private), CI green incl. esp32-emulation

Wave 4 decisions:
- M1/M3 half-codeword: LOW nibble zero-prefixed into RS (ISO Annex N.1 precedent) — both sides aligned, EC difference pinned by test
- Feature unification: in-workspace builds always carry qr-core/micro (cargo additivity); off-switch is for external consumers — accepted
- MicroOptions has no forced-mode field — accepted gap
- NEW from Richard-MD: QR decode path conformance defect — v1-L/M/Q/H, v2-L, v3-L must cap corrections at published r (< ec/2, ISO 7.5.1/Table 9 footnote a); current d/2 cap can return wrong payload as success. Step 5f.

---

## Step History

### Step 9 — benchmarks — BUILT, AWAITING REVIEW
*Date: 2026-08-11 · Builder: Bob-BM · Review request: `handoff/REVIEW-REQUEST-bench.md`*

New workspace member `benchmarks/`. Hand-rolled harness, **zero new
dependencies** (no criterion, no `test::Bencher`).

Files changed:
- `Cargo.toml` — `"benchmarks"` added to `members` (only root edit)
- `benchmarks/Cargo.toml` — dev-deps on the six qr crates + `image`; `autobenches = false`; five `[[bench]]` entries with `harness = false`; own `micro` feature forwarding to siblings
- `benchmarks/src/lib.rs` — harness: auto-calibration to ~1s/bench, warmup, per-iteration timing, min/median/p95/max, throughput/ns-op column, aligned table, `BENCH_QUICK=1` smoke mode
- `benchmarks/benches/common/mod.rs` — in-code fixtures: luma conversion, 720p frame building, blit, bilinear rotate, deterministic payloads, capacity-filling symbols
- `benchmarks/benches/{core,encode,decode,detect,video}.rs` — the five bench targets
- `benchmarks/README.md` — run instructions + full baseline table (Apple M4, rustc 1.96.0, 2026-08-11) + commentary
- `.github/workflows/ci.yml` — new `bench-smoke` job: `BENCH_QUICK=1 cargo bench -p benchmarks` (build-and-run gate, not a perf gate)

Key decisions:
- **Dev-deps take the siblings at `workspace = true`.** Round 1 used explicit paths with `default-features = false`, justified as protecting CI's `--no-default-features` lane. Richard-BM proved that false: the lane is already unified `micro`-on at HEAD via `tests-integration` → `qr-video` → `qr-detect`, and `benchmarks` cannot close it. Reverted to the convention; the escalation about that lane is his, below.
- **Sub-microsecond benches batch internally** (`xB` in the name) because a single `Instant::now` pair (~41 ns on this machine) is the same order as the work. Covers GF, BCH, small RS, `decode` v1, `decode_micro`.
- **Three stages have no public entry point and are measured by difference**: binarize+scan as `detect_luma_single_pass` (retry = the gap to `detect_luma`), the DP as pinned-version `mode: None` vs `Some(Byte)`, mask selection as `mask: None` vs `Some(0)`.
- **Every fixture asserts its identity** (version, decoded payload on both upright and rotated frames, detection counts on all four detector frames including both single-pass outcomes, `Stats` deltas for video) before timing, so a bench cannot silently drift onto another code path.
- Answers KG-15's ask in passing: the qr-video benches use a real decodable fixture and assert `roi_hits > 0` with `full_scans` unchanged, so steady-state ROI is genuinely what is measured.

Headline baseline (medians, Apple M4, verbatim table in `benchmarks/README.md`):
encode v1-M 42.42 µs · encode v40-L 10.21 ms · decode v1-M 1.74 µs · decode
v40-L 629.88 µs · detect 720p happy 2.13 ms · detect 720p miss 4.07 ms (flat
frame 6.88 ms) · video ROI frame 902.71 µs · video full-scan frame 2.15 ms ·
`gf::mul` 0.58 ns/op.

Findings: **mask selection is 92.4% of `encode()`** at both v1 and v40 (auto
42.08 µs vs `mask: Some(0)` 3.21 µs; 9.81 ms vs 744.50 µs) — measured, not
inferred, and it is why encode costs 24x decode on the same symbol. The
segmentation DP costs ~1% on min and nothing on median. Rotation costs 1%. ROI
tracking gives 2.4x. New gap KG-17 below.

Guards: `cargo test --workspace` and `--no-default-features` green; clippy
`-D warnings` clean on both lanes; `cargo fmt --all --check` clean; full bench
suite and `BENCH_QUICK=1` both run clean.

### Step 0 — Workspace scaffold — COMPLETE
*Date: 2026-07-23*

Files changed:
- `Cargo.toml` — workspace, resolver 3, edition 2024, 6 members
- `crates/{qr-core,qr-encode,qr-decode,qr-detect,qr-video,qr-render}/` — skeletons, compile green
- git initialized, scaffold committed

### Wave 2 — encode/decode/render/detect — IN PROGRESS
*Date: 2026-07-23*
Four Bobs (Opus) parallel: Step 2a encode, 2b decode, 2c render, 2d detect.
Per-crate review requests: handoff/REVIEW-REQUEST-{encode,decode,render,detect}.md.
Detect codes against decode's brief-defined API; decode call feature-gatable if sibling lags.

### Step 5a — qr-core micro foundation — BUILT, AWAITING REVIEW
*Date: 2026-08-10 — Bob-M*

Files: `crates/qr-core/src/micro.rs` (new, 734), `crates/qr-core/tests/micro.rs` (new, 707),
`Cargo.toml` (+`micro` feature, default-on), `lib.rs` (+gated `pub mod micro`),
`bch.rs` (`bch_remainder` → `pub(crate)` so micro reuses the shared BCH(15,5); ISO Annex C.2
specifies the same generator for both symbologies).

API: `MicroVersion`, `MicroEcLevel`, `MicroSymbolInfo`, `MicroMode`, `micro_mask_bit`,
`micro_format_info_{encode,decode,read,write}`, `MICRO_FORMAT_INFO_POSITIONS`,
`micro_is_function_module`, `micro_data_modules()`. All parallel types — no existing
signature or enum changed. Detail + ISO citations per table: handoff/REVIEW-REQUEST-micro-core.md.

Key decisions:
- `MicroEcLevel { Detection, L, M, Q }` is a NEW parallel enum, not `EcLevel`. ISO Table 13
  symbol 0 is "Error detection only" — M1 has no EC level, and H never applies. Deviates from
  the brief's "allowed EcLevel subsets" wording; flagged for Arch ack.
- `micro_data_modules()` is an allocation-free iterator and the single source of truth for
  encode+decode placement (the `is_function_module` pattern). Takes only the version: module
  order does not vary with EC level; only the bit-stream index of the M3 half codeword does.
- Micro masks forward to `mask_bit` via `MICRO_MASK_TO_QR = [1,4,6,7]` — ISO Table 10 lists
  both references against one condition column, so no formula is duplicated.
- Micro mask-selection score (ISO 7.8.3.2 edge count) deliberately NOT in core — it belongs in
  qr-encode (5b), by symmetry with N1–N4.

Verification: every table read page-targeted from the ISO PDF (Tables 1, 2, 3, 7, 9, 10, 13, C.1;
clauses 5.3.2.2–5.3.8, 7.4.10, 7.4.11, 7.5.1, 7.6, 7.7.3, 7.9.2, Annex C). Figures 4 and 26
rasterised and read. **Annex I.3 / Figure I.4 rendered at 600 dpi and decoded to a 13×13 grid;
the M2-L example is reproduced from core primitives with 0 mismatches across all 169 modules**,
which pins placement order, mask mapping, format-info value and format-info bit positions to
printed ISO output.

Tests: 14 new. `cargo test -p qr-core` = 40 pass (micro on) / 26 pass (micro off, file cfg'd out).
`cargo check --workspace` + `clippy -D warnings` green in both configs. no_std proven by
`cargo check -p qr-core --target {thumbv7em-none-eabihf,riscv32imc-unknown-none-elf}` in both configs.

### Step 1 — qr-core — COMPLETE, REVIEWED CLEAR, COMMITTED (f8f74ab)
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

### Step 8 — browser demo (WASM) — BUILT, AWAITING REVIEW
*Date: 2026-08-11 — Bob-W*

Files changed:
- `Cargo.toml` — `exclude = ["esp-selftest", "demo-wasm"]` only; no other root edit
- `.gitignore` — ignore generated `demo-wasm/www/pkg/`
- `demo-wasm/` — NEW, outside the workspace: cdylib + `wasm-bindgen`, two exports
  (`scan(luma, w, h) -> JSON string`, `generate(text, ec, micro) -> SVG or "ERROR: …"`).
  Deps: path deps on qr-core/qr-encode/qr-render/qr-detect + wasm-bindgen. No
  web-sys, no serde — JSON hand-written, luma computed in JS, raw buffers only.
- `demo-wasm/www/` — index.html + app.js + style.css, no framework/CDN/bundler.
  Camera scan loop (rAF throttled to ~10 fps, 720 px longest edge), corner
  overlay incl. located-but-unreadable symbols, distinct-payload list with
  timestamps, file-input path for camera-less testing; live SVG generation with
  EC select, Micro checkbox, inline errors, download link.
- `demo-wasm/README.md` — build (wasm-pack, with a wasm-bindgen-cli fallback),
  serve, WASM API reference.
- `.github/workflows/ci.yml` — new `demo-build` job: pinned wasm-pack tarball,
  fmt + clippy for the excluded crate, `wasm-pack build --target web`, asset
  wiring check. No browser test.

Key decisions: EC is never silently downgraded — Micro + `H` is a readable
error, and `D` (M1 detection-only) is exposed as a micro-only selection so all
of M1–M4 is reachable. Undecodable candidates are returned with corners so the
overlay can show "found but unreadable".

Verified: wasm-pack build clean; demo fmt + clippy clean; `cargo test
--workspace` green; `esp-selftest` release build green; 21-assertion headless
node round trip through the real wasm binary (generate → rasterise SVG → scan)
ALL PASS; `python3 -m http.server` serves every asset (wasm as
`application/wasm`). Camera/UI interaction is unverified — needs a real browser.
See handoff/REVIEW-REQUEST-demo.md.

### Step 7 — ESP32-class execution test in CI — REVIEWED CLEAR, ROUND-2 FIXES APPLIED
*Date: 2026-08-11 — Bob-E32 (Richard-E32: CLEAR, 0 BLOCKER / 1 MAJOR / 8 MINOR — all addressed)*

Files changed:
- `Cargo.toml` — `exclude = ["esp-selftest"]` only; no other root edit
- `esp-selftest/` — NEW, outside the workspace: bare-metal `no_std`/`no_main`
  firmware for QEMU `virt` riscv32imc. Hand-rolled `_start` (global_asm), linker
  script (RAM 0x8000_0000), 192 KiB bump allocator, NS16550A UART0 driver at
  0x1000_0000, panic handler, SiFive test finisher exit at 0x0010_0000. Zero
  dependencies outside qr-core/qr-encode/qr-decode (default features, micro on).
- `.github/workflows/ci.yml` — new `esp32-emulation` job: apt qemu-system-misc,
  riscv32imc target, own `cargo fmt --check`, build, run under `timeout 120`

What it executes on the emulated core: QR roundtrip v1/2/3/5/7/8/10 × L/M/Q/H
plus an auto-version symbol (29); all 8 Micro QR symbols with numeric capacity
probed not tabulated (8); RS recovery incl. 5-Q four-block at full capacity (3);
ISO 7.5.1 r-cap rejection incl. the headline 1-L-with-3-errors case (7).
Local result: `ALL PASS`, QEMU exit 0, heap peak 75 348 / 196 608 bytes.

Key decisions:
- Corruption is injected at exact codeword granularity by replaying the ISO
  7.7.3 zigzag (`codeword_modules`), so the capacity-boundary cases are exact
  rather than bounded-from-above.
- Arena reset between suites: a bump allocator cannot reclaim non-LIFO frees, so
  the raw peak measured fragmentation (122 KB, climbing per suite) rather than
  working set. Reset makes the footprint bounded by the worst single suite.
- CI job restates RUSTFLAGS at job level: a `RUSTFLAGS` env var overrides
  `[target.*] rustflags` wholesale, so the workflow-level `-D warnings` would
  drop the linker script (verified: `undefined symbol: __ebss`).
- Negative controls run and reverted: forced assertion failure → exit 1 with
  detail; 2 KiB heap → panic handler prints OOM → exit 1.

Round-2 fixes (Richard-E32 review, all 9 findings):
- MAJOR: trap vector installed. `_start` sets `mtvec`; `_trap` re-points `sp` at
  the stack top and `rust_trap` prints the named cause + mcause/mepc/mtval, then
  exits fail, with a re-entry guard. Verified by injecting an unmapped load:
  `TRAP: load access fault  mcause 0x5 mepc 0x80002858 mtval 0x0`, exit 1 —
  previously a 120 s silent `timeout`.
- Link args moved from `.cargo/config.toml` rustflags to `build.rs`
  (`cargo::rustc-link-arg-bins`, absolute `-T` path, `rerun-if-changed`), which
  RUSTFLAGS cannot clobber. `ci.yml` job `env:` block deleted; plain workflow
  `-D warnings` now verified to link and run green.
- `_start` clears `mstatus.MIE` + `mie` so the allocator's `unsafe impl Sync`
  rests on enforced state, not on reset defaults.
- Linker `ASSERT(__ebss + 0x10000 < _stack_start)` — verified it fires.
- Local `misdecode_protection`/`correction_capacity` (ISO Formula 3); suite 4 is
  now a `(version, level, published r)` table cross-checked against the formula,
  8 cases; suite 3's 5-Q asserts 4 blocks of capacity 9.
- `checked_next_multiple_of` in the allocator; micro probe asserts it found a
  boundary rather than hitting its ceiling; CI gained a clippy step for the
  excluded crate; doc comments corrected on `codeword_modules` (independent
  hand-maintained duplicate) and on building from inside the crate directory.

`cargo test --workspace` green (exit 0), root `cargo fmt --all --check` clean,
esp-selftest fmt + clippy clean, QEMU `ALL PASS` exit 0.
See handoff/REVIEW-REQUEST-esp-selftest.md.

---

## Known Gaps

- **KG-1** — CLOSED by Step 3b (v5-Q 4-block corruption test, errors_corrected == 12)
- **KG-2** — CLOSED by Step 3b (independent zigzag walk pinned to ISO Annex I stream)
- **KG-3** — CLOSED by Step 3b (Annex I EC bytes pinned to PDF values, verified by Richard-I against §I.2.3)
- **KG-4** — decode: Kanji exposed as raw Shift-JIS bytes, no SJIS→UTF-8 table; structured append parsed-and-skipped, not reassembled — logged 2026-07-23
- **KG-5** — CLOSED by Step 3b (border clamping + full_scan_interval cadence + ROI-miss fallback tests)
- **KG-6** — CLOSED 2026-08-11 (7f75a0b): optimal DP mixed-mode segmentation (brute-force verified), Kanji opt-in via Options.assume_shift_jis (default off — UTF-8 CJK stays byte mode), ECI designators. Richard: round 2 CLEAR, zero outstanding
- **KG-7** — detect: severe perspective on high versions uses single BR alignment anchor, not full interior alignment grid — logged 2026-07-23
- **KG-8** — PARTIALLY CLOSED by Step 5a: qr-core micro foundation built behind the default-on `micro` feature (tables, format info, 4 masks, function map, placement iterator). Encode/decode/detect remain unsupported until Steps 5b–5d. Original entry: Micro QR (M1–M4) unsupported across all crates. ISO 18004:2024 includes it (sizes 9+2*M, single finder, own format-info/EC/mode tables; M1 numeric-only). Cleanly additive to current design; detect needs a single-finder algorithm. If built: `micro` cargo feature, parallel types (no shared Symbol enum). Awaiting Project Owner decision — logged 2026-07-23
- **KG-9** — detect: rotation envelope for large versions at steep angles (v20/v40 fail 35–40°, pass 45°) is bounded by the module estimator's own +6–8% high bias in the mixed chord-taper regime (Richard-F measured; NOT a raster precision floor). Next lever: filter corner-tapered scanlines before averaging — logged 2026-07-23
- **KG-10** — qr-detect not rustfmt-clean prior to Step 4; fmt applied in Step 4 commit — logged 2026-07-23
- **KG-11** — CLOSED 2026-08-11 (627c69e): layered retry — /8 primary, /16 full-frame rescan when nothing decodes (decode-aware trigger, feature-graded). Vignette to 0.97 QR / >=0.999 micro via public entry points; happy path 1.01x. Per-pixel two-scale discriminators measured unsound and rejected. 3 review rounds
- **KG-12** — CLOSED 2026-08-11 (0c36cdc): interop verification REFUTED the low-nibble convention — segno + zxing-cpp both left-justify (nibble<<4). Fixed both sides; 40/40 both directions; golden external vectors pinned in tests-integration/tests/micro_golden.rs
- **KG-13** — micro detect: a bridged timing module can truncate a symbol into a self-consistent smaller crop scoring 0 function mismatches (Richard-MT observation); rare, needs a discriminator beyond function-pattern validation — logged 2026-08-10
- **KG-14** — micro detect: function_mismatches hardcodes the micro function map, tied to qr-core only by debug_assert — logged 2026-08-10
- **KG-15** — qr-video: fixtures never decode, so its suite only exercises the retry double-pass path; detect_full single-pass-hit case uncovered. Add one decodable fixture next time in that crate (Richard-BIN MINOR-6) — logged 2026-08-11
- **KG-16** — detect: multi-code vignette frame where /8 finds one code suppresses retry for a second code needing /16 — documented crate-docs limitation, accepted — logged 2026-08-11
- **KG-13 update (Step 10)** — relative timing gap band moved the bridged-crop surface +2/-1 in 810 damaged-timing warps (measured, recorded at the code); clamp exclusive of 2.0 prevents index renumbering — 2026-08-11
- **KG-18** — micro detect: quiet-zone ring check applies to refined (perspective) path only, not the affine fast path — deliberate asymmetry (fast path predates it, regression risk), rationale in REVIEW-FEEDBACK-persp.md — logged 2026-08-11
- **KG-19** — micro detect: square_axes (aspect assumption in orientation frames) is the next envelope limiter for M4/far-edge-compressed directions past 12% keystone — logged 2026-08-11
- **KG-17** — detect: a *featureless* frame binarizes twice. `binarize.rs:90-96` discards the completed Bradley pass (integral image + 921,600 window evaluations) when `dark_count == 0 || dark_count == total` and runs a full Otsu pass over the whole frame on top of it. A uniform frame always hits `dark_count == 0`, so it pays for two binarizations instead of one; `detect_luma` then finds nothing and retries, doing both again. Measured in Step 9 on 720p: flat frame 6.88 ms vs cluttered symbol-free frame 4.07 ms vs happy path 2.13 ms; single-pass 3.44 ms flat vs 2.04 ms cluttered, and that 1.40 ms delta is exactly one Otsu pass over 921,600 px. NOT finder-scan cost — the resulting binary image is uniform, so the finder scan on a flat frame is the cheapest it ever gets (root cause identified by Richard-BM, correcting Bob's initial guess). Fix is small and local to `binarize.rs`: decide before paying for the adaptive pass, or reuse its statistics. Richard recommends its own step against qr-detect rather than leaving it here — logged 2026-08-11

---

## Architecture Decisions
*Locked decisions that cannot be changed without breaking the system.*

- Workspace layout: crates/ dir, 6 crates: qr-core (no_std foundation), qr-encode (no_std+alloc), qr-decode (no_std+alloc), qr-detect (std+image), qr-video (std, trait-based FrameSource, no hard capture dep), qr-render (std+image) — 2026-07-23
- All shared QR domain logic (GF256, RS, BCH, tables, masks, function-pattern map, BitMatrix) lives in qr-core; encode/decode must not duplicate it — 2026-07-23
- Coordinate convention (x, y) = (column, row), origin top-left — 2026-07-23
- Zero external deps except `image` (detect/render) and gated video I/O — 2026-07-23
- Build waves: 1=core, 2=encode+decode+render+detect parallel, 3=video+integration — 2026-07-23
