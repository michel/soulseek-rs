use std::cmp::Ordering as CmpOrdering;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::io::{self, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{
    Receiver, Sender, SyncSender, TryRecvError, TrySendError, channel,
    sync_channel,
};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mio::event::Event;
use mio::{Events, Poll, Registry, Token, Waker};

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
    type Message: Send + 'static;

    /// Handle a single message
    fn handle(&mut self, msg: Self::Message);

    /// Called when actor starts (optional hook)
    fn on_start(&mut self) {}

    /// Called when actor stops (optional hook)
    fn on_stop(&mut self) {}

    /// Optional periodic tick for background work
    fn tick(&mut self) {}

    /// Allows an actor to stop itself after handling a message, IO event, or tick.
    fn should_stop(&self) -> bool {
        false
    }

    /// Optional reactor callback for actors that own non-blocking IO.
    fn handle_io_event(&mut self, _registry: &Registry, _event: &Event) {}

    /// Monotonic value that changes when an actor's IO source or interest changes.
    fn io_generation(&self) -> u64 {
        0
    }

    /// Register an actor-owned IO source with the shared reactor.
    ///
    /// Returns `true` when an IO source is available and registered, and `false`
    /// for actors without an active IO source.
    fn register_io(
        &mut self,
        _registry: &Registry,
        _token: Token,
    ) -> io::Result<bool> {
        Ok(false)
    }

    /// Update the registered IO interest after actor state changes.
    fn reregister_io(
        &mut self,
        _registry: &Registry,
        _token: Token,
    ) -> io::Result<bool> {
        Ok(false)
    }

    /// Remove an actor-owned IO source from the shared reactor.
    fn deregister_io(&mut self, _registry: &Registry) -> io::Result<()> {
        Ok(())
    }

    /// Optional timer cadence. Returning `None` keeps the actor fully event-driven.
    fn tick_interval(&self) -> Option<Duration> {
        None
    }
}

pub struct ActorHandle<M: Send> {
    pub(crate) sender: SyncSender<ActorMessage<M>>,
    token: Token,
    ready_tokens: Arc<Mutex<VecDeque<Token>>>,
    wake_pending: Arc<AtomicBool>,
    waker: Arc<Waker>,
}

impl<M: Send> Clone for ActorHandle<M> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            token: self.token,
            ready_tokens: self.ready_tokens.clone(),
            wake_pending: self.wake_pending.clone(),
            waker: self.waker.clone(),
        }
    }
}

impl<M: Send> std::fmt::Debug for ActorHandle<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActorHandle")
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
}

impl<M: Send> ActorHandle<M> {
    pub fn send(&self, msg: M) -> Result<(), String> {
        self.sender
            .try_send(ActorMessage::UserMessage(msg))
            .map_err(mailbox_send_error)?;
        self.wake_actor();
        Ok(())
    }

    /// Request actor to stop gracefully
    pub fn stop(&self) -> Result<(), String> {
        self.sender
            .try_send(ActorMessage::Stop)
            .map_err(mailbox_send_error)?;
        self.wake_actor();
        Ok(())
    }

    fn wake_actor(&self) {
        if !self.wake_pending.swap(true, Ordering::AcqRel)
            && let Ok(mut ready_tokens) = self.ready_tokens.lock()
        {
            ready_tokens.push_back(self.token);
        }
        let _ = self.waker.wake();
    }
}

fn mailbox_send_error<M>(error: TrySendError<ActorMessage<M>>) -> String {
    match error {
        TrySendError::Full(_) => "actor mailbox is full".to_string(),
        TrySendError::Disconnected(_) => {
            "actor mailbox is disconnected".to_string()
        }
    }
}

/// Internal actor message wrapper
pub(crate) enum ActorMessage<M> {
    UserMessage(M),
    Stop,
}

