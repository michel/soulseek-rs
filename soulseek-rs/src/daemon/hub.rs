//! The single drainer, and the fan-out that makes multiple clients possible.
//!
//! The library's `take_*` accessors are destructive — they hand over their
//! buffer and empty it. That is fine for one process, but a daemon serving two
//! clients cannot let either of them drain: whoever asked first would silently
//! steal the other's events. So the daemon drains once, on its own cadence,
//! and every attached connection gets its own copy.
//!
//! A subscriber that stops reading must not be able to stall the drainer, so
//! the queues are bounded and a full one costs that client its connection
//! rather than costing everyone else their events.

use super::proto::{
    BrowseEvent, ChatMessageDto, DownloadStatusEvent, Event, Response,
    SessionLossEvent, UploadInfoDto, UserMessageDto,
};
use crate::api::SessionApi;
use soulseek_rs::DownloadStatus;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the drainer samples the session. The TUI already redraws on this
/// cadence, so events reach a remote UI as promptly as a local one.
pub const POLL: Duration = Duration::from_millis(100);

/// How many events may back up for one client before it is disconnected.
/// Generous enough that an ordinary consumer never notices, small enough that
/// a dead one is dropped promptly instead of growing without bound.
const QUEUE_DEPTH: usize = 1024;

/// How many private messages the daemon keeps for `message.history`, so a
/// client that attaches later still sees what arrived while nobody was
/// listening.
const HISTORY_LIMIT: usize = 500;

struct Subscriber {
    id: u64,
    sender: SyncSender<Response>,
}

/// Fans daemon-side events out to every attached connection.
pub struct Hub {
    subscribers: Mutex<Vec<Subscriber>>,
    next_id: AtomicU64,
    /// The conversation so far, newest last — both directions, so a client
    /// that attaches later sees a whole conversation rather than half of one.
    history: Mutex<Vec<ChatMessageDto>>,
}

impl Hub {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(0),
            history: Mutex::new(Vec::new()),
        }
    }

    /// Attach a connection. The returned receiver yields notifications until
    /// [`Hub::unsubscribe`] or a full queue drops it.
    pub fn subscribe(&self) -> (u64, Receiver<Response>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = sync_channel(QUEUE_DEPTH);
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(Subscriber { id, sender });
        }
        (id, receiver)
    }

    pub fn unsubscribe(&self, id: u64) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|subscriber| subscriber.id != id);
        }
    }

    pub fn subscribers(&self) -> usize {
        self.subscribers.lock().map_or(0, |subscribers| subscribers.len())
    }

    /// Send one event to everyone, dropping any client that has stopped
    /// keeping up. The drainer must never block on a subscriber.
    fn publish(&self, event: Event, params: serde_json::Value) {
        let Ok(mut subscribers) = self.subscribers.lock() else {
            return;
        };
        let notification = Response::notification(event.wire_name(), params);
        subscribers.retain(|subscriber| {
            match subscriber.sender.try_send(notification.clone()) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) => {
                    soulseek_rs::warn!(
                        "control client {} is not reading its events; dropping it",
                        subscriber.id
                    );
                    false
                }
                Err(TrySendError::Disconnected(_)) => false,
            }
        });
    }

    /// Add one line to the conversation, oldest lines falling off the front.
    pub fn remember(&self, message: ChatMessageDto) {
        let Ok(mut history) = self.history.lock() else {
            return;
        };
        history.push(message);
        let overflow = history.len().saturating_sub(HISTORY_LIMIT);
        if overflow > 0 {
            history.drain(0..overflow);
        }
    }

    /// Seed the conversation from disk at startup.
    pub fn restore(&self, messages: Vec<ChatMessageDto>) {
        if let Ok(mut history) = self.history.lock() {
            *history = messages;
        }
    }

    #[must_use]
    pub fn history(&self) -> Vec<ChatMessageDto> {
        self.history
            .lock()
            .map_or_else(|_| Vec::new(), |history| history.clone())
    }
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

/// A transfer the daemon started, whose status channel it pumps into events.
pub struct PendingDownload {
    pub username: String,
    pub filename: String,
    pub updates: Receiver<DownloadStatus>,
}

