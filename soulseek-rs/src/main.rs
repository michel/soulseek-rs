use clap::{Parser, Subcommand};
use color_eyre::Result;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    style::{Color, Modifier, Style},
    widgets::{
        Block, Borders, HighlightSpacing, List, ListItem, ListState,
        StatefulWidget,
    },
    DefaultTerminal, Frame,
};
use soulseek_rs::{Client, ClientSettings, PeerAddress};
use std::{env, time::Duration};

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
        } => search_and_download(
            &username,
            &password,
            &server_host,
            server_port,
            !cli.disable_listener,
            cli.listener_port,
            &query,
            timeout,
            &download_dir,
            cli.verbose,
        ),
    }
}

fn search_and_download(
    username: &str,
    password: &str,
    server_host: &str,
    server_port: u16,
    enable_listener: bool,
    listener_port: u32,
    query: &str,
    timeout: u64,
    download_dir: &str,
    verbose: u8,
) -> Result<()> {
    if verbose > 0 {
        println!(
            "🔌 Connecting to Soulseek server {}:{}...",
            server_host, server_port
        );
    }

    let settings = ClientSettings {
        username: username.to_string(),
        password: password.to_string(),
        server_address: PeerAddress::new(server_host.to_string(), server_port),
        enable_listen: enable_listener,
        listen_port: listener_port,
    };

    let mut client = Client::with_settings(settings);
    client.connect();
    client
        .login()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to login: {}", e))?;

    if verbose > 0 {
        println!("🔍 Searching for: {}", query);
    }

    let results = client
        .search(query, Duration::from_secs(timeout))
        .map_err(|e| color_eyre::eyre::eyre!("Search failed: {}", e))?;

    if results.is_empty() {
        println!("❌ No results found for '{}'", query);
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
    let selection = show_file_selector(options)?;

    match selection {
        None => {
            println!("❌ Cancelled");
            return Ok(());
        }
        Some(idx) if idx == all_files.len() => {
            println!("❌ Cancelled");
            return Ok(());
        }
        Some(idx) => {
            let (username, file) = &all_files[idx];
            download_file(&mut client, username, file, download_dir)?;
        }
    }

    Ok(())
}

fn download_file(
    client: &mut Client,
    username: &str,
    file: &soulseek_rs::File,
    download_dir: &str,
) -> Result<()> {
    println!(
        "⬇️  Downloading: {} ({:.2} MB)",
        file.name,
        file.size as f64 / 1_048_576.0
    );

    client
        .download(
            file.name.clone(),
            username.to_string(),
            file.size,
            download_dir.to_string(),
        )
        .map_err(|e| {
            color_eyre::eyre::eyre!("Failed to download {}: {}", file.name, e)
        })?;

    let filename = file
        .name
        .split(['/', '\\'])
        .next_back()
        .unwrap_or(&file.name);
    let download_path = format!("{}/{}", download_dir, filename);

    println!("✅ Downloaded: {} → {}", file.name, download_path);
    Ok(())
}

struct FileSelector {
    items: Vec<String>,
    state: ListState,
    should_exit: bool,
    selected_index: Option<usize>,
}

impl FileSelector {
    fn new(items: Vec<String>) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));

        Self {
            items,
            state,
            should_exit: false,
            selected_index: None,
        }
    }

    fn run(&mut self, mut terminal: DefaultTerminal) -> Result<Option<usize>> {
        while !self.should_exit {
            terminal.draw(|frame| self.render(frame))?;

            if let Event::Key(key) = event::read()? {
                self.handle_key(key);
            }
        }

        Ok(self.selected_index)
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
            KeyCode::Enter => {
                self.selected_index = self.state.selected();
                self.should_exit = true;
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.selected_index = None;
                self.should_exit = true;
            }
            _ => {}
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

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();

        let list_items: Vec<ListItem> = self
            .items
            .iter()
            .map(|item| ListItem::new(item.as_str()))
            .collect();

        let list = List::new(list_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Select a file to download (↑↓/jk: navigate, Enter: select, Esc/q: cancel)")
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            )
            .highlight_symbol("> ")
            .highlight_spacing(HighlightSpacing::Always);

        StatefulWidget::render(list, area, frame.buffer_mut(), &mut self.state);
    }
}

fn show_file_selector(options: Vec<String>) -> Result<Option<usize>> {
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
