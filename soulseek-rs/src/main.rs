mod cli;
mod config;
mod directories;
mod models;
mod persist;
mod port_mapping;
mod ui;

use clap::Parser;
use cli::{Cli, Commands, parse_server_address};
use color_eyre::Result;
use config::SearchConfig;
use soulseek_rs::{Client, ClientSettings, PeerAddress};
use std::{
    env,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use ui::{FileSelector, launch_main_tui, show_multi_download_progress};

fn main() -> Result<()> {
    dotenv::dotenv().ok();
    color_eyre::install()?;

    let cli = Cli::parse();

    init_logging(&cli);

    // Layer CLI/env values over config.toml over built-in defaults.
    let config_path = persist::paths::config_file();
    let file_config = match &config_path {
        Some(path) => persist::config::FileConfig::load(path)?,
        None => persist::config::FileConfig::default(),
    };
    let resolved = persist::config::resolve(&cli, &file_config);

    // `portmap` is a local network diagnostic; it needs no server credentials,
    // so handle it before requiring a username/password.
    if matches!(cli.command, Some(Commands::Portmap)) {
        println!(
            "Testing automatic port mapping for TCP {}…",
            resolved.listener_port
        );
        println!("{}", port_mapping::diagnose(resolved.listener_port));
        return Ok(());
    }

    let username = resolved.username.clone().ok_or_else(|| {
        color_eyre::eyre::eyre!(
            "Username required: use --username, set SOULSEEK_USERNAME, or add it to config.toml"
        )
    })?;

    let password = cli.password.clone().ok_or_else(|| {
        color_eyre::eyre::eyre!(
            "Password required: use --password or set SOULSEEK_PASSWORD env var"
        )
    })?;

    let (server_host, server_port) = parse_server_address(&resolved.server)?;

    // Resolve the optional shared/upload directory up front; a misconfigured
    // one is a warning, not a fatal error (the client just shares nothing).
    let shared_directory = match directories::resolve_shared_directory(
        resolved.shared_dir.as_deref(),
    ) {
        Ok(dir) => dir.map(|path| path.display().to_string()),
        Err(e) => {
            eprintln!("⚠️  Ignoring shared directory: {e}");
            None
        }
    };

    let settings = ClientSettings {
        username: username.clone(),
        password: password.clone(),
        server_address: PeerAddress::new(server_host.clone(), server_port),
        enable_listen: !resolved.disable_listener,
        listen_port: resolved.listener_port,
        shared_directory: shared_directory.clone(),
    };

    match cli.command {
        Some(Commands::Search {
            query,
            timeout,
            download_dir,
            max_concurrent_downloads,
        }) => {
            let config = SearchConfig {
                username,
                password,
                server_host,
                server_port,
                enable_listener: !resolved.disable_listener,
                listener_port: resolved.listener_port,
                query,
                timeout,
                download_dir,
                verbose: cli.verbose,
                max_concurrent_downloads,
                shared_directory,
            };
            search_and_download(config)
        }
        Some(Commands::Message {
            username: recipient,
            message,
        }) => send_private_message(&settings, &recipient, &message),
        Some(Commands::Browse { username: target }) => {
            browse_user(&settings, &target)
        }
        Some(Commands::Rooms) => list_rooms(&settings),
        Some(Commands::Chat {
            room,
            message,
            listen_secs,
        }) => chat_room(&settings, &room, message.as_deref(), listen_secs),
        // Handled before the credential check above.
        Some(Commands::Portmap) => unreachable!(),
        None => run_default_tui(
            settings,
            resolved.download_dir.clone(),
            resolved.max_concurrent_downloads,
            Duration::from_secs(resolved.search_timeout),
        ),
    }
}

fn init_logging(cli: &Cli) {
    let log_level = match cli.verbose {
        0 => "ERROR",
        1 => "WARN",
        2 => "INFO",
        3 => "DEBUG",
        _ => "TRACE",
    };
    // SAFETY: Called before any threads are spawned
    unsafe { env::set_var("LOG_LEVEL", log_level) };

    if let Some(log_file) = &cli.log_file {
        // SAFETY: Called before any threads are spawned
        unsafe {
            env::set_var("LOG_FILE", log_file.to_string_lossy().to_string());
        };
    }
}

/// Connect, log in, and run the interactive TUI (the default no-subcommand path).
fn run_default_tui(
    settings: ClientSettings,
    download_dir: String,
    max_concurrent_downloads: usize,
    search_timeout: Duration,
) -> Result<()> {
    use ratatui::crossterm::{
        event::EnableMouseCapture,
        execute,
        terminal::{Clear, ClearType},
    };

    // Enable logger buffering BEFORE connection to prevent log artifacts
    soulseek_rs::utils::logger::enable_buffering();

    // Best-effort: make ourselves reachable behind a home router so
    // firewalled peers can connect back. Kept alive for the session.
    let _port_mapper = settings
        .enable_listen
        .then(|| port_mapping::PortMapper::spawn(settings.listen_port));

    let mut client = Client::with_settings(settings);
    client
        .connect()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to connect: {}", e))?;
    client
        .login()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to login: {}", e))?;

    let client = Arc::new(client);

    // Clear screen and enable mouse capture before initializing TUI
    let _ =
        execute!(std::io::stdout(), Clear(ClearType::All), EnableMouseCapture);

    let terminal = ratatui::init();

    launch_main_tui(
        terminal,
        client,
        download_dir,
        max_concurrent_downloads,
        search_timeout,
    )
}

fn browse_user(settings: &ClientSettings, target: &str) -> Result<()> {
    use std::time::Instant;

    let _port_mapper = settings
        .enable_listen
        .then(|| port_mapping::PortMapper::spawn(settings.listen_port));
    let mut client = Client::with_settings(settings.clone());
    client
        .connect()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to connect: {}", e))?;
    if !client
        .login()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to login: {}", e))?
    {
        return Err(color_eyre::eyre::eyre!("Login rejected by server"));
    }

    client
        .browse_user(target)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to browse: {}", e))?;

    println!("📂 Requesting shared files from {target}...");
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(directories) = client.take_browse_result(target) {
            if directories.is_empty() {
                println!("({target} shares nothing)");
            }
            for directory in directories {
                println!("\n{}/", directory.name);
                for (name, size) in directory.files {
                    println!("  {name}  ({size} bytes)");
                }
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(color_eyre::eyre::eyre!(
        "Timed out waiting for {target}'s file list"
    ))
}

/// Connect and log in, returning the ready client or a descriptive error.
fn connect_and_login(settings: &ClientSettings) -> Result<Client> {
    let mut client = Client::with_settings(settings.clone());
    client
        .connect()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to connect: {}", e))?;
    if !client
        .login()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to login: {}", e))?
    {
        return Err(color_eyre::eyre::eyre!("Login rejected by server"));
    }
    Ok(client)
}

fn list_rooms(settings: &ClientSettings) -> Result<()> {
    use std::time::Instant;

    let client = connect_and_login(settings)?;
    client
        .request_room_list()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to list rooms: {}", e))?;

    println!("📋 Fetching room list...");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut rooms = client.room_list();
    while rooms.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
        rooms = client.room_list();
    }

    rooms.sort_by(|a, b| {
        b.user_count
            .cmp(&a.user_count)
            .then_with(|| a.name.cmp(&b.name))
    });
    if rooms.is_empty() {
        println!("(no public rooms reported)");
    } else {
        println!("{:>6}  room", "users");
        for room in rooms {
            println!("{:>6}  {}", room.user_count, room.name);
        }
    }
    Ok(())
}

fn chat_room(
    settings: &ClientSettings,
    room: &str,
    message: Option<&str>,
    listen_secs: u64,
) -> Result<()> {
    use soulseek_rs::types::RoomEvent;
    use std::time::Instant;

    let client = connect_and_login(settings)?;
    client
        .join_room(room)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to join room: {}", e))?;

    if let Some(text) = message {
        client
            .say_in_room(room, text)
            .map_err(|e| color_eyre::eyre::eyre!("Failed to say: {}", e))?;
        // Let the server actor flush before we drop the client.
        std::thread::sleep(Duration::from_millis(500));
        println!("💬 {room}: {text}");
        return Ok(());
    }

    println!(
        "💬 Joined {room}. Listening for {listen_secs}s (Ctrl-C to quit)..."
    );
    let deadline = Instant::now() + Duration::from_secs(listen_secs);
    while Instant::now() < deadline {
        for event in client.take_room_events() {
            match event {
                RoomEvent::Message {
                    room: r,
                    username,
                    message,
                } if r == room => println!("<{username}> {message}"),
                RoomEvent::UserJoined { room: r, username } if r == room => {
                    println!("→ {username} joined");
                }
                RoomEvent::UserLeft { room: r, username } if r == room => {
                    println!("← {username} left");
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = client.leave_room(room);
    Ok(())
}

fn send_private_message(
    settings: &ClientSettings,
    recipient: &str,
    message: &str,
) -> Result<()> {
    let mut client = Client::with_settings(settings.clone());
    client
        .connect()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to connect: {}", e))?;
    if !client
        .login()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to login: {}", e))?
    {
        return Err(color_eyre::eyre::eyre!("Login rejected by server"));
    }

    client
        .send_private_message(recipient, message)
        .map_err(|e| {
            color_eyre::eyre::eyre!("Failed to send message: {}", e)
        })?;

    // The send is dispatched asynchronously; give the server actor a moment to
    // flush it to the socket before we drop the client and exit.
    std::thread::sleep(Duration::from_millis(500));

    println!("✉️  Message sent to {recipient}");
    Ok(())
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
        shared_directory: config.shared_directory.clone(),
    };

    let _port_mapper = settings
        .enable_listen
        .then(|| port_mapping::PortMapper::spawn(settings.listen_port));

    let mut client = Client::with_settings(settings);
    client
        .connect()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to connect: {}", e))?;
    client
        .login()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to login: {}", e))?;

    if config.verbose > 0 {
        println!("🔍 Searching for: {}", config.query);
    }

    // Wrap client in Arc for sharing with FileSelector
    let client = Arc::new(client);

    let cancel_flag = Arc::new(AtomicBool::new(false));

    let search_client = client.clone();
    let search_query = config.query.clone();
    let search_timeout = Duration::from_secs(config.timeout);
    let search_cancel = cancel_flag.clone();

    let _search_handle = std::thread::spawn(move || {
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
    let (terminal, selected_indices) = file_selector.run(terminal)?;

    // Cancel search thread - no need to wait for it
    cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed);

    let results = client.get_search_results(&config.query);

    if selected_indices.is_empty() {
        ratatui::restore();
        println!("❌ No files selected for download");
        return Ok(());
    }

    // Directly map selected indices to file data (skip expensive all_files conversion)
    let selected_files: Vec<_> = selected_indices
        .iter()
        .filter_map(|&idx| {
            let mut current = 0;
            for result in &results {
                let next = current + result.files.len();
                if idx < next {
                    let file_idx = idx - current;
                    let file = &result.files[file_idx];
                    return Some((
                        file.name.clone(),
                        result.username.clone(),
                        file.size,
                    ));
                }
                current = next;
            }
            None
        })
        .collect();

    if selected_files.is_empty() {
        ratatui::restore();
        println!("❌ No files found in search results");
        return Ok(());
    }

    // Show multi-download progress view immediately (initializes downloads asynchronously)
    show_multi_download_progress(
        terminal,
        client,
        selected_files,
        config.download_dir,
        config.max_concurrent_downloads,
    )?;

    println!("\n✨ Download complete!");

    Ok(())
}
