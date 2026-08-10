# Review Request — Step 7: ESP32-class execution test in CI

**Builder:** Bob-E32
**Date:** 2026-08-11 (round 2)
**Ready for Review: YES**

---

## Round 2 — response to Richard-E32

All ten findings addressed. Nothing deferred.

| # | Finding | Done |
|---|---|---|
| **1 MAJOR** | No trap vector — any fault is a 120 s silent hang | `_start` now installs `mtvec`; `_trap` re-points `sp` at the stack top, reads `mcause`/`mepc`/`mtval` and hands them to `rust_trap`, which names the cause and exits fail. Re-entry guarded. **Verified by injecting a fault** — transcript below. |
| 2 | Flag duplication | Ruling taken: link args moved to `build.rs` via `cargo::rustc-link-arg-bins`, absolute `-T` path, `rerun-if-changed=link.ld`. `[build] target` stays in `.cargo/config.toml`. The `env:` block and its five-line comment are gone from `ci.yml`. |
| 3 | No Clippy in CI | `cargo clippy --release -- -D warnings` step added, `clippy` component requested. |
| 4 | Micro probe never proves a boundary | `ensure!(capacity < PROBE_CEILING, …)`. |
| 5 | Suite 3 computes `r` without `p` | Local `misdecode_protection` / `correction_capacity` implementing ISO Formula (3); 5-Q now asserts 4 blocks of capacity 9 rather than assuming. |
| 6 | `next_multiple_of` can wrap in release | `checked_next_multiple_of`. |
| 7 | `SAFETY` rests on an unenforced property | `_start` now does `csrci mstatus, 8` and `csrw mie, zero`; comments reworded to say established, not assumed. |
| 8 | No stack/`.bss` collision guard | `ASSERT(__ebss + 0x10000 < _stack_start, …)`. **Verified it fires** — below. |
| 9 | `codeword_modules` presented as a replay, is a duplicate | Doc comment now says independent reimplementation, maintained by hand, and why sharing the code would test nothing. |
| 10 | Only builds from inside its own directory | Documented in the crate module docs with the working command. |

Your two rulings on the open questions are taken as written: the heap figure keeps
its "largest single suite, this allocator" qualifier and the bump allocator stays;
version sampling stays at 7 of 40.

Suite 4 went from 7 cases to 8 — it is now a table of `(version, level, published r)`
driven uniformly through `recovers(r)` + `rejects(r + 1)`, which added the missing
1-L `recovers(2)` side.

---

## What this is

Until now the ESP32 claim was a *compile* claim: `cargo check -p qr-core -p
qr-encode -p qr-decode --target riscv32imc-unknown-none-elf` in the
`cross-targets` job. It proved the crates type-check and codegen for the ISA. It
did not prove a single line of them ever executed there.

`esp-selftest/` closes that. It is a bare-metal firmware for the QEMU `virt`
RISC-V machine — hand-rolled reset vector, linker script, bump allocator, UART
driver, panic handler, zero dependencies outside the three crates under test —
that runs the encoder and decoder end to end on an emulated riscv32imc core and
reports the verdict through QEMU's exit status. CI executes it on every push.

The crate is deliberately **excluded from the workspace**. It only builds for
one bare-metal target and links against a linker script, so it must not be
swept into `cargo test --workspace` or the host-target lint jobs.

---

## Files

| File | Lines | What |
|---|---|---|
| `Cargo.toml` (root) | 12–14 | **The sole root edit:** `exclude = ["esp-selftest"]` plus its rationale comment. Nothing else in the file changed. |
| `esp-selftest/Cargo.toml` | 1–26 | Package, path-deps on qr-core/qr-encode/qr-decode with **default features** (so `micro` is on, exactly as a crates.io consumer would get it), `panic = "abort"` in both profiles, `lto`/`codegen-units = 1` in release. |
| `esp-selftest/.cargo/config.toml` | 1–9 | `[build] target` only — target selection is not a rustflag and a build script cannot set it. |
| `esp-selftest/build.rs` | 1–32 | Emits the linker script (absolute path) and `--nmagic` as `cargo::rustc-link-arg-bins`, plus `rerun-if-changed`. |
| `esp-selftest/link.ld` | 1–71 | QEMU `virt` memory map, section layout, stack/`.bss` collision `ASSERT`. |
| `esp-selftest/src/runtime.rs` | 1–330 | Reset vector, trap vector and handler, UART0 console, SiFive test finisher exit, bump allocator, panic handler. |
| `esp-selftest/src/main.rs` | 1–500 | The four self-test suites and the entry point. |
| `.github/workflows/ci.yml` | 64–101 | New `esp32-emulation` job. |

