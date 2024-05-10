use core::fmt::Write;
use crate::io::{
    Readable,
    Writable
};

// TODO: Don't hard code this
pub const UART0_BASE: u64 = 0x1000_0000;

const RBR_OFFSET: isize = 0x00;
const THR_OFFSET: isize = 0x00;
// const FCR_OFFSET: isize = 0x08;
const LCR_OFFSET: isize = 0x0C;
const LSR_OFFSET: isize = 0x14;

const LSR_DR_BITMASK: u8   = 0x1;
const LSR_THRE_BITMASK: u8 = 0x1 << 5;

// There are more fields that we don't really care about right now


pub struct UartHandler {
    rbr: *const u8,
    thr: *mut u8,
    lcr: *mut u8,
    lsr: *const u8,
}

impl Readable<u8> for UartHandler {
    fn read(&self) -> Option<u8> {
        unsafe {
            let has_data = self.lsr.read_volatile() & LSR_DR_BITMASK;
            if has_data == 0 { return None; }
            Some(self.rbr.read_volatile())
        }
    }
}

impl Writable<u8> for UartHandler {
    fn write(&self, v: u8) -> Result<(), ()> {
        unsafe {
            let has_space = self.lsr.read_volatile() & LSR_THRE_BITMASK;
            if has_space == 0 { return Err(()); }
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
    pub fn new(base: u64) -> UartHandler {
        let base_ptr = base as *const u8;
        unsafe {
            let handler = UartHandler { 
                rbr: base_ptr.byte_offset(RBR_OFFSET) as *const u8,
                thr: base_ptr.byte_offset(THR_OFFSET) as *mut u8,
                lcr: base_ptr.byte_offset(LCR_OFFSET) as *mut u8,
                lsr: base_ptr.byte_offset(LSR_OFFSET) as *const u8,
            };
            // handler.lcr.write_volatile(0x00000003); // Set word length
            // handler.fcr.write_volatile(0x00000001); // Enable FIFO
            handler
        }
    }
}


#[macro_export]
macro_rules! print {
    ($($args:tt)+) => ({
        use core::fmt::Write;
        use crate::uart::{UART0_BASE, UartHandler};
        let mut uart_out = UartHandler::new(UART0_BASE);
        let _ = write!(&mut uart_out, $($args)+);
    });
}


#[macro_export]
macro_rules! println {
    () => ({
        print!("\r\n")
    });
    ($fmt:expr) => ({
        print!(concat!($fmt, "\r\n"))
    });
    ($fmt:expr, $($args:tt)+) => ({
        print!(concat!($fmt, "\r\n"), $($args)+)
    });
}

