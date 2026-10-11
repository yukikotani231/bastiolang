/*
 * The AArch64 boot test's platform (tests/aarch64.sh): the least a kernel
 * built with `--target kernel --arch aarch64` needs to run on QEMU's virt
 * machine. It maps the first 4 GiB for the kernel, turns the MMU on, calls
 * bst_kmain with the memory map, and provides the trap entry behind cpu_run.
 * It is test scaffolding, not a platform: one processor, no interrupt
 * controller, no device tree (the memory map is the one QEMU is started
 * with). BastiOS's own platform replaces it.
 */
        .section .text.boot
        .global _start
_start:
        mrs     x1, mpidr_el1
        and     x1, x1, #0xff
        cbnz    x1, park
        ldr     x1, =stack_top
        mov     sp, x1
        ldr     x1, =__bss_start
        ldr     x2, =__bss_end
1:      cmp     x1, x2
        b.hs    2f
        str     xzr, [x1], #8
        b       1b
2:
        /* FP/SIMD usable at EL0 (the kernel itself is built without it). */
        mov     x1, #(3 << 20)
        msr     cpacr_el1, x1
        ldr     x1, =vectors
        msr     vbar_el1, x1

        /* The kernel's map, in top-level slot 0 (every user table copies
         * it): 1 GiB blocks, EL1 only, never executable at EL0. Block 0
         * holds the devices (MAIR index 1, device memory, not executable);
         * blocks 1-3 are normal memory (MAIR index 0), where RAM starts. */
        ldr     x1, =kernel_l1
        orr     x2, x1, #3
        ldr     x3, =bst_kernel_root
        str     x2, [x3]
        ldr     x2, =DEVICE_BLOCK
        str     x2, [x1]
        ldr     x2, =NORMAL_BLOCK
        mov     x4, #1
