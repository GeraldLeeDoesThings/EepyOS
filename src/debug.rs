use core::arch::global_asm;

use crate::println;

extern "C" {
    pub fn test_context_asm() -> u64;
}

/// Prints a value as hex. Useful for calling from assembly.
#[no_mangle]
extern "C" fn print_reg_hex(val: u64) {
    println!("{:#010x}", val);
}

/// Tests that the context switch works.
#[allow(
    unused,
    reason = "Here just in case I get paranoid about the context switch in the future"
)]
pub extern "C" fn test_context() -> u64 {
    // SAFETY: Nescessary evil to test the context switch.
    unsafe { test_context_asm() }
}

global_asm!(include_str!("debug.S"));
