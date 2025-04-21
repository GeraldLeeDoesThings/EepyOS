use crate::{
    context::{activate_context, ActivationResult, RegisterContext},
    println,
    resource::Resource,
    sync::{Mutex, MutexGuardMut, MutexLockError},
    syscall::exit,
    time::set_timecmp_delay_ms,
};
use core::{error::Error, fmt::Display, ptr::addr_of};

/// The state of a thread.
#[derive(Clone, Copy, Debug)]
pub enum ThreadState {
    /// This thread has been interrupted. The interrupt itself is being
    /// processed by the kernel.
    Interrupted,
    /// This thread is currently running.
    Running,
    /// This thread is ready and able to run.
    Ready,
    /// This thread is never permitted to run again.
    Zombie,
}

/// A thread, and all the information needed to run it.
pub struct ThreadControlBlock {
    /// The thread's register values.
    registers: RegisterContext,
    /// The thread's program counter.
    pc: usize,
    /// The thread's state.
    state: ThreadState,
    /// A process-wise unique value.
    id: u16,
    /// This thread's scheduling priority.
    priority: u16,
    /// The number of times this thread has not been selected since last being
    /// run, multiplied by its [`ThreadControlBlock::priority`].
    need: u32,
    /// A globally unique value associated with the process that owns this
    /// thread.
    owning_process_id: u16,
    /// A mutex to guard the creation of handles to this thread.
    handle_lock: Mutex<()>,
}

/// The result of running a thread.
pub struct ThreadActivationResult<'a> {
    /// The thread that was run.
    pub thread: &'a mut ThreadControlBlock,
    /// A code indicating why `thread` stopped running.
    pub cause: usize,
}

// TODO: Stop using thread handles at all. (No more pointers!!)
/// A handle to `thread`, exposing some extra functions.
pub struct ThreadHandle<'a> {
    /// A mutex guard to ensure the thread's [`ThreadControlBlock::handle_lock`]
    /// is held for this object's lifetime.
    _guard: MutexGuardMut<'a, ()>,
    /// The thread which this handle references.
    thread: *mut ThreadControlBlock,
}

/// An error that may occur when activating (running) a thread.
#[derive(Debug)]
pub enum ThreadActivationError {
    /// Failed to claim a thread's handle.
    FailedToClaim(ThreadHandleClaimError),
    /// Thread state is not [`ThreadState::Ready`].
    ThreadNotReady(ThreadState),
}

/// An error that may occure when resolving a thread that has been interrupted.
#[derive(Debug)]
pub enum ThreadResolveInterruptError {
    /// Thread state is not [`ThreadState::Interrupted`]. Contains the actual
    /// thread state.
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

impl Error for ThreadActivationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FailedToClaim(err) => Some(err),
            Self::ThreadNotReady(_) => None,
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

/// An error that may occur when claiming a thread handle.
#[derive(Debug)]
pub enum ThreadHandleClaimError {
    /// The thread already has a handle.
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

/// A possible thread considered for activation.
pub struct CandidateThread<'a> {
    /// The best 'need' value observed.
    pub best: u32,
    /// A handle to the thread with the highest observed need.
    pub handle: Option<ThreadHandle<'a>>,
}

impl<'a> CandidateThread<'a> {
    /// Creates a new candidate thread.
    pub const fn new(best: u32, handle: Option<ThreadHandle<'a>>) -> Self {
        CandidateThread { best, handle }
    }
}

#[allow(clippy::derivable_impls, reason = "Being explicit.")]
impl Default for CandidateThread<'_> {
    fn default() -> Self {
        Self {
            best: 0,
            handle: None,
        }
    }
}

impl ThreadControlBlock {
    /// Creates a new thread control block.
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

    /// Attempts to retreieve a handle to this thread.
    ///
    /// # Errors
    ///
    /// Returns an error if a handle is already held to this thread.
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

