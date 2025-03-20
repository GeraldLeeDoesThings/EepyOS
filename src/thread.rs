use crate::{
    context::{activate_context, ActivationResult, RegisterContext},
    println,
    resource::Resource,
    sync::{Mutex, MutexGuardMut, MutexLockError},
    syscall::exit,
    time::set_timecmp_delay_ms,
};
use core::{error::Error, fmt::Display, ptr::addr_of};

#[derive(Clone, Copy, Debug)]
pub enum ThreadState {
    Interrupted,
    Running,
    Ready,
    Zombie,
}

pub struct ThreadControlBlock {
    registers: RegisterContext,
    pc: usize,
    state: ThreadState,
    id: u16,
    priority: u16,
    need: u32,
    owning_process_id: u16,
    handle_lock: Mutex<()>,
}

pub struct ThreadActivationResult<'a> {
    pub thread: &'a mut ThreadControlBlock,
    pub cause: usize,
}

pub struct ThreadHandle<'a> {
    _guard: MutexGuardMut<'a, ()>,
    thread: *mut ThreadControlBlock,
}

#[derive(Debug)]
pub enum ThreadActivationError {
    FailedToClaim(ThreadHandleClaimError),
    ThreadNotReady(ThreadState),
}

#[derive(Debug)]
pub enum ThreadResolveInterruptError {
    ThreadNotInterrupted(ThreadState),
}

impl Display for ThreadState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Interrupted => write!(f, "Interrupted"),
            Self::Running => write!(f, "Running"),
            Self::Ready => write!(f, "Ready"),
            Self::Zombie => write!(f, "Zombie"),
        }
    }
}

impl Display for ThreadActivationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FailedToClaim(err) => write!(f, "Error while claiming thread:\n{err}"),
            Self::ThreadNotReady(state) => write!(
                f,
                "Thread state must be 'Ready', but the state is '{state}'."
            ),
        }
    }
}

impl Display for ThreadResolveInterruptError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ThreadNotInterrupted(state) => write!(
                f,
                "Thread state must be 'Interrupted', but the state is '{state}'."
            ),
        }
    }
}

#[allow(clippy::match_wildcard_for_single_variants)]
impl Error for ThreadActivationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FailedToClaim(err) => Some(err),
            _ => None,
        }
    }

    fn description(&self) -> &'static str {
        "description() is deprecated; use Display"
    }

    fn cause(&self) -> Option<&dyn Error> {
        self.source()
    }

    fn provide<'a>(&'a self, _request: &mut core::error::Request<'a>) {}
}

#[derive(Debug)]
pub enum ThreadHandleClaimError {
    HandleAlreadyClaimed(MutexLockError),
}

impl Display for ThreadHandleClaimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::HandleAlreadyClaimed(reason) => {
                write!(f, "Thread handle is already claimed: {reason}")
            }
        }
    }
}

impl Error for ThreadHandleClaimError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }

    fn description(&self) -> &'static str {
        "description() is deprecated; use Display"
    }

    fn cause(&self) -> Option<&dyn Error> {
        self.source()
    }

    fn provide<'a>(&'a self, _request: &mut core::error::Request<'a>) {}
}

pub struct CandidateThread<'a> {
    pub best: u32,
    pub handle: Option<ThreadHandle<'a>>,
}

impl<'a> CandidateThread<'a> {
    pub const fn new(best: u32, handle: Option<ThreadHandle<'a>>) -> Self {
        CandidateThread { best, handle }
    }
}

#[allow(clippy::derivable_impls)]
impl Default for CandidateThread<'_> {
    fn default() -> Self {
        Self {
            best: 0,
            handle: None,
        }
    }
}

impl ThreadControlBlock {
    pub fn new(
        code: extern "C" fn() -> usize,
        id: u16,
        priority: u16,
        stack_base: usize,
        owning_process_id: u16,
    ) -> Self {
        let mut tcb = Self {
            registers: RegisterContext::all_zero(),
            pc: code as usize,
            state: ThreadState::Ready,
            id,
            priority,
            need: u32::from(priority),
            owning_process_id,
            handle_lock: Mutex::new(()),
        };
        tcb.registers.sp = stack_base;
        tcb.registers.ra = exit as usize;
        tcb
    }

