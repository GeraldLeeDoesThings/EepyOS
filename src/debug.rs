use core::arch::global_asm;

use crate::println;

extern "C" {
    pub fn test_context_asm() -> u64;
}

#[no_mangle]
extern "C" fn print_reg_hex(val: u64) {
    println!("{:#010x}", val);
}

#[allow(unused)]
pub extern "C" fn test_context() -> u64 {
    unsafe { test_context_asm() }
}

global_asm!(include_str!("debug.S"));
