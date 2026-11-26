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
    SessionLossEvent, UploadInfoDto, UploadStatusDto, UserMessageDto,
};
use crate::api::SessionApi;
use soulseek_rs::{DownloadStatus, SessionLoss, UploadInfo};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    /// How to end this client's connection. A stalled client cannot be
    /// reached any other way, and leaving it attached but silently eventless
    /// is worse than disconnecting it: it would keep working while missing
    /// every room message, transfer update and upload it should have seen.
    ///
    /// A closure rather than the socket itself, so the hub stays ignorant of
    /// what a connection is made of.
    disconnect: Box<dyn Fn() + Send + Sync>,
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
    /// [`Hub::unsubscribe`], or until a full queue costs the client its
    /// connection.
    pub fn subscribe(
        &self,
        disconnect: impl Fn() + Send + Sync + 'static,
    ) -> (u64, Receiver<Response>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = sync_channel(QUEUE_DEPTH);
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(Subscriber {
                id,
                sender,
                disconnect: Box::new(disconnect),
            });
        }
        (id, receiver)
    }

    pub fn unsubscribe(&self, id: u64) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|subscriber| subscriber.id != id);
        }
    }

    pub fn subscribers(&self) -> usize {
        self.subscribers
            .lock()
            .map_or(0, |subscribers| subscribers.len())
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
                        "control client {} is not reading its events; \
                         disconnecting it",
                        subscriber.id
                    );
                    // Closing the socket is the point: a client dropped from
                    // the fan-out but left connected would carry on believing
                    // it was subscribed, silently blind to everything after.
                    (subscriber.disconnect)();
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
    /// Last upload state announced per (user, file), for every upload still in
    /// the session's table, so one that has not moved is not re-announced
    /// every tick.
    uploads: HashMap<(String, String), UploadInfoDto>,
    last_loss: Option<SessionLoss>,
}

/// How long to keep asking whether a peer has answered a browse request.
///
/// A peer that is offline or uninterested never replies, and the daemon would
/// otherwise poll for it until restart — so a script that browsed a few
/// thousand absent peers would leave the drainer doing thousands of pointless
/// lookups every tick. Comfortably longer than any client's own browse
/// timeout, so this expiry never beats a real answer.
const BROWSE_PATIENCE: Duration = Duration::from_mins(2);

/// Peers we have asked for a listing and are still waiting on.
/// `take_browse_result` is per-user and destructive, so the daemon has to know
/// who to ask about.
#[derive(Default)]
pub struct PendingBrowses(Mutex<HashMap<String, Instant>>);

impl PendingBrowses {
    pub fn expect(&self, username: &str) {
        if let Ok(mut pending) = self.0.lock() {
            // Asking again restarts the clock: it is a fresh request.
            pending.insert(username.to_string(), Instant::now());
        }
    }

    /// Who to ask about this tick, forgetting the ones that have run out of
    /// patience.
    fn snapshot(&self) -> Vec<String> {
        let Ok(mut pending) = self.0.lock() else {
            return Vec::new();
        };
        pending.retain(|_, asked| asked.elapsed() < BROWSE_PATIENCE);
        pending.keys().cloned().collect()
    }

