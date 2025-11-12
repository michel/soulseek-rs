use crate::models::FileDisplayData;
use crate::ui::{format_bytes, get_bitrate, header_style, highlight_style};
use color_eyre::Result;
use ratatui::{
    crossterm::event::{self, poll, Event, KeyCode, KeyEvent, KeyEventKind},
    layout::{Alignment, Constraint, Layout},
    style::{Color, Style},
    widgets::{
        Block, Borders, Cell, Paragraph, Row, StatefulWidget, Table,
        TableState, Wrap,
    },
    DefaultTerminal, Frame,
};
use soulseek_rs::Client;
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub struct FileSelector {
    all_items: Vec<FileDisplayData>,
    items: Vec<FileDisplayData>,
    filtered_indices: Vec<usize>,
    state: TableState,
    should_exit: bool,
    selected_indices: HashSet<usize>,
    search_query: String,
    is_searching: bool,
    client: Option<Arc<Client>>,
    soulseek_query: String,
    search_timeout: Duration,
    search_start_time: Instant,
    search_cancel_flag: Arc<AtomicBool>,
    search_active: bool,
    spinner_state: usize,
    last_spinner_update: Instant,
    last_result_count: usize,
}

impl FileSelector {
    pub fn new_with_live_search(
        client: Arc<Client>,
        query: String,
        timeout: Duration,
        cancel_flag: Arc<AtomicBool>,
    ) -> Self {
        let mut state = TableState::default();
        state.select(Some(0));

        Self {
            all_items: Vec::new(),
            items: Vec::new(),
            filtered_indices: Vec::new(),
            state,
            should_exit: false,
            selected_indices: HashSet::new(),
            search_query: String::new(),
            is_searching: false,
            client: Some(client),
            soulseek_query: query,
            search_timeout: timeout,
            search_start_time: Instant::now(),
            search_cancel_flag: cancel_flag,
            search_active: true,
            spinner_state: 0,
            last_spinner_update: Instant::now(),
            last_result_count: 0,
        }
    }