`esp-selftest/Cargo.lock` is committed (binary crate). `esp-selftest/target/`
is already covered by the root `.gitignore`'s `target/`.

---

## Design

### Memory map (QEMU `virt`)

| Address | Device | Used for |
|---|---|---|
| `0x0010_0000` | `sifive_test` finisher | `0x5555` → QEMU exits 0; `(1<<16)\|0x3333` → QEMU exits 1 |
| `0x1000_0000` | NS16550A UART0 | Progress and result lines on QEMU stdio. Polls LSR (offset 5) bit 5 (THRE) before each byte. |
| `0x8000_0000` | RAM (8 MiB declared) | ELF load address; `.text` / `.rodata` / `.data` / `.bss` |
| `0x8080_0000` | — | `_stack_start`; full descending stack from the top of the declared region |

8 MiB is the *declared* region only, so the stack top has a definition; QEMU
`virt` defaults to 128 MiB, so it is comfortably backed. Actual footprint is far
smaller — see the heap note below.

### Boot

`_start` (in `.text.init`, `KEEP`-ed first in `.text`, `ENTRY(_start)`) runs on a
hart with no stack and no zeroed `.bss`:

1. `la sp, _stack_start`
2. `csrci mstatus, 8` / `csrw mie, zero` — interrupts off *by construction*, since
   the allocator's `unsafe impl Sync` rests on it
3. `la t0, _trap` / `csrw mtvec, t0` — trap vector, direct mode (`_trap` is
   `.balign 4`, so the mode bits are zero)
4. word-loop `__sbss`..`__ebss` storing zero (the linker aligns both bounds to 4)
5. `j rust_main`

`rust_main` never returns — every path ends in the finisher.

### Traps

`_trap` re-points `sp` at the top of the stack before doing anything else, so the
handler still runs when the fault *was* a stack overflow, then passes
`mcause`/`mepc`/`mtval` to `rust_trap`. That names the cause from the privileged
spec's table, prints the three registers and exits fail. A `Cell<bool>` guard
means a fault raised inside the handler leaves immediately instead of recursing.

This is the fix for the MAJOR, and it earns its keep for exactly the reason you
gave: a misaligned load or a wild pointer is the bug class only this crate can
reach, and with `mtvec` at its reset value of zero every one of them was a
two-minute stall with an empty log.

### Heap

192 KiB `static` in `.bss` (all-zero, so it costs nothing in the ELF image — the
stripped binary is 99 KB), well inside the ~400 KiB of DRAM an ESP32-C3 has.

The allocator is a bump pointer with two refinements that are not decoration:
freeing the *most recent* block rolls the pointer back, and growing the most
recent block extends it in place. Without those, every `Vec` growth inside the
encoder and decoder would strand its old buffer.

**One thing worth your attention.** I first measured a 122 KB peak and it climbed
monotonically suite to suite (75 KB → 103 KB → 122 KB → 122 KB). That is not
working set, it is fragmentation: a bump allocator cannot reclaim blocks freed
out of LIFO order, and the encode/decode paths interleave their allocations.
Left alone, adding one more version to the sweep could have exhausted the heap
for reasons that have nothing to do with the code under test. So the harness
resets the arena between suites (`runtime::heap_reset`, an `unsafe fn` with the
precondition documented: a suite returns nothing but a `usize`, so nothing it
allocated is live). The reported peak is now the largest **single suite**:
**75 348 bytes**.

Read that number as "this workload's working set fits an ESP32-C3 several times
over", not as "a general-purpose allocator would use 75 KB" — it is still this
allocator's behaviour, including whatever fragmentation one suite accumulates.

