
.section ".text.boot"
.global _start
_start:
        // Make sure we boot with core 0
        csrr t0, mhartid
        bne t0, zero, wait_for_boot

        // Zero out bss
        la a0, _bss_start
        la a1, _bss_end
        jal clear_bss

        // Disable interrupts
        csrw mie, zero

        j kmain
        j wait_for_boot

clear_bss:
        sd zero, (a0)
        addi a0, a0, 8
        bne a0, a1, clear_bss
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

