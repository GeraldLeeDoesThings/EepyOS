
#include "consts.S"

.section ".text.boot"
.global _start
_start:
        // Save hart id
        mv a2, a0
        // Save dtb pointer
        mv a3, a1

        // Zero out bss
        la a0, _bss_start
        la a1, _bss_end
        jal clear_bss

        // Disable interrupts
        csrw sie, zero

        mv a0, a2
        mv a1, a3
        addi a2, a2, 1
        la t0, _stack_base
        li t1, KERNEL_STACK_SIZE
        mul sp, a2, t1
        add sp, sp, t0
        
        bnez a0, wait_for_boot
        j kmain
        j wait_for_boot

clear_bss:
        sd zero, (a0)
        addi a0, a0, 8
        bltu a0, a1, clear_bss
        ret

wait_for_boot:
        wfi
        j wait_for_boot
