mod cli;
mod commands;
mod directories;
mod models;
mod output;
mod persist;
mod port_mapping;
mod ui;

use clap::Parser;
use cli::{Cli, Commands, ConfigCommand, SharesCommand, parse_server_address};
use commands::Ctx;
use output::{CliError, CliResult, Exit, Out};
use soulseek_rs::{ClientSettings, PeerAddress};
use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;
use std::{env, io};
use ui::launch_main_tui;

fn main() -> ExitCode {
    dotenv::dotenv().ok();
    let _ = color_eyre::install();

    let cli = Cli::parse();
    init_logging(&cli);

    let out = Out::new(cli.json, cli.quiet);
    match run(cli, &out) {
        Ok(()) => Exit::Ok.into(),
        Err(error) => {
            out.error(&error.message);
            error.exit.into()
        }
    }
}

fn run(mut cli: Cli, out: &Out) -> CliResult {
    let config_path = config_path(&cli);
    let file_config = match &config_path {
        Some(path) => persist::config::FileConfig::load(path)
            .map_err(|e| CliError::usage(e.to_string()))?,
        None => persist::config::FileConfig::default(),
    };
    let resolved = persist::config::resolve(&cli, &file_config);

    let Some(command) = cli.command.take() else {
        return run_default_tui(&cli, &resolved, config_path, &file_config);
    };

    // Anything that only reads the network stack or the config file runs
    // before credentials are demanded: asking a script for a login it does not
    // need would be a poor trade for a `config get`.
    let store = commands::settings::Store {
        path: config_path,
        config: file_config,
    };
    match command {
        Commands::Portmap => {
            return commands::portmap(out, resolved.listener_port);
        }
        Commands::Config(ConfigCommand::Path) => {
            return commands::settings::config_path(out, &store);
        }
        Commands::Config(ConfigCommand::List) => {
            commands::settings::config_list(out, &store);
            return Ok(());
        }
        Commands::Config(ConfigCommand::Get { ref key }) => {
            return commands::settings::config_get(out, &store, key);
        }
        Commands::Config(ConfigCommand::Set { ref key, ref value }) => {
            return commands::settings::config_set(out, &store, key, value);
        }
        Commands::Shares(SharesCommand::List) => {
            return commands::settings::shares_list(
                out,
                &store,
                &resolved.shared_dirs,
            );
        }
        Commands::Shares(SharesCommand::Add { ref directory }) => {
            return commands::settings::shares_add(out, &store, directory);
        }
        Commands::Shares(SharesCommand::Remove { ref directory }) => {
            return commands::settings::shares_remove(out, &store, directory);
        }
        _ => {}
    }

    let ctx = context(&cli, &resolved, out)?;
    match command {
        Commands::Shares(SharesCommand::Status) => {
            commands::settings::shares_status(&ctx)
        }
        Commands::Shares(SharesCommand::Reindex) => {
            commands::settings::shares_reindex(&ctx)
        }
        other => commands::run(&ctx, other),
    }
}

/// Which config file to read: an explicit `--config`, the platform default,
/// or none at all under `--no-config`.
fn config_path(cli: &Cli) -> Option<PathBuf> {
    if cli.no_config {
        return None;
    }
    cli.config.clone().or_else(persist::paths::config_file)
}

/// Assemble everything a one-shot command needs, demanding the credentials
/// the interactive login screen would otherwise ask for.
fn context(
    cli: &Cli,
    resolved: &persist::config::Resolved,
    out: &Out,
) -> CliResult<Ctx> {
    let username = resolved.username.clone().ok_or_else(|| {
        CliError::usage(
            "no username: pass --username, set SOULSEEK_USERNAME, or put one \
             in config.toml",
        )
    })?;
    let password = password(cli, resolved)?.ok_or_else(|| {
        CliError::usage(
            "no password: pass --password/--password-stdin, set \
             SOULSEEK_PASSWORD, store one in the OS keychain, or set \
             password_cmd",
        )
    })?;
    let (host, port) =
        parse_server_address(&resolved.server).map_err(CliError::usage)?;

    Ok(Ctx {
        out: out.clone(),
        settings: ClientSettings {
            username,
            password,
            server_address: PeerAddress::new(host, port),
            enable_listen: !resolved.disable_listener,
            listen_port: resolved.listener_port,
            shared_directories: shared_directories(resolved, out),
        },
        download_dir: resolved.download_dir.clone(),
        max_concurrent_downloads: resolved.max_concurrent_downloads,
        search_timeout: Duration::from_secs(resolved.search_timeout),
    })
}

/// Resolve the password: stdin beats the flag/env, which beats the keychain,
/// which beats `password_cmd`.
fn password(
    cli: &Cli,
    resolved: &persist::config::Resolved,
) -> CliResult<Option<String>> {
    if cli.password_stdin {
        let mut line = String::new();
        io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| CliError::usage(format!("cannot read stdin: {e}")))?;
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return Err(CliError::usage("no password on stdin"));
        }
        return Ok(Some(line.to_string()));
    }
    Ok(persist::secret::resolve_password(
        cli.password.as_deref(),
        resolved.username.as_deref(),
        resolved.password_cmd.as_deref(),
        &persist::secret::KeyringStore,
    ))
}

/// Validate the configured shares. A bad entry is a warning, not a failure:
/// the client simply shares less.
fn shared_directories(
    resolved: &persist::config::Resolved,
    out: &Out,
) -> Vec<String> {
    let mut valid = Vec::new();
    for dir in &resolved.shared_dirs {
        match directories::resolve_shared_directory(Some(dir)) {
            Ok(Some(path)) => valid.push(path.display().to_string()),
            Ok(None) => {}
            Err(e) => out.warn(&format!("ignoring shared directory: {e}")),
        }
    }
    valid
}

