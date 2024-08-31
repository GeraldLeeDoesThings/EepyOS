
.section ".text"

get_time:
    rdtime a0
    ret

set_timecmp:
    li a6, 0
    li a7, 0x54494D45
    ecall
    ret

.global get_time
.global set_timecmp
