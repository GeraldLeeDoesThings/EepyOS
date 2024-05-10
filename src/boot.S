
.section ".text.boot"
.global _start
_start:
        // Save hart id, stored in a0
        mv a2, a0

        // Zero out bss
        la a0, _bss_start
        la a1, _bss_end
        jal clear_bss

        // Disable interrupts
        csrw sie, zero

        mv a0, a2
        la sp, _stack_start
        
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

.section ".bss"
.balign 16
stack:
        .rept 0x10000
        .byte 0
        .endr
stackend:

