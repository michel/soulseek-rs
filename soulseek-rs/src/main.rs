use clap::{Parser, Subcommand};
use color_eyre::Result;
use ratatui::{
    crossterm::event::{self, poll, Event, KeyCode, KeyEvent, KeyEventKind},
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{
        Block, Borders, HighlightSpacing, List, ListItem, ListState, Paragraph,
        StatefulWidget,
    },
    DefaultTerminal, Frame,
};
use soulseek_rs::{Client, ClientSettings, DownloadStatus, PeerAddress};
use std::{collections::HashSet, env, sync::mpsc::Receiver, time::Duration};

#[derive(Parser, Debug)]
#[command(
    name = "soulseek",
    author,
    version,
    about = "Soulseek client in Rust 🦀",
    long_about = None,
    arg_required_else_help = true
)]
struct Cli {
    #[arg(short, long, env = "SOULSEEK_USERNAME")]
    username: Option<String>,

    #[arg(short, long, env = "SOULSEEK_PASSWORD")]
    password: Option<String>,

    #[arg(
        short,
        long,
        env = "SOULSEEK_SERVER",
        default_value = "server.slsknet.org:2416"
    )]
    server: String,

    #[arg(long, env = "DISABLE_LISTENER")]
    disable_listener: bool,

    #[arg(short, long, env = "LISTENER_PORT", default_value = "2234")]
    listener_port: u32,

    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Search {
        query: String,

        #[arg(short, long, default_value = "10")]
        timeout: u64,

        #[arg(short, long, default_value = "~/Downloads")]
        download_dir: String,

        #[arg(
            short = 'c',
            long,
            env = "MAX_CONCURRENT_DOWNLOADS",
            default_value = "5"
        )]
        max_concurrent_downloads: usize,
    },
}

fn parse_server_address(server: &str) -> Result<(String, u16)> {
    let parts: Vec<&str> = server.split(':').collect();
    if parts.len() != 2 {
        return Err(color_eyre::eyre::eyre!(
            "Invalid server format. Expected 'host:port', got '{}'",
            server
        ));
    }
    let host = parts[0].to_string();
    let port = parts[1].parse::<u16>().map_err(|_| {
        color_eyre::eyre::eyre!("Invalid port number: {}", parts[1])
    })?;
    Ok((host, port))
}

struct SearchConfig {
    username: String,
    password: String,
    server_host: String,
    server_port: u16,
    enable_listener: bool,
    listener_port: u32,
    query: String,
    timeout: u64,
    download_dir: String,
    verbose: u8,
    max_concurrent_downloads: usize,
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    let log_level = match cli.verbose {
        0 => "ERROR",
        1 => "WARN",
        2 => "INFO",
        3 => "DEBUG",
        _ => "TRACE",
    };
    env::set_var("LOG_LEVEL", log_level);

    let username = cli.username
        .ok_or_else(|| color_eyre::eyre::eyre!("Username required: use --username or set SOULSEEK_USERNAME env var"))?;

    let password = cli.password
        .ok_or_else(|| color_eyre::eyre::eyre!("Password required: use --password or set SOULSEEK_PASSWORD env var"))?;

    let (server_host, server_port) = parse_server_address(&cli.server)?;

    match cli.command {
        Commands::Search {
            query,
            timeout,
            download_dir,
            max_concurrent_downloads,
        } => {
            let config = SearchConfig {
                username,
                password,
                server_host,
                server_port,
                enable_listener: !cli.disable_listener,
                listener_port: cli.listener_port,
                query,
                timeout,
                download_dir,
                verbose: cli.verbose,
                max_concurrent_downloads,
            };
            search_and_download(config)
        }
    }
}

