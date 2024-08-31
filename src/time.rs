use core::arch::global_asm;

pub const TIMER_FREQ: u64 = 400_0000; // ticks / second
const US_TO_TICKS: u64 = TIMER_FREQ / 100_0000; // ticks / microsecond

pub fn set_timecmp_delay(delay_us: u64) {
    unsafe { set_timecmp(get_time() + delay_us * US_TO_TICKS) }
}

pub fn set_timecmp_delay_ms(delay_ms: u64) {
    set_timecmp_delay(delay_ms * 1000);
}

extern "C" {
    pub fn get_time() -> u64;
    pub fn set_timecmp(time: u64);
}

global_asm!(include_str!("time.S"));
