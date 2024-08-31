
.section ".text"

get_stval:
    csrr a0, stval
    ret

.global get_stval

