
activate_page_table:
    srli a0, a0, 12
    slli a1, a1, 44
    li t0, 8
    slli t0, t0, 60
    or t0, t0, a0
    or t0, t0, a1
    csrw satp, t0
    mv a0, t0
    ret

emit_mmu_fence_asm:
    sfence.vma zero, zero
    ret

.section ".text"
