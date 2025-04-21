use core::error::Error;
use core::fmt::Display;

use crate::resource::{Resource, ResourceClaimError, ResourceManager};
use crate::thread::ThreadHandle;

use super::consts::MAX_THREADS;
use super::thread::{CandidateThread, ThreadControlBlock};

/// The status of a process.
#[derive(Clone, Copy)]
pub enum ProcessStatus {
    /// Possibly has runnable threads.
    Ready,
    /// All threads are dead.
    _Zombie,
}

pub struct ProcessControlBlock {
    /// A unique value identifying this process.
    _id: u16,
    /// A container for threads belonging to this process.
    threads: ResourceManager<Option<ThreadControlBlock>, MAX_THREADS>,
    /// The priority of this process, used (eventually...) for scheduling.
    _priority: u16,
    /// The status of this process.
    status: ProcessStatus,
    /// A reference memory address. Should be removed now that the heap works.
    _memory_base: usize,
}

/// An error that may occur when creating a process control block.
#[derive(Debug)]
pub enum ProcessControlBlockCreationError {
    /// Failed to reserve space for the initial thread of this process.
    CouldNotClaimMainThread(ResourceClaimError),
    /// Initial thread of this process has a non-zero id somehow.
    MainThreadHasNonZeroID,
}

impl Display for ProcessControlBlockCreationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CouldNotClaimMainThread(inner_err) => write!(
                f,
                "Failed to claim main thread from resource manager due to error:\n{inner_err}"
            ),
            Self::MainThreadHasNonZeroID => write!(f, "Main thread was assigned non-zero ID."),
        }
    }
}

impl Error for ProcessControlBlockCreationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CouldNotClaimMainThread(err) => Some(err),
            Self::MainThreadHasNonZeroID => None,
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

impl ProcessControlBlock {
    /// Creates a new process control block, with entry function `main`.
    pub fn new(
        main: extern "C" fn() -> usize,
        id: u16,
        priority: u16,
        memory_base: usize,
    ) -> Result<Self, ProcessControlBlockCreationError> {
        let mut empty = Self {
            _id: id,
            threads: ResourceManager::new([const { None }; MAX_THREADS]),
            _priority: priority,
            status: ProcessStatus::Ready,
            _memory_base: memory_base,
        };

        match empty.threads.claim_first(Some(ThreadControlBlock::new(
            main,
            0,
            priority,
            memory_base,
            id,
        ))) {
            Ok(index) => match index {
                0 => Ok(empty),
                _ => Err(ProcessControlBlockCreationError::MainThreadHasNonZeroID),
            },
            Err(err) => Err(ProcessControlBlockCreationError::CouldNotClaimMainThread(
                err,
            )),
        }
    }

    /// Chooses a thread from amoung the threads owned by this process.
    pub fn choose<'a>(&'a mut self, mut candidate: CandidateThread<'a>) -> CandidateThread<'a> {
        for thread in (&mut self.threads.iter_mut()).flatten() {
            if let Ok(handle) = thread.get_handle() {
                if let Some(new_best) = handle.consider(candidate.best) {
                    candidate = CandidateThread::new(new_best, Some(handle));
                }
            }
        }
        candidate
    }
}

impl Resource for Option<ProcessControlBlock> {
    fn exhausted(&self) -> bool {
        self.as_ref()
            .is_none_or(|process| matches!(process.status, ProcessStatus::_Zombie))
    }
}

impl<const SIZE: usize> ResourceManager<Option<ProcessControlBlock>, SIZE> {
    /// Chooses a thread from amoung a pool of processes.
    pub fn choose_next_thread(&mut self) -> Option<ThreadHandle> {
        self.iter_mut()
            .fold(
                CandidateThread::default(),
                |acc, candidate| match candidate {
                    None => acc,
                    Some(candidate_pcb) => candidate_pcb.choose(acc),
                },
            )
            .handle
    }
}
