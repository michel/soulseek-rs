use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

type Job = Box<dyn FnOnce() + Send + 'static>;

pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: mpsc::Sender<Job>,
    active_threads: Arc<AtomicUsize>,
}

impl ThreadPool {
    pub fn new(size: usize) -> ThreadPool {
        assert!(size > 0);

        let (sender, receiver) = mpsc::channel();
        let receiver = Arc::new(Mutex::new(receiver));
        let active_threads = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::with_capacity(size);

        for id in 0..size {
            workers.push(Worker::new(
                id,
                Arc::clone(&receiver),
                Arc::clone(&active_threads),
                size,
            ));
        }

        ThreadPool {
            workers,
            sender,
            active_threads,
        }
    }

    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let job = Box::new(f);
        self.sender.send(job).unwrap();
    }

    pub fn thread_count(&self) -> usize {
        self.workers.len()
    }

    pub fn active_thread_count(&self) -> usize {
        self.active_threads.load(Ordering::SeqCst)
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                thread.join().unwrap();
            }
        }
    }
}

struct Worker {
    _id: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn new(
        id: usize,
        receiver: Arc<Mutex<mpsc::Receiver<Job>>>,
        _active_threads: Arc<AtomicUsize>,
        _total_threads: usize,
    ) -> Worker {
        let thread = thread::spawn(move || loop {
            let job = receiver.lock().unwrap().recv();
            match job {
                Ok(job) => {
                    // let active =
                    //     active_threads.fetch_add(1, Ordering::SeqCst) + 1;
                    // trace!(
                    //     "Thread {} started job (active: {}/{})",
                    //     id,
                    //     active,
                    //     total_threads
                    // );

                    job();

                    // let active =
                    // active_threads.fetch_sub(1, Ordering::SeqCst) - 1;
                    // trace!(
                    //     "Thread {} finished job (active: {}/{})",
                    //     id,
                    //     active,
                    //     total_threads
                    // );
                }
                Err(_) => {
                    break;
                }
            }
        });

        Worker {
            _id: id,
            thread: Some(thread),
        }
    }
}
