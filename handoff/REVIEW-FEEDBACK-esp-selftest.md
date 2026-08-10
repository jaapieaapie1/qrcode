# Review Feedback — Step 7: ESP32-class execution test in CI

**Reviewer:** Richard-E32
**Date:** 2026-08-11
**Verdict: Step 7 CLEAR**
**Ready for Builder: YES**

Counts: **0 BLOCKER, 1 MAJOR, 8 MINOR.**

---

## What I actually did

Not a read-through. I ran it.

| Check | Result |
|---|---|
| `git diff HEAD --stat` | 3 files: `ci.yml` (+44), `Cargo.toml` (+3), `BUILD-LOG.md` (+37). **Nothing under `crates/` touched.** Claim verified. |
| `cargo build --release` + QEMU 11.0.3 (local) | `ALL PASS`, **exit 0**. Transcript matches Bob's line for line, heap peak 75348 identical. |
| `RUSTFLAGS="-D warnings"` (bare, clean target dir) | Link fails: `undefined symbol: __ebss`. Bob's diagnosis confirmed — loud, not silent. |
| `RUSTFLAGS="-D warnings -C link-arg=-Tlink.ld -C link-arg=--nmagic"` (the exact CI string), clean target dir | Builds and runs green. |
| CI run step replayed verbatim under `bash -e` + `set -o pipefail` | `pipeline_status=0`, `grep_status=0`, `STEP_EXIT=0`. |
| `cargo fmt --check`, `cargo clippy --release` in `esp-selftest` | Both clean. |
| `cargo test --workspace` | Green, exit 0. `cargo metadata` members unchanged — `esp-selftest` correctly absent. |
| ELF layout (`objdump -h`, `nm`) | `_start` @ `0x80000000` (first), `__sbss` `0x80010550`, `__ebss` `0x8004055c` (both 4-aligned), `_stack_start` `0x80800000` (16-aligned). `.bss` = `0x3000c` ≈ 192 KiB heap + 12 B allocator state. **No orphan sections**, `.data` empty, LMA==VMA under `--nmagic`. ~7.7 MiB between `__ebss` and the stack top. |
| Zigzag replay vs. encoder | `main.rs:87-108` is a **line-for-line match** of `qr-encode/src/matrix.rs:24-50`, including the `right == 6` timing-column skip and `upward = ((right+1) & 2) == 0`. Element `8*i` is genuinely the MSB of interleaved codeword `i`. Corruption granularity claim is honest. |
| r-cap arithmetic vs. `qr-decode/src/extract.rs:146-173` | `p`: 1-L=3, 1-M=2, 2-L=2, 1-Q=1, 5-Q=0. So r = 2 / 4 / 4 / 6 / 9. **Every case in suite 4 sits exactly on the real boundary**, and the 1-L 3-error case is genuinely 3 errors against r=2. Confirmed. |

### Negative verification (performed, then reverted)

I mutated `main.rs:352` from `decoded.errors_corrected == errors` to
`== errors + 1` — deliberately a *different* assertion from the two Bob broke,
and in the middle suite, so it tests both the short-circuit and the exit path.

```
[1/4] qr roundtrip, v1-v10 sampled.......... PASS (29 symbols)
[2/4] micro qr, all 8 symbols............... PASS (8 symbols)
[3/4] reed-solomon recovery.................
    FAIL: v1 L: reported 2 corrections, injected 2
[3/4] reed-solomon recovery: FAILED
heap peak 75348 of 196608 bytes (largest single suite)
FAILED
STEP_EXIT=1
```

Suite 4 correctly skipped, `FAILED` banner (so the `grep -q '^ALL PASS'` gate
also fails), finisher wrote `0x13333`, QEMU exited 1, `pipefail` propagated it,
`bash -e` failed the step. **A failing assertion does reach exit-fail** — it is
not print-only.

**Restored byte-for-byte.** `md5 src/main.rs` = `6acceae83eab19f91e2b604a35ef650d`
before the edit and after the revert; `diff -q` against a pre-edit copy reports
identical. Rebuilt and re-ran: `ALL PASS`, exit 0. Working tree is exactly as I
found it — no `run.log`, no stray files.

---

## Soundness findings

### The allocator is correct, including the part that usually is not

