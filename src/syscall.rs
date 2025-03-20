use core::arch::global_asm;

use crate::thread::{ThreadActivationResult, ThreadHandle};

pub const EXIT: usize = 0;
pub const YIELD: usize = 1;

#[no_mangle]
pub extern "C" fn exit(status: usize) -> ! {
    unsafe {
        syscall_1a(EXIT, status);
    }
    unreachable!("Execution survived exiting.")
}

#[no_mangle]
pub extern "C" fn p_yield() {
    unsafe {
        syscall(YIELD);
    }
}

pub fn handle_syscall(
    activation: &ThreadActivationResult,
    handle: &ThreadHandle,
    _supervisor: bool,
) {
    let args = activation.thread.get_args();
    let code = args.first().unwrap();
    match *code {
        EXIT => handle.kill(),
        YIELD => handle.resolve_interrupt_or_kill(true),
        _ => unimplemented!("Unknown Syscall: {:#010x}", *code), // Handle unknown syscalls later
    }
}

#[allow(unused)]
extern "C" {
    pub fn syscall(code: usize) -> isize;
    pub fn syscall_1a(code: usize, arg1: usize) -> isize;
    pub fn syscall_2a(code: usize, arg1: usize, arg2: usize) -> isize;
    pub fn syscall_3a(code: usize, arg1: usize, arg2: usize, arg3: usize) -> isize;
    pub fn syscall_4a(code: usize, arg1: usize, arg2: usize, arg3: usize, arg4: usize) -> isize;
    pub fn syscall_5a(
        code: usize,
        arg1: usize,
        arg2: usize,
        arg3: usize,
        arg4: usize,
        arg5: usize,
    ) -> isize;
    pub fn syscall_6a(
        code: usize,
        arg1: usize,
        arg2: usize,
        arg3: usize,
        arg4: usize,
        arg5: usize,
        arg6: usize,
    ) -> isize;
    pub fn syscall_7a(
        code: usize,
        arg1: usize,
        arg2: usize,
        arg3: usize,
        arg4: usize,
        arg5: usize,
        arg6: usize,
        arg7: usize,
    ) -> isize;
}

global_asm!(include_str!("syscall.S"));
