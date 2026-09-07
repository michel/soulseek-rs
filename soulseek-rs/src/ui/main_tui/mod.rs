mod browse;
mod downloads;
mod input;
mod render;
mod rooms;
mod search;
mod settings;

use crate::api::SessionApi;
use crate::models::AppState;
use crate::persist::{
    snapshot::{Snapshot, restore_messages, restore_searches},
    state::{PersistedMessage, StateStore},
};
use color_eyre::Result;
use ratatui::{
    DefaultTerminal,
    crossterm::event::{self, Event, KeyEventKind, poll},
};
use std::{sync::Arc, time::Duration};

pub struct MainTui {
    client: Arc<dyn SessionApi>,
    state: AppState,
    download_dir: String,
    search_timeout: Duration,
    spinner_state: usize,
    store: Option<StateStore>,
    /// Where settings changes are persisted — the config file this run
    /// resolved, so `--config` is honoured.
    config_path: Option<std::path::PathBuf>,
    /// Last snapshot written to disk, to skip no-op saves.
    saved_snapshot: Snapshot,
}

impl MainTui {
    pub fn new(
        client: Arc<dyn SessionApi>,
        download_dir: String,
        search_timeout: Duration,
        store: Option<StateStore>,
        config_path: Option<std::path::PathBuf>,
    ) -> Self {
        let mut tui = Self {
            client,
            state: AppState::new(),
            download_dir,
            search_timeout,
            spinner_state: 0,
            store,
            config_path,
            saved_snapshot: Snapshot::default(),
        };
        tui.restore_persisted_state();
        tui
    }

    /// Bring back last session's state: search history, chat rooms
    /// (rejoined on the server), and downloads — incomplete ones are
    /// re-enqueued so they resume automatically.
    fn restore_persisted_state(&mut self) {
        // Whatever the session has been collecting comes first: attached to a
        // daemon this is the conversation so far, including everything that
        // arrived while nothing was showing it. Locally it is empty and the
        // snapshot below is the whole record.
        let history: Vec<PersistedMessage> = self
            .client
            .message_history()
            .into_iter()
            .map(|message| PersistedMessage {
                peer: message.peer,
                outgoing: message.outgoing,
                text: message.text,
                at: chrono::DateTime::from_timestamp(message.at, 0)
                    .unwrap_or_default()
                    .into(),
            })
            .collect();
        restore_messages(&mut self.state, &history);

        let Some(store) = &self.store else {
            self.saved_snapshot = Snapshot::capture(&self.state);
            return;
        };

        restore_searches(&mut self.state, &store.load_search_queries());
        restore_messages(&mut self.state, &store.load_messages());
        // Restore the unread badge so messages left unread carry over. Set
        // before the baseline snapshot below so it isn't seen as a change.
        self.state.unread_messages = store.load_unread();

        for room in store.load_rooms() {
            if self.state.rooms.focus_or_open(&room)
                && let Err(e) = self.client.join_room(&room)
            {
                soulseek_rs::warn!("Could not rejoin {room}: {e}");
            }
        }

        let downloads = store.load_downloads();
        self.saved_snapshot = Snapshot::capture(&self.state);
        // Completed entries are shown as-is; the rest re-enqueue below and
        // reappear through the normal downloads channel.
        self.saved_snapshot.downloads.clone_from(&downloads);

        let sender = self.downloads_sender();
        for entry in downloads {
            if entry.completed {
                self.state.downloads.push(crate::models::DownloadEntry {
                    download: soulseek_rs::types::Download {
                        username: entry.username,
                        filename: entry.filename,
                        token: 0,
                        size: entry.size,
                        download_directory: entry.download_directory,
                        status: soulseek_rs::DownloadStatus::Completed,
                        sender: std::sync::mpsc::channel().0,
                        queue_position: None,
                        metadata: soulseek_rs::types::DownloadMetadata::default(
                        ),
                    },
                    receiver: None,
                });
            } else {
                let client = self.client.clone();
                let sender = sender.clone();
                std::thread::spawn(move || {
                    match client.download(
                        entry.filename.clone(),
                        entry.username,
                        entry.size,
                        entry.download_directory,
                    ) {
                        Ok((download, rx)) => {
                            let _ = sender.send((download, rx));
                        }
                        Err(e) => soulseek_rs::warn!(
                            "Could not resume {}: {e}",
                            entry.filename
                        ),
                    }
                });
            }
        }
    }

    /// Write state to disk when it differs from what was last saved.
    fn save_persisted_state(&mut self) {
        let Some(store) = &self.store else { return };
        let snapshot = Snapshot::capture(&self.state);
        if snapshot == self.saved_snapshot {
            return;
        }
        if snapshot.downloads != self.saved_snapshot.downloads
            && let Err(e) = store.save_downloads(&snapshot.downloads)
        {
            soulseek_rs::warn!("Could not save downloads state: {e}");
        }
        if snapshot.queries != self.saved_snapshot.queries
            && let Err(e) = store.save_search_queries(&snapshot.queries)
        {
            soulseek_rs::warn!("Could not save search history: {e}");
        }
        if snapshot.rooms != self.saved_snapshot.rooms
            && let Err(e) = store.save_rooms(&snapshot.rooms)
        {
            soulseek_rs::warn!("Could not save room state: {e}");
        }
        if snapshot.messages != self.saved_snapshot.messages
            && let Err(e) = store.save_messages(&snapshot.messages)
        {
            soulseek_rs::warn!("Could not save message history: {e}");
        }
        if snapshot.unread_messages != self.saved_snapshot.unread_messages
            && let Err(e) = store.save_unread(snapshot.unread_messages)
        {
            soulseek_rs::warn!("Could not save unread count: {e}");
        }
        self.saved_snapshot = snapshot;
    }

    pub fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        use ratatui::crossterm::{event::DisableMouseCapture, execute};

        // Run the event loop, then restore the terminal unconditionally: if the
        // loop returns early with an error the terminal must still be taken out
        // of raw mode / the alternate screen and mouse capture disabled, or the
        // user is left with a corrupted terminal.
        let result = self.run_event_loop(&mut terminal);
        self.save_persisted_state();

        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        ratatui::restore();
        soulseek_rs::utils::logger::disable_buffering();

