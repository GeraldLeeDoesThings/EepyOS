use core::{
    cell::UnsafeCell,
    error::Error,
    fmt::Display,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::consts::MAX_LOCK_ACQUIRE_CYCLES;

/// A lock primitive for synchronization.
pub struct Lock {
    /// A bool representing if this lock is currently held.
    claimed: AtomicBool,
}

/// A guard around an object of type `T` that synchronizes all accesses
/// with a lock.
pub struct Mutex<T> {
    /// The object being guarded by this mutex.
    guarded: UnsafeCell<T>,
    /// A lock to synchronize accesses with.
    lock: Lock,
}

/// A held mutex, guarding a mutable reference to it guarded data.
/// When this guard is dropped, the mutex is released, allowing
/// other threads to access the underlying object.
pub struct MutexGuardMut<'a, T: 'a> {
    /// The mutex being held.
    mutex: &'a Mutex<T>,
}

/// A held mutex, guarding a reference to it guarded data.
/// When this guard is dropped, the mutex is released, allowing
/// other threads to access the underlying object.
pub struct MutexGuard<'a, T: 'a> {
    /// The mutex being held.
    mutex: &'a Mutex<T>,
}

/// An error that may occur when claiming a mutex.
#[derive(Debug)]
pub enum MutexLockError {
    /// The mutex is already held.
    AlreadyHeld,
}

impl Display for MutexLockError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AlreadyHeld => write!(f, "Mutex is already held."),
        }
    }
}

impl Error for MutexLockError {
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

impl Lock {
    /// Creates a new lock, which is initally not held.
    pub const fn new() -> Self {
        Self {
            claimed: AtomicBool::new(false),
        }
    }

    /// Tries to claim this lock. Return value is determined by
    /// [`AtomicBool::compare_exchange`].
    pub fn claim(&self) -> Result<bool, bool> {
        self.claimed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
    }

    /// Repeatedly tries to claim this lock until successful.
    ///
    /// # Panics
    ///
    /// This function panics if it fails to acquire the lock for too many
    /// attempts.
    pub fn claim_blocking(&self) {
        let mut claimed = self.claim();
        let mut limit: usize = 0;
        while claimed.is_err() && limit < MAX_LOCK_ACQUIRE_CYCLES {
            claimed = self.claim();
            limit += 1;
        }
        assert!(
            limit < MAX_LOCK_ACQUIRE_CYCLES,
            "Took too long to claim lock!"
        );
        assert!(self.is_held());
    }

    /// Tries to release this lock. Return value is determined by
    /// [`AtomicBool::compare_exchange`].
    pub fn release(&self) -> Result<bool, bool> {
        self.claimed
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
    }

    /// Returns `true` if this lock is currently held.
    pub fn is_held(&self) -> bool {
        self.claimed.load(Ordering::SeqCst)
    }
}

// SAFETY: Lock is synchronized with atomic operations.
unsafe impl Sync for Lock {}

impl<T> Mutex<T> {
    /// Creates a new mutex guarding `val`.
    pub const fn new(val: T) -> Self {
        Self {
            guarded: UnsafeCell::new(val),
            lock: Lock::new(),
        }
    }

    /// Attempts to obtain a mutable reference to the value guarded by this
    /// mutex. For an infallible version of this function, see
    /// [`Self::lock_blocking_mut`].
    ///
    /// # Errors
    ///
    /// This function returns an error if this mutex is already held.
    pub fn lock_mut(&self) -> Result<MutexGuardMut<'_, T>, MutexLockError> {
        match self.lock.claim() {
            Ok(_) => Ok(MutexGuardMut { mutex: self }),
            Err(_) => Err(MutexLockError::AlreadyHeld),
        }
    }

    /// Attempts to obtain a reference to the value guarded by this mutex.
    /// For an infallible version of this function, see [`Self::lock_blocking`].
    ///
    /// # Errors
    ///
    /// This function returns an error if this mutex is already held.
    #[allow(unused, reason = "May be used later")]
    pub fn lock(&self) -> Result<MutexGuard<'_, T>, MutexLockError> {
        match self.lock.claim() {
            Ok(_) => Ok(MutexGuard { mutex: self }),
            Err(_) => Err(MutexLockError::AlreadyHeld),
        }
    }

    /// Repeatedly tries to obtain a mutable reference to the value guarded by
    /// this mutex until successful. For a non-blocking version of this
    /// function, see [`Self::lock_mut`].
    ///
    /// # Panics
    ///
    /// This function panics if it cannot claim this mutex's internal lock in
    /// time.
    pub fn lock_blocking_mut(&self) -> MutexGuardMut<'_, T> {
        self.lock.claim_blocking();
        MutexGuardMut { mutex: self }
    }

    /// Repeatedly tries to obtain a reference to the value guarded by this
    /// mutex until successful. For a non-blocking version of this function,
    /// see [`Self::lock`].
    ///
    /// # Panics
    ///
    /// This function panics if it cannot claim this mutex's internal lock in
    /// time.
    pub fn lock_blocking(&self) -> MutexGuard<'_, T> {
        self.lock.claim_blocking();
        MutexGuard { mutex: self }
    }

    /// Returns `true` if this mutex's internal lock is currently held.
    pub fn is_held(&self) -> bool {
        self.lock.is_held()
    }
}

// SAFETY: Mutex guards access with a lock, which is thread-safe.
unsafe impl<T> Sync for Mutex<T> {}

impl<T> Deref for MutexGuardMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: Access is unique since creation of this guard requires claiming a
        // lock.
        unsafe {
            self.mutex
                .guarded
                .get()
                .as_ref()
                .expect("Mutex wrapped null pointer!")
        }
    }
}

impl<T> DerefMut for MutexGuardMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: Access is unique since creation of this guard requires claiming a
        // lock.
        unsafe {
            self.mutex
                .guarded
                .get()
                .as_mut()
                .expect("Mutex wrapped null pointer!")
        }
    }
}

#[allow(clippy::match_wild_err_arm, reason = "Invariant violation.")]
impl<T> Drop for MutexGuardMut<'_, T> {
    fn drop(&mut self) {
        match self.mutex.lock.release() {
            Ok(_) => (),
            Err(_) => panic!("Mutex lock failed to release."),
        }
    }
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: Access is unique since creation of this guard requires claiming a
        // lock.
        unsafe {
            self.mutex
                .guarded
                .get()
                .as_ref()
                .expect("Mutex wrapped null pointer!")
        }
    }
}

#[allow(clippy::match_wild_err_arm, reason = "Invariant violation.")]
impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        match self.mutex.lock.release() {
            Ok(_) => (),
            Err(_) => panic!("Mutex lock failed to release."),
        }
    }
}