fn search_and_download(config: SearchConfig) -> Result<()> {
    if config.verbose > 0 {
        println!(
            "🔌 Connecting to Soulseek server {}:{}...",
            config.server_host, config.server_port
        );
    }

    let settings = ClientSettings {
        username: config.username.clone(),
        password: config.password.clone(),
        server_address: PeerAddress::new(
            config.server_host.clone(),
            config.server_port,
        ),
        enable_listen: config.enable_listener,
        listen_port: config.listener_port,
    };

    let mut client = Client::with_settings(settings);
    client.connect();
    client
        .login()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to login: {}", e))?;

    if config.verbose > 0 {
        println!("🔍 Searching for: {}", config.query);
    }

    let results = client
        .search(&config.query, Duration::from_secs(config.timeout))
        .map_err(|e| color_eyre::eyre::eyre!("Search failed: {}", e))?;

    if results.is_empty() {
        println!("❌ No results found for '{}'", config.query);
        return Ok(());
    }

    let mut all_files: Vec<(String, &soulseek_rs::File)> = Vec::new();

    for result in &results {
        for file in &result.files {
            all_files.push((result.username.clone(), file));
        }
    }

    if all_files.is_empty() {
        println!("❌ No files found in search results");
        return Ok(());
    }

    println!("\n📋 Found {} file(s)\n", all_files.len());

    let mut options: Vec<String> = all_files
        .iter()
        .enumerate()
        .map(|(i, (username, file))| {
            let size_mb = file.size as f64 / 1_048_576.0;
            format!(
                "{}. {} ({:.2} MB) - from {}",
                i + 1,
                file.name,
                size_mb,
                username
            )
        })
        .collect();

    options.push("❌ Cancel".to_string());

    // Show TUI file selector
    let selections = show_file_selector(options)?;

    if selections.is_empty() {
        println!("❌ Cancelled or no files selected");
        return Ok(());
    }

    // Filter out the "Cancel" option (last index)
    let valid_selections: Vec<usize> = selections
        .into_iter()
        .filter(|&idx| idx < all_files.len())
        .collect();

    if valid_selections.is_empty() {
        println!("❌ No valid files selected");
        return Ok(());
    }

    println!(
        "\n📥 Starting download of {} file(s)...\n",
        valid_selections.len()
    );

    // Prepare all downloads
    let mut download_states = Vec::new();
    let mut receivers = Vec::new();

    for idx in valid_selections.iter() {
        let (username, file) = &all_files[*idx];

        // Initiate download
        let receiver = client
            .download(
                file.name.clone(),
                username.to_string(),
                file.size,
                config.download_dir.clone(),
            )
            .map_err(|e| {
                color_eyre::eyre::eyre!(
                    "Failed to start download {}: {}",
                    file.name,
                    e
                )
            })?;

        // Create download state
        let download_state = FileDownloadState::new(
            file.name.clone(),
            username.to_string(),
            file.size,
        );

        download_states.push(download_state);
        receivers.push(receiver);
    }

    // Show multi-download progress view (handles parallel execution and concurrency)
    show_multi_download_progress(
        download_states,
        receivers,
        config.max_concurrent_downloads,
    )?;

    println!("\n✨ Download complete!");

    Ok(())
}

#[derive(Debug, Clone)]
struct FileDownloadState {
    filename: String,
    #[allow(dead_code)]
    username: String,
    total_bytes: u64,
    bytes_downloaded: u64,
    speed_bytes_per_sec: f64,
    status: DownloadStatus,
}

impl FileDownloadState {
    fn new(filename: String, username: String, total_bytes: u64) -> Self {
        Self {
            filename,
            username,
            total_bytes,
            bytes_downloaded: 0,
            speed_bytes_per_sec: 0.0,
            status: DownloadStatus::Queued,
        }
    }

    fn update_status(&mut self, status: DownloadStatus) {
        match &status {
            DownloadStatus::InProgress {
                bytes_downloaded,
                total_bytes,
                speed_bytes_per_sec,
            } => {
                self.bytes_downloaded = *bytes_downloaded;
                self.total_bytes = *total_bytes;
                self.speed_bytes_per_sec = *speed_bytes_per_sec;
            }
            DownloadStatus::Completed => {
                self.bytes_downloaded = self.total_bytes;
            }
            _ => {}
        }
        self.status = status;
    }

    fn is_finished(&self) -> bool {
        matches!(
            self.status,
            DownloadStatus::Completed
                | DownloadStatus::Failed
                | DownloadStatus::TimedOut
        )
    }
}

struct MultiDownloadProgress {
    downloads: Vec<FileDownloadState>,
    receivers: Vec<Option<Receiver<DownloadStatus>>>,
    list_state: ListState,
    max_concurrent: usize,
    active_count: usize,
    should_exit: bool,
}

impl MultiDownloadProgress {
    fn new(
        downloads: Vec<FileDownloadState>,
        receivers: Vec<Receiver<DownloadStatus>>,
        max_concurrent: usize,
    ) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        let receivers = receivers.into_iter().map(Some).collect();

