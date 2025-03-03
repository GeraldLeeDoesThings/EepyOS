use core::{
    cell::UnsafeCell,
    error::Error,
    fmt::Display,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::consts::MAX_LOCK_ACQUIRE_CYCLES;

pub struct Lock {
    claimed: AtomicBool,
}

pub struct Mutex<T> {
    guarded: UnsafeCell<T>,
    lock: Lock,
}

pub struct MutexGuardMut<'a, T: 'a> {
    mutex: &'a Mutex<T>,
}

pub struct MutexGuard<'a, T: 'a> {
    mutex: &'a Mutex<T>,
}

#[derive(Debug)]
pub enum MutexLockError {
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

    fn description(&self) -> &str {
        "description() is deprecated; use Display"
    }

    fn cause(&self) -> Option<&dyn Error> {
        self.source()
    }

    fn provide<'a>(&'a self, _request: &mut core::error::Request<'a>) {}
}

impl Lock {
    pub const fn new() -> Lock {
        Lock {
            claimed: AtomicBool::new(false),
        }
    }

    pub fn claim(&self) -> Result<bool, bool> {
        self.claimed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
    }

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

    pub fn release(&self) -> Result<bool, bool> {
        self.claimed
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
    }

    pub fn is_held(&self) -> bool {
        self.claimed.load(Ordering::SeqCst)
    }
}

impl<T> Mutex<T> {
    pub const fn new(val: T) -> Mutex<T> {
        Mutex {
            guarded: UnsafeCell::new(val),
            lock: Lock::new(),
        }
    }

    pub fn lock_mut(&self) -> Result<MutexGuardMut<'_, T>, MutexLockError> {
        match self.lock.claim() {
            Ok(_) => Ok(MutexGuardMut { mutex: self }),
            Err(_) => Err(MutexLockError::AlreadyHeld),
        }
    }

    #[allow(unused)]
    pub fn lock(&self) -> Result<MutexGuard<'_, T>, MutexLockError> {
        match self.lock.claim() {
            Ok(_) => Ok(MutexGuard { mutex: self }),
            Err(_) => Err(MutexLockError::AlreadyHeld),
        }
    }

    pub fn lock_blocking_mut(&self) -> MutexGuardMut<'_, T> {
        self.lock.claim_blocking();
        MutexGuardMut { mutex: self }
    }

    pub fn lock_blocking(&self) -> MutexGuard<'_, T> {
        self.lock.claim_blocking();
        MutexGuard { mutex: self }
    }

    pub fn is_held(&self) -> bool {
        self.lock.is_held()
    }
}

unsafe impl<T> Sync for Mutex<T> {}

impl<'a, T> Deref for MutexGuardMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe {
            self.mutex
                .guarded
                .get()
                .as_ref()
                .expect("Mutex wrapped null pointer!")
        }
    }
}

impl<'a, T> DerefMut for MutexGuardMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            self.mutex
                .guarded
                .get()
                .as_mut()
                .expect("Mutex wrapped null pointer!")
        }
    }
}

impl<'a, T> Drop for MutexGuardMut<'a, T> {
    fn drop(&mut self) {
        match self.mutex.lock.release() {
            Ok(_) => (),
            Err(_) => panic!("Mutex lock failed to release."),
        }
    }
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe {
            self.mutex
                .guarded
                .get()
                .as_ref()
                .expect("Mutex wrapped null pointer!")
        }
    }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        match self.mutex.lock.release() {
            Ok(_) => (),
            Err(_) => panic!("Mutex lock failed to release."),
        }
    }
}
