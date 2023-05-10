#![no_main]
#![no_std]

use core::arch::asm;
use core::panic::PanicInfo;
use core::unreachable;

static mut BOOTLOADER_RETURN_ADDRESS: i64 = 0;

#[no_mangle]
fn _start() {
    unsafe {
        asm!(
            "mv {0}, ra",
            out(reg) BOOTLOADER_RETURN_ADDRESS,
        );
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

