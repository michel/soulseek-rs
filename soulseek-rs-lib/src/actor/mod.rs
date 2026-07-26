use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use crate::{error, trace};

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
            .map_err(|e| format!("Failed to send message: {e}"))
    }

    /// Request actor to stop gracefully
    pub fn stop(&self) -> Result<(), String> {
        self.sender
            .send(ActorMessage::Stop)
            .map_err(|e| format!("Failed to send stop signal: {e}"))
    }
}

/// Internal actor message wrapper
pub(crate) enum ActorMessage<M> {
    UserMessage(M),
    Stop,
}

/// Stack size for an actor thread. An actor loop parses messages into
/// heap-allocated buffers and never recurses, so it needs a fraction of the
/// 2 MiB default — and at a thousand concurrent peers that difference is the
/// difference between 2 GiB of reserved address space and 256 MiB.
const ACTOR_STACK_SIZE: usize = 256 * 1024;

/// Actor system that manages actor lifecycle.
///
/// Each actor owns a thread for its whole lifetime. That is deliberate: an
/// actor loop blocks in `recv_timeout` until it is stopped, so running actors
/// on a fixed-size pool would make the pool a semaphore capping the client at
/// one peer per core — and every peer past the cap would silently never start.
#[derive(Default)]
pub struct ActorSystem;

impl ActorSystem {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Spawn a new actor and return its handle
    pub fn spawn<A: Actor>(&self, actor: A) -> ActorHandle<A::Message> {
        self.spawn_with_handle(actor, |_, _| {})
    }

    /// Spawn a new actor with initialization callback and return its handle
    /// The callback receives the actor handle before `on_start` is called
    pub fn spawn_with_handle<A: Actor, F>(
        &self,
        mut actor: A,
        init: F,
    ) -> ActorHandle<A::Message>
    where
        F: FnOnce(&mut A, ActorHandle<A::Message>) + Send + 'static,
    {
        let (sender, receiver) = channel::<ActorMessage<A::Message>>();
        let handle = ActorHandle { sender };
        let handle_for_init = handle.clone();

        let spawned = thread::Builder::new()
            .name("soulseek-actor".to_string())
            .stack_size(ACTOR_STACK_SIZE)
            .spawn(move || {
                init(&mut actor, handle_for_init);
                actor.on_start();
                Self::run_actor_loop(&mut actor, receiver);
                actor.on_stop();
            });

        if let Err(e) = spawned {
            error!("[actor_system] failed to spawn actor thread: {}", e);
        }

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
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if last_tick.elapsed() >= tick_interval {
                        tick_count += 1;
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
        trace!(
            "[actor_system] run_actor_loop ENDED - processed {} messages, {} ticks",
            message_count, tick_count
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
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
        let system = ActorSystem::new();

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

        // Give actor time to process the stop message
        std::thread::sleep(Duration::from_millis(100));
    }

    // Actors must not be capped by a worker count: every peer runs one, and a
    // cap silently strands every peer past it. Far more actors than cores must
    // all make progress concurrently.
    #[test]
    fn far_more_actors_than_cores_all_run() {
        let system = ActorSystem::new();
        let count = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..256)
            .map(|_| {
                system.spawn(CounterActor {
                    count: count.clone(),
                })
            })
            .collect();
        for handle in &handles {
            handle.send(1).unwrap();
        }

        let deadline = Instant::now() + Duration::from_secs(10);
        while count.load(Ordering::SeqCst) < 256 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            count.load(Ordering::SeqCst),
            256,
            "every actor should have processed its message"
        );

        for handle in &handles {
            handle.stop().unwrap();
        }
    }
}
