use crate::thread::{ThreadActivationResult, ThreadHandle};

pub const IS_INTERRUPT_MASK: usize = 0x8000_0000_0000_0000;
pub const SOFTWARE_INTERRUPT: usize = 1;
pub const TIMER_INTERRUPT: usize = 5;
pub const EXTERNAL_INTERRUPT: usize = 9;

#[allow(clippy::match_same_arms)]
pub fn handle_interrupt(activation: &ThreadActivationResult, handle: &ThreadHandle) {
    let reason: usize = activation.cause ^ IS_INTERRUPT_MASK;
    match reason {
        SOFTWARE_INTERRUPT => handle.kill(), // No idea how to handle this for now
        TIMER_INTERRUPT => handle.resolve_interrupt_or_kill(false), // Do nothing, just need to reschedule
        EXTERNAL_INTERRUPT => handle.kill(), // No idea how to handle this for now
        _ => panic!("Unknown interrupt encountered: {}", reason),
    }
}
