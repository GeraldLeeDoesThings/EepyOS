use core::arch::global_asm;

use crate::{println, syscall::p_yield};

extern "C" {
    pub fn test_context_asm() -> u64;
}

#[no_mangle]
extern "C" fn print_reg_hex(val: u64) {
    println!("{:#010x}", val);
}

pub extern "C" fn test_context() -> u64 {
    unsafe { test_context_asm() }
}

global_asm!(include_str!("debug.S"));