/// Actor system that manages actor lifecycle
pub struct ActorSystem {
    command_sender: Sender<SystemCommand>,
    ready_tokens: Arc<Mutex<VecDeque<Token>>>,
    waker: Arc<Waker>,
    next_token: AtomicUsize,
    reactor_thread: Mutex<Option<JoinHandle<()>>>,
}

impl ActorSystem {
    #[must_use]
    pub fn new() -> Self {
        let poll = Poll::new().expect("failed to create mio poll reactor");
        let waker = Arc::new(
            Waker::new(poll.registry(), WAKE_TOKEN)
                .expect("failed to create mio reactor waker"),
        );
        let (command_sender, command_receiver) = channel::<SystemCommand>();
        let ready_tokens = Arc::new(Mutex::new(VecDeque::new()));
        let reactor_ready_tokens = ready_tokens.clone();
        let reactor_waker = waker.clone();

        let reactor_thread = thread::spawn(move || {
            Reactor::new(
                poll,
                command_receiver,
                reactor_ready_tokens,
                reactor_waker,
            )
            .run();
        });

        Self {
            command_sender,
            ready_tokens,
            waker,
            next_token: AtomicUsize::new(ACTOR_TOKEN_START),
            reactor_thread: Mutex::new(Some(reactor_thread)),
        }
    }

    /// Spawn a new actor and return its handle
    pub fn spawn<A: Actor>(&self, actor: A) -> ActorHandle<A::Message> {
        let (sender, receiver) =
            sync_channel::<ActorMessage<A::Message>>(ACTOR_MAILBOX_BOUND);
        let token = self.next_actor_token();
        let wake_pending = Arc::new(AtomicBool::new(false));
        let handle = ActorHandle {
            sender,
            token,
            ready_tokens: self.ready_tokens.clone(),
            wake_pending: wake_pending.clone(),
            waker: self.waker.clone(),
        };

        self.spawn_actor_cell(token, actor, receiver, wake_pending);

        handle
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
        let (sender, receiver) =
            sync_channel::<ActorMessage<A::Message>>(ACTOR_MAILBOX_BOUND);
        let token = self.next_actor_token();
        let wake_pending = Arc::new(AtomicBool::new(false));
        let handle = ActorHandle {
            sender,
            token,
            ready_tokens: self.ready_tokens.clone(),
            wake_pending: wake_pending.clone(),
            waker: self.waker.clone(),
        };
        let handle_for_init = handle.clone();

        init(&mut actor, handle_for_init);
        self.spawn_actor_cell(token, actor, receiver, wake_pending);

        handle
    }

    fn next_actor_token(&self) -> Token {
        Token(self.next_token.fetch_add(1, Ordering::Relaxed))
    }

    fn spawn_actor_cell<A: Actor>(
        &self,
        token: Token,
        actor: A,
        receiver: Receiver<ActorMessage<A::Message>>,
        wake_pending: Arc<AtomicBool>,
    ) {
        let actor =
            Box::new(RuntimeActor::new(token, actor, receiver, wake_pending));
        let _ = self
            .command_sender
            .send(SystemCommand::Spawn { token, actor });
        let _ = self.waker.wake();
    }
}

impl Default for ActorSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ActorSystem {
    fn drop(&mut self) {
        let _ = self.command_sender.send(SystemCommand::Shutdown);
        let _ = self.waker.wake();

        if let Ok(mut handle) = self.reactor_thread.lock()
            && let Some(handle) = handle.take()
        {
            let _ = handle.join();
        }
    }
}

pub(crate) struct OutboundBuffer {
    writes: VecDeque<PendingWrite>,
}