---

## What actually runs

| Suite | Coverage |
|---|---|
| 1. QR roundtrip | Versions 1, 2, 3, 5, 7, 8, 10 × L/M/Q/H = 28 symbols, each filled to within three codewords of capacity in byte mode, encoded then decoded byte-exact with version, EC level and `errors_corrected == 0` all asserted. Sampling is not arbitrary: 1–3 are single-block, 5 is four-block interleaved, 7 is the first version carrying version information, 10 the first with a 16-bit byte-mode character count. Plus one auto-version/auto-mode URL symbol asserted back as UTF-8 text. **29 symbols.** |
| 2. Micro QR | All eight symbols of ISO Table 13 — M1-Detection, M2-L/M, M3-L/M, M4-L/M/Q. Numeric capacity is **probed** (encode 1..40 digits, take the last that fits) rather than tabulated here, and the largest fitting payload is roundtripped; that exercises the capacity arithmetic instead of restating it. M3 and M4 additionally roundtrip a byte-mode payload. **8 symbols.** |
| 3. Reed-Solomon recovery | 1-L at r=2, 1-H at r=8, and 5-Q at the full 36 (four interleaved blocks × 9, the hardest multi-block case), each recovered to the exact original payload with `errors_corrected` matching the injected count. **3 cases.** |
| 4. ISO 7.5.1 misdecode protection | The r-cap from commit 6174b10, executed on the target. A table of `(version, level, r published in ISO Table 9)` — 1-L r=2, 1-M r=4, 2-L r=4, 1-Q r=6 — each run through `recovers(r)` then `rejects(r + 1)`. The published literal is checked against a local implementation of Formula (3) before it is used, so neither the table nor the formula is trusted alone. The headline is 1-L: **3 errors must be rejected** though Reed-Solomon could solve them; 1-Q is the control where p=1 does not narrow the bound. **8 cases.** |

Corruption is exact, not approximate. `codeword_modules()` replays the ISO 7.7.3
two-column zigzag — the same walk the encoder places data with — so element
`8*i` is the MSB of interleaved codeword `i`, and flipping it injects exactly
one erroneous codeword. Damaging *modules* would only have bounded the codeword
error count from above, which is useless for a test that has to sit precisely on
a capacity boundary.

---

## Local run transcript

```
$ cd esp-selftest && cargo build --release
    Finished `release` profile [optimized] target(s) in 0.75s
$ qemu-system-riscv32 -machine virt -nographic -bios none \
    -kernel target/riscv32imc-unknown-none-elf/release/esp-selftest

qr-selftest: riscv32imc-unknown-none-elf on qemu virt
             qr-core + qr-encode + qr-decode, no_std + alloc

[1/4] qr roundtrip, v1-v10 sampled.......... PASS (29 symbols)
[2/4] micro qr, all 8 symbols............... PASS (8 symbols)
[3/4] reed-solomon recovery................. PASS (3 cases)
[4/4] iso 7.5.1 misdecode protection........ PASS (8 cases)

heap peak 75348 of 196608 bytes (largest single suite)
ALL PASS
EXIT=0
```

QEMU 10.x from Homebrew, macOS arm64. Both device addresses worked as briefed on
the first attempt; nothing needed adjusting.

### Negative controls

An always-green harness proves nothing, so every failure path is exercised
deliberately and reverted.

**Trap handler (round 2, the MAJOR)** — injected an unmapped load,
`read_volatile(0 as *const u32)`, at the top of `rust_main`:

```
TRAP: load access fault
  mcause 0x00000005  mepc 0x80002858  mtval 0x00000000
ALL PASS not reached
EXIT=1
```

Cause decoded, faulting PC and address reported, exit 1, immediate. Before the
fix this exact fault was a 120-second `timeout` with nothing in the log.

**Linker `ASSERT` (round 2)** — raised `HEAP_SIZE` to 8 MiB:

```
rust-lld: error: stack would collide with .bss
```

Fails at link time, not at run time, and says why.