        result
    }

    fn run_event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.state.should_exit {
            terminal.draw(|frame| self.render(frame))?;

            // Poll for search results updates
            self.update_search_results();

            // Poll for download updates
            self.update_downloads();

            // Poll for incoming private messages
            self.poll_private_messages();

            // Poll for a browse (shared-file listing) response
            self.poll_browse_result();

            // Poll for chat-room events
            self.poll_room_events();

            // Refresh the uploads we are serving to peers
            self.state.uploads = self.client.uploads();

            self.spinner_state = (self.spinner_state + 1) % 10;

            self.save_persisted_state();

            // Drain every queued input event before the next draw: key
            // autorepeat outpaces the frame time, and handling one event per
            // frame makes the backlog keep scrolling for seconds after the
            // key is released.
            if poll(Duration::from_millis(100))? {
                loop {
                    match event::read()? {
                        Event::Key(key) if key.kind == KeyEventKind::Press => {
                            self.handle_key_event(key);
                        }
                        Event::Mouse(mouse) => {
                            self.handle_mouse_event(mouse);
                        }
                        _ => {}
                    }
                    if self.state.should_exit || !poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

pub fn launch_main_tui(
    terminal: DefaultTerminal,
    client: Arc<dyn SessionApi>,
    download_dir: String,
    search_timeout: Duration,
    store: Option<StateStore>,
    config_path: Option<std::path::PathBuf>,
) -> Result<()> {
    let tui =
        MainTui::new(client, download_dir, search_timeout, store, config_path);
    tui.run(terminal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::proto::ChatMessageDto;
    use crate::models::{FocusedPane, MessageDirection, SearchStatus};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// A stand-in for a daemon: it already holds a conversation, a transfer
    /// queue, and a search cache, none of which this window created.
    #[derive(Default)]
    struct TalkativeSession {
        history: Vec<ChatMessageDto>,
        downloads: Vec<soulseek_rs::types::Download>,
        searches: std::sync::Mutex<Vec<crate::api::SessionSearch>>,
        /// What `try_get_search_results` hands back to the window.
        results: std::sync::Mutex<Vec<soulseek_rs::SearchResult>>,
        /// How many times a window asked for the search list.
        all_searches_calls: std::sync::atomic::AtomicUsize,
        /// How many times a window pulled a full result set across. Against a
        /// daemon each pull is the whole set over the socket, so the tests
        /// count them.
        result_fetches: std::sync::atomic::AtomicUsize,
        /// Whether this session is a daemon's. Local sessions do not sync.
        shared: bool,
        /// What `set_download_directory` was last asked for, if anything.
        download_dir_set: std::sync::Mutex<Option<String>>,
        /// What `set_shared_directories` was last asked for, if anything.
        shares_set: std::sync::Mutex<Option<Vec<String>>>,
        download_cancelled: std::sync::Mutex<Option<(String, String)>>,
        upload_cancelled: std::sync::Mutex<Option<(String, String)>>,
    }

    fn queued(username: &str, filename: &str) -> soulseek_rs::types::Download {
        at_status(username, filename, soulseek_rs::DownloadStatus::Queued)
    }

    fn completed(
        username: &str,
        filename: &str,
    ) -> soulseek_rs::types::Download {
        at_status(username, filename, soulseek_rs::DownloadStatus::Completed)
    }

    fn at_status(
        username: &str,
        filename: &str,
        status: soulseek_rs::DownloadStatus,
    ) -> soulseek_rs::types::Download {
        let (sender, _) = std::sync::mpsc::channel();
        soulseek_rs::types::Download {
            username: username.to_string(),
            filename: filename.to_string(),
            token: 1,
            size: 4096,
            download_directory: "/tmp".to_string(),
            status,
            sender,
            queue_position: None,
            metadata: soulseek_rs::types::DownloadMetadata::default(),
        }
    }

    fn key(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
    }

    impl SessionApi for TalkativeSession {
        fn message_history(&self) -> Vec<ChatMessageDto> {
            self.history.clone()
        }
        fn daemon_endpoint(&self) -> Option<String> {
            self.shared.then(|| "test-daemon".to_string())
        }
        fn username(&self) -> String {
            "tester".to_string()
        }
        fn listen_port(&self) -> Option<u16> {
            None
        }
        fn session_loss(&self) -> Option<soulseek_rs::SessionLoss> {
            None
        }
        fn search_with_cancel(
            &self,
            _query: &str,
            _timeout: Duration,
            _cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
        ) -> soulseek_rs::Result<Vec<soulseek_rs::SearchResult>> {
            Ok(Vec::new())
        }
        fn get_search_results(
            &self,
            _key: &str,
        ) -> Vec<soulseek_rs::SearchResult> {
            Vec::new()
        }
        fn all_searches(&self) -> Vec<crate::api::SessionSearch> {
            self.all_searches_calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.searches.lock().expect("not poisoned").clone()
        }
        fn forget_search(&self, query: &str) -> bool {
            let mut searches = self.searches.lock().expect("not poisoned");
            let before = searches.len();
            searches.retain(|search| search.query != query);
            searches.len() != before
        }
        fn get_search_results_count(&self, _key: &str) -> usize {
            0
        }
        fn try_get_search_results(
            &self,
            _key: &str,
        ) -> Option<Vec<soulseek_rs::SearchResult>> {
            self.result_fetches
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(self.results.lock().expect("not poisoned").clone())
        }
        fn start_wishlist_search(
            &self,
            _query: &str,
        ) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn wishlist_interval(&self) -> Duration {
            Duration::from_mins(12)
        }
        fn download(
            &self,
            _filename: String,
            _username: String,
            _size: u64,
            _directory: String,
        ) -> soulseek_rs::Result<(
            soulseek_rs::types::Download,
            std::sync::mpsc::Receiver<soulseek_rs::DownloadStatus>,
        )> {
            Err(soulseek_rs::SoulseekRs::NotConnected)
        }
        fn download_with_metadata(
            &self,
            _filename: String,
            _username: String,
            _size: u64,
            _directory: String,
            _metadata: soulseek_rs::types::DownloadMetadata,
        ) -> soulseek_rs::Result<(
            soulseek_rs::types::Download,
            std::sync::mpsc::Receiver<soulseek_rs::DownloadStatus>,
        )> {
            Err(soulseek_rs::SoulseekRs::NotConnected)
        }
        fn get_all_downloads(&self) -> Vec<soulseek_rs::types::Download> {
            self.downloads.clone()
        }
        fn pause_download(&self, _u: &str, _f: &str) -> bool {
            false
        }
        fn resume_download(&self, _u: &str, _f: &str) -> bool {
            false
        }
        fn remove_queued_download(&self, _u: &str, _f: &str) -> bool {
            false
        }
        fn remove_download(&self, _u: &str, _f: &str) -> bool {
            false
        }
        fn uploads(&self) -> Vec<soulseek_rs::UploadInfo> {
            Vec::new()
        }
        fn take_upload_events(&self) -> Vec<soulseek_rs::UploadInfo> {
            Vec::new()
        }
        fn cancel_upload(&self, u: &str, f: &str) -> bool {
            *self.upload_cancelled.lock().expect("not poisoned") =
                Some((u.to_string(), f.to_string()));
            true
        }
        fn cancel_download(&self, u: &str, f: &str) -> bool {
            *self.download_cancelled.lock().expect("not poisoned") =
                Some((u.to_string(), f.to_string()));
            true
        }
        fn set_upload_slots(&self, _slots: usize) {}
        fn check_privileges(&self) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn own_privilege_seconds(&self) -> Option<u32> {
            None
        }
        fn request_room_list(&self) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn join_room(&self, _room: &str) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn leave_room(&self, _room: &str) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn say_in_room(
            &self,
            _room: &str,
            _message: &str,
        ) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn room_members(&self, _room: &str) -> Vec<String> {
            Vec::new()
        }
        fn room_member_stats(
            &self,
            _room: &str,
        ) -> Vec<soulseek_rs::types::RoomUserStats> {
            Vec::new()
        }
        fn take_room_events(&self) -> Vec<soulseek_rs::RoomEvent> {
            Vec::new()
        }
        fn send_private_message(
            &self,
            _u: &str,
            _m: &str,
        ) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn take_private_messages(&self) -> Vec<soulseek_rs::UserMessage> {
            Vec::new()
        }
        fn browse_user(&self, _username: &str) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn take_browse_result(
            &self,
            _username: &str,
        ) -> Option<Vec<soulseek_rs::SharedDirectory>> {
            None
        }
        fn request_user_info(
            &self,
            _username: &str,
        ) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn watch_user(&self, _username: &str) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn unwatch_user(&self, _username: &str) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn watched_users(&self) -> Vec<String> {
            Vec::new()
        }
        fn user_info(&self, _username: &str) -> Option<soulseek_rs::UserInfo> {
            None
        }
        fn shared_counts(&self) -> (u32, u32) {
            (0, 0)
        }
        fn shared_directories(&self) -> Vec<String> {
            Vec::new()
        }
        fn set_shared_directories(
            &self,
            directories: Vec<String>,
        ) -> soulseek_rs::Result<()> {
            *self.shares_set.lock().expect("not poisoned") = Some(directories);
            Ok(())
        }
        fn set_download_directory(
            &self,
            directory: String,
        ) -> soulseek_rs::Result<()> {
            *self.download_dir_set.lock().expect("not poisoned") =
                Some(directory);
            Ok(())
        }
    }

    fn tui(history: Vec<ChatMessageDto>) -> MainTui {
        with_session(TalkativeSession {
            history,
            shared: true,
            ..TalkativeSession::default()
        })
    }

    fn shared(searches: Vec<crate::api::SessionSearch>) -> TalkativeSession {
        TalkativeSession {
            searches: std::sync::Mutex::new(searches),
            shared: true,
            ..TalkativeSession::default()
        }
    }

    fn search_of(
        query: &str,
        files: usize,
        age: u64,
    ) -> crate::api::SessionSearch {
        crate::api::SessionSearch {
            query: query.to_string(),
            files,
            started_secs_ago: Some(age),
        }
    }

    #[test]
    fn x_cancels_the_highlighted_download() {
        let session = Arc::new(TalkativeSession::default());
        let mut tui = attach(session.clone());
        tui.state.downloads.push(crate::models::DownloadEntry {
            download: at_status(
                "bob",
                "song.mp3",
                soulseek_rs::DownloadStatus::InProgress {
                    bytes_downloaded: 1,
                    total_bytes: 4096,
                    speed_bytes_per_sec: 1.0,
                },
            ),
            receiver: None,
        });
        tui.state.downloads_table_state.select(Some(0));
        tui.state.focused_pane = FocusedPane::Downloads;

        tui.handle_key_event(key('x'));

        assert_eq!(
            *session.download_cancelled.lock().expect("not poisoned"),
            Some(("bob".to_string(), "song.mp3".to_string()))
        );
        assert_eq!(
            *session.upload_cancelled.lock().expect("not poisoned"),
            None
        );
    }

    #[test]
    fn d_clears_a_cancelled_row() {
        let mut tui = with_session(TalkativeSession::default());
        tui.state.downloads.push(crate::models::DownloadEntry {
            download: at_status(
                "bob",
                "gone.mp3",
                soulseek_rs::DownloadStatus::Cancelled,
            ),
            receiver: None,
        });
        tui.state.downloads_table_state.select(Some(0));
        tui.state.focused_pane = FocusedPane::Downloads;

        tui.handle_key_event(key('d'));

        assert!(tui.state.downloads.is_empty(), "the row goes");
    }

    #[test]
    fn x_on_a_finished_download_asks_nothing() {
        let session = Arc::new(TalkativeSession::default());
        let mut tui = attach(session.clone());
        tui.state.downloads.push(crate::models::DownloadEntry {
            download: completed("bob", "done.mp3"),
            receiver: None,
        });
        tui.state.downloads_table_state.select(Some(0));
        tui.state.focused_pane = FocusedPane::Downloads;

        tui.handle_key_event(key('x'));

        assert_eq!(
            *session.download_cancelled.lock().expect("not poisoned"),
            None
        );
    }

    #[test]
    fn x_on_an_upload_row_still_cancels_the_upload() {
        let session = Arc::new(TalkativeSession::default());
        let mut tui = attach(session.clone());
        tui.state.downloads.push(crate::models::DownloadEntry {
            download: queued("bob", "song.mp3"),
            receiver: None,
        });
        tui.state.uploads.push(soulseek_rs::types::UploadInfo {
            username: "alice".to_string(),
            filename: "served.mp3".to_string(),
            size: 10,
            bytes_sent: 1,
            speed_bytes_per_sec: 1.0,
            status: soulseek_rs::types::UploadStatus::InProgress,
        });
        tui.state.downloads_table_state.select(Some(1));
        tui.state.focused_pane = FocusedPane::Downloads;

        tui.handle_key_event(key('x'));

        assert_eq!(
            *session.upload_cancelled.lock().expect("not poisoned"),
            Some(("alice".to_string(), "served.mp3".to_string()))
        );
        assert_eq!(
            *session.download_cancelled.lock().expect("not poisoned"),
            None
        );
    }

    fn with_session(session: TalkativeSession) -> MainTui {
        attach(Arc::new(session))
    }

    fn attach(session: Arc<TalkativeSession>) -> MainTui {
        MainTui::new(
            session,
            "/tmp".to_string(),
            Duration::from_secs(1),
            None,
            None,
        )
    }

    #[test]
    fn an_attached_session_opens_with_the_conversation_already_in_it() {
        // A TUI attached to a daemon has no state file of its own; the
        // conversation has to come from the session, or it starts blank while
        // the daemon has been collecting messages for hours.
        let tui = tui(vec![
            ChatMessageDto {
                peer: "bob".into(),
                outgoing: false,
                text: "hi".into(),
                at: 1,
            },
            ChatMessageDto {
                peer: "bob".into(),
                outgoing: true,
                text: "hello".into(),
                at: 2,
            },
        ]);

        assert_eq!(tui.state.messages.len(), 2);
        assert_eq!(tui.state.messages[0].peer, "bob");
        assert_eq!(tui.state.messages[0].direction, MessageDirection::Incoming);
        assert_eq!(
            tui.state.messages[1].direction,
            MessageDirection::Outgoing,
            "both halves of the conversation, not just what arrived"
        );
        assert_eq!(tui.state.chat_peers(), ["bob"]);
    }

    #[test]
    fn a_window_shows_transfers_it_did_not_start() {
        // Two windows on one daemon are two views of one queue. A transfer
        // queued in the other one has to appear here, or closing a window
        // would look like losing its downloads.
        let mut tui = with_session(TalkativeSession {
            downloads: vec![queued("bob", "@@x\\theirs.mp3")],
            shared: true,
            ..TalkativeSession::default()
        });
        assert!(
            tui.state.downloads.is_empty(),
            "nothing until the first poll"
        );

        tui.update_downloads();

        assert_eq!(tui.state.downloads.len(), 1);
        assert_eq!(tui.state.downloads[0].download.filename, "@@x\\theirs.mp3");
        assert!(
            tui.state.downloads[0].receiver.is_none(),
            "someone else's transfer has no channel of ours"
        );

        // Polling again must not double it up.
        tui.update_downloads();
        assert_eq!(tui.state.downloads.len(), 1);
    }

    #[test]
    fn a_transfer_the_session_forgets_leaves_every_window() {
        let mut tui = with_session(TalkativeSession {
            downloads: vec![queued("bob", "gone.mp3")],
            shared: true,
            ..TalkativeSession::default()
        });
        tui.update_downloads();
        assert_eq!(tui.state.downloads.len(), 1);

        // The session no longer has it — cancelled from the other window.
        tui.client = Arc::new(TalkativeSession {
            shared: true,
            ..TalkativeSession::default()
        });
        tui.update_downloads();
        assert!(
            tui.state.downloads.is_empty(),
            "a queue is shared in both directions"
        );
    }

    #[test]
    fn d_clears_a_finished_row_but_not_a_running_one() {
        // Finished transfers are what a list fills up with after a restart, so
        // `d` has to be able to dismiss one — restored or not. A running
        // transfer is not ours to discard, and stays put.
        let mut tui = with_session(TalkativeSession::default());
        tui.state.downloads.push(crate::models::DownloadEntry {
            download: completed("bob", "done.mp3"),
            receiver: None,
        });
        tui.state.downloads_table_state.select(Some(0));
        tui.state.focused_pane = FocusedPane::Downloads;

        tui.handle_key_event(key('d'));

        assert!(tui.state.downloads.is_empty(), "the row goes too");
        assert_eq!(tui.state.downloads_table_state.selected(), None);

        tui.state.downloads.push(crate::models::DownloadEntry {
            download: at_status(
                "bob",
                "still.mp3",
                soulseek_rs::DownloadStatus::InProgress {
                    bytes_downloaded: 1,
                    total_bytes: 2,
                    speed_bytes_per_sec: 1.0,
                },
            ),
            receiver: None,
        });
        tui.state.downloads_table_state.select(Some(0));

        tui.handle_key_event(key('d'));

        assert_eq!(tui.state.downloads.len(), 1, "it is not disposable");
    }

    #[test]
    fn a_window_shows_searches_run_in_another() {
        let mut tui =
            with_session(shared(vec![search_of("their query", 7, 9_999)]));
        tui.update_search_results();

        assert_eq!(tui.state.searches.len(), 1);
        assert_eq!(tui.state.searches[0].query, "their query");

        // And it is not added a second time on the next frame.
        tui.update_search_results();
        assert_eq!(tui.state.searches.len(), 1);
    }

    #[test]
    fn a_search_another_window_is_still_running_shows_as_active() {
        // It had said "Done" the moment it appeared, because an imported
        // search carried no notion of still collecting — so the other window
        // looked finished while results were still arriving.
        let mut tui =
            with_session(shared(vec![search_of("still going", 3, 0)]));
        tui.update_search_results();

        assert_eq!(tui.state.searches[0].status, SearchStatus::Active);
        assert_eq!(
            tui.state.searches[0].known_files, 3,
            "and its count comes across before the results do"
        );
    }

    #[test]
    fn a_search_that_has_run_its_window_shows_as_done() {
        let mut tui =
            with_session(shared(vec![search_of("finished", 12, 9_999)]));
        tui.update_search_results();
        assert_eq!(tui.state.searches[0].status, SearchStatus::Completed);
    }

    #[test]
    fn removing_a_search_removes_it_from_the_session() {
        // Removing it only from this window would last until the next sync,
        // which would put it straight back — and in a shared session it would
        // come back for every other window too.
        let mut tui =
            with_session(shared(vec![search_of("unwanted", 1, 9_999)]));
        tui.update_search_results();
        assert_eq!(tui.state.searches.len(), 1);

        tui.remove_search_at_index(0);
        tui.update_search_results();

        assert!(tui.state.searches.is_empty(), "and it stays gone");
    }

    #[test]
    fn clearing_every_search_clears_them_in_the_session_too() {
        let mut tui = with_session(shared(vec![
            search_of("one", 1, 9_999),
            search_of("two", 2, 9_999),
        ]));
        tui.update_search_results();
        assert_eq!(tui.state.searches.len(), 2);

        tui.clear_all_searches();
        tui.update_search_results();

        assert!(
            tui.state.searches.is_empty(),
            "clearing has to reach the session, or every row comes back"
        );
    }

    #[test]
    fn a_search_dismissed_in_another_window_leaves_this_one() {
        let session = shared(vec![search_of("theirs", 4, 9_999)]);
        let mut tui = with_session(session);
        tui.update_search_results();
        assert_eq!(tui.state.searches.len(), 1);

        // The other window dismissed it.
        tui.client.forget_search("theirs");
        tui.update_search_results();

        assert!(tui.state.searches.is_empty());
    }

    fn files_from_bob(names: &[&str]) -> soulseek_rs::SearchResult {
        soulseek_rs::SearchResult {
            token: 0,
            files: names
                .iter()
                .map(|name| soulseek_rs::types::File {
                    username: "bob".to_string(),
                    name: (*name).to_string(),
                    size: 42,
                    attribs: std::collections::HashMap::new(),
                })
                .collect(),
            slots: 1,
            speed: 0,
            username: "bob".to_string(),
        }
    }

    #[test]
    fn a_tick_asks_the_session_for_its_searches_once() {
        // Against a daemon every call is a round trip, and the daemon answers
        // it by walking its whole search cache — twice per frame was half the
        // cost of attaching at all.
        let session = Arc::new(shared(vec![search_of("q", 0, 9_999)]));
        let mut tui = attach(session.clone());

        tui.update_search_results();

        assert_eq!(
            session
                .all_searches_calls
                .load(std::sync::atomic::Ordering::Relaxed),
            1,
            "one sync means one listing, not one per use of it"
        );
    }

    /// An attached window that selected a two-file search and has already
    /// ticked past its one necessary fetch.
    fn window_that_pulled_two_files() -> (Arc<TalkativeSession>, MainTui) {
        let session = Arc::new(TalkativeSession {
            results: std::sync::Mutex::new(vec![files_from_bob(&[
                "@@x\\a.mp3",
                "@@x\\b.mp3",
            ])]),
            ..shared(vec![search_of("aphex twin", 2, 9_999)])
        });
        let mut tui = attach(session.clone());
        tui.state.selected_search_index = Some(0);
        tui.update_search_results();
        tui.update_search_results();
        (session, tui)
    }

    #[test]
    fn a_selected_search_with_nothing_new_is_not_pulled_across_again() {
        // The result set only accumulates, so an unchanged count means a
        // fetch would hand back what this window already holds — against a
        // daemon that is the entire result set over the socket, every frame,
        // forever.
        let (session, tui) = window_that_pulled_two_files();

        assert_eq!(tui.state.searches[0].results.len(), 2);
        assert_eq!(
            session
                .result_fetches
                .load(std::sync::atomic::Ordering::Relaxed),
            1,
            "the set crosses once; after that the unchanged count spares it"
        );
    }

    #[test]
    fn fresh_results_still_reach_a_window_that_skipped_a_fetch() {
        let (session, mut tui) = window_that_pulled_two_files();

        // A late responder lands in the session.
        *session.searches.lock().expect("not poisoned") =
            vec![search_of("aphex twin", 3, 9_999)];
        *session.results.lock().expect("not poisoned") =
            vec![files_from_bob(&["@@x\\a.mp3", "@@x\\b.mp3", "@@x\\c.mp3"])];
        tui.update_search_results();

        assert_eq!(
            tui.state.searches[0].results.len(),
            3,
            "a changed count must bring the new results across"
        );
    }

    #[test]
    fn a_local_window_fetches_results_without_counting_on_the_session() {
        // Locally nothing maintains known_files, so the count gate must stay
        // out of the way: a search whose count still reads zero has to get
        // its results anyway.
        let session = Arc::new(TalkativeSession {
            results: std::sync::Mutex::new(vec![files_from_bob(&[
                "@@x\\a.mp3",
                "@@x\\b.mp3",
            ])]),
            shared: false,
            ..TalkativeSession::default()
        });
        let mut tui = attach(session);
        tui.state.searches.push(crate::models::SearchEntry {
            query: "mine".to_string(),
            status: SearchStatus::Active,
            results: Vec::new(),
            known_files: 0,
            owned: true,
            start_time: std::time::Instant::now(),
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });

        tui.update_search_results();

        assert_eq!(
            tui.state.searches[0].results.len(),
            2,
            "a zero count matching zero held rows must not read as \
             nothing-to-fetch on a local session"
        );
    }

    #[test]
    fn a_window_with_its_own_session_is_left_alone() {
        // Nothing to share, so nothing is synced — and in particular the
        // completed transfers restored from disk, which the session has never
        // heard of, are not mistaken for cancelled and deleted.
        let mut tui = with_session(TalkativeSession {
            downloads: Vec::new(),
            shared: false,
            ..TalkativeSession::default()
        });
        tui.state.downloads.push(crate::models::DownloadEntry {
            download: queued("bob", "from-disk.mp3"),
            receiver: None,
        });

        tui.update_downloads();

        assert_eq!(
            tui.state.downloads.len(),
            1,
            "a local window's history must survive the tick"
        );
    }

    #[test]
    fn a_local_session_starts_from_its_own_snapshot_instead() {
        // Locally the session knows no history and the state file is the whole
        // record, so nothing is invented.
        assert!(tui(Vec::new()).state.messages.is_empty());
    }

    /// Attached, both folders in the settings form name paths on the
    /// *daemon's* machine: they go over the socket exactly as typed, and
    /// nothing is created or validated against this machine's filesystem.
    #[test]
    fn applying_settings_attached_pushes_both_folders_to_the_daemon() {
        let session = Arc::new(TalkativeSession {
            shared: true,
            ..TalkativeSession::default()
        });
        let scratch = tempfile::tempdir().expect("a temp dir");
        let config_path = scratch.path().join("config.toml");
        let mut tui = MainTui::new(
            session.clone(),
            "/daemon/old".to_string(),
            Duration::from_secs(1),
            None,
            Some(config_path.clone()),
        );

        let daemon_only = scratch.path().join("only-on-the-daemon");
        let daemon_dir = daemon_only.to_str().expect("utf-8").to_string();
        tui.open_settings();
        {
            let settings = tui.state.settings.as_mut().expect("open");
            settings.download_dir.clone_from(&daemon_dir);
            settings.share_dirs = vec!["/not-here-either".to_string()];
        }
        tui.apply_settings();

        assert_eq!(
            *session.download_dir_set.lock().expect("not poisoned"),
            Some(daemon_dir.clone()),
            "the folder is pushed to the daemon"
        );
        assert_eq!(
            *session.shares_set.lock().expect("not poisoned"),
            Some(vec!["/not-here-either".to_string()]),
            "shares go over as typed — the daemon's filesystem judges them"
        );
        assert!(
            !daemon_only.exists(),
            "no directory is created on this machine for a daemon's path"
        );
        assert_eq!(tui.download_dir, daemon_dir);

        let saved = crate::persist::config::FileConfig::load(&config_path)
            .expect("config readable");
        assert_eq!(
            saved.download_dir.as_deref(),
            Some(daemon_dir.as_str()),
            "the change still lands in config.toml for the next restart"
        );
    }

    /// Locally the same form works on this machine: the folder is created
    /// here and share paths that do not exist here are dropped before the
    /// session hears about them.
    #[test]
    fn applying_settings_locally_stays_on_this_machine() {
        let session = Arc::new(TalkativeSession {
            shared: false,
            ..TalkativeSession::default()
        });
        let scratch = tempfile::tempdir().expect("a temp dir");
        let config_path = scratch.path().join("config.toml");
        let share = scratch.path().join("music");
        std::fs::create_dir_all(&share).expect("share dir");
        let mut tui = MainTui::new(
            session.clone(),
            "/tmp".to_string(),
            Duration::from_secs(1),
            None,
            Some(config_path),
        );

        let downloads = scratch.path().join("downloads");
        let downloads_dir = downloads.to_str().expect("utf-8").to_string();
        tui.open_settings();
        {
            let settings = tui.state.settings.as_mut().expect("open");
            settings.download_dir.clone_from(&downloads_dir);
            settings.share_dirs = vec![
                share.to_str().expect("utf-8").to_string(),
                "/definitely-not-a-real-share".to_string(),
            ];
        }
        tui.apply_settings();

        // The seam is told in both modes; a local session accepts and takes
        // the directory per download instead of storing it.
        assert_eq!(
            *session.download_dir_set.lock().expect("not poisoned"),
            Some(downloads_dir.clone())
        );
        assert_eq!(
            *session.shares_set.lock().expect("not poisoned"),
            Some(vec![share.to_str().expect("utf-8").to_string()]),
            "paths missing on this machine are dropped before applying"
        );
        assert!(downloads.is_dir(), "the folder is created here");
        assert_eq!(tui.download_dir, downloads_dir);
    }

    fn results(n: usize) -> Vec<crate::models::FileDisplayData> {
        (0..n)
            .map(|i| crate::models::FileDisplayData {
                filename: format!("@@x\\Music\\{i:02}.mp3"),
                username: "bob".to_string(),
                ..Default::default()
            })
            .collect()
    }

    fn results_tui(n: usize) -> MainTui {
        let mut tui = with_session(TalkativeSession::default());
        tui.state.results_items = results(n);
        tui.state.focused_pane = FocusedPane::Results;
        tui.state.results_pane_area =
            Some(ratatui::layout::Rect::new(0, 0, 80, 13));
        tui
    }

    fn press(tui: &mut MainTui, code: KeyCode) {
        tui.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn ctrl(tui: &mut MainTui, ch: char) {
        tui.handle_key_event(KeyEvent::new(
            KeyCode::Char(ch),
            KeyModifiers::CONTROL,
        ));
    }

    fn selected(tui: &MainTui) -> Option<usize> {
        tui.state.results_table_state.selected()
    }

    #[test]
    fn end_and_home_jump_to_the_last_and_first_result() {
        let mut tui = results_tui(30);
        press(&mut tui, KeyCode::End);
        assert_eq!(selected(&tui), Some(29));
        press(&mut tui, KeyCode::Home);
        assert_eq!(selected(&tui), Some(0));
        press(&mut tui, KeyCode::Char('G'));
        assert_eq!(selected(&tui), Some(29));
        press(&mut tui, KeyCode::Char('g'));
        assert_eq!(selected(&tui), Some(0));
    }

    #[test]
    fn page_keys_move_a_pane_of_rows_and_stop_at_the_edges() {
        let mut tui = results_tui(30);
        press(&mut tui, KeyCode::PageDown);
        assert_eq!(selected(&tui), Some(10));
        ctrl(&mut tui, 'f');
        assert_eq!(selected(&tui), Some(20));
        press(&mut tui, KeyCode::PageDown);
        assert_eq!(selected(&tui), Some(29), "clamps instead of wrapping");
        press(&mut tui, KeyCode::PageUp);
        assert_eq!(selected(&tui), Some(19));
        ctrl(&mut tui, 'b');
        assert_eq!(selected(&tui), Some(9));
        press(&mut tui, KeyCode::PageUp);
        assert_eq!(selected(&tui), Some(0));
        assert!(
            !tui.state.show_browse && !tui.state.command_bar_active,
            "ctrl-b pages up instead of opening browse"
        );
    }

    #[test]
    fn ctrl_d_and_ctrl_u_move_half_a_pane() {
        let mut tui = results_tui(30);
        ctrl(&mut tui, 'd');
        assert_eq!(selected(&tui), Some(5));
        ctrl(&mut tui, 'u');
        assert_eq!(selected(&tui), Some(0));
    }

    #[test]
    fn navigation_keys_do_nothing_without_results() {
        let mut tui = results_tui(0);
        tui.state.results_table_state.select(None);
        press(&mut tui, KeyCode::End);
        press(&mut tui, KeyCode::PageDown);
        ctrl(&mut tui, 'd');
        assert_eq!(selected(&tui), None);
    }

    #[test]
    fn l_and_h_scroll_the_highlighted_name_up_to_its_end() {
        let mut tui = results_tui(3);
        tui.state.results_items[1].filename = "x".repeat(40);
        tui.state.results_table_state.select(Some(1));
        press(&mut tui, KeyCode::Char('l'));
        assert_eq!(tui.state.results_name_offset, 8);
        press(&mut tui, KeyCode::Right);
        assert_eq!(tui.state.results_name_offset, 16);
        press(&mut tui, KeyCode::Char('$'));
        assert_eq!(tui.state.results_name_offset, 30);
        press(&mut tui, KeyCode::Char('l'));
        assert_eq!(tui.state.results_name_offset, 30, "stops at the end");
        press(&mut tui, KeyCode::Char('h'));
        assert_eq!(tui.state.results_name_offset, 22);
        press(&mut tui, KeyCode::Left);
        assert_eq!(tui.state.results_name_offset, 14);
        press(&mut tui, KeyCode::Char('0'));
        assert_eq!(tui.state.results_name_offset, 0);
        press(&mut tui, KeyCode::Char('h'));
        assert_eq!(tui.state.results_name_offset, 0);
    }

    #[test]
    fn scrolling_follows_the_highlighted_row_not_the_longest_one() {
        let mut tui = results_tui(3);
        tui.state.results_items[0].filename = "short.mp3".to_string();
        tui.state.results_items[1].filename = "x".repeat(40);
        tui.state.results_items[2].filename = "short.mp3".to_string();
        press(&mut tui, KeyCode::Char('$'));
        assert_eq!(tui.state.results_name_offset, 0, "row 0 fits already");
        tui.state.results_table_state.select(Some(1));
        press(&mut tui, KeyCode::Char('$'));
        assert_eq!(tui.state.results_name_offset, 30);
        tui.state.results_table_state.select(Some(2));
        press(&mut tui, KeyCode::Char('h'));
        assert_eq!(
            tui.state.results_name_offset, 0,
            "a step left lands within the row now highlighted"
        );
    }

    #[test]
    fn picking_another_search_starts_its_results_unscrolled() {
        let mut tui = results_tui(3);
        tui.state.results_items[1].filename = "x".repeat(40);
        tui.state.results_table_state.select(Some(1));
        press(&mut tui, KeyCode::Char('$'));
        assert_eq!(tui.state.results_name_offset, 30);

        tui.state.searches.push(crate::models::SearchEntry {
            query: "other".to_string(),
            status: SearchStatus::Active,
            results: results(2),
            known_files: 2,
            owned: true,
            start_time: std::time::Instant::now(),
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        tui.state.searches_table_state.select(Some(0));
        tui.state.focused_pane = FocusedPane::Searches;
        press(&mut tui, KeyCode::Enter);
        assert_eq!(tui.state.results_name_offset, 0);
    }

    #[test]
    fn ctrl_keys_are_ignored_outside_the_results_pane() {
        let mut tui = results_tui(3);
        tui.state.searches.push(crate::models::SearchEntry {
            query: "keep me".to_string(),
            status: SearchStatus::Active,
            results: Vec::new(),
            known_files: 0,
            owned: true,
            start_time: std::time::Instant::now(),
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        tui.state.searches_table_state.select(Some(0));
        tui.state.focused_pane = FocusedPane::Searches;
        ctrl(&mut tui, 'd');
        assert_eq!(tui.state.searches.len(), 1, "ctrl-d is not d");
        ctrl(&mut tui, 'b');
        assert!(!tui.state.command_bar_active, "ctrl-b is not b");
        ctrl(&mut tui, 'q');
        assert!(!tui.state.should_exit, "ctrl-q is not q");
    }

    #[test]
    fn ctrl_keys_page_while_a_filter_is_being_typed() {
        let mut tui = results_tui(30);
        press(&mut tui, KeyCode::Char('/'));
        press(&mut tui, KeyCode::Char('m'));
        ctrl(&mut tui, 'd');
        assert_eq!(selected(&tui), Some(5));
        assert_eq!(tui.state.results_filter_query, "m", "no d appended");
        press(&mut tui, KeyCode::End);
        assert_eq!(selected(&tui), Some(29));
    }

    #[test]
    fn space_after_a_filter_that_matches_nothing_does_not_panic() {
        let mut tui = results_tui(3);
        press(&mut tui, KeyCode::Char('/'));
        press(&mut tui, KeyCode::Char('z'));
        press(&mut tui, KeyCode::Enter);
        assert!(tui.state.results_filtered_items.is_empty());
        press(&mut tui, KeyCode::Char(' '));
        assert!(tui.state.results_selected_indices.is_empty());
    }

    fn screen_sized(tui: &mut MainTui, width: u16, height: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("backend");
        terminal.draw(|frame| tui.render(frame)).expect("draw");
        terminal.backend().to_string()
    }

    fn screen_of(tui: &mut MainTui) -> String {
        screen_sized(tui, 160, 40)
    }

    /// The screen row carrying a pane's legend, `[2] Results` say, without
    /// the quotes the test backend wraps each row in.
    fn row_with<'a>(screen: &'a str, needle: &str) -> &'a str {
        let line = screen
            .lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no row with {needle:?} in\n{screen}"));
        let start = line.find('"').map_or(0, |i| i + 1);
        let end = line.rfind('"').unwrap_or(line.len());
        &line[start..end]
    }

    /// A window with a search, its results, and a transfer, so every pane
    /// has something to show.
    fn furnished_tui() -> MainTui {
        let mut tui = results_tui(3);
        tui.state.searches.push(crate::models::SearchEntry {
            query: "aphex twin".to_string(),
            status: SearchStatus::Completed,
            results: results(3),
            known_files: 3,
            owned: true,
            start_time: std::time::Instant::now(),
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        tui.state.selected_search_index = Some(0);
        tui.state.downloads.push(crate::models::DownloadEntry {
            download: queued("bob", "@@x\\Music\\00.mp3"),
            receiver: None,
        });
        tui
    }

    #[test]
    fn the_results_pane_has_the_whole_width_to_itself() {
        let mut tui = furnished_tui();
        let screen = screen_of(&mut tui);

        let results = row_with(&screen, "[2] Results");
        assert!(results.starts_with('╭'), "{results}");
        assert!(results.trim_end().ends_with('╮'), "{results}");
        assert!(!results.contains("[1]"), "nothing beside it: {results}");

        // The other three share the row underneath, in this order.
        let row = row_with(&screen, "[1] Searches");
        let searches = row.find("[1]").expect("searches");
        let downloads = row.find("[3]").expect("downloads");
        let info = row.find("[Info]").expect("info");
        assert!(searches < downloads && downloads < info, "{row}");
    }

    #[test]
    fn tab_and_shift_tab_walk_the_panes_in_legend_order() {
        let mut tui = furnished_tui();
        tui.state.focused_pane = FocusedPane::Searches;
        press(&mut tui, KeyCode::Tab);
        assert_eq!(tui.state.focused_pane, FocusedPane::Results);
        press(&mut tui, KeyCode::Tab);
        assert_eq!(tui.state.focused_pane, FocusedPane::Downloads);
        press(&mut tui, KeyCode::Tab);
        assert_eq!(tui.state.focused_pane, FocusedPane::Searches);
        press(&mut tui, KeyCode::BackTab);
        assert_eq!(tui.state.focused_pane, FocusedPane::Downloads);
    }

    #[test]
    fn w_hides_the_focused_pane_and_its_number_brings_it_back() {
        let mut tui = furnished_tui();
        tui.state.focused_pane = FocusedPane::Searches;

        press(&mut tui, KeyCode::Char('w'));

        let screen = screen_of(&mut tui);
        assert!(!screen.contains("[1] Searches"), "{screen}");
        assert!(screen.contains("[2] Results"), "{screen}");
        assert!(screen.contains("[3] Downloads"), "{screen}");
        assert_eq!(tui.state.focused_pane, FocusedPane::Results);
        assert_eq!(tui.state.searches_pane_area, None, "nothing to click");
        // The transfers widen into the space.
        let row = row_with(&screen, "[3] Downloads");
        assert!(row.starts_with('╭'), "{row}");

        press(&mut tui, KeyCode::Char('1'));

        let screen = screen_of(&mut tui);
        assert!(screen.contains("[1] Searches"), "{screen}");
        assert_eq!(tui.state.focused_pane, FocusedPane::Searches);
    }

    #[test]
    fn hiding_the_results_gives_the_row_the_whole_height() {
        let mut tui = furnished_tui();
        press(&mut tui, KeyCode::Char('w'));
        let screen = screen_of(&mut tui);
        assert!(!screen.contains("[2] Results"), "{screen}");
        // The row now starts right under the status bar.
        let status = screen
            .lines()
            .position(|line| line.contains("Status"))
            .expect("status bar");
        let row = screen
            .lines()
            .position(|line| line.contains("[1] Searches"))
            .expect("searches");
        assert_eq!(row, status + 3, "{screen}");
    }

    #[test]
    fn the_last_pane_standing_cannot_be_hidden() {
        // Results goes, then Downloads; Searches is what is left, and a
        // third press leaves it there.
        let mut tui = furnished_tui();
        press(&mut tui, KeyCode::Char('w'));
        press(&mut tui, KeyCode::Char('w'));
        press(&mut tui, KeyCode::Char('w'));
        let screen = screen_of(&mut tui);
        assert!(screen.contains("[1] Searches"), "{screen}");
        assert!(!screen.contains("[3] Downloads"), "{screen}");
        assert_eq!(tui.state.focused_pane, FocusedPane::Searches);
    }

    #[test]
    fn z_zooms_the_focused_pane_and_esc_or_z_leaves_it() {
        let mut tui = furnished_tui();
        press(&mut tui, KeyCode::Char('z'));

        let screen = screen_of(&mut tui);
        assert!(screen.contains("[2] Results"), "{screen}");
        assert!(!screen.contains("[1] Searches"), "{screen}");
        assert!(!screen.contains("[3] Downloads"), "{screen}");
        assert!(!screen.contains("[Info]"), "{screen}");
        assert!(screen.contains("[z → unzoom]"), "{screen}");

        // Tab while zoomed switches which pane fills the window.
        press(&mut tui, KeyCode::Tab);
        let screen = screen_of(&mut tui);
        assert!(screen.contains("[3] Downloads"), "{screen}");
        assert!(!screen.contains("[2] Results"), "{screen}");

        press(&mut tui, KeyCode::Esc);
        assert!(!tui.state.layout.zoomed);
        let screen = screen_of(&mut tui);
        assert!(screen.contains("[2] Results"), "{screen}");
        assert!(screen.contains("[Info]"), "{screen}");

        press(&mut tui, KeyCode::Char('z'));
        press(&mut tui, KeyCode::Char('z'));
        assert!(!tui.state.layout.zoomed, "z toggles");
    }

    #[test]
    fn a_search_brings_a_hidden_results_pane_back() {
        let mut tui = furnished_tui();
        press(&mut tui, KeyCode::Char('w'));
        assert!(!tui.state.layout.is_visible(FocusedPane::Results));

        press(&mut tui, KeyCode::Char('s'));
        press(&mut tui, KeyCode::Char('x'));
        press(&mut tui, KeyCode::Enter);

        assert!(tui.state.layout.is_visible(FocusedPane::Results));
        assert_eq!(tui.state.focused_pane, FocusedPane::Results);
    }

    #[test]
    fn question_mark_opens_the_keys_list_and_closes_it_again() {
        let mut tui = furnished_tui();
        press(&mut tui, KeyCode::Char('?'));

        let screen = screen_of(&mut tui);
        assert!(screen.contains("Keys"), "{screen}");
        assert!(screen.contains("Tab / Shift-Tab"), "{screen}");
        assert!(screen.contains("clear every finished one"), "{screen}");

        // Other keys are swallowed while it is open.
        press(&mut tui, KeyCode::Char('q'));
        assert!(!tui.state.should_exit, "q closes the list, not the app");
        assert!(!tui.state.show_help);

        press(&mut tui, KeyCode::Char('?'));
        press(&mut tui, KeyCode::Char('w'));
        assert!(tui.state.show_help, "w did not reach the panes");
        assert!(tui.state.layout.is_visible(FocusedPane::Results));
        press(&mut tui, KeyCode::Esc);
        assert!(!tui.state.show_help);
    }

    #[test]
    fn on_a_small_terminal_the_keys_list_stacks_and_scrolls() {
        let mut tui = furnished_tui();
        press(&mut tui, KeyCode::Char('?'));

        let screen = screen_sized(&mut tui, 80, 24);
        let panes = row_with(&screen, "Panes");
        assert!(!panes.contains("Searches"), "one column: {panes}");
        assert!(screen.contains("focus a pane, hidden or not"), "{screen}");
        assert!(!screen.contains("next room, chat or user"), "{screen}");

        press(&mut tui, KeyCode::End);
        let screen = screen_sized(&mut tui, 80, 24);
        assert!(screen.contains("next room, chat or user"), "{screen}");
        assert!(!screen.contains("next / previous pane"), "{screen}");

        press(&mut tui, KeyCode::Home);
        let screen = screen_sized(&mut tui, 80, 24);
        assert!(screen.contains("Tab / Shift-Tab"), "{screen}");

        // A wide window shows both columns at once, nothing to scroll.
        press(&mut tui, KeyCode::End);
        let screen = screen_of(&mut tui);
        let panes = row_with(&screen, "Panes");
        assert!(panes.contains("Searches"), "two columns: {panes}");
        assert!(screen.contains("Tab / Shift-Tab"), "{screen}");
    }

    #[test]
    fn a_click_where_a_pane_just_hid_does_not_focus_it() {
        let mut tui = furnished_tui();
        let _ = screen_of(&mut tui);
        let searches = tui.state.searches_pane_area.expect("laid out");
        // Hidden, but not yet redrawn: the old area is still on record.
        tui.state.focused_pane = FocusedPane::Searches;
        press(&mut tui, KeyCode::Char('w'));
        assert_eq!(tui.state.focused_pane, FocusedPane::Results);
        tui.handle_mouse_event(ratatui::crossterm::event::MouseEvent {
            kind: ratatui::crossterm::event::MouseEventKind::Down(
                ratatui::crossterm::event::MouseButton::Left,
            ),
            column: searches.x + 1,
            row: searches.y + 1,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(tui.state.focused_pane, FocusedPane::Results);
    }

    #[test]
    fn l_and_h_scroll_a_transfer_name_and_a_query_too() {
        let mut tui = furnished_tui();
        tui.state.downloads[0].download.filename = "y".repeat(120);
        tui.state.searches[0].query = "z".repeat(60);
        let _ = screen_of(&mut tui);

        tui.state.focused_pane = FocusedPane::Downloads;
        press(&mut tui, KeyCode::Char('l'));
        assert_eq!(tui.state.downloads_name_offset, 8);
        press(&mut tui, KeyCode::Char('$'));
        let end = tui.state.downloads_name_offset;
        assert!(end > 8, "the end is past a step: {end}");
        let screen = screen_of(&mut tui);
        assert!(row_with(&screen, "…yyyy").contains("…yyyy"), "{screen}");
        press(&mut tui, KeyCode::Char('0'));
        assert_eq!(tui.state.downloads_name_offset, 0);

        tui.state.focused_pane = FocusedPane::Searches;
        press(&mut tui, KeyCode::Right);
        assert_eq!(tui.state.searches_query_offset, 8);
        press(&mut tui, KeyCode::End);
        assert_eq!(
            tui.state.searches_table_state.selected(),
            Some(0),
            "End is still a row key, $ is the name key"
        );
        press(&mut tui, KeyCode::Char('$'));
        assert!(tui.state.searches_query_offset > 8);
        let screen = screen_of(&mut tui);
        assert!(screen.contains("…zzzz"), "{screen}");
        assert!(
            tui.state.results_name_offset == 0,
            "each list scrolls on its own"
        );
    }

    fn browsed(tui: &mut MainTui, files: usize) {
        tui.state.browse.open("bob");
        let listing = vec![soulseek_rs::SharedDirectory {
            name: "Music".to_string(),
            files: (0..files).map(|i| (format!("{i:02}.mp3"), 1)).collect(),
        }];
        tui.state
            .browse
            .active_tab_mut()
            .expect("tab")
            .load(&listing);
        tui.state.show_browse = true;
        let _ = screen_of(tui);
    }

    #[test]
    fn page_keys_walk_the_browse_tree_and_ctrl_d_does_not_download() {
        let session = Arc::new(TalkativeSession::default());
        let mut tui = attach(session);
        browsed(&mut tui, 40);
        let rows = tui.state.browse.active_tab().expect("tab").rows().len();
        assert_eq!(rows, 41, "the folder and its files");

        press(&mut tui, KeyCode::End);
        let selected = |tui: &MainTui| {
            tui.state.browse.active_tab().expect("tab").selected_row
        };
        assert_eq!(selected(&tui), 40);
        press(&mut tui, KeyCode::Home);
        assert_eq!(selected(&tui), 0);
        press(&mut tui, KeyCode::PageDown);
        let page = selected(&tui);
        assert!(page > 1 && page < 40, "a page of rows: {page}");
        ctrl(&mut tui, 'd');
        assert!(selected(&tui) > page, "ctrl-d pages");
        assert!(
            tui.state.downloads.is_empty()
                && tui.state.downloads_receiver_channel.is_none(),
            "and does not download the folder"
        );
        press(&mut tui, KeyCode::Char('G'));
        assert_eq!(selected(&tui), 40);
        assert!(tui.state.show_browse, "still open");
    }

    fn in_a_room(tui: &mut MainTui, messages: usize) {
        tui.state.rooms.apply_event(
            soulseek_rs::RoomEvent::Joined {
                room: "jazz".to_string(),
                users: (0..30).map(|i| format!("user{i:02}")).collect(),
            },
            None,
        );
        tui.state.rooms.focus_or_open("jazz");
        for i in 0..messages {
            tui.state.rooms.apply_event(
                soulseek_rs::RoomEvent::Message {
                    room: "jazz".to_string(),
                    username: "alice".to_string(),
                    message: format!("line {i:03}"),
                },
                Some("jazz"),
            );
        }
        tui.state.show_rooms = true;
        let _ = screen_of(tui);
    }

    #[test]
    fn page_up_scrolls_a_room_log_back_and_end_returns_to_the_newest() {
        let mut tui = with_session(TalkativeSession::default());
        in_a_room(&mut tui, 120);

        let screen = screen_of(&mut tui);
        assert!(screen.contains("line 119"), "tails by default: {screen}");
        assert!(!screen.contains("line 000"), "{screen}");

        press(&mut tui, KeyCode::PageUp);
        let screen = screen_of(&mut tui);
        assert!(!screen.contains("line 119"), "scrolled back: {screen}");

        press(&mut tui, KeyCode::Home);
        let screen = screen_of(&mut tui);
        assert!(screen.contains("line 000"), "the oldest: {screen}");
        press(&mut tui, KeyCode::PageUp);
        let screen = screen_of(&mut tui);
        assert!(screen.contains("line 000"), "clamped at the top: {screen}");

        press(&mut tui, KeyCode::End);
        let screen = screen_of(&mut tui);
        assert!(screen.contains("line 119"), "{screen}");

        // The member list keeps its own keys, and ctrl-x is not x.
        press(&mut tui, KeyCode::Down);
        assert_eq!(tui.state.rooms.user_selected, 1);
        ctrl(&mut tui, 'x');
        assert_eq!(tui.state.rooms.open.len(), 1, "still in the room");
    }

    #[test]
    fn page_keys_move_through_the_room_list() {
        let mut tui = with_session(TalkativeSession::default());
        tui.state.rooms.apply_event(
            soulseek_rs::RoomEvent::List(
                (0..60)
                    .map(|i| soulseek_rs::types::RoomInfo {
                        name: format!("room{i:02}"),
                        user_count: 60 - i,
                    })
                    .collect(),
            ),
            None,
        );
        tui.state.show_rooms = true;
        let _ = screen_of(&mut tui);

        press(&mut tui, KeyCode::End);
        assert_eq!(tui.state.rooms.list_selected, 59);
        press(&mut tui, KeyCode::PageUp);
        assert!(tui.state.rooms.list_selected < 59);
        press(&mut tui, KeyCode::Char('g'));
        assert_eq!(tui.state.rooms.list_selected, 0);
        ctrl(&mut tui, 'f');
        assert!(tui.state.rooms.list_selected > 1);
    }

    #[test]
    fn page_up_scrolls_a_conversation_and_switching_chats_resets_it() {
        let mut tui = tui((0..80)
            .map(|i| ChatMessageDto {
                peer: "bob".into(),
                outgoing: i % 2 == 0,
                text: format!("msg {i:03}"),
                at: i,
            })
            .chain(std::iter::once(ChatMessageDto {
                peer: "carol".into(),
                outgoing: false,
                text: "hello".into(),
                at: 100,
            }))
            .collect());
        tui.state.chat_peer = Some("bob".to_string());
        tui.state.show_messages = true;
        let screen = screen_of(&mut tui);
        assert!(screen.contains("msg 079"), "{screen}");

        press(&mut tui, KeyCode::PageUp);
        let screen = screen_of(&mut tui);
        assert!(!screen.contains("msg 079"), "scrolled back: {screen}");
        press(&mut tui, KeyCode::Char('g'));
        let screen = screen_of(&mut tui);
        assert!(screen.contains("msg 000"), "{screen}");

        press(&mut tui, KeyCode::Tab);
        assert_eq!(tui.state.active_chat_peer(), Some("carol"));
        assert!(
            tui.state.chat_view.following(),
            "a fresh chat starts at its end"
        );
        press(&mut tui, KeyCode::BackTab);
        let screen = screen_of(&mut tui);
        assert!(screen.contains("msg 079"), "{screen}");
    }

    #[test]
    fn the_shortcut_bar_wraps_instead_of_cutting_keys_off() {
        let mut tui = furnished_tui();
        let screen = screen_of(&mut tui);
        assert!(screen.contains("[q → quit]"), "{screen}");
        let bar_rows =
            screen.lines().filter(|line| line.contains(" → ")).count();
        assert_eq!(bar_rows, 2, "{screen}");

        let wide = screen_sized(&mut tui, 300, 40);
        let bar_rows = wide.lines().filter(|line| line.contains(" → ")).count();
        assert_eq!(bar_rows, 1, "{wide}");
    }

    #[test]
    fn page_keys_work_in_the_transfers_and_searches_lists_too() {
        let mut tui = furnished_tui();
        for i in 0..30 {
            tui.state.downloads.push(crate::models::DownloadEntry {
                download: queued("bob", &format!("{i}.mp3")),
                receiver: None,
            });
        }
        tui.state.focused_pane = FocusedPane::Downloads;
        tui.state.downloads_table_state.select(Some(0));
        let _ = screen_of(&mut tui); // lays the panes out, which sets the page size

        press(&mut tui, KeyCode::End);
        assert_eq!(tui.state.downloads_table_state.selected(), Some(30));
        press(&mut tui, KeyCode::Home);
        assert_eq!(tui.state.downloads_table_state.selected(), Some(0));
        press(&mut tui, KeyCode::PageDown);
        let page = tui.state.downloads_table_state.selected().expect("row");
        assert!(page > 1 && page < 30, "a pane of rows, not one: {page}");
        ctrl(&mut tui, 'd');
        assert_eq!(tui.state.downloads.len(), 31, "ctrl-d pages, d deletes");
        assert!(tui.state.downloads_table_state.selected() > Some(page));

        tui.state.focused_pane = FocusedPane::Searches;
        press(&mut tui, KeyCode::Char('G'));
        assert_eq!(tui.state.searches_table_state.selected(), Some(0));
        press(&mut tui, KeyCode::Char('g'));
        assert_eq!(tui.state.searches_table_state.selected(), Some(0));
        assert_eq!(tui.state.searches.len(), 1, "g and G only move");
    }

    #[test]
    fn clicking_where_a_hidden_pane_was_focuses_whatever_is_there_now() {
        let mut tui = furnished_tui();
        let _ = screen_of(&mut tui);
        let searches = tui.state.searches_pane_area.expect("laid out");
        let inside = ratatui::crossterm::event::MouseEvent {
            kind: ratatui::crossterm::event::MouseEventKind::Down(
                ratatui::crossterm::event::MouseButton::Left,
            ),
            column: searches.x + 1,
            row: searches.y + 1,
            modifiers: KeyModifiers::NONE,
        };
        tui.handle_mouse_event(inside);
        assert_eq!(tui.state.focused_pane, FocusedPane::Searches);

        press(&mut tui, KeyCode::Char('w'));
        let _ = screen_of(&mut tui);
        tui.handle_mouse_event(inside);
        assert_eq!(tui.state.focused_pane, FocusedPane::Downloads);
    }

    #[test]
    fn the_info_pane_describes_the_highlighted_result() {
        let mut tui = results_tui(3);
        tui.state.results_items[1].filename =
            "@@x\\Music\\Album\\Second Track.mp3".to_string();
        tui.state.results_items[1].bitrate = Some(320);
        tui.state.results_table_state.select(Some(1));

        let screen = screen_of(&mut tui);
        assert!(screen.contains("@@x/Music/Album"), "{screen}");
        assert!(screen.contains("Bitrate"), "{screen}");

        tui.state.focused_pane = FocusedPane::Downloads;
        let screen = screen_of(&mut tui);
        assert!(!screen.contains("@@x/Music/Album"), "{screen}");
    }
}
