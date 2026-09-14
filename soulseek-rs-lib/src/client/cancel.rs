use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[derive(Clone)]
pub struct CancelHandle(Arc<Mutex<bool>>);

impl CancelHandle {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(false)))
    }

    pub fn cancel(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = true;
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn live(&self) -> Option<MutexGuard<'_, bool>> {
        let guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        (!*guard).then_some(guard)
    }
}
