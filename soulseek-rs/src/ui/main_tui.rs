use crate::models::{
    AppState, BrowseState, BrowseStatus, ChatMessage, CommandBarMode,
    DownloadEntry, FileDisplayData, FocusedPane, MessageDirection, RoomsView,
    SearchEntry, SearchStatus, files_under, find_node,
};
use crate::ui::panes::{
    ResultsPaneParams, render_browse_pane, render_download_info_pane,
    render_downloads_pane, render_results_pane, render_rooms_pane,
    render_searches_pane,
};
use crate::ui::{
    border_style, border_type, format_shortcuts_styled, render_download_stats,
};
use color_eyre::Result;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind, poll,
    },
    layout::{Constraint, Layout, Position, Rect},
    widgets::{Block, Borders, Paragraph},
};
use soulseek_rs::{Client, DownloadStatus};
use std::{
    sync::{
        Arc,
        atomic::AtomicBool,
        mpsc::{self},
    },
    thread,
    time::{Duration, Instant},
};

const COMMAND_BAR_PREFIX: &str = "search: ";
const MESSAGE_BAR_PREFIX: &str = "message (to: recipient text): ";
const BROWSE_BAR_PREFIX: &str = "browse user: ";

/// How long to wait for a browse response before showing a timeout notice.
const BROWSE_TIMEOUT: Duration = Duration::from_secs(20);

const fn command_bar_prefix(mode: CommandBarMode) -> &'static str {
    match mode {
        CommandBarMode::Search => COMMAND_BAR_PREFIX,
        CommandBarMode::Message => MESSAGE_BAR_PREFIX,
        CommandBarMode::Browse => BROWSE_BAR_PREFIX,
    }
}

pub struct MainTui {
    client: Arc<Client>,
    state: AppState,
    download_dir: String,
    #[allow(dead_code)]
    max_concurrent_downloads: usize,
    search_timeout: Duration,
    #[allow(dead_code)]
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

            // Update spinner
            self.spinner_state = (self.spinner_state + 1) % 10;