/// Everything the drainer owns between ticks.
///
/// Kept apart from [`Hub`] because it is single-threaded by construction: only
/// the drainer touches it, so none of it needs a lock.
#[derive(Default)]
struct DrainState {
    /// Last upload state reported per (user, file), so an upload that has not
    /// moved is not re-announced every tick.
    uploads: HashMap<(String, String), UploadInfoDto>,
    /// Whether the session-loss event has already gone out. It is terminal, so
    /// it is worth saying exactly once.
    reported_loss: bool,
}

/// Peers we have asked for a listing and are still waiting on. `take_browse_result`
/// is per-user and destructive, so the daemon has to know who to ask about.
#[derive(Default)]
pub struct PendingBrowses(Mutex<Vec<String>>);

impl PendingBrowses {
    pub fn expect(&self, username: &str) {
        if let Ok(mut pending) = self.0.lock()
            && !pending.iter().any(|name| name == username)
        {
            pending.push(username.to_string());
        }
    }

    fn snapshot(&self) -> Vec<String> {
        self.0.lock().map_or_else(|_| Vec::new(), |pending| pending.clone())
    }

    fn settled(&self, username: &str) {
        self.forget(username);
    }

    /// Stop waiting on a peer whose request never reached the wire.
    pub fn forget(&self, username: &str) {
        if let Ok(mut pending) = self.0.lock() {
            pending.retain(|name| name != username);
        }
    }
}

/// Drain everything the session has buffered and fan it out, forever.
///
/// This is the only caller of the `take_*` family in daemon mode; anything else
/// draining them would take events out of this loop's hands.
pub fn run(
    session: Arc<dyn SessionApi>,
    hub: Arc<Hub>,
    browses: Arc<PendingBrowses>,
    downloads: Arc<Mutex<Vec<PendingDownload>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    let mut state = DrainState::default();
    while !stop.load(Ordering::Relaxed) {
        tick(&*session, &hub, &browses, &downloads, &mut state);
        std::thread::sleep(POLL);
    }
}

fn tick(
    session: &dyn SessionApi,
    hub: &Hub,
    browses: &PendingBrowses,
    downloads: &Mutex<Vec<PendingDownload>>,
    state: &mut DrainState,
) {
    for event in session.take_room_events() {
        hub.publish(
            Event::Room,
            serde_json::to_value(super::proto::RoomEventDto::from(&event))
                .unwrap_or(serde_json::Value::Null),
        );
    }

    for message in session.take_private_messages() {
        let dto = UserMessageDto::from(&message);
        hub.remember(ChatMessageDto {
            peer: dto.username.clone(),
            outgoing: false,
            text: dto.message.clone(),
            at: i64::from(dto.timestamp),
        });
        hub.publish(
            Event::Message,
            serde_json::to_value(&dto).unwrap_or(serde_json::Value::Null),
        );
    }

    publish_uploads(session, hub, state);
    publish_browses(session, hub, browses);
    publish_downloads(hub, downloads);

    if !state.reported_loss
        && let Some(loss) = session.session_loss()
    {
        state.reported_loss = true;
        hub.publish(
            Event::SessionLoss,
            serde_json::to_value(SessionLossEvent { loss: loss.into() })
                .unwrap_or(serde_json::Value::Null),
        );
    }
}

/// Report an upload whenever it differs from what was last announced.
///
/// Both sources matter: the drained events carry states that came and went
/// between two ticks (a small file served straight out of the queue), and the
/// live table carries progress.
fn publish_uploads(session: &dyn SessionApi, hub: &Hub, state: &mut DrainState) {
    let sampled = session
        .take_upload_events()
        .iter()
        .chain(session.uploads().iter())
        .map(UploadInfoDto::from)
        .collect::<Vec<_>>();

    for upload in sampled {
        let key = (upload.username.clone(), upload.filename.clone());
        if state.uploads.get(&key) == Some(&upload) {
            continue;
        }
        hub.publish(
            Event::Upload,
            serde_json::to_value(&upload).unwrap_or(serde_json::Value::Null),
        );
        state.uploads.insert(key, upload);
    }
}

fn publish_browses(
    session: &dyn SessionApi,
    hub: &Hub,
    browses: &PendingBrowses,
) {
    for username in browses.snapshot() {
        let Some(directories) = session.take_browse_result(&username) else {
            continue;
        };
        browses.settled(&username);
        hub.publish(
            Event::Browse,
            serde_json::to_value(BrowseEvent {
                username,
                directories: directories.iter().map(Into::into).collect(),
            })
            .unwrap_or(serde_json::Value::Null),
        );
    }
}

