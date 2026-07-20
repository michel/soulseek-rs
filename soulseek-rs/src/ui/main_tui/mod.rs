mod browse;
mod downloads;
mod input;
mod render;
mod rooms;
mod search;

use crate::models::AppState;
use crate::persist::{
    snapshot::{Snapshot, restore_searches},
    state::StateStore,
};
use color_eyre::Result;
use ratatui::{
    DefaultTerminal,
    crossterm::event::{self, Event, KeyEventKind, poll},
};
use soulseek_rs::Client;
use std::{sync::Arc, time::Duration};

pub struct MainTui {
    client: Arc<Client>,
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
        client: Arc<Client>,
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
        let Some(store) = &self.store else { return };

        restore_searches(&mut self.state, &store.load_search_queries());

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
    client: Arc<Client>,
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
