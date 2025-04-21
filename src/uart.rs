use crate::io::{Readable, Writable};
use core::fmt::Write;

// TODO: Don't hard code this
/// Base address for UART 0
pub const UART0_BASE: u64 = 0x1000_0000;

/// Offset in bytes for the receiver buffer register.
const RBR_OFFSET: isize = 0x00;
/// Offset in bytes for the transmitter holding register.
const THR_OFFSET: isize = 0x00;
// const FCR_OFFSET: isize = 0x08;
/// Offset in bytes for the line control register.
const LCR_OFFSET: isize = 0x0C;
/// Offset in bytes for the line status register.
const LSR_OFFSET: isize = 0x14;

/// Line status bitmask for data ready bit.
const LSR_DR_BITMASK: u8 = 0x1;
/// Line status bitmask for trasmit ready bit.
const LSR_THRE_BITMASK: u8 = 0x1 << 5;

// There are more fields that we don't really care about right now

/// A collection of pointers to a UART and (a subset of) its registers.
pub struct UartHandler {
    /// The read buffer register.
    rbr: *const u8,
    /// The transmit holding register.
    thr: *mut u8,
    /// The line control register.
    _lcr: *mut u8,
    /// The line status register.
    lsr: *const u8,
}

impl Readable<u8> for UartHandler {
    fn read(&self) -> Option<u8> {
        // SAFETY: By the correctness of the UART layout.
        let has_data = unsafe { self.lsr.read_volatile() & LSR_DR_BITMASK };
        if has_data == 0 {
            return None;
        }
        // SAFETY: By the correctness of the UART layout.
        Some(unsafe { self.rbr.read_volatile() })
    }
}

impl Writable<u8> for UartHandler {
    fn write(&self, v: u8) -> Result<(), ()> {
        // SAFETY: By the correctness of the UART layout.
        let has_space = unsafe { self.lsr.read_volatile() & LSR_THRE_BITMASK };
        if has_space == 0 {
            return Err(());
        }
        // SAFETY: By the correctness of the UART layout.
        unsafe { self.thr.write_volatile(v) };
        Ok(())
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
    /// Creates a new uart handler from a given base address. Consider
    /// using the safer [`Self::new_from_uart_index`] instead.
    ///
    /// # Safety
    ///
    /// `base` must be the base address of a UART.
    pub const unsafe fn new(base: u64) -> Self {
        let base_ptr = base as *const u8;
        // handler.lcr.write_volatile(0x00000003); // Set word length
        // handler.fcr.write_volatile(0x00000001); // Enable FIFO
        Self {
            // SAFETY: By the safety requirements of this function.
            rbr: unsafe { base_ptr.byte_offset(RBR_OFFSET).cast() },
            // SAFETY: By the safety requirements of this function.
            thr: unsafe { base_ptr.byte_offset(THR_OFFSET).cast_mut() },
            // SAFETY: By the safety requirements of this function.
            _lcr: unsafe { base_ptr.byte_offset(LCR_OFFSET).cast_mut() },
            // SAFETY: By the safety requirements of this function.
            lsr: unsafe { base_ptr.byte_offset(LSR_OFFSET).cast() },
        }
    }

    /// Tries to create a new UART from an `index` into all the known
    /// UARTs. Returns a `None` if no UART corresponds to `index`.
    #[allow(dead_code, unused_variables, reason = "TODO")]
    pub const fn new_from_uart_index(index: u64) -> Option<Self> {
        todo!();
    }
}

#[macro_export]
macro_rules! print {
    ($($args:tt)+) => ({
        use core::fmt::Write;
        use $crate::uart::{UART0_BASE, UartHandler};
        // SAFETY: UART0_BASE is correct.
        let mut uart_out = unsafe { UartHandler::new(UART0_BASE) };
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
