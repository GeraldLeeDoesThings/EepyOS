
.section ".text"

init_exception_handler:
    la t0, context_return
    csrw stvec, t0
    ret

.global init_exception_handler
