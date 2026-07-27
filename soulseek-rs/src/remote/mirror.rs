//! The client-side copy of what the daemon drained.
//!
//! A remote session cannot ask the daemon to `take_*` on its behalf — that is
//! destructive, and with two clients attached the first to ask would steal the
//! other's events. So the daemon pushes, this mirror accumulates, and
//! `take_*` empties the mirror instead. From the caller's side the semantics
//! are identical: each event is delivered once, to each client.

use crate::daemon::proto::{
    BrowseEvent, DownloadStatusEvent, Event, RoomEventDto, SessionLossEvent,
    UploadInfoDto, UserMessageDto,
};
use soulseek_rs::{
    DownloadStatus, RoomEvent, SessionLoss, SharedDirectory, UploadInfo,
    UserMessage,
};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::Sender;

/// How many undrained events of one kind a client may accumulate.
///
/// Every attached client is sent everything, including events for work another
/// client asked for. A command that only drains room events would otherwise
/// grow its private-message buffer for as long as it runs. Dropping the oldest
/// is right for a buffer nobody is reading: the newest state is the useful one.
const BUFFER_LIMIT: usize = 4096;

/// Everything a pushed event can land in, behind one lock.
///
/// One lock rather than several: it is taken for the length of a `Vec::push`
/// on an event thread that has nothing else to do, and a single lock cannot
/// deadlock against itself.
#[derive(Default)]
struct Buffers {
    rooms: Vec<RoomEvent>,
    messages: Vec<UserMessage>,
    uploads: Vec<UploadInfo>,
    browses: HashMap<String, Vec<SharedDirectory>>,
    /// Where each in-flight transfer's status goes, keyed the way the daemon
    /// names it. Registered by `download`, dropped when the transfer ends.
    downloads: HashMap<(String, String), Sender<DownloadStatus>>,
    loss: Option<SessionLoss>,
}

#[derive(Default)]
pub struct Mirror(Mutex<Buffers>);

impl Mirror {
    /// File one pushed event. Anything unparseable is dropped with a warning
    /// rather than killing the connection: a newer daemon may legitimately
    /// send a shape this client does not know yet.
    pub fn apply(&self, event: Event, params: serde_json::Value) {
        let Ok(mut buffers) = self.0.lock() else {
            return;
        };
        match event {
            Event::Room => {
                if let Ok(room) = parse::<RoomEventDto>(event, params) {
                    push(&mut buffers.rooms, room.into());
                }
            }
            Event::Message => {
                if let Ok(message) = parse::<UserMessageDto>(event, params) {
                    push(&mut buffers.messages, message.into());
                }
            }
            Event::Upload => {
                if let Ok(upload) = parse::<UploadInfoDto>(event, params) {
                    push(&mut buffers.uploads, upload.into());
                }
            }
            Event::Browse => {
                if let Ok(browse) = parse::<BrowseEvent>(event, params) {
                    if buffers.browses.len() >= BUFFER_LIMIT {
                        buffers.browses.clear();
                    }
                    buffers.browses.insert(
                        browse.username,
                        browse
                            .directories
                            .into_iter()
                            .map(Into::into)
                            .collect(),
                    );
                }
            }
            Event::DownloadStatus => {
                if let Ok(update) = parse::<DownloadStatusEvent>(event, params)
                {
                    Self::route(&mut buffers, update);
                }
            }
            Event::SessionLoss => {
                if let Ok(loss) = parse::<SessionLossEvent>(event, params) {
                    // The first loss is the one worth keeping: a displaced
                    // session also drops its socket a moment later.
                    buffers.loss.get_or_insert_with(|| loss.loss.into());
                }
            }
        }
    }