/// Pump each transfer's status channel, dropping it once it can say no more.
fn publish_downloads(hub: &Hub, downloads: &Mutex<Vec<PendingDownload>>) {
    let Ok(mut downloads) = downloads.lock() else {
        return;
    };
    downloads.retain(|pending| {
        loop {
            match pending.updates.try_recv() {
                Ok(status) => hub.publish(
                    Event::DownloadStatus,
                    serde_json::to_value(DownloadStatusEvent {
                        username: pending.username.clone(),
                        filename: pending.filename.clone(),
                        status: (&status).into(),
                    })
                    .unwrap_or(serde_json::Value::Null),
                ),
                Err(std::sync::mpsc::TryRecvError::Empty) => return true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return false;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn drain(receiver: &Receiver<Response>) -> Vec<String> {
        let mut seen = Vec::new();
        while let Ok(response) = receiver.try_recv() {
            seen.push(response.method.unwrap_or_default());
        }
        seen
    }

    #[test]
    fn every_subscriber_receives_the_same_event() {
        // The whole reason the hub exists: two clients must both see an event
        // that only exists once in the library's buffer.
        let hub = Hub::new();
        let (_, first) = hub.subscribe();
        let (_, second) = hub.subscribe();

        hub.publish(Event::Room, json!({ "event": "left", "room": "lobby" }));

        assert_eq!(drain(&first), ["event.room"]);
        assert_eq!(drain(&second), ["event.room"]);
    }

    #[test]
    fn unsubscribing_stops_delivery() {
        let hub = Hub::new();
        let (id, receiver) = hub.subscribe();
        hub.unsubscribe(id);
        hub.publish(Event::Upload, json!({}));
        assert!(drain(&receiver).is_empty());
        assert_eq!(hub.subscribers(), 0);
    }

    #[test]
    fn a_client_that_stops_reading_is_dropped_and_the_others_keep_up() {
        let hub = Hub::new();
        let (_, slow) = hub.subscribe();
        let (_, healthy) = hub.subscribe();

        // Overrun the slow client's queue while the healthy one keeps reading,
        // which is exactly the asymmetry the bound exists to survive.
        let mut received = 0;
        for _ in 0..(QUEUE_DEPTH + 10) {
            hub.publish(Event::Upload, json!({}));
            received += drain(&healthy).len();
        }

        assert_eq!(
            hub.subscribers(),
            1,
            "a client that never reads must lose its connection, not stall the \
             drainer"
        );
        assert_eq!(
            received,
            QUEUE_DEPTH + 10,
            "the healthy client must not lose events to the slow one"
        );
        drop(slow);
    }

    #[test]
    fn a_dropped_receiver_removes_its_subscriber() {
        let hub = Hub::new();
        let (_, receiver) = hub.subscribe();
        drop(receiver);
        hub.publish(Event::Message, json!({}));
        assert_eq!(hub.subscribers(), 0);
    }

    #[test]
    fn history_keeps_the_newest_messages_within_the_cap() {
        let hub = Hub::new();
        for at in 0..(HISTORY_LIMIT as i64 + 5) {
            hub.remember(ChatMessageDto {
                peer: "bob".into(),
                outgoing: false,
                text: format!("m{at}"),
                at,
            });
        }
        let history = hub.history();
        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(history[0].at, 5, "the oldest messages are the ones dropped");
        assert_eq!(history[HISTORY_LIMIT - 1].at, HISTORY_LIMIT as i64 + 4);
    }

    #[test]
    fn history_carries_both_directions_of_a_conversation() {
        // Half a conversation is worse than none: a remote client must see
        // what this account said as well as what it was told.
        let hub = Hub::new();
        hub.remember(ChatMessageDto {
            peer: "bob".into(),
            outgoing: false,
            text: "hi".into(),
            at: 1,
        });
        hub.remember(ChatMessageDto {
            peer: "bob".into(),
            outgoing: true,
            text: "hello".into(),
            at: 2,
        });
        let history = hub.history();
        assert_eq!(history.len(), 2);
        assert!(!history[0].outgoing);
        assert!(history[1].outgoing);
    }

    #[test]
    fn a_browse_is_only_awaited_once() {
        let pending = PendingBrowses::default();
        pending.expect("bob");
        pending.expect("bob");
        assert_eq!(pending.snapshot(), ["bob"]);
        pending.settled("bob");
        assert!(pending.snapshot().is_empty());
    }
}