        Self {
            downloads,
            receivers,
            list_state,
            max_concurrent,
            active_count: 0,
            should_exit: false,
        }
    }

    fn run(&mut self, mut terminal: DefaultTerminal) -> Result<()> {
        // Start initial batch of downloads
        self.start_next_batch();

        loop {
            terminal.draw(|frame| self.render(frame))?;

            // Poll all receivers for status updates
            for i in 0..self.receivers.len() {
                if let Some(receiver) = &self.receivers[i] {
                    if let Ok(status) = receiver.try_recv() {
                        let was_active = !self.downloads[i].is_finished();
                        self.downloads[i].update_status(status);
                        let is_finished = self.downloads[i].is_finished();

                        // If download just finished, decrement active count and start next
                        if was_active && is_finished {
                            self.active_count =
                                self.active_count.saturating_sub(1);
                            self.start_next_batch();
                        }
                    }
                }
            }

            // Handle keyboard input
            if poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key);
                }
            }

            // Exit when user cancels or all downloads finished
            if self.should_exit || self.all_finished() {
                break;
            }
        }

        Ok(())
    }

    fn start_next_batch(&mut self) {
        // Start queued downloads up to max_concurrent limit
        for i in 0..self.downloads.len() {
            if self.active_count >= self.max_concurrent {
                break;
            }

            if matches!(self.downloads[i].status, DownloadStatus::Queued) {
                self.active_count += 1;
                // Download will start automatically since receiver exists
            }
        }
    }

    fn all_finished(&self) -> bool {
        self.downloads.iter().all(|d| d.is_finished())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Home | KeyCode::Char('g') => self.select_first(),
            KeyCode::End | KeyCode::Char('G') => self.select_last(),
            KeyCode::Esc | KeyCode::Char('q') => {
                self.should_exit = true;
            }
            _ => {}
        }
    }

    fn select_previous(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => self.downloads.len().saturating_sub(1),
        };
        self.list_state.select(Some(i));
    }

    fn select_next(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) if i < self.downloads.len().saturating_sub(1) => i + 1,
            _ => 0,
        };
        self.list_state.select(Some(i));
    }

    fn select_first(&mut self) {
        self.list_state.select(Some(0));
    }

    fn select_last(&mut self) {
        if !self.downloads.is_empty() {
            self.list_state.select(Some(self.downloads.len() - 1));
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();

        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

        // Top: Overall stats
        self.render_stats(frame, chunks[0]);

        // Middle: List of downloads
        self.render_downloads_list(frame, chunks[1]);

        // Bottom: Controls
        self.render_controls(frame, chunks[2]);
    }

    fn render_stats(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let completed = self
            .downloads
            .iter()
            .filter(|d| matches!(d.status, DownloadStatus::Completed))
            .count();
        let failed = self
            .downloads
            .iter()
            .filter(|d| {
                matches!(
                    d.status,
                    DownloadStatus::Failed | DownloadStatus::TimedOut
                )
            })
            .count();
        let queued = self
            .downloads
            .iter()
            .filter(|d| matches!(d.status, DownloadStatus::Queued))
            .count();

        let total_downloaded: u64 =
            self.downloads.iter().map(|d| d.bytes_downloaded).sum();
        let total_size: u64 =
            self.downloads.iter().map(|d| d.total_bytes).sum();
        let overall_progress = if total_size > 0 {
            (total_downloaded as f64 / total_size as f64 * 100.0) as u8
        } else {
            0
        };
        let total_speed: f64 = self
            .downloads
            .iter()
            .filter(|d| matches!(d.status, DownloadStatus::InProgress { .. }))
            .map(|d| d.speed_bytes_per_sec)
            .sum();
        let speed_mb = total_speed / 1_048_576.0;

        let stats_text = format!(
            "Downloads: {} active, {} completed, {} failed, {} queued | Overall: {}% • {:.1} MB/s",
            self.active_count, completed, failed, queued, overall_progress, speed_mb
        );

        let stats_widget = Paragraph::new(stats_text)
            .block(Block::default().borders(Borders::ALL).title("Status"))
            .style(Style::default().fg(Color::Cyan));

        frame.render_widget(stats_widget, area);
    }

    fn render_downloads_list(
        &mut self,
        frame: &mut Frame,
        area: ratatui::layout::Rect,
    ) {
        let list_items: Vec<ListItem> = self
            .downloads
            .iter()
            .map(|download| {
                let status_icon = match download.status {
                    DownloadStatus::Queued => "⋯",
                    DownloadStatus::InProgress { .. } => "⧗",
                    DownloadStatus::Completed => "✓",
                    DownloadStatus::Failed => "✗",
                    DownloadStatus::TimedOut => "⏱",
                };

                let progress_bar = render_inline_progress_bar(download);
                let filename = truncate_filename(&download.filename, 40);

                let line = format!(
                    "[{}] {:<40} {}",
                    status_icon, filename, progress_bar
                );

                let style = match download.status {
                    DownloadStatus::Queued => Style::default().fg(Color::Gray),
                    DownloadStatus::InProgress { .. } => {
                        Style::default().fg(Color::Yellow)
                    }
                    DownloadStatus::Completed => {
                        Style::default().fg(Color::Green)
                    }
                    DownloadStatus::Failed | DownloadStatus::TimedOut => {
                        Style::default().fg(Color::Red)
                    }
                };

                ListItem::new(line).style(style)
            })
            .collect();

        let list = List::new(list_items)
            .block(Block::default().borders(Borders::ALL).title("Files"))
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ")
            .highlight_spacing(HighlightSpacing::Always);

        StatefulWidget::render(
            list,
            area,
            frame.buffer_mut(),
            &mut self.list_state,
        );
    }

    fn render_controls(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let controls_text =
            "↑↓/jk: scroll • Home/End: jump • Esc/q: cancel all";

        let controls_widget = Paragraph::new(controls_text)
            .block(Block::default().borders(Borders::ALL).title("Controls"))
            .style(Style::default().fg(Color::DarkGray));

        frame.render_widget(controls_widget, area);
    }
}

