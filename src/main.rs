#![no_main]
#![no_std]

mod io;
mod uart;

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::unreachable;
use io::Writable;

use crate::io::Readable;

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
    println!("Welcome to EepyOS!");
    loop {
        if let Some(inp) = console.read() {
            match console.write(inp) {
                Ok(()) => (),
                Err(()) => {
                    let mut rval = console.read();
                    while rval.is_some() {
                        rval = console.read();
                    }
                },
            }
        }
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