    /// Stop waiting on a peer — because it answered, or because its request
    /// never reached the wire.
    pub fn forget(&self, username: &str) {
        if let Ok(mut pending) = self.0.lock() {
            pending.remove(username);
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

    let loss = session.session_loss();
    if loss == state.last_loss {
        return;
    }
    state.last_loss = loss;
    if let Some(loss) = loss {
        hub.publish(
            Event::SessionLoss,
            serde_json::to_value(SessionLossEvent { loss: loss.into() })
                .unwrap_or(serde_json::Value::Null),
        );
    }
}

/// How many upload events one tick may send about uploads that will still be
/// there next tick.
///
/// A change to one upload can be news about all of them: serving the head of
/// the queue moves every waiting peer up a place, and a peer queueing an album
/// folder adds a row per file in one burst. Announced all at once, either
/// overruns a client's event queue inside a single tick and costs it its
/// connection, however fast it reads — which is what a long queue did to
/// every client that attached. Whatever does not fit is not lost: the next
/// tick compares against what was announced, not against what changed, so it
/// goes out then, and a row that moved twice in the meantime is announced
/// once. Comfortably under [`QUEUE_DEPTH`], and still over a thousand a
/// second.
const UPLOAD_EVENTS_PER_TICK: usize = 128;

/// Report an upload whenever it differs from what was last announced.
///
/// Both sources matter: the drained events carry states that came and went
/// between two ticks (a small file served straight out of the queue), and the
/// live table carries progress. Each upload is announced at most once a tick,
/// in its latest state.
fn publish_uploads(
    session: &dyn SessionApi,
    hub: &Hub,
    state: &mut DrainState,
) {
    let drained = session.take_upload_events();
    let live = session.uploads();
    publish_upload_sample(hub, state, &drained, &live);
}

/// One tick's worth of upload news: `drained` is what came and went since the
/// last tick, `live` the session's table as it stands.
fn publish_upload_sample(
    hub: &Hub,
    state: &mut DrainState,
    drained: &[UploadInfo],
    live: &[UploadInfo],
) {
    // One entry per upload, the latest state winning: the drained events are
    // older than the table, and within the table a finished transfer sorts
    // ahead of the same file queued or streaming again.
    let mut sampled: Vec<(UploadInfoDto, bool)> = Vec::new();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    let entries = drained
        .iter()
        .map(|upload| (upload, false))
        .chain(live.iter().map(|upload| (upload, true)));
    for (upload, is_live) in entries {
        let dto = UploadInfoDto::from(upload);
        let key = (dto.username.clone(), dto.filename.clone());
        if let Some(&at) = index.get(&key) {
            sampled[at] = (dto, is_live);
        } else {
            index.insert(key, sampled.len());
            sampled.push((dto, is_live));
        }
    }

    // An upload that is gone from the table cannot change again. Forgetting
    // it here is what keeps this map the size of the table rather than the
    // size of everything the daemon has ever served.
    state
        .uploads
        .retain(|key, _| index.get(key).is_some_and(|&at| sampled[at].1));

    let mut passing = Vec::new();
    let mut lasting = Vec::new();
    let mut shuffled = Vec::new();
    for (upload, is_live) in sampled {
        let key = (upload.username.clone(), upload.filename.clone());
        let announced = state.uploads.get(&key);
        if announced == Some(&upload) {
            continue;
        }
        if !is_live {
            passing.push(upload);
        } else if announced.is_some_and(|before| only_moved(before, &upload)) {
            shuffled.push(upload);
        } else {
            lasting.push(upload);
        }
    }

    // A state that only ever existed between two ticks is announced now or
    // never; the library caps how many of those it holds.
    for upload in &passing {
        announce(hub, upload);
    }
    // Everything else waits its turn, a new row or a change of status ahead
    // of a row that merely moved up the queue.
    for upload in lasting
        .into_iter()
        .chain(shuffled)
        .take(UPLOAD_EVENTS_PER_TICK)
    {
        announce(hub, &upload);
        state
            .uploads
            .insert((upload.username.clone(), upload.filename.clone()), upload);
    }
}

fn announce(hub: &Hub, upload: &UploadInfoDto) {
    hub.publish(
        Event::Upload,
        serde_json::to_value(upload).unwrap_or(serde_json::Value::Null),
    );
}

/// Whether the only news is a new place in the queue.
const fn only_moved(before: &UploadInfoDto, now: &UploadInfoDto) -> bool {
    matches!(before.status, UploadStatusDto::Queued { .. })
        && matches!(now.status, UploadStatusDto::Queued { .. })
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
        browses.forget(&username);
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
    use soulseek_rs::UploadStatus;

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
        let (_, first) = hub.subscribe(|| {});
        let (_, second) = hub.subscribe(|| {});

        hub.publish(Event::Room, json!({ "event": "left", "room": "lobby" }));

        assert_eq!(drain(&first), ["event.room"]);
        assert_eq!(drain(&second), ["event.room"]);
    }

    #[test]
    fn unsubscribing_stops_delivery() {
        let hub = Hub::new();
        let (id, receiver) = hub.subscribe(|| {});
        hub.unsubscribe(id);
        hub.publish(Event::Upload, json!({}));
        assert!(drain(&receiver).is_empty());
        assert_eq!(hub.subscribers(), 0);
    }

    #[test]
    fn a_client_that_stops_reading_is_dropped_and_the_others_keep_up() {
        let hub = Hub::new();
        let (_, slow) = hub.subscribe(|| {});
        let (_, healthy) = hub.subscribe(|| {});

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

    fn upload(user: &str, file: &str, status: UploadStatus) -> UploadInfo {
        UploadInfo {
            username: user.to_string(),
            filename: file.to_string(),
            size: 4096,
            bytes_sent: 0,
            speed_bytes_per_sec: 0.0,
            status,
        }
    }

    /// A queue of `rows` waiting uploads, places counted from `first_place`.
    fn queue(rows: usize, first_place: u32) -> Vec<UploadInfo> {
        (0..rows)
            .map(|row| {
                upload(
                    "amy",
                    &format!("{row}.mp3"),
                    UploadStatus::Queued(first_place + row as u32),
                )
            })
            .collect()
    }

    /// Run one tick and return how many events it sent.
    fn tick_uploads(
        hub: &Hub,
        state: &mut DrainState,
        receiver: &Receiver<Response>,
        drained: &[UploadInfo],
        live: &[UploadInfo],
    ) -> usize {
        publish_upload_sample(hub, state, drained, live);
        drain(receiver).len()
    }

    // Issue #112: with several thousand rows queued, a client that only
    // authenticated and read was sent an event per row and disconnected
    // within seconds, however fast it read.
    #[test]
    fn a_long_queue_cannot_overrun_a_client_that_is_reading() {
        let hub = Hub::new();
        let (_, receiver) = hub.subscribe(|| {});
        let mut state = DrainState::default();
        let rows = QUEUE_DEPTH * 8;

        // The queue appears, then its head is served and every row moves up.
        let mut sent = 0;
        for first_place in [1, 1, 1, 0, 0, 0] {
            let live = queue(rows, first_place + 1);
            let now = tick_uploads(&hub, &mut state, &receiver, &[], &live);
            assert!(
                now <= UPLOAD_EVENTS_PER_TICK,
                "one tick sent {now} events"
            );
            sent += now;
        }

        assert!(sent > 0, "the queue is still announced, a slice at a time");
        assert_eq!(
            hub.subscribers(),
            1,
            "a client that reads between ticks keeps its connection"
        );
    }

    #[test]
    fn rows_held_back_by_the_budget_are_announced_on_later_ticks() {
        let hub = Hub::new();
        let (_, receiver) = hub.subscribe(|| {});
        let mut state = DrainState::default();
        let rows = UPLOAD_EVENTS_PER_TICK * 3 + 7;
        let live = queue(rows, 1);

        let mut sent = 0;
        for _ in 0..4 {
            sent += tick_uploads(&hub, &mut state, &receiver, &[], &live);
        }
        assert_eq!(sent, rows, "every row is announced exactly once");
        assert_eq!(
            tick_uploads(&hub, &mut state, &receiver, &[], &live),
            0,
            "and an unchanged queue is then silent"
        );
    }

    #[test]
    fn a_new_upload_is_announced_ahead_of_rows_that_only_moved() {
        let hub = Hub::new();
        let (_, receiver) = hub.subscribe(|| {});
        let mut state = DrainState::default();
        let rows = UPLOAD_EVENTS_PER_TICK * 2;
        let mut live = queue(rows, 2);
        for _ in 0..2 {
            publish_upload_sample(&hub, &mut state, &[], &live);
        }
        let _ = drain(&receiver);

        // Everything moves up, and a transfer starts at the back of the table.
        live = queue(rows, 1);
        live.push(upload("bob", "new.mp3", UploadStatus::InProgress));
        publish_upload_sample(&hub, &mut state, &[], &live);

        let first = receiver.try_recv().expect("an event");
        assert_eq!(
            first.params.as_ref().and_then(|p| p["username"].as_str()),
            Some("bob"),
            "a change of status must not wait behind a queue shuffle"
        );
    }

    // A finished upload stays in the session's table as history. It used to
    // be announced again on every tick for as long as it was listed.
    #[test]
    fn a_finished_upload_is_announced_once() {
        let hub = Hub::new();
        let (_, receiver) = hub.subscribe(|| {});
        let mut state = DrainState::default();
        let live = [upload("amy", "a.mp3", UploadStatus::Completed)];

        assert_eq!(tick_uploads(&hub, &mut state, &receiver, &[], &live), 1);
        for _ in 0..5 {
            assert_eq!(
                tick_uploads(&hub, &mut state, &receiver, &[], &live),
                0
            );
        }
    }

    #[test]
    fn an_upload_listed_twice_is_announced_once_in_its_latest_state() {
        let hub = Hub::new();
        let (_, receiver) = hub.subscribe(|| {});
        let mut state = DrainState::default();
        // Served yesterday, being served again now.
        let live = [
            upload("amy", "a.mp3", UploadStatus::Completed),
            upload("amy", "a.mp3", UploadStatus::InProgress),
        ];

        publish_upload_sample(&hub, &mut state, &[], &live);
        let event = receiver.try_recv().expect("an event");
        assert_eq!(
            event
                .params
                .as_ref()
                .and_then(|p| p["status"]["state"].as_str()),
            Some("in_progress")
        );
        assert!(receiver.try_recv().is_err(), "one event, not two");
        assert_eq!(tick_uploads(&hub, &mut state, &receiver, &[], &live), 0);
    }

    #[test]
    fn a_state_that_came_and_went_between_ticks_is_still_announced() {
        let hub = Hub::new();
        let (_, receiver) = hub.subscribe(|| {});
        let mut state = DrainState::default();
        let drained = [upload("amy", "a.mp3", UploadStatus::Cancelled)];

        assert_eq!(tick_uploads(&hub, &mut state, &receiver, &drained, &[]), 1);
        assert!(
            state.uploads.is_empty(),
            "nothing is remembered about an upload that is gone"
        );
    }

    #[test]
    fn uploads_that_leave_the_table_are_forgotten() {
        let hub = Hub::new();
        let (_, receiver) = hub.subscribe(|| {});
        let mut state = DrainState::default();
        let live = queue(10, 1);
        publish_upload_sample(&hub, &mut state, &[], &live);
        assert_eq!(state.uploads.len(), 10);

        // The peer vanished and took its queue with it.
        publish_upload_sample(&hub, &mut state, &[], &[]);
        assert!(state.uploads.is_empty());
        let _ = drain(&receiver);
    }

    #[test]
    fn a_dropped_receiver_removes_its_subscriber() {
        let hub = Hub::new();
        let (_, receiver) = hub.subscribe(|| {});
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
        assert_eq!(
            history[0].at, 5,
            "the oldest messages are the ones dropped"
        );
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
        pending.forget("bob");
        assert!(pending.snapshot().is_empty());
    }

    #[test]
    fn a_browse_that_is_never_answered_stops_being_polled() {
        // A peer that is offline never replies; without an expiry the drainer
        // would look for its answer on every tick until the daemon restarts.
        let asked = Instant::now()
            .checked_sub(BROWSE_PATIENCE + Duration::from_secs(1))
            .expect("the clock is past the epoch");
        let pending = PendingBrowses(Mutex::new(HashMap::from([(
            "ghost".to_string(),
            asked,
        )])));
        assert!(pending.snapshot().is_empty());
    }
}
