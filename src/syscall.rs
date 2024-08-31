use core::arch::global_asm;

use crate::thread::{ThreadActivationResult, ThreadHandle};

pub const EXIT: u64 = 0;

pub fn exit(status: u64) -> ! {
    unsafe {
        syscall_1a(EXIT, status);
    }
    unreachable!("Execution survived exiting.")
}

pub fn handle_syscall(
    activation: &ThreadActivationResult,
    handle: &ThreadHandle,
    supervisor: bool,
) {
    let args = activation.thread.get_args();
    let code = args.get(0).unwrap();
    match *code {
        EXIT => handle.kill(),
        _ => unimplemented!("Unknown Syscall: {:#010x}", *code), // Handle unknown syscalls later
    }
}

extern "C" {
    pub fn syscall(code: u64) -> i64;
    pub fn syscall_1a(code: u64, arg1: u64) -> i64;
    pub fn syscall_2a(code: u64, arg1: u64, arg2: u64) -> i64;
    pub fn syscall_3a(code: u64, arg1: u64, arg2: u64, arg3: u64) -> i64;
    pub fn syscall_4a(code: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64) -> i64;
    pub fn syscall_5a(code: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> i64;
    pub fn syscall_6a(
        code: u64,
        arg1: u64,
        arg2: u64,
        arg3: u64,
        arg4: u64,
        arg5: u64,
        arg6: u64,
    ) -> i64;
    pub fn syscall_7a(
        code: u64,
        arg1: u64,
        arg2: u64,
        arg3: u64,
        arg4: u64,
        arg5: u64,
        arg6: u64,
        arg7: u64,
    ) -> i64;
}

global_asm!(include_str!("syscall.S"));