**Failure reporting (round 2)** — changed the 1-Q table entry's published `r`
from 6 to 7, so the formula check catches it:

```
[4/4] iso 7.5.1 misdecode protection........
    FAIL: v1 Q: capacity is 6, ISO Table 9 says 7
[4/4] iso 7.5.1 misdecode protection: FAILED

heap peak 75348 of 196608 bytes (largest single suite)
FAILED
PIPELINE_EXIT=1
GREP_GATE=1
```

Both CI gates fail independently, as they did in round 1.

**Panic handler and OOM (round 1)** — shrank the heap to 2 KiB:

```
[1/4] qr roundtrip, v1-v10 sampled..........
PANIC: memory allocation of 44 bytes failed
  at .../library/alloc/src/alloc.rs:573:9
ALL PASS not reached
EXIT=1
```

All reverted; the pass transcript above was re-run from the restored tree.

---

## CI job

`esp32-emulation`, ubuntu-latest, matching the existing style
(`checkout@v5`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`
scoped with `workspaces: esp-selftest`):

1. `apt-get install -y qemu-system-misc` (this is the package that carries
   `qemu-system-riscv32`), then print its version.
2. `cargo fmt --check` and `cargo clippy --release -- -D warnings` in
   `esp-selftest` — neither the `lint` job's `cargo fmt --all` nor its
   `cargo clippy --workspace` can see an excluded crate, so both need their own
   gate.
3. `cargo build --release` from `esp-selftest`.
4. Run under `timeout 120` with `set -o pipefail`, and **additionally**
   `grep -q '^ALL PASS' run.log`.

Two independent assertions on purpose: the exit status comes from the SiFive
finisher, and the banner grep catches a QEMU that ignored the finisher or a run
killed by `timeout` (the GitHub Actions default shell is `bash -e` *without*
pipefail, hence the explicit `set -o pipefail` before the `tee`).

### The RUSTFLAGS trap, and why the job now has no `env:` block

Round 1 discovered that a `RUSTFLAGS` environment variable **replaces** the
`[target.*] rustflags` config table rather than merging with it, so the
workflow-level `RUSTFLAGS: -D warnings` silently dropped `-T link.ld`:

```
$ RUSTFLAGS="-D warnings" cargo build --release
rust-lld: error: undefined symbol: __ebss
```

Round 1 worked around it by restating the flags in a job-level `env:`. Round 2
takes your ruling and removes the cause instead: `build.rs` emits them as
`cargo::rustc-link-arg-bins`, which is additive and cannot be clobbered. The
`env:` block and its comment are deleted, and the job simply inherits the
workflow value. Re-verified from a clean target directory:

```
$ RUSTFLAGS="-D warnings" cargo build --release   # the exact CI environment
    Finished `release` profile [optimized] target(s)
$ qemu-system-riscv32 ...                          → ALL PASS, exit 0
```

The absolute `-T` path also removes the dependency on cargo's working directory
for the *link* (target selection still needs it — MINOR 10, now documented).

---

## Verification

Round 2, all re-run after the changes:

- `cargo test --workspace` — green, exit 0 (20 test binaries), unchanged by the `exclude`.
- `cargo fmt --all --check` at root — clean.
- `esp-selftest`: `cargo fmt --check` clean, `cargo clippy --release -- -D warnings` clean.
- QEMU run — `ALL PASS`, exit 0, `grep` gate passes.
- `RUSTFLAGS="-D warnings"` from a clean target dir — links and runs green.
- Trap handler, linker `ASSERT` and a broken assertion — all fail as intended, then reverted.

---

## Open questions

None outstanding. Round 1's three were resolved: flag duplication by the
`build.rs` move; the heap figure keeps its "largest single suite, this
allocator" qualifier and the bump allocator stays; version sampling stays at
7 of 40.

One thing to carry forward rather than fix here: `codeword_modules` is a
hand-maintained duplicate of the encoder's placement walk (MINOR 9). That is
deliberate — sharing the code would make the test tautological — but it is a
standing maintenance obligation, now stated in the doc comment rather than
implied.

No Known Gaps were opened. Nothing outside Step 7 was touched.
