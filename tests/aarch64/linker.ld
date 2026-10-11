/* The AArch64 boot test's kernel image: QEMU's virt machine loads it at
 * the start of RAM (plus the usual 512 KiB) and enters _start at EL1 with
 * the MMU off. */
ENTRY(_start)
SECTIONS {
    . = 0x40080000;
    .text : { *(.text.boot) *(.text*) }
    .rodata : { *(.rodata*) }
    .data : { *(.data*) }
    .bss (NOLOAD) : ALIGN(4096) {
        __bss_start = .;
        *(.bss*) *(COMMON)
        __bss_end = .;
    }
    . = ALIGN(4096);
    . += 0x10000;
    stack_top = .;
    _end = .;
    /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
}
