use core::arch::global_asm;

/// Timer frequency in ticks / second
pub const TIMER_FREQ: u64 = 400_0000;
/// Timer frequency in ticks / microsecond
const US_TO_TICKS: u64 = TIMER_FREQ / 100_0000;

/// Configures the timer to fire an interrupt after `delay_us` microseconds.
pub fn set_timecmp_delay(delay_us: u64) {
    // SAFETY: asm wrapper.
    let time = unsafe { get_time() };
    // SAFETY: asm wrapper.
    unsafe { set_timecmp(time + delay_us * US_TO_TICKS) }
}

/// Configures the timer to fire an interrupt after `delay_ms` miliseconds.
pub fn set_timecmp_delay_ms(delay_ms: u64) {
    set_timecmp_delay(delay_ms * 1000);
}

extern "C" {
    pub fn get_time() -> u64;
    pub fn set_timecmp(time: u64);
}

global_asm!(include_str!("time.S"));
