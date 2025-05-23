use core::arch::global_asm;

/// Registers for a thread. Used as the memory layout for trap frames.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
#[allow(
    clippy::missing_docs_in_private_items,
    reason = "Register names are self descriptive"
)]
pub struct RegisterContext {
    pub ra: usize,
    pub sp: usize,
    pub gp: usize,
    pub tp: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
    pub ft0: usize,
    pub ft1: usize,
    pub ft2: usize,
    pub ft3: usize,
    pub ft4: usize,
    pub ft5: usize,
    pub ft6: usize,
    pub ft7: usize,
    pub fs0: usize,
    pub fs1: usize,
    pub fa0: usize,
    pub fa1: usize,
    pub fa2: usize,
    pub fa3: usize,
    pub fa4: usize,
    pub fa5: usize,
    pub fa6: usize,
    pub fa7: usize,
    pub fs2: usize,
    pub fs3: usize,
    pub fs4: usize,
    pub fs5: usize,
    pub fs6: usize,
    pub fs7: usize,
    pub fs8: usize,
    pub fs9: usize,
    pub fs10: usize,
    pub fs11: usize,
    pub ft8: usize,
    pub ft9: usize,
    pub ft10: usize,
    pub ft11: usize,
}

/// The result of activating (running) a thread.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ActivationResult {
    /// The program counter when trapping (sepc).
    pub pc: usize,
    /// The trap cause (scause).
    pub cause: usize,
}

impl RegisterContext {
    /// Constructs a register context with all registers set to zero.
    pub const fn all_zero() -> Self {
        Self {
            ra: 0,
            sp: 0,
            gp: 0,
            tp: 0,
            t0: 0,
            t1: 0,
            t2: 0,
            s0: 0,
            s1: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            a3: 0,
            a4: 0,
            a5: 0,
            a6: 0,
            a7: 0,
            s2: 0,
            s3: 0,
            s4: 0,
            s5: 0,
            s6: 0,
            s7: 0,
            s8: 0,
            s9: 0,
            s10: 0,
            s11: 0,
            t3: 0,
            t4: 0,
            t5: 0,
            t6: 0,
            ft0: 0,
            ft1: 0,
            ft2: 0,
            ft3: 0,
            ft4: 0,
            ft5: 0,
            ft6: 0,
            ft7: 0,
            fs0: 0,
            fs1: 0,
            fa0: 0,
            fa1: 0,
            fa2: 0,
            fa3: 0,
            fa4: 0,
            fa5: 0,
            fa6: 0,
            fa7: 0,
            fs2: 0,
            fs3: 0,
            fs4: 0,
            fs5: 0,
            fs6: 0,
            fs7: 0,
            fs8: 0,
            fs9: 0,
            fs10: 0,
            fs11: 0,
            ft8: 0,
            ft9: 0,
            ft10: 0,
            ft11: 0,
        }
    }
}

extern "C" {
    pub fn activate_context(pc: usize, context_base: usize, hart_id: usize) -> ActivationResult;
    pub fn init_context();
}

global_asm!(include_str!("context.S"));
