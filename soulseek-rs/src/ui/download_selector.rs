use crate::models::FileDisplayData;
use crate::ui::{
    border_style, border_type, format_bytes, format_shortcuts_styled,
    get_bitrate, get_spinner_char, header_style, highlight_style,
    primary_style, success_style, warning_style, HIGHLIGHT_SYMBOL,
};
use color_eyre::Result;
use ratatui::text::{Line, Span};
use ratatui::{
    crossterm::event::{self, poll, Event, KeyCode, KeyEvent, KeyEventKind},
    layout::{Alignment, Constraint, Layout},
    style::Style,
    widgets::{
        Block, Borders, Cell, HighlightSpacing, Paragraph, Row, StatefulWidget,
        Table, TableState, Wrap,
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
    filter_query: String,
    is_filtering: bool,
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
            filter_query: String::new(),
            is_filtering: false,
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

    pub fn run(
        &mut self,
        mut terminal: DefaultTerminal,
    ) -> Result<(DefaultTerminal, Vec<usize>)> {
        while !self.should_exit {
            terminal.draw(|frame| self.render(frame))?;
            self.spinner_state = (self.spinner_state + 1) % 10;

            // Update spinner animation every 80ms
            self.last_spinner_update = Instant::now();

            // Poll for new search results if active
            if self.search_active {
                if let Some(ref client) = self.client {
                    let current_count =
                        client.get_search_results_count(&self.soulseek_query);
                    if current_count != self.last_result_count {
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

            if poll(timeout)?
                && let Event::Key(key) = event::read()? {
                    self.handle_key(key);
                }
        }

        Ok((terminal, self.selected_indices.iter().copied().collect()))
    }

    fn update_results_from_client(&mut self) {
        if let Some(ref client) = self.client {
            let search_results =
                client.get_search_results(&self.soulseek_query);

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
        if let Some(filtered_idx) = self.state.selected()
            && let Some(&original_idx) = self.filtered_indices.get(filtered_idx)
            {
                if self.selected_indices.contains(&original_idx) {
                    self.selected_indices.remove(&original_idx);
                } else {
                    self.selected_indices.insert(original_idx);
                }
            }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }

        if self.is_filtering {
            match key.code {
                KeyCode::Char(' ') => {
                    self.toggle_selection();
                }
                KeyCode::Char(c) => {
                    self.filter_query.push(c);
                    self.apply_filter();
                }
                KeyCode::Backspace => {
                    self.filter_query.pop();
                    self.apply_filter();
                }
                KeyCode::Esc => {
                    self.filter_query.clear();
                    self.apply_filter();
                    self.is_filtering = false;
                }
                KeyCode::Enter => {
                    // If nothing selected, select the current item under cursor
                    if self.selected_indices.is_empty()
                        && let Some(filtered_idx) = self.state.selected()
                            && let Some(&original_idx) =
                                self.filtered_indices.get(filtered_idx)
                            {
                                self.selected_indices.insert(original_idx);
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
                    self.is_filtering = true;
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
                    if self.selected_indices.is_empty()
                        && let Some(filtered_idx) = self.state.selected()
                            && let Some(&original_idx) =
                                self.filtered_indices.get(filtered_idx)
                            {
                                self.selected_indices.insert(original_idx);
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
        let query_lower = self.filter_query.to_lowercase();

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

        // Always split layout into table, optional info, and controls footer
        let has_info = self.is_filtering
            || !self.selected_indices.is_empty()
            || !self.filter_query.is_empty();

        let chunks = if has_info {
            Layout::vertical([
                Constraint::Min(0),
                Constraint::Length(3),
                Constraint::Length(3),
            ])
            .split(area)
        } else {
            Layout::vertical([Constraint::Min(0), Constraint::Length(3)])
                .split(area)
        };

        let table_area = chunks[0];
        let (info_area, controls_area) = if has_info {
            (Some(chunks[1]), chunks[2])
        } else {
            (None, chunks[1])
        };

        let title = if self.search_active {
            // Live Soulseek search is active
            let spinner = get_spinner_char(self.spinner_state);
            let elapsed = self.search_start_time.elapsed().as_secs();
            let total = self.search_timeout.as_secs();
            format!(
                "{} Searching: '{}' - {} results ({}/{}s)",
                spinner,
                self.soulseek_query,
                self.all_items.len(),
                elapsed,
                total
            )
        } else if self.is_filtering {
            format!(
                "Multi-select files to download ({}/{} matches)",
                self.items.len(),
                self.all_items.len()
            )
        } else {
            format!(
                "Multi-select files to download ({} selected)",
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
                    success_style()
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
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style(true))
                    .border_type(border_type(true))
                    .title(title),
            )
            .column_spacing(1)
            .row_highlight_style(highlight_style())
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);

        StatefulWidget::render(
            table,
            table_area,
            frame.buffer_mut(),
            &mut self.state,
        );

        if self.items.is_empty() {
            let spinner = get_spinner_char(self.spinner_state);

            let mut loading_message: Vec<Span> = Vec::new();
            if self.items.is_empty() && self.search_active {
                loading_message.extend(vec![
                    Span::raw(spinner),
                    Span::raw(" Searching for '"),
                    Span::styled(self.soulseek_query.clone(), primary_style()),
                    Span::raw(format!(
                        "' [{}/{}s]",
                        self.search_start_time.elapsed().as_secs(),
                        self.search_timeout.as_secs()
                    )),
                ])
            } else {
                loading_message.extend(vec![
                    Span::raw(spinner),
                    Span::raw(" Searching for '"),
                    Span::styled(self.soulseek_query.clone(), primary_style()),
                    Span::raw("'; No results yet"),
                ]);
            };

            // Center the loading message
            let vertical = Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(3),
                Constraint::Fill(1),
            ])
            .split(table_area);

            let loading_message_line = Line::from(loading_message);
            // Calculate responsive width: text + generous padding for borders and spacing, max 80% of screen
            let text_width =
                loading_message_line.to_string().chars().count() as u16 + 5; // +10 for borders, padding, and safety margin
            let max_width = (table_area.width * 80) / 100;
            let widget_width = text_width.min(max_width);

            let horizontal = Layout::horizontal([
                Constraint::Fill(1),
                Constraint::Length(widget_width),
                Constraint::Fill(1),
            ])
            .split(vertical[1]);

            let loading_widget = Paragraph::new(loading_message_line)
                .style(Style::default())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(primary_style())
                        .border_type(border_type(false)),
                )
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });

            frame.render_widget(loading_widget, horizontal[1]);
        }

        if let Some(info_area) = info_area {
            let (info_text, title, style) = if self.is_filtering {
                (
                    format!("Filter: {}", self.filter_query),
                    "Filter",
                    warning_style(),
                )
            } else if !self.filter_query.is_empty() {
                (
                    format!(
                        "Current filter: {} (press / to modify, Esc to clear)",
                        self.filter_query
                    ),
                    "Filter",
                    primary_style(),
                )
            } else {
                (
                    format!(
                        "{} file(s) selected for download",
                        self.selected_indices.len()
                    ),
                    "Selection",
                    success_style(),
                )
            };

            let info_widget = Paragraph::new(info_text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(border_type(false))
                        .title(title),
                )
                .style(style);

            frame.render_widget(info_widget, info_area);
        }

        // Render controls footer
        self.render_controls(frame, controls_area);
    }

    fn render_controls(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let controls_line = if self.is_filtering {
            format_shortcuts_styled(&[
                ("Space", "toggle"),
                ("a", "select-all"),
                ("A", "deselect-all"),
                ("Enter", "download"),
                ("Esc", "exit filter"),
            ])
        } else if self.search_active {
            format_shortcuts_styled(&[
                ("Space", "toggle"),
                ("a", "select-all"),
                ("A", "deselect-all"),
                ("Enter", "download"),
                ("Esc/q", "cancel"),
            ])
        } else {
            format_shortcuts_styled(&[
                ("Space", "toggle"),
                ("a", "select-all"),
                ("A", "deselect-all"),
                ("Enter", "download"),
                ("Esc/q", "cancel"),
                ("/", "filter"),
            ])
        };

        let controls_widget = Paragraph::new(controls_line).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(border_type(false))
                .title("Controls"),
        );

        frame.render_widget(controls_widget, area);
    }
}
