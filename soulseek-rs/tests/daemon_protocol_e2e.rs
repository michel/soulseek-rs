//! The control protocol as a third party meets it: a socket, JSON-RPC lines,
//! and the gate that decides who gets past `auth`.
//!
//! The CLI suite drives the daemon through this binary, which means it only
//! ever sends well-formed requests over a Unix socket. Everything here is the
//! other half — the TCP transport and its token, and what happens to a caller
//! who is hostile, broken, or simply not authenticated. None of it is
//! reachable through the command surface, and all of it is load-bearing: on
//! the socket, connecting *is* authenticating.
//!
//! Like the rest of the e2e suite this needs a soulfind server and skips
//! without one; `SOULSEEK_E2E_REQUIRED=1` turns that skip into a failure.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_soulseek-rs");

/// Every environment variable the binary reads, cleared so a developer's shell
/// cannot change what a test observes — and so no run finds a real daemon.
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

/// A daemon bound to loopback TCP as well as its socket, so both transports
/// can be exercised against one session.
struct Daemon {
    child: Child,
    state: PathBuf,
    port: u16,
    _server: Soulfind,
}

/// The binary with a scrubbed environment: no test may see the developer's
/// real daemon, and `state` decides which one it does see.
fn cli_command(state: &Path) -> Command {
    let mut command = Command::new(BIN);
    for name in CLI_ENV_VARS {
        command.env_remove(name);
    }
    command.env("SOULSEEK_STATE_DIR", state);
    command.current_dir(std::env::temp_dir());
    command
}

impl Daemon {
    fn start() -> Option<Self> {
        let server = Soulfind::start()?;
        let state = scratch("daemon-proto");
        let port = free_port()?;

        let mut command = cli_command(&state);
        command.args([
            "--no-config",
            "--server",
            &server.address(),
            "--username",
            "cli_e2e_proto",
            "--password",
            "pw",
            "--listener-port",
            &free_port()?.to_string(),
            "--download-dir",
            state.join("downloads").to_str()?,
            "daemon",
            "--bind",
            &format!("127.0.0.1:{port}"),
            // Exercised on every boot: the limit must parse and apply
            // without disturbing anything else the suite observes.
            "--upload-slots",
            "8",
        ]);
        command.stdout(Stdio::null()).stderr(Stdio::null());
        let child = command.spawn().ok()?;

        let daemon = Self {
            child,
            state,
            port,
            _server: server,
        };
        daemon.wait_until_ready()?;
        Some(daemon)
    }

    /// Ready means logged in, not merely bound: the daemon binds before it
    /// logs in, and a client that connects between the two would be asking
    /// about a session that is not there yet.
    fn wait_until_ready(&self) -> Option<()> {
        let deadline = Instant::now() + Duration::from_mins(1);
        while Instant::now() < deadline {
            if let Some(mut connection) = self.connect_tcp()
                && let Some(reply) = connection.call(&format!(
                    r#"{{"jsonrpc":"2.0","id":1,"method":"auth","params":{{"protocol":1,"token":"{}"}}}}"#,
                    self.token()
                ))
                && reply.contains("\"username\"")
            {
                return Some(());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        None
    }

    fn token(&self) -> String {
        std::fs::read_to_string(self.state.join("daemon.token"))
            .map(|token| token.trim().to_string())
            .unwrap_or_default()
    }

    fn socket_path(&self) -> PathBuf {
        self.state.join("daemon.sock")
    }

    fn connect_tcp(&self) -> Option<Connection> {
        TcpStream::connect(("127.0.0.1", self.port))
            .ok()
            .map(|stream| {
                let read = stream.try_clone().expect("a second handle");
                Connection::from_parts(Box::new(read), Box::new(stream))
            })
    }

    #[cfg(unix)]
    fn connect_socket(&self) -> Option<Connection> {
        let stream =
            std::os::unix::net::UnixStream::connect(self.socket_path()).ok()?;
        let read = stream.try_clone().expect("a second handle");
        Some(Connection::from_parts(Box::new(read), Box::new(stream)))
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.state);
    }
}

/// One control connection, spoken to by hand.
struct Connection {
    reader: BufReader<Box<dyn std::io::Read + Send>>,
    writer: Box<dyn Write + Send>,
}

impl Connection {
    /// Boxed halves, so one type covers both transports.
    fn from_parts(
        read: Box<dyn std::io::Read + Send>,
        write: Box<dyn Write + Send>,
    ) -> Self {
        Self {
            reader: BufReader::new(read),
            writer: write,
        }
    }

