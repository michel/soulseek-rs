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
    #[allow(dead_code)]
    max_concurrent_downloads: usize,
    search_timeout: Duration,
    spinner_state: usize,
    store: Option<StateStore>,
    /// Last snapshot written to disk, to skip no-op saves.
    saved_snapshot: Snapshot,
}

impl MainTui {
    pub fn new(
        client: Arc<dyn SessionApi>,
        download_dir: String,
        max_concurrent_downloads: usize,
        search_timeout: Duration,
        store: Option<StateStore>,
    ) -> Self {
        let mut tui = Self {
            client,
            state: AppState::new(),
            download_dir,
            max_concurrent_downloads,
            search_timeout,
            spinner_state: 0,
            store,
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
    max_concurrent_downloads: usize,
    search_timeout: Duration,
    store: Option<StateStore>,
) -> Result<()> {
    let tui = MainTui::new(
        client,
        download_dir,
        max_concurrent_downloads,
        search_timeout,
        store,
    );
    tui.run(terminal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::proto::ChatMessageDto;
    use crate::models::MessageDirection;

    /// A session that knows a conversation and nothing else — what attaching
    /// to a daemon looks like from the TUI's side.
    struct TalkativeSession(Vec<ChatMessageDto>);

    impl SessionApi for TalkativeSession {
        fn message_history(&self) -> Vec<ChatMessageDto> {
            self.0.clone()
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
        fn get_search_results_count(&self, _key: &str) -> usize {
            0
        }
        fn try_get_search_results(
            &self,
            _key: &str,
        ) -> Option<Vec<soulseek_rs::SearchResult>> {
            None
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
            Vec::new()
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
        fn cancel_upload(&self, _u: &str, _f: &str) -> bool {
            false
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
            _directories: Vec<String>,
        ) -> soulseek_rs::Result<()> {
            Ok(())
        }
    }

    fn tui(history: Vec<ChatMessageDto>) -> MainTui {
        MainTui::new(
            Arc::new(TalkativeSession(history)),
            "/tmp".to_string(),
            1,
            Duration::from_secs(1),
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
    fn a_local_session_starts_from_its_own_snapshot_instead() {
        // Locally the session knows no history and the state file is the whole
        // record, so nothing is invented.
        assert!(tui(Vec::new()).state.messages.is_empty());
    }
}
