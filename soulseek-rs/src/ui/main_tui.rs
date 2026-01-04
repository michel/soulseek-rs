use crate::models::{
    AppState, DownloadEntry, FileDisplayData, FocusedPane, SearchEntry,
    SearchStatus,
};
use crate::ui::panes::{
    render_downloads_pane, render_results_pane, render_searches_pane,
    ResultsPaneParams,
};
use crate::ui::{
    border_style, border_type, format_shortcuts_styled, render_download_stats,
};
use color_eyre::Result;
use ratatui::{
    crossterm::event::{
        self, poll, Event, KeyCode, KeyEvent, KeyEventKind, MouseButton,
        MouseEvent, MouseEventKind,
    },
    layout::{Constraint, Layout},
    widgets::{Block, Borders, Paragraph},
    DefaultTerminal, Frame,
};
use soulseek_rs::{Client, DownloadStatus};
use std::{
    sync::{
        atomic::AtomicBool,
        mpsc::{self},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

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

        // Store results and downloads pane areas
        self.state.results_pane_area = Some(right_chunks[0]);
        self.state.downloads_pane_area = Some(right_chunks[1]);

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
            right_chunks[1],
            &self.state.downloads,
            &mut self.state.downloads_table_state,
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

    fn render_shortcuts(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let shortcuts = match self.state.focused_pane {
            FocusedPane::Searches => vec![
                ("s", "search"),
                ("1-3", "focus pane"),
                ("↑↓", "navigate"),
                ("Enter", "view results"),
                ("d", "remove search"),
                ("C", "clear all"),
                ("q", "quit"),
            ],
            FocusedPane::Results if self.state.results_is_filtering => vec![
                ("Type", "filter"),
                ("Esc", "clear filter"),
                ("1-3", "focus pane"),
                ("q", "quit"),
            ],
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
                vec![("1-3", "focus pane"), ("↑↓", "navigate"), ("q", "quit")]
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

    fn render_command_bar(
        &self,
        frame: &mut Frame,
        area: ratatui::layout::Rect,
    ) {
        // Vim-style command bar with ":" prefix
        let command_text = format!("search: {}", self.state.command_bar_input);

        let paragraph = Paragraph::new(command_text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style(true))
                .border_type(border_type(true)),
        );

        frame.render_widget(paragraph, area);
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
        match key.code {
            KeyCode::Enter => {
                let query = self.state.command_bar_input.trim().to_string();
                if !query.is_empty() {
                    self.start_search(query);
                }
                self.state.command_bar_active = false;
                self.state.command_bar_input.clear();
            }
            KeyCode::Esc => {
                self.state.command_bar_active = false;
                self.state.command_bar_input.clear();
            }
            KeyCode::Char(c) => {
                self.state.command_bar_input.push(c);
            }
            KeyCode::Backspace => {
                self.state.command_bar_input.pop();
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
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.state.searches.is_empty() {
                    let current =
                        self.state.searches_table_state.selected().unwrap_or(0);
                    let new = if current == 0 {
                        self.state.searches.len() - 1
                    } else {
                        current - 1
                    };
                    self.state.searches_table_state.select(Some(new));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.state.searches.is_empty() {
                    let current =
                        self.state.searches_table_state.selected().unwrap_or(0);
                    let new = (current + 1) % self.state.searches.len();
                    self.state.searches_table_state.select(Some(new));
                }
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
            KeyCode::Up | KeyCode::Char('k') => {
                if items_count > 0 {
                    let current =
                        self.state.results_table_state.selected().unwrap_or(0);
                    let new = if current == 0 {
                        items_count - 1
                    } else {
                        current - 1
                    };
                    self.state.results_table_state.select(Some(new));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if items_count > 0 {
                    let current =
                        self.state.results_table_state.selected().unwrap_or(0);
                    let new = (current + 1) % items_count;
                    self.state.results_table_state.select(Some(new));
                }
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
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.state.downloads.is_empty() {
                    let current = self
                        .state
                        .downloads_table_state
                        .selected()
                        .unwrap_or(0);
                    let new = if current == 0 {
                        self.state.downloads.len() - 1
                    } else {
                        current - 1
                    };
                    self.state.downloads_table_state.select(Some(new));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.state.downloads.is_empty() {
                    let current = self
                        .state
                        .downloads_table_state
                        .selected()
                        .unwrap_or(0);
                    let new = (current + 1) % self.state.downloads.len();
                    self.state.downloads_table_state.select(Some(new));
                }
            }
            _ => {}
        }
        Ok(())
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
                            });
                        }
                    }

                    // Update selected search if this is the active one
                    if let Some(selected_idx) = selected_search_index
                        && selected_idx == idx {
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
        let selected_files: Vec<(String, String, u64)> = self
            .state
            .results_selected_indices
            .iter()
            .filter_map(|&idx| self.state.results_items.get(idx))
            .map(|file| {
                (file.filename.clone(), file.username.clone(), file.size)
            })
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
            for (filename, username, size) in selected_files.into_iter() {
                match client.download(
                    filename.clone(),
                    username.clone(),
                    size,
                    download_dir.clone(),
                ) {
                    Ok((download, rx)) => {
                        let _ = sender.send((download, rx));
                    }
                    Err(e) => {
                        eprintln!(
                            "Failed to start download for {}: {}",
                            filename, e
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
