#![no_main]
#![no_std]
#![feature(error_generic_member_access)]
#![feature(impl_trait_in_assoc_type)]

mod consts;
mod context;
mod exception;
mod io;
mod process;
mod resource;
mod sync;
mod thread;
mod uart;

use consts::MAX_PROCESSES;
use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::unreachable;
use exception::init_exception_handler;
use io::Writable;
use process::ProcessControlBlock;
use resource::ResourceManager;
use uart::{UartHandler, UART0_BASE};

use crate::io::Readable;

global_asm!(include_str!("boot.S"));

static mut BOOTLOADER_RETURN_ADDRESS: i64 = 0;
static mut PROCESS_TABLE: ResourceManager<Option<ProcessControlBlock>, MAX_PROCESSES> =
    ResourceManager::new([const { None }; MAX_PROCESSES]);

#[no_mangle]
#[allow(dead_code)]
extern "C" fn kmain() -> ! {
    unsafe {
        asm!(
            "mv {0}, ra",
            out(reg) BOOTLOADER_RETURN_ADDRESS,
        );
    }
    let console = UartHandler::new(UART0_BASE);
    println!("Welcome to EepyOS!");

    unsafe {
        init_exception_handler();
        let maybe_test_process = ProcessControlBlock::new(test, 0, 10, 0x6000_0000);

        match maybe_test_process {
            Ok(pcb) => {
                if PROCESS_TABLE.claim_first(Some(pcb)).is_ok() {
                    println!("Process spawned successfully!")
                } else {
                    println!("Process spawned with unexpected ID")
                }
            }
            Err(_) => println!("Failed to spawn a process!"),
        }

        // PROCESS_TABLE[0] = Some();
        // let r = PROCESS_TABLE[0].unwrap().threads[0].unwrap().activate();
    }

    loop {
        unsafe {
            let scheduled_thread = match PROCESS_TABLE.choose_next_thread() {
                None => continue,
                Some(chosen_thread) => chosen_thread,
            };

            let run_result = match scheduled_thread.activate() {
                Ok(result) => result,
                Err(msg) => {
                    println!("Error trying to run thread: {}", msg);
                    continue;
                }
            };

            // TODO: Handle the result
        }

        if let Some(inp) = console.read() {
            match console.write(inp) {
                Ok(()) => (),
                Err(()) => {
                    let mut rval = console.read();
                    while rval.is_some() {
                        rval = console.read();
                    }
                }
            }
        }
    }
}

extern "C" fn test() -> i64 {
    // TODO: Move elsewhere
    println!("Hello world!");
    return 0;
}

#[no_mangle]
#[panic_handler]
unsafe fn panic(info: &PanicInfo) -> ! {
    if let Some(msg) = info.message().as_str() {
        println!("Kernel panic: {}", msg);
    }
    asm!(
        "mv ra, {0}",
        "ret",
        in(reg) BOOTLOADER_RETURN_ADDRESS,
    );
    unreachable!();
}