    /// Send one line and read the reply, `None` if the daemon hung up first.
    fn call(&mut self, line: &str) -> Option<String> {
        self.send(line)?;
        self.read()
    }

    fn send(&mut self, line: &str) -> Option<()> {
        writeln!(self.writer, "{line}").ok()?;
        self.writer.flush().ok()
    }

    fn read(&mut self) -> Option<String> {
        let mut reply = String::new();
        match self.reader.read_line(&mut reply) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(reply),
        }
    }

    fn authenticate(&mut self, token: Option<&str>) -> Option<String> {
        let params = match token {
            Some(token) => format!(r#"{{"protocol":1,"token":"{token}"}}"#),
            None => r#"{"protocol":1}"#.to_string(),
        };
        self.call(&format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"auth","params":{params}}}"#
        ))
    }
}

macro_rules! daemon_or_skip {
    () => {
        match Daemon::start() {
            Some(daemon) => daemon,
            None => {
                let required = std::env::var("SOULSEEK_E2E_REQUIRED")
                    .is_ok_and(|v| v != "0" && !v.is_empty());
                assert!(
                    !required,
                    "SOULSEEK_E2E_REQUIRED is set but no daemon could be \
                     started (set SOULFIND_BIN)"
                );
                println!(
                    "daemon protocol e2e skipped: no soulfind server (set \
                     SOULFIND_BIN to run)"
                );
                return;
            }
        }
    };
}

#[test]
fn tcp_needs_the_token_and_the_socket_does_not() {
    let daemon = daemon_or_skip!();

    let mut anonymous = daemon.connect_tcp().expect("connect");
    let refused = anonymous.authenticate(None).expect("an answer");
    assert!(
        refused.contains("\"error\""),
        "TCP without a token must be refused: {refused}"
    );

    let mut wrong = daemon.connect_tcp().expect("connect");
    let bad = wrong
        .authenticate(Some(&"0".repeat(64)))
        .expect("an answer");
    assert!(
        bad.contains("\"error\""),
        "a wrong token must be refused: {bad}"
    );

    let mut good = daemon.connect_tcp().expect("connect");
    let accepted = good.authenticate(Some(&daemon.token())).expect("an answer");
    assert!(
        accepted.contains("\"result\"") && accepted.contains("cli_e2e_proto"),
        "the right token must be accepted: {accepted}"
    );

    #[cfg(unix)]
    {
        // The socket's permissions have already answered the question.
        let mut local = daemon.connect_socket().expect("connect");
        let reply = local.authenticate(None).expect("an answer");
        assert!(
            reply.contains("\"result\""),
            "a Unix socket needs no token: {reply}"
        );
    }
}

