use crate::models::{
    AppState, DownloadEntry, FileDisplayData, FocusedPane, SearchEntry,
    SearchStatus,
};
use crate::ui::panes::{
    ResultsPaneParams, render_download_info_pane, render_downloads_pane,
    render_results_pane, render_searches_pane,
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
        while !self.state.should_exit {
            terminal.draw(|frame| self.render(frame))?;

            // Poll for search results updates
            self.update_search_results();

            // Poll for download updates
            self.update_downloads();

            // Update spinner
            self.spinner_state = (self.spinner_state + 1) % 10;

            // Poll for input events
            if poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.handle_key_event(key)?;
                    }
                    Event::Mouse(mouse) => {
                        self.handle_mouse_event(mouse)?;
                    }
                    _ => {}
                }
            }
        }

        // Disable mouse capture before exiting
        use ratatui::crossterm::{event::DisableMouseCapture, execute};
        let _ = execute!(std::io::stdout(), DisableMouseCapture);

        // Restore terminal state
        ratatui::restore();

        soulseek_rs::utils::logger::disable_buffering();
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

        // Render Results pane
        let results_items = if self.state.results_filter_query.is_empty() {
            &self.state.results_items
        } else {
            &self.state.results_filtered_items
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
    }

    fn render_shortcuts(&self, frame: &mut Frame, area: Rect) {
        let shortcuts = if self.state.command_bar_active {
            vec![
                ("Type", "search term"),
                ("←→", "move cursor"),
                ("Backspace/Del", "edit"),
                ("Enter", "search"),
                ("Esc", "cancel"),
            ]
        } else {
            match self.state.focused_pane {
                FocusedPane::Searches => vec![
                    ("s", "search"),
                    ("1-3", "focus pane"),
                    ("↑↓", "navigate"),
                    ("Enter", "view results"),
                    ("d", "remove search"),
                    ("C", "clear all"),
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
                    ("/", "filter"),
                    ("a/A", "select all/none"),
                    ("Enter", "download"),
                    ("1-3", "focus pane"),
                    ("↑↓", "navigate"),
                    ("q", "quit"),
                ],
                FocusedPane::Downloads => {
                    vec![
                        ("1-3", "focus pane"),
                        ("↑↓", "navigate"),
                        ("p", "pause/resume"),
                        ("d", "delete queued"),
                        ("q", "quit"),
                    ]
                }
            }
        };

        let shortcuts_line = format_shortcuts_styled(&shortcuts);
        let shortcuts_widget = Paragraph::new(shortcuts_line).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(border_type(false))
                .title("Shortcuts"),
        );

        frame.render_widget(shortcuts_widget, area);
    }

    fn render_command_bar(&self, frame: &mut Frame, area: Rect) {
        let content_width = area.width.saturating_sub(2);
        let prefix_width = COMMAND_BAR_PREFIX.chars().count() as u16;
        let input_width = content_width.saturating_sub(prefix_width);
        let (visible_input, cursor_column) = visible_input_at_cursor(
            &self.state.command_bar_input,
            self.state.command_bar_cursor_position,
            input_width,
        );
        let command_text = format!("{COMMAND_BAR_PREFIX}{visible_input}");

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

    fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        // Command bar takes priority
        if self.state.command_bar_active {
            return self.handle_command_bar_input(key);
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
                return Ok(());
            }
            KeyCode::Char('1') => {
                self.state.focused_pane = FocusedPane::Searches;
                return Ok(());
            }
            KeyCode::Char('2') => {
                self.state.focused_pane = FocusedPane::Results;
                return Ok(());
            }
            KeyCode::Char('3') => {
                self.state.focused_pane = FocusedPane::Downloads;
                return Ok(());
            }
            KeyCode::Char('s') => {
                self.state.command_bar_active = true;
                self.state.command_bar_input.clear();
                self.state.command_bar_cursor_position = 0;
                return Ok(());
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

    fn handle_command_bar_input(&mut self, key: KeyEvent) -> Result<()> {
        self.state.command_bar_cursor_position = clamp_cursor_to_char_boundary(
            &self.state.command_bar_input,
            self.state.command_bar_cursor_position,
        );

        match key.code {
            KeyCode::Enter => {
                let query = self.state.command_bar_input.trim().to_string();
                if !query.is_empty() {
                    self.start_search(query);
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
        Ok(())
    }

    fn handle_filter_input(&mut self, key: KeyEvent) -> Result<()> {
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
        Ok(())
    }

    fn handle_searches_input(&mut self, key: KeyEvent) -> Result<()> {
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
        Ok(())
    }

    fn handle_results_input(&mut self, key: KeyEvent) -> Result<()> {
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
        Ok(())
    }

    fn handle_downloads_input(&mut self, key: KeyEvent) -> Result<()> {
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
            _ => {}
        }
        Ok(())
    }

    fn toggle_selected_download_pause(&mut self) {
        let Some(index) = self.state.downloads_table_state.selected() else {
            return;
        };
        let Some(download_entry) = self.state.downloads.get(index) else {
            return;
        };

        let download = &download_entry.download;
        match download.status {
            DownloadStatus::InProgress { .. } => {
                self.client
                    .pause_download(&download.username, &download.filename);
            }
            DownloadStatus::Paused { .. } => {
                self.client
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

    fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Result<()> {
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            let (col, row) = (mouse.column, mouse.row);

            // Check if click is within searches pane
            if let Some(area) = self.state.searches_pane_area
                && col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.state.focused_pane = FocusedPane::Searches;
                return Ok(());
            }

            // Check if click is within results pane
            if let Some(area) = self.state.results_pane_area
                && col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.state.focused_pane = FocusedPane::Results;
                return Ok(());
            }

            // Check if click is within downloads pane
            if let Some(area) = self.state.downloads_pane_area
                && col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.state.focused_pane = FocusedPane::Downloads;
                return Ok(());
            }
        }

        Ok(())
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

    fn apply_filter(&mut self) {
        let query = self.state.results_filter_query.to_lowercase();
        if query.is_empty() {
            self.state.results_filtered_items =
                self.state.results_items.clone();
            self.state.results_filtered_indices =
                (0..self.state.results_items.len()).collect();
        } else {
            self.state.results_filtered_items.clear();
            self.state.results_filtered_indices.clear();

            for (idx, item) in self.state.results_items.iter().enumerate() {
                if item.filename.to_lowercase().contains(&query)
                    || item.username.to_lowercase().contains(&query)
                {
                    self.state.results_filtered_items.push(item.clone());
                    self.state.results_filtered_indices.push(idx);
                }
            }
        }

        if !self.state.results_filtered_items.is_empty() {
            self.state.results_table_state.select(Some(0));
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
                    eprintln!("Search failed: {}", e);
                }
            }
        });
    }

    fn update_search_results(&mut self) {
        let timeout = self.search_timeout;
        let selected_search_index = self.state.selected_search_index;

        // Collect all queries first
        let queries: Vec<(usize, String)> = self
            .state
            .searches
            .iter()
            .enumerate()
            .map(|(idx, s)| (idx, s.query.clone()))
            .collect();

        // Fetch all results in one go (single lock acquisition per query)
        // Use try_get_search_results to avoid blocking the UI thread
        let all_results: Vec<(usize, Vec<_>)> = queries
            .into_iter()
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

                    // Update selected search if this is the active one
                    if let Some(selected_idx) = selected_search_index
                        && selected_idx == idx
                    {
                        self.state.results_items = search.results.clone();
                        self.state.results_filtered_items =
                            search.results.clone();
                        self.state.results_filtered_indices =
                            (0..search.results.len()).collect();
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
            for file in selected_files.into_iter() {
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

fn clamp_cursor_to_char_boundary(input: &str, cursor_position: usize) -> usize {
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
        .map(|(index, _)| index)
        .unwrap_or(0)
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
            .map(char::len_utf8)
            .unwrap_or(0)
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
