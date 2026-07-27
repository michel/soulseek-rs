//! Wiring the command line to a session: which config, which credentials, and
//! whether this run opens a session of its own or borrows a daemon's.

use crate::cli::{
    Cli, Commands, ConfigCommand, DaemonCommand, SharesCommand, WishCommand,
    parse_server_address,
};
use crate::commands::Ctx;
use crate::output::{CliError, CliResult, Exit, Out};
use soulseek_rs::{ClientSettings, ClientVersion, PeerAddress};
use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::{env, io};
use crate::ui::launch_main_tui;

pub fn run(mut cli: Cli, out: &Out) -> CliResult {
    let config_path = config_path(&cli);
    let file_config = match &config_path {
        Some(path) => crate::persist::config::FileConfig::load(path)
            .map_err(|e| CliError::usage(e.to_string()))?,
        None => crate::persist::config::FileConfig::default(),
    };
    let resolved = crate::persist::config::resolve(&cli, &file_config);

    let Some(command) = cli.command.take() else {
        // A running daemon already holds a session; the TUI attaches to it
        // rather than logging in a second time, which the server would answer
        // by cutting the daemon off.
        if let Some(endpoint) = daemon_endpoint(&cli) {
            return run_attached_tui(&cli, &resolved, &endpoint);
        }
        return run_default_tui(&cli, &resolved, config_path, &file_config);
    };

    // Anything that only reads the network stack or the config file runs
    // before credentials are demanded: asking a script for a login it does not
    // need would be a poor trade for a `config get`.
    let store = crate::commands::settings::Store {
        path: config_path,
        config: file_config,
    };
    match command {
        Commands::Portmap => {
            return crate::commands::portmap(out, resolved.listener_port);
        }
        Commands::Skills(ref command) => {
            return crate::commands::skills::run(out, command);
        }
        Commands::Completions(ref command) => {
            return crate::commands::completions::run(out, command);
        }
        Commands::Config(ConfigCommand::Path) => {
            return crate::commands::settings::config_path(out, &store);
        }
        Commands::Config(ConfigCommand::List) => {
            crate::commands::settings::config_list(out, &store);
            return Ok(());
        }
        Commands::Config(ConfigCommand::Get { ref key }) => {
            return crate::commands::settings::config_get(out, &store, key);
        }
        Commands::Config(ConfigCommand::Set { ref key, ref value }) => {
            return crate::commands::settings::config_set(out, &store, key, value);
        }
        Commands::Shares(SharesCommand::List) => {
            return crate::commands::settings::shares_list(
                out,
                &store,
                &resolved.shared_dirs,
            );
        }
        Commands::Shares(SharesCommand::Add { ref directory }) => {
            return crate::commands::settings::shares_add(out, &store, directory);
        }
        Commands::Shares(SharesCommand::Remove { ref directory }) => {
            return crate::commands::settings::shares_remove(out, &store, directory);
        }
        Commands::Wish(WishCommand::Add { ref query }) => {
            return crate::commands::wish::add(out, &store, query);
        }
        Commands::Wish(WishCommand::Remove { ref query }) => {
            return crate::commands::wish::remove(out, &store, query);
        }
        Commands::Wish(WishCommand::List) => {
            return crate::commands::wish::list(out, &store);
        }
        // Controlling a daemon needs no account of our own: the daemon is
        // already logged in, and demanding credentials a script never had
        // would turn "is it running?" into a configuration error.
        Commands::Daemon(crate::cli::DaemonArgs {
            command: Some(ref command),
            ..
        }) => {
            let endpoint = daemon_endpoint(&cli);
            let token = cli.daemon_token.clone();
            return match command {
                DaemonCommand::Token => crate::commands::daemon::token(out),
                DaemonCommand::Status => {
                    crate::commands::daemon::status(out, endpoint.as_ref(), token)
                }
                DaemonCommand::Stop => {
                    crate::commands::daemon::stop(out, endpoint.as_ref(), token)
                }
            };
        }
        _ => {}
    }

    let ctx = context(&cli, &resolved, out)?;
    match command {
        Commands::Shares(SharesCommand::Status) => {
            crate::commands::settings::shares_status(&ctx)
        }
        Commands::Shares(SharesCommand::Reindex) => {
            crate::commands::settings::shares_reindex(&ctx)
        }
        Commands::Wish(WishCommand::Run(ref args)) => {
            crate::commands::wish::run(&ctx, &store, args)
        }
        Commands::Daemon(ref args) => {
            crate::daemon::run(&ctx, args.bind.as_deref(), args.upload_slots)
        }
        Commands::Serve(ref args) => {
            let sweeper = args
                .wishlist
                .then(|| crate::commands::wish::Sweeper::new(store.config.wishes()))
                .flatten();
            if args.wishlist && sweeper.is_none() {
                out.warn(
                    "--wishlist has nothing to look for; add one with \
                          `wish add QUERY`",
                );
            }
            crate::commands::peer::serve(&ctx, args, sweeper)
        }
        other => crate::commands::run(&ctx, other),
    }
}

