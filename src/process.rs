use core::error::Error;
use core::fmt::Display;

use crate::resource::{Resource, ResourceClaimError, ResourceManager};
use crate::thread::ThreadHandle;

use super::consts::MAX_THREADS;
use super::thread::{CandidateThread, ThreadControlBlock};

#[derive(Clone, Copy)]
pub enum ProcessStatus {
    Ready,
    Zombie,
}

pub struct ProcessControlBlock {
    id: u16,
    threads: ResourceManager<Option<ThreadControlBlock>, MAX_THREADS>,
    priority: u16,
    status: ProcessStatus,
    memory_base: u64,
}

#[derive(Debug)]
pub enum ProcessControlBlockCreationError {
    CouldNotClaimMainThread(ResourceClaimError),
    MainThreadHasNonZeroID,
}

impl Display for ProcessControlBlockCreationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CouldNotClaimMainThread(inner_err) => write!(
                f,
                "Failed to claim main thread from resource manager due to error:\n{}",
                inner_err
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

    fn description(&self) -> &str {
        "description() is deprecated; use Display"
    }

    fn cause(&self) -> Option<&dyn Error> {
        self.source()
    }

    fn provide<'a>(&'a self, _request: &mut core::error::Request<'a>) {}
}

impl ProcessControlBlock {
    pub fn new(
        main: extern "C" fn() -> i64,
        id: u16,
        priority: u16,
        memory_base: u64,
    ) -> Result<ProcessControlBlock, ProcessControlBlockCreationError> {
        let mut empty = ProcessControlBlock {
            id: id,
            threads: ResourceManager::new([const { None }; MAX_THREADS]),
            priority: priority,
            status: ProcessStatus::Ready,
            memory_base: memory_base,
        };

        match empty.threads.claim_first(Some(ThreadControlBlock::new(
            main,
            0,
            priority,
            memory_base,
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

    pub fn choose<'a>(&'a mut self, mut candidate: CandidateThread<'a>) -> CandidateThread<'a> {
        for maybe_thread in &mut self.threads.iter_mut() {
            if let Some(thread) = maybe_thread {
                if let Ok(handle) = thread.get_handle() {
                    if let Some(new_best) = handle.consider(candidate.best) {
                        candidate = CandidateThread::new(new_best, Some(handle));
                    }
                }
            }
        }
        candidate
    }
}

impl Resource for Option<ProcessControlBlock> {
    fn exhausted(&self) -> bool {
        match self {
            None => true,
            Some(process) => match process.status {
                ProcessStatus::Zombie => true,
                _ => false,
            },
        }
    }
}

impl<const SIZE: usize> ResourceManager<Option<ProcessControlBlock>, SIZE> {
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