    /// Attempts to activate this thread, running it until it is interrupted.
    /// The timer is configured to interrupt the thread after one second, if
    /// nothing else interrupts it first.
    ///
    /// # Errors
    ///
    /// This function returns an error if this thread is not ready to run.
    fn activate(
        &mut self,
        hart_id: usize,
    ) -> Result<ThreadActivationResult, ThreadActivationError> {
        match self.state {
            ThreadState::Ready => {
                self.need = u32::from(self.priority);
                self.state = ThreadState::Running;
                // SAFETY: asm wrapper.
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

    /// Consider this thread for running, returning `None` if this thread
    /// is not a better candidate than the thread corresponding to `best`.
    /// This function incremenets [`Self::need`] if this [`Self::state`] is
    /// [`ThreadState::Ready`].
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

    /// Retreives the registers corresponding to arguments (a0, a1).
    /// This function is intended to be used when handling syscalls,
    /// since this is the only way a thread can pass args to the kernel.
    pub const fn get_args(&self) -> [usize; 2] {
        [self.registers.a0, self.registers.a1]
    }

    /// Sets a return value for the thread, by setting the a0 register.
    /// This function is intended to be used when handling syscalls,
    /// since this is an idomatic way to return something to the thread
    /// from the kenrel.
    const fn set_return_val(&mut self, val: usize) {
        self.registers.a0 = val;
    }

    /// Returns this thread's need value, indicating how much priority
    /// should be given towards running this thread.
    pub const fn get_need(&self) -> u32 {
        self.need
    }

    /// Prevents this thread from being run again, by setting its state to
    /// [`ThreadState::Zombie`].
    ///
    /// # Panics
    ///
    /// This function panics if the thread is currently running.
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

    /// Prepares this thread to run again, after being interrupted.
    ///
    /// # Errors
    ///
    /// This function returns an error if the thread was not interrupted.
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
    /// Calls [`ThreadControlBlock::activate`] on the underlying thread.
    pub fn activate(
        &self,
        hart_id: usize,
    ) -> Result<ThreadActivationResult, ThreadActivationError> {
        // SAFETY: Pointer is from a reference.
        let thread = unsafe { self.thread.as_mut().unwrap() };
        assert!(thread.handle_lock.is_held());
        thread.activate(hart_id)
    }

    /// Calls [`ThreadControlBlock::consider`] on the underlying thread.
    pub fn consider(&self, best: u32) -> Option<u32> {
        // SAFETY: Pointer is from a reference.
        let thread = unsafe { self.thread.as_mut().unwrap() };
        assert!(thread.handle_lock.is_held());
        thread.consider(best)
    }

    /// Calls [`ThreadControlBlock::set_return_val`] on the underlying thread.
    pub fn set_return_val(&self, val: usize) {
        // SAFETY: Pointer is from a reference.
        let thread = unsafe { self.thread.as_mut().unwrap() };
        assert!(thread.handle_lock.is_held());
        thread.set_return_val(val);
    }

    /// Calls [`ThreadControlBlock::kill`] on the underlying thread.
    pub fn kill(&self) {
        // SAFETY: Pointer is from a reference.
        let thread = unsafe { self.thread.as_mut().unwrap() };
        assert!(thread.handle_lock.is_held());
        thread.kill();
    }

    /// Calls [`ThreadControlBlock::resolve_interrupt`] on the underlying
    /// thread.
    pub fn resolve_interrupt(&self, synchronous: bool) -> Result<(), ThreadResolveInterruptError> {
        // SAFETY: Pointer is from a reference.
        let thread = unsafe { self.thread.as_mut().unwrap() };
        assert!(thread.handle_lock.is_held());
        thread.resolve_interrupt(synchronous)
    }

    /// Calls [`ThreadControlBlock::resolve_interrupt`] on the underlying
    /// thread, and kills the thread (with [`ThreadControlBlock::kill`]) if
    /// it fails.
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