/// Which config file to read: an explicit `--config`, the platform default,
/// or none at all under `--no-config`.
fn config_path(cli: &Cli) -> Option<PathBuf> {
    if cli.no_config {
        return None;
    }
    cli.config.clone().or_else(crate::persist::paths::config_file)
}

/// Assemble everything a one-shot command needs, demanding the credentials
/// the interactive login screen would otherwise ask for.
fn context(
    cli: &Cli,
    resolved: &crate::persist::config::Resolved,
    out: &Out,
) -> CliResult<Ctx> {
    let daemon = daemon_endpoint(cli);
    // A run routed to a daemon needs no account of its own: the daemon is
    // already logged in, and asking for credentials it will not use would
    // stop a script that never had any.
    let (username, password) = if daemon.is_some() {
        (String::new(), String::new())
    } else {
        (
            resolved.username.clone().ok_or_else(|| {
                CliError::usage(
                    "no username: pass --username, set SOULSEEK_USERNAME, or \
                     put one in config.toml",
                )
            })?,
            password(cli, resolved)?.ok_or_else(|| {
                CliError::usage(
                    "no password: pass --password/--password-stdin, set \
                     SOULSEEK_PASSWORD, store one in the OS keychain, or set \
                     password_cmd",
                )
            })?,
        )
    };
    let (host, port) =
        parse_server_address(&resolved.server).map_err(CliError::usage)?;

    // A daemon has its own server and its own filesystem. Asking it once, up
    // front, is what keeps every command that reports a path or a server
    // honest — `whoami` naming the right server, `download` naming the file
    // where the bytes actually landed.
    let described = daemon.as_ref().and_then(|endpoint| {
        describe_daemon(endpoint, cli.daemon_token.clone())
    });
    let server_address = described.as_ref().map_or_else(
        || PeerAddress::new(host, port),
        |status| {
            parse_server_address(&status.server).map_or_else(
                |_| PeerAddress::new(status.server.clone(), 0),
                |(host, port)| PeerAddress::new(host, port),
            )
        },
    );
    let download_dir = described.as_ref().map_or_else(
        || resolved.download_dir.clone(),
        |status| status.download_dir.clone(),
    );

    Ok(Ctx {
        out: out.clone(),
        settings: ClientSettings {
            username,
            password,
            server_address,
            enable_listen: !resolved.disable_listener,
            listen_port: resolved.listener_port,
            shared_directories: shared_directories(resolved, out),
            version: ClientVersion::REFERENCE_CLIENT,
        },
        download_dir,
        max_concurrent_downloads: resolved.max_concurrent_downloads,
        search_timeout: Duration::from_secs(resolved.search_timeout),
        daemon,
        daemon_token: cli.daemon_token.clone(),
        session: std::sync::OnceLock::new(),
    })
}

/// Ask a daemon what it is, so this run describes the session it will actually
/// use. A daemon that cannot be reached is left to `Session::open` to report,
/// which produces a better message than anything this early probe could.
fn describe_daemon(
    endpoint: &crate::remote::Endpoint,
    token: Option<String>,
) -> Option<crate::daemon::proto::DaemonStatus> {
    let token = token.or_else(crate::daemon::stored_token);
    crate::remote::RemoteSession::connect(endpoint, token)
        .ok()?
        .ask_for(crate::daemon::proto::Method::DaemonStatus)
        .ok()
}

/// Which daemon this run should use, if any.
///
/// An address given explicitly always wins. Otherwise a daemon listening on
/// the local socket is used automatically — the convention Docker and
/// ssh-agent set, and what lets an agent run a command with no flags at all.
/// With nothing listening, the run opens its own session exactly as it always
/// has.
fn daemon_endpoint(cli: &Cli) -> Option<crate::remote::Endpoint> {
    if cli.no_daemon {
        return None;
    }
    if let Some(address) = &cli.daemon {
        return Some(crate::remote::Endpoint::parse(address));
    }
    crate::daemon::local_daemon().map(crate::remote::Endpoint::Socket)
}