    /// Deliver a transfer update to whoever is waiting on it, forgetting the
    /// transfer once it can produce nothing further.
    fn route(buffers: &mut Buffers, update: DownloadStatusEvent) {
        let key = (update.username, update.filename);
        let status = DownloadStatus::from(update.status);
        let terminal = matches!(
            status,
            DownloadStatus::Completed
                | DownloadStatus::Failed(_)
                | DownloadStatus::TimedOut
        );
        let Some(sender) = buffers.downloads.get(&key) else {
            return;
        };
        // A caller that dropped its receiver is not an error: it stopped
        // caring, which is exactly what `remove_download` does.
        let delivered = sender.send(status).is_ok();
        if terminal || !delivered {
            buffers.downloads.remove(&key);
        }
    }

    /// Stop tracking every transfer, so anything blocked on one is told the
    /// connection is gone.
    ///
    /// Dropping the senders is the signal: a caller waiting on its receiver
    /// sees a disconnect immediately instead of sitting out its full transfer
    /// timeout and then reporting a misleading "gave up".
    pub fn abandon_downloads(&self) {
        if let Ok(mut buffers) = self.0.lock() {
            buffers.downloads.clear();
        }
    }

    /// Register where a transfer's updates should go before asking for it, so
    /// an update that lands immediately is not dropped on the floor.
    pub fn expect_download(
        &self,
        username: &str,
        filename: &str,
        sender: Sender<DownloadStatus>,
    ) {
        if let Ok(mut buffers) = self.0.lock() {
            buffers
                .downloads
                .insert((username.to_string(), filename.to_string()), sender);
        }
    }

    pub fn forget_download(&self, username: &str, filename: &str) {
        if let Ok(mut buffers) = self.0.lock() {
            buffers
                .downloads
                .remove(&(username.to_string(), filename.to_string()));
        }
    }

    pub fn take_rooms(&self) -> Vec<RoomEvent> {
        self.take(|buffers| std::mem::take(&mut buffers.rooms))
    }

    pub fn take_messages(&self) -> Vec<UserMessage> {
        self.take(|buffers| std::mem::take(&mut buffers.messages))
    }

    pub fn take_uploads(&self) -> Vec<UploadInfo> {
        self.take(|buffers| std::mem::take(&mut buffers.uploads))
    }

    pub fn take_browse(&self, username: &str) -> Option<Vec<SharedDirectory>> {
        self.0.lock().ok()?.browses.remove(username)
    }

    pub fn loss(&self) -> Option<SessionLoss> {
        self.0.lock().ok()?.loss
    }

    fn take<T: Default>(&self, drain: impl FnOnce(&mut Buffers) -> T) -> T {
        self.0
            .lock()
            .map_or_else(|_| T::default(), |mut b| drain(&mut b))
    }
}

/// Append, dropping the oldest once the buffer is at its limit.
fn push<T>(buffer: &mut Vec<T>, item: T) {
    if buffer.len() >= BUFFER_LIMIT {
        buffer.remove(0);
    }
    buffer.push(item);
}

