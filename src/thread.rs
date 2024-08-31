use crate::{
    println,
    resource::Resource,
    sync::{Mutex, MutexGuard, MutexLockError},
    syscall::exit, time::{set_timecmp, set_timecmp_delay_ms},
};
use core::{error::Error, fmt::Display, ptr::addr_of};

use super::context::{activate_context, ActivationResult, RegisterContext};

#[derive(Clone, Copy, Debug)]
pub enum ThreadState {
    Interrupted,
    Running,
    Ready,
    Zombie,
}

pub struct ThreadControlBlock {
    registers: RegisterContext,
    pc: u64,
    state: ThreadState,
    id: u16,
    priority: u16,
    need: u32,
    handle_lock: Mutex<()>,
}

pub struct ThreadActivationResult<'a> {
    pub thread: &'a mut ThreadControlBlock,
    pub cause: u64,
}

pub struct ThreadHandle<'a> {
    _guard: MutexGuard<'a, ()>,
    thread: *mut ThreadControlBlock,
}

#[derive(Debug)]
pub enum ThreadActivationError {
    FailedToClaim(ThreadHandleClaimError),
    ThreadNotReady(ThreadState),
}

#[derive(Debug)]
pub enum ThreadResolveInterruptError {
    ThreadNotInterrupted(ThreadState)
}

impl Display for ThreadState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ThreadState::Interrupted => write!(f, "Interrupted"),
            ThreadState::Running => write!(f, "Running"),
            ThreadState::Ready => write!(f, "Ready"),
            ThreadState::Zombie => write!(f, "Zombie"),
        }
    }
}

impl Display for ThreadActivationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FailedToClaim(err) => write!(f, "Error while claiming thread:\n{}", err),
            Self::ThreadNotReady(state) => write!(
                f,
                "Thread state must be 'Ready', but the state is '{}'.",
                state
            ),
        }
    }
}

impl Display for ThreadResolveInterruptError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ThreadNotInterrupted(state) => write!(f, "Thread state must be 'Interrupted', but the state is '{}'.", state),
        }
    }
}

impl Error for ThreadActivationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FailedToClaim(err) => Some(err),
            _ => None,
        }
    }

    fn description(&self) -> &str {
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
            ThreadHandleClaimError::HandleAlreadyClaimed(reason) => {
                write!(f, "Thread handle is already claimed: {}", reason)
            }
        }
    }
}

impl Error for ThreadHandleClaimError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }

    fn description(&self) -> &str {
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
    pub fn new(best: u32, handle: Option<ThreadHandle<'a>>) -> CandidateThread<'a> {
        CandidateThread {
            best: best,
            handle: handle,
        }
    }
}

impl<'a> Default for CandidateThread<'a> {
    fn default() -> Self {
        Self {
            best: 0,
            handle: None,
        }
    }
}

impl<'a> ThreadControlBlock {
    pub fn new(
        code: extern "C" fn() -> u64,
        id: u16,
        priority: u16,
        stack_base: u64,
    ) -> ThreadControlBlock {
        let mut tcb = ThreadControlBlock {
            registers: RegisterContext::all_zero(),
            pc: code as u64,
            state: ThreadState::Ready,
            id: id,
            priority: priority,
            need: priority as u32,
            handle_lock: Mutex::new(()),
        };
        tcb.registers.sp = stack_base;
        tcb.registers.ra = exit as u64;
        tcb
    }

    pub fn get_handle(&mut self) -> Result<ThreadHandle<'_>, ThreadHandleClaimError> {
        let t: *mut ThreadControlBlock = self;
        match self.handle_lock.lock() {
            Ok(handle) => Ok(ThreadHandle {
                _guard: handle,
                thread: t,
            }),
            Err(mutex_err) => return Err(ThreadHandleClaimError::HandleAlreadyClaimed(mutex_err)),
        }
    }

    fn activate(&mut self, hart_id: u64) -> Result<ThreadActivationResult, ThreadActivationError> {
        match self.state {
            ThreadState::Ready => {
                self.need = self.priority as u32;
                self.state = ThreadState::Running;
                unsafe {
                    set_timecmp_delay_ms(1000);
                    let result: ActivationResult =
                        activate_context(self.pc, addr_of!(self.registers) as u64, hart_id);
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

    fn consider(&mut self, best: u32) -> Option<u32> {
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

    pub fn get_args(&self) -> [u64; 2] {
        [self.registers.a0, self.registers.a1]
    }

    fn set_return_val(&mut self, val: u64) {
        self.registers.a0 = val;
    }

    pub fn get_need(&self) -> u32 {
        self.need
    }

    fn kill(&mut self) {
        println!("Killing thread with id {}", self.id);
        match self.state {
            ThreadState::Running => panic!("Tried to kill running thread with id: {}", self.id),
            _ => self.state = ThreadState::Zombie,
        }
    }

    fn resolve_interrupt(&mut self) -> Result<(), ThreadResolveInterruptError> {
        match self.state {
            ThreadState::Interrupted => {
                self.state = ThreadState::Ready;
                Ok(())
            }
            _ => Err(ThreadResolveInterruptError::ThreadNotInterrupted(self.state))
        }
    }
}

impl<'a> ThreadHandle<'a> {
    pub fn activate(&self, hart_id: u64) -> Result<ThreadActivationResult, ThreadActivationError> {
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

    pub fn set_return_val(&self, val: u64) {
        unsafe {
            assert!((*self.thread).handle_lock.is_held());
            (*self.thread).set_return_val(val)
        }
    }

    pub fn kill(&self) {
        unsafe {
            assert!((*self.thread).handle_lock.is_held());
            (*self.thread).kill()
        }
    }

    pub fn resolve_interrupt(&self) -> Result<(), ThreadResolveInterruptError> {
        unsafe {
            assert!((*self.thread).handle_lock.is_held());
            (*self.thread).resolve_interrupt()
        }
    }
}

impl Resource for Option<ThreadControlBlock> {
    fn exhausted(&self) -> bool {
        match self {
            None => true,
            Some(thread) => match thread.state {
                ThreadState::Zombie => true,
                _ => false,
            },
        }
    }
}
