use crate::io::{Readable, Writable};
use core::fmt::Write;

// TODO: Don't hard code this
pub const UART0_BASE: u64 = 0x1000_0000;

const RBR_OFFSET: isize = 0x00;
const THR_OFFSET: isize = 0x00;
// const FCR_OFFSET: isize = 0x08;
const LCR_OFFSET: isize = 0x0C;
const LSR_OFFSET: isize = 0x14;

const LSR_DR_BITMASK: u8 = 0x1;
const LSR_THRE_BITMASK: u8 = 0x1 << 5;

// There are more fields that we don't really care about right now

pub struct UartHandler {
    rbr: *const u8,
    thr: *mut u8,
    _lcr: *mut u8,
    lsr: *const u8,
}

impl Readable<u8> for UartHandler {
    fn read(&self) -> Option<u8> {
        unsafe {
            let has_data = self.lsr.read_volatile() & LSR_DR_BITMASK;
            if has_data == 0 {
                return None;
            }
            Some(self.rbr.read_volatile())
        }
    }
}

impl Writable<u8> for UartHandler {
    fn write(&self, v: u8) -> Result<(), ()> {
        unsafe {
            let has_space = self.lsr.read_volatile() & LSR_THRE_BITMASK;
            if has_space == 0 {
                return Err(());
            }
            self.thr.write_volatile(v);
            Ok(())
        }
    }
}

impl Write for UartHandler {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for c in s.bytes() {
            let mut write_successful = self.write(c);
            while write_successful.is_err() {
                write_successful = self.write(c);
            }
        }
        Ok(())
    }
}

impl UartHandler {
    pub const fn new(base: u64) -> Self {
        let base_ptr = base as *const u8;
        unsafe {
            // handler.lcr.write_volatile(0x00000003); // Set word length
            // handler.fcr.write_volatile(0x00000001); // Enable FIFO
            Self {
                rbr: base_ptr.byte_offset(RBR_OFFSET).cast(),
                thr: base_ptr.byte_offset(THR_OFFSET).cast_mut(),
                _lcr: base_ptr.byte_offset(LCR_OFFSET).cast_mut(),
                lsr: base_ptr.byte_offset(LSR_OFFSET).cast(),
            }
        }
    }
}

#[macro_export]
macro_rules! print {
    ($($args:tt)+) => ({
        use core::fmt::Write;
        use $crate::uart::{UART0_BASE, UartHandler};
        let mut uart_out = UartHandler::new(UART0_BASE);
        let _ = write!(&mut uart_out, $($args)+);
    });
}

#[macro_export]
macro_rules! println {
    () => ({
        $crate::print!("\r\n")
    });
    ($fmt:expr) => ({
        $crate::print!(concat!($fmt, "\r\n"))
    });
    ($fmt:expr, $($args:tt)+) => ({
        $crate::print!(concat!($fmt, "\r\n"), $($args)+)
    });
}
