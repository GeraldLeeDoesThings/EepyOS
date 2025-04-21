use core::arch::global_asm;

use crate::{
    println,
    reg::get_stval,
    syscall::handle_syscall,
    thread::{ThreadActivationResult, ThreadHandle},
};

/// `pc` has been set to a misaligned value.
pub const INSTUCTION_ADDRESS_MISALIGNED: usize = 0;
/// Tried to execute instructions in protected memory.
pub const INSTRUCTION_ACCESS_FAULT: usize = 1;
/// Tried to execute an unknown instruction.
pub const ILLEGAL_INSTRUCTION: usize = 2;
/// Executed a breakpoint.
pub const BREAKPOINT: usize = 3;
/// Tried to load a misaligned address.
pub const LOAD_ADDRESS_MISALIGNED: usize = 4;
/// Tried to load protected memory.
pub const LOAD_ACCESS_FAULT: usize = 5;
/// Tried to access a misaligned address as part of an atomic instruction.
pub const STORE_AMO_ADDRESS_MISALIGNED: usize = 6;
/// Tried to access protected memory as part of an atomic instruction.
pub const STORE_AMO_ACCESS_FAULT: usize = 7;
/// Userspace program made a syscall.
pub const USER_ENVIRONMENT_CALL: usize = 8;
/// Kernel made a syscall.
pub const SUPERVISOR_ENVIRONMENT_CALL: usize = 9;
/// Tried to execute instructions in a non-executable (or unmapped) page.
pub const INSTRUCTION_PAGE_FAULT: usize = 12;
/// Tried to load from a non-readable (or unmapped) page.
pub const LOAD_PAGE_FAULT: usize = 13;
/// Tried to access memory from a page that is either unmapped or does not have
/// sufficient permissions, as part of an atomic operation.
pub const STORE_AMO_PAGE_FAULT: usize = 15;

extern "C" {
    pub fn init_exception_handler();
}

global_asm!(include_str!("exception.S"));

/// Handles an exception in `activation` occuring in the thread pointed to by
/// `handle`.
#[allow(
    clippy::match_same_arms,
    reason = "Will handle arms differently in the future"
)]
pub fn handle_exception(activation: &ThreadActivationResult, handle: &ThreadHandle) {
    match activation.cause {
        INSTUCTION_ADDRESS_MISALIGNED => unimplemented!("Instruction Address Misaligned"),
        INSTRUCTION_ACCESS_FAULT => unimplemented!("Instruction Access Fault"),
        ILLEGAL_INSTRUCTION => handle.kill(),
        BREAKPOINT => unimplemented!("Breakpoint"),
        LOAD_ADDRESS_MISALIGNED => handle.kill(),
        LOAD_ACCESS_FAULT => {
            // SAFETY: Function just fetches a register
            println!("Error at: {:#010x}", unsafe { get_stval() });
            unimplemented!("Load Access Fault");
        }
        STORE_AMO_ADDRESS_MISALIGNED => handle.kill(),
        STORE_AMO_ACCESS_FAULT => unimplemented!("Store AMO Access Fault"),
        USER_ENVIRONMENT_CALL => handle_syscall(activation, handle, false),
        SUPERVISOR_ENVIRONMENT_CALL => handle_syscall(activation, handle, true),
        INSTRUCTION_PAGE_FAULT => {
            println!("Instruction Page Fault");
            handle.kill();
        }
        LOAD_PAGE_FAULT => unimplemented!("Load Page Fault"),
        STORE_AMO_PAGE_FAULT => unimplemented!("Store AMO Page Fault"),
        reason => panic!("Unknown exception encountered: {:#010x}", reason),
    }
}
