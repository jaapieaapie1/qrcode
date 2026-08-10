//! The bare-metal runtime: reset vector, heap, console and machine exit.
//!
//! Everything here is hand-rolled — the crate has no dependency outside the
//! three `qr-*` crates under test, which is the point: the self-test proves the
//! QR stack runs on a machine that has nothing but RAM, a UART and an ISA.
//!
//! ## Memory map (QEMU `virt`)
//!
//! | Address       | Device                                          |
//! |---------------|-------------------------------------------------|
//! | `0x0010_0000` | SiFive test finisher — writing terminates QEMU   |
//! | `0x1000_0000` | NS16550A UART0                                  |
//! | `0x8000_0000` | RAM — the ELF is loaded here, `_start` is first  |

use core::alloc::{GlobalAlloc, Layout};
use core::cell::{Cell, UnsafeCell};
use core::fmt::{self, Write};
use core::ptr::{NonNull, null_mut, read_volatile, write_volatile};

// ---------------------------------------------------------------------------
// Reset vector
// ---------------------------------------------------------------------------

// `_start` runs on a hart with no stack, no zeroed `.bss` and no C runtime. It
// establishes the stack pointer from the linker-provided `_stack_start`, masks
// interrupts, installs the trap vector, clears `.bss` word by word (the linker
// aligns both bounds to 4), and jumps into Rust. It never returns —
// `rust_main` always exits through the test finisher.
core::arch::global_asm!(
    ".section .text.init, \"ax\", @progbits",
    ".globl _start",
    "_start:",
    "    la    sp, _stack_start",
    // Interrupts off by construction rather than by assumption: `mstatus.MIE`
    // and every source in `mie` are already zero out of reset, but the
    // allocator's `unsafe impl Sync` rests on that, so establish it explicitly.
    "    csrci mstatus, 8",
    "    csrw  mie, zero",
    // Trap vector, direct mode — the low two bits of `mtvec` select the mode
    // and `_trap` is 4-byte aligned, so they are zero.
    "    la    t0, _trap",
    "    csrw  mtvec, t0",
    "    la    t0, __sbss",
    "    la    t1, __ebss",
    "1:  bgeu  t0, t1, 2f",
    "    sw    zero, 0(t0)",
    "    addi  t0, t0, 4",
    "    j     1b",
    "2:  j     rust_main",
    // Every exception lands here. The faulting context is not resumable and the
    // firmware is about to exit, so no registers are saved — but `sp` is
    // re-pointed at the top of the stack first, so the handler can still run,
    // and still print, when the fault *was* a stack overflow.
    ".balign 4",
    ".globl _trap",
    "_trap:",
    "    la    sp, _stack_start",
    "    csrr  a0, mcause",
    "    csrr  a1, mepc",
    "    csrr  a2, mtval",
    "    j     rust_trap",
);

// ---------------------------------------------------------------------------
// Console — NS16550A UART0
// ---------------------------------------------------------------------------

/// Base of the NS16550A that `-machine virt` wires to the QEMU stdio.
const UART0: *mut u8 = 0x1000_0000 as *mut u8;
/// Line Status Register offset.
const UART_LSR: usize = 5;
/// LSR bit 5: transmitter holding register empty.
const UART_LSR_THRE: u8 = 0x20;

/// A zero-sized handle over UART0, so `core::fmt` can drive it.
pub struct Uart;

impl Uart {
    fn put(byte: u8) {
        // SAFETY: UART0 is a device mapping that `virt` always provides; the
        // registers are byte-wide and this is the only writer on the machine.
        unsafe {
            while read_volatile(UART0.add(UART_LSR)) & UART_LSR_THRE == 0 {}
            write_volatile(UART0, byte);
        }
    }
}

impl Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.as_bytes() {
            // The QEMU console is line-oriented; keep CRLF so the output is
            // readable whether or not the terminal translates.
            if *byte == b'\n' {
                Uart::put(b'\r');
            }
            Uart::put(*byte);
        }
        Ok(())
    }
}

/// Backing function for [`print!`] and [`println!`].
pub fn _print(args: fmt::Arguments<'_>) {
    // Writing to a UART cannot fail, so the result is genuinely uninteresting.
    let _ = Uart.write_fmt(args);
}

