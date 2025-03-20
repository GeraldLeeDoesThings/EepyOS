use core::arch::global_asm;

use crate::{
    println,
    reg::get_stval,
    syscall::handle_syscall,
    thread::{ThreadActivationResult, ThreadHandle},
};

pub const INSTUCTION_ADDRESS_MISALIGNED: usize = 0;
pub const INSTRUCTION_ACCESS_FAULT: usize = 1;
pub const ILLEGAL_INSTRUCTION: usize = 2;
pub const BREAKPOINT: usize = 3;
pub const LOAD_ADDRESS_MISALIGNED: usize = 4;
pub const LOAD_ACCESS_FAULT: usize = 5;
pub const STORE_AMO_ADDRESS_MISALIGNED: usize = 6;
pub const STORE_AMO_ACCESS_FAULT: usize = 7;
pub const USER_ENVIRONMENT_CALL: usize = 8;
pub const SUPERVISOR_ENVIRONMENT_CALL: usize = 9;
pub const INSTRUCTION_PAGE_FAULT: usize = 12;
pub const LOAD_PAGE_FAULT: usize = 13;
pub const STORE_AMO_PAGE_FAULT: usize = 15;

extern "C" {
    pub fn init_exception_handler();
}

global_asm!(include_str!("exception.S"));

#[allow(clippy::match_same_arms)]
pub fn handle_exception(activation: &ThreadActivationResult, handle: &ThreadHandle) {
    match activation.cause {
        INSTUCTION_ADDRESS_MISALIGNED => unimplemented!("Instruction Address Misaligned"),
        INSTRUCTION_ACCESS_FAULT => unimplemented!("Instruction Access Fault"),
        ILLEGAL_INSTRUCTION => handle.kill(),
        BREAKPOINT => unimplemented!("Breakpoint"),
        LOAD_ADDRESS_MISALIGNED => handle.kill(),
        LOAD_ACCESS_FAULT => {
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