/// Resolve the password: stdin beats the flag/env, which beats the keychain,
/// which beats `password_cmd`.
fn password(
    cli: &Cli,
    resolved: &crate::persist::config::Resolved,
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
    Ok(crate::persist::secret::resolve_password(
        cli.password.as_deref(),
        resolved.username.as_deref(),
        resolved.password_cmd.as_deref(),
        &crate::persist::secret::KeyringStore,
    ))
}

/// Validate the configured shares. A bad entry is a warning, not a failure:
/// the client simply shares less.
fn shared_directories(
    resolved: &crate::persist::config::Resolved,
    out: &Out,
) -> Vec<String> {
    let mut valid = Vec::new();
    for dir in &resolved.shared_dirs {
        match crate::directories::resolve_shared_directory(Some(dir)) {
            Ok(Some(path)) => valid.push(path.display().to_string()),
            Ok(None) => {}
            Err(e) => out.warn(&format!("ignoring shared directory: {e}")),
        }
    }
    valid
}

/// Decide how much this run says on stderr.
///
/// `-v` raises the level; without it, an explicit `LOG_LEVEL`/`RUST_LOG` from
/// the environment is honoured, and otherwise logging stays at errors only so
/// stderr carries this program's own output and nothing else.
pub fn init_logging(cli: &Cli) {
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
    resolved: &crate::persist::config::Resolved,
    config_path: Option<PathBuf>,
    file_config: &crate::persist::config::FileConfig,
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
    let secret_store = crate::persist::secret::KeyringStore;
    let initial_password = password(cli, resolved)?;

    // Enable logger buffering BEFORE connection to prevent log artifacts
    soulseek_rs::utils::logger::enable_buffering();

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
            version: ClientVersion::REFERENCE_CLIENT,
        };

    // Clear screen and enable mouse capture before initializing TUI
    let _ =
        execute!(std::io::stdout(), Clear(ClearType::All), EnableMouseCapture);
    let mut terminal = ratatui::init();

    let outcome = crate::ui::login::run_login_flow(
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

    // Best-effort: make ourselves reachable behind a home router so firewalled
    // peers can connect back. Mapped only once the listener is up, and for the
    // port it really bound. Kept alive for the session.
    let _port_mapper = outcome
        .client
        .listen_port()
        .map(crate::port_mapping::PortMapper::spawn);

    let store =
        crate::persist::paths::state_dir().map(crate::persist::state::StateStore::new);

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

/// Run the TUI against a daemon's session.
///
/// No login screen: the daemon is already logged in, so there is nothing to
/// ask for and nothing to store. Everything the UI shows comes back over the
/// socket, and what it persists stays daemon-side.
fn run_attached_tui(
    cli: &Cli,
    resolved: &crate::persist::config::Resolved,
    endpoint: &crate::remote::Endpoint,
) -> CliResult {
    use ratatui::crossterm::{
        event::EnableMouseCapture,
        execute,
        terminal::{Clear, ClearType},
    };

    if !std::io::stdout().is_terminal() {
        return Err(CliError::usage(
            "the interactive interface needs a terminal; run a subcommand for \
             scripted use (see --help)",
        ));
    }

    let token = cli
        .daemon_token
        .clone()
        .or_else(crate::daemon::stored_token);
    let session = crate::remote::RemoteSession::connect(endpoint, token)
        .map_err(|e| CliError::connection(format!("cannot use the daemon at {endpoint}: {e}")))?;

    soulseek_rs::utils::logger::enable_buffering();
    let _ =
        execute!(std::io::stdout(), Clear(ClearType::All), EnableMouseCapture);
    let terminal = ratatui::init();

    // No StateStore: the daemon owns what survives a restart, so a second
    // writer here would fight it for the same files.
    launch_main_tui(
        terminal,
        Arc::new(session),
        resolved.download_dir.clone(),
        resolved.max_concurrent_downloads,
        Duration::from_secs(resolved.search_timeout),
        None,
    )
    .map_err(|e| CliError::new(Exit::Failure, e.to_string()))
}

/// After a successful login, remember the username in config.toml and — when
/// it was typed into the form — the password in the OS keychain. Both are
/// best-effort: failing to persist must not break a working session.
fn persist_credentials(
    outcome: &crate::ui::login::LoginOutcome,
    config_path: Option<PathBuf>,
    file_config: &crate::persist::config::FileConfig,
    secret_store: &dyn crate::persist::secret::SecretStore,
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
