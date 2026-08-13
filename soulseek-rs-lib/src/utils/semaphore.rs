use std::sync::{Arc, Condvar, Mutex, PoisonError};

pub struct Semaphore {
    permits: Mutex<usize>,
    available: Condvar,
}

pub struct Permit(Arc<Semaphore>);

impl Semaphore {
    #[must_use]
    pub const fn new(permits: usize) -> Self {
        Self {
            permits: Mutex::new(permits),
            available: Condvar::new(),
        }
    }

    #[must_use]
    pub fn acquire(self: &Arc<Self>) -> Permit {
        let mut permits =
            self.permits.lock().unwrap_or_else(PoisonError::into_inner);
        while *permits == 0 {
            permits = self
                .available
                .wait(permits)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *permits -= 1;
        Permit(Arc::clone(self))
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut permits = self
            .0
            .permits
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *permits += 1;
        self.0.available.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn permits_bound_concurrent_holders() {
        let semaphore = Arc::new(Semaphore::new(2));
        let first = semaphore.acquire();
        let _second = semaphore.acquire();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _third = semaphore.acquire();
            tx.send(()).unwrap();
        });

        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "a third acquire must block while both permits are held"
        );
        drop(first);
        assert!(
            rx.recv_timeout(Duration::from_secs(2)).is_ok(),
            "releasing a permit must wake the waiter"
        );
    }
}
