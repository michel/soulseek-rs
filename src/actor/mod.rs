use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::trace;
use crate::utils::thread_pool::ThreadPool;

pub mod peer_actor;
pub mod peer_registry;
pub mod server_actor;

#[derive(Debug, Clone)]
pub enum ConnectionState {
    Disconnected,
    Connecting { since: Instant },
    Connected,
}
/// Core actor trait - each actor processes messages
pub trait Actor: Send + 'static {
    type Message: Send + Clone + 'static;

    /// Handle a single message
    fn handle(&mut self, msg: Self::Message);

    /// Called when actor starts (optional hook)
    fn on_start(&mut self) {}

    /// Called when actor stops (optional hook)
    fn on_stop(&mut self) {}

    /// Optional periodic tick for background work
    fn tick(&mut self) {}
}

#[derive(Clone)]
pub struct ActorHandle<M: Send> {
    pub(crate) sender: Sender<ActorMessage<M>>,
}

impl<M: Send> ActorHandle<M> {
    pub fn send(&self, msg: M) -> Result<(), String> {
        self.sender
            .send(ActorMessage::UserMessage(msg))
            .map_err(|e| format!("Failed to send message: {}", e))
    }

    /// Request actor to stop gracefully
    pub fn stop(&self) -> Result<(), String> {
        self.sender
            .send(ActorMessage::Stop)
            .map_err(|e| format!("Failed to send stop signal: {}", e))
    }
}

/// Internal actor message wrapper
pub(crate) enum ActorMessage<M> {
    UserMessage(M),
    Stop,
    #[allow(dead_code)]
    Tick,
}

/// Actor system that manages actor lifecycle
pub struct ActorSystem {
    thread_pool: Arc<ThreadPool>,
}

impl ActorSystem {
    pub fn new(thread_pool: Arc<ThreadPool>) -> Self {
        ActorSystem { thread_pool }
    }

    /// Spawn a new actor and return its handle
    pub fn spawn<A: Actor>(&self, mut actor: A) -> ActorHandle<A::Message> {
        let (sender, receiver) = channel::<ActorMessage<A::Message>>();
        let handle = ActorHandle {
            sender: sender.clone(),
        };

        // Start the actor event loop on the thread pool
        self.thread_pool.execute(move || {
            actor.on_start();
            Self::run_actor_loop(&mut actor, receiver);
            actor.on_stop();
        });

        handle
    }

    /// Spawn a new actor with initialization callback and return its handle
    /// The callback receives the actor handle before on_start is called
    pub fn spawn_with_handle<A: Actor, F>(
        &self,
        mut actor: A,
        init: F,
    ) -> ActorHandle<A::Message>
    where
        F: FnOnce(&mut A, ActorHandle<A::Message>) + Send + 'static,
    {
        let (sender, receiver) = channel::<ActorMessage<A::Message>>();
        let handle = ActorHandle {
            sender: sender.clone(),
        };
        let handle_for_init = handle.clone();

        self.thread_pool.execute(move || {
            init(&mut actor, handle_for_init);
            actor.on_start();
            Self::run_actor_loop(&mut actor, receiver);
            actor.on_stop();
        });

        handle
    }

    fn run_actor_loop<A: Actor>(
        actor: &mut A,
        receiver: Receiver<ActorMessage<A::Message>>,
    ) {
        let tick_interval = Duration::from_millis(100);
        let mut last_tick = Instant::now();
        let mut message_count = 0;
        let mut tick_count = 0;

        loop {
            match receiver.recv_timeout(tick_interval) {
                Ok(ActorMessage::UserMessage(msg)) => {
                    message_count += 1;
                    actor.handle(msg);
                }
                Ok(ActorMessage::Stop) => {
                    trace!(
                        "[actor_system] Received Stop message, breaking loop"
                    );
                    break;
                }
                Ok(ActorMessage::Tick) => {
                    tick_count += 1;
                    trace!(
                        "[actor_system] Received explicit Tick message #{}",
                        tick_count
                    );

                    actor.tick();
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if last_tick.elapsed() >= tick_interval {
                        tick_count += 1;
                        // if tick_count % 10 == 0 {
                        //     trace!("[actor_system] Periodic tick #{} (every 1s log)", tick_count);
                        // }
                        actor.tick();
                        last_tick = Instant::now();
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    trace!(
                        "[actor_system] Channel disconnected, breaking loop"
                    );
                    break;
                }
            }
        }
        trace!("[actor_system] run_actor_loop ENDED - processed {} messages, {} ticks", message_count, tick_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CounterActor {
        count: Arc<AtomicUsize>,
    }

    impl Actor for CounterActor {
        type Message = usize;

        fn handle(&mut self, msg: Self::Message) {
            self.count.fetch_add(msg, Ordering::SeqCst);
        }

        fn on_start(&mut self) {
            println!("Counter actor started");
        }

        fn on_stop(&mut self) {
            println!("Counter actor stopped");
        }
    }

    #[test]
    fn test_actor_system() {
        let thread_pool = Arc::new(ThreadPool::new(4));
        let system = ActorSystem::new(thread_pool);

        let count = Arc::new(AtomicUsize::new(0));
        let actor = CounterActor {
            count: count.clone(),
        };

        let handle = system.spawn(actor);

        // Send some messages
        handle.send(1).unwrap();
        handle.send(2).unwrap();
        handle.send(3).unwrap();

        // Give actor time to process
        std::thread::sleep(Duration::from_millis(100));

        assert_eq!(count.load(Ordering::SeqCst), 6);

        handle.stop().unwrap();
    }
}
