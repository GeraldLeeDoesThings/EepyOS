
.section ".text"

test_context_asm:
    li t0, 0x123
    li a0, 0x456
    li a1, 0x789
    li a2, 0x987
    li a3, 0x654
    li t6, 0x321
    addi sp, sp, -16
    sd ra, 0(sp)
    call dump_regs
    call p_yield
    call dump_regs
    ld ra, 0(sp)
    addi sp, sp, 16
    li a0, 0
    ret

dump_regs: // t0, a0, a1, a2, a3, t6
    addi sp, sp, -64
    sd ra, 0(sp)
    sd t0, 8(sp)
    sd a0, 16(sp)
    sd a1, 24(sp)
    sd a2, 32(sp)
    sd a3, 40(sp)
    sd t6, 48(sp)
    mv a0, t0
    call print_reg_hex
    ld a0, 16(sp)
    call print_reg_hex
    ld a0, 24(sp)
    call print_reg_hex
    ld a0, 32(sp)
    call print_reg_hex
    ld a0, 40(sp)
    call print_reg_hex
    ld a0, 48(sp)
    call print_reg_hex
    mv a0, sp
    call print_reg_hex
    ld ra, 0(sp)
    ld t0, 8(sp)
    ld a0, 16(sp)
    ld a1, 24(sp)
    ld a2, 32(sp)
    ld a3, 40(sp)
    ld t6, 48(sp)
    addi sp, sp, 64
    ret

.global test_context_asm
.global dump_regs
