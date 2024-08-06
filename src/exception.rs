use core::arch::global_asm;

extern "C" {
    pub fn init_exception_handler();
}

global_asm!(include_str!("exception.S"));
