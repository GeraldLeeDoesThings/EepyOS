/// The maximum number of processes.
pub const MAX_PROCESSES: usize = 4;
/// The maximum number of threads.
pub const MAX_THREADS: usize = 2;

/// Default stack size for a new process.
pub const _DEFAULT_STACK_SIZE: usize = 4096;
/// Number of cycles to wait before failing to acquire a lock.
/// The locks used by the kernel must never be held excessively long.
pub const MAX_LOCK_ACQUIRE_CYCLES: usize = 10_000_000;
