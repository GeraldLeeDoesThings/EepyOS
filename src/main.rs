#![no_main]
#![no_std]

mod io;
mod uart;

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::unreachable;
use io::Writable;

global_asm!(include_str!("boot.S"));

static mut BOOTLOADER_RETURN_ADDRESS: i64 = 0;


#[no_mangle]
#[allow(dead_code)]
extern "C" fn kmain() -> ! {
    unsafe {
        asm!(
            "mv {0}, ra",
            out(reg) BOOTLOADER_RETURN_ADDRESS,
        );
    }
    let console = uart::UartHandler::new(uart::UART0_BASE);
    let mut spam = b'g';
    loop {
        match console.write(spam) {
            Ok(()) => spam = b'g',
            Err(()) => spam = b'b',
        }
        println!("Linebreak {}!", spam);
    }
}


#[no_mangle]
#[panic_handler]
unsafe fn panic(_info: &PanicInfo) -> ! {
    asm!(
        "mv ra, {0}",
        "ret",
        in(reg) BOOTLOADER_RETURN_ADDRESS,
    );
    unreachable!();
}