impl OutboundBuffer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            writes: VecDeque::new(),
        }
    }

    pub fn push(&mut self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            self.writes.push_back(PendingWrite { bytes, written: 0 });
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }

    pub fn flush<W: Write>(&mut self, writer: &mut W) -> io::Result<()> {
        while let Some(front) = self.writes.front_mut() {
            match writer.write(&front.bytes[front.written..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write queued actor bytes",
                    ));
                }
                Ok(bytes_written) => {
                    front.written += bytes_written;
                    if front.written == front.bytes.len() {
                        self.writes.pop_front();
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }
}

struct PendingWrite {
    bytes: Vec<u8>,
    written: usize,
}

const WAKE_TOKEN: Token = Token(0);
const ACTOR_TOKEN_START: usize = 1;
const ACTOR_MAILBOX_BOUND: usize = 1024;
const MAILBOX_DRAIN_BUDGET: usize = 64;

enum SystemCommand {
    Spawn {
        token: Token,
        actor: Box<dyn ActorCell>,
    },
    Shutdown,
}

trait ActorCell: Send {
    fn start(&mut self, registry: &Registry);
    fn drain_mailbox(&mut self, registry: &Registry) -> bool;
    fn handle_io_event(&mut self, registry: &Registry, event: &Event);
    fn tick(&mut self, registry: &Registry);
    fn stop(&mut self, registry: &Registry);
    fn is_stopped(&self) -> bool;
    fn scheduled_tick(&self) -> Option<(Instant, u64)>;
}

struct RuntimeActor<A: Actor> {
    token: Token,
    actor: A,
    receiver: Receiver<ActorMessage<A::Message>>,
    wake_pending: Arc<AtomicBool>,
    io_registered: bool,
    synced_io_generation: Option<u64>,
    stopped: bool,
    on_stop_called: bool,
    next_tick: Option<Instant>,
    tick_generation: u64,
    message_count: usize,
    tick_count: usize,
}

impl<A: Actor> RuntimeActor<A> {
    const fn new(
        token: Token,
        actor: A,
        receiver: Receiver<ActorMessage<A::Message>>,
        wake_pending: Arc<AtomicBool>,
    ) -> Self {
        Self {
            token,
            actor,
            receiver,
            wake_pending,
            io_registered: false,
            synced_io_generation: None,
            stopped: false,
            on_stop_called: false,
            next_tick: None,
            tick_generation: 0,
            message_count: 0,
            tick_count: 0,
        }
    }

    fn run_actor_call<F>(&mut self, label: &str, call: F)
    where
        F: FnOnce(&mut A),
    {
        if catch_unwind(AssertUnwindSafe(|| call(&mut self.actor))).is_err() {
            error!(
                "[actor_system:{:?}] actor panicked during {}",
                self.token, label
            );
            self.stopped = true;
        }
    }

    fn after_action(&mut self, registry: &Registry) {
        if self.actor.should_stop() {
            self.stopped = true;
        }

        if self.stopped {
            self.stop(registry);
            return;
        }

        let current_io_generation = self.actor.io_generation();
        if self.synced_io_generation != Some(current_io_generation) {
            self.sync_io_registration(registry, current_io_generation);
        }
        if self.stopped {
            self.stop(registry);
            return;
        }

        self.reschedule_tick();
    }

    fn sync_io_registration(
        &mut self,
        registry: &Registry,
        current_io_generation: u64,
    ) {
        let result = if self.io_registered {
            self.actor.reregister_io(registry, self.token)
        } else {
            self.actor.register_io(registry, self.token)
        };

        match result {
            Ok(registered) => {
                self.io_registered = registered;
                self.synced_io_generation = Some(current_io_generation);
            }
            Err(e) => {
                error!(
                    "[actor_system:{:?}] failed to sync IO registration: {}",
                    self.token, e
                );
                self.stopped = true;
            }
        }
    }

    fn reschedule_tick(&mut self) {
        self.tick_generation = self.tick_generation.wrapping_add(1);
        self.next_tick = self.actor.tick_interval().map(|interval| {
            Instant::now()
                .checked_add(interval)
                .unwrap_or_else(Instant::now)
        });
    }
}

impl<A: Actor> ActorCell for RuntimeActor<A> {
    fn start(&mut self, registry: &Registry) {
        self.run_actor_call("on_start", Actor::on_start);
        self.after_action(registry);
    }

    fn drain_mailbox(&mut self, registry: &Registry) -> bool {
        self.wake_pending.store(false, Ordering::Release);

        for _ in 0..MAILBOX_DRAIN_BUDGET {
            match self.receiver.try_recv() {
                Ok(ActorMessage::UserMessage(msg)) => {
                    self.message_count += 1;
                    self.run_actor_call("handle", |actor| actor.handle(msg));
                    self.after_action(registry);
                    if self.stopped {
                        return false;
                    }
                }
                Ok(ActorMessage::Stop) => {
                    trace!(
                        "[actor_system:{:?}] received Stop message",
                        self.token
                    );
                    self.stop(registry);
                    return false;
                }
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => {
                    trace!(
                        "[actor_system:{:?}] channel disconnected",
                        self.token
                    );
                    self.stop(registry);
                    return false;
                }
            }
        }

        self.wake_pending.store(true, Ordering::Release);
        true
    }

    fn handle_io_event(&mut self, registry: &Registry, event: &Event) {
        self.run_actor_call("handle_io_event", |actor| {
            actor.handle_io_event(registry, event);
        });
        self.after_action(registry);
    }

    fn tick(&mut self, registry: &Registry) {
        self.tick_count += 1;
        self.run_actor_call("tick", Actor::tick);
        self.after_action(registry);
    }

    fn stop(&mut self, registry: &Registry) {
        if self.io_registered {
            if let Err(e) = self.actor.deregister_io(registry) {
                error!(
                    "[actor_system:{:?}] failed to deregister IO: {}",
                    self.token, e
                );
            }
            self.io_registered = false;
        }

        if !self.on_stop_called {
            self.run_actor_call("on_stop", Actor::on_stop);
            self.on_stop_called = true;
        }

        self.stopped = true;

        trace!(
            "[actor_system:{:?}] actor stopped - processed {} messages, {} ticks",
            self.token, self.message_count, self.tick_count
        );
    }

    fn is_stopped(&self) -> bool {
        self.stopped
    }

    fn scheduled_tick(&self) -> Option<(Instant, u64)> {
        self.next_tick.map(|tick| (tick, self.tick_generation))
    }
}

struct Reactor {
    poll: Poll,
    events: Events,
    commands: Receiver<SystemCommand>,
    ready_tokens: Arc<Mutex<VecDeque<Token>>>,
    waker: Arc<Waker>,
    actors: HashMap<usize, Box<dyn ActorCell>>,
    ticks: BinaryHeap<ScheduledTick>,
    shutdown: bool,
}

impl Reactor {
    fn new(
        poll: Poll,
        commands: Receiver<SystemCommand>,
        ready_tokens: Arc<Mutex<VecDeque<Token>>>,
        waker: Arc<Waker>,
    ) -> Self {
        Self {
            poll,
            events: Events::with_capacity(1024),
            commands,
            ready_tokens,
            waker,
            actors: HashMap::new(),
            ticks: BinaryHeap::new(),
            shutdown: false,
        }
    }

    fn run(&mut self) {
        while !self.shutdown {
            let timeout = self.poll_timeout();
            match self.poll.poll(&mut self.events, timeout) {
                Ok(()) => {}
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    error!("[actor_system] reactor poll failed: {}", e);
                    break;
                }
            }

            let ready_events: Vec<Event> =
                self.events.iter().cloned().collect();
            for event in ready_events {
                if event.token() == WAKE_TOKEN {
                    self.drain_commands();
                    self.drain_ready_actors();
                } else {
                    self.dispatch_io_event(event);
                }
            }

            self.drain_commands();
            self.drain_ready_actors();
            self.process_due_ticks();
        }

        self.stop_all();
    }

    fn poll_timeout(&self) -> Option<Duration> {
        self.ticks.peek().map(|tick| {
            tick.when
                .checked_duration_since(Instant::now())
                .unwrap_or_default()
        })
    }

    fn drain_commands(&mut self) {
        loop {
            match self.commands.try_recv() {
                Ok(command) => self.handle_command(command),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.shutdown = true;
                    break;
                }
            }
        }
    }

    fn handle_command(&mut self, command: SystemCommand) {
        match command {
            SystemCommand::Spawn { token, mut actor } => {
                actor.start(self.poll.registry());
                if actor.is_stopped() {
                    actor.stop(self.poll.registry());
                } else {
                    self.schedule_actor(token, &*actor);
                    self.actors.insert(token.0, actor);
                }
            }
            SystemCommand::Shutdown => {
                self.shutdown = true;
            }
        }
    }

    fn drain_ready_actors(&mut self) {
        let ready_tokens = match self.ready_tokens.lock() {
            Ok(mut ready_tokens) => ready_tokens.drain(..).collect::<Vec<_>>(),
            Err(_) => return,
        };

        for token in ready_tokens {
            let needs_reschedule = self
                .with_actor(token, |actor, registry| {
                    actor.drain_mailbox(registry)
                })
                .unwrap_or(false);

            if needs_reschedule
                && let Ok(mut ready_tokens) = self.ready_tokens.lock()
            {
                ready_tokens.push_back(token);
                let _ = self.waker.wake();
            }
        }
    }

    fn dispatch_io_event(&mut self, event: Event) {
        let token = event.token();
        let _ = self.with_actor(token, |actor, registry| {
            actor.handle_io_event(registry, &event);
        });
    }

    fn process_due_ticks(&mut self) {
        let now = Instant::now();

        while self.ticks.peek().is_some_and(|tick| tick.when <= now) {
            let Some(tick) = self.ticks.pop() else {
                break;
            };

            let token = Token(tick.token);
            let Some(actor) = self.actors.get(&tick.token) else {
                continue;
            };
            if actor.scheduled_tick() != Some((tick.when, tick.generation)) {
                continue;
            }

            let _ = self.with_actor(token, |actor, registry| {
                actor.tick(registry);
            });
        }
    }

    fn with_actor<F, R>(&mut self, token: Token, action: F) -> Option<R>
    where
        F: FnOnce(&mut dyn ActorCell, &Registry) -> R,
    {
        let mut actor = self.actors.remove(&token.0)?;

        let result = action(&mut *actor, self.poll.registry());

        if actor.is_stopped() {
            actor.stop(self.poll.registry());
        } else {
            self.schedule_actor(token, &*actor);
            self.actors.insert(token.0, actor);
        }

        Some(result)
    }

    fn schedule_actor(&mut self, token: Token, actor: &dyn ActorCell) {
        if let Some((when, generation)) = actor.scheduled_tick() {
            self.ticks.push(ScheduledTick {
                when,
                token: token.0,
                generation,
            });
        }
    }

    fn stop_all(&mut self) {
        for (_, mut actor) in self.actors.drain() {
            actor.stop(self.poll.registry());
        }
        self.ticks.clear();
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ScheduledTick {
    when: Instant,
    token: usize,
    generation: u64,
}

impl Ord for ScheduledTick {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other
            .when
            .cmp(&self.when)
            .then_with(|| other.token.cmp(&self.token))
            .then_with(|| other.generation.cmp(&self.generation))
    }
}

impl PartialOrd for ScheduledTick {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

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

        // Stop the actor
        handle.stop().unwrap();

        // Give actor time to process the stop message
        std::thread::sleep(Duration::from_millis(100));
    }

    #[test]
    fn actor_reschedules_after_mailbox_budget() {
        let system = ActorSystem::new();

        let count = Arc::new(AtomicUsize::new(0));
        let actor = CounterActor {
            count: count.clone(),
        };
        let handle = system.spawn(actor);
        let message_count = MAILBOX_DRAIN_BUDGET + 8;

        for _ in 0..message_count {
            handle.send(1).unwrap();
        }

        let deadline = Instant::now() + Duration::from_secs(1);
        while count.load(Ordering::SeqCst) < message_count
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(count.load(Ordering::SeqCst), message_count);
    }
}