/// `-v` raises the level; without it, an explicit `LOG_LEVEL`/`RUST_LOG` from
/// the environment is honoured, and otherwise logging stays at errors only so
/// stderr carries this program's own output and nothing else.
fn init_logging(cli: &Cli) {
    let level = match cli.verbose {
        0 => (env::var_os("LOG_LEVEL").is_none()
            && env::var_os("RUST_LOG").is_none())
        .then_some("ERROR"),
        1 => Some("WARN"),
        2 => Some("INFO"),
        3 => Some("DEBUG"),
        _ => Some("TRACE"),
    };
    // SAFETY: called before any threads are spawned.
    unsafe {
        if let Some(level) = level {
            env::set_var("LOG_LEVEL", level);
        }
        if let Some(log_file) = &cli.log_file {
            env::set_var("LOG_FILE", log_file.as_os_str());
        }
    }
}

/// Run the interactive TUI (the default no-subcommand path): bring the
/// terminal up first, run the login/registration screen (skipped past when
/// stored credentials work), persist whatever logged in, then enter the
/// main UI.
fn run_default_tui(
    cli: &Cli,
    resolved: &persist::config::Resolved,
    config_path: Option<PathBuf>,
    file_config: &persist::config::FileConfig,
) -> CliResult {
    use ratatui::crossterm::{
        event::EnableMouseCapture,
        execute,
        terminal::{Clear, ClearType},
    };

    // Without a terminal the TUI would panic on its first draw. A script that
    // reached this point wanted a subcommand, so say that instead of crashing.
    if !std::io::stdout().is_terminal() {
        return Err(CliError::usage(
            "the interactive interface needs a terminal; run a subcommand for \
             scripted use (see --help)",
        ));
    }

    let out = Out::new(false, false);
    let (server_host, server_port) =
        parse_server_address(&resolved.server).map_err(CliError::usage)?;

    // Make sure the download folder exists up front: first-run defaults point
    // download and shared at the same fresh Downloads/Soulseek folder, and
    // shared-directory validation requires an existing path.
    if let Err(e) = std::fs::create_dir_all(&resolved.download_dir) {
        out.warn(&format!(
            "could not create download directory {}: {e}",
            resolved.download_dir
        ));
    }

    let shared_directories = shared_directories(resolved, &out);
    let secret_store = persist::secret::KeyringStore;
    let initial_password = password(cli, resolved)?;

    // Enable logger buffering BEFORE connection to prevent log artifacts
    soulseek_rs::utils::logger::enable_buffering();

    // Best-effort: make ourselves reachable behind a home router so
    // firewalled peers can connect back. Kept alive for the session.
    let _port_mapper = (!resolved.disable_listener)
        .then(|| port_mapping::PortMapper::spawn(resolved.listener_port));

    let enable_listen = !resolved.disable_listener;
    let listen_port = resolved.listener_port;
    let make_settings =
        move |username: String, password: String| ClientSettings {
            username,
            password,
            server_address: PeerAddress::new(server_host.clone(), server_port),
            enable_listen,
            listen_port,
            shared_directories: shared_directories.clone(),
        };

    // Clear screen and enable mouse capture before initializing TUI
    let _ =
        execute!(std::io::stdout(), Clear(ClearType::All), EnableMouseCapture);
    let mut terminal = ratatui::init();

    let outcome = ui::login::run_login_flow(
        &mut terminal,
        &make_settings,
        resolved.username.clone(),
        initial_password,
    );

    let outcome = match outcome {
        Ok(Some(outcome)) => outcome,
        Ok(None) => {
            // User cancelled at the login screen.
            ratatui::restore();
            soulseek_rs::utils::logger::disable_buffering();
            return Ok(());
        }
        Err(e) => {
            ratatui::restore();
            soulseek_rs::utils::logger::disable_buffering();
            return Err(CliError::new(Exit::Failure, e.to_string()));
        }
    };

    persist_credentials(&outcome, config_path, file_config, &secret_store);

    let store =
        persist::paths::state_dir().map(persist::state::StateStore::new);

    launch_main_tui(
        terminal,
        Arc::new(outcome.client),
        resolved.download_dir.clone(),
        resolved.max_concurrent_downloads,
        Duration::from_secs(resolved.search_timeout),
        store,
    )
    .map_err(|e| CliError::new(Exit::Failure, e.to_string()))
}

/// After a successful login, remember the username in config.toml and — when
/// it was typed into the form — the password in the OS keychain. Both are
/// best-effort: failing to persist must not break a working session.
fn persist_credentials(
    outcome: &ui::login::LoginOutcome,
    config_path: Option<PathBuf>,
    file_config: &persist::config::FileConfig,
    secret_store: &dyn persist::secret::SecretStore,
) {
    if let Some(path) = config_path
        && file_config.username.as_deref() != Some(&outcome.username)
    {
        let mut updated = file_config.clone();
        updated.username = Some(outcome.username.clone());
        if let Err(e) = updated.save(&path) {
            eprintln!("⚠️  Could not save config: {e}");
        }
    }
    if outcome.entered_via_form
        && let Err(e) = secret_store.set(&outcome.username, &outcome.password)
    {
        eprintln!("⚠️  Could not store password in keychain: {e}");
    }
}
