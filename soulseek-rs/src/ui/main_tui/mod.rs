mod browse;
mod downloads;
mod input;
mod render;
mod rooms;
mod search;

use crate::models::AppState;
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
}

impl MainTui {
    pub fn new(
        client: Arc<Client>,
        download_dir: String,
        max_concurrent_downloads: usize,
        search_timeout: Duration,
    ) -> Self {
        Self {
            client,
            state: AppState::new(),
            download_dir,
            max_concurrent_downloads,
            search_timeout,
            spinner_state: 0,
        }
    }

    pub fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        use ratatui::crossterm::{event::DisableMouseCapture, execute};

        // Run the event loop, then restore the terminal unconditionally: if the
        // loop returns early with an error the terminal must still be taken out
        // of raw mode / the alternate screen and mouse capture disabled, or the
        // user is left with a corrupted terminal.
        let result = self.run_event_loop(&mut terminal);

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
) -> Result<()> {
    let tui = MainTui::new(
        client,
        download_dir,
        max_concurrent_downloads,
        search_timeout,
    );
    tui.run(terminal)
}