    pub fn run(&mut self, mut terminal: DefaultTerminal) -> Result<Vec<usize>> {
        while !self.should_exit {
            terminal.draw(|frame| self.render(frame))?;

            // Update spinner animation every 80ms
            if self.search_active
                && self.last_spinner_update.elapsed()
                    >= Duration::from_millis(80)
            {
                self.spinner_state = (self.spinner_state + 1) % 10;
                self.last_spinner_update = Instant::now();
            }

            // Poll for new search results if active
            if self.search_active {
                if let Some(ref client) = self.client {
                    let current_count = client.get_search_results_count();
                    if current_count != self.last_result_count {
                        // New results available - update the list
                        self.update_results_from_client();
                        self.last_result_count = current_count;
                    }
                }

                // Check if search timeout reached
                if self.search_start_time.elapsed() >= self.search_timeout {
                    self.search_active = false;
                    self.search_cancel_flag.store(true, Ordering::Relaxed);
                }
            }

            // Check for keyboard input with timeout for polling
            let timeout = if self.search_active {
                Duration::from_millis(50)
            } else {
                Duration::from_millis(100)
            };

            if poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key);
                }
            }
        }

        Ok(self.selected_indices.iter().copied().collect())
    }

    fn update_results_from_client(&mut self) {
        if let Some(ref client) = self.client {
            let search_results = client.get_search_results();

            // Convert search results to FileDisplayData
            let mut new_items = Vec::new();
            for result in &search_results {
                for file in &result.files {
                    new_items.push(FileDisplayData {
                        filename: file.name.clone(),
                        size: file.size,
                        username: result.username.clone(),
                        speed: result.speed,
                        slots: result.slots,
                        bitrate: get_bitrate(&file.attribs),
                    });
                }
            }

            // Update items and reset state
            let len = new_items.len();
            self.all_items = new_items.clone();
            self.items = new_items;
            self.filtered_indices = (0..len).collect();

            // Keep selection valid or set to 0
            if self.state.selected().is_none() && !self.items.is_empty() {
                self.state.select(Some(0));
            }
        }
    }

    fn toggle_selection(&mut self) {
        if let Some(filtered_idx) = self.state.selected() {
            if let Some(&original_idx) = self.filtered_indices.get(filtered_idx)
            {
                if self.selected_indices.contains(&original_idx) {
                    self.selected_indices.remove(&original_idx);
                } else {
                    self.selected_indices.insert(original_idx);
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }

        if self.is_searching {
            match key.code {
                KeyCode::Char(' ') => {
                    self.toggle_selection();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.apply_filter();
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.apply_filter();
                }
                KeyCode::Esc => {
                    self.is_searching = false;
                }
                KeyCode::Enter => {
                    // If nothing selected, select the current item under cursor
                    if self.selected_indices.is_empty() {
                        if let Some(filtered_idx) = self.state.selected() {
                            if let Some(&original_idx) =
                                self.filtered_indices.get(filtered_idx)
                            {
                                self.selected_indices.insert(original_idx);
                            }
                        }
                    }
                    self.should_exit = true;
                }
                KeyCode::Up | KeyCode::Down => {
                    if key.code == KeyCode::Up {
                        self.select_previous();
                    } else {
                        self.select_next();
                    }
                }
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Char(' ') => {
                    self.toggle_selection();
                }
                KeyCode::Char('/') => {
                    self.is_searching = true;
                }
                KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
                KeyCode::Down | KeyCode::Char('j') => self.select_next(),
                KeyCode::Home | KeyCode::Char('g') => self.select_first(),
                KeyCode::End | KeyCode::Char('G') => self.select_last(),
                KeyCode::Char('a') => {
                    // Select all visible/filtered items
                    for &original_idx in &self.filtered_indices {
                        self.selected_indices.insert(original_idx);
                    }
                }
                KeyCode::Char('A') => {
                    // Deselect all items
                    self.selected_indices.clear();
                }
                KeyCode::Enter => {
                    // If nothing selected, select the current item under cursor
                    if self.selected_indices.is_empty() {
                        if let Some(filtered_idx) = self.state.selected() {
                            if let Some(&original_idx) =
                                self.filtered_indices.get(filtered_idx)
                            {
                                self.selected_indices.insert(original_idx);
                            }
                        }
                    }
                    self.should_exit = true;
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.selected_indices.clear();
                    self.should_exit = true;
                }
                _ => {}
            }
        }
    }

    fn select_previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => self.items.len() - 1,
        };
        self.state.select(Some(i));
    }

    fn select_next(&mut self) {
        let i = match self.state.selected() {
            Some(i) if i < self.items.len() - 1 => i + 1,
            _ => 0,
        };
        self.state.select(Some(i));
    }

    fn select_first(&mut self) {
        self.state.select(Some(0));
    }

    fn select_last(&mut self) {
        self.state.select(Some(self.items.len() - 1));
    }

    fn apply_filter(&mut self) {
        let query_lower = self.search_query.to_lowercase();

        if query_lower.is_empty() {
            self.items = self.all_items.clone();
            self.filtered_indices = (0..self.all_items.len()).collect();
        } else {
            self.items.clear();
            self.filtered_indices.clear();

            for (i, item) in self.all_items.iter().enumerate() {
                if item.filename.to_lowercase().contains(&query_lower)
                    || item.username.to_lowercase().contains(&query_lower)
                {
                    self.items.push(item.clone());
                    self.filtered_indices.push(i);
                }
            }
        }

        if self.items.is_empty() {
            self.state.select(None);
        } else {
            self.state.select(Some(0));
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();

        let (table_area, info_area) = if self.is_searching
            || !self.selected_indices.is_empty()
            || !self.search_query.is_empty()
        {
            let chunks =
                Layout::vertical([Constraint::Min(0), Constraint::Length(3)])
                    .split(area);
            (chunks[0], Some(chunks[1]))
        } else {
            (area, None)
        };

        let title = if self.search_active {
            // Live Soulseek search is active
            let spinner_chars =
                ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let spinner = spinner_chars[self.spinner_state];
            let elapsed = self.search_start_time.elapsed().as_secs();
            let total = self.search_timeout.as_secs();
            format!(
                "{} Searching: '{}' - {} results ({}/{}s) - Space: toggle, a: select-all, A: deselect-all, Enter: download, Esc/q: cancel",
                spinner,
                self.soulseek_query,
                self.all_items.len(),
                elapsed,
                total
            )
        } else if self.is_searching {
            format!(
                "Multi-select files to download ({}/{} matches, Space: toggle, a: select-all, A: deselect-all, Enter: download, Esc: exit search)",
                self.items.len(),
                self.all_items.len()
            )
        } else {
            format!(
                "Multi-select files to download ({} selected, Space: toggle, a: select-all, A: deselect-all, Enter: download, Esc/q: cancel, /: search)",
                self.selected_indices.len()
            )
        };

        let header = Row::new(vec![
            Cell::from(""),
            Cell::from("Filename"),
            Cell::from("Size"),
            Cell::from("Username"),
            Cell::from("Speed"),
            Cell::from("Slots"),
            Cell::from("Bitrate"),
        ])
        .style(header_style())
        .height(1);

        let rows: Vec<Row> = self
            .items
            .iter()
            .enumerate()
            .map(|(filtered_idx, item)| {
                let original_idx = self.filtered_indices[filtered_idx];
                let is_selected = self.selected_indices.contains(&original_idx);
                let checkbox = if is_selected { "[✓]" } else { "[ ]" };

                let speed_mb = item.speed as f64 / 1_048_576.0;
                let speed_str = if speed_mb > 0.0 {
                    format!("{:.1} MB/s", speed_mb)
                } else {
                    "-".to_string()
                };

                let slots_str = format!("{}", item.slots);

                let bitrate_str = match item.bitrate {
                    Some(br) => format!("{} kbps", br),
                    None => "-".to_string(),
                };

                let cells = vec![
                    Cell::from(checkbox),
                    Cell::from(item.filename.clone()),
                    Cell::from(format_bytes(item.size)),
                    Cell::from(item.username.clone()),
                    Cell::from(speed_str),
                    Cell::from(slots_str),
                    Cell::from(bitrate_str),
                ];

                let style = if is_selected {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };

                Row::new(cells).style(style).height(1)
            })
            .collect();

        let widths = [
            Constraint::Length(5),
            Constraint::Min(30),
            Constraint::Length(12),
            Constraint::Length(15),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Length(10),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL).title(title))
            .column_spacing(1)
            .row_highlight_style(highlight_style())
            .highlight_symbol("> ");

        StatefulWidget::render(
            table,
            table_area,
            frame.buffer_mut(),
            &mut self.state,
        );

        // Show loading placeholder when search is active but no results yet
        if self.search_active && self.items.is_empty() {
            let spinner_chars =
                ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let spinner = spinner_chars[self.spinner_state];

            let loading_text =
                format!("{} Searching for '{}'", spinner, self.soulseek_query);

            // Center the loading message
            let vertical = Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(3),
                Constraint::Fill(1),
            ])
            .split(table_area);

            let horizontal = Layout::horizontal([
                Constraint::Fill(1),
                Constraint::Max(80),
                Constraint::Fill(1),
            ])
            .split(vertical[1]);

            let loading_widget = Paragraph::new(loading_text)
                .style(Style::default().fg(Color::Cyan))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });

            frame.render_widget(loading_widget, horizontal[1]);
        }

        if let Some(info_area) = info_area {
            let (info_text, title, color) = if self.is_searching {
                (
                    format!("Search: {}", self.search_query),
                    "Filter",
                    Color::Yellow,
                )
            } else if !self.search_query.is_empty() {
                (
                    format!(
                        "Current filter: {} (press / to modify, c to clear)",
                        self.search_query
                    ),
                    "Filter",
                    Color::Cyan,
                )
            } else {
                (
                    format!(
                        "{} file(s) selected for download",
                        self.selected_indices.len()
                    ),
                    "Selection",
                    Color::Green,
                )
            };

            let info_widget = Paragraph::new(info_text)
                .block(Block::default().borders(Borders::ALL).title(title))
                .style(Style::default().fg(color));

            frame.render_widget(info_widget, info_area);
        }
    }
}
