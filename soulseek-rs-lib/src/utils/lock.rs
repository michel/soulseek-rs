use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::error::{Result, SoulseekRs};

pub trait RwLockExt<T> {
    fn read_safe(&self) -> Result<RwLockReadGuard<'_, T>>;
    fn write_safe(&self) -> Result<RwLockWriteGuard<'_, T>>;
}

impl<T> RwLockExt<T> for RwLock<T> {
    fn read_safe(&self) -> Result<RwLockReadGuard<'_, T>> {
        self.read().map_err(|_| SoulseekRs::LockPoisoned)
    }

    fn write_safe(&self) -> Result<RwLockWriteGuard<'_, T>> {
        self.write().map_err(|_| SoulseekRs::LockPoisoned)
    }
}

pub trait MutexExt<T> {
    fn lock_safe(&self) -> Result<MutexGuard<'_, T>>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_safe(&self) -> Result<MutexGuard<'_, T>> {
        self.lock().map_err(|_| SoulseekRs::LockPoisoned)
    }
}
