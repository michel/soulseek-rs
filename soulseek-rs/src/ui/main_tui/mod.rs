mod browse;
mod downloads;
mod input;
mod render;
mod rooms;
mod search;
mod settings;

use crate::api::SessionApi;
use crate::models::{AppState, TuiExit};
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
    /// Which divider the left mouse button is holding, if any.
    resize_drag: Option<input::ResizeDrag>,
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
            resize_drag: None,
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

    pub fn run(mut self, mut terminal: DefaultTerminal) -> Result<TuiExit> {
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

        result.map(|()| self.state.exit.unwrap_or(TuiExit::Quit))
    }

    /// What the window holds, for a test to read back after driving it.
    #[must_use]
    pub const fn state(&self) -> &AppState {
        &self.state
    }

    /// One frame's worth of catching up with the session: everything the
    /// event loop does between drawing and reading the keyboard.
    pub fn poll_session(&mut self) {
        self.update_search_results();
        self.update_downloads();
        self.poll_private_messages();
        self.poll_browse_result();
        self.poll_room_events();
        self.state.uploads = self.client.uploads();
        self.spinner_state = (self.spinner_state + 1) % 10;
        self.save_persisted_state();
    }

    fn run_event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while self.state.exit.is_none() {
            terminal.draw(|frame| self.render(frame))?;

            self.poll_session();

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
                    if self.state.exit.is_some() || !poll(Duration::ZERO)? {
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
) -> Result<TuiExit> {
    let tui =
        MainTui::new(client, download_dir, search_timeout, store, config_path);
    tui.run(terminal)
}

#[cfg(test)]
mod tests;