3:      orr     x5, x2, x4, lsl #30
        str     x5, [x1, x4, lsl #3]
        add     x4, x4, #1
        cmp     x4, #4
        b.ne    3b

        mov     x1, #0xff               /* index 0 normal write-back, index 1 device-nGnRnE */
        msr     mair_el1, x1
        ldr     x1, =TCR
        msr     tcr_el1, x1
        ldr     x1, =bst_kernel_root
        msr     ttbr0_el1, x1
        isb
        tlbi    vmalle1
        dsb     nsh
        isb
        mrs     x1, sctlr_el1
        orr     x1, x1, #1              /* M: the MMU */
        orr     x1, x1, #(1 << 2)       /* C: data cache */
        orr     x1, x1, #(1 << 12)      /* I: instruction cache */
        bic     x1, x1, #(1 << 19)      /* WXN off: a writable page may hold code */
        msr     sctlr_el1, x1
        isb

        ldr     x0, =memmap
        bl      bst_kmain
        /* Power off through PSCI (QEMU answers HVC at EL1). */
        ldr     w0, =0x84000008
        hvc     #0
park:   wfe
        b       park

        .equ    DEVICE_BLOCK, 1 | (1 << 2) | (1 << 10) | (1 << 53) | (1 << 54)
        .equ    NORMAL_BLOCK, 1 | (3 << 8) | (1 << 10) | (1 << 54)
        /* 48-bit addresses from TTBR0 (T0SZ 16), 4 KiB granule, write-back
         * inner shareable walks, no TTBR1 walks (EPD1), 40-bit physical. */
        .equ    TCR, 16 | (1 << 8) | (1 << 10) | (3 << 12) | (1 << 23) | (2 << 32)

/*
 * uint64_t bst_cpu_enter(void): run the saved user context at EL0 until the
 * next exception, save it back, and return its kind: 128 for SVC, 14 for an
 * abort, 32 for an interrupt, 6 for anything else. Of SPSR only the
 * condition flags are kept: EL0t, interrupts unmasked.
 */
        .text
        .global bst_cpu_enter
bst_cpu_enter:
        stp     x29, x30, [sp, #-96]!
        stp     x19, x20, [sp, #16]
        stp     x21, x22, [sp, #32]
        stp     x23, x24, [sp, #48]
        stp     x25, x26, [sp, #64]
        stp     x27, x28, [sp, #80]
        ldr     x30, =user_ctx
        ldr     x1, [x30, #248]
        msr     sp_el0, x1
        ldr     x1, [x30, #256]
        msr     elr_el1, x1
        ldr     x1, [x30, #264]
        and     x1, x1, #0xf0000000
        msr     spsr_el1, x1
        ldr     x1, [x30, #288]
        msr     tpidr_el0, x1
        ldp     x0, x1, [x30, #0]
        ldp     x2, x3, [x30, #16]
        ldp     x4, x5, [x30, #32]
        ldp     x6, x7, [x30, #48]
        ldp     x8, x9, [x30, #64]
        ldp     x10, x11, [x30, #80]
        ldp     x12, x13, [x30, #96]
        ldp     x14, x15, [x30, #112]
        ldp     x16, x17, [x30, #128]
        ldp     x18, x19, [x30, #144]
        ldp     x20, x21, [x30, #160]
        ldp     x22, x23, [x30, #176]
        ldp     x24, x25, [x30, #192]
        ldp     x26, x27, [x30, #208]
        ldp     x28, x29, [x30, #224]
        ldr     x30, [x30, #240]
        eret

/* From EL0: save the context, then return from bst_cpu_enter with the kind
 * in x0. SP_EL1 is still where bst_cpu_enter left it. */
.macro  user_trap next
        stp     x0, x1, [sp, #-16]!
        ldr     x0, =user_ctx
        stp     x2, x3, [x0, #16]
        stp     x4, x5, [x0, #32]
        stp     x6, x7, [x0, #48]
        stp     x8, x9, [x0, #64]
        stp     x10, x11, [x0, #80]
        stp     x12, x13, [x0, #96]
        stp     x14, x15, [x0, #112]
        stp     x16, x17, [x0, #128]
        stp     x18, x19, [x0, #144]
        stp     x20, x21, [x0, #160]
        stp     x22, x23, [x0, #176]
        stp     x24, x25, [x0, #192]
        stp     x26, x27, [x0, #208]
        stp     x28, x29, [x0, #224]
        str     x30, [x0, #240]
        ldp     x2, x3, [sp], #16
        stp     x2, x3, [x0, #0]
        mrs     x1, sp_el0
        str     x1, [x0, #248]
        mrs     x1, elr_el1
        str     x1, [x0, #256]
        mrs     x1, spsr_el1
        str     x1, [x0, #264]
        mrs     x1, esr_el1
        str     x1, [x0, #272]
        mrs     x1, far_el1
        str     x1, [x0, #280]
        mrs     x1, tpidr_el0
        str     x1, [x0, #288]
        b       \next
.endm

/* Classifies a synchronous exception from EL0 by its class (ESR bits 26-31). */
user_sync_kind:
        mrs     x1, esr_el1
        lsr     x1, x1, #26
        mov     x0, #128
        cmp     x1, #0x15               /* SVC from AArch64 */
        b.eq    user_return
        mov     x0, #14
        cmp     x1, #0x20               /* instruction abort from EL0 */
        b.eq    user_return
        cmp     x1, #0x24               /* data abort from EL0 */
        b.eq    user_return
        mov     x0, #6
        b       user_return
user_return:
        ldp     x19, x20, [sp, #16]
        ldp     x21, x22, [sp, #32]
        ldp     x23, x24, [sp, #48]
        ldp     x25, x26, [sp, #64]
        ldp     x27, x28, [sp, #80]
        ldp     x29, x30, [sp], #96
        ret

el0_sync:
        user_trap user_sync_kind
el0_irq:
        user_trap user_irq_kind
user_irq_kind:
        mov     x0, #32
        b       user_return

/* From EL1 itself: a bug in the trusted base or the compiler. */
.macro  kernel_trap vec
        mov     x0, #\vec
        mrs     x1, elr_el1
        mrs     x2, far_el1
        mrs     x3, esr_el1
        bl      bst_kernel_fault
        b       park
.endm

        .balign 2048
vectors:
        .balign 128
        kernel_trap 0
        .balign 128
        kernel_trap 1
        .balign 128
        kernel_trap 2
        .balign 128
        kernel_trap 3
        .balign 128
        kernel_trap 4
        .balign 128
        kernel_trap 5
        .balign 128
        kernel_trap 6
        .balign 128
        kernel_trap 7
        .balign 128
        b       el0_sync
        .balign 128
        b       el0_irq
        .balign 128
        kernel_trap 10
        .balign 128
        kernel_trap 11
        .balign 128
        kernel_trap 12
        .balign 128
        kernel_trap 13
        .balign 128
        kernel_trap 14
        .balign 128
        kernel_trap 15

        .global bst_platform_ctx
bst_platform_ctx:
        ldr     x0, =user_ctx
        ret

        .section .rodata
        .balign 8
/* The memory map bst_kmain gets: one run, the 128 MiB QEMU is given. */
memmap: .quad   1, 0x40000000, 128 << 20

        .bss
        .balign 4096
        .global bst_kernel_root
bst_kernel_root: .skip 4096
kernel_l1:      .skip 4096
user_ctx:       .skip 37 * 8