fn render_inline_progress_bar(download: &FileDownloadState) -> String {
    let progress = if download.total_bytes > 0 {
        download.bytes_downloaded as f64 / download.total_bytes as f64
    } else {
        0.0
    };

    let bar_width = 20;
    let filled = (progress * bar_width as f64) as usize;
    let empty = bar_width - filled;

    let bar = format!("[{}{}]", "█".repeat(filled), "░".repeat(empty));

    let percent = (progress * 100.0) as u8;

    match download.status {
        DownloadStatus::InProgress { .. } => {
            let speed_mb = download.speed_bytes_per_sec / 1_048_576.0;
            format!("{} {:3}% {:.1}MB/s", bar, percent, speed_mb)
        }
        DownloadStatus::Completed => {
            format!("{} 100% ✓ Complete", bar)
        }
        DownloadStatus::Failed => {
            format!("{} {:3}% ✗ Failed", bar, percent)
        }
        DownloadStatus::TimedOut => {
            format!("{} {:3}% ⏱ Timed Out", bar, percent)
        }
        DownloadStatus::Queued => {
            format!("{} {:3}% ⋯ Queued", bar, percent)
        }
    }
}

fn truncate_filename(filename: &str, max_len: usize) -> String {
    if filename.len() <= max_len {
        return filename.to_string();
    }

    let parts: Vec<&str> = filename.split(['/', '\\']).collect();
    let name = parts.last().unwrap_or(&filename);

    if name.len() <= max_len {
        return name.to_string();
    }

    format!("...{}", &name[name.len() - (max_len - 3)..])
}

fn show_multi_download_progress(
    downloads: Vec<FileDownloadState>,
    receivers: Vec<Receiver<DownloadStatus>>,
    max_concurrent: usize,
) -> Result<()> {
    soulseek_rs::utils::logger::enable_buffering();

    let terminal = ratatui::init();
    let mut progress =
        MultiDownloadProgress::new(downloads, receivers, max_concurrent);
    let result = progress.run(terminal);
    ratatui::restore();

    soulseek_rs::utils::logger::flush_buffered_logs();

    result
}

struct FileSelector {
    all_items: Vec<String>,
    items: Vec<String>,
    filtered_indices: Vec<usize>,
    state: ListState,
    should_exit: bool,
    selected_indices: HashSet<usize>,
    search_query: String,
    is_searching: bool,
}