#[test]
fn nothing_can_be_asked_before_authenticating() {
    let daemon = daemon_or_skip!();
    let mut connection = daemon.connect_tcp().expect("connect");

    let reply = connection
        .call(r#"{"jsonrpc":"2.0","id":1,"method":"daemon.status"}"#)
        .expect("an answer");
    assert!(
        reply.contains("\"error\""),
        "a request before auth must be refused: {reply}"
    );
    assert!(
        !reply.contains("cli_e2e_proto"),
        "an unauthenticated caller must learn nothing about the session: \
         {reply}"
    );
    assert!(
        connection
            .call(r#"{"jsonrpc":"2.0","id":2,"method":"daemon.status"}"#)
            .is_none(),
        "the connection must be dropped, not left open to try again"
    );
}

#[test]
fn an_error_carries_an_id_so_a_client_can_match_it() {
    let daemon = daemon_or_skip!();
    let mut connection = daemon.connect_tcp().expect("connect");
    connection
        .authenticate(Some(&daemon.token()))
        .expect("authenticated");

    let reply = connection
        .call(r#"{"jsonrpc":"2.0","id":7,"method":"no.such.method"}"#)
        .expect("an answer");
    assert!(reply.contains("\"id\":7"), "{reply}");
    assert!(reply.contains("-32601"), "{reply}");

    // A malformed line still gets an id — null — rather than looking like a
    // notification the client should ignore.
    let malformed = connection.call("{ this is not json").expect("an answer");
    assert!(malformed.contains("\"id\":null"), "{malformed}");
    assert!(malformed.contains("-32700"), "{malformed}");
}

#[test]
fn a_notification_is_answered_with_silence() {
    let daemon = daemon_or_skip!();
    let mut connection = daemon.connect_tcp().expect("connect");
    connection
        .authenticate(Some(&daemon.token()))
        .expect("authenticated");

    // No id, so no reply — the next line read must be the answer to the
    // request that follows it, not a reply to this one.
    connection
        .send(r#"{"jsonrpc":"2.0","method":"no.such.method"}"#)
        .expect("sent");
    let reply = connection
        .call(r#"{"jsonrpc":"2.0","id":9,"method":"daemon.status"}"#)
        .expect("an answer");
    assert!(
        reply.contains("\"id\":9"),
        "a notification must not produce a reply of its own: {reply}"
    );
}

#[test]
fn an_oversized_line_cannot_make_the_daemon_allocate() {
    let daemon = daemon_or_skip!();
    let mut connection = daemon.connect_tcp().expect("connect");

    // Over the 1 MiB cap, and never terminated: an unauthenticated peer must
    // not be able to make the daemon buffer without bound.
    let flood = "x".repeat(2 * 1024 * 1024);
    let _ = connection.send(&flood);
    assert!(
        connection.read().is_none(),
        "an oversized request must end the connection"
    );

    // The daemon is still serving everyone else.
    let mut healthy = daemon.connect_tcp().expect("connect");
    let reply = healthy
        .authenticate(Some(&daemon.token()))
        .expect("an answer");
    assert!(reply.contains("\"result\""), "{reply}");
}

#[test]
fn a_silent_connection_is_closed_rather_than_held_open() {
    let daemon = daemon_or_skip!();
    let mut connection = daemon.connect_tcp().expect("connect");

    // Says nothing at all. The handshake timeout is 30s; well inside a minute.
    let deadline = Instant::now() + Duration::from_mins(1);
    let mut closed = false;
    while Instant::now() < deadline {
        if connection.read().is_none() {
            closed = true;
            break;
        }
    }
    assert!(
        closed,
        "a peer that connects and never authenticates must not hold a thread \
         forever"
    );
}

#[test]
fn discover_serves_a_contract_whose_parameters_match_what_is_accepted() {
    let daemon = daemon_or_skip!();
    let mut connection = daemon.connect_tcp().expect("connect");
    connection
        .authenticate(Some(&daemon.token()))
        .expect("authenticated");

    let reply = connection
        .call(r#"{"jsonrpc":"2.0","id":1,"method":"rpc.discover"}"#)
        .expect("an answer");
    let document: serde_json::Value =
        serde_json::from_str(&reply).expect("valid JSON");
    let methods = document["result"]["methods"]
        .as_array()
        .expect("the contract lists methods");

    // The published parameter names have to be the keys the daemon reads.
    // Published as one descriptor named `params`, a generated client would
    // send {"params":{"params":{…}}} and every call would fail -32602 — which
    // is exactly what this asserts cannot happen.
    let join = methods
        .iter()
        .find(|m| m["name"] == "room.join")
        .expect("room.join is published");
    let names: Vec<&str> = join["params"]
        .as_array()
        .expect("params is an array")
        .iter()
        .map(|p| p["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(names, ["room"], "{join}");

    // And a call built from that description is accepted.
    let accepted = connection
        .call(
            r#"{"jsonrpc":"2.0","id":2,"method":"room.join","params":{"room":"cli_e2e_proto_room"}}"#,
        )
        .expect("an answer");
    assert!(
        accepted.contains("\"result\""),
        "a call shaped by the published contract must be accepted: {accepted}"
    );
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct Soulfind {
    child: Option<Child>,
    db: PathBuf,
    port: u16,
}

impl Soulfind {
    fn start() -> Option<Self> {
        let bin = soulfind_binary()?;
        let port = free_port()?;
        let db = std::env::temp_dir().join(format!("soulfind-proto-{port}.db"));
        let _ = std::fs::remove_file(&db);

        let child = Command::new(bin)
            .args(["-d", db.to_str()?, "-p", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(Self {
                    child: Some(child),
                    db,
                    port,
                });
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let mut child = child;
        let _ = child.kill();
        let _ = child.wait();
        None
    }

    fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

impl Drop for Soulfind {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_file(&self.db);
    }
}

fn soulfind_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SOULFIND_BIN") {
        let path = PathBuf::from(path);
        return path.exists().then_some(path);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
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

fn scratch(label: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU32 =
        std::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir()
        .join(format!("soulseek-{label}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&path).expect("scratch directory");
    path
}

/// A client configured for a remote daemon reports the *daemon's* world.
///
/// This is the download-box workflow: the token lives in the client's config
/// file rather than on its command line, and it has no Soulseek account of its
/// own. Reading the token from the flag alone made the status probe fail
/// silently, and every command that names a path or a server then quietly fell
/// back to this machine's defaults — `whoami` claiming downloads land in the
/// laptop's home directory when they land on the box.
#[test]
fn a_config_driven_remote_client_reports_the_daemons_world() {
    let daemon = daemon_or_skip!();
    let config = scratch("remote-client-config");
    let state = scratch("remote-client-state");

    // A laptop that knows only where the daemon is. No username, no password,
    // no server — exactly what the documented setup produces.
    std::fs::write(
        config.join("config.toml"),
        format!(
            "daemon = \"127.0.0.1:{}\"\ndaemon_token = \"{}\"\n",
            daemon.port,
            daemon.token()
        ),
    )
    .expect("config file");

    let mut command = cli_command(&state);
    command.env("SOULSEEK_CONFIG_DIR", &config);
    command.args(["--json", "whoami"]);
    let output = command.output().expect("the binary should run");

    assert!(
        output.status.success(),
        "a client configured for a remote daemon should just work: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find(|line| !line.trim().is_empty())
            .expect("one record"),
    )
    .expect("valid JSON");

    assert_eq!(
        record["user"], "cli_e2e_proto",
        "the daemon's account, not this machine's"
    );
    assert!(
        record["server"]
            .as_str()
            .is_some_and(|server| server.starts_with("127.0.0.1:")),
        "the server the daemon is connected to, not a local default: {record}"
    );
    assert!(
        record["download_dir"].as_str().is_some_and(
            |dir| dir.starts_with(std::env::temp_dir().to_str().unwrap())
        ),
        "the directory the bytes actually land in, on the daemon's host, not \
         this machine's Downloads folder: {record}"
    );

    let _ = std::fs::remove_dir_all(&config);
    let _ = std::fs::remove_dir_all(&state);
}

/// Several clients at once, all seeing the same session.
///
/// This is the promise daemon mode makes and the reason the fan-out exists:
/// the server allows one login, so the alternative to sharing is displacing.
/// Each client gets its own connection, its own event queue, and the same
/// answers.
#[test]
fn many_clients_share_one_session_and_agree_about_it() {
    let daemon = daemon_or_skip!();

    // Held open together, not one after another: the point is concurrency.
    let mut clients: Vec<Connection> = (0..8)
        .map(|_| {
            let mut connection = daemon.connect_tcp().expect("connect");
            let reply = connection
                .authenticate(Some(&daemon.token()))
                .expect("an answer");
            assert!(reply.contains("\"result\""), "client rejected: {reply}");
            connection
        })
        .collect();

    // Every one of them is told the same thing about the same session.
    let mut seen = Vec::new();
    for (i, client) in clients.iter_mut().enumerate() {
        let reply = client
            .call(&format!(
                r#"{{"jsonrpc":"2.0","id":{},"method":"daemon.status"}}"#,
                100 + i
            ))
            .unwrap_or_else(|| panic!("client {i} lost its connection"));
        let status: serde_json::Value =
            serde_json::from_str(&reply).expect("valid JSON");
        assert_eq!(
            status["id"],
            serde_json::json!(100 + i),
            "replies must not be crossed between clients: {reply}"
        );
        seen.push((
            status["result"]["username"].clone(),
            status["result"]["server"].clone(),
        ));
    }
    assert!(
        seen.windows(2).all(|pair| pair[0] == pair[1]),
        "every client should describe the same session: {seen:?}"
    );

    // And the daemon counts them all, rather than having quietly dropped some.
    let reply = clients[0]
        .call(r#"{"jsonrpc":"2.0","id":200,"method":"daemon.status"}"#)
        .expect("still connected");
    let status: serde_json::Value =
        serde_json::from_str(&reply).expect("valid JSON");
    assert_eq!(
        status["result"]["clients"],
        serde_json::json!(8),
        "all eight should still be attached: {reply}"
    );
}

/// Connecting and dropping, over and over, must not degrade the daemon.
///
/// Every command makes at least one throwaway connection while it works out
/// whether a daemon is there, so a per-connection resource that is not
/// released shows up as a daemon that works for a while and then refuses
/// everyone.
#[test]
fn rapid_connect_and_drop_does_not_wear_the_daemon_out() {
    let daemon = daemon_or_skip!();

    // Comfortably more than the connection cap, so a leak of one slot per
    // connection would have exhausted it well before the end.
    for _ in 0..200 {
        drop(daemon.connect_tcp().expect("connect"));
    }
    // Half-open ones too: authenticated, then abandoned mid-conversation.
    for _ in 0..50 {
        let mut connection = daemon.connect_tcp().expect("connect");
        connection.authenticate(Some(&daemon.token()));
        drop(connection);
    }

    // Give the daemon a moment to notice the hangups, then prove it is well.
    std::thread::sleep(Duration::from_secs(2));
    let mut connection = daemon.connect_tcp().expect("connect");
    let reply = connection
        .authenticate(Some(&daemon.token()))
        .expect("the daemon should still be answering");
    assert!(reply.contains("\"result\""), "{reply}");
    let status = connection
        .call(r#"{"jsonrpc":"2.0","id":1,"method":"daemon.status"}"#)
        .expect("an answer");
    let parsed: serde_json::Value =
        serde_json::from_str(&status).expect("valid JSON");
    assert_eq!(
        parsed["result"]["clients"],
        serde_json::json!(1),
        "abandoned connections must not still be counted: {status}"
    );
}

/// What one client does, the other sees.
///
/// Two TUIs on one daemon are two views of a single session, not two sessions
/// that happen to share a login. A search run in one appears in the other's
/// list, and a transfer queued in one appears in the other's queue — which is
/// what makes closing one window harmless.
#[test]
fn one_clients_work_shows_up_in_another() {
    let daemon = daemon_or_skip!();
    let mut first = daemon.connect_tcp().expect("connect");
    first.authenticate(Some(&daemon.token())).expect("first in");
    let mut second = daemon.connect_tcp().expect("connect");
    second
        .authenticate(Some(&daemon.token()))
        .expect("second in");

    // Nothing searched yet, so neither sees anything.
    let queries = |connection: &mut Connection, id: u32| -> Vec<String> {
        let reply = connection
            .call(&format!(
                r#"{{"jsonrpc":"2.0","id":{id},"method":"search.list"}}"#
            ))
            .expect("an answer");
        let parsed: serde_json::Value =
            serde_json::from_str(&reply).expect("valid JSON");
        parsed["result"]["searches"]
            .as_array()
            .expect("searches is an array")
            .iter()
            .filter_map(|search| search["query"].as_str().map(String::from))
            .collect()
    };
    assert!(!queries(&mut second, 1).contains(&"shared_probe".to_string()));

    // The first client searches.
    first
        .call(
            r#"{"jsonrpc":"2.0","id":2,"method":"search.start","params":{"query":"shared_probe"}}"#,
        )
        .expect("an answer");

    // The second client sees that search without having run it.
    assert!(
        queries(&mut second, 3).contains(&"shared_probe".to_string()),
        "a search in one window must appear in the other"
    );

    // And its results are the same set, because there is one search cache.
    let results = |connection: &mut Connection, id: u32| -> usize {
        let reply = connection
            .call(&format!(
                r#"{{"jsonrpc":"2.0","id":{id},"method":"search.results","params":{{"query":"shared_probe"}}}}"#
            ))
            .expect("an answer");
        let parsed: serde_json::Value =
            serde_json::from_str(&reply).expect("valid JSON");
        parsed["result"]["results"]
            .as_array()
            .expect("results is an array")
            .len()
    };
    assert_eq!(
        results(&mut first, 4),
        results(&mut second, 5),
        "both windows read one search cache"
    );
}

/// A client that attaches later starts from the session's state.
///
/// This is what makes a second window useful at all: it opens showing what is
/// already going on rather than a blank slate that fills only with what it
/// does itself.
#[test]
fn a_client_that_attaches_later_sees_what_is_already_there() {
    let daemon = daemon_or_skip!();

    // An early client does some work.
    let mut early = daemon.connect_tcp().expect("connect");
    early.authenticate(Some(&daemon.token())).expect("in");
    early
        .call(
            r#"{"jsonrpc":"2.0","id":1,"method":"search.start","params":{"query":"earlier_probe"}}"#,
        )
        .expect("an answer");
    early
        .call(
            r#"{"jsonrpc":"2.0","id":2,"method":"room.join","params":{"room":"cli_e2e_late_room"}}"#,
        )
        .expect("an answer");

    // A window opened afterwards is told about it.
    let mut late = daemon.connect_tcp().expect("connect");
    late.authenticate(Some(&daemon.token())).expect("in");

    let reply = late
        .call(r#"{"jsonrpc":"2.0","id":3,"method":"search.list"}"#)
        .expect("an answer");
    let parsed: serde_json::Value =
        serde_json::from_str(&reply).expect("valid JSON");
    let mut queries = parsed["result"]["searches"]
        .as_array()
        .expect("searches is an array")
        .iter()
        .filter_map(|search| search["query"].as_str());
    assert!(
        queries.any(|query| query == "earlier_probe"),
        "a window that opens later must see the searches already run: {reply}"
    );

    // The transfer queue is the session's, so both read the same one.
    for (connection, id) in [(&mut early, 4), (&mut late, 5)] {
        let reply = connection
            .call(&format!(
                r#"{{"jsonrpc":"2.0","id":{id},"method":"download.list"}}"#
            ))
            .expect("an answer");
        let parsed: serde_json::Value =
            serde_json::from_str(&reply).expect("valid JSON");
        assert!(
            parsed["result"]["downloads"].is_array(),
            "both windows read one queue: {reply}"
        );
    }
}

/// The download folder is a session setting, settable over the wire.
///
/// `download.start` deliberately refuses a per-transfer destination, so
/// `download.set_dir` is the only way a remote owner can move where bytes
/// land — and `daemon.status` has to report the move, because that status is
/// what every attached client trusts for "where did my file go".
#[test]
#[cfg(unix)]
fn the_download_folder_moves_when_asked_and_status_says_so() {
    let daemon = daemon_or_skip!();
    let mut connection = daemon.connect_socket().expect("socket");
    connection.authenticate(None).expect("an answer");

    let moved = scratch("moved-downloads").join("deeper");
    let reply = connection
        .call(&format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"download.set_dir","params":{{"directory":{}}}}}"#,
            serde_json::json!(moved.to_str().expect("utf-8 path"))
        ))
        .expect("an answer");
    assert!(reply.contains(r#""ok":true"#), "got {reply}");
    assert!(
        moved.is_dir(),
        "the daemon creates the folder it will download into"
    );

    let status = connection
        .call(r#"{"jsonrpc":"2.0","id":3,"method":"daemon.status"}"#)
        .expect("an answer");
    let parsed: serde_json::Value =
        serde_json::from_str(&status).expect("valid JSON");
    assert_eq!(
        parsed["result"]["download_dir"],
        moved.to_str().expect("utf-8 path"),
        "status must name the folder transfers now land in: {status}"
    );

    // An empty path is refused as bad parameters, not applied as "here".
    let refused = connection
        .call(
            r#"{"jsonrpc":"2.0","id":4,"method":"download.set_dir","params":{"directory":"  "}}"#,
        )
        .expect("an answer");
    assert!(refused.contains("-32602"), "got {refused}");
    let _ = std::fs::remove_dir_all(moved.parent().expect("parent"));
}

/// A wishlist search must read as *running* to the other windows.
///
/// The daemon can only tell a running search from a finished one by the start
/// time it recorded; a wishlist search that never records one renders as
/// completed everywhere the moment it goes out.
#[test]
#[cfg(unix)]
fn a_wishlist_search_is_reported_as_just_started() {
    let daemon = daemon_or_skip!();
    let mut connection = daemon.connect_socket().expect("socket");
    connection.authenticate(None).expect("an answer");

    let sent = connection
        .call(
            r#"{"jsonrpc":"2.0","id":2,"method":"search.wishlist","params":{"query":"wishlist_probe"}}"#,
        )
        .expect("an answer");
    assert!(sent.contains(r#""ok":true"#), "got {sent}");

    let listed = connection
        .call(r#"{"jsonrpc":"2.0","id":3,"method":"search.list"}"#)
        .expect("an answer");
    let parsed: serde_json::Value =
        serde_json::from_str(&listed).expect("valid JSON");
    let age = parsed["result"]["searches"]
        .as_array()
        .expect("searches is an array")
        .iter()
        .find(|search| search["query"] == "wishlist_probe")
        .and_then(|search| search["started_secs_ago"].as_u64())
        .expect("the wishlist search is listed with an age");
    assert!(
        age < 60,
        "a search sent moments ago must read as just started, got {age}s"
    );
}

/// The upload-slot limit is retunable on a live session.
#[test]
fn the_upload_slot_limit_can_be_retuned_live() {
    let daemon = daemon_or_skip!();
    let mut connection = daemon.connect_tcp().expect("tcp");
    connection
        .authenticate(Some(&daemon.token()))
        .expect("an answer");
    let reply = connection
        .call(
            r#"{"jsonrpc":"2.0","id":2,"method":"upload.slots","params":{"slots":3}}"#,
        )
        .expect("an answer");
    assert!(reply.contains(r#""ok":true"#), "got {reply}");
}

/// A wrong or missing token is the CLI's problem to explain, not just the
/// wire's: the user typed `--daemon-token`, so the failure has to come back
/// as a connection error rather than a hang or a fallback to local defaults.
#[test]
fn the_cli_reports_a_bad_token_as_a_connection_error() {
    let daemon = daemon_or_skip!();
    let state = scratch("cli-bad-token");
    let run = |token: Option<&str>| {
        let mut command = cli_command(&state);
        command.args([
            "--no-config",
            "--daemon",
            &format!("127.0.0.1:{}", daemon.port),
        ]);
        if let Some(token) = token {
            command.args(["--daemon-token", token]);
        }
        command.arg("whoami");
        command.output().expect("the binary should run")
    };

    let wrong = run(Some(&"0".repeat(64)));
    assert_eq!(
        wrong.status.code(),
        Some(3),
        "a rejected token is a connection error: {}",
        String::from_utf8_lossy(&wrong.stderr)
    );
    assert!(
        String::from_utf8_lossy(&wrong.stdout).trim().is_empty(),
        "a failing command must write nothing to stdout"
    );

    // No token at all: this state dir never ran a daemon, so there is no
    // cookie file to fall back on, and TCP without a token is a hard no.
    let missing = run(None);
    assert_eq!(
        missing.status.code(),
        Some(3),
        "TCP with no token to offer is a connection error: {}",
        String::from_utf8_lossy(&missing.stderr)
    );

    // The right token still works from the same machine state.
    let right = run(Some(&daemon.token()));
    assert_eq!(
        right.status.code(),
        Some(0),
        "the real token must keep working: {}",
        String::from_utf8_lossy(&right.stderr)
    );
    let _ = std::fs::remove_dir_all(&state);
}
