/* Linker script for the QEMU `virt` RISC-V machine.
 *
 * `-machine virt -bios none -kernel <elf>` maps RAM at 0x8000_0000 and starts
 * the hart at the ELF entry point, so everything lives in one RAM region and
 * `_start` only has to set up `sp` and clear `.bss`.
 *
 * LENGTH is 8 MiB — far more than the firmware needs (it is well under the
 * 400 KiB an ESP32-C3 has), but the stack top is derived from it and QEMU's
 * `virt` default of 128 MiB covers it comfortably.
 */

OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY
{
  RAM (rwx) : ORIGIN = 0x80000000, LENGTH = 8M
}

/* Full descending stack, growing down from the top of RAM. */
_stack_start = ORIGIN(RAM) + LENGTH(RAM);

SECTIONS
{
  .text : ALIGN(4)
  {
    KEEP(*(.text.init))
    *(.text .text.*)
    . = ALIGN(4);
  } > RAM

  .rodata : ALIGN(4)
  {
    *(.rodata .rodata.*)
    *(.srodata .srodata.*)
    . = ALIGN(4);
  } > RAM

  .data : ALIGN(4)
  {
    *(.data .data.*)
    *(.sdata .sdata.*)
    . = ALIGN(4);
  } > RAM

  .bss (NOLOAD) : ALIGN(4)
  {
    __sbss = .;
    *(.sbss .sbss.*)
    *(.bss .bss.*)
    *(COMMON)
    . = ALIGN(4);
    __ebss = .;
  } > RAM

  /DISCARD/ :
  {
    *(.eh_frame)
    *(.eh_frame_hdr)
    *(.comment)
    *(.riscv.attributes)
  }

  /* There is no guard page: a runaway stack walks straight into `.bss` and
   * corrupts the heap silently. Today ~7.7 MiB separates them, but a future
   * heap enlargement must not be allowed to quietly eat the gap, so require at
   * least 64 KiB of daylight at link time. */
  ASSERT(__ebss + 0x10000 < _stack_start, "stack would collide with .bss")
}