    pub fn get_handle(&mut self) -> Result<ThreadHandle<'_>, ThreadHandleClaimError> {
        let t: *mut Self = self;
        match self.handle_lock.lock_mut() {
            Ok(handle) => Ok(ThreadHandle {
                _guard: handle,
                thread: t,
            }),
            Err(mutex_err) => Err(ThreadHandleClaimError::HandleAlreadyClaimed(mutex_err)),
        }
    }

    fn activate(
        &mut self,
        hart_id: usize,
    ) -> Result<ThreadActivationResult, ThreadActivationError> {
        match self.state {
            ThreadState::Ready => {
                self.need = u32::from(self.priority);
                self.state = ThreadState::Running;
                unsafe {
                    set_timecmp_delay_ms(1000);
                    let result: ActivationResult =
                        activate_context(self.pc, addr_of!(self.registers) as usize, hart_id);
                    self.pc = result.pc;
                    self.state = ThreadState::Interrupted;
                    Ok(ThreadActivationResult {
                        thread: self,
                        cause: result.cause,
                    })
                }
            }
            _ => Err(ThreadActivationError::ThreadNotReady(self.state)),
        }
    }

    const fn consider(&mut self, best: u32) -> Option<u32> {
        match self.state {
            ThreadState::Ready => {
                self.need += self.priority as u32;
                if self.need > best {
                    Some(self.need)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub const fn get_args(&self) -> [usize; 2] {
        [self.registers.a0, self.registers.a1]
    }

    const fn set_return_val(&mut self, val: usize) {
        self.registers.a0 = val;
    }

    pub const fn get_need(&self) -> u32 {
        self.need
    }

    fn kill(&mut self) {
        println!(
            "Killing thread with id {} from process {}",
            self.id, self.owning_process_id
        );
        match self.state {
            ThreadState::Running => panic!(
                "Tried to kill running thread with id: {} from process {}",
                self.id, self.owning_process_id
            ),
            _ => self.state = ThreadState::Zombie,
        }
    }

    const fn resolve_interrupt(
        &mut self,
        synchronous: bool,
    ) -> Result<(), ThreadResolveInterruptError> {
        match self.state {
            ThreadState::Interrupted => {
                self.state = ThreadState::Ready;
                if synchronous {
                    self.pc += 4;
                }
                Ok(())
            }
            _ => Err(ThreadResolveInterruptError::ThreadNotInterrupted(
                self.state,
            )),
        }
    }
}

impl ThreadHandle<'_> {
    pub fn activate(
        &self,
        hart_id: usize,
    ) -> Result<ThreadActivationResult, ThreadActivationError> {
        unsafe {
            assert!((*self.thread).handle_lock.is_held());
            (*self.thread).activate(hart_id)
        }
    }

    pub fn consider(&self, best: u32) -> Option<u32> {
        unsafe {
            assert!((*self.thread).handle_lock.is_held());
            (*self.thread).consider(best)
        }
    }

    pub fn set_return_val(&self, val: usize) {
        unsafe {
            assert!((*self.thread).handle_lock.is_held());
            (*self.thread).set_return_val(val);
        }
    }

    pub fn kill(&self) {
        unsafe {
            assert!((*self.thread).handle_lock.is_held());
            (*self.thread).kill();
        }
    }

    pub fn resolve_interrupt(&self, synchronous: bool) -> Result<(), ThreadResolveInterruptError> {
        unsafe {
            assert!((*self.thread).handle_lock.is_held());
            (*self.thread).resolve_interrupt(synchronous)
        }
    }

    pub fn resolve_interrupt_or_kill(&self, synchronous: bool) {
        if self.resolve_interrupt(synchronous).is_err() {
            self.kill();
            println!("Mismatched thread state! Killing thread.");
        }
    }
}

impl Resource for Option<ThreadControlBlock> {
    fn exhausted(&self) -> bool {
        self.as_ref()
            .is_none_or(|thread| matches!(thread.state, ThreadState::Zombie))
    }
}