/// `print!` over UART0.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::runtime::_print(format_args!($($arg)*)) };
}

/// `println!` over UART0.
#[macro_export]
macro_rules! println {
    () => { $crate::runtime::_print(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::runtime::_print(format_args!("{}\n", format_args!($($arg)*))) };
}

// ---------------------------------------------------------------------------
// Machine exit — SiFive test finisher
// ---------------------------------------------------------------------------

/// The `sifive_test` MMIO device `-machine virt` maps at 0x0010_0000.
const TEST_FINISHER: *mut u32 = 0x0010_0000 as *mut u32;
/// Status word meaning "pass": QEMU exits 0.
const FINISH_PASS: u32 = 0x5555;
/// Status word meaning "fail": QEMU exits with the code in the top 16 bits.
const FINISH_FAIL: u32 = 0x3333;

/// Terminates QEMU: exit status 0 on `pass`, 1 otherwise.
pub fn exit(pass: bool) -> ! {
    let word = if pass {
        FINISH_PASS
    } else {
        (1u32 << 16) | FINISH_FAIL
    };
    // SAFETY: a 32-bit store to the finisher; the device stops the machine
    // before the store retires as far as the guest is concerned.
    unsafe { write_volatile(TEST_FINISHER, word) };
    // Unreachable in practice — park the hart if a QEMU build ever ignores it.
    loop {
        core::hint::spin_loop();
    }
}

/// Guards against a fault raised while the trap handler is itself running,
/// which would otherwise re-enter forever.
struct TrapGuard(Cell<bool>);

// SAFETY: single hart, and interrupts are masked in `_start` and never enabled.
unsafe impl Sync for TrapGuard {}

static IN_TRAP: TrapGuard = TrapGuard(Cell::new(false));

/// The standard machine exception causes of the RISC-V privileged spec.
fn exception_name(cause: usize) -> &'static str {
    match cause {
        0 => "instruction address misaligned",
        1 => "instruction access fault",
        2 => "illegal instruction",
        3 => "breakpoint",
        4 => "load address misaligned",
        5 => "load access fault",
        6 => "store/AMO address misaligned",
        7 => "store/AMO access fault",
        8 | 9 | 11 => "environment call",
        12 => "instruction page fault",
        13 => "load page fault",
        15 => "store/AMO page fault",
        _ => "reserved or custom cause",
    }
}

/// Machine trap handler, entered from `_trap` with the cause registers already
/// in the argument registers.
///
/// This is the whole reason the crate is worth having: a misaligned load, an
/// illegal instruction or a wild pointer is exactly the class of bug that only
/// appears on riscv32imc and that no host test can reach. Without a vector,
/// `mtvec` reads zero, the hart jumps to unmapped address `0` and spins until
/// CI's `timeout` fires — a two minute stall with an empty log. With it, the
/// same fault prints its cause and exits non-zero immediately.
#[unsafe(no_mangle)]
pub extern "C" fn rust_trap(mcause: usize, mepc: usize, mtval: usize) -> ! {
    if IN_TRAP.0.replace(true) {
        // Faulted inside the handler. Leave without recursing or printing.
        exit(false);
    }

    let interrupt = mcause & (1 << (usize::BITS - 1)) != 0;
    let cause = mcause & !(1 << (usize::BITS - 1));
    let kind = if interrupt {
        "interrupt (should be masked)"
    } else {
        exception_name(cause)
    };

    println!();
    println!("TRAP: {kind}");
    println!("  mcause {mcause:#010x}  mepc {mepc:#010x}  mtval {mtval:#010x}");
    println!("ALL PASS not reached");
    exit(false)
}

// ---------------------------------------------------------------------------
// Heap — bump allocator with last-block reuse
// ---------------------------------------------------------------------------

/// 192 KiB, comfortably inside the ~400 KiB of DRAM an ESP32-C3 has, and the
/// number the peak report at the end of the run is measured against.
pub const HEAP_SIZE: usize = 192 * 1024;

#[repr(align(16))]
struct HeapStorage(UnsafeCell<[u8; HEAP_SIZE]>);

// SAFETY: the machine is single-hart, and `_start` clears `mstatus.MIE` and
// `mie` before any Rust runs and nothing ever re-enables them, so the allocator
// below is the only accessor and never re-enters.
unsafe impl Sync for HeapStorage {}

