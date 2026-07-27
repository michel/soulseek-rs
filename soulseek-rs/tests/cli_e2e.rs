//! End-to-end tests of the scriptable command surface, driving the real
//! binary the way a script would: arguments in, records on stdout, progress on
//! stderr, a verdict in the exit code.
//!
//! The tests split in two. The first half needs no server and always runs.
//! The second half talks to a real Soulseek server (soulfind) and follows the
//! same server-optional rule as the library suite: with `SOULFIND_BIN` (or a
//! sibling `soulfind/bin/soulfind` checkout) it spawns one, otherwise it skips
//! with a notice. `SOULSEEK_E2E_REQUIRED=1` turns a missing server into a
//! failure, which is how CI guarantees these actually run.

#![allow(clippy::doc_markdown)]

use soulseek_rs::{Client, ClientSettings, PeerAddress};
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_soulseek-rs");

/// Exit codes the command surface promises. Kept as literals so a change to
/// the enum has to be a deliberate change here too.
const EXIT_OK: i32 = 0;
const EXIT_USAGE: i32 = 2;
const EXIT_CONNECTION: i32 = 3;
const EXIT_NO_RESULTS: i32 = 4;
const EXIT_SESSION_LOST: i32 = 7;

/// Every environment variable the CLI reads, cleared for each run so a
/// developer's shell (or a stray `.env`) cannot change what a test observes.
const CLI_ENV_VARS: [&str; 15] = [
    "SOULSEEK_CONFIG_DIR",
    "SOULSEEK_STATE_DIR",
    "SOULSEEK_DAEMON",
    "SOULSEEK_DAEMON_TOKEN",
    "SOULSEEK_USERNAME",
    "SOULSEEK_PASSWORD",
    "SOULSEEK_PASSWORD_CMD",
    "SOULSEEK_SERVER",
    "SOULSEEK_NO_LISTENER",
    "SOULSEEK_LISTENER_PORT",
    "SOULSEEK_DOWNLOAD_DIR",
    "SOULSEEK_SHARED_DIR",
    "SOULSEEK_MAX_CONCURRENT_DOWNLOADS",
    "SOULSEEK_SEARCH_TIMEOUT",
    "SOULSEEK_CONFIG",
];

/// Build a command for the binary, isolated from the developer's environment
/// and from the repository's own working directory.
///
/// The state directory is not merely cleared but pointed somewhere empty: a
/// client with no state directory of its own would look in the real one, find
/// whatever daemon the developer happens to be running, and route the whole
/// suite through their live Soulseek session.
fn command(args: &[&str]) -> Command {
    let mut command = Command::new(BIN);
    for name in CLI_ENV_VARS {
        command.env_remove(name);
    }
    command.env("SOULSEEK_STATE_DIR", isolated_state_dir());
    command.current_dir(std::env::temp_dir());
    command.args(args);
    command
}

/// An empty state directory shared by every run in this test process, so
/// auto-routing finds nothing.
fn isolated_state_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let path = std::env::temp_dir()
            .join(format!("soulseek-cli-e2e-state-{}", std::process::id()));
        std::fs::create_dir_all(&path).expect("state directory");
        path
    })
    .clone()
}

/// Which kind of session the suite drives.
///
/// The point of the switch is that the *same* tests run both ways: identical
/// records, identical exit codes, whether the session is this process's own or
/// a daemon's. CI runs the suite twice, once per mode, and that equivalence is
/// the guarantee daemon mode rests on.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Mode {
    Local,
    Daemon,
}

fn mode() -> Mode {
    match std::env::var("SOULSEEK_E2E_MODE").as_deref() {
        Ok("daemon") => Mode::Daemon,
        _ => Mode::Local,
    }
}

fn run(args: &[&str]) -> Output {
    command(args).output().expect("the binary should run")
}

