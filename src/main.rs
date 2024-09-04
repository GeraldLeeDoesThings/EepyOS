#![no_main]
#![no_std]
#![feature(error_generic_member_access)]
#![feature(impl_trait_in_assoc_type)]

mod consts;
mod context;
mod debug;
mod exception;
mod heap;
mod interrupt;
mod io;
mod process;
mod reg;
mod resource;
mod sync;
mod syscall;
mod thread;
mod time;
mod uart;

use consts::MAX_PROCESSES;
use context::init_context;
use debug::test_context;
use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::unreachable;
use exception::{handle_exception, init_exception_handler};
use interrupt::{handle_interrupt, IS_INTERRUPT_MASK};
use io::Writable;
use process::ProcessControlBlock;
use resource::ResourceManager;
use uart::{UartHandler, UART0_BASE};

use crate::io::Readable;

global_asm!(include_str!("consts.S"));
global_asm!(include_str!("boot.S"));

static mut BOOTLOADER_RETURN_ADDRESS: i64 = 0;
static mut PROCESS_TABLE: ResourceManager<Option<ProcessControlBlock>, MAX_PROCESSES> =
    ResourceManager::new([const { None }; MAX_PROCESSES]);

#[no_mangle]
#[allow(dead_code)]
extern "C" fn kmain(hart_id: u64, _dtb: *const u8) -> ! {
    unsafe {
        asm!(
            "mv {0}, ra",
            out(reg) BOOTLOADER_RETURN_ADDRESS,
        );
    }
    let console = UartHandler::new(UART0_BASE);
    println!("Welcome to EepyOS!");
    println!("Hello from core: {}", hart_id);

    unsafe {
        init_exception_handler();
        init_context();
        let maybe_test_process = ProcessControlBlock::new(test, 0, 10, 0x5000_0000);

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

        let _ = PROCESS_TABLE
            .claim_first(Some(
                ProcessControlBlock::new(test2, 1, 9, 0x5100_0000).unwrap(),
            ))
            .expect("Failed to spawn second process");

        let _ = PROCESS_TABLE
            .claim_first(Some(
                ProcessControlBlock::new(test3, 2, 11, 0x5200_0000).unwrap(),
            ))
            .expect("Failed to spawn third process");

        let _ = PROCESS_TABLE
            .claim_first(Some(
                ProcessControlBlock::new(test_context, 3, 11, 0x5300_0000).unwrap(),
            ))
            .expect("Failed to spawn fourth process");
    }

    loop {
        unsafe {
            // TODO: Track number of "living" threads per process
            let scheduled_thread = match PROCESS_TABLE.choose_next_thread() {
                None => {
                    println!("Out of threads to schedule, starting echo loop...");
                    break;
                }
                Some(chosen_thread) => chosen_thread,
            };

            let run_result = match scheduled_thread.activate(hart_id) {
                Ok(result) => result,
                Err(msg) => {
                    println!("Error trying to run thread: {}", msg);
                    break;
                }
            };

            if run_result.cause & IS_INTERRUPT_MASK > 0 {
                handle_interrupt(&run_result, &scheduled_thread);
            } else {
                handle_exception(&run_result, &scheduled_thread);
            }
        }
    }

    loop {
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

extern "C" fn test() -> u64 {
    // TODO: Move elsewhere
    println!("Hello world!");
    return 0;
}

extern "C" fn test2() -> u64 {
    // TODO: Move elsewhere
    println!("Hello from another process!");
    return 0;
}

extern "C" fn test3() -> u64 {
    // TODO: Move elsewhere
    println!("Looping forever... (in userspace)");
    loop {}
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