/// All-zero, so it lands in `.bss` and costs nothing in the ELF image.
static HEAP: HeapStorage = HeapStorage(UnsafeCell::new([0; HEAP_SIZE]));

/// Bump allocator with two refinements that matter for `alloc::vec::Vec`:
/// freeing the most recent block rolls the bump pointer back, and growing the
/// most recent block extends it in place. Without those, every `Vec` growth
/// would leak its old buffer and the encode/decode sweeps would exhaust the
/// heap; with them, a sequence of build-then-drop workloads reuses the same
/// bytes over and over.
struct Bump {
    /// Offset of the first free byte, relative to the heap base.
    next: Cell<usize>,
    /// High-water mark of `next`, reported at the end of the run.
    peak: Cell<usize>,
    /// Address of the most recent live allocation, or 0 for "none".
    last: Cell<usize>,
}

// SAFETY: as for `HeapStorage` — single hart, interrupts masked in `_start`.
unsafe impl Sync for Bump {}

impl Bump {
    const fn new() -> Bump {
        Bump {
            next: Cell::new(0),
            peak: Cell::new(0),
            last: Cell::new(0),
        }
    }

    fn base(&self) -> usize {
        HEAP.0.get() as usize
    }
}

unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let base = self.base();
        // Checked, not plain, `next_multiple_of`: release builds have overflow
        // checks off, so a wrapping round-up would produce a small `start` that
        // then passes the bounds test below and hands back a pointer near
        // address zero. Unreachable with the alignments the QR stack asks for,
        // but the guard is free and matches the `checked_add` below.
        let Some(start) = (base + self.next.get()).checked_next_multiple_of(layout.align()) else {
            return null_mut();
        };
        let Some(end) = start.checked_add(layout.size()) else {
            return null_mut();
        };
        if end > base + HEAP_SIZE {
            return null_mut();
        }
        self.next.set(end - base);
        if self.next.get() > self.peak.get() {
            self.peak.set(self.next.get());
        }
        self.last.set(start);
        start as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        if ptr as usize == self.last.get() {
            self.next.set(ptr as usize - self.base());
            // The rolled-back block must not be reused as an in-place grow
            // target by a later `realloc`.
            self.last.set(0);
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let base = self.base();
        if ptr as usize == self.last.get() {
            let end = ptr as usize + new_size;
            if end <= base + HEAP_SIZE {
                self.next.set(end - base);
                if self.next.get() > self.peak.get() {
                    self.peak.set(self.next.get());
                }
                return ptr;
            }
            return null_mut();
        }
        // SAFETY: forwarded unchanged from the caller's contract.
        unsafe {
            let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
            let fresh = self.alloc(new_layout);
            if let Some(dst) = NonNull::new(fresh) {
                core::ptr::copy_nonoverlapping(
                    ptr,
                    dst.as_ptr(),
                    core::cmp::min(layout.size(), new_size),
                );
                self.dealloc(ptr, layout);
            }
            fresh
        }
    }
}

#[global_allocator]
static ALLOCATOR: Bump = Bump::new();

/// Bytes of heap ever handed out at once — the number that says whether this
/// workload would fit a real ESP32-C3.
pub fn heap_peak() -> usize {
    ALLOCATOR.peak.get()
}

/// Releases the whole arena, as an embedded firmware would between phases of
/// work. The high-water mark is deliberately kept, so [`heap_peak`] still
/// reports the largest a single phase ever grew.
///
/// # Safety
///
/// Every allocation made since the previous reset is invalidated. The caller
/// must guarantee none is still live. The self-test calls this only between
/// suites, and a suite hands back nothing but a count.
pub unsafe fn heap_reset() {
    ALLOCATOR.next.set(0);
    ALLOCATOR.last.set(0);
}

// ---------------------------------------------------------------------------
// Panic
// ---------------------------------------------------------------------------

/// Prints the panic message and location, then exits QEMU with a failure
/// status. This is also the path a failed allocation takes: the default
/// `handle_alloc_error` panics.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    println!();
    println!("PANIC: {}", info.message());
    if let Some(loc) = info.location() {
        println!("  at {}:{}:{}", loc.file(), loc.line(), loc.column());
    }
    println!("ALL PASS not reached");
    exit(false)
}
