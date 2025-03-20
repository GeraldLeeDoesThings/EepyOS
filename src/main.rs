#![no_main]
#![no_std]
#![feature(allocator_api)]
#![feature(atomic_try_update)]
#![feature(box_as_ptr)]
#![feature(const_box)]
#![feature(error_generic_member_access)]
#![feature(impl_trait_in_assoc_type)]
#![feature(new_range_api)]
#![feature(new_zeroed_alloc)]
#![feature(pointer_is_aligned_to)]
#![feature(slice_ptr_get)]
#![feature(vec_push_within_capacity)]

#![warn(clippy::all, clippy::nursery, clippy::pedantic, clippy::cargo)]

mod console;
mod consts;
mod context;
mod data;
mod debug;
mod exception;
mod heap;
mod interrupt;
mod io;
mod mmu;
mod process;
mod reg;
mod resource;
mod sync;
mod syscall;
mod thread;
mod time;
mod uart;

use console::exec_command;
use consts::MAX_PROCESSES;
use context::init_context;
use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::{str, unreachable};
use exception::{handle_exception, init_exception_handler};
use heap::init_allocators;
use interrupt::{handle_interrupt, IS_INTERRUPT_MASK};
use mmu::Sv39PageTable;
use process::ProcessControlBlock;
use resource::ResourceManager;
use sync::Mutex;
use uart::{UartHandler, UART0_BASE};

use crate::io::Readable;
extern crate alloc;

global_asm!(include_str!("consts.S"));
global_asm!(include_str!("boot.S"));

static mut BOOTLOADER_RETURN_ADDRESS: i64 = 0;
static PROCESS_TABLE: Mutex<ResourceManager<Option<ProcessControlBlock>, MAX_PROCESSES>> =
    Mutex::new(ResourceManager::new([const { None }; MAX_PROCESSES]));

#[no_mangle]
#[allow(dead_code)]
extern "C" fn kmain(hart_id: usize, _dtb: *const u8) -> ! {
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
        init_allocators();
        let maybe_test_process = ProcessControlBlock::new(test, 0, 10, 0x5000_0000);

        match maybe_test_process {
            Ok(pcb) => {
                if PROCESS_TABLE
                    .lock_blocking_mut()
                    .claim_first(Some(pcb))
                    .is_ok()
                {
                    println!("Process spawned successfully!");
                } else {
                    println!("Process spawned with unexpected ID");
                }
            }
            Err(_) => println!("Failed to spawn a process!"),
        }

        let _ = PROCESS_TABLE
            .lock_blocking_mut()
            .claim_first(Some(
                ProcessControlBlock::new(test2, 1, 9, 0x5100_0000).unwrap(),
            ))
            .expect("Failed to spawn second process");

        /*
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
        */
    }

    let mut root_page_table = Sv39PageTable::new();
    root_page_table.as_mut().flat_map();
    println!("Table Address: {:p}", root_page_table);
    root_page_table.as_mut().activate();

    loop {
        // TODO: Track number of "living" threads per process
        // TODO: Drop this ref after thread has been claimed properly
        let mut process_table_ref = PROCESS_TABLE.lock_blocking_mut();
        let scheduled_thread = match process_table_ref.choose_next_thread() {
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

    let mut console_buffer: [u8; 128] = [0; 128];
    let mut write_index: usize = 0;

    loop {
        if let Some(inp) = console.read() {
            match inp {
                b'\n' | b'\r' => {
                    let command_str = str::from_utf8(&console_buffer[0..write_index]).unwrap();
                    println!("");
                    let mut command_args = command_str.split(' ');
                    if let Some(command) = command_args.next() {
                        exec_command(command, &mut command_args);
                    }
                    write_index = 0;
                }
                _ if write_index < 128 => {
                    console_buffer[write_index] = inp;
                    write_index += 1;
                    print!("{}", inp as char);
                }
                _ => println!("Buffer is full!"),
            }
        }
    }
}

extern "C" fn test() -> usize {
    // TODO: Move elsewhere
    println!("Hello world!");
    0
}

extern "C" fn test2() -> usize {
    // TODO: Move elsewhere
    println!("Hello from another process!");
    0
}

#[allow(clippy::empty_loop, clippy::infinite_loop, unused)]
extern "C" fn test3() -> usize {
    // TODO: Move elsewhere
    println!("Looping forever... (in userspace)");
    loop {}
}

#[allow(clippy::no_mangle_with_rust_abi)]
#[no_mangle]
#[panic_handler]
unsafe fn panic(info: &PanicInfo) -> ! {
    if let Some(msg) = info.message().as_str() {
        println!("Kernel panic: {}", msg);
    } else {
        println!("Generic Kernel panic!");
    }
    asm!(
        "mv ra, {0}",
        "ret",
        in(reg) BOOTLOADER_RETURN_ADDRESS,
    );
    unreachable!();
}