You asked me to look hardest at alignment. `Bump::alloc`
(`runtime.rs:178-193`) computes `start = (base + next).next_multiple_of(layout.align())`
and returns `start`. `layout.align()` is respected for every allocation, not
just assumed from the arena's `#[repr(align(16))]`. OOM returns `null_mut()` on
both the `checked_add` overflow path and the arena-exhausted path — no UB, and
the default `handle_alloc_error` turns it into a panic, which Bob demonstrated.

The invariant that makes `realloc`'s in-place grow sound — *if `ptr == last`
then `next == ptr + old_size`* — holds, because `alloc` sets `last` and `next`
together and `dealloc` clears `last` to `0` whenever it rolls `next` back. That
`last.set(0)` on line 200 is the load-bearing line; without it a later `realloc`
would extend a freed block. The comment on 198-199 shows Bob knew it. `0` is a
safe sentinel because the arena base is `0x8001_0550`.

`realloc` returning `null_mut()` from the in-place path rather than falling back
to a fresh allocation (line 215) is correct, not a bug: if `ptr == last`, an
in-place grow needs strictly *less* headroom than a fresh block would.

`heap_reset` clears **both** `next` and `last`. Clearing only `next` would leave
a stale `last` pointing into the reset region and the next `realloc` could
extend over a live allocation. Bob got this right. The `# Safety` clause
(`runtime.rs:246-251`) states the precondition; the call site (`main.rs:453-455`)
carries a matching `SAFETY:` note, and the precondition genuinely holds — a
suite's return type is `Result<usize, Failed>`, both `Copy`, so nothing
allocated survives the boundary. `heap_peak` is deliberately not reset and that
is documented.

Panic handler: `core::fmt` only, no allocation, no recursion, ends in `exit(false)`.
The OOM path routes through it and Bob demonstrated it. Clean.

MMIO: both UART accesses are `read_volatile`/`write_volatile`; the finisher is a
single `write_volatile` of `u32`. `(1<<16) | 0x3333` is the correct `sifive_test`
encoding — QEMU takes `value & 0xffff == 0x3333` as fail and exits with
`value >> 16`, i.e. 1. Verified empirically above. The post-write `spin_loop`
park is the right belt-and-braces.

Boot: `_start` is first in RAM *and* is `ENTRY`, so it works whether QEMU honours
the ELF entry or jumps to `0x8000_0000`. `bgeu` guards the zero-length `.bss`
case. Both bounds 4-aligned, confirmed from the symbol table. No `.data` copy
loop is needed and none is present — correct, because `--nmagic` with no `AT>`
gives LMA == VMA and QEMU's ELF loader writes `.data` straight into RAM. That is
the right call and it stays right if `.data` ever becomes non-empty.

No overlap between the allocator arena and any section: the heap is a `static`
*inside* `.bss`, so the linker accounts for it. That avoids the classic
"heap starts at `_end`" hack that silently collides. Good instinct.

---

## MAJOR

**1. No trap vector. `mtvec` is never set, so any exception is a 120-second
silent hang instead of a diagnosis.** — `esp-selftest/src/runtime.rs:28-40`

This is the one that matters, and it matters *because* of what this crate is for.
The entire point of Step 7 is to catch failures that only appear on riscv32imc
and that no host test can reach — a misaligned load, an illegal instruction, a
wild pointer. Every one of those traps. With `mtvec` at its reset value of 0,
the hart jumps to `0x0`, which is unmapped on `virt`, faults again, and spins
forever. CI is not *unsafe* — `timeout 120` fires, the pipeline status is 124,
`pipefail` propagates it and the `grep` gate fails too, so it cannot produce a
false pass. But the deliverable converts its single most valuable signal into a
two-minute stall with an empty log.

Fix: in `_start`, `la t0, _trap; csrw mtvec, t0`, and give `_trap` a handler that
reads `mcause`/`mepc`/`mtval`, prints them over the UART and calls `exit(false)`.
Roughly fifteen lines, and it turns "CI hung, no idea why" into
"`mcause=4 mepc=0x8000abcd` — misaligned load". Do this before the next arch is
added, not after.

---

## MINOR

**2. Flag duplication — ruling below.** `.cargo/config.toml:9` and `ci.yml:72`
must stay in sync by hand; nothing enforces it. See the ruling section.

