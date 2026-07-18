use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use crate::utils::lock::MutexExt;

type Job = Box<dyn FnOnce() + Send + 'static>;

enum Message {
    NewJob(Job),
    Terminate,
}

pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: Option<mpsc::Sender<Message>>,
}

impl ThreadPool {
    pub fn new(size: usize) -> ThreadPool {
        assert!(size > 0);

        let (sender, receiver) = mpsc::channel();
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(size);

        for _ in 0..size {
            workers.push(Worker::new(Arc::clone(&receiver)));
        }

        ThreadPool {
            workers,
            sender: Some(sender),
        }
    }

    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let job = Box::new(f);
        if let Some(ref sender) = self.sender {
            let _ = sender.send(Message::NewJob(job));
        }
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            for _ in &self.workers {
                let _ = sender.send(Message::Terminate);
            }
        }

        // Ignore join errors: a worker whose job panicked returns Err here, but
        // Drop must never itself panic (that would abort the process).
        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ThreadPool;
    use std::sync::mpsc;
    use std::time::Duration;

    // A single panicking job must not permanently kill its worker: subsequent
    // jobs still need to run. Otherwise one malformed network message could
    // leak a pool worker until the whole pool is exhausted (a DoS).
    #[test]
    fn pool_survives_a_panicking_job() {
        let pool = ThreadPool::new(1);
        pool.execute(|| panic!("boom"));

        let (tx, rx) = mpsc::channel();
        pool.execute(move || {
            let _ = tx.send(42);
        });

        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).ok(),
            Some(42),
            "worker died after a panicking job"
        );
    }
}

struct Worker {
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn new(receiver: Arc<Mutex<mpsc::Receiver<Message>>>) -> Worker {
        let thread = thread::spawn(move || {
            loop {
                let message = match receiver.lock_safe() {
                    Ok(rx) => rx.recv(),
                    Err(_) => break,
                };
                match message {
                    // Contain a panicking job so it kills only that job, not the
                    // worker: the pool has a fixed number of workers and a lost
                    // one is never replaced. The lock is already released here,
                    // so a panic cannot poison the shared receiver.
                    Ok(Message::NewJob(job)) => {
                        let _ = std::panic::catch_unwind(
                            std::panic::AssertUnwindSafe(job),
                        );
                    }
                    Ok(Message::Terminate) | Err(_) => break,
                }
            }
        });

        Worker {
            thread: Some(thread),
        }
    }
}
