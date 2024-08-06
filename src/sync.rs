use core::{
    error::Error,
    fmt::Display,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

pub struct Lock {
    claimed: AtomicBool,
}

pub struct Mutex<T> {
    guarded: T,
    lock: Lock,
}

pub struct MutexGuard<'a, T: 'a> {
    mutex: &'a mut Mutex<T>,
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
    pub fn new() -> Lock {
        Lock {
            claimed: AtomicBool::new(false),
        }
    }

    pub fn claim(&self) -> Result<bool, bool> {
        self.claimed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
    }

    pub fn release(&self) -> Result<bool, bool> {
        self.claimed
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::Relaxed)
    }

    pub fn is_held(&self) -> bool {
        self.claimed.load(Ordering::Relaxed)
    }
}

impl<T> Mutex<T> {
    pub fn new(val: T) -> Mutex<T> {
        Mutex {
            guarded: val,
            lock: Lock::new(),
        }
    }

    pub fn lock(&mut self) -> Result<MutexGuard<'_, T>, MutexLockError> {
        match self.lock.claim() {
            Ok(_) => Ok(MutexGuard { mutex: self }),
            Err(_) => Err(MutexLockError::AlreadyHeld),
        }
    }

    pub fn is_held(&self) -> bool {
        self.lock.is_held()
    }
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.mutex.guarded
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.mutex.guarded
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
