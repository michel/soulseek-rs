mod cli;
mod config;
mod models;
mod ui;

use clap::Parser;
use cli::{parse_server_address, Cli, Commands};
use color_eyre::Result;
use config::SearchConfig;
use models::FileDownloadState;
use soulseek_rs::{Client, ClientSettings, PeerAddress};
use std::{
    env,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use ui::{show_multi_download_progress, FileSelector};

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