**3. `esp-selftest` gets no Clippy in CI.** — `.github/workflows/ci.yml:88-90`
The `lint` job's `cargo clippy --workspace --all-targets` cannot see an excluded
crate — the same reasoning that (correctly) earned the crate its own `fmt` step
applies to Clippy. `-D warnings` in the job env covers *rustc* warnings at build
time but not a single Clippy lint. Bob ran it locally and it is clean; make that
permanent. One line after the `fmt` step: `cargo clippy --release -- -D warnings`.

**4. The micro capacity probe never proves it found a boundary.** — `main.rs:298-304`
`for n in 1..=40` records the last size that fits. M4-L numeric is 35, so there
are five digits of headroom today and the probe is finding the true maximum. But
if a capacity ever reached 40 the probe would silently cap there, roundtrip a
sub-maximal payload and still report PASS — testing less than the comment on
295-296 claims. Add `ensure!(capacity < 40, "M{} {ec:?}: probe hit its 40-digit ceiling", version.number());`
so the ceiling can never be mistaken for the capacity.

**5. Suite 3 computes `r` without consulting `p`.** — `main.rs:375`
`let per_block = blocks.ec_per_block as usize / 2;` is the raw Reed-Solomon
bound. It is right for 5-Q only because `p == 0` there. The comment on 370-373
says "p = 0, so r = 9", so the reasoning is sound and stated — but the code does
not encode it, and pointing the case at a version with `p > 0` would over-inject
and fail confusingly. Either assert `misdecode_protection == 0` for the chosen
version or subtract `p` explicitly, mirroring `extract.rs:170-173`.

**6. `next_multiple_of` can wrap in release.** — `runtime.rs:180`
Release has `overflow-checks` off, so a `layout.align()` large enough to push
`base + next` past `usize::MAX` wraps to a small value; `end` then passes the
`end > base + HEAP_SIZE` test and the caller gets a pointer near address 0.
Unreachable in practice — `Layout` forbids aligns that large and nothing in the
QR stack asks for more than 16 — but the guard is free:
`let Some(start) = (base + self.next.get()).checked_next_multiple_of(layout.align()) else { return null_mut(); };`
You are already using `checked_add` two lines down; be consistent.

**7. A `SAFETY` comment asserts something the code does not establish.** —
`runtime.rs:138-139` and `160`
"runs with interrupts masked" justifies `unsafe impl Sync` on `HeapStorage` and
`Bump`. It is *true* — `mstatus.MIE` and `mie` are zero out of reset and nothing
enables them — but it is true by omission, not by construction. Either add
`csrw mie, zero` / `csrci mstatus, 8` to `_start` (cheap, and pairs naturally
with the `mtvec` fix in MAJOR 1), or reword to "interrupts are disabled out of
reset and this firmware never enables them". Do not leave an unsafe
justification resting on a property no line of code enforces.

**8. The linker script does not assert the stack clears `.bss`.** —
`esp-selftest/link.ld:21`
`_stack_start` is `ORIGIN + LENGTH` unconditionally. Today there is 7.7 MiB of
daylight between `__ebss` and the stack top, and there is no guard page — a
runaway recursion walks straight into the heap and corrupts it silently rather
than faulting. Add `ASSERT(__ebss < _stack_start - 0x10000, "stack/bss collision")`
at the end of `SECTIONS` so a future heap enlargement cannot quietly eat the gap.

**9. `codeword_modules` is a verbatim copy of the encoder's placement walk.** —
`main.rs:87-108` vs `qr-encode/src/matrix.rs:24-50`
It is presented as a "replay", but it is a duplicate, so it can drift from the
encoder without anything noticing. The saving grace is that the failure would be
loud rather than silent: suite 4 asserts *both* sides of four capacity
boundaries, and a wrong-granularity corruption would break one of them
immediately. Leaving it duplicated is defensible — an independent reimplementation
has review value that calling the encoder's function would lose — but say so in
the doc comment, because "identical to the walk the encoder uses" currently reads
as a guarantee when it is a hand-maintained coincidence.

