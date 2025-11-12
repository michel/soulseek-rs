use clap::{Parser, Subcommand};
use color_eyre::Result;
use ratatui::{
    crossterm::event::{self, poll, Event, KeyCode, KeyEvent, KeyEventKind},
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{
        Block, Borders, Cell, Paragraph, Row, StatefulWidget, Table,
        TableState, Wrap,
    },
    DefaultTerminal, Frame,
};
use soulseek_rs::{Client, ClientSettings, DownloadStatus, PeerAddress};
use std::{
    collections::HashSet,
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Parser, Debug)]
#[command(
    name = "soulseek-rs",
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

    // Wrap client in Arc for sharing with FileSelector
    let client = Arc::new(client);

    // Create cancel flag for search
    let cancel_flag = Arc::new(AtomicBool::new(false));

    // Start search in background thread
    let search_client = client.clone();
    let search_query = config.query.clone();
    let search_timeout = Duration::from_secs(config.timeout);
    let search_cancel = cancel_flag.clone();

    let search_handle = std::thread::spawn(move || {
        search_client.search_with_cancel(
            &search_query,
            search_timeout,
            Some(search_cancel),
        )
    });

    // Launch FileSelector immediately with live search enabled
    let terminal = ratatui::init();
    let mut file_selector = FileSelector::new_with_live_search(
        client.clone(),
        config.query.clone(),
        Duration::from_secs(config.timeout),
        cancel_flag.clone(),
    );
    let selected_indices = file_selector.run(terminal)?;
    ratatui::restore();

    // Wait for search thread to complete
    let _ = search_handle.join();

    // Get final results
    let results = client.get_search_results();

    if selected_indices.is_empty() {
        println!("❌ No files selected for download");
        return Ok(());
    }

    // Convert results to all_files format
    let mut all_files: Vec<(String, soulseek_rs::File, u8, u32)> = Vec::new();
    for result in &results {
        for file in &result.files {
            all_files.push((
                result.username.clone(),
                file.clone(),
                result.slots,
                result.speed,
            ));
        }
    }

    if all_files.is_empty() {
        println!("❌ No files found in search results");
        return Ok(());
    }

    println!("\n📋 Found {} file(s)\n", all_files.len());

    println!(
        "\n📥 Starting download of {} file(s)...\n",
        selected_indices.len()
    );

    // Prepare all downloads
    let mut download_states = Vec::new();
    let mut receivers = Vec::new();

    for idx in selected_indices.iter() {
        let (username, file, _, _) = &all_files[*idx];

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
    list_state: TableState,
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
        let mut list_state = TableState::default();
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
        let header = Row::new(vec![
            Cell::from("St"),
            Cell::from("Filename"),
            Cell::from("Username"),
            Cell::from("Size"),
            Cell::from("Progress"),
            Cell::from("%"),
            Cell::from("Speed"),
        ])
        .style(
            Style::default()
                .fg(Color::Cyan)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .height(1);

        let rows: Vec<Row> = self
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

                let progress = if download.total_bytes > 0 {
                    download.bytes_downloaded as f64
                        / download.total_bytes as f64
                } else {
                    0.0
                };

                let bar_width = 20;
                let filled = (progress * bar_width as f64) as usize;
                let empty = bar_width - filled;
                let progress_bar =
                    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty));

                let percent = (progress * 100.0) as u8;
                let percent_str = format!("{}%", percent);

                let size_str = format_bytes_progress(
                    download.bytes_downloaded,
                    download.total_bytes,
                );

                let speed_str = match download.status {
                    DownloadStatus::InProgress { .. } => {
                        format_speed(download.speed_bytes_per_sec)
                    }
                    _ => "-".to_string(),
                };

                let cells = vec![
                    Cell::from(status_icon),
                    Cell::from(download.filename.clone()),
                    Cell::from(download.username.clone()),
                    Cell::from(size_str),
                    Cell::from(progress_bar),
                    Cell::from(percent_str),
                    Cell::from(speed_str),
                ];

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

                Row::new(cells).style(style).height(1)
            })
            .collect();

        let widths = [
            Constraint::Length(3),
            Constraint::Min(30),
            Constraint::Length(15),
            Constraint::Length(20),
            Constraint::Length(22),
            Constraint::Length(5),
            Constraint::Length(12),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL).title("Files"))
            .column_spacing(1)
            .row_highlight_style(
                Style::default()
                    .bg(Color::Magenta)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        StatefulWidget::render(
            table,
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

fn format_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    format!("{:.1} MB", mb)
}

fn format_bytes_progress(downloaded: u64, total: u64) -> String {
    let downloaded_mb = downloaded as f64 / 1_048_576.0;
    let total_mb = total as f64 / 1_048_576.0;
    format!("{:.1}/{:.1} MB", downloaded_mb, total_mb)
}

fn format_speed(speed_bytes_per_sec: f64) -> String {
    let mb = speed_bytes_per_sec / 1_048_576.0;
    format!("{:.1} MB/s", mb)
}

fn get_bitrate(attribs: &std::collections::HashMap<u32, u32>) -> Option<u32> {
    attribs.get(&0).copied()
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

#[derive(Clone)]
struct FileDisplayData {
    filename: String,
    size: u64,
    username: String,
    speed: u32,
    slots: u8,
    bitrate: Option<u32>,
}

struct FileSelector {
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
    fn new_with_live_search(
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

    fn run(&mut self, mut terminal: DefaultTerminal) -> Result<Vec<usize>> {
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
                KeyCode::Char('c') => {
                    // Clear filter
                    self.search_query.clear();
                    self.apply_filter();
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
        } else if !self.search_query.is_empty() {
            format!(
                "Multi-select files to download ({} selected, {} filtered, Space: toggle, a: select-all, A: deselect-all, Enter: download, Esc/q: cancel, /: modify filter, c: clear filter)",
                self.selected_indices.len(),
                self.items.len()
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
        .style(
            Style::default()
                .fg(Color::Cyan)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
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
            .row_highlight_style(
                Style::default()
                    .bg(Color::Magenta)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
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