impl FileSelector {
    fn new(items: Vec<String>) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));

        let len = items.len();
        let filtered_indices: Vec<usize> = (0..len).collect();

        Self {
            all_items: items.clone(),
            items,
            filtered_indices,
            state,
            should_exit: false,
            selected_indices: HashSet::new(),
            search_query: String::new(),
            is_searching: false,
        }
    }

    fn run(&mut self, mut terminal: DefaultTerminal) -> Result<Vec<usize>> {
        while !self.should_exit {
            terminal.draw(|frame| self.render(frame))?;

            if let Event::Key(key) = event::read()? {
                self.handle_key(key);
            }
        }

        Ok(self.selected_indices.iter().copied().collect())
    }

    fn toggle_selection(&mut self) {
        if let Some(filtered_idx) = self.state.selected() {
            if let Some(&original_idx) = self.filtered_indices.get(filtered_idx)
            {
                // Ignore the last item (Cancel option)
                if original_idx < self.all_items.len() - 1 {
                    if self.selected_indices.contains(&original_idx) {
                        self.selected_indices.remove(&original_idx);
                    } else {
                        self.selected_indices.insert(original_idx);
                    }
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
                    self.search_query.clear();
                    self.apply_filter();
                }
                KeyCode::Enter => {
                    // If nothing selected, select the current item under cursor
                    if self.selected_indices.is_empty() {
                        if let Some(filtered_idx) = self.state.selected() {
                            if let Some(&original_idx) =
                                self.filtered_indices.get(filtered_idx)
                            {
                                if original_idx < self.all_items.len() - 1 {
                                    self.selected_indices.insert(original_idx);
                                }
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
                KeyCode::Enter => {
                    // If nothing selected, select the current item under cursor
                    if self.selected_indices.is_empty() {
                        if let Some(filtered_idx) = self.state.selected() {
                            if let Some(&original_idx) =
                                self.filtered_indices.get(filtered_idx)
                            {
                                if original_idx < self.all_items.len() - 1 {
                                    self.selected_indices.insert(original_idx);
                                }
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
                if item.to_lowercase().contains(&query_lower) {
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

        let (list_area, info_area) = if self.is_searching
            || !self.selected_indices.is_empty()
        {
            let chunks =
                Layout::vertical([Constraint::Min(0), Constraint::Length(3)])
                    .split(area);
            (chunks[0], Some(chunks[1]))
        } else {
            (area, None)
        };

        let list_items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .map(|(filtered_idx, item)| {
                let original_idx = self.filtered_indices[filtered_idx];
                let is_selected = self.selected_indices.contains(&original_idx);
                let prefix = if is_selected { "[✓] " } else { "[ ] " };
                let display_text = format!("{}{}", prefix, item);

                if is_selected {
                    ListItem::new(display_text)
                        .style(Style::default().fg(Color::Green))
                } else {
                    ListItem::new(display_text)
                }
            })
            .collect();

        let title = if self.is_searching {
            format!(
                "Multi-select files ({}/{} matches, Space: toggle, Enter: download, Esc: clear search)",
                self.items.len(),
                self.all_items.len()
            )
        } else {
            format!(
                "Multi-select files ({} selected, Space: toggle, Enter: download, Esc/q: cancel)",
                self.selected_indices.len()
            )
        };

        let list = List::new(list_items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ")
            .highlight_spacing(HighlightSpacing::Always);

        StatefulWidget::render(
            list,
            list_area,
            frame.buffer_mut(),
            &mut self.state,
        );

        if let Some(info_area) = info_area {
            let info_text = if self.is_searching {
                format!("Search: {}", self.search_query)
            } else {
                format!(
                    "{} file(s) selected for download",
                    self.selected_indices.len()
                )
            };

            let info_widget = Paragraph::new(info_text)
                .block(Block::default().borders(Borders::ALL).title(
                    if self.is_searching {
                        "Filter"
                    } else {
                        "Selection"
                    },
                ))
                .style(Style::default().fg(if self.is_searching {
                    Color::Yellow
                } else {
                    Color::Green
                }));

            frame.render_widget(info_widget, info_area);
        }
    }
}

fn show_file_selector(options: Vec<String>) -> Result<Vec<usize>> {
    // Enable log buffering before TUI starts
    soulseek_rs::utils::logger::enable_buffering();

    let terminal = ratatui::init();
    let mut selector = FileSelector::new(options);
    let result = selector.run(terminal);
    ratatui::restore();

    // Flush all buffered logs after TUI exits
    soulseek_rs::utils::logger::flush_buffered_logs();

    result
}