fn code(output: &Output) -> i32 {
    output
        .status
        .code()
        .expect("the process should not be signalled")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Records are one per line; blank trailing lines are not records.
fn records(output: &Output) -> Vec<String> {
    stdout(output)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(String::from)
        .collect()
}

// ---------------------------------------------------------------------------
// No server required
// ---------------------------------------------------------------------------

#[test]
fn help_lists_every_scriptable_command() {
    let output = run(&["--help"]);
    assert_eq!(code(&output), EXIT_OK);
    let help = stdout(&output);
    for command in [
        "search",
        "download",
        "get",
        "browse",
        "room",
        "message",
        "portmap",
        "skills",
        "completions",
        "wish",
    ] {
        assert!(help.contains(command), "--help should mention {command}");
    }
}

#[test]
fn help_documents_the_exit_codes_a_script_branches_on() {
    let help = stdout(&run(&["--help"]));
    assert!(help.contains("Exit codes"), "help should list exit codes");
    for code in ["2", "3", "4", "5", "6", "7"] {
        assert!(help.contains(code), "exit code {code} should be documented");
    }
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
    let output = run(&["--definitely-not-a-flag"]);
    assert_eq!(code(&output), EXIT_USAGE);
}

#[test]
fn missing_credentials_are_a_usage_error_with_a_pointer_to_the_fix() {
    let output = run(&["--no-config", "search", "anything"]);
    assert_eq!(code(&output), EXIT_USAGE);
    assert!(stderr(&output).contains("SOULSEEK_USERNAME"));
    assert!(stdout(&output).is_empty(), "stdout must stay data-only");
}

#[test]
fn a_falsey_boolean_environment_variable_is_accepted() {
    // `DISABLE_LISTENER=1` used to abort every invocation with a clap parse
    // error, which made the binary unusable from a container.
    for value in ["1", "0", "true", "false", "yes", ""] {
        let output = command(&["--no-config", "search", "anything"])
            .env("SOULSEEK_NO_LISTENER", value)
            .output()
            .expect("the binary should run");
        assert!(
            !stderr(&output).contains("invalid value"),
            "SOULSEEK_NO_LISTENER={value:?} should be accepted"
        );
    }
}

#[test]
fn global_flags_are_accepted_after_the_subcommand() {
    let output = run(&["search", "anything", "--no-config", "--json"]);
    assert_eq!(
        code(&output),
        EXIT_USAGE,
        "should reach the credential check, not a parse error"
    );
    assert!(!stderr(&output).contains("unexpected argument"));
}

#[test]
fn the_password_environment_value_is_not_echoed_in_help() {
    let output = command(&["--help"])
        .env("SOULSEEK_PASSWORD", "hunter2")
        .output()
        .expect("the binary should run");
    assert!(
        !stdout(&output).contains("hunter2"),
        "--help must not print the password it inherited"
    );
}

#[test]
fn download_without_a_target_is_a_usage_error() {
    assert_eq!(code(&run(&["download", "--no-config"])), EXIT_USAGE);
    assert_eq!(code(&run(&["download", "--no-config", "bob"])), EXIT_USAGE);
}

#[test]
fn download_stdin_cannot_be_combined_with_positional_arguments() {
    let output = run(&["download", "--stdin", "bob", "file.mp3"]);
    assert_eq!(code(&output), EXIT_USAGE);
}

#[test]
fn a_malformed_server_address_is_a_usage_error() {
    let output = run(&[
        "--no-config",
        "--username",
        "u",
        "--password",
        "p",
        "--server",
        "no-port-here",
        "room",
        "list",
    ]);
    assert_eq!(code(&output), EXIT_USAGE);
}

#[test]
fn an_unreachable_server_is_a_connection_error() {
    // Port 1 is reserved and nothing listens on it, so the connect fails fast.
    let output = run(&[
        "--no-config",
        "--username",
        "u",
        "--password",
        "p",
        "--server",
        "127.0.0.1:1",
        "--no-listener",
        "room",
        "list",
    ]);
    assert_eq!(code(&output), EXIT_CONNECTION);
    assert!(stdout(&output).is_empty(), "stdout must stay data-only");
}

#[test]
fn without_a_terminal_the_tui_reports_instead_of_panicking() {
    // Captured stdout is not a terminal, which is exactly the cron/CI case.
    let output = run(&["--no-config"]);
    assert_eq!(code(&output), EXIT_USAGE);
    assert!(
        stderr(&output).contains("terminal"),
        "should explain why, got: {}",
        stderr(&output)
    );
}

#[test]
fn portmap_emits_a_record_whose_verdict_matches_the_exit_code() {
    let output = run(&["--no-config", "--quiet", "--json", "portmap"]);
    let record: serde_json::Value =
        serde_json::from_str(records(&output).first().expect("one record"))
            .expect("portmap should emit valid JSON");

    assert!(record["port"].is_number());
    let ok = record["ok"].as_bool().expect("ok should be a boolean");
    // Whether a router answers depends on the network, but the exit code and
    // the record must always agree.
    assert_eq!(
        ok,
        code(&output) == EXIT_OK,
        "exit code and the ok field must agree"
    );
    if !ok {
        assert_eq!(code(&output), EXIT_NO_RESULTS);
    }
}

#[test]
fn quiet_keeps_progress_off_stderr() {
    let loud = run(&["--no-config", "--json", "portmap"]);
    let quiet = run(&["--no-config", "--quiet", "--json", "portmap"]);
    assert!(
        stderr(&quiet).lines().count() < stderr(&loud).lines().count(),
        "--quiet should suppress the progress narration"
    );
    assert_eq!(
        records(&loud).len(),
        records(&quiet).len(),
        "--quiet must not change the records"
    );
}

/// What the first record says happened, parsed rather than matched as text.
fn action(output: &Output) -> String {
    let record: serde_json::Value =
        serde_json::from_str(records(output).first().expect("one record"))
            .expect("valid JSON");
    record["action"]
        .as_str()
        .expect("a record carries an action")
        .to_string()
}

#[test]
fn installing_the_skill_is_repeatable_and_reversible() {
    let target = Scratch::new("skills");
    let dir = target.display();
    let installed = target.path().join("soulseek-rs").join("SKILL.md");

    let first = run(&["--json", "skills", "install", "--dir", &dir]);
    assert_eq!(code(&first), EXIT_OK, "stderr: {}", stderr(&first));
    assert_eq!(action(&first), "installed");
    let written = std::fs::read_to_string(&installed).expect("a skill file");
    assert!(written.contains("name: soulseek-rs"));
    assert!(written.contains("--json"));

    let second = run(&["--json", "skills", "install", "--dir", &dir]);
    assert_eq!(
        action(&second),
        "unchanged",
        "reinstalling is how you update"
    );

    // A neighbour in the same folder proves the sweep removes our file rather
    // than the directory it happened to be sitting in.
    let neighbour = installed.with_file_name("notes.md");
    std::fs::write(&neighbour, "mine").expect("a neighbouring file");

    let removed = run(&["--json", "skills", "uninstall", "--dir", &dir]);
    assert_eq!(action(&removed), "removed");
    assert!(!installed.exists());
    assert!(neighbour.exists(), "a file we did not write must survive");

    let again = run(&["--json", "skills", "uninstall", "--dir", &dir]);
    assert_eq!(code(&again), EXIT_OK, "removing nothing is not a failure");
    assert_eq!(action(&again), "absent");
}

/// A mistyped `--dir` names somebody else's directory far more often than it
/// names one of ours, so uninstall has to leave it standing.
#[test]
fn uninstall_leaves_a_skill_it_did_not_write() {
    let target = Scratch::new("skills-foreign");
    let foreign = target.path().join("soulseek-rs").join("SKILL.md");
    std::fs::create_dir_all(foreign.parent().expect("a parent"))
        .expect("mkdir");
    std::fs::write(&foreign, "---\nname: someone-else\n---\n").expect("write");

    let output =
        run(&["--json", "skills", "uninstall", "--dir", &target.display()]);
    assert_eq!(code(&output), EXIT_OK);
    assert_eq!(action(&output), "absent");
    assert!(foreign.exists(), "a foreign skill must survive");
}

#[test]
fn listing_skills_reports_without_writing_anything() {
    let target = Scratch::new("skills-list");

    let output = run(&["--json", "skills", "list", "--dir", &target.display()]);
    assert_eq!(code(&output), EXIT_OK);
    assert_eq!(action(&output), "absent");
    assert!(
        !target.path().join("soulseek-rs").exists(),
        "list must not create anything"
    );
}

/// fish is the shell the whole round trip can be driven for on every platform:
/// its completions directory hangs off `XDG_CONFIG_HOME`, so nothing outside
/// the scratch directory is read or written. bash and zsh also append a line
/// to a real rc file under `$HOME`, which is not a thing to point a test at on
/// Windows — the unit tests cover that half.
#[test]
fn installing_completions_is_repeatable_and_reversible() {
    let target = Scratch::new("completions");
    let installed = target
        .path()
        .join("fish")
        .join("completions")
        .join("soulseek-rs.fish");
    let completions = |verb: &str| {
        command(&["--json", "completions", verb, "--shell", "fish"])
            .env("XDG_CONFIG_HOME", target.path())
            .output()
            .expect("the binary should run")
    };

    let first = completions("install");
    assert_eq!(code(&first), EXIT_OK, "stderr: {}", stderr(&first));
    assert_eq!(action(&first), "installed");
    let script = std::fs::read_to_string(&installed).expect("a fish script");
    assert!(script.contains("complete -c soulseek-rs"));
    assert!(
        script.contains("-l download-dir"),
        "flags are completed too"
    );

    assert_eq!(
        action(&completions("install")),
        "unchanged",
        "reinstalling is how you update"
    );

    let removed = completions("uninstall");
    assert_eq!(action(&removed), "removed");
    assert!(!installed.exists());

    let again = completions("uninstall");
    assert_eq!(code(&again), EXIT_OK, "removing nothing is not a failure");
    assert_eq!(action(&again), "absent");
}

// --- wishlist, the half that only touches the config file ------------------

#[test]
fn a_wish_is_added_listed_and_removed_through_the_config_file() {
    let dir = Scratch::new("wish");
    let config = dir.path().join("config.toml");

    let added = with_config(&config, &["wish", "add", "aphex twin"]);
    assert_eq!(code(&added), EXIT_OK, "stderr: {}", stderr(&added));
    assert_eq!(records(&added), ["aphex twin"]);

    let listed = with_config(&config, &["wish", "list"]);
    assert_eq!(code(&listed), EXIT_OK);
    assert_eq!(records(&listed), ["aphex twin"]);

    let removed = with_config(&config, &["wish", "remove", "APHEX TWIN"]);
    assert_eq!(code(&removed), EXIT_OK, "stderr: {}", stderr(&removed));
    assert!(stdout(&removed).is_empty(), "remove prints nothing");

    let empty = with_config(&config, &["wish", "list"]);
    assert_eq!(code(&empty), EXIT_NO_RESULTS);
    assert!(stdout(&empty).is_empty(), "stdout must stay data-only");
}

#[test]
fn adding_the_same_wish_twice_is_not_an_error_and_stores_it_once() {
    let dir = Scratch::new("wish");
    let config = dir.path().join("config.toml");

    with_config(&config, &["wish", "add", "boards of canada"]);
    let again = with_config(&config, &["wish", "add", "Boards Of Canada"]);
    assert_eq!(code(&again), EXIT_OK, "a repeat is not a failure");

    let listed = with_config(&config, &["wish", "list"]);
    assert_eq!(records(&listed), ["boards of canada"]);
}

#[test]
fn a_wish_holding_a_comma_survives_the_config_file() {
    let dir = Scratch::new("wish");
    let config = dir.path().join("config.toml");

    with_config(&config, &["wish", "add", "autechre, incunabula"]);
    let listed = with_config(&config, &["--json", "wish", "list"]);
    let stored: Vec<serde_json::Value> = records(&listed)
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    assert_eq!(stored.len(), 1, "one wish, not two split on the comma");
    assert_eq!(stored[0]["query"], "autechre, incunabula");
}

#[test]
fn removing_a_wish_that_is_not_there_is_the_no_results_code() {
    let dir = Scratch::new("wish");
    let config = dir.path().join("config.toml");

    let output = with_config(&config, &["wish", "remove", "never added"]);
    assert_eq!(code(&output), EXIT_NO_RESULTS);
    assert!(stdout(&output).is_empty());
}

#[test]
fn an_empty_wish_query_is_a_usage_error() {
    let dir = Scratch::new("wish");
    let config = dir.path().join("config.toml");

    let output = with_config(&config, &["wish", "add", "   "]);
    assert_eq!(code(&output), EXIT_USAGE);
    assert!(stdout(&output).is_empty());
}

#[test]
fn wish_add_without_a_config_file_to_write_to_is_a_usage_error() {
    let output = run(&["--no-config", "wish", "add", "nowhere to keep this"]);
    assert_eq!(code(&output), EXIT_USAGE);
    assert!(stdout(&output).is_empty());
}

// ---------------------------------------------------------------------------
// Server required (soulfind)
// ---------------------------------------------------------------------------

/// Only one test at a time may drive a server.
///
/// Each of these tests runs a soulfind, one or more real clients, and the
/// binary itself. Letting the harness start a dozen of those at once starves
/// the servers, and a login that goes unanswered fails a test for a reason
/// that has nothing to do with the code under test. Serializing here keeps the
/// suite deterministic whatever `--test-threads` the runner picks.
static SERVER_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A soulfind we spawned for one test, held for that test's lifetime.
struct TestServer {
    host: String,
    port: u16,
    child: Option<Child>,
    db: Option<PathBuf>,
    _gate: std::sync::MutexGuard<'static, ()>,
    /// Daemons started for this server, one per (user, session flags). Dropped
    /// with the server, which kills them.
    daemons: std::sync::Mutex<std::collections::HashMap<String, DaemonFixture>>,
}

/// A `soulseek-rs daemon` running against the test server, with a state
/// directory of its own so its socket and token cannot collide with another
/// fixture's — or with the developer's.
struct DaemonFixture {
    child: Child,
    socket: PathBuf,
    state: PathBuf,
}

impl DaemonFixture {
    fn start(
        server: &TestServer,
        user: &str,
        session_flags: &[String],
    ) -> Self {
        static COUNTER: std::sync::atomic::AtomicU32 =
            std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let state = std::env::temp_dir()
            .join(format!("soulseek-e2e-daemon-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&state).expect("daemon state directory");

        let mut args = vec![
            "--no-config".to_string(),
            "--server".to_string(),
            server.address(),
            "--username".to_string(),
            user.to_string(),
            "--password".to_string(),
            "pw".to_string(),
        ];
        // A listener of its own unless the test asked for something specific:
        // transfers arrive on a connection the uploader opens back to us.
        if !session_flags
            .iter()
            .any(|flag| flag == "--listener-port" || flag == "--no-listener")
        {
            args.push("--listener-port".to_string());
            args.push(free_port().expect("listener port").to_string());
        }
        args.extend(session_flags.iter().cloned());
        args.push("daemon".to_string());

        let mut command = Command::new(BIN);
        for name in CLI_ENV_VARS {
            command.env_remove(name);
        }
        command.env("SOULSEEK_STATE_DIR", &state);
        command.current_dir(std::env::temp_dir());
        command.args(&args);
        command.stdout(Stdio::null()).stderr(Stdio::null());
        let child = command.spawn().expect("the daemon should start");

        let socket = state.join("daemon.sock");
        let fixture = Self {
            child,
            socket,
            state,
        };
        fixture.wait_until_ready();
        fixture
    }

    /// Wait until the socket actually accepts, not merely exists: the daemon
    /// binds before it logs in, so a client that connects too early would be
    /// answering questions about a session that is not there yet.
    fn wait_until_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(45);
        while Instant::now() < deadline {
            #[cfg(unix)]
            if std::os::unix::net::UnixStream::connect(&self.socket).is_ok() {
                // Binding happens first, so also wait for the login to land.
                if self.answers() {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("the daemon never became ready at {}", self.socket.display());
    }

    /// Whether the daemon answers `daemon status` with a session.
    fn answers(&self) -> bool {
        let mut command = Command::new(BIN);
        for name in CLI_ENV_VARS {
            command.env_remove(name);
        }
        command.env("SOULSEEK_STATE_DIR", &self.state);
        command.current_dir(std::env::temp_dir());
        command.args([
            "--no-config",
            "--json",
            "--daemon",
            &self.socket.display().to_string(),
            "daemon",
            "status",
        ]);
        command.output().is_ok_and(|output| output.status.success())
    }
}

impl Drop for DaemonFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.state);
    }
}

impl TestServer {
    fn resolve() -> Option<Self> {
        // A test that panicked while holding the gate poisoned it; the lock
        // guards nothing but scheduling, so take it back and carry on.
        let gate = SERVER_GATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if let Ok(addr) = std::env::var("SOULSEEK_TEST_SERVER") {
            let (host, port) = addr.rsplit_once(':')?;
            let port = port.parse().ok()?;
            wait_until_listening(host, port, Duration::from_secs(2))?;
            return Some(Self {
                host: host.to_string(),
                port,
                child: None,
                db: None,
                _gate: gate,
                daemons: std::sync::Mutex::new(std::collections::HashMap::new()),
            });
        }

        let bin = soulfind_binary()?;
        let port = free_port()?;
        let db = std::env::temp_dir().join(format!("soulfind-cli-{port}.db"));
        let _ = std::fs::remove_file(&db);

        let mut child = Command::new(&bin)
            .arg("-p")
            .arg(port.to_string())
            .arg("-d")
            .arg(&db)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        if wait_until_listening("127.0.0.1", port, Duration::from_secs(5))
            .is_none()
        {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }

        Some(Self {
            host: "127.0.0.1".to_string(),
            port,
            child: Some(child),
            db: Some(db),
            _gate: gate,
            daemons: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Arguments that point the binary at this server as `user`.
    ///
    /// The listener stays on: a Soulseek transfer is delivered over a
    /// connection the *uploader* opens back to us, so a downloader without a
    /// listener never receives its file.
    fn args(&self, user: &str) -> Vec<String> {
        self.args_on(user, free_port().expect("listener port"))
    }

    /// The same arguments on a chosen listener port, for the tests about what
    /// happens when two runs want one port.
    fn args_on(&self, user: &str, port: u16) -> Vec<String> {
        vec![
            "--no-config".to_string(),
            "--quiet".to_string(),
            "--server".to_string(),
            self.address(),
            "--username".to_string(),
            user.to_string(),
            "--password".to_string(),
            "pw".to_string(),
            "--listener-port".to_string(),
            port.to_string(),
        ]
    }

    /// Client arguments pointing at a daemon logged in as `user`, starting one
    /// if this account has not been seen.
    ///
    /// Strictly one daemon per account: the server allows a single session per
    /// name, so a second daemon under the same name would displace the first
    /// and every command already routed to it would start failing with
    /// "session lost". The session flags of the first call configure it — a
    /// later call's are ignored, which is exactly what happens in real daemon
    /// mode, where shares and the listener belong to the daemon.
    fn daemon_args(&self, user: &str, session_flags: &[String]) -> Vec<String> {
        let socket = self.daemon_for(user, session_flags);
        vec![
            "--no-config".to_string(),
            "--quiet".to_string(),
            "--daemon".to_string(),
            socket.display().to_string(),
        ]
    }

    fn daemon_for(&self, user: &str, session_flags: &[String]) -> PathBuf {
        let key = format!("{}|{user}", self.address());
        let mut daemons = self.daemons.lock().expect("daemon registry");
        if let Some(existing) = daemons.get(&key) {
            return existing.socket.clone();
        }
        let handle = DaemonFixture::start(self, user, session_flags);
        let socket = handle.socket.clone();
        daemons.insert(key, handle);
        socket
    }

    /// An in-process client, used to stand up the other end of a test.
    fn client(&self, user: &str, shares: Vec<String>) -> Client {
        let port = free_port().expect("peer port");
        let mut client = Client::with_settings(ClientSettings {
            username: user.to_string(),
            password: "pw".to_string(),
            server_address: PeerAddress::new(self.host.clone(), self.port),
            enable_listen: true,
            listen_port: port,
            shared_directories: shares,
            version: soulseek_rs::ClientVersion::default(),
        });
        client.connect().expect("peer connect");
        assert!(client.login().expect("peer login"), "peer should log in");
        client
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(db) = self.db.as_ref() {
            let _ = std::fs::remove_file(db);
        }
    }
}

fn soulfind_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SOULFIND_BIN") {
        let path = PathBuf::from(path);
        return path.exists().then_some(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|dir| dir.join("soulfind/bin/soulfind"))
        .find(|candidate| candidate.exists())
}

fn free_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()
        .map(|addr| addr.port())
}

fn wait_until_listening(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Option<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(mut addrs) = format!("{host}:{port}").to_socket_addrs()
            && let Some(addr) = addrs.next()
            && TcpStream::connect_timeout(&addr, Duration::from_millis(200))
                .is_ok()
        {
            return Some(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

macro_rules! server_or_skip {
    () => {
        match TestServer::resolve() {
            Some(server) => server,
            None => {
                let required = std::env::var("SOULSEEK_E2E_REQUIRED")
                    .is_ok_and(|v| v != "0" && !v.is_empty());
                assert!(
                    !required,
                    "SOULSEEK_E2E_REQUIRED is set but no soulfind server could \
                     be started (set SOULFIND_BIN or SOULSEEK_TEST_SERVER)"
                );
                println!(
                    "cli e2e skipped: no soulfind server (set SOULFIND_BIN or \
                     SOULSEEK_TEST_SERVER to run)"
                );
                return;
            }
        }
    };
}

/// A scratch directory that cleans itself up.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        static COUNTER: std::sync::atomic::AtomicU32 =
            std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "soulseek-cli-e2e-{label}-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("scratch directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn display(&self) -> String {
        self.0.display().to_string()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Contents of the file every transfer test shares.
fn probe_bytes() -> Vec<u8> {
    (0..4096u32).map(|i| (i % 251) as u8).collect()
}

/// Run the binary against `server` as `user`, appending `args`.
///
/// In daemon mode the same call is served by a daemon instead. The flags that
/// shape a *session* rather than a command — shares, the listener, where
/// downloads land — are handed to the daemon rather than the client, because
/// that is where they now belong. The test itself does not change, which is
/// the whole point: if the records and exit code come out the same either way,
/// daemon mode is equivalent.
fn cli(server: &TestServer, user: &str, args: &[&str]) -> Output {
    let mut all = match mode() {
        Mode::Local => server.args(user),
        Mode::Daemon => {
            let (session_flags, _) = split_session_flags(args);
            server.daemon_args(user, &session_flags)
        }
    };
    let command_args: Vec<String> = match mode() {
        Mode::Local => args.iter().map(|a| (*a).to_string()).collect(),
        Mode::Daemon => split_session_flags(args).1,
    };
    all.extend(command_args);
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    run(&refs)
}

/// Split arguments into the ones that configure a session and the ones that
/// are the command. Only meaningful in daemon mode, where the two halves go to
/// different processes.
fn split_session_flags(args: &[&str]) -> (Vec<String>, Vec<String>) {
    /// Flags that take a value and describe the session, not the command.
    const SESSION_PAIRS: [&str; 4] = [
        "--shared-dir",
        "--listener-port",
        "--download-dir",
        "--max-concurrent-downloads",
    ];
    /// Session flags that stand alone.
    const SESSION_SWITCHES: [&str; 2] = ["--no-listener", "--listener"];

    let mut session = Vec::new();
    let mut command = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if SESSION_PAIRS.contains(arg) {
            session.push((*arg).to_string());
            if let Some(value) = rest.next() {
                session.push((*value).to_string());
            }
        } else if SESSION_SWITCHES.contains(arg) {
            session.push((*arg).to_string());
        } else {
            command.push((*arg).to_string());
        }
    }
    (session, command)
}

/// Wait out the SetWaitPort registrations so peer lookups resolve.
fn settle() {
    std::thread::sleep(Duration::from_secs(1));
}

/// Like [`cli`], but keeping a config file so a command that reads or writes
/// one (the wishlist) sees it.
///
/// `--no-config` is dropped for the obvious reason, and `--quiet` with it:
/// these commands report the schedule the *server* chose on stderr, which is
/// the only place a test can see it.
fn cli_with_config(
    server: &TestServer,
    user: &str,
    config: &Path,
    args: &[&str],
) -> Output {
    let mut all: Vec<String> = server
        .args(user)
        .into_iter()
        .filter(|arg| arg != "--no-config" && arg != "--quiet")
        .collect();
    all.push("--config".to_string());
    all.push(config.to_str().expect("utf-8 path").to_string());
    all.extend(args.iter().map(|a| (*a).to_string()));
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    run(&refs)
}

#[test]
fn a_wrong_password_is_a_connection_error() {
    let server = server_or_skip!();
    // Registers the name with password "pw".
    let _owner = server.client("cli_e2e_owner", Vec::new());

    let output = run(&[
        "--no-config",
        "--quiet",
        "--server",
        &server.address(),
        "--username",
        "cli_e2e_owner",
        "--password",
        "definitely-wrong",
        "--no-listener",
        "room",
        "list",
    ]);
    assert_eq!(code(&output), EXIT_CONNECTION);
    assert!(stdout(&output).is_empty());
}

#[test]
fn search_prints_a_record_that_download_can_consume() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    std::fs::write(share.path().join("cli_probe_qwix.bin"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_sharer_a", vec![share.display()]);
    settle();

    let output = cli(
        &server,
        "cli_e2e_seeker_a",
        &["--search-timeout", "5", "search", "qwix"],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let record = records(&output)
        .into_iter()
        .find(|line| line.contains("cli_probe_qwix"))
        .expect("the sharer's file should be listed");

    // user TAB size TAB bitrate TAB path — the columns `download --stdin`
    // reads back.
    let fields: Vec<&str> = record.split('\t').collect();
    assert_eq!(fields.len(), 4, "record was {record:?}");
    assert_eq!(fields[0], "cli_e2e_sharer_a");
    assert_eq!(fields[1], probe_bytes().len().to_string());
    assert!(fields[3].contains("cli_probe_qwix.bin"));
}

#[test]
fn search_json_records_carry_the_fields_a_filter_needs() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    std::fs::write(share.path().join("cli_probe_zorb.bin"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_sharer_b", vec![share.display()]);
    settle();

    let output = cli(
        &server,
        "cli_e2e_seeker_b",
        &["--search-timeout", "5", "--json", "search", "zorb"],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let record: serde_json::Value = records(&output)
        .iter()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .expect("each line should be valid JSON")
        })
        .find(|value| {
            value["path"].as_str().is_some_and(|p| p.contains("zorb"))
        })
        .expect("the sharer's file should be listed");

    assert_eq!(record["user"], "cli_e2e_sharer_b");
    assert_eq!(record["size"], probe_bytes().len() as u64);
    assert!(record["free_slot"].is_boolean());
    assert!(record.get("speed").is_some());
}

#[test]
fn a_search_with_no_matches_exits_with_the_no_results_code() {
    let server = server_or_skip!();
    let _peer = server.client("cli_e2e_quiet_peer", Vec::new());
    settle();

    let output = cli(
        &server,
        "cli_e2e_seeker_c",
        &["--search-timeout", "3", "search", "nothingmatchesthisqzx"],
    );
    assert_eq!(code(&output), EXIT_NO_RESULTS);
    assert!(stdout(&output).is_empty(), "stdout must stay data-only");
}

/// One share holding three files that differ in name, extension and size, so
/// every result filter has something to keep and something to drop.
///
/// `flt_beta_skipme.flac` is deliberately the small one and
/// `flt_gamma_keepme.ogg` the large one, so a size bound and an extension
/// bound select different files and a combination of the two is a real test
/// rather than two spellings of the same assertion.
struct FilterFixture {
    _share: Scratch,
}

impl FilterFixture {
    const SMALL: usize = 128;
    const MEDIUM: usize = 4096;
    const LARGE: usize = 8192;

    fn new(server: &TestServer, sharer: &str) -> (Self, Client) {
        let share = Scratch::new("filter-share");
        for (name, size) in [
            ("flt_alpha_keepme.mp3", Self::MEDIUM),
            ("flt_beta_skipme.flac", Self::SMALL),
            ("flt_gamma_keepme.ogg", Self::LARGE),
        ] {
            std::fs::write(share.path().join(name), vec![7u8; size])
                .expect("share file");
        }
        let client = server.client(sharer, vec![share.display()]);
        settle();
        (Self { _share: share }, client)
    }
}

/// Search for the fixture as a fresh user and return the matching file names,
/// sorted so the assertion does not depend on result order.
///
/// A new username per call matters: soulfind answers a repeat of the same
/// search from the same user with nothing.
fn filtered_names(
    server: &TestServer,
    seeker: &str,
    filter: &[&str],
) -> (i32, Vec<String>) {
    let mut args = vec!["--search-timeout", "5", "search", "flt"];
    args.extend_from_slice(filter);
    let output = cli(server, seeker, &args);
    let mut names: Vec<String> = records(&output)
        .iter()
        .filter_map(|line| line.split('\t').nth(3))
        .filter_map(|path| path.rsplit(['/', '\\']).next())
        .filter(|name| name.starts_with("flt_"))
        .map(String::from)
        .collect();
    names.sort();
    (code(&output), names)
}

#[test]
fn exclude_drops_the_results_whose_path_contains_the_term() {
    let server = server_or_skip!();
    let (_fixture, _sharer) = FilterFixture::new(&server, "cli_e2e_flt_excl");

    let (exit, names) =
        filtered_names(&server, "cli_e2e_seek_excl", &["--exclude", "skipme"]);
    assert_eq!(exit, EXIT_OK);
    assert_eq!(names, ["flt_alpha_keepme.mp3", "flt_gamma_keepme.ogg"]);
}

#[test]
fn extension_keeps_one_type_and_repeats_into_a_union() {
    let server = server_or_skip!();
    let (_fixture, _sharer) = FilterFixture::new(&server, "cli_e2e_flt_ext");

    let (exit, one) = filtered_names(
        &server,
        "cli_e2e_seek_ext_one",
        &["--extension", "mp3"],
    );
    assert_eq!(exit, EXIT_OK);
    assert_eq!(one, ["flt_alpha_keepme.mp3"]);

    let (exit, several) = filtered_names(
        &server,
        "cli_e2e_seek_ext_many",
        &["--extension", ".flac", "--extension", "OGG"],
    );
    assert_eq!(exit, EXIT_OK);
    assert_eq!(several, ["flt_beta_skipme.flac", "flt_gamma_keepme.ogg"]);
}

#[test]
fn size_bounds_narrow_the_result_set_from_either_end() {
    let server = server_or_skip!();
    let (_fixture, _sharer) = FilterFixture::new(&server, "cli_e2e_flt_size");

    let (exit, big) = filtered_names(
        &server,
        "cli_e2e_seek_size_min",
        &["--min-size", &FilterFixture::MEDIUM.to_string()],
    );
    assert_eq!(exit, EXIT_OK);
    assert_eq!(big, ["flt_alpha_keepme.mp3", "flt_gamma_keepme.ogg"]);

    let (exit, small) = filtered_names(
        &server,
        "cli_e2e_seek_size_max",
        &["--max-size", &FilterFixture::SMALL.to_string()],
    );
    assert_eq!(exit, EXIT_OK);
    assert_eq!(small, ["flt_beta_skipme.flac"]);
}

#[test]
fn filters_given_together_all_have_to_be_satisfied() {
    let server = server_or_skip!();
    let (_fixture, _sharer) = FilterFixture::new(&server, "cli_e2e_flt_comb");

    let (exit, names) = filtered_names(
        &server,
        "cli_e2e_seek_comb",
        &[
            "--exclude",
            "skipme",
            "--min-size",
            &FilterFixture::LARGE.to_string(),
            "--extension",
            "ogg",
        ],
    );
    assert_eq!(exit, EXIT_OK);
    assert_eq!(names, ["flt_gamma_keepme.ogg"]);
}

#[test]
fn a_filter_that_matches_nothing_is_the_no_results_code() {
    let server = server_or_skip!();
    let (_fixture, _sharer) = FilterFixture::new(&server, "cli_e2e_flt_none");

    let output = cli(
        &server,
        "cli_e2e_seek_none",
        &[
            "--search-timeout",
            "5",
            "search",
            "flt",
            "--extension",
            "aiff",
        ],
    );
    assert_eq!(
        code(&output),
        EXIT_NO_RESULTS,
        "stderr: {}",
        stderr(&output)
    );
    assert!(stdout(&output).is_empty(), "stdout must stay data-only");
}

#[test]
fn get_honours_the_same_filters_search_does() {
    let server = server_or_skip!();
    let (_fixture, _sharer) = FilterFixture::new(&server, "cli_e2e_flt_get");
    let target = Scratch::new("filter-downloads");

    let output = cli(
        &server,
        "cli_e2e_seek_get",
        &[
            "--search-timeout",
            "5",
            "--download-dir",
            &target.display(),
            "get",
            "flt",
            "--extension",
            "ogg",
            "--timeout",
            "30",
        ],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));
    assert!(
        target.path().join("flt_gamma_keepme.ogg").exists(),
        "the only .ogg should be what got fetched; stdout: {}",
        stdout(&output)
    );
}

#[test]
fn wish_run_finds_a_shared_file_through_a_wishlist_search() {
    let server = server_or_skip!();
    let share = Scratch::new("wish-share");
    std::fs::write(share.path().join("wisher_probe_krel.mp3"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_wish_sharer", vec![share.display()]);
    let dir = Scratch::new("wish-config");
    let config = dir.path().join("config.toml");
    settle();

    let added = with_config(&config, &["wish", "add", "krel"]);
    assert_eq!(code(&added), EXIT_OK, "stderr: {}", stderr(&added));

    let output = cli_with_config(
        &server,
        "cli_e2e_wish_seeker",
        &config,
        &["--search-timeout", "5", "wish", "run"],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let record = records(&output)
        .into_iter()
        .find(|line| line.contains("wisher_probe_krel"))
        .expect("the wishlist search should find the sharer's file");
    // The same four columns `search` prints, so this pipes into
    // `download --stdin` with no reshaping.
    let fields: Vec<&str> = record.split('\t').collect();
    assert_eq!(fields.len(), 4, "record was {record:?}");
    assert_eq!(fields[0], "cli_e2e_wish_sharer");
    assert_eq!(fields[1], probe_bytes().len().to_string());
}

#[test]
fn wish_run_output_feeds_download_stdin() {
    let server = server_or_skip!();
    let share = Scratch::new("wish-share");
    std::fs::write(share.path().join("wisher_probe_vosk.mp3"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_wish_sharer_b", vec![share.display()]);
    let dir = Scratch::new("wish-config");
    let config = dir.path().join("config.toml");
    let target = Scratch::new("wish-downloads");
    settle();

    with_config(&config, &["wish", "add", "vosk"]);
    let found = cli_with_config(
        &server,
        "cli_e2e_wish_seeker_b",
        &config,
        &["--search-timeout", "5", "--json", "wish", "run"],
    );
    assert_eq!(code(&found), EXIT_OK, "stderr: {}", stderr(&found));

    let mut child = command(&[
        "--no-config",
        "--quiet",
        "--server",
        &server.address(),
        "--username",
        "cli_e2e_wish_puller",
        "--password",
        "pw",
        "--listener-port",
        &free_port().expect("listener port").to_string(),
        "--download-dir",
        &target.display(),
        "download",
        "--stdin",
        "--timeout",
        "30",
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("download should start");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdout(&found).as_bytes())
        .expect("feed the records back");
    let pulled = child.wait_with_output().expect("download should finish");

    assert_eq!(code(&pulled), EXIT_OK, "stderr: {}", stderr(&pulled));
    assert!(
        target.path().join("wisher_probe_vosk.mp3").exists(),
        "the wished-for file should have landed; stdout: {}",
        stdout(&pulled)
    );
}

#[test]
fn wish_run_with_an_empty_wishlist_is_the_no_results_code() {
    let server = server_or_skip!();
    let _peer = server.client("cli_e2e_wish_quiet", Vec::new());
    let dir = Scratch::new("wish-config");
    let config = dir.path().join("config.toml");

    let output = cli_with_config(
        &server,
        "cli_e2e_wish_none",
        &config,
        &["wish", "run"],
    );
    assert_eq!(code(&output), EXIT_NO_RESULTS);
    assert!(stdout(&output).is_empty(), "stdout must stay data-only");
}

#[test]
fn wish_run_only_must_name_a_stored_wish() {
    let server = server_or_skip!();
    let _peer = server.client("cli_e2e_wish_quiet_b", Vec::new());
    let dir = Scratch::new("wish-config");
    let config = dir.path().join("config.toml");
    with_config(&config, &["wish", "add", "stored one"]);

    let unknown = cli_with_config(
        &server,
        "cli_e2e_wish_only_bad",
        &config,
        &["wish", "run", "--only", "not stored"],
    );
    assert_eq!(code(&unknown), EXIT_USAGE, "a typo is not an ad-hoc search");
    assert!(stdout(&unknown).is_empty());
}

#[test]
fn wish_run_only_narrows_the_sweep_to_that_wish() {
    let server = server_or_skip!();
    let share = Scratch::new("wish-share");
    for name in ["wisher_probe_zant.mp3", "wisher_probe_hurl.mp3"] {
        std::fs::write(share.path().join(name), probe_bytes())
            .expect("share file");
    }
    let _sharer = server.client("cli_e2e_wish_sharer_c", vec![share.display()]);
    let dir = Scratch::new("wish-config");
    let config = dir.path().join("config.toml");
    settle();

    with_config(&config, &["wish", "add", "zant"]);
    with_config(&config, &["wish", "add", "hurl"]);

    let output = cli_with_config(
        &server,
        "cli_e2e_wish_only_ok",
        &config,
        &["--search-timeout", "5", "wish", "run", "--only", "ZANT"],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let paths = stdout(&output);
    assert!(paths.contains("wisher_probe_zant"), "stdout was {paths:?}");
    assert!(
        !paths.contains("wisher_probe_hurl"),
        "--only must not run the other wish; stdout was {paths:?}"
    );
}

#[test]
fn wish_run_filters_apply_to_a_wishlist_search_too() {
    let server = server_or_skip!();
    let share = Scratch::new("wish-share");
    std::fs::write(share.path().join("wisher_probe_gleb.mp3"), probe_bytes())
        .expect("share file");
    std::fs::write(share.path().join("wisher_probe_gleb.flac"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_wish_sharer_d", vec![share.display()]);
    let dir = Scratch::new("wish-config");
    let config = dir.path().join("config.toml");
    settle();

    with_config(&config, &["wish", "add", "gleb"]);
    let output = cli_with_config(
        &server,
        "cli_e2e_wish_filtered",
        &config,
        &[
            "--search-timeout",
            "5",
            "wish",
            "run",
            "--extension",
            "flac",
        ],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));
    let paths = stdout(&output);
    assert!(paths.contains("wisher_probe_gleb.flac"), "{paths:?}");
    assert!(!paths.contains("wisher_probe_gleb.mp3"), "{paths:?}");
}

#[test]
fn serve_with_the_wishlist_sweeps_on_the_interval_the_server_announced() {
    let server = server_or_skip!();
    let their_share = Scratch::new("wish-their-share");
    std::fs::write(
        their_share.path().join("wisher_probe_yalt.mp3"),
        probe_bytes(),
    )
    .expect("share file");
    let _sharer =
        server.client("cli_e2e_wish_sharer_e", vec![their_share.display()]);

    // A serving client needs something of its own to share.
    let our_share = Scratch::new("wish-our-share");
    std::fs::write(our_share.path().join("ours.bin"), probe_bytes())
        .expect("share file");
    let dir = Scratch::new("wish-config");
    let config = dir.path().join("config.toml");
    settle();

    with_config(&config, &["wish", "add", "yalt"]);
    let output = cli_with_config(
        &server,
        "cli_e2e_wish_server",
        &config,
        &[
            "--search-timeout",
            "5",
            "--shared-dir",
            &our_share.display(),
            "serve",
            "--wishlist",
            "--duration",
            "12",
        ],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("wisher_probe_yalt"),
        "the first sweep happens at startup; stdout: {}",
        stdout(&output)
    );
    // Soulfind announces 12 minutes for an unprivileged account, and the
    // schedule is the server's to set — so that is what must be reported.
    assert!(
        stderr(&output).contains("next wishlist sweep in 720s"),
        "the announced interval should drive the timer; stderr: {}",
        stderr(&output)
    );
}

#[test]
fn serve_wishlist_with_nothing_wished_for_still_serves() {
    let server = server_or_skip!();
    let our_share = Scratch::new("wish-our-share");
    std::fs::write(our_share.path().join("ours.bin"), probe_bytes())
        .expect("share file");
    let dir = Scratch::new("wish-config");
    let config = dir.path().join("config.toml");

    let output = cli_with_config(
        &server,
        "cli_e2e_wish_server_b",
        &config,
        &[
            "--shared-dir",
            &our_share.display(),
            "serve",
            "--wishlist",
            "--duration",
            "2",
        ],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("nothing to look for"),
        "an empty wishlist should be said once, not serve-and-be-silent; \
         stderr: {}",
        stderr(&output)
    );
}

#[test]
fn browse_lists_shares_as_paths_download_understands() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    std::fs::write(share.path().join("browsable.bin"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_sharer_c", vec![share.display()]);
    settle();

    let output =
        cli(&server, "cli_e2e_browser", &["browse", "cli_e2e_sharer_c"]);
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let record = records(&output)
        .into_iter()
        .find(|line| line.contains("browsable.bin"))
        .expect("the shared file should be listed");

    // user TAB size TAB path
    let fields: Vec<&str> = record.split('\t').collect();
    assert_eq!(fields.len(), 3, "record was {record:?}");
    assert_eq!(fields[0], "cli_e2e_sharer_c");
    assert_eq!(fields[1], probe_bytes().len().to_string());
}

#[test]
fn download_fetches_a_file_and_prints_where_it_landed() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    let target = Scratch::new("downloads");
    std::fs::write(share.path().join("fetch_me.bin"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_sharer_d", vec![share.display()]);
    settle();

    // Learn the peer-side path the way a script would.
    let listing =
        cli(&server, "cli_e2e_fetcher", &["browse", "cli_e2e_sharer_d"]);
    assert_eq!(code(&listing), EXIT_OK, "stderr: {}", stderr(&listing));
    let row = records(&listing)
        .into_iter()
        .find(|line| line.contains("fetch_me.bin"))
        .expect("the shared file should be listed");
    let remote_path = row.rsplit('\t').next().expect("path column").to_string();

    let output = cli(
        &server,
        "cli_e2e_fetcher",
        &[
            "--download-dir",
            &target.display(),
            "download",
            "cli_e2e_sharer_d",
            &remote_path,
            "--size",
            &probe_bytes().len().to_string(),
            "--timeout",
            "40",
        ],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let reported = records(&output);
    assert_eq!(reported.len(), 1, "one file, one record");
    let written = std::fs::read(reported[0].trim())
        .expect("the reported path should be the file that was written");
    assert_eq!(written, probe_bytes());
}

#[test]
fn download_without_a_size_looks_it_up_in_the_peers_shares() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    let target = Scratch::new("downloads");
    std::fs::write(share.path().join("sizeless.bin"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_sharer_g", vec![share.display()]);
    settle();

    let listing =
        cli(&server, "cli_e2e_sizeless", &["browse", "cli_e2e_sharer_g"]);
    let remote_path = records(&listing)
        .into_iter()
        .find(|line| line.contains("sizeless.bin"))
        .and_then(|row| row.rsplit('\t').next().map(String::from))
        .expect("the shared file should be listed");

    // No --size: the size has to come from the peer's listing, and the remote
    // path there is the full backslash path, not the bare file name.
    let output = cli(
        &server,
        "cli_e2e_sizeless",
        &[
            "--download-dir",
            &target.display(),
            "download",
            "cli_e2e_sharer_g",
            &remote_path,
            "--timeout",
            "40",
        ],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let reported = records(&output);
    assert_eq!(reported.len(), 1);
    assert_eq!(
        std::fs::read(reported[0].trim()).expect("downloaded file"),
        probe_bytes()
    );
}

#[test]
fn download_reads_the_records_search_emits_from_stdin() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    let target = Scratch::new("downloads");
    std::fs::write(share.path().join("piped.bin"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_sharer_e", vec![share.display()]);
    settle();

    let listing =
        cli(&server, "cli_e2e_piper", &["browse", "cli_e2e_sharer_e"]);
    let row = records(&listing)
        .into_iter()
        .find(|line| line.contains("piped.bin"))
        .expect("the shared file should be listed");

    let mut args = server.args("cli_e2e_piper");
    args.extend(
        [
            "--download-dir",
            &target.display(),
            "download",
            "--stdin",
            "--timeout",
            "40",
        ]
        .iter()
        .map(|a| (*a).to_string()),
    );
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut child = command(&refs)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary should run");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(format!("{row}\n").as_bytes())
        .expect("write the record");
    let output = child.wait_with_output().expect("the binary should finish");

    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));
    let reported = records(&output);
    assert_eq!(reported.len(), 1);
    assert_eq!(
        std::fs::read(reported[0].trim()).expect("downloaded file"),
        probe_bytes()
    );
}

#[test]
fn get_searches_picks_and_downloads_in_one_command() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    let target = Scratch::new("downloads");
    std::fs::write(share.path().join("cli_probe_vroom.bin"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_sharer_f", vec![share.display()]);
    settle();

    let output = cli(
        &server,
        "cli_e2e_getter",
        &[
            "--search-timeout",
            "5",
            "--download-dir",
            &target.display(),
            "get",
            "vroom",
            "--timeout",
            "40",
        ],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let reported = records(&output);
    assert_eq!(reported.len(), 1, "--pick best downloads one file");
    assert_eq!(
        std::fs::read(reported[0].trim()).expect("downloaded file"),
        probe_bytes()
    );
}

#[test]
fn get_without_matches_exits_no_results_and_downloads_nothing() {
    let server = server_or_skip!();
    let target = Scratch::new("downloads");
    let _peer = server.client("cli_e2e_silent_peer", Vec::new());
    settle();

    let output = cli(
        &server,
        "cli_e2e_getter_b",
        &[
            "--search-timeout",
            "3",
            "--download-dir",
            &target.display(),
            "get",
            "nothingmatchesthisqzx",
        ],
    );
    assert_eq!(code(&output), EXIT_NO_RESULTS);
    assert_eq!(
        std::fs::read_dir(target.path())
            .expect("download dir")
            .count(),
        0,
        "nothing should have been written"
    );
}

#[test]
fn room_say_reaches_a_listener_and_room_list_shows_the_room() {
    let server = server_or_skip!();
    let listener = server.client("cli_e2e_room_listener", Vec::new());
    listener.join_room("cli_e2e_lobby").expect("join");
    settle();

    let said = cli(
        &server,
        "cli_e2e_speaker",
        &["room", "say", "cli_e2e_lobby", "hello from a script"],
    );
    assert_eq!(code(&said), EXIT_OK, "stderr: {}", stderr(&said));
    assert!(
        stdout(&said).is_empty(),
        "an action command emits no records"
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut heard = false;
    while Instant::now() < deadline && !heard {
        heard = listener.take_room_events().iter().any(|event| {
            matches!(
                event,
                soulseek_rs::types::RoomEvent::Message { message, .. }
                    if message == "hello from a script"
            )
        });
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(heard, "the room should have received the message");

    let listed = cli(&server, "cli_e2e_lister", &["room", "list"]);
    assert_eq!(code(&listed), EXIT_OK, "stderr: {}", stderr(&listed));
    let row = records(&listed)
        .into_iter()
        .find(|line| line.ends_with("cli_e2e_lobby"))
        .expect("the joined room should be listed");
    let (users, _) = row.split_once('\t').expect("users TAB room");
    assert!(
        users.parse::<u32>().is_ok(),
        "the user count should be a bare number, got {users:?}"
    );
}

#[test]
fn room_listen_streams_what_is_said_in_the_room() {
    let server = server_or_skip!();
    let speaker = server.client("cli_e2e_room_speaker", Vec::new());
    speaker.join_room("cli_e2e_lobby_b").expect("join");
    settle();

    let mut args = server.args("cli_e2e_room_reader");
    args.extend(
        [
            "room",
            "listen",
            "cli_e2e_lobby_b",
            "--duration",
            "8",
            "--json",
        ]
        .iter()
        .map(|a| (*a).to_string()),
    );
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let child = command(&refs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary should run");

    // Give the listener time to join before saying anything.
    std::thread::sleep(Duration::from_secs(3));
    speaker
        .say_in_room("cli_e2e_lobby_b", "streamed line")
        .expect("say");

    // A second arrival: the README says a join is its own record type, with an
    // empty message field.
    let latecomer = server.client("cli_e2e_room_joiner", Vec::new());
    latecomer.join_room("cli_e2e_lobby_b").expect("join");

    let output = child.wait_with_output().expect("the binary should finish");
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let events: Vec<serde_json::Value> = records(&output)
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    let said = events
        .iter()
        .find(|event| event["message"] == "streamed line")
        .unwrap_or_else(|| panic!("stdout was: {:?}", stdout(&output)));
    assert_eq!(said["type"], "message");
    assert_eq!(said["room"], "cli_e2e_lobby_b");
    assert_eq!(said["user"], "cli_e2e_room_speaker");

    // The listener's own arrival is a join too, so pick the one that matters.
    let joined = events
        .iter()
        .find(|event| event["user"] == "cli_e2e_room_joiner")
        .unwrap_or_else(|| panic!("no join record in {:?}", stdout(&output)));
    assert_eq!(joined["type"], "join");
    assert_eq!(joined["room"], "cli_e2e_lobby_b");
    assert_eq!(joined["message"], "");
}

#[test]
fn a_private_message_reaches_its_recipient() {
    let server = server_or_skip!();
    let recipient = server.client("cli_e2e_recipient", Vec::new());
    settle();

    let output = cli(
        &server,
        "cli_e2e_sender",
        &["message", "send", "cli_e2e_recipient", "ping from a script"],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).is_empty(),
        "an action command emits no records"
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut received = false;
    while Instant::now() < deadline && !received {
        received = recipient
            .take_private_messages()
            .iter()
            .any(|message| message.message() == "ping from a script");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(received, "the recipient should have received the message");
}

#[test]
fn message_read_streams_incoming_private_messages() {
    let server = server_or_skip!();
    let sender = server.client("cli_e2e_pm_sender", Vec::new());
    settle();

    let mut args = server.args("cli_e2e_pm_reader");
    args.extend(
        ["--json", "message", "read", "--duration", "8"]
            .iter()
            .map(|a| (*a).to_string()),
    );
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let child = command(&refs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary should run");

    std::thread::sleep(Duration::from_secs(3));
    sender
        .send_private_message("cli_e2e_pm_reader", "inbox line")
        .expect("send");

    let output = child.wait_with_output().expect("the binary should finish");
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let read = records(&output).iter().any(|line| {
        serde_json::from_str::<serde_json::Value>(line).is_ok_and(|value| {
            value["message"] == "inbox line"
                && value["user"] == "cli_e2e_pm_sender"
        })
    });
    assert!(read, "stdout was: {:?}", stdout(&output));
}

// ---------------------------------------------------------------------------
// Being a peer: whoami, user, serve, shares, room users
// ---------------------------------------------------------------------------

#[test]
fn whoami_confirms_the_login_and_reports_what_we_offer() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    std::fs::write(share.path().join("offered.bin"), probe_bytes())
        .expect("share file");

    let output = cli(
        &server,
        "cli_e2e_whoami",
        &["--shared-dir", &share.display(), "--json", "whoami"],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let record: serde_json::Value =
        serde_json::from_str(records(&output).first().expect("one record"))
            .expect("valid JSON");
    assert_eq!(record["user"], "cli_e2e_whoami");
    assert_eq!(record["shared_files"], 1, "the shared file is indexed");
    assert!(record["server"].as_str().is_some_and(|s| s.contains(':')));
}

#[test]
fn whoami_reports_how_much_privilege_time_the_account_has() {
    let server = server_or_skip!();
    let _owner = server.client("cli_e2e_priv_owner", Vec::new());

    let output = cli(&server, "cli_e2e_priv_owner", &["--json", "whoami"]);
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let record: serde_json::Value =
        serde_json::from_str(&records(&output)[0]).expect("valid JSON");
    // Zero, not null: the server answered, and the answer is "no privileges".
    // Null would mean it never said, which is a different fact.
    assert_eq!(
        record["privilege_seconds"], 0,
        "a plain account should read as zero seconds, got {record}"
    );
}

#[test]
fn serve_reports_a_queued_upload_once_the_slots_are_full() {
    let server = server_or_skip!();
    let share = Scratch::new("queue-share");
    for name in ["q_one.bin", "q_two.bin"] {
        std::fs::write(share.path().join(name), probe_bytes())
            .expect("share file");
    }
    let target_a = Scratch::new("queue-dl-a");
    let target_b = Scratch::new("queue-dl-b");

    let serving = command(&[
        "--no-config",
        "--server",
        &server.address(),
        "--username",
        "cli_e2e_queue_server",
        "--password",
        "pw",
        "--listener-port",
        &free_port().expect("listener port").to_string(),
        "--shared-dir",
        &share.display(),
        "--json",
        "serve",
        "--upload-slots",
        "1",
        "--duration",
        "18",
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("serve should start");
    settle();
    settle();

    // Two peers pull at once against a single slot, so one of them has to wait.
    let mut pulls = Vec::new();
    for (user, target) in [
        ("cli_e2e_queue_a", &target_a),
        ("cli_e2e_queue_b", &target_b),
    ] {
        pulls.push(
            command(&[
                "--no-config",
                "--quiet",
                "--server",
                &server.address(),
                "--username",
                user,
                "--password",
                "pw",
                "--listener-port",
                &free_port().expect("listener port").to_string(),
                "--download-dir",
                &target.display(),
                "--search-timeout",
                "5",
                "get",
                "q_",
                "--pick",
                "all",
                "--timeout",
                "30",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("puller should start"),
        );
    }
    for mut pull in pulls {
        let _ = pull.wait();
    }

    let uploaded = serving.wait_with_output().expect("serve should finish");
    let statuses: Vec<String> = stdout(&uploaded)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|value| {
            value["status"]
                .as_str()
                .map(std::string::ToString::to_string)
        })
        .collect();
    assert!(
        statuses.iter().any(|status| status == "completed"),
        "at least one upload should have gone through; stdout: {}",
        stdout(&uploaded)
    );
    assert!(
        statuses.iter().any(|status| status == "queued"),
        "with one slot and two peers pulling, `serve` should report a queued \
         upload; statuses were {statuses:?}"
    );
}

#[test]
fn whoami_fails_with_the_connection_code_on_a_bad_password() {
    let server = server_or_skip!();
    let _owner = server.client("cli_e2e_whoami_owner", Vec::new());

    let output = run(&[
        "--no-config",
        "--quiet",
        "--server",
        &server.address(),
        "--username",
        "cli_e2e_whoami_owner",
        "--password",
        "wrong",
        "--no-listener",
        "whoami",
    ]);
    assert_eq!(code(&output), EXIT_CONNECTION);
}

#[test]
fn user_reports_a_peers_status_and_share_counts() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    std::fs::create_dir_all(share.path().join("album")).expect("album");
    for name in ["a.bin", "b.bin"] {
        std::fs::write(share.path().join("album").join(name), probe_bytes())
            .expect("share file");
    }
    let _subject = server.client("cli_e2e_subject", vec![share.display()]);
    settle();

    let output = cli(
        &server,
        "cli_e2e_asker",
        &["--json", "user", "cli_e2e_subject"],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let record: serde_json::Value =
        serde_json::from_str(records(&output).first().expect("one record"))
            .expect("valid JSON");
    assert_eq!(record["user"], "cli_e2e_subject");
    assert_ne!(record["status"], "offline", "the subject is logged in");
    assert_eq!(record["shared_files"], 2);
}

#[test]
fn serve_stays_online_and_reports_an_upload_a_peer_pulls() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    let target = Scratch::new("downloads");
    std::fs::write(share.path().join("served.bin"), probe_bytes())
        .expect("share file");

    // A serving process: online, sharing, streaming upload records.
    let mut args = server.args("cli_e2e_server");
    args.retain(|arg| arg != "--quiet");
    args.extend(
        [
            "--shared-dir",
            &share.display(),
            "--json",
            "serve",
            "--duration",
            "25",
        ]
        .iter()
        .map(|a| (*a).to_string()),
    );
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let child = command(&refs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary should run");

    // Give it time to log in and register its listen port, then pull a file
    // from it the way any peer would: browse to learn the path, then download.
    std::thread::sleep(Duration::from_secs(4));
    let listing = cli(&server, "cli_e2e_puller", &["browse", "cli_e2e_server"]);
    let remote_path = records(&listing)
        .into_iter()
        .find(|line| line.contains("served.bin"))
        .and_then(|row| row.rsplit('\t').next().map(String::from))
        .unwrap_or_else(|| {
            panic!(
                "a serving process should answer a browse; exit={} stderr={}",
                code(&listing),
                stderr(&listing)
            )
        });

    let puller = cli(
        &server,
        "cli_e2e_puller",
        &[
            "--download-dir",
            &target.display(),
            "download",
            "cli_e2e_server",
            &remote_path,
            "--size",
            &probe_bytes().len().to_string(),
            "--timeout",
            "20",
        ],
    );

    let output = child.wait_with_output().expect("serve should finish");
    assert_eq!(code(&output), EXIT_OK, "serve stderr: {}", stderr(&output));

    // The download may take the browse path for its remote name; what this
    // test pins is that serve REPORTED the upload it served.
    let uploads: Vec<serde_json::Value> = records(&output)
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    assert!(
        !uploads.is_empty(),
        "serve should report the upload. puller exit={} stderr={} serve stdout={:?}",
        code(&puller),
        stderr(&puller),
        stdout(&output)
    );
    let last = &uploads[uploads.len() - 1];
    assert_eq!(last["user"], "cli_e2e_puller");
    assert!(
        last["path"]
            .as_str()
            .is_some_and(|p| p.contains("served.bin"))
    );
    // The fields the README promises a JSON upload record carries.
    for key in ["status", "user", "path", "bytes_sent", "size", "speed"] {
        assert!(
            last.get(key).is_some(),
            "serve should carry `{key}`: {last}"
        );
    }
}

#[test]
fn serve_refuses_to_run_with_nothing_to_share() {
    let server = server_or_skip!();
    let output = cli(
        &server,
        "cli_e2e_empty_server",
        &["--shared-dir", "", "serve", "--duration", "5"],
    );
    assert_eq!(code(&output), EXIT_USAGE);
    assert!(stderr(&output).contains("share"), "got {}", stderr(&output));
}

#[test]
fn shares_status_reports_what_the_network_will_see() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    for name in ["one.bin", "two.bin", "three.bin"] {
        std::fs::write(share.path().join(name), probe_bytes())
            .expect("share file");
    }

    let output = cli(
        &server,
        "cli_e2e_shares",
        &[
            "--shared-dir",
            &share.display(),
            "--json",
            "shares",
            "status",
        ],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let record: serde_json::Value =
        serde_json::from_str(records(&output).first().expect("one record"))
            .expect("valid JSON");
    assert_eq!(record["files"], 3);
    assert!(record["folders"].as_u64().is_some_and(|n| n >= 1));
}

#[test]
fn shares_reindex_picks_up_a_file_added_after_startup() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    std::fs::write(share.path().join("first.bin"), probe_bytes())
        .expect("share file");
    std::fs::write(share.path().join("second.bin"), probe_bytes())
        .expect("share file");

    let output = cli(
        &server,
        "cli_e2e_reindex",
        &[
            "--shared-dir",
            &share.display(),
            "--json",
            "shares",
            "reindex",
        ],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let record: serde_json::Value =
        serde_json::from_str(records(&output).first().expect("one record"))
            .expect("valid JSON");
    assert_eq!(record["files"], 2, "re-indexing should see both files");
}

#[test]
fn room_users_lists_who_is_in_the_room() {
    let server = server_or_skip!();
    let resident = server.client("cli_e2e_resident", Vec::new());
    resident.join_room("cli_e2e_roster").expect("join");
    settle();

    let output = cli(
        &server,
        "cli_e2e_roster_asker",
        &["room", "users", "cli_e2e_roster"],
    );
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));
    assert!(
        records(&output)
            .iter()
            .any(|line| line == "cli_e2e_resident"),
        "the resident should be listed, got {:?}",
        records(&output)
    );
}

// ---------------------------------------------------------------------------
// Configuration (no server needed)
// ---------------------------------------------------------------------------

/// Run the binary against a throwaway config file.
fn with_config(config: &Path, args: &[&str]) -> Output {
    let mut all = vec!["--config", config.to_str().expect("utf-8 path")];
    all.extend_from_slice(args);
    run(&all)
}

#[test]
fn config_set_writes_a_value_that_get_reads_back() {
    let dir = Scratch::new("config");
    let config = dir.path().join("config.toml");

    let set =
        with_config(&config, &["config", "set", "download_dir", "/music"]);
    assert_eq!(code(&set), EXIT_OK, "stderr: {}", stderr(&set));

    let got = with_config(&config, &["config", "get", "download_dir"]);
    assert_eq!(code(&got), EXIT_OK);
    assert_eq!(records(&got), ["download_dir\t/music"]);

    // And the next run actually starts from it.
    let listed = with_config(&config, &["--json", "config", "list"]);
    let values: Vec<serde_json::Value> = records(&listed)
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    assert!(
        values
            .iter()
            .any(|v| v["key"] == "download_dir" && v["value"] == "/music"),
        "config list should show the saved value"
    );
}

#[test]
fn config_rejects_an_unknown_key_and_a_bad_value() {
    let dir = Scratch::new("config");
    let config = dir.path().join("config.toml");

    let unknown = with_config(&config, &["config", "set", "colour", "blue"]);
    assert_eq!(code(&unknown), EXIT_USAGE);
    assert!(stderr(&unknown).contains("unknown setting"));

    let bad =
        with_config(&config, &["config", "set", "listener_port", "not-a-port"]);
    assert_eq!(code(&bad), EXIT_USAGE);
}

#[test]
fn config_get_of_an_unset_key_is_the_no_results_code() {
    let dir = Scratch::new("config");
    let config = dir.path().join("config.toml");
    let output = with_config(&config, &["config", "get", "username"]);
    assert_eq!(code(&output), EXIT_NO_RESULTS);
}

#[test]
fn config_path_reports_the_file_in_use() {
    let dir = Scratch::new("config");
    let config = dir.path().join("config.toml");
    let output = with_config(&config, &["config", "path"]);
    assert_eq!(code(&output), EXIT_OK);
    assert!(
        records(&output)[0].contains("config.toml"),
        "got {:?}",
        records(&output)
    );
}

#[test]
fn shares_add_and_remove_persist_to_the_config_file() {
    let dir = Scratch::new("config");
    let shared = Scratch::new("shared");
    let config = dir.path().join("config.toml");

    let added = with_config(&config, &["shares", "add", &shared.display()]);
    assert_eq!(code(&added), EXIT_OK, "stderr: {}", stderr(&added));

    let listed = with_config(&config, &["shares", "list"]);
    assert_eq!(code(&listed), EXIT_OK);
    assert!(
        records(&listed)
            .iter()
            .any(|line| line.contains(&shared.display())),
        "the added folder should be listed, got {:?}",
        records(&listed)
    );

    let removed =
        with_config(&config, &["shares", "remove", &shared.display()]);
    assert_eq!(code(&removed), EXIT_OK, "stderr: {}", stderr(&removed));

    let after = with_config(&config, &["shares", "list"]);
    assert!(
        !records(&after)
            .iter()
            .any(|line| line.contains(&shared.display())),
        "the folder should be gone, got {:?}",
        records(&after)
    );
}

#[test]
fn shares_add_refuses_a_folder_that_is_not_there() {
    let dir = Scratch::new("config");
    let config = dir.path().join("config.toml");
    let output =
        with_config(&config, &["shares", "add", "/definitely/not/here"]);
    assert_eq!(code(&output), EXIT_USAGE);
    assert!(
        stderr(&output).contains("does not exist"),
        "got {}",
        stderr(&output)
    );
}

/// The contract every scripted consumer relies on: with `--json`, stdout
/// carries newline-delimited JSON and nothing else, while the progress
/// narration still goes to stderr.
#[test]
fn json_mode_keeps_stdout_pure_for_every_command() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    std::fs::write(share.path().join("purity_probe.bin"), probe_bytes())
        .expect("share file");
    let peer = server.client("cli_e2e_purity_peer", vec![share.display()]);
    peer.join_room("cli_e2e_purity_room").expect("join");
    settle();

    // Deliberately NOT --quiet: progress must be produced, and must not land
    // on stdout. Commands are run for their output shape, not their verdict —
    // several legitimately exit non-zero here.
    let cases: Vec<Vec<&str>> = vec![
        vec!["whoami"],
        vec!["search", "purity_probe"],
        vec!["browse", "cli_e2e_purity_peer"],
        vec!["user", "cli_e2e_purity_peer"],
        vec!["room", "list"],
        vec!["room", "users", "cli_e2e_purity_room"],
        vec!["room", "listen", "cli_e2e_purity_room", "--duration", "2"],
        vec!["message", "read", "--duration", "2"],
        vec!["shares", "status"],
        vec!["shares", "list"],
        vec!["config", "list"],
        vec!["config", "path"],
        vec!["portmap"],
        vec!["search", "nothing_matches_this_qzx"],
        vec!["browse", "no_such_user_qzx"],
    ];

    let mut narrated = false;
    for case in cases {
        let mut args = server.args("cli_e2e_purity");
        args.retain(|arg| arg != "--quiet");
        args.push("--json".to_string());
        args.push("--search-timeout".to_string());
        args.push("3".to_string());
        args.extend(case.iter().map(|a| (*a).to_string()));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = run(&refs);

        for line in stdout(&output).lines() {
            assert!(
                serde_json::from_str::<serde_json::Value>(line).is_ok(),
                "`{}` put a non-JSON line on stdout: {line:?}",
                case.join(" ")
            );
        }
        narrated |= !stderr(&output).trim().is_empty();
    }

    assert!(
        narrated,
        "progress should still be reported — on stderr, where it belongs"
    );
}

/// Failure must never leave half a record behind for a parser to trip on.
#[test]
fn a_failing_command_writes_nothing_at_all_to_stdout() {
    let server = server_or_skip!();
    for case in [
        vec!["search", "nothing_matches_this_qzx"],
        vec!["browse", "no_such_user_qzx"],
    ] {
        let mut args = server.args("cli_e2e_empty_stdout");
        args.push("--json".to_string());
        args.push("--search-timeout".to_string());
        args.push("3".to_string());
        args.extend(case.iter().map(|a| (*a).to_string()));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = run(&refs);

        assert_ne!(code(&output), EXIT_OK, "`{}` should fail", case.join(" "));
        assert!(
            stdout(&output).trim().is_empty(),
            "`{}` failed but still wrote to stdout: {:?}",
            case.join(" "),
            stdout(&output)
        );
    }
}

/// Asking about a user nobody has heard of is a question the server answers,
/// not an error: the answer is "offline", and a script branches on the record
/// rather than on a failure.
#[test]
fn asking_about_an_unknown_user_succeeds_with_an_offline_record() {
    let server = server_or_skip!();
    let output = cli(
        &server,
        "cli_e2e_unknown_asker",
        &["--json", "user", "nobody_qzx"],
    );

    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));
    let record: serde_json::Value =
        serde_json::from_str(records(&output).first().expect("one record"))
            .expect("valid JSON");
    assert_eq!(record["status"], "offline");
    assert_eq!(record["shared_files"], 0);
}

/// Assert that `output` succeeded and that its first record carries every key
/// the documentation promises. `label` names the invocation in the failure.
fn assert_record_keys(output: &Output, label: &str, keys: &[&str]) {
    assert_eq!(
        code(output),
        EXIT_OK,
        "`{label}` should succeed, stderr: {}",
        stderr(output)
    );
    let record: serde_json::Value = serde_json::from_str(
        records(output).first().expect("at least one record"),
    )
    .expect("valid JSON");
    for key in keys {
        assert!(
            record.get(*key).is_some(),
            "`{label}` should carry `{key}`, got {record}"
        );
    }
}

/// "Thirteen of these need no credentials, because they never touch the
/// network." A script's first call is one of these, so an account must not be
/// a prerequisite for any of them.
#[test]
fn the_commands_advertised_as_credential_free_run_without_an_account() {
    let dir = Scratch::new("nocred");
    let config = dir.path().join("config.toml");
    let target = Scratch::new("nocred-skills");
    let skills = target.display();
    let shared = Scratch::new("nocred-shared");
    let shared_path = shared.display();
    // `completions` writes where the shell looks, which is under the home
    // directory: give it a throwaway one rather than the developer's.
    let home = Scratch::new("nocred-home");

    let cases: Vec<Vec<&str>> = vec![
        vec!["config", "path"],
        vec!["config", "list"],
        vec!["config", "set", "search_timeout", "12"],
        vec!["config", "get", "search_timeout"],
        vec!["shares", "add", &shared_path],
        vec!["shares", "list"],
        vec!["shares", "remove", &shared_path],
        // The wishlist is stored, not searched, until `wish run` asks the
        // network — so managing it must not demand an account either.
        vec!["wish", "add", "something rare"],
        vec!["wish", "list"],
        vec!["wish", "remove", "something rare"],
        vec!["skills", "install", "--dir", &skills],
        vec!["skills", "list", "--dir", &skills],
        vec!["skills", "uninstall", "--dir", &skills],
        vec!["completions", "install", "--shell", "fish"],
        vec!["completions", "uninstall", "--shell", "fish"],
    ];

    for case in cases {
        let mut all = vec!["--config", config.to_str().expect("utf-8 path")];
        all.extend_from_slice(&case);
        let mut invocation = command(&all);
        invocation.env("HOME", home.path());
        invocation.env("XDG_CONFIG_HOME", home.path().join("config"));
        let output = invocation.output().expect("the binary should run");
        assert_eq!(
            code(&output),
            EXIT_OK,
            "`{}` should not need credentials, stderr: {}",
            case.join(" "),
            stderr(&output)
        );
    }

    // `portmap` is the thirteenth: it talks to the router, never to the
    // server, so it answers with its own verdict rather than a credentials
    // error.
    let portmap = with_config(&config, &["portmap"]);
    assert!(
        matches!(code(&portmap), EXIT_OK | EXIT_NO_RESULTS),
        "portmap should not need credentials, got {} / {}",
        code(&portmap),
        stderr(&portmap)
    );
}

/// "Every wait expressed in seconds is bounded to one day, so a mistyped flag
/// is rejected rather than hanging until the heat death of the universe."
#[test]
fn a_wait_longer_than_a_day_is_rejected_rather_than_accepted() {
    let day = 86_400;
    let over = (day + 1).to_string();
    let cases: Vec<Vec<&str>> = vec![
        vec!["--no-config", "--search-timeout", &over, "search", "x"],
        vec![
            "--no-config",
            "room",
            "listen",
            "lobby",
            "--duration",
            &over,
        ],
        vec!["--no-config", "message", "read", "--duration", &over],
        vec!["--no-config", "serve", "--duration", &over],
        vec!["--no-config", "get", "x", "--timeout", &over],
    ];

    // Missing credentials are a usage error too, so the exit code alone would
    // not prove the bound rejected anything: the range has to be named.
    for case in cases {
        let output = run(&case);
        assert_eq!(
            code(&output),
            EXIT_USAGE,
            "`{}` should be a usage error, stderr: {}",
            case.join(" "),
            stderr(&output)
        );
        assert!(
            stderr(&output).contains(&format!("1..={day}")),
            "`{}` should be rejected by the one-day bound, stderr: {}",
            case.join(" "),
            stderr(&output)
        );
    }
}

/// "`SOULSEEK_CONFIG_DIR` and `SOULSEEK_STATE_DIR` relocate the config and
/// state directories wholesale" — what a containerised run relies on.
#[test]
fn the_config_directory_can_be_relocated_wholesale() {
    let home = Scratch::new("relocated");
    let config_dir = home.path().join("config");
    let state_dir = home.path().join("state");

    let mut set = command(&["config", "set", "search_timeout", "17"]);
    set.env("SOULSEEK_CONFIG_DIR", &config_dir);
    set.env("SOULSEEK_STATE_DIR", &state_dir);
    let output = set.output().expect("the binary should run");
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let written = config_dir.join("config.toml");
    assert!(
        written.exists(),
        "the setting should land under SOULSEEK_CONFIG_DIR, not the default"
    );

    let mut path = command(&["config", "path"]);
    path.env("SOULSEEK_CONFIG_DIR", &config_dir);
    let reported = path.output().expect("the binary should run");
    assert!(
        records(&reported)[0].ends_with(written.to_str().expect("utf-8 path")),
        "got {:?}",
        records(&reported)
    );
}

/// The shipped SKILL.md tells an agent, key by key, what the offline commands
/// print. An agent has no way to notice a rename, so pin them here.
#[test]
fn the_offline_json_records_carry_the_keys_the_skill_file_names() {
    let dir = Scratch::new("skill-keys");
    let config = dir.path().join("config.toml");
    let shared = Scratch::new("skill-keys-shared");
    let shared_path = shared.display();
    let skills = Scratch::new("skill-keys-agent");
    let skills_path = skills.display();

    with_config(&config, &["shares", "add", &shared_path]);

    let cases: Vec<(Vec<&str>, Vec<&str>)> = vec![
        (
            vec!["--json", "shares", "list"],
            vec!["directory", "usable"],
        ),
        (
            vec!["--json", "skills", "list", "--dir", &skills_path],
            vec!["agent", "path", "action"],
        ),
        (vec!["--json", "config", "list"], vec!["key", "value"]),
        (vec!["--json", "config", "path"], vec!["key", "value"]),
    ];

    for (case, keys) in cases {
        let output = with_config(&config, &case);
        assert_record_keys(&output, &case.join(" "), &keys);
    }

    // `completions` writes into the shell's own directory, so it only ever
    // runs against a throwaway home.
    let home = Scratch::new("skill-keys-home");
    for verb in ["install", "uninstall"] {
        let mut invocation = command(&[
            "--no-config",
            "--json",
            "completions",
            verb,
            "--shell",
            "fish",
        ]);
        invocation.env("HOME", home.path());
        invocation.env("XDG_CONFIG_HOME", home.path().join("config"));
        let output = invocation.output().expect("the binary should run");
        assert_record_keys(
            &output,
            &format!("completions {verb}"),
            &["shell", "path", "action"],
        );
    }
}

/// The website's connectivity section: "`serve` refuses to start with the
/// listener off", because a source nobody can connect to serves nothing.
#[test]
fn serve_refuses_to_run_without_a_listener() {
    let share = Scratch::new("serve-nolistener");
    let output = run(&[
        "--no-config",
        "--username",
        "someone",
        "--password",
        "pw",
        "--no-listener",
        "--shared-dir",
        &share.display(),
        "serve",
        "--duration",
        "5",
    ]);
    assert_eq!(code(&output), EXIT_USAGE, "stdout: {}", stdout(&output));
    assert!(
        stderr(&output).contains("listener"),
        "the error should name the listener, got {}",
        stderr(&output)
    );
}

/// The four commands the README calls out as acting rather than reporting:
/// "say nothing on stdout and answer with their exit code alone".
#[test]
fn the_action_commands_answer_with_their_exit_code_alone() {
    let server = server_or_skip!();
    let shared = Scratch::new("action-shared");
    let shared_path = shared.display();
    let dir = Scratch::new("action-config");
    let config = dir.path().join("config.toml");
    let listener = server.client("cli_e2e_action_peer", Vec::new());
    listener.join_room("cli_e2e_action_room").expect("join");
    settle();

    for case in [
        vec!["room", "say", "cli_e2e_action_room", "hello room"],
        vec!["message", "send", "cli_e2e_action_peer", "hello there"],
    ] {
        let output = cli(&server, "cli_e2e_action", &case);
        assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));
        assert!(
            stdout(&output).is_empty(),
            "`{}` should print nothing on stdout, got {:?}",
            case.join(" "),
            stdout(&output)
        );
    }

    for case in [
        vec!["shares", "add", shared_path.as_str()],
        vec!["shares", "remove", shared_path.as_str()],
    ] {
        let output = with_config(&config, &case);
        assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));
        assert!(
            stdout(&output).is_empty(),
            "`{}` should print nothing on stdout, got {:?}",
            case.join(" "),
            stdout(&output)
        );
    }
}

/// The recipe the README leads with: `--min-bitrate` and `--free-slots` keep
/// only what a script asked for, rather than filtering nothing at all.
#[test]
fn search_filters_drop_what_they_advertise_dropping() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    std::fs::write(share.path().join("cli_probe_filt.bin"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_sharer_filt", vec![share.display()]);
    settle();

    let unfiltered = cli(
        &server,
        "cli_e2e_filterer",
        &["--search-timeout", "5", "--json", "search", "filt"],
    );
    assert_eq!(
        code(&unfiltered),
        EXIT_OK,
        "stderr: {}",
        stderr(&unfiltered)
    );
    assert!(
        records(&unfiltered)
            .iter()
            .any(|line| line.contains("cli_probe_filt")),
        "the probe should be findable before filtering"
    );

    // A .bin carries no bitrate, so a bitrate floor must exclude it rather
    // than let an unknown through.
    let filtered = cli(
        &server,
        "cli_e2e_filterer",
        &[
            "--search-timeout",
            "5",
            "search",
            "filt",
            "--min-bitrate",
            "320",
        ],
    );
    assert_eq!(
        code(&filtered),
        EXIT_NO_RESULTS,
        "a bitrate floor should drop a result with no bitrate, got {:?}",
        records(&filtered)
    );

    // `--free-slots` is a narrowing filter too: it may keep everything, but it
    // must never invent results the unfiltered search did not have.
    let free = cli(
        &server,
        "cli_e2e_filterer",
        &["--search-timeout", "5", "search", "filt", "--free-slots"],
    );
    assert!(
        matches!(code(&free), EXIT_OK | EXIT_NO_RESULTS),
        "stderr: {}",
        stderr(&free)
    );
    assert!(
        records(&free).len() <= records(&unfiltered).len(),
        "a filter must not add results"
    );
}

/// "Nothing sensitive on the command line or in the environment":
/// `--password-stdin` reads the first line of stdin, `docker login` style.
#[test]
fn the_password_can_arrive_on_stdin_instead_of_in_argv() {
    let server = server_or_skip!();
    let _owner = server.client("cli_e2e_stdin_pw", Vec::new());

    let args = [
        "--no-config",
        "--quiet",
        "--server",
        &server.address(),
        "--username",
        "cli_e2e_stdin_pw",
        "--password-stdin",
        "--no-listener",
        "whoami",
    ];
    let mut child = command(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary should run");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"pw\n")
        .expect("write the password");
    let output = child.wait_with_output().expect("the binary should finish");

    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));
    assert!(
        records(&output)[0].starts_with("cli_e2e_stdin_pw\t"),
        "got {:?}",
        records(&output)
    );
}

/// The `--json` paragraph names, field by field, what JSON carries that text
/// does not. A missing key silently breaks the `jq` filters the README sells.
#[test]
fn json_records_carry_every_extra_field_the_readme_names() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    std::fs::write(share.path().join("cli_probe_keys.bin"), probe_bytes())
        .expect("share file");
    let target = Scratch::new("downloads");
    let share_path = share.display();
    let sharer = server.client("cli_e2e_sharer_keys", vec![share.display()]);
    sharer.join_room("cli_e2e_keys_room").expect("join");
    settle();

    let mut expected: Vec<(Vec<&str>, Vec<&str>)> = vec![
        (
            vec!["search", "keys", "--search-timeout", "5"],
            vec![
                "user",
                "size",
                "bitrate",
                "path",
                "duration",
                "slots",
                "speed",
                "free_slot",
            ],
        ),
        (
            vec!["browse", "cli_e2e_sharer_keys"],
            vec!["user", "size", "path", "directory"],
        ),
        (
            vec!["whoami"],
            vec![
                "user",
                "server",
                "shared_folders",
                "shared_files",
                "listening",
                "listen_port",
                "download_dir",
            ],
        ),
        (
            vec!["user", "cli_e2e_sharer_keys"],
            vec![
                "user",
                "status",
                "average_speed",
                "shared_files",
                "privileged",
                "shared_folders",
            ],
        ),
        (
            vec!["shares", "status", "--shared-dir", &share_path],
            vec!["folders", "files", "directories"],
        ),
        (
            vec!["shares", "reindex", "--shared-dir", &share_path],
            vec!["folders", "files", "directories"],
        ),
    ];
    expected.push((vec!["room", "list"], vec!["users", "room"]));
    expected.push((
        vec!["room", "users", "cli_e2e_keys_room"],
        vec!["room", "user"],
    ));

    for (case, keys) in expected {
        let mut args: Vec<&str> = vec!["--json"];
        args.extend(case.iter().copied());
        let output = cli(&server, "cli_e2e_keys", &args);
        assert_record_keys(&output, &case.join(" "), &keys);
    }

    // `download` adds the user, remote path and size alongside the local file.
    let listing =
        cli(&server, "cli_e2e_keys", &["browse", "cli_e2e_sharer_keys"]);
    let remote_path = records(&listing)
        .into_iter()
        .find(|line| line.contains("cli_probe_keys.bin"))
        .and_then(|row| row.rsplit('\t').next().map(String::from))
        .expect("the shared file should be listed");
    // A fresh name: the peer connects back to the port this user announced at
    // login, and the browse runs above left an older one registered.
    let downloaded = cli(
        &server,
        "cli_e2e_keys_fetcher",
        &[
            "--json",
            "--download-dir",
            &target.display(),
            "download",
            "cli_e2e_sharer_keys",
            &remote_path,
            "--timeout",
            "40",
        ],
    );
    assert_record_keys(
        &downloaded,
        "download",
        &["file", "user", "path", "size"],
    );

    // `portmap` adds the port it probed; the verdict lives in the exit code,
    // which is `EXIT_NO_RESULTS` on a network with no cooperating router.
    let portmap = cli(&server, "cli_e2e_keys", &["--json", "portmap"]);
    let record: serde_json::Value =
        serde_json::from_str(records(&portmap).first().expect("one record"))
            .expect("valid JSON");
    for key in ["ok", "backend", "external", "port"] {
        assert!(record.get(key).is_some(), "portmap should carry `{key}`");
    }
}

// ---------------------------------------------------------------------------
// Several invocations at once: what an agent driving this CLI actually does.
// ---------------------------------------------------------------------------

/// Spawn the binary as `user` with an explicit listener port, without waiting.
fn spawn_cli(
    server: &TestServer,
    user: &str,
    port: u16,
    args: &[&str],
) -> std::process::Child {
    let mut all = server.args_on(user, port);
    all.extend(args.iter().map(|a| (*a).to_string()));
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    command(&refs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary should start")
}

#[test]
fn concurrent_searches_sharing_a_listener_port_all_find_the_file() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    std::fs::write(share.path().join("cli_probe_multi.bin"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_sharer_multi", vec![share.display()]);
    settle();

    // One configured port between them, which is what a config file gives
    // three agents started at the same moment. Losing the race for it used to
    // cost that invocation its listener, and with it every search result.
    let port = free_port().expect("shared listener port");
    // Collected on purpose: all three have to be running before any is waited
    // on, which is the whole point of the test.
    #[allow(clippy::needless_collect)]
    let running: Vec<_> = (0..3)
        .map(|i| {
            spawn_cli(
                &server,
                &format!("cli_e2e_par_{i}"),
                port,
                &["--search-timeout", "10", "search", "multi"],
            )
        })
        .collect();

    for (i, child) in running.into_iter().enumerate() {
        let output = child.wait_with_output().expect("wait");
        assert_eq!(
            code(&output),
            EXIT_OK,
            "run {i} failed: {}",
            stderr(&output)
        );
        assert!(
            records(&output)
                .iter()
                .any(|line| line.contains("cli_probe_multi")),
            "run {i} listed nothing"
        );
    }
}

#[test]
fn a_session_taken_over_mid_search_is_not_reported_as_no_results() {
    let server = server_or_skip!();
    let name = "cli_e2e_evicted";

    // Run it loud, so its own progress line says when it is logged in: the
    // eviction has to land after that, or this test would race the login it is
    // trying to interrupt.
    let mut args = server.args_on(name, free_port().expect("listener port"));
    args.retain(|arg| arg != "--quiet");
    args.extend(
        ["--search-timeout", "20", "search", "nothingmatchesthisqzx"]
            .map(String::from),
    );
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut searching = command(&refs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary should start");

    let (logged_in, wait_for_login) = std::sync::mpsc::channel();
    let stderr = searching.stderr.take().expect("piped stderr");
    let progress = std::thread::spawn(move || {
        let mut seen = String::new();
        for line in std::io::BufRead::lines(std::io::BufReader::new(stderr))
            .map_while(Result::ok)
        {
            if line.contains("logged in as") {
                let _ = logged_in.send(());
            }
            seen.push_str(&line);
            seen.push('\n');
        }
        seen
    });
    wait_for_login
        .recv_timeout(Duration::from_mins(1))
        .expect("the search should log in");

    // Now take the username away, the way a second invocation under one
    // account does.
    let _thief = server.client(name, Vec::new());

    let status = searching.wait().expect("wait");
    let seen = progress.join().expect("progress reader");
    assert_eq!(
        status.code(),
        Some(EXIT_SESSION_LOST),
        "a killed session must not look like an answered query: {seen}"
    );

    let mut written = String::new();
    std::io::Read::read_to_string(
        &mut searching.stdout.take().expect("piped stdout"),
        &mut written,
    )
    .expect("read stdout");
    assert!(written.trim().is_empty(), "stdout stays data-only");
}

#[test]
fn concurrent_downloads_sharing_a_listener_port_all_land() {
    let server = server_or_skip!();
    let share = Scratch::new("share");
    std::fs::write(share.path().join("cli_probe_dual.bin"), probe_bytes())
        .expect("share file");
    let _sharer = server.client("cli_e2e_sharer_dual", vec![share.display()]);
    settle();

    // A transfer arrives over a connection the *uploader* opens back to us, so
    // a run without a listener never receives its file however well its search
    // went. Two `get` runs, one configured port between them.
    let port = free_port().expect("shared listener port");
    let targets: Vec<Scratch> =
        (0..2).map(|i| Scratch::new(&format!("dl{i}"))).collect();
    // Collected on purpose: both have to be running before either is waited
    // on, which is the whole point of the test.
    #[allow(clippy::needless_collect)]
    let running: Vec<_> = targets
        .iter()
        .enumerate()
        .map(|(i, target)| {
            spawn_cli(
                &server,
                &format!("cli_e2e_dual_{i}"),
                port,
                &[
                    "--download-dir",
                    &target.display(),
                    "--search-timeout",
                    "10",
                    "get",
                    "dual",
                    "--timeout",
                    "40",
                ],
            )
        })
        .collect();

    for (i, (child, target)) in running.into_iter().zip(&targets).enumerate() {
        let output = child.wait_with_output().expect("wait");
        assert_eq!(
            code(&output),
            EXIT_OK,
            "run {i} failed: {}",
            stderr(&output)
        );
        let written = std::fs::read(target.path().join("cli_probe_dual.bin"))
            .unwrap_or_else(|e| panic!("run {i} wrote no file: {e}"));
        assert_eq!(written, probe_bytes(), "run {i} wrote the wrong bytes");
    }
}
