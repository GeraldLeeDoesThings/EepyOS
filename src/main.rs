//! This crate implements a kernel for the Star64 single board computer.

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
#![warn(clippy::missing_docs_in_private_items)]
#![deny(
    clippy::allow_attributes_without_reason,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks,
    unsafe_op_in_unsafe_fn
)]

/// Debug console for testing heap allocations.
mod console;
/// Constants unified in one module.
mod consts;
/// Structs used to store trap frames (registers) when
/// threads are interrupted.
mod context;
/// Generic data structures.
mod data;
/// Assembly wrappers for debugging.
mod debug;
/// Handler for exceptions after thread activaton.
mod exception;
/// Allocators to allow heap allocations.
mod heap;
/// Handler for interrupts after thread activaton.
mod interrupt;
/// Trait defintions for readable and writable objects.
mod io;
/// Handles page tables and MMU.
mod mmu;
/// Helper functions for working with pointers.
mod pointer;
/// Process control block definitions.
mod process;
/// Assembly wrappers for fetching control register values.
mod reg;
/// Resource trait to manage exhaustable resources.
mod resource;
/// Synchronization primitives.
mod sync;
/// Functions for making generic syscalls, and syscall code constants.
mod syscall;
/// Thread control block definitions.
mod thread;
/// Functions for reading from and setting timers.
mod time;
/// Functions for uart communication.
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

/// The return address back into the bootloader.
static mut BOOTLOADER_RETURN_ADDRESS: i64 = 0;
/// A datastructure holding control blocks for all processes and threads.
static PROCESS_TABLE: Mutex<ResourceManager<Option<ProcessControlBlock>, MAX_PROCESSES>> =
    Mutex::new(ResourceManager::new([const { None }; MAX_PROCESSES]));

/// The main loop of the kernel.
#[no_mangle]
#[allow(dead_code, reason = "Heavy debug usage")]
extern "C" fn kmain(hart_id: usize, _dtb: *const u8) -> ! {
    // SAFETY: Just saves a register.
    #[allow(
        clippy::multiple_unsafe_ops_per_block,
        reason = "Too painful to rewrite"
    )]
    unsafe {
        asm!(
            "mv {0}, ra",
            out(reg) BOOTLOADER_RETURN_ADDRESS,
        );
    }
    // SAFETY: UART0_BASE is correct.
    let console = unsafe { UartHandler::new(UART0_BASE) };
    println!("Welcome to EepyOS!");
    println!("Hello from core: {}", hart_id);

    // SAFETY: Single threaded access, only called once.
    unsafe {
        init_exception_handler();
    }
    // SAFETY: Single threaded access, only called once.
    unsafe {
        init_context();
    }
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

/// Prints something.
extern "C" fn test() -> usize {
    // TODO: Move elsewhere
    println!("Hello world!");
    0
}

/// Just prints something else.
extern "C" fn test2() -> usize {
    // TODO: Move elsewhere
    println!("Hello from another process!");
    0
}

/// Tests if threads are interrupted by the timer.
#[allow(
    clippy::empty_loop,
    clippy::infinite_loop,
    unused,
    reason = "Debug function"
)]
extern "C" fn test3() -> usize {
    // TODO: Move elsewhere
    println!("Looping forever... (in userspace)");
    loop {}
}

/// The panic handler for the kernel.
#[allow(clippy::no_mangle_with_rust_abi, reason = "Panic handler")]
#[no_mangle]
#[panic_handler]
unsafe fn panic(info: &PanicInfo) -> ! {
    if let Some(msg) = info.message().as_str() {
        println!("Kernel panic: {}", msg);
    } else {
        println!("Generic Kernel panic!");
    }
    // TODO: Restore the stack pointer too.
    #[allow(
        clippy::multiple_unsafe_ops_per_block,
        reason = "Too painful to rewrite"
    )]
    // SAFETY: Returns to bootloader, single threaded.
    unsafe {
        asm!(
            "mv ra, {0}",
            "ret",
            in(reg) BOOTLOADER_RETURN_ADDRESS,
        );
    }
    unreachable!();
}
