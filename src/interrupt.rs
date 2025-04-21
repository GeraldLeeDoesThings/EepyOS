use crate::thread::{ThreadActivationResult, ThreadHandle};

/// Bitmask fetching a bit indicating if an interrupt occured.
pub const IS_INTERRUPT_MASK: usize = 0x8000_0000_0000_0000;
/// A software interrupt (syscall).
pub const SOFTWARE_INTERRUPT: usize = 1;
/// A timer interrupt.
pub const TIMER_INTERRUPT: usize = 5;
/// An external interrupt.
pub const EXTERNAL_INTERRUPT: usize = 9;

/// Handles an interrupt taken during a thread activation.
#[allow(clippy::match_same_arms, reason = "Will differentiate later")]
pub fn handle_interrupt(activation: &ThreadActivationResult, handle: &ThreadHandle) {
    let reason: usize = activation.cause ^ IS_INTERRUPT_MASK;
    match reason {
        // No idea how to handle this for now
        SOFTWARE_INTERRUPT => handle.kill(),
        // Do nothing, just need to reschedule
        TIMER_INTERRUPT => handle.resolve_interrupt_or_kill(false),
        // No idea how to handle this for now
        EXTERNAL_INTERRUPT => handle.kill(),
        _ => panic!("Unknown interrupt encountered: {}", reason),
    }
}
