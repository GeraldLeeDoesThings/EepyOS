use crate::{println, thread::{ThreadActivationResult, ThreadHandle}};

pub const IS_INTERRUPT_MASK: u64 = 0x80000000_00000000;
pub const SOFTWARE_INTERRUPT: u64 = 1;
pub const TIMER_INTERRUPT: u64 = 5;
pub const EXTERNAL_INTERRUPT: u64 = 9;

pub fn handle_interrupt(activation: &ThreadActivationResult, handle: &ThreadHandle) {
    let reason: u64 = activation.cause ^ IS_INTERRUPT_MASK;
    match reason {
        SOFTWARE_INTERRUPT => handle.kill(), // No idea how to handle this for now
        TIMER_INTERRUPT => {
            match handle.resolve_interrupt() {
                Ok(_) => {},
                Err(_) => {
                    handle.kill();
                    println!("Mismatched thread state! Killing thread.")
                },
            }
        }                // Do nothing, just need to reschedule
        EXTERNAL_INTERRUPT => handle.kill(), // No idea how to handle this for now
        _ => panic!("Unknown interrupt encountered: {}", reason),
    }
}