**10. The crate only builds from inside its own directory.** —
`esp-selftest/.cargo/config.toml`
I checked: `cargo build --release --manifest-path esp-selftest/Cargo.toml` from
the repo root does **not** pick up `.cargo/config.toml` (discovery is
cwd-relative, not manifest-relative), so it builds for the *host* and dies in a
pile of mach-o/aarch64 assembler errors. CI is fine — it uses
`working-directory: esp-selftest` — and the failure is loud. But it will catch
somebody out. One line in the crate's module docs: "build from within
`esp-selftest/`; the target and link flags come from its `.cargo/config.toml`,
which Cargo discovers from the working directory."

---

## Ruling: `.cargo/config.toml` vs `build.rs`

**Move the link arguments to `build.rs`. Keep `[build] target` in
`.cargo/config.toml`.**

Bob's reasoning for keeping the status quo was that the brief specified it and
that a bare `cargo build --release` should keep working for a human in that
directory. The second point is the real one, and `build.rs` satisfies it just as
well — a build script runs unconditionally, on every invocation, from any
caller. So the argument reduces to "the brief said so", and a brief that predates
a discovered footgun does not outrank the footgun.

The case for moving:

- `cargo::rustc-link-arg-bins` is **additive**. `RUSTFLAGS` cannot clobber it.
  That deletes the entire failure class, not just today's instance of it.
- It deletes the duplication. Right now the flags live in two files that must
  agree, with nothing enforcing agreement, and the *only* consumer that reads
  `.cargo/config.toml` is a human — CI ignores it wholesale. That is the worst
  arrangement: a config file that looks authoritative and is not.
- It lets you pass an **absolute** path to the linker script
  (`-T{}/link.ld` from `CARGO_MANIFEST_DIR`) instead of the cwd-relative
  `-Tlink.ld`, which removes a second, quieter dependency on where cargo happens
  to be standing.
- Pair it with `cargo::rerun-if-changed=link.ld`, which you do not have today —
  editing the linker script currently does not force a relink.

What stays: `[build] target = "riscv32imc-unknown-none-elf"` must remain in
`.cargo/config.toml`, because target selection is not a rustflag and a build
script cannot set it. That is fine — it is not the thing `RUSTFLAGS` clobbers,
and MINOR 10 above already tells the reader where it comes from.

After the move, drop the two link args from the `ci.yml` job `env:` and leave
`RUSTFLAGS: -D warnings` — at which point the job can simply inherit the
workflow-level value and the `env:` block disappears entirely, along with its
five-line comment. That is the tell that this is the right shape.

**This does not block Step 7.** What is committed works, is verified end to end
on both flag paths, and fails loudly rather than quietly. Take it as the first
cleanup of Step 8.

---

## On the other two open questions

**The heap number.** Report it as Bob wrote it — "largest single suite, this
allocator" — and do not let 75 KB escape into any outward-facing claim without
that qualifier attached. The number is honest *and* narrow, and the paragraph at
lines 91-94 of the review request says so plainly, which is more than most people
would have written. Replacing the bump allocator with a free list to make the
figure more general **is** scope creep for Step 7 and Bob was right to refuse it.
If a general figure is ever wanted, that is its own step with its own brief.

**Version sampling.** 7 of 40 is the right call and the justification is
structural rather than arbitrary — single-block, interleaved multi-block, first
with version information, first with a 16-bit count. The host suite covers the
full sweep. Emulating all 40 buys duplicate coverage at real CI wall-clock cost.
Leave it. If anyone wants 1..=40, that is Arch's call, not a review finding.

---

## Cleared

Reviewed the whole of `esp-selftest/` (`Cargo.toml`, `.cargo/config.toml`,
`link.ld`, `src/runtime.rs`, `src/main.rs`), the root `Cargo.toml` `exclude`, and
the `esp32-emulation` job in `.github/workflows/ci.yml`. Built and executed the
firmware under QEMU on both the local and the exact CI flag strings; verified the
pass path exits 0, negative-verified that a broken assertion exits 1 through the
finisher and fails both CI gates, and restored the source byte-for-byte.
Confirmed by diff that no product crate was touched and by `cargo test --workspace`
that the `exclude` breaks nothing. The reset vector, linker map, allocator,
panic path and MMIO are sound; the corruption is genuinely codeword-granular and
the ISO 7.5.1 boundary cases sit exactly where the decoder's own `p` table puts
them. The one thing I want fixed before this crate grows is the trap vector.

**Step 7 CLEAR.**