            // Poll for input events
            if poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.handle_key_event(key);
                    }
                    Event::Mouse(mouse) => {
                        self.handle_mouse_event(mouse);
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    fn render(&mut self, frame: &mut Frame) {
        // Create layout based on whether command bar is active
        let main_chunks = if self.state.command_bar_active {
            Layout::vertical([
                Constraint::Length(3), // Status bar
                Constraint::Fill(1),   // Main content
                Constraint::Length(3), // Command bar
                Constraint::Length(3), // Shortcuts
            ])
            .split(frame.area())
        } else {
            Layout::vertical([
                Constraint::Length(3), // Status bar
                Constraint::Fill(1),   // Main content
                Constraint::Length(3), // Shortcuts
            ])
            .split(frame.area())
        };

        // Render status bar
        render_download_stats(
            frame,
            main_chunks[0],
            &self.state.downloads,
            self.state.active_downloads_count,
        );

        // Split main content area
        let content_chunks = Layout::horizontal([
            Constraint::Percentage(30), // Searches pane
            Constraint::Percentage(70), // Results + Downloads
        ])
        .split(main_chunks[1]);

        // Store searches pane area
        self.state.searches_pane_area = Some(content_chunks[0]);

        // Render Searches pane (left)
        render_searches_pane(
            frame,
            content_chunks[0],
            &self.state.searches,
            &mut self.state.searches_table_state,
            self.state.focused_pane == FocusedPane::Searches,
        );

        // Split right side into Results (top) and Downloads (bottom)
        let right_chunks = Layout::vertical([
            Constraint::Percentage(60), // Results
            Constraint::Percentage(40), // Downloads
        ])
        .split(content_chunks[1]);

        // Split bottom-right into Downloads list and Info pane
        let downloads_chunks = Layout::horizontal([
            Constraint::Percentage(60), // Downloads list
            Constraint::Percentage(40), // Info pane
        ])
        .split(right_chunks[1]);

        // Store results and downloads pane areas
        self.state.results_pane_area = Some(right_chunks[0]);
        self.state.downloads_pane_area = Some(downloads_chunks[0]);

        // Render Results pane. When a filter is active the rendered rows are a
        // subset, so the pane also needs the mapping back to unfiltered indices
        // to render the selection checkboxes correctly.
        let (results_items, results_original_indices) =
            if self.state.results_filter_query.is_empty() {
                (&self.state.results_items, None)
            } else {
                (
                    &self.state.results_filtered_items,
                    Some(self.state.results_filtered_indices.as_slice()),
                )
            };

        let active_search_query = self
            .state
            .selected_search_index
            .and_then(|idx| self.state.searches.get(idx))
            .map(|search| search.query.as_str());

        render_results_pane(
            frame,
            right_chunks[0],
            ResultsPaneParams {
                items: results_items,
                table_state: &mut self.state.results_table_state,
                selected_indices: &self.state.results_selected_indices,
                original_indices: results_original_indices,
                filter_query: &self.state.results_filter_query,
                is_filtering: self.state.results_is_filtering,
                focused: self.state.focused_pane == FocusedPane::Results,
                active_search_query,
            },
        );

        // Render Downloads pane
        render_downloads_pane(
            frame,
            downloads_chunks[0],
            &self.state.downloads,
            &mut self.state.downloads_table_state,
            self.state.focused_pane == FocusedPane::Downloads,
        );

        // Render Info pane next to downloads
        let selected_download = self
            .state
            .downloads_table_state
            .selected()
            .and_then(|idx| self.state.downloads.get(idx));
        render_download_info_pane(
            frame,
            downloads_chunks[1],
            selected_download,
            self.state.focused_pane == FocusedPane::Downloads,
        );

        // Render command bar if active (vim-style, above shortcuts)
        if self.state.command_bar_active {
            self.render_command_bar(frame, main_chunks[2]);
            self.render_shortcuts(frame, main_chunks[3]);
        } else {
            self.render_shortcuts(frame, main_chunks[2]);
        }

        // Messages inbox overlays everything when open.
        if self.state.show_messages {
            self.render_messages_popup(frame);
        }

        // Browse tree overlays everything when open.
        if self.state.show_browse
            && let Some(browse) = self.state.browse.as_ref()
        {
            let area = centered_rect(80, 80, frame.area());
            frame.render_widget(ratatui::widgets::Clear, area);
            render_browse_pane(
                frame,
                area,
                browse,
                &mut self.state.browse_table_state,
                self.spinner_state,
            );
        }

        // Chat rooms overlay everything when open.
        if self.state.show_rooms {
            let area = centered_rect(85, 80, frame.area());
            frame.render_widget(ratatui::widgets::Clear, area);
            render_rooms_pane(
                frame,
                area,
                &self.state.rooms,
                &mut self.state.rooms_list_table_state,
            );
        }
    }

    fn render_messages_popup(&self, frame: &mut Frame) {
        let area = centered_rect(70, 60, frame.area());

        let lines: Vec<ratatui::text::Line> = if self.state.messages.is_empty()
        {
            vec![ratatui::text::Line::from(
                "No messages yet. Press 'm' to send one.",
            )]
        } else {
            self.state
                .messages
                .iter()
                .map(|m| {
                    let (arrow, who) = match m.direction {
                        MessageDirection::Incoming => ("⇦ from", &m.peer),
                        MessageDirection::Outgoing => ("⇨ to  ", &m.peer),
                    };
                    ratatui::text::Line::from(format!(
                        "{arrow} {who}: {}",
                        m.text
                    ))
                })
                .collect()
        };

        let popup = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style(true))
                .border_type(border_type(true))
                .title(" Messages  (m: compose, i/Esc: close) "),
        );

        frame.render_widget(ratatui::widgets::Clear, area);
        frame.render_widget(popup, area);
    }

    /// Context shortcuts for the chat-rooms popup.
    fn rooms_shortcuts(&self) -> Vec<(&'static str, &'static str)> {
        if self.state.rooms.composing {
            return vec![
                ("Type", "message"),
                ("Enter", "send"),
                ("Esc", "cancel"),
            ];
        }
        match self.state.rooms.view {
            RoomsView::List => {
                if self.state.rooms.list_is_filtering {
                    vec![
                        ("Type", "filter"),
                        ("Enter", "join match"),
                        ("Esc", "clear filter"),
                    ]
                } else {
                    vec![
                        ("↑↓", "move"),
                        ("Enter", "join"),
                        ("/", "filter"),
                        ("Tab", "open rooms"),
                        ("Esc", "close"),
                    ]
                }
            }
            RoomsView::Chat => vec![
                ("Enter", "type message"),
                ("Tab", "switch room"),
                ("l", "room list"),
                ("x", "leave"),
                ("q", "close"),
            ],
        }
    }

    fn render_shortcuts(&self, frame: &mut Frame, area: Rect) {
        // Unread badges for the inbox and chat shortcuts.
        let inbox_label = if self.state.unread_messages > 0 {
            format!("inbox ({})", self.state.unread_messages)
        } else {
            "inbox".to_string()
        };
        let chat_unread = self.state.rooms.total_unread();
        let chat_label = if chat_unread > 0 {
            format!("chat ({chat_unread})")
        } else {
            "chat".to_string()
        };

        let shortcuts = if self.state.show_rooms {
            self.rooms_shortcuts()
        } else if self.state.show_browse {
            vec![
                ("↑↓", "move"),
                ("→←", "expand/collapse"),
                ("Enter", "open/download"),
                ("d", "download folder"),
                ("Esc", "close"),
            ]
        } else if self.state.command_bar_active {
            match self.state.command_bar_mode {
                CommandBarMode::Search => vec![
                    ("Type", "search term"),
                    ("←→", "move cursor"),
                    ("Backspace/Del", "edit"),
                    ("Enter", "search"),
                    ("Esc", "cancel"),
                ],
                CommandBarMode::Message => vec![
                    ("Type", "recipient then message"),
                    ("Enter", "send"),
                    ("Esc", "cancel"),
                ],
                CommandBarMode::Browse => vec![
                    ("Type", "username"),
                    ("Enter", "browse"),
                    ("Esc", "cancel"),
                ],
            }
        } else {
            match self.state.focused_pane {
                FocusedPane::Searches => vec![
                    ("s", "search"),
                    ("m", "message"),
                    ("i", inbox_label.as_str()),
                    ("c", chat_label.as_str()),
                    ("b", "browse user"),
                    ("1-3", "focus pane"),
                    ("↑↓", "navigate"),
                    ("Enter", "results"),
                    ("q", "quit"),
                ],
                FocusedPane::Results if self.state.results_is_filtering => {
                    vec![
                        ("Type", "filter"),
                        ("Esc", "clear filter"),
                        ("1-3", "focus pane"),
                        ("q", "quit"),
                    ]
                }
                FocusedPane::Results => vec![
                    ("Space", "select"),
                    ("Enter", "download"),
                    ("b", "browse owner"),
                    ("c", chat_label.as_str()),
                    ("/", "filter"),
                    ("a/A", "select all/none"),
                    ("1-3", "focus pane"),
                    ("q", "quit"),
                ],
                FocusedPane::Downloads => {
                    vec![
                        ("p", "pause/resume"),
                        ("r", "retry failed"),
                        ("d", "delete queued"),
                        ("c", "clear finished"),
                        ("b", "browse user"),
                        ("1-3", "focus pane"),
                        ("q", "quit"),
                    ]
                }
            }
        };

        let shortcuts_line = format_shortcuts_styled(&shortcuts);
        // Surface our own sharing status in the block title.
        let title = format!(
            "Shortcuts · Sharing: {}",
            self.client.shared_directory().unwrap_or("off")
        );
        let shortcuts_widget = Paragraph::new(shortcuts_line).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(border_type(false))
                .title(title),
        );

        frame.render_widget(shortcuts_widget, area);
    }

    fn render_command_bar(&self, frame: &mut Frame, area: Rect) {
        let prefix = command_bar_prefix(self.state.command_bar_mode);
        let content_width = area.width.saturating_sub(2);
        let prefix_width = prefix.chars().count() as u16;
        let input_width = content_width.saturating_sub(prefix_width);
        let (visible_input, cursor_column) = visible_input_at_cursor(
            &self.state.command_bar_input,
            self.state.command_bar_cursor_position,
            input_width,
        );
        let command_text = format!("{prefix}{visible_input}");

        let paragraph = Paragraph::new(command_text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style(true))
                .border_type(border_type(true)),
        );

        frame.render_widget(paragraph, area);

        if area.width > 2 && area.height > 2 {
            let cursor_x = area
                .x
                .saturating_add(1)
                .saturating_add(prefix_width)
                .saturating_add(cursor_column)
                .min(area.x.saturating_add(area.width.saturating_sub(2)));
            frame.set_cursor_position(Position::new(cursor_x, area.y + 1));
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) {
        // Command bar takes priority
        if self.state.command_bar_active {
            return self.handle_command_bar_input(key);
        }

        // Messages popup: any of i/Esc/q closes it.
        if self.state.show_messages {
            if matches!(key.code, KeyCode::Char('i' | 'q') | KeyCode::Esc) {
                self.state.show_messages = false;
            }
            return;
        }

        // Browse popup takes over navigation while open.
        if self.state.show_browse {
            return self.handle_browse_input(key);
        }

        // Rooms popup takes over navigation while open.
        if self.state.show_rooms {
            return self.handle_rooms_input(key);
        }

        // Filter mode in Results pane
        if self.state.results_is_filtering
            && self.state.focused_pane == FocusedPane::Results
        {
            return self.handle_filter_input(key);
        }

        // Global shortcuts
        match key.code {
            KeyCode::Char('q') => {
                self.state.should_exit = true;
                return;
            }
            KeyCode::Char('1') => {
                self.state.focused_pane = FocusedPane::Searches;
                return;
            }
            KeyCode::Char('2') => {
                self.state.focused_pane = FocusedPane::Results;
                return;
            }
            KeyCode::Char('3') => {
                self.state.focused_pane = FocusedPane::Downloads;
                return;
            }
            KeyCode::Char('s') => {
                self.state.command_bar_active = true;
                self.state.command_bar_mode = CommandBarMode::Search;
                self.state.command_bar_input.clear();
                self.state.command_bar_cursor_position = 0;
                return;
            }
            KeyCode::Char('m') => {
                self.state.command_bar_active = true;
                self.state.command_bar_mode = CommandBarMode::Message;
                self.state.command_bar_input.clear();
                self.state.command_bar_cursor_position = 0;
                return;
            }
            KeyCode::Char('i') => {
                self.state.show_messages = true;
                self.state.unread_messages = 0;
                return;
            }
            // Chat rooms. In the Downloads pane `c` clears finished downloads,
            // so only open chat from the other panes (like `b` is contextual).
            KeyCode::Char('c')
                if self.state.focused_pane != FocusedPane::Downloads =>
            {
                self.start_rooms();
                return;
            }
            KeyCode::Char('b') => {
                // From a highlighted search result, browse its owner directly;
                // otherwise prompt for a username.
                if self.state.focused_pane == FocusedPane::Results
                    && let Some(owner) = self.highlighted_result_owner()
                {
                    self.start_browse(owner);
                } else {
                    self.state.command_bar_active = true;
                    self.state.command_bar_mode = CommandBarMode::Browse;
                    self.state.command_bar_input.clear();
                    self.state.command_bar_cursor_position = 0;
                }
                return;
            }
            _ => {}
        }

        // Pane-specific shortcuts
        match self.state.focused_pane {
            FocusedPane::Searches => self.handle_searches_input(key),
            FocusedPane::Results => self.handle_results_input(key),
            FocusedPane::Downloads => self.handle_downloads_input(key),
        }
    }

    fn handle_browse_input(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            self.state.show_browse = false;
            return;
        }

        // Snapshot the flattened rows + current selection, then drop the borrow.
        let (rows, sel, row) = {
            let Some(browse) = self.state.browse.as_ref() else {
                self.state.show_browse = false;
                return;
            };
            if browse.status != BrowseStatus::Loaded {
                return;
            }
            let rows = browse.rows();
            if rows.is_empty() {
                return;
            }
            let sel = browse.selected_row.min(rows.len() - 1);
            let row = rows[sel].clone();
            (rows, sel, row)
        };

        // Downloads need `&self.client` free of the browse borrow.
        match key.code {
            KeyCode::Enter if !row.is_folder => {
                self.queue_browse_files(vec![(
                    row.path.clone(),
                    row.size.unwrap_or(0),
                )]);
                return;
            }
            KeyCode::Char('d') => {
                let files = if row.is_folder {
                    self.browse_folder_files(&row.path)
                } else {
                    vec![(row.path.clone(), row.size.unwrap_or(0))]
                };
                self.queue_browse_files(files);
                return;
            }
            _ => {}
        }

        // Navigation and expand/collapse mutate the browse state.
        if let Some(browse) = self.state.browse.as_mut() {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    browse.selected_row = sel.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    browse.selected_row = (sel + 1).min(rows.len() - 1);
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    if row.is_folder && !row.expanded {
                        browse.expanded.insert(row.path.clone());
                    } else if row.is_folder {
                        browse.selected_row = (sel + 1).min(rows.len() - 1);
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if row.is_folder && row.expanded {
                        browse.expanded.remove(&row.path);
                    } else if let Some(parent) =
                        (0..sel).rev().find(|&i| rows[i].depth < row.depth)
                    {
                        browse.selected_row = parent;
                    }
                }
                KeyCode::Enter => {
                    // Folder toggle (files handled above).
                    if row.expanded {
                        browse.expanded.remove(&row.path);
                    } else {
                        browse.expanded.insert(row.path.clone());
                    }
                }
                _ => {}
            }
            // Re-clamp against the new flattened length.
            let new_len = browse.rows().len();
            browse.selected_row =
                browse.selected_row.min(new_len.saturating_sub(1));
        }

        let selected = self.state.browse.as_ref().map(|b| b.selected_row);
        self.state.browse_table_state.select(selected);
    }

    /// Files (`path`, `size`) under the browse-tree folder at `path`.
    fn browse_folder_files(&self, path: &str) -> Vec<(String, u64)> {
        self.state
            .browse
            .as_ref()
            .and_then(|b| find_node(&b.tree, path))
            .map(files_under)
            .unwrap_or_default()
    }

    /// Queue downloads of `files` (path, size) from the currently-browsed user.
    fn queue_browse_files(&mut self, files: Vec<(String, u64)>) {
        let Some(username) =
            self.state.browse.as_ref().map(|b| b.username.clone())
        else {
            return;
        };
        if files.is_empty() {
            return;
        }
        if self.state.downloads_receiver_channel.is_none() {
            let (sender, receiver) = mpsc::channel();
            self.state.downloads_receiver_channel = Some(receiver);
            self.state.downloads_sender_channel = Some(sender);
        }
        let client = self.client.clone();
        let download_dir = self.download_dir.clone();
        let sender = self.state.downloads_sender_channel.clone().unwrap();
        thread::spawn(move || {
            for (path, size) in files {
                match client.download(
                    path.clone(),
                    username.clone(),
                    size,
                    download_dir.clone(),
                ) {
                    Ok((download, rx)) => {
                        let _ = sender.send((download, rx));
                    }
                    Err(e) => {
                        eprintln!("Failed to queue download {path}: {e}");
                    }
                }
            }
        });
    }

    /// Open the chat-rooms popup and refresh the room list. If rooms are
    /// already open, jump straight to the chat view; otherwise show the list.
    fn start_rooms(&mut self) {
        let _ = self.client.request_room_list();
        self.state.show_rooms = true;
        if self.state.rooms.open.is_empty() {
            self.state.rooms.view = RoomsView::List;
        } else {
            self.state.rooms.view = RoomsView::Chat;
            self.state.rooms.mark_active_read();
        }
    }

    fn handle_rooms_input(&mut self, key: KeyEvent) {
        // Composing a message captures typing.
        if self.state.rooms.composing {
            self.handle_room_compose_input(key);
            return;
        }
        // Filtering the room list captures typing.
        if self.state.rooms.view == RoomsView::List
            && self.state.rooms.list_is_filtering
        {
            self.handle_room_filter_input(key);
            return;
        }

        match self.state.rooms.view {
            RoomsView::List => self.handle_rooms_list_input(key),
            RoomsView::Chat => self.handle_rooms_chat_input(key),
        }
    }

    fn handle_rooms_list_input(&mut self, key: KeyEvent) {
        let len = self.state.rooms.filtered_rooms().len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.show_rooms = false;
            }
            KeyCode::Char('/') => {
                self.state.rooms.list_is_filtering = true;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.rooms.list_selected =
                    self.state.rooms.list_selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if len > 0 {
                    self.state.rooms.list_selected =
                        (self.state.rooms.list_selected + 1).min(len - 1);
                }
            }
            KeyCode::Enter => self.join_selected_room(),
            KeyCode::Tab if !self.state.rooms.open.is_empty() => {
                self.state.rooms.view = RoomsView::Chat;
                self.state.rooms.mark_active_read();
            }
            _ => {}
        }
    }

    fn handle_room_filter_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.state.rooms.list_is_filtering = false;
                self.state.rooms.list_filter.clear();
                self.state.rooms.list_selected = 0;
            }
            KeyCode::Enter => {
                self.state.rooms.list_is_filtering = false;
                self.join_selected_room();
            }
            KeyCode::Char(c) => {
                self.state.rooms.list_filter.push(c);
                self.state.rooms.list_selected = 0;
            }
            KeyCode::Backspace => {
                self.state.rooms.list_filter.pop();
                self.state.rooms.list_selected = 0;
            }
            _ => {}
        }
    }

    fn handle_rooms_chat_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.state.show_rooms = false,
            KeyCode::Esc | KeyCode::Char('l') => {
                self.state.rooms.view = RoomsView::List;
            }
            KeyCode::Tab => self.state.rooms.next_tab(),
            KeyCode::BackTab => self.state.rooms.prev_tab(),
            KeyCode::Char('x') => self.leave_active_room(),
            KeyCode::Enter if self.state.rooms.active_room().is_some() => {
                self.state.rooms.composing = true;
            }
            _ => {}
        }
    }

    fn handle_room_compose_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.send_room_message(),
            KeyCode::Esc => {
                self.state.rooms.composing = false;
                if let Some(room) =
                    self.state.rooms.open.get_mut(self.state.rooms.active)
                {
                    room.input.clear();
                }
            }
            KeyCode::Backspace => {
                if let Some(room) =
                    self.state.rooms.open.get_mut(self.state.rooms.active)
                {
                    room.input.pop();
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(room) =
                    self.state.rooms.open.get_mut(self.state.rooms.active)
                {
                    room.input.push(c);
                }
            }
            _ => {}
        }
    }

    /// Join (or focus) the room highlighted in the list view.
    fn join_selected_room(&mut self) {
        if let Some(name) = self.state.rooms.selected_room_name() {
            let newly_opened = self.state.rooms.focus_or_open(&name);
            if newly_opened && let Err(e) = self.client.join_room(&name) {
                eprintln!("Failed to join {name}: {e}");
            }
        }
    }

    /// Leave the active room and close its tab.
    fn leave_active_room(&mut self) {
        if let Some(name) = self.state.rooms.close_active()
            && let Err(e) = self.client.leave_room(&name)
        {
            eprintln!("Failed to leave {name}: {e}");
        }
    }

    /// Send the active room's compose buffer. The server echoes the message
    /// back as a RoomEvent, which is what actually renders it in the log.
    fn send_room_message(&mut self) {
        let active = self.state.rooms.active;
        let (room, text) = match self.state.rooms.open.get(active) {
            Some(room) if !room.input.trim().is_empty() => {
                (room.name.clone(), room.input.trim().to_string())
            }
            _ => {
                self.state.rooms.composing = false;
                return;
            }
        };
        if let Err(e) = self.client.say_in_room(&room, &text) {
            eprintln!("Failed to say in {room}: {e}");
        }
        if let Some(room) = self.state.rooms.open.get_mut(active) {
            room.input.clear();
        }
        self.state.rooms.composing = false;
    }

    /// Drain chat-room events into the rooms state, tracking unread badges.
    fn poll_room_events(&mut self) {
        let viewing = if self.state.show_rooms
            && self.state.rooms.view == RoomsView::Chat
        {
            self.state.rooms.active_room().map(|r| r.name.clone())
        } else {
            None
        };
        for event in self.client.take_room_events() {
            self.state.rooms.apply_event(event, viewing.as_deref());
        }
    }

    fn handle_command_bar_input(&mut self, key: KeyEvent) {
        self.state.command_bar_cursor_position = clamp_cursor_to_char_boundary(
            &self.state.command_bar_input,
            self.state.command_bar_cursor_position,
        );

        match key.code {
            KeyCode::Enter => {
                let input = self.state.command_bar_input.trim().to_string();
                if !input.is_empty() {
                    match self.state.command_bar_mode {
                        CommandBarMode::Search => self.start_search(input),
                        CommandBarMode::Message => {
                            self.send_message_from_input(&input);
                        }
                        CommandBarMode::Browse => self.start_browse(input),
                    }
                }
                self.state.command_bar_active = false;
                self.state.command_bar_input.clear();
                self.state.command_bar_cursor_position = 0;
            }
            KeyCode::Esc => {
                self.state.command_bar_active = false;
                self.state.command_bar_input.clear();
                self.state.command_bar_cursor_position = 0;
            }
            KeyCode::Backspace => {
                let cursor_position = self.state.command_bar_cursor_position;
                if cursor_position > 0 {
                    let previous_position = previous_char_boundary(
                        &self.state.command_bar_input,
                        cursor_position,
                    );
                    self.state
                        .command_bar_input
                        .drain(previous_position..cursor_position);
                    self.state.command_bar_cursor_position = previous_position;
                }
            }
            KeyCode::Delete => {
                let cursor_position = self.state.command_bar_cursor_position;
                if cursor_position < self.state.command_bar_input.len() {
                    let next_position = next_char_boundary(
                        &self.state.command_bar_input,
                        cursor_position,
                    );
                    self.state
                        .command_bar_input
                        .drain(cursor_position..next_position);
                }
            }
            KeyCode::Left => {
                self.state.command_bar_cursor_position = previous_char_boundary(
                    &self.state.command_bar_input,
                    self.state.command_bar_cursor_position,
                );
            }
            KeyCode::Right => {
                self.state.command_bar_cursor_position = next_char_boundary(
                    &self.state.command_bar_input,
                    self.state.command_bar_cursor_position,
                );
            }
            KeyCode::Home => {
                self.state.command_bar_cursor_position = 0;
            }
            KeyCode::End => {
                self.state.command_bar_cursor_position =
                    self.state.command_bar_input.len();
            }
            KeyCode::Char('a')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.state.command_bar_cursor_position = 0;
            }
            KeyCode::Char('e')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.state.command_bar_cursor_position =
                    self.state.command_bar_input.len();
            }
            KeyCode::Char('u')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.state.command_bar_input.clear();
                self.state.command_bar_cursor_position = 0;
            }
            KeyCode::Char(c)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL
                        | KeyModifiers::ALT
                        | KeyModifiers::SUPER
                        | KeyModifiers::HYPER
                        | KeyModifiers::META,
                ) =>
            {
                let cursor_position = self.state.command_bar_cursor_position;
                self.state.command_bar_input.insert(cursor_position, c);
                self.state.command_bar_cursor_position =
                    cursor_position + c.len_utf8();
            }
            _ => {}
        }
    }

    fn handle_filter_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.state.results_is_filtering = false;
                self.state.results_filter_query.clear();
                self.state.results_filtered_items =
                    self.state.results_items.clone();
                self.state.results_filtered_indices =
                    (0..self.state.results_items.len()).collect();
            }
            KeyCode::Char(c) => {
                self.state.results_filter_query.push(c);
                self.apply_filter();
            }
            KeyCode::Backspace => {
                self.state.results_filter_query.pop();
                self.apply_filter();
            }
            _ => {}
        }
    }

    fn handle_searches_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k')
                if !self.state.searches.is_empty() =>
            {
                let current =
                    self.state.searches_table_state.selected().unwrap_or(0);
                let new = if current == 0 {
                    self.state.searches.len() - 1
                } else {
                    current - 1
                };
                self.state.searches_table_state.select(Some(new));
            }
            KeyCode::Down | KeyCode::Char('j')
                if !self.state.searches.is_empty() =>
            {
                let current =
                    self.state.searches_table_state.selected().unwrap_or(0);
                let new = (current + 1) % self.state.searches.len();
                self.state.searches_table_state.select(Some(new));
            }
            KeyCode::Enter => {
                if let Some(selected) =
                    self.state.searches_table_state.selected()
                {
                    self.state.selected_search_index = Some(selected);
                    if let Some(search) = self.state.searches.get(selected) {
                        self.state.results_items = search.results.clone();
                        self.state.results_filtered_items =
                            search.results.clone();
                        self.state.results_filtered_indices =
                            (0..search.results.len()).collect();
                        self.state.results_selected_indices.clear();
                        self.state.results_table_state.select(Some(0));
                        self.state.focused_pane = FocusedPane::Results;
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(selected) =
                    self.state.searches_table_state.selected()
                {
                    self.remove_search_at_index(selected);
                }
            }
            KeyCode::Char('C') => {
                self.clear_all_searches();
            }
            _ => {}
        }
    }

    fn handle_results_input(&mut self, key: KeyEvent) {
        let items_count = if self.state.results_filter_query.is_empty() {
            self.state.results_items.len()
        } else {
            self.state.results_filtered_items.len()
        };

        match key.code {
            KeyCode::Up | KeyCode::Char('k') if items_count > 0 => {
                let current =
                    self.state.results_table_state.selected().unwrap_or(0);
                let new = if current == 0 {
                    items_count - 1
                } else {
                    current - 1
                };
                self.state.results_table_state.select(Some(new));
            }
            KeyCode::Down | KeyCode::Char('j') if items_count > 0 => {
                let current =
                    self.state.results_table_state.selected().unwrap_or(0);
                let new = (current + 1) % items_count;
                self.state.results_table_state.select(Some(new));
            }
            KeyCode::Char(' ') => {
                if let Some(current) = self.state.results_table_state.selected()
                {
                    let actual_index =
                        if self.state.results_filter_query.is_empty() {
                            current
                        } else {
                            self.state.results_filtered_indices[current]
                        };

                    if self
                        .state
                        .results_selected_indices
                        .contains(&actual_index)
                    {
                        self.state
                            .results_selected_indices
                            .remove(&actual_index);
                    } else {
                        self.state
                            .results_selected_indices
                            .insert(actual_index);
                    }
                }
            }
            KeyCode::Char('/') => {
                self.state.results_is_filtering = true;
                self.state.results_filter_query.clear();
            }
            KeyCode::Char('a') => {
                let indices: Vec<usize> =
                    if self.state.results_filter_query.is_empty() {
                        (0..self.state.results_items.len()).collect()
                    } else {
                        self.state.results_filtered_indices.clone()
                    };
                self.state.results_selected_indices.extend(indices);
            }
            KeyCode::Char('A') => {
                self.state.results_selected_indices.clear();
            }
            KeyCode::Enter => {
                self.queue_selected_downloads();
            }
            _ => {}
        }
    }

    fn handle_downloads_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k')
                if !self.state.downloads.is_empty() =>
            {
                let current =
                    self.state.downloads_table_state.selected().unwrap_or(0);
                let new = if current == 0 {
                    self.state.downloads.len() - 1
                } else {
                    current - 1
                };
                self.state.downloads_table_state.select(Some(new));
            }
            KeyCode::Down | KeyCode::Char('j')
                if !self.state.downloads.is_empty() =>
            {
                let current =
                    self.state.downloads_table_state.selected().unwrap_or(0);
                let new = (current + 1) % self.state.downloads.len();
                self.state.downloads_table_state.select(Some(new));
            }
            KeyCode::Char('p') => {
                self.toggle_selected_download_pause();
            }
            KeyCode::Char('d') => {
                self.remove_selected_queued_download();
            }
            KeyCode::Char('r') => {
                self.retry_selected_download();
            }
            KeyCode::Char('c') => {
                self.clear_finished_downloads();
            }
            _ => {}
        }
    }

    /// Re-queue the selected download if it failed or timed out.
    fn retry_selected_download(&mut self) {
        let Some(index) = self.state.downloads_table_state.selected() else {
            return;
        };
        let Some(entry) = self.state.downloads.get(index) else {
            return;
        };
        if !matches!(
            entry.download.status,
            DownloadStatus::Failed(_) | DownloadStatus::TimedOut
        ) {
            return;
        }
        let filename = entry.download.filename.clone();
        let username = entry.download.username.clone();
        let size = entry.download.size;
        let directory = entry.download.download_directory.clone();

        if self.state.downloads_receiver_channel.is_none() {
            let (sender, receiver) = mpsc::channel();
            self.state.downloads_receiver_channel = Some(receiver);
            self.state.downloads_sender_channel = Some(sender);
        }
        let client = self.client.clone();
        let sender = self.state.downloads_sender_channel.clone().unwrap();

        // Drop the old failed entry; the retry pushes a fresh one.
        self.state.downloads.remove(index);
        self.select_download_after_removal(index);

        thread::spawn(move || {
            match client.download(filename.clone(), username, size, directory) {
                Ok((download, rx)) => {
                    let _ = sender.send((download, rx));
                }
                Err(e) => eprintln!("Failed to retry {filename}: {e}"),
            }
        });
    }

    /// Remove all completed / failed / timed-out downloads from the list.
    fn clear_finished_downloads(&mut self) {
        self.state.downloads.retain(|entry| {
            !matches!(
                entry.download.status,
                DownloadStatus::Completed
                    | DownloadStatus::Failed(_)
                    | DownloadStatus::TimedOut
            )
        });
        let len = self.state.downloads.len();
        if len == 0 {
            self.state.downloads_table_state.select(None);
        } else {
            let selected = self
                .state
                .downloads_table_state
                .selected()
                .unwrap_or(0)
                .min(len - 1);
            self.state.downloads_table_state.select(Some(selected));
        }
    }

    fn toggle_selected_download_pause(&self) {
        let Some(index) = self.state.downloads_table_state.selected() else {
            return;
        };
        let Some(download_entry) = self.state.downloads.get(index) else {
            return;
        };

        let download = &download_entry.download;
        match download.status {
            DownloadStatus::InProgress { .. } => {
                let _ = self
                    .client
                    .pause_download(&download.username, &download.filename);
            }
            DownloadStatus::Paused { .. } => {
                let _ = self
                    .client
                    .resume_download(&download.username, &download.filename);
            }
            _ => {}
        }
    }

    fn remove_selected_queued_download(&mut self) {
        let Some(index) = self.state.downloads_table_state.selected() else {
            return;
        };
        let Some(download_entry) = self.state.downloads.get(index) else {
            return;
        };

        let download = &download_entry.download;
        if !matches!(download.status, DownloadStatus::Queued) {
            return;
        }

        if !self
            .client
            .remove_queued_download(&download.username, &download.filename)
        {
            return;
        }

        self.state.downloads.remove(index);
        self.select_download_after_removal(index);
    }

    fn select_download_after_removal(&mut self, removed_index: usize) {
        if self.state.downloads.is_empty() {
            self.state.downloads_table_state.select(None);
            return;
        }

        let next_index = removed_index.min(self.state.downloads.len() - 1);
        self.state.downloads_table_state.select(Some(next_index));
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            let (col, row) = (mouse.column, mouse.row);

            // Check if click is within searches pane
            if let Some(area) = self.state.searches_pane_area
                && col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.state.focused_pane = FocusedPane::Searches;
                return;
            }

            // Check if click is within results pane
            if let Some(area) = self.state.results_pane_area
                && col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.state.focused_pane = FocusedPane::Results;
                return;
            }

            // Check if click is within downloads pane
            if let Some(area) = self.state.downloads_pane_area
                && col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.state.focused_pane = FocusedPane::Downloads;
            }
        }
    }

    fn remove_search_at_index(&mut self, index: usize) {
        if index >= self.state.searches.len() {
            return;
        }

        // Cancel the search if it's active
        if let Some(search) = self.state.searches.get(index) {
            search
                .cancel_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        // Check if we're removing the currently active search
        let was_active_search = self.state.selected_search_index == Some(index);

        // Remove the search
        self.state.searches.remove(index);

        // Update selected_search_index
        if let Some(current_idx) = self.state.selected_search_index {
            if current_idx == index {
                // Removed the active search - clear it
                self.state.selected_search_index = None;
            } else if current_idx > index {
                // Active search was after removed one - decrement index
                self.state.selected_search_index = Some(current_idx - 1);
            }
            // If current_idx < index, no change needed
        }

        // If we removed the active search, clear results pane
        if was_active_search {
            self.clear_results_pane();
        }

        // Update table selection
        if self.state.searches.is_empty() {
            self.state.searches_table_state.select(None);
        } else {
            let new_selection = if index >= self.state.searches.len() {
                self.state.searches.len() - 1
            } else {
                index
            };
            self.state.searches_table_state.select(Some(new_selection));
        }
    }

    fn clear_all_searches(&mut self) {
        // Cancel all active searches
        for search in &self.state.searches {
            search
                .cancel_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        // Clear searches
        self.state.searches.clear();
        self.state.searches_table_state.select(None);
        self.state.selected_search_index = None;

        // Clear results pane
        self.clear_results_pane();
    }

    fn clear_results_pane(&mut self) {
        self.state.results_items.clear();
        self.state.results_filtered_items.clear();
        self.state.results_filtered_indices.clear();
        self.state.results_selected_indices.clear();
        self.state.results_table_state.select(None);
        self.state.results_filter_query.clear();
        self.state.results_is_filtering = false;
    }

    fn recompute_results_filter(&mut self) {
        let (items, indices) = filter_results(
            &self.state.results_items,
            &self.state.results_filter_query,
        );
        self.state.results_filtered_items = items;
        self.state.results_filtered_indices = indices;
    }

    fn apply_filter(&mut self) {
        self.recompute_results_filter();
        if !self.state.results_filtered_items.is_empty() {
            self.state.results_table_state.select(Some(0));
        }
    }

    /// Drain any private messages received since the last tick into the inbox.
    fn poll_private_messages(&mut self) {
        for msg in self.client.take_private_messages() {
            self.state.messages.push(ChatMessage {
                direction: MessageDirection::Incoming,
                peer: msg.username().to_string(),
                text: msg.message().to_string(),
            });
            // Badge the inbox when it isn't currently open.
            if !self.state.show_messages {
                self.state.unread_messages += 1;
            }
        }
    }

    /// Request a user's shared files and open the browse popup.
    fn start_browse(&mut self, username: String) {
        let username = username.trim().to_string();
        if username.is_empty() {
            return;
        }
        let _ = self.client.browse_user(&username);
        self.state.browse = Some(BrowseState::loading(username));
        self.state.show_browse = true;
        self.state.browse_table_state.select(Some(0));
    }

    /// The username of the highlighted search result (filter-aware).
    fn highlighted_result_owner(&self) -> Option<String> {
        let selected = self.state.results_table_state.selected()?;
        let items = if self.state.results_filter_query.is_empty() {
            &self.state.results_items
        } else {
            &self.state.results_filtered_items
        };
        items.get(selected).map(|f| f.username.clone())
    }

    /// Drain a browse response into the browse state, or time it out.
    fn poll_browse_result(&mut self) {
        let Some((username, status, requested_at)) = self
            .state
            .browse
            .as_ref()
            .map(|b| (b.username.clone(), b.status, b.requested_at))
        else {
            return;
        };
        if status != BrowseStatus::Loading {
            return;
        }
        if let Some(directories) = self.client.take_browse_result(&username) {
            if let Some(browse) = self.state.browse.as_mut() {
                browse.load(&directories);
            }
            self.state.browse_table_state.select(Some(0));
        } else if requested_at.elapsed() > BROWSE_TIMEOUT
            && let Some(browse) = self.state.browse.as_mut()
        {
            browse.status = BrowseStatus::TimedOut;
        }
    }

    /// Parse a `<recipient> <text>` compose line and send it.
    fn send_message_from_input(&mut self, input: &str) {
        let Some((recipient, text)) = input.split_once(char::is_whitespace)
        else {
            return;
        };
        let recipient = recipient.trim();
        let text = text.trim();
        if recipient.is_empty() || text.is_empty() {
            return;
        }

        match self.client.send_private_message(recipient, text) {
            Ok(()) => self.state.messages.push(ChatMessage {
                direction: MessageDirection::Outgoing,
                peer: recipient.to_string(),
                text: text.to_string(),
            }),
            Err(e) => eprintln!("Failed to send message: {e}"),
        }
    }

    fn start_search(&mut self, query: String) {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let search_entry = SearchEntry {
            query: query.clone(),
            status: SearchStatus::Active,
            results: Vec::new(),
            start_time: Instant::now(),
            cancel_flag: cancel_flag.clone(),
        };

        self.state.searches.push(search_entry);
        let search_index = self.state.searches.len() - 1;
        self.state.searches_table_state.select(Some(search_index));

        // Make this search the active one
        self.state.selected_search_index = Some(search_index);

        // Initialize results display (empty at first)
        self.state.results_items.clear();
        self.state.results_filtered_items.clear();
        self.state.results_filtered_indices.clear();
        self.state.results_selected_indices.clear();
        self.state.results_table_state.select(Some(0));

        // Switch focus to Results pane
        self.state.focused_pane = FocusedPane::Results;

        // Start search in background
        let client = self.client.clone();
        let timeout = self.search_timeout;

        thread::spawn(move || {
            match client.search_with_cancel(
                &query,
                timeout,
                Some(cancel_flag.clone()),
            ) {
                Ok(_results) => {
                    // Results will be polled in update_search_results
                }
                Err(e) => {
                    eprintln!("Search failed: {e}");
                }
            }
        });
    }

    fn update_search_results(&mut self) {
        let timeout = self.search_timeout;
        let selected_search_index = self.state.selected_search_index;

        // Fetch all results in one go (single lock acquisition per query)
        // Use try_get_search_results to avoid blocking the UI thread
        let all_results: Vec<(usize, Vec<_>)> = self
            .state
            .searches
            .iter()
            .enumerate()
            .map(|(idx, s)| (idx, s.query.clone()))
            .filter_map(|(idx, query)| {
                self.client
                    .try_get_search_results(&query)
                    .map(|results| (idx, results))
            })
            .collect();

        // Now update state without holding any client locks
        for (idx, search_results) in all_results {
            if let Some(search) = self.state.searches.get_mut(idx) {
                if !search_results.is_empty() {
                    search.results.clear();
                    for result in search_results {
                        for file in result.files {
                            search.results.push(FileDisplayData {
                                filename: file.name.clone(),
                                size: file.size,
                                username: result.username.clone(),
                                speed: result.speed,
                                slots: result.slots,
                                bitrate: file.attribs.get(&0).copied(),
                                length_seconds: file.attribs.get(&1).copied(),
                            });
                        }
                    }

                    // Update selected search if this is the active one. Re-derive
                    // the filtered view from the current query so an active
                    // filter is preserved as new results stream in, rather than
                    // being clobbered by the full unfiltered list.
                    if let Some(selected_idx) = selected_search_index
                        && selected_idx == idx
                    {
                        self.state.results_items = search.results.clone();
                        let (items, indices) = filter_results(
                            &self.state.results_items,
                            &self.state.results_filter_query,
                        );
                        self.state.results_filtered_items = items;
                        self.state.results_filtered_indices = indices;
                    }
                }

                // Mark as completed after timeout
                if search.status == SearchStatus::Active
                    && search.start_time.elapsed() > timeout
                {
                    search.status = SearchStatus::Completed;
                }
            }
        }
    }

    fn queue_selected_downloads(&mut self) {
        let selected_files: Vec<FileDisplayData> = self
            .state
            .results_selected_indices
            .iter()
            .filter_map(|&idx| self.state.results_items.get(idx))
            .cloned()
            .collect();

        if selected_files.is_empty() {
            return;
        }

        if self.state.downloads_receiver_channel.is_none() {
            let (sender, receiver) = mpsc::channel();
            self.state.downloads_receiver_channel = Some(receiver);
            self.state.downloads_sender_channel = Some(sender);
        }

        let client = self.client.clone();
        let download_dir = self.download_dir.clone();
        let sender = self.state.downloads_sender_channel.clone().unwrap();

        thread::spawn(move || {
            for file in selected_files {
                let metadata = soulseek_rs::types::DownloadMetadata {
                    bitrate: file.bitrate,
                    length_seconds: file.length_seconds,
                    peer_upload_speed: Some(file.speed),
                    peer_free_slots: Some(file.slots),
                };
                match client.download_with_metadata(
                    file.filename.clone(),
                    file.username.clone(),
                    file.size,
                    download_dir.clone(),
                    metadata,
                ) {
                    Ok((download, rx)) => {
                        let _ = sender.send((download, rx));
                    }
                    Err(e) => {
                        eprintln!(
                            "Failed to start download for {}: {}",
                            file.filename, e
                        );
                    }
                }
            }
        });

        // Clear selection
        self.state.results_selected_indices.clear();
    }

    fn update_downloads(&mut self) {
        // Check for new downloads from background thread
        if let Some(ref receiver) = self.state.downloads_receiver_channel {
            while let Ok((download, download_receiver)) = receiver.try_recv() {
                self.state.downloads.push(DownloadEntry {
                    download,
                    receiver: Some(download_receiver),
                });
            }
        }

        // Update download states
        self.state.active_downloads_count = 0;
        for download_entry in &mut self.state.downloads {
            if let Some(ref receiver) = download_entry.receiver {
                while let Ok(status) = receiver.try_recv() {
                    download_entry.download.status = status;
                }
            }

            if matches!(
                download_entry.download.status,
                DownloadStatus::InProgress { .. }
            ) {
                self.state.active_downloads_count += 1;
            }
        }
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

/// A `Rect` centered within `area`, sized to the given percentages.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

const fn clamp_cursor_to_char_boundary(
    input: &str,
    cursor_position: usize,
) -> usize {
    if cursor_position >= input.len() {
        return input.len();
    }

    let mut cursor_position = cursor_position;
    while cursor_position > 0 && !input.is_char_boundary(cursor_position) {
        cursor_position -= 1;
    }
    cursor_position
}

fn previous_char_boundary(input: &str, cursor_position: usize) -> usize {
    let cursor_position = clamp_cursor_to_char_boundary(input, cursor_position);

    input[..cursor_position]
        .char_indices()
        .last()
        .map_or(0, |(index, _)| index)
}

fn next_char_boundary(input: &str, cursor_position: usize) -> usize {
    let cursor_position = clamp_cursor_to_char_boundary(input, cursor_position);

    if cursor_position >= input.len() {
        return input.len();
    }

    cursor_position
        + input[cursor_position..]
            .chars()
            .next()
            .map_or(0, char::len_utf8)
}

fn visible_input_at_cursor(
    input: &str,
    cursor_position: usize,
    width: u16,
) -> (String, u16) {
    if width == 0 {
        return (String::new(), 0);
    }

    let cursor_position = clamp_cursor_to_char_boundary(input, cursor_position);
    let cursor_character_index = input[..cursor_position].chars().count();
    let max_cursor_column = usize::from(width.saturating_sub(1));
    let start_character_index =
        cursor_character_index.saturating_sub(max_cursor_column);

    let visible_input = input
        .chars()
        .skip(start_character_index)
        .take(usize::from(width))
        .collect();
    let cursor_column = cursor_character_index
        .saturating_sub(start_character_index)
        .min(max_cursor_column);

    (visible_input, cursor_column as u16)
}

/// Filter `items` by a case-insensitive substring match on filename or
/// username. Returns the matching items alongside their indices in the original
/// list, so callers can translate a filtered display index back to the
/// unfiltered results. An empty query returns everything (identity mapping).
fn filter_results(
    items: &[FileDisplayData],
    query: &str,
) -> (Vec<FileDisplayData>, Vec<usize>) {
    let query = query.to_lowercase();
    if query.is_empty() {
        return (items.to_vec(), (0..items.len()).collect());
    }

    let mut filtered_items = Vec::new();
    let mut filtered_indices = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        if item.filename.to_lowercase().contains(&query)
            || item.username.to_lowercase().contains(&query)
        {
            filtered_items.push(item.clone());
            filtered_indices.push(idx);
        }
    }
    (filtered_items, filtered_indices)
}

#[cfg(test)]
mod tests {
    use super::filter_results;
    use crate::models::FileDisplayData;

    fn file(filename: &str, username: &str) -> FileDisplayData {
        FileDisplayData {
            filename: filename.to_string(),
            username: username.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_query_returns_identity_mapping() {
        let items = vec![file("a.mp3", "bob"), file("b.flac", "amy")];
        let (filtered, indices) = filter_results(&items, "");
        assert_eq!(filtered.len(), 2);
        assert_eq!(indices, vec![0, 1]);
    }

    #[test]
    fn query_matches_filename_and_username_and_maps_indices() {
        let items = vec![
            file("track.mp3", "bob"),
            file("song.flac", "alice"),
            file("alice_demo.mp3", "carol"),
        ];
        // "alice" matches item 1 (username) and item 2 (filename).
        let (filtered, indices) = filter_results(&items, "alice");
        assert_eq!(filtered.len(), 2);
        assert_eq!(indices, vec![1, 2]);
        assert_eq!(filtered[0].filename, "song.flac");
        assert_eq!(filtered[1].filename, "alice_demo.mp3");
    }

    #[test]
    fn query_is_case_insensitive() {
        let items = vec![file("The Weeknd.mp3", "dj")];
        let (filtered, indices) = filter_results(&items, "WEEKND");
        assert_eq!(filtered.len(), 1);
        assert_eq!(indices, vec![0]);
    }
}
