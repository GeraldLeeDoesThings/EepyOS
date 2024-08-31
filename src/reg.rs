use core::arch::global_asm;

extern "C" {
    pub fn get_stval() -> u64;
}

global_asm!(include_str!("reg.S"));
