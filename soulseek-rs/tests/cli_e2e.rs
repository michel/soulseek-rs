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

/// Every environment variable the CLI reads, cleared for each run so a
/// developer's shell (or a stray `.env`) cannot change what a test observes.
const CLI_ENV_VARS: [&str; 11] = [
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
fn command(args: &[&str]) -> Command {
    let mut command = Command::new(BIN);
    for name in CLI_ENV_VARS {
        command.env_remove(name);
    }
    command.current_dir(std::env::temp_dir());
    command.args(args);
    command
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
        "search", "download", "get", "browse", "room", "message", "portmap",
    ] {
        assert!(help.contains(command), "--help should mention {command}");
    }
}

#[test]
fn help_documents_the_exit_codes_a_script_branches_on() {
    let help = stdout(&run(&["--help"]));
    assert!(help.contains("Exit codes"), "help should list exit codes");
    for code in ["2", "3", "4", "5", "6"] {
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
            free_port().expect("listener port").to_string(),
        ]
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
fn cli(server: &TestServer, user: &str, args: &[&str]) -> Output {
    let mut all = server.args(user);
    all.extend(args.iter().map(|a| (*a).to_string()));
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    run(&refs)
}

/// Wait out the SetWaitPort registrations so peer lookups resolve.
fn settle() {
    std::thread::sleep(Duration::from_secs(1));
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

    let output = child.wait_with_output().expect("the binary should finish");
    assert_eq!(code(&output), EXIT_OK, "stderr: {}", stderr(&output));

    let heard = records(&output).iter().any(|line| {
        serde_json::from_str::<serde_json::Value>(line)
            .is_ok_and(|value| value["message"] == "streamed line")
    });
    assert!(heard, "stdout was: {:?}", stdout(&output));
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
