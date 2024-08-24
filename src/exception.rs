use core::arch::global_asm;

use crate::{
    syscall::handle_syscall,
    thread::{ThreadActivationResult, ThreadHandle},
};

pub const INSTUCTION_ADDRESS_MISALIGNED: u64 = 0;
pub const INSTRUCTION_ACCESS_FAULT: u64 = 1;
pub const ILLEGAL_INSTRUCTION: u64 = 2;
pub const BREAKPOINT: u64 = 3;
pub const LOAD_ADDRESS_MISALIGNED: u64 = 4;
pub const LOAD_ACCESS_FAULT: u64 = 5;
pub const STORE_AMO_ADDRESS_MISALIGNED: u64 = 6;
pub const STORE_AMO_ACCESS_FAULT: u64 = 7;
pub const USER_ENVIRONMENT_CALL: u64 = 8;
pub const SUPERVISOR_ENVIRONMENT_CALL: u64 = 9;
pub const INSTRUCTION_PAGE_FAULT: u64 = 12;
pub const LOAD_PAGE_FAULT: u64 = 13;
pub const STORE_AMO_PAGE_FAULT: u64 = 15;

extern "C" {
    pub fn init_exception_handler();
}

global_asm!(include_str!("exception.S"));

pub fn handle_exception(activation: &ThreadActivationResult, handle: &ThreadHandle) {
    match activation.cause {
        INSTUCTION_ADDRESS_MISALIGNED => unimplemented!("Instruction Address Misaligned"),
        INSTRUCTION_ACCESS_FAULT => unimplemented!("Instruction Access Fault"),
        ILLEGAL_INSTRUCTION => handle.kill(),
        BREAKPOINT => unimplemented!("Breakpoint"),
        LOAD_ADDRESS_MISALIGNED => handle.kill(),
        LOAD_ACCESS_FAULT => unimplemented!("Load Access Fault"),
        STORE_AMO_ADDRESS_MISALIGNED => handle.kill(),
        STORE_AMO_ACCESS_FAULT => unimplemented!("Store AMO Access Fault"),
        USER_ENVIRONMENT_CALL => handle_syscall(activation, handle, false),
        SUPERVISOR_ENVIRONMENT_CALL => handle_syscall(activation, handle, true),
        INSTRUCTION_PAGE_FAULT => unimplemented!("Instruction Page Fault"),
        LOAD_PAGE_FAULT => unimplemented!("Load Page Fault"),
        STORE_AMO_PAGE_FAULT => unimplemented!("Store AMO Page Fault"),
        reason => panic!("Unknown exception encountered: {:#010x}", reason),
    }
}