fn parse<T: serde::de::DeserializeOwned>(
    event: Event,
    params: serde_json::Value,
) -> Result<T, ()> {
    serde_json::from_value(params).map_err(|e| {
        soulseek_rs::warn!("ignoring an unreadable {event} event: {e}");
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::proto::DownloadStatusDto;
    use serde_json::json;

    #[test]
    fn a_room_event_is_delivered_once_and_then_gone() {
        let mirror = Mirror::default();
        mirror.apply(Event::Room, json!({ "event": "left", "room": "lobby" }));

        assert_eq!(
            mirror.take_rooms(),
            [RoomEvent::Left {
                room: "lobby".into()
            }]
        );
        assert!(
            mirror.take_rooms().is_empty(),
            "draining twice must not replay what was already handed over"
        );
    }

    #[test]
    fn a_browse_result_is_keyed_by_peer() {
        let mirror = Mirror::default();
        mirror.apply(
            Event::Browse,
            json!({
                "username": "bob",
                "directories": [{ "name": "@@x", "files": [["a.mp3", 42]] }],
            }),
        );

        assert!(mirror.take_browse("sue").is_none());
        let listing = mirror.take_browse("bob").expect("bob answered");
        assert_eq!(listing[0].name, "@@x");
        assert_eq!(listing[0].files, [("a.mp3".to_string(), 42)]);
        assert!(mirror.take_browse("bob").is_none(), "taken means taken");
    }

    #[test]
    fn a_transfer_update_reaches_the_caller_waiting_on_it() {
        let mirror = Mirror::default();
        let (sender, receiver) = std::sync::mpsc::channel();
        mirror.expect_download("bob", "a.mp3", sender);

        mirror.apply(
            Event::DownloadStatus,
            serde_json::to_value(DownloadStatusEvent {
                username: "bob".into(),
                filename: "a.mp3".into(),
                status: DownloadStatusDto::Queued,
            })
            .unwrap(),
        );

        assert!(matches!(receiver.try_recv(), Ok(DownloadStatus::Queued)));
    }

    #[test]
    fn a_finished_transfer_stops_being_tracked() {
        let mirror = Mirror::default();
        let (sender, receiver) = std::sync::mpsc::channel();
        mirror.expect_download("bob", "a.mp3", sender);

        for status in [DownloadStatusDto::Completed, DownloadStatusDto::Queued]
        {
            mirror.apply(
                Event::DownloadStatus,
                serde_json::to_value(DownloadStatusEvent {
                    username: "bob".into(),
                    filename: "a.mp3".into(),
                    status,
                })
                .unwrap(),
            );
        }

        assert!(matches!(receiver.try_recv(), Ok(DownloadStatus::Completed)));
        assert!(
            receiver.try_recv().is_err(),
            "a completed transfer must not keep receiving updates"
        );
    }

    #[test]
    fn an_update_for_an_unknown_transfer_is_harmless() {
        let mirror = Mirror::default();
        mirror.apply(
            Event::DownloadStatus,
            serde_json::to_value(DownloadStatusEvent {
                username: "nobody".into(),
                filename: "x.mp3".into(),
                status: DownloadStatusDto::Completed,
            })
            .unwrap(),
        );
    }

    #[test]
    fn the_first_session_loss_is_the_one_reported() {
        let mirror = Mirror::default();
        assert!(mirror.loss().is_none());
        mirror.apply(Event::SessionLoss, json!({ "loss": "displaced" }));
        mirror.apply(Event::SessionLoss, json!({ "loss": "disconnected" }));
        assert_eq!(
            mirror.loss(),
            Some(SessionLoss::Displaced),
            "being displaced is the reason worth reporting; the dropped socket \
             is its consequence"
        );
    }

    #[test]
    fn an_undrained_buffer_stops_growing_at_its_limit() {
        // A client is sent every event, including ones for work it never asked
        // for; a command that drains only rooms must not grow without bound.
        let mirror = Mirror::default();
        for _ in 0..(BUFFER_LIMIT + 50) {
            mirror.apply(
                Event::Room,
                json!({ "event": "left", "room": "lobby" }),
            );
        }
        assert_eq!(mirror.take_rooms().len(), BUFFER_LIMIT);
    }

    #[test]
    fn abandoning_transfers_wakes_whoever_is_waiting_on_them() {
        let mirror = Mirror::default();
        let (sender, receiver) = std::sync::mpsc::channel();
        mirror.expect_download("bob", "a.mp3", sender);

        mirror.abandon_downloads();

        assert!(
            matches!(receiver.recv(), Err(std::sync::mpsc::RecvError)),
            "a caller blocked on a transfer must be told the daemon is gone, \
             not left to time out"
        );
    }

    #[test]
    fn an_unreadable_event_is_ignored_rather_than_fatal() {
        // A newer daemon may send a shape this client does not know. Dropping
        // one event beats dropping the connection.
        let mirror = Mirror::default();
        mirror.apply(Event::Room, json!({ "event": "invented" }));
        assert!(mirror.take_rooms().is_empty());
    }
}
