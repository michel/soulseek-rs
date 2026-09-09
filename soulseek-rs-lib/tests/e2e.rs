//! End-to-end integration tests against a real Soulseek server (soulfind).
//!
//! These tests are SERVER-OPTIONAL so `cargo test` stays green everywhere:
//!   * If `SOULSEEK_TEST_SERVER=host:port` is set, they connect to it.
//!   * Else if a soulfind binary is found (via `SOULFIND_BIN`, or a sibling
//!     `../soulfind/bin/soulfind` checkout), they spawn it on an ephemeral port
//!     with a throwaway database.
//!   * Else they SKIP with a printed notice (the test still passes).
//!
//! To run them against a locally built soulfind:
//! ```sh
//! SOULFIND_BIN=/path/to/soulfind \
//!   cargo test -p soulseek-rs-lib --test e2e -- --nocapture
//! ```
//!
//! The harness sticks to `std` and the library's own public API.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use soulseek_rs::message::Message;
use soulseek_rs::message::distributed;
use soulseek_rs::message::server::MessageFactory;
use soulseek_rs::{
    Client, ClientSettings, ClientVersion, ConnectionType, DownloadStatus,
    PeerAddress, SessionLoss, UploadStatus,
};

/// Only one test at a time may drive a server.
///
/// Each of these tests runs its own soulfind plus real clients and, for the
/// queue tests, peer sockets held open on purpose. Letting the harness start a
/// handful of those at once starves them, and a login that goes unanswered
/// fails a test for a reason that has nothing to do with the code under test:
/// seen on CI as `login: Timeout` in two download tests that the change under
/// test never went near. The CLI suite already carries this gate for the same
/// reason.
static SERVER_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A Soulseek server to test against: either a child soulfind process we
/// spawned, or an external server referenced by `SOULSEEK_TEST_SERVER`.
struct TestServer {
    host: String,
    port: u16,
    child: Option<Child>,
    db: Option<PathBuf>,
    _gate: std::sync::MutexGuard<'static, ()>,
}

impl TestServer {
    /// Resolve a server to test against, or `None` if the suite should skip.
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
        Self::spawn(gate)
    }

    /// Spawn a local soulfind on an ephemeral port with a throwaway database.
    fn spawn(gate: std::sync::MutexGuard<'static, ()>) -> Option<Self> {
        let bin = soulfind_binary()?;
        let port = free_port()?;
        let db = std::env::temp_dir().join(format!("soulfind-e2e-{port}.db"));
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
            // Server never came up (e.g. a toolchain/SQLite issue); skip.
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

    fn settings(&self, username: &str, password: &str) -> ClientSettings {
        ClientSettings {
            username: username.to_string(),
            password: password.to_string(),
            server_address: PeerAddress::new(self.host.clone(), self.port),
            enable_listen: false,
            listen_port: 0,
            shared_directories: Vec::new(),
            accept_children: false,
            version: ClientVersion::default(),
        }
    }

    /// Settings with the peer listener enabled on `port`, exercising the
    /// `SetWaitPort` step of the post-login handshake.
    fn listening_settings(
        &self,
        username: &str,
        password: &str,
        port: u16,
    ) -> ClientSettings {
        ClientSettings {
            enable_listen: true,
            listen_port: port,
            ..self.settings(username, password)
        }
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
    if let Ok(p) = std::env::var("SOULFIND_BIN") {
        let p = PathBuf::from(p);
        return p.exists().then_some(p);
    }
    // Fall back to a `soulfind/bin/soulfind` checkout somewhere above this
    // workspace (the workspace may be nested, e.g. under `.../src/rust/`).
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .map(|dir| dir.join("soulfind/bin/soulfind"))
        .find(|candidate| candidate.exists())
}

fn free_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
    listener.local_addr().ok().map(|addr| addr.port())
}

fn wait_until_listening(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Option<()> {
    let addr = format!("{host}:{port}");
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(mut addrs) = addr.to_socket_addrs()
            && let Some(sa) = addrs.next()
            && TcpStream::connect_timeout(&sa, Duration::from_millis(200))
                .is_ok()
        {
            return Some(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

/// Resolve a test server or return early with a skip notice.
///
/// Set `SOULSEEK_E2E_REQUIRED=1` (as CI does) to turn a missing server into a
/// hard failure instead of a silent skip, so the suite genuinely runs there.
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
                    "e2e skipped: no soulfind server (set SOULFIND_BIN or \
                     SOULSEEK_TEST_SERVER to run)"
                );
                return;
            }
        }
    };
}

#[test]
fn connect_and_login_succeed() {
    let server = server_or_skip!();
    let mut client =
        Client::with_settings(server.settings("e2e_user", "e2e_pw"));
    client.connect().expect("connect to soulfind");
    let logged_in = client.login().expect("login to soulfind");
    assert!(logged_in, "login should succeed (soulfind auto-registers)");
}

#[test]
fn search_round_trips_without_error() {
    let server = server_or_skip!();
    let mut client =
        Client::with_settings(server.settings("e2e_search", "e2e_pw"));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    // A fresh server has no shared files, so the search simply has to
    // round-trip without error and leave an (empty) queryable result set.
    let query = "nonexistent query xyzzy";
    let _ = client.search(query, Duration::from_secs(2));
    assert!(client.get_search_results(query).is_empty());

    // The search must also be tracked in client state under its key, proving
    // the request was actually registered and not silently dropped.
    assert!(
        client.get_all_searches().contains_key(query),
        "the issued search should be registered under its query key"
    );
}

#[test]
fn a_search_is_forwarded_to_a_connected_peer() {
    let server = server_or_skip!();

    // The server distributes each search to other connected users, so a second
    // client exercises the *incoming* FileSearch handler. We can't observe that
    // handler's state directly, but if it mishandled the forwarded bytes it
    // would take the receiver's session down — so we prove the receiver is
    // still alive afterwards by round-tripping its own search.
    let mut searcher =
        Client::with_settings(server.settings("e2e_searcher", "pw"));
    let mut receiver =
        Client::with_settings(server.settings("e2e_receiver", "pw"));
    searcher.connect().expect("searcher connect");
    receiver.connect().expect("receiver connect");
    assert!(searcher.login().expect("searcher login"));
    assert!(receiver.login().expect("receiver login"));

    let _ = searcher.search("some shared song", Duration::from_secs(2));

    // Receiver stays functional after handling the forwarded search.
    let probe = "receiver still alive";
    let _ = receiver.search(probe, Duration::from_secs(2));
    assert!(receiver.get_all_searches().contains_key(probe));
}

#[test]
fn a_private_message_is_delivered_between_users() {
    let server = server_or_skip!();

    // Two logged-in users, one messages the other through the server.
    let mut alice =
        Client::with_settings(server.settings("e2e_alice_pm", "pw"));
    let mut bob = Client::with_settings(server.settings("e2e_bob_pm", "pw"));
    alice.connect().expect("alice connect");
    bob.connect().expect("bob connect");
    assert!(alice.login().expect("alice login"));
    assert!(bob.login().expect("bob login"));

    let body = "hello bob, this is alice";
    alice
        .send_private_message("e2e_bob_pm", body)
        .expect("send private message");

    // Delivery is asynchronous; poll Bob's inbox until the message arrives.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut received = Vec::new();
    while Instant::now() < deadline {
        received.extend(bob.take_private_messages());
        if !received.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let message = received
        .iter()
        .find(|m| m.message() == body)
        .expect("bob should receive alice's message");
    assert_eq!(message.username(), "e2e_alice_pm");
}

#[test]
fn a_chat_room_message_is_delivered_between_users() {
    use soulseek_rs::types::RoomEvent;
    let server = server_or_skip!();

    let room = "e2e_room_chat";
    let mut alice =
        Client::with_settings(server.settings("e2e_alice_room", "pw"));
    let mut bob = Client::with_settings(server.settings("e2e_bob_room", "pw"));
    alice.connect().expect("alice connect");
    bob.connect().expect("bob connect");
    assert!(alice.login().expect("alice login"));
    assert!(bob.login().expect("bob login"));

    alice.join_room(room).expect("alice joins room");
    bob.join_room(room).expect("bob joins room");

    // Give both joins time to register on the server before speaking.
    std::thread::sleep(Duration::from_millis(500));
    let _ = alice.take_room_events();
    let _ = bob.take_room_events();

    let body = "hello room, this is alice";
    alice.say_in_room(room, body).expect("alice says in room");

    // Delivery is asynchronous; poll Bob's room events until the message lands.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = None;
    while Instant::now() < deadline {
        for event in bob.take_room_events() {
            if let RoomEvent::Message {
                room: r,
                username,
                message,
            } = event
                && r == room
                && message == body
            {
                got = Some(username);
            }
        }
        if got.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert_eq!(
        got.as_deref(),
        Some("e2e_alice_room"),
        "bob should receive alice's room message"
    );
}

#[test]
fn the_room_list_includes_a_joined_room() {
    let server = server_or_skip!();

    let room = "e2e_room_listed";
    let mut alice =
        Client::with_settings(server.settings("e2e_alice_list", "pw"));
    alice.connect().expect("alice connect");
    assert!(alice.login().expect("alice login"));
    alice.join_room(room).expect("alice joins room");

    // Once a user is in the room the server should advertise it in RoomList.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut listed = false;
    while Instant::now() < deadline {
        alice.request_room_list().expect("request room list");
        std::thread::sleep(Duration::from_millis(250));
        if alice.room_list().iter().any(|r| r.name == room) {
            listed = true;
            break;
        }
    }
    assert!(listed, "the joined room should appear in the room list");
}

#[test]
fn login_succeeds_with_listener_enabled() {
    let server = server_or_skip!();

    // With the listener enabled the client also sends SetWaitPort during the
    // post-login handshake; the server must accept it and keep the session.
    let port = free_port().expect("free listener port");
    let mut client = Client::with_settings(server.listening_settings(
        "e2e_listener",
        "pw",
        port,
    ));
    client.connect().expect("connect with listener");
    assert!(
        client.login().expect("login with listener enabled"),
        "the handshake including SetWaitPort should still log in"
    );
}

#[test]
fn wrong_password_is_rejected() {
    let server = server_or_skip!();

    // soulfind auto-registers a username on first login and binds it to that
    // password, so a second login with a different password must be rejected.
    let user = "e2e_pw_user";
    let mut first =
        Client::with_settings(server.settings(user, "correct-horse"));
    first.connect().expect("connect (registering login)");
    assert!(
        first.login().expect("first login"),
        "registration should log in"
    );
    drop(first);

    let mut second =
        Client::with_settings(server.settings(user, "wrong-password"));
    second.connect().expect("connect (wrong password)");
    // The server may signal rejection either as a non-success status or as an
    // authentication error; both mean "not logged in", only `Ok(true)` accepts.
    assert!(
        !matches!(second.login(), Ok(true)),
        "a mismatched password must not be accepted"
    );
}

#[test]
fn registered_username_can_relogin_with_same_password() {
    let server = server_or_skip!();

    // The TUI's stored-credentials flow depends on this pair of server
    // behaviors: a fresh username is registered by simply logging in, and a
    // later session (a "restart") with the same credentials is accepted.
    let user = "e2e_relogin_user";
    let mut first = Client::with_settings(server.settings(user, "pw-123"));
    first.connect().expect("connect (registering login)");
    assert!(
        first.login().expect("registering login"),
        "a fresh username should be auto-registered"
    );
    drop(first);

    let mut second = Client::with_settings(server.settings(user, "pw-123"));
    second.connect().expect("connect (relogin)");
    assert!(
        second.login().expect("relogin"),
        "the same credentials must log in again after a restart"
    );
}

// ---------------------------------------------------------------------------
// Peer-to-peer download coverage.
//
// The client cannot serve files (the upload side is not implemented), so a real
// download is exercised with a minimal in-process "mock uploader" that speaks
// the peer protocol using the library's own public `Message` wire format. It
// drives the same path a real peer would after a search: a `P` control
// connection to our listener, a `QueueUpload` → `TransferRequest` →
// `TransferResponse` negotiation, then an `F` connection that streams the bytes.
// ---------------------------------------------------------------------------

/// Configuration for a one-shot mock uploader.
struct MockUpload {
    listen_addr: String,
    peer_username: String,
    filename: String,
    content: Vec<u8>,
    token: u32,
    ready: Sender<()>,
    token_delay: Duration,
}

/// Build a `PeerInit` (peer code 1) frame: `[len][1][username][conn_type][token]`.
fn peer_init_bytes(username: &str, conn_type: &str, token: u32) -> Vec<u8> {
    let mut m = Message::new();
    m.write_int8(1)
        .write_string(username)
        .write_string(conn_type)
        .write_int32(token);
    m.get_buffer()
}

/// Read one length-prefixed peer message (`[len:4 LE][payload]`) from a blocking
/// stream. The returned `Message` keeps the length prefix, so `get_message_code`
/// and `set_pointer(8)` behave exactly as they do inside the library.
fn read_framed(stream: &mut TcpStream) -> std::io::Result<Message> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;
    let mut data = len_buf.to_vec();
    data.extend_from_slice(&payload);
    Ok(Message::new_with_data(data))
}

fn connect_retry(addr: &str, timeout: Duration) -> std::io::Result<TcpStream> {
    let deadline = Instant::now() + timeout;
    loop {
        match TcpStream::connect(addr) {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                if Instant::now() >= deadline {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn open_control_and_await_queue(
    cfg: &MockUpload,
) -> std::io::Result<TcpStream> {
    let mut p = connect_retry(&cfg.listen_addr, Duration::from_secs(5))?;
    p.set_read_timeout(Some(Duration::from_secs(10)))?;
    p.write_all(&peer_init_bytes(&cfg.peer_username, "P", 0))?;
    p.flush()?;
    let _ = cfg.ready.send(());

    loop {
        let msg = read_framed(&mut p)?;
        if msg.get_message_code() == 43 {
            break;
        }
    }
    Ok(p)
}

/// Build a TransferRequest (peer code 40). The size becomes the download's
/// expected size, so it must match the content.
fn transfer_request(cfg: &MockUpload) -> Message {
    let mut tr = Message::new();
    tr.write_int32(40)
        .write_int32(1) // direction: upload
        .write_int32(cfg.token)
        .write_string(&cfg.filename)
        .write_int64(cfg.content.len() as u64);
    tr
}

fn run_mock_uploader(cfg: &MockUpload) -> std::io::Result<u64> {
    let mut p = open_control_and_await_queue(cfg)?;

    p.write_all(&transfer_request(cfg).get_buffer())?;
    p.flush()?;

    // Wait for the downloader to allow it (TransferResponse, peer code 41).
    loop {
        let msg = read_framed(&mut p)?;
        if msg.get_message_code() == 41 {
            break;
        }
    }

    serve_file_over_f(
        &cfg.listen_addr,
        &cfg.peer_username,
        cfg.token,
        &cfg.content,
        cfg.token_delay,
    )
}

fn run_mock_uploader_f_first(cfg: &MockUpload) -> std::io::Result<u64> {
    let mut p = open_control_and_await_queue(cfg)?;

    let listen_addr = cfg.listen_addr.clone();
    let peer_username = cfg.peer_username.clone();
    let token = cfg.token;
    let content = cfg.content.clone();
    let token_delay = cfg.token_delay;
    let f = std::thread::spawn(move || {
        serve_file_over_f(
            &listen_addr,
            &peer_username,
            token,
            &content,
            token_delay,
        )
    });

    std::thread::sleep(Duration::from_millis(300));

    p.write_all(&transfer_request(cfg).get_buffer())?;
    p.flush()?;

    f.join()
        .map_err(|_| std::io::Error::other("F connection thread panicked"))?
}

fn run_mock_slow_uploader(
    cfg: &MockUpload,
    stall_after: Option<usize>,
) -> std::io::Result<usize> {
    let mut p = open_control_and_await_queue(cfg)?;
    p.write_all(&transfer_request(cfg).get_buffer())?;
    p.flush()?;
    expect_code(&mut p, 41, Duration::from_secs(10))?;

    let mut f = connect_retry(&cfg.listen_addr, Duration::from_secs(5))?;
    f.set_read_timeout(Some(Duration::from_secs(10)))?;
    f.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut init = peer_init_bytes(&cfg.peer_username, "F", cfg.token);
    init.extend_from_slice(&cfg.token.to_le_bytes());
    f.write_all(&init)?;
    let mut start = [0u8; 8];
    f.read_exact(&mut start)?;

    let mut sent = 0;
    for (n, chunk) in cfg.content.chunks(4096).enumerate() {
        if stall_after == Some(n) {
            f.set_read_timeout(Some(Duration::from_secs(15)))?;
            match f.read(&mut [0u8; 1]) {
                Ok(0) => {
                    return Err(std::io::Error::other(
                        "the downloader hung up",
                    ));
                }
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(e),
            }
        }
        f.write_all(chunk)?;
        f.flush()?;
        sent += chunk.len();
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(sent)
}

fn run_mock_uploader_offering_after(
    cfg: &MockUpload,
    queued: &Sender<()>,
    go: &Receiver<()>,
) -> std::io::Result<(bool, String)> {
    let mut p = open_control_and_await_queue(cfg)?;
    let _ = queued.send(());
    go.recv()
        .map_err(|_| std::io::Error::other("the test never said go"))?;

    p.write_all(&transfer_request(cfg).get_buffer())?;
    p.flush()?;
    let mut msg = expect_code(&mut p, 41, Duration::from_secs(10))?;
    msg.set_pointer(8);
    let _token = msg.read_int32();
    let allowed = msg.read_bool();
    let reason = if allowed {
        String::new()
    } else {
        msg.read_string()
    };
    Ok((allowed, reason))
}

fn run_refusing_uploader(
    cfg: &MockUpload,
    refusal: &Message,
) -> std::io::Result<()> {
    let mut p = open_control_and_await_queue(cfg)?;

    p.write_all(&refusal.get_buffer())?;
    p.flush()?;
    std::thread::sleep(Duration::from_millis(500));
    Ok(())
}

/// Open an F (file transfer) connection to `downloader_addr` and stream
/// `content`; the downloader sends an 8-byte START_DOWNLOAD offset before we
/// send the bytes.
///
/// Like a real peer, only the bytes past that offset are sent. Returns the
/// offset the downloader asked for.
fn serve_file_over_f(
    downloader_addr: &str,
    username: &str,
    token: u32,
    content: &[u8],
    token_delay: Duration,
) -> std::io::Result<u64> {
    let mut f = connect_retry(downloader_addr, Duration::from_secs(5))?;
    f.set_read_timeout(Some(Duration::from_secs(10)))?;
    if token_delay.is_zero() {
        let mut init = peer_init_bytes(username, "F", token);
        init.extend_from_slice(&token.to_le_bytes());
        f.write_all(&init)?;
    } else {
        f.write_all(&peer_init_bytes(username, "F", token))?;
        std::thread::sleep(token_delay);
        f.write_all(&token.to_le_bytes())?;
    }
    f.flush()?;

    let mut start = [0u8; 8];
    f.read_exact(&mut start)?;
    let offset = u64::from_le_bytes(start);
    f.write_all(&content[offset as usize..])?;
    f.flush()?;

    // Keep the connection open briefly so the reader drains everything.
    std::thread::sleep(Duration::from_millis(500));
    Ok(offset)
}

/// Log a raw socket in to the server and drain up to the login response,
/// returning the still-open stream (the user stays online while it lives).
fn login_raw(
    server_addr: &str,
    username: &str,
    password: &str,
) -> std::io::Result<TcpStream> {
    let mut srv = connect_retry(server_addr, Duration::from_secs(5))?;
    srv.set_read_timeout(Some(Duration::from_secs(10)))?;
    srv.write_all(
        &MessageFactory::build_login_message(
            username,
            password,
            ClientVersion::default(),
        )
        .get_buffer(),
    )?;
    srv.flush()?;
    loop {
        let msg = read_framed(&mut srv)?;
        if msg.get_message_code() == 1 {
            break;
        }
    }
    Ok(srv)
}

/// Read framed messages from `stream` until one has `code`, or the deadline
/// passes. Returns the matching message.
fn read_until_code(
    stream: &mut TcpStream,
    code: u32,
    timeout: Duration,
) -> Option<Message> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match read_framed(stream) {
            Ok(msg) if msg.get_message_code() == code => return Some(msg),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

/// Like [`read_until_code`] but turns a miss into an `io::Error` for `?`.
fn expect_code(
    stream: &mut TcpStream,
    code: u32,
    timeout: Duration,
) -> std::io::Result<Message> {
    read_until_code(stream, code, timeout).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("timed out waiting for message code {code}"),
        )
    })
}

/// Wait for a download to reach `Completed`, watching both its status channel
/// and the client's download list (whichever reports first). Returns false if it
/// fails, times out, or the deadline passes.
fn wait_for_completion(
    client: &Client,
    status_rx: &Receiver<DownloadStatus>,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match status_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(DownloadStatus::Completed) => return true,
            Ok(DownloadStatus::Failed(_) | DownloadStatus::TimedOut) => {
                return false;
            }
            _ => {}
        }
        if client
            .get_all_downloads()
            .iter()
            .any(|d| matches!(d.status, DownloadStatus::Completed))
        {
            return true;
        }
    }
    false
}

fn unique_download_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "soulseek-e2e-dl-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn file_downloads_over_p_and_f(
    peer_username: &str,
    token_delay: Duration,
    run_uploader: fn(&MockUpload) -> std::io::Result<u64>,
) {
    let server = server_or_skip!();

    // Downloader: connected to the server with its peer listener enabled.
    let listen_port = free_port().expect("free listen port");
    let mut client = Client::with_settings(server.listening_settings(
        &format!("{peer_username}_dl"),
        "pw",
        listen_port,
    ));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    let filename = "mock_song.mp3";
    let content: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
    let size = content.len() as u64;
    let token = 424_242_u32;
    let download_dir = unique_download_dir();

    // Start the mock uploader; it signals once its P connection is established.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let cfg = MockUpload {
        listen_addr: format!("127.0.0.1:{listen_port}"),
        peer_username: peer_username.to_string(),
        filename: filename.to_string(),
        content: content.clone(),
        token,
        ready: ready_tx,
        token_delay,
    };
    let uploader = std::thread::spawn(move || {
        if let Err(e) = run_uploader(&cfg) {
            eprintln!("[mock uploader] {e}");
        }
    });

    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("mock uploader P connection");

    // The listener registers an incoming peer under its plain username; give
    // that registration a moment to complete before queuing the download.
    std::thread::sleep(Duration::from_millis(1500));

    let (_download, status_rx) = client
        .download(
            filename.to_string(),
            peer_username.to_string(),
            size,
            download_dir.display().to_string(),
        )
        .expect("start download");

    let completed =
        wait_for_completion(&client, &status_rx, Duration::from_secs(20));
    let _ = uploader.join();

    assert!(completed, "the download should reach Completed");

    let written = std::fs::read(download_dir.join(filename))
        .expect("downloaded file should exist");
    assert_eq!(written, content, "downloaded bytes should match the source");

    let _ = std::fs::remove_dir_all(&download_dir);
}

#[test]
fn a_file_downloads_from_a_peer_over_p_and_f_connections() {
    file_downloads_over_p_and_f(
        "e2e_mockpeer",
        Duration::ZERO,
        run_mock_uploader,
    );
}

#[test]
fn a_file_downloads_when_the_transfer_token_arrives_late() {
    file_downloads_over_p_and_f(
        "e2e_latepeer",
        Duration::from_millis(300),
        run_mock_uploader,
    );
}

#[test]
fn a_download_completes_when_the_f_connection_beats_token_registration() {
    file_downloads_over_p_and_f(
        "e2e_earlypeer",
        Duration::ZERO,
        run_mock_uploader_f_first,
    );
}

fn cancel_mid_transfer(peer_username: &str, stall_after: Option<usize>) {
    let server = server_or_skip!();

    let listen_port = free_port().expect("free listen port");
    let mut client = Client::with_settings(server.listening_settings(
        &format!("{peer_username}_dl"),
        "pw",
        listen_port,
    ));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    let filename = "cancel_me.mp3";
    let content: Vec<u8> = (0..400_000u32).map(|i| (i % 251) as u8).collect();
    let download_dir = unique_download_dir();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let cfg = MockUpload {
        listen_addr: format!("127.0.0.1:{listen_port}"),
        peer_username: peer_username.to_string(),
        filename: filename.to_string(),
        content: content.clone(),
        token: 424_250_u32,
        ready: ready_tx,
        token_delay: Duration::ZERO,
    };
    let uploader =
        std::thread::spawn(move || run_mock_slow_uploader(&cfg, stall_after));
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("mock uploader P connection");
    std::thread::sleep(Duration::from_millis(1500));

    let (_download, status_rx) = client
        .download(
            filename.to_string(),
            peer_username.to_string(),
            content.len() as u64,
            download_dir.display().to_string(),
        )
        .expect("start download");

    let part = download_dir.join(format!("{filename}.part"));
    assert!(
        wait_for(|| part.exists()),
        "the transfer should be streaming"
    );
    if stall_after.is_some() {
        std::thread::sleep(Duration::from_secs(2));
    }
    let cancelled_at = Instant::now();
    assert!(client.cancel_download(peer_username, filename));

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut cancelled = false;
    while Instant::now() < deadline {
        match status_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(DownloadStatus::Cancelled) => {
                cancelled = true;
                break;
            }
            Ok(DownloadStatus::Completed | DownloadStatus::Failed(_)) => break,
            _ => {}
        }
    }
    assert!(cancelled, "the status channel should report the cancel");
    assert!(wait_for(|| !part.exists()), "the partial file should go");
    assert!(!download_dir.join(filename).exists());
    assert!(client.get_all_downloads().iter().any(|d| {
        d.filename == filename && matches!(d.status, DownloadStatus::Cancelled)
    }));

    let outcome = uploader.join().expect("mock uploader thread");
    assert!(
        outcome.is_err(),
        "the uploader should see its connection drop, got {outcome:?}"
    );
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(8),
        "the socket should be released within seconds, not at the read \
         deadline"
    );
    assert!(
        status_rx.try_recv().is_err(),
        "nothing follows Cancelled on the channel"
    );
    let _ = std::fs::remove_dir_all(&download_dir);
}

#[test]
fn a_cancelled_download_stops_and_drops_its_partial_file() {
    cancel_mid_transfer("e2e_cancel_peer", None);
}

#[test]
fn a_cancelled_stalled_download_is_released_promptly() {
    cancel_mid_transfer("e2e_stalled_peer", Some(2));
}

#[test]
fn a_cancelled_queued_download_declines_the_peers_offer() {
    let server = server_or_skip!();

    let listen_port = free_port().expect("free listen port");
    let mut client = Client::with_settings(server.listening_settings(
        "e2e_cancel_queued_dl",
        "pw",
        listen_port,
    ));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    let filename = "never_starts.mp3";
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (queued_tx, queued_rx) = std::sync::mpsc::channel();
    let (go_tx, go_rx) = std::sync::mpsc::channel();
    let cfg = MockUpload {
        listen_addr: format!("127.0.0.1:{listen_port}"),
        peer_username: "e2e_cancel_queued_peer".to_string(),
        filename: filename.to_string(),
        content: vec![0; 10],
        token: 424_251_u32,
        ready: ready_tx,
        token_delay: Duration::ZERO,
    };
    let uploader = std::thread::spawn(move || {
        run_mock_uploader_offering_after(&cfg, &queued_tx, &go_rx)
    });
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("mock uploader P connection");
    std::thread::sleep(Duration::from_millis(1500));

    let download_dir = unique_download_dir();
    let (_download, status_rx) = client
        .download(
            filename.to_string(),
            "e2e_cancel_queued_peer".to_string(),
            10,
            download_dir.display().to_string(),
        )
        .expect("start download");
    queued_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the peer should receive our queue request");

    assert!(client.cancel_download("e2e_cancel_queued_peer", filename));
    assert!(matches!(
        status_rx.recv_timeout(Duration::from_secs(2)),
        Ok(DownloadStatus::Cancelled)
    ));

    go_tx.send(()).expect("mock uploader waiting");
    let (allowed, reason) = uploader
        .join()
        .expect("mock uploader thread")
        .expect("the peer should get an answer to its offer");
    assert!(
        !allowed,
        "a cancelled download must not accept the transfer"
    );
    assert_eq!(reason, "Cancelled");
    assert!(client.get_all_downloads().iter().any(|d| {
        d.filename == filename && matches!(d.status, DownloadStatus::Cancelled)
    }));
    assert!(!download_dir.join(format!("{filename}.part")).exists());
    let _ = std::fs::remove_dir_all(&download_dir);
}

fn refusal_fails_the_download_quickly(
    peer_username: &str,
    build_refusal: impl FnOnce(&str) -> Message,
) {
    let server = server_or_skip!();

    let listen_port = free_port().expect("free listen port");
    let mut client = Client::with_settings(server.listening_settings(
        &format!("{peer_username}_dl"),
        "pw",
        listen_port,
    ));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    let filename = "gone_song.mp3";
    let refusal = build_refusal(filename);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let cfg = MockUpload {
        listen_addr: format!("127.0.0.1:{listen_port}"),
        peer_username: peer_username.to_string(),
        filename: filename.to_string(),
        content: Vec::new(),
        token: 0,
        ready: ready_tx,
        token_delay: Duration::ZERO,
    };
    let uploader = std::thread::spawn(move || {
        if let Err(e) = run_refusing_uploader(&cfg, &refusal) {
            eprintln!("[mock refusing uploader] {e}");
        }
    });

    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("mock uploader P connection");
    std::thread::sleep(Duration::from_millis(1500));

    let download_dir = unique_download_dir();
    let (_download, status_rx) = client
        .download(
            filename.to_string(),
            peer_username.to_string(),
            0,
            download_dir.display().to_string(),
        )
        .expect("start download");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut failed = false;
    while Instant::now() < deadline {
        match status_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(DownloadStatus::Failed(_)) => {
                failed = true;
                break;
            }
            Ok(DownloadStatus::Completed) => {
                panic!("the refused download must not complete")
            }
            _ => {}
        }
    }
    let _ = uploader.join();
    let _ = std::fs::remove_dir_all(&download_dir);

    assert!(failed, "the peer's refusal should fail the download fast");
}

#[test]
fn an_upload_failed_reply_fails_the_download_quickly() {
    refusal_fails_the_download_quickly("e2e_failing_peer", |filename| {
        let mut refusal = Message::new();
        refusal.write_int32(46).write_string(filename);
        refusal
    });
}

#[test]
fn an_upload_denied_reply_fails_the_download_quickly() {
    refusal_fails_the_download_quickly("e2e_denying_peer", |filename| {
        let mut refusal = Message::new();
        refusal
            .write_int32(50)
            .write_string(filename)
            .write_string("File not shared.");
        refusal
    });
}

#[test]
fn an_interrupted_download_resumes_from_its_partial_file() {
    let server = server_or_skip!();

    let listen_port = free_port().expect("free listen port");
    let mut client = Client::with_settings(server.listening_settings(
        "e2e_resume_dl",
        "pw",
        listen_port,
    ));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    let filename = "resumed_file.bin";
    let content: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
    let size = content.len() as u64;
    let already_have = 800usize;
    let download_dir = unique_download_dir();

    // Stand in for a transfer that died after 800 of 2000 bytes.
    std::fs::write(
        download_dir.join(format!("{filename}.part")),
        &content[..already_have],
    )
    .expect("seed the partial file");

    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let cfg = MockUpload {
        listen_addr: format!("127.0.0.1:{listen_port}"),
        peer_username: "e2e_resume_peer".to_string(),
        filename: filename.to_string(),
        content: content.clone(),
        token: 424_243_u32,
        ready: ready_tx,
        token_delay: Duration::ZERO,
    };
    let (offset_tx, offset_rx) = std::sync::mpsc::channel();
    let uploader = std::thread::spawn(move || match run_mock_uploader(&cfg) {
        Ok(offset) => {
            let _ = offset_tx.send(offset);
        }
        Err(e) => eprintln!("[mock uploader] {e}"),
    });

    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("mock uploader P connection");
    std::thread::sleep(Duration::from_millis(1500));

    let (_download, status_rx) = client
        .download(
            filename.to_string(),
            "e2e_resume_peer".to_string(),
            size,
            download_dir.display().to_string(),
        )
        .expect("start download");

    let completed =
        wait_for_completion(&client, &status_rx, Duration::from_secs(20));
    let _ = uploader.join();

    assert!(completed, "the resumed download should reach Completed");
    assert_eq!(
        offset_rx.recv_timeout(Duration::from_secs(1)),
        Ok(already_have as u64),
        "the peer should be asked to start past the bytes we already had"
    );
    assert_eq!(
        std::fs::read(download_dir.join(filename))
            .expect("downloaded file should exist"),
        content,
        "the resumed prefix and the fetched tail should form the whole file"
    );
    assert!(
        !download_dir.join(format!("{filename}.part")).exists(),
        "the .part should be renamed away once complete"
    );

    let _ = std::fs::remove_dir_all(&download_dir);
}

#[test]
fn soulfind_brokers_connect_to_peer_between_users() {
    // The firewalled download mode relies on the server forwarding a
    // ConnectToPeer request to the target user. Confirm soulfind does this: a
    // requester (with a wait port so the server knows its address) asks the
    // server to broker a connection to an online target, and the target must
    // receive a forwarded ConnectToPeer (server code 18) naming the requester.
    let server = server_or_skip!();
    let addr = format!("{}:{}", server.host, server.port);

    let mut target =
        login_raw(&addr, "e2e_broker_target", "pw").expect("target login");

    let mut requester =
        login_raw(&addr, "e2e_broker_req", "pw").expect("requester login");
    let req_port = free_port().expect("free port");
    requester
        .write_all(
            &MessageFactory::build_set_wait_port_message(req_port).get_buffer(),
        )
        .expect("set wait port");
    requester.flush().expect("flush wait port");

    let token = 987_654_u32;
    requester
        .write_all(
            &MessageFactory::build_connect_to_peer(
                token,
                "e2e_broker_target",
                ConnectionType::P,
            )
            .get_buffer(),
        )
        .expect("send ConnectToPeer");
    requester.flush().expect("flush ConnectToPeer");

    let mut brokered = read_until_code(&mut target, 18, Duration::from_secs(5))
        .expect("target should receive a brokered ConnectToPeer");
    brokered.set_pointer(8);
    assert_eq!(
        brokered.read_string(),
        "e2e_broker_req",
        "the brokered message should name the requester"
    );
}

// ---------------------------------------------------------------------------
// Direct-connection download: the client initiates the peer connection.
//
// Here the mock is a server-registered peer. It logs in to soulfind and
// advertises a listen port (SetWaitPort), so when our client asks the server
// for the peer's address (GetPeerAddress) and dials it directly, the mock
// accepts that inbound P connection and then serves the file. This exercises
// the outbound PeerInit handshake and the auto-connecting download() path.
// ---------------------------------------------------------------------------

struct MockDirectUpload {
    server_addr: String,
    username: String,
    password: String,
    listen_port: u16,
    downloader_listen_addr: String,
    downloader_username: String,
    filename: String,
    content: Vec<u8>,
    token: u32,
    ready: Sender<()>,
    /// Hang up the control connection before streaming, the way clients that
    /// drop idle peer sockets do while a transfer runs on its own connection.
    close_control_first: bool,
}

fn run_mock_direct_peer(cfg: &MockDirectUpload) -> std::io::Result<()> {
    // 1. Log in to the server so it knows this user is online.
    let mut srv = connect_retry(&cfg.server_addr, Duration::from_secs(5))?;
    srv.set_read_timeout(Some(Duration::from_secs(10)))?;
    srv.write_all(
        &MessageFactory::build_login_message(
            &cfg.username,
            &cfg.password,
            ClientVersion::default(),
        )
        .get_buffer(),
    )?;
    srv.flush()?;
    loop {
        let msg = read_framed(&mut srv)?;
        if msg.get_message_code() == 1 {
            break; // login response
        }
    }

    // 2. Bind the peer listener, then advertise its port so the server can hand
    //    our address to the downloader. Bind all interfaces: soulfind reports
    //    the host's LAN address (not 127.0.0.1), and the downloader dials that.
    let listener = std::net::TcpListener::bind(("0.0.0.0", cfg.listen_port))?;
    srv.write_all(
        &MessageFactory::build_set_wait_port_message(cfg.listen_port)
            .get_buffer(),
    )?;
    srv.flush()?;
    let _ = cfg.ready.send(());

    // 3. Accept the downloader's inbound P (control) connection and validate its
    //    PeerInit (peer code 1, int8 code so the fields start at offset 5). The
    //    accept is bounded so a misrouted connection fails the test instead of
    //    hanging it.
    listener.set_nonblocking(true)?;
    let accept_deadline = Instant::now() + Duration::from_secs(15);
    let (mut p, _addr) = loop {
        match listener.accept() {
            Ok(pair) => break pair,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= accept_deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "no inbound P connection from the downloader",
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    };
    p.set_nonblocking(false)?;
    p.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut init = read_framed(&mut p)?;
    assert_eq!(init.get_init_code(), 1, "expected inbound PeerInit");
    init.set_pointer(5);
    assert_eq!(init.read_string(), cfg.downloader_username, "PeerInit user");
    assert_eq!(init.read_string(), "P", "PeerInit connection type");

    // 4. Negotiate the transfer exactly as in the passive path.
    loop {
        let mut msg = read_framed(&mut p)?;
        if msg.get_message_code() == 43 {
            msg.set_pointer(8);
            let _requested = msg.read_string();
            break;
        }
    }
    let mut tr = Message::new();
    tr.write_int32(40)
        .write_int32(1)
        .write_int32(cfg.token)
        .write_string(&cfg.filename)
        .write_int64(cfg.content.len() as u64);
    p.write_all(&tr.get_buffer())?;
    p.flush()?;
    loop {
        let msg = read_framed(&mut p)?;
        if msg.get_message_code() == 41 {
            break; // TransferResponse
        }
    }

    if cfg.close_control_first {
        drop(p);
        // Give the downloader time to notice the hangup before the bytes move,
        // so the test measures what it does about it rather than outrunning it.
        std::thread::sleep(Duration::from_secs(2));
    }

    // 5. Stream the bytes over an F connection to the downloader's listener.
    //    `srv` stays in scope so the peer remains online for the whole transfer.
    serve_file_over_f(
        &cfg.downloader_listen_addr,
        &cfg.username,
        cfg.token,
        &cfg.content,
        Duration::ZERO,
    )
    .map(|_| ())
}

#[test]
fn a_file_downloads_from_a_peer_via_direct_connection() {
    let server = server_or_skip!();

    // Downloader with a listener enabled (needed for the F leg).
    let client_port = free_port().expect("free client listen port");
    let mut client = Client::with_settings(server.listening_settings(
        "e2e_direct_dl",
        "pw",
        client_port,
    ));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    let mock_port = free_port().expect("free mock listen port");
    let filename = "direct_song.mp3";
    let content: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
    let size = content.len() as u64;
    let token = 515_151_u32;
    let download_dir = unique_download_dir();

    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let cfg = MockDirectUpload {
        server_addr: format!("{}:{}", server.host, server.port),
        username: "e2e_directpeer".to_string(),
        password: "pw".to_string(),
        listen_port: mock_port,
        downloader_listen_addr: format!("127.0.0.1:{client_port}"),
        downloader_username: "e2e_direct_dl".to_string(),
        filename: filename.to_string(),
        content: content.clone(),
        token,
        ready: ready_tx,
        close_control_first: false,
    };
    let uploader = std::thread::spawn(move || {
        if let Err(e) = run_mock_direct_peer(&cfg) {
            eprintln!("[mock direct peer] {e}");
        }
    });

    ready_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("mock direct peer ready");
    // Let the server finish processing SetWaitPort before we resolve the address.
    std::thread::sleep(Duration::from_secs(1));

    // Target the plain username so download() takes the direct-connect path.
    let (_download, status_rx) = client
        .download(
            filename.to_string(),
            "e2e_directpeer".to_string(),
            size,
            download_dir.display().to_string(),
        )
        .expect("start download");

    let completed =
        wait_for_completion(&client, &status_rx, Duration::from_secs(20));
    let _ = uploader.join();

    assert!(completed, "the direct download should reach Completed");

    let written = std::fs::read(download_dir.join(filename))
        .expect("downloaded file should exist");
    assert_eq!(written, content, "downloaded bytes should match the source");

    let _ = std::fs::remove_dir_all(&download_dir);
}

// ---------------------------------------------------------------------------
// Firewalled download: the peer is unreachable directly, so the connection is
// brokered through the server.
//
// The mock advertises a port nobody listens on, so the downloader's direct
// connection fails. The client then asks the server to broker the connection
// (ConnectToPeer); the mock, reading its server stream, sees the forwarded
// request and connects back to the downloader with a PierceFirewall. That
// pierced connection becomes the P control channel, and the file is served.
// ---------------------------------------------------------------------------

struct MockFirewalledUpload {
    server_addr: String,
    username: String,
    password: String,
    bogus_port: u16,
    downloader_listen_addr: String,
    filename: String,
    content: Vec<u8>,
    token: u32,
    ready: Sender<()>,
}

fn run_mock_firewalled_peer(cfg: &MockFirewalledUpload) -> std::io::Result<()> {
    // 1. Log in and advertise a port that nobody listens on, so the downloader's
    //    direct connection is refused and it falls back to server brokering.
    let mut srv = login_raw(&cfg.server_addr, &cfg.username, &cfg.password)?;
    srv.write_all(
        &MessageFactory::build_set_wait_port_message(cfg.bogus_port)
            .get_buffer(),
    )?;
    srv.flush()?;
    let _ = cfg.ready.send(());

    // 2. Wait for the server-brokered ConnectToPeer (server code 18) and read
    //    the correlation token (after username, type, ip and port).
    let mut ctp = expect_code(&mut srv, 18, Duration::from_secs(15))?;
    ctp.set_pointer(8);
    let _who = ctp.read_string();
    let _conn_type = ctp.read_string();
    let _ip = ctp.read_int32();
    let _port = ctp.read_int32();
    let connect_token = ctp.read_int32();

    // 3. Connect back to the downloader with a PierceFirewall (peer code 0);
    //    this becomes the P control connection.
    let mut p =
        connect_retry(&cfg.downloader_listen_addr, Duration::from_secs(5))?;
    p.set_read_timeout(Some(Duration::from_secs(10)))?;
    p.write_all(
        &MessageFactory::build_pierce_firewall_message(connect_token)
            .get_buffer(),
    )?;
    p.flush()?;

    // 4. Negotiate the transfer over the pierced connection.
    let _queue = expect_code(&mut p, 43, Duration::from_secs(10))?;
    let mut tr = Message::new();
    tr.write_int32(40)
        .write_int32(1)
        .write_int32(cfg.token)
        .write_string(&cfg.filename)
        .write_int64(cfg.content.len() as u64);
    p.write_all(&tr.get_buffer())?;
    p.flush()?;
    let _response = expect_code(&mut p, 41, Duration::from_secs(10))?;

    // 5. Serve the bytes over an F connection to the downloader's listener.
    serve_file_over_f(
        &cfg.downloader_listen_addr,
        &cfg.username,
        cfg.token,
        &cfg.content,
        Duration::ZERO,
    )
    .map(|_| ())
}

#[test]
fn a_file_downloads_from_a_firewalled_peer_via_server_broker() {
    let server = server_or_skip!();

    let client_port = free_port().expect("free client listen port");
    let mut client = Client::with_settings(server.listening_settings(
        "e2e_fw_dl",
        "pw",
        client_port,
    ));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    let bogus_port = free_port().expect("bogus port"); // advertised, unlistened
    let filename = "firewalled_song.mp3";
    let content: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
    let size = content.len() as u64;
    let token = 606_060_u32;
    let download_dir = unique_download_dir();

    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let cfg = MockFirewalledUpload {
        server_addr: format!("{}:{}", server.host, server.port),
        username: "e2e_fw_peer".to_string(),
        password: "pw".to_string(),
        bogus_port,
        downloader_listen_addr: format!("127.0.0.1:{client_port}"),
        filename: filename.to_string(),
        content: content.clone(),
        token,
        ready: ready_tx,
    };
    let uploader = std::thread::spawn(move || {
        if let Err(e) = run_mock_firewalled_peer(&cfg) {
            eprintln!("[mock firewalled peer] {e}");
        }
    });

    ready_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("mock firewalled peer ready");
    std::thread::sleep(Duration::from_secs(1));

    let (_download, status_rx) = client
        .download(
            filename.to_string(),
            "e2e_fw_peer".to_string(),
            size,
            download_dir.display().to_string(),
        )
        .expect("start download");

    let completed =
        wait_for_completion(&client, &status_rx, Duration::from_secs(25));
    let _ = uploader.join();

    assert!(completed, "the firewalled download should reach Completed");

    let written = std::fs::read(download_dir.join(filename))
        .expect("downloaded file should exist");
    assert_eq!(written, content, "downloaded bytes should match the source");

    let _ = std::fs::remove_dir_all(&download_dir);
}

// ---------------------------------------------------------------------------
// Two real clients: one shares a file, the other searches for it and downloads
// it — the entire search + connect + upload/download stack, no mock peer.
// ---------------------------------------------------------------------------

/// A sharer serving `share_dir` and a searcher, both logged in and listening,
/// after a beat for soulfind to register both SetWaitPorts.
fn sharer_and_searcher(
    server: &TestServer,
    share_dir: &std::path::Path,
    sharer: &str,
    searcher: &str,
) -> (Client, Client) {
    let mut sharing = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.listening_settings(
            sharer,
            "pw",
            free_port().expect("sharer port"),
        )
    });
    sharing.connect().expect("sharer connect");
    assert!(sharing.login().expect("sharer login"));

    let mut searching = Client::with_settings(server.listening_settings(
        searcher,
        "pw",
        free_port().expect("searcher port"),
    ));
    searching.connect().expect("searcher connect");
    assert!(searching.login().expect("searcher login"));

    std::thread::sleep(Duration::from_secs(1));
    (sharing, searching)
}

/// `sharer`'s answer to `query`, once it reaches `searcher`.
fn reply_from(
    searcher: &Client,
    query: &str,
    sharer: &str,
) -> Option<soulseek_rs::SearchResult> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let reply = searcher
            .get_search_results(query)
            .into_iter()
            .find(|result| result.username == sharer);
        if reply.is_some() || Instant::now() >= deadline {
            return reply;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn two_real_clients_search_and_download() {
    let server = server_or_skip!();

    // Sharer with one distinctively named file.
    let share_dir = unique_download_dir();
    let content: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
    let filename = "e2e_probe_xyzzy.bin";
    std::fs::write(share_dir.join(filename), &content).unwrap();

    let (sharer, leecher) =
        sharer_and_searcher(&server, &share_dir, "e2e_sharer", "e2e_leecher");

    let query = "xyzzy";
    let _ = leecher.search(query, Duration::from_secs(3));

    let (result_path, size) = reply_from(&leecher, query, "e2e_sharer")
        .and_then(|reply| {
            reply
                .files
                .into_iter()
                .find(|file| file.name.contains("e2e_probe_xyzzy"))
        })
        .map(|file| (file.name, file.size))
        .expect("leecher should find the sharer's file");
    assert_eq!(size, content.len() as u64);

    // Download it from the sharer.
    let download_dir = unique_download_dir();
    let (_download, status_rx) = leecher
        .download(
            result_path.clone(),
            "e2e_sharer".to_string(),
            size,
            download_dir.display().to_string(),
        )
        .expect("start download");

    let completed =
        wait_for_completion(&leecher, &status_rx, Duration::from_secs(25));
    assert!(completed, "the download should complete");

    // The virtual path is backslash-separated; the saved file uses the basename.
    let basename = result_path.rsplit(['\\', '/']).next().unwrap();
    let written = std::fs::read(download_dir.join(basename))
        .expect("downloaded file should exist");
    assert_eq!(written, content, "downloaded bytes should match the source");

    // The uploader side tracked the transfer and saw it complete.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut uploads = sharer.uploads();
    while Instant::now() < deadline
        && !uploads
            .iter()
            .any(|u| u.status == soulseek_rs::types::UploadStatus::Completed)
    {
        std::thread::sleep(Duration::from_millis(200));
        uploads = sharer.uploads();
    }
    let upload = uploads
        .iter()
        .find(|u| u.status == soulseek_rs::types::UploadStatus::Completed)
        .expect("uploader should record a completed upload");
    assert_eq!(upload.username, "e2e_leecher");
    assert_eq!(upload.bytes_sent, size);

    // The next reply advertises what that upload measured.
    let _ = leecher.search(query, Duration::from_secs(1));
    let reply = reply_from(&leecher, query, "e2e_sharer")
        .expect("the sharer answers again");
    assert!(
        reply.speed > 0,
        "a finished upload sets the advertised speed"
    );

    // The server hears about it too (SendUploadSpeed), which is where every
    // other client's `user` lookup and the server's own ranking read it.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut recorded = 0;
    while Instant::now() < deadline && recorded == 0 {
        leecher
            .request_user_info("e2e_sharer")
            .expect("ask about the sharer");
        std::thread::sleep(Duration::from_millis(250));
        recorded = leecher
            .user_info("e2e_sharer")
            .and_then(|info| info.stats)
            .map_or(0, |stats| stats.average_speed);
    }
    assert!(recorded > 0, "the server should record the upload's speed");

    let _ = std::fs::remove_dir_all(share_dir);
    let _ = std::fs::remove_dir_all(download_dir);
}

#[test]
fn a_runtime_share_update_is_visible_to_browsers() {
    let server = server_or_skip!();

    // The sharer starts sharing NOTHING, then adds a directory at runtime
    // (what the TUI settings screen does).
    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(server.listening_settings(
        "e2e_reshare",
        "pw",
        sharer_port,
    ));
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));
    assert!(sharer.shared_directories().is_empty());

    let share_dir = unique_download_dir();
    std::fs::create_dir_all(share_dir.join("new")).unwrap();
    std::fs::write(share_dir.join("new").join("late.mp3"), b"yyyy").unwrap();
    sharer
        .set_shared_directories(vec![share_dir.display().to_string()])
        .expect("runtime share update");
    assert_eq!(sharer.shared_directories().len(), 1);

    let browser_port = free_port().expect("browser port");
    let mut browser = Client::with_settings(server.listening_settings(
        "e2e_reshare_browser",
        "pw",
        browser_port,
    ));
    browser.connect().expect("browser connect");
    assert!(browser.login().expect("browser login"));

    std::thread::sleep(Duration::from_secs(1));
    browser.browse_user("e2e_reshare").expect("browse request");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut listing = None;
    while Instant::now() < deadline {
        if let Some(result) = browser.take_browse_result("e2e_reshare") {
            listing = Some(result);
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let directories = listing.expect("browser should receive the listing");
    assert!(
        directories
            .iter()
            .any(|d| d.files.iter().any(|f| f.name == "late.mp3")),
        "the listing should include the file shared at runtime"
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

#[test]
fn browse_a_peers_shared_files() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    std::fs::create_dir_all(share_dir.join("album")).unwrap();
    std::fs::write(share_dir.join("album").join("track.flac"), b"xxxx")
        .unwrap();

    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.listening_settings("e2e_browsee", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));

    let browser_port = free_port().expect("browser port");
    let mut browser = Client::with_settings(server.listening_settings(
        "e2e_browser",
        "pw",
        browser_port,
    ));
    browser.connect().expect("browser connect");
    assert!(browser.login().expect("browser login"));

    std::thread::sleep(Duration::from_secs(1));

    browser.browse_user("e2e_browsee").expect("browse request");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut listing = None;
    while Instant::now() < deadline {
        if let Some(result) = browser.take_browse_result("e2e_browsee") {
            listing = Some(result);
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let directories = listing.expect("browser should receive the listing");
    assert!(
        directories
            .iter()
            .any(|d| { d.files.iter().any(|f| f.name == "track.flac") }),
        "the listing should include the shared file"
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

// Browsing a peer that is NOT listening exercises the server-brokered
// (firewalled) path: the direct dial fails, the client asks the server to
// broker, and the peer connects back to our listener. This is the path that
// matters on the real network, where most peers are firewalled. The browser
// MUST be listening so the peer can connect back.
#[test]
fn browse_a_firewalled_peer_via_broker() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    std::fs::create_dir_all(share_dir.join("album")).unwrap();
    std::fs::write(share_dir.join("album").join("hidden.flac"), b"xxxx")
        .unwrap();

    // Sharer does NOT listen (firewalled): the browser's direct dial will fail,
    // forcing the server-brokered connect-back.
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.settings("e2e_fw_sharer", "pw")
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));

    // Browser listens so the firewalled peer can connect back to it.
    let browser_port = free_port().expect("browser port");
    let mut browser = Client::with_settings(server.listening_settings(
        "e2e_fw_browser",
        "pw",
        browser_port,
    ));
    browser.connect().expect("browser connect");
    assert!(browser.login().expect("browser login"));

    std::thread::sleep(Duration::from_secs(1));

    browser
        .browse_user("e2e_fw_sharer")
        .expect("browse request");

    // The brokered round-trip has more hops; give it a generous deadline.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut listing = None;
    while Instant::now() < deadline {
        if let Some(result) = browser.take_browse_result("e2e_fw_sharer") {
            listing = Some(result);
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let directories =
        listing.expect("browser should receive the firewalled peer's listing");
    assert!(
        directories
            .iter()
            .any(|d| d.files.iter().any(|f| f.name == "hidden.flac")),
        "the brokered listing should include the shared file"
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

/// Browse `sharer_addr` exactly the way a third-party client (SoulseekQt,
/// Nicotine+) does: dial the peer's listener directly, announce with a
/// `PeerInit(P)`, ask for the share list (peer code 4) and read the
/// `SharedFileListResponse` (peer code 5) back off the same socket.
fn third_party_browse(
    sharer_addr: &str,
    username: &str,
    read_delay: Duration,
    timeout: Duration,
) -> std::io::Result<Vec<soulseek_rs::SharedDirectory>> {
    let mut p = connect_retry(sharer_addr, Duration::from_secs(5))?;
    p.set_read_timeout(Some(timeout))?;
    p.write_all(&peer_init_bytes(username, "P", 0))?;
    p.write_all(&MessageFactory::build_get_share_file_list().get_buffer())?;
    p.flush()?;

    // A real peer is not always ready to drain the socket the instant the
    // response starts arriving; a slow reader must not cost us the listing.
    std::thread::sleep(read_delay);

    let mut response = expect_code(&mut p, 5, timeout)?;
    response.set_pointer(8);
    Ok(soulseek_rs::message::peer::parse_shared_file_list(
        &mut response,
    ))
}

// A third-party client browsing us over a direct connection — the scenario of
// running soulseek-rs and SoulseekQt side by side. The other two browse tests
// only prove soulseek-rs can talk to itself.
#[test]
fn a_third_party_client_browses_our_shares_directly() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    std::fs::create_dir_all(share_dir.join("album")).unwrap();
    std::fs::write(share_dir.join("album").join("track.flac"), b"xxxx")
        .unwrap();

    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.listening_settings("e2e_qt_sharer", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));

    // The browsing client is a real logged-in user, as SoulseekQt would be.
    let server_addr = format!("{}:{}", server.host, server.port);
    let _qt = login_raw(&server_addr, "e2e_qt_browser", "pw")
        .expect("third-party client logs in");

    let directories = third_party_browse(
        &format!("127.0.0.1:{sharer_port}"),
        "e2e_qt_browser",
        Duration::ZERO,
        Duration::from_secs(15),
    )
    .expect("third-party client should receive a SharedFileListResponse");

    assert!(
        directories
            .iter()
            .any(|d| d.files.iter().any(|f| f.name == "track.flac")),
        "the listing should include the shared file, got {directories:?}"
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

// A third-party client that cannot dial us directly browses us through the
// server broker: it asks the server to broker, we connect back with a
// PierceFirewall, and it browses over that connection. This is the path a
// second client on the same machine takes whenever the direct dial to our
// advertised (public) address does not come back to us.
#[test]
fn a_third_party_client_browses_our_shares_via_the_server_broker() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    std::fs::create_dir_all(share_dir.join("album")).unwrap();
    std::fs::write(share_dir.join("album").join("brokered.flac"), b"xxxx")
        .unwrap();

    // The sharer does not listen at all, so the browse can only be brokered.
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.settings("e2e_qt_brok_sharer", "pw")
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));

    // The third-party client listens and advertises its port, then asks the
    // server to broker a P connection from the sharer.
    //
    // Bind every interface, not just loopback: the server brokers back the
    // address it observed for this client, which is the host's routable IP
    // (172.17.0.2 under Docker, 10.x on a CI runner), not 127.0.0.1. A
    // loopback-only listener refuses that dial and the browse never arrives.
    let listener =
        std::net::TcpListener::bind("0.0.0.0:0").expect("browser listener");
    let browser_port = listener.local_addr().unwrap().port();
    let server_addr = format!("{}:{}", server.host, server.port);
    let mut srv = login_raw(&server_addr, "e2e_qt_brok_browser", "pw")
        .expect("third-party client logs in");
    srv.write_all(
        &MessageFactory::build_set_wait_port_message(browser_port).get_buffer(),
    )
    .expect("set wait port");
    srv.flush().expect("flush wait port");
    std::thread::sleep(Duration::from_secs(1));

    let token = 424_242_u32;
    srv.write_all(
        &MessageFactory::build_connect_to_peer(
            token,
            "e2e_qt_brok_sharer",
            ConnectionType::P,
        )
        .get_buffer(),
    )
    .expect("ask the server to broker");
    srv.flush().expect("flush ConnectToPeer");

    // The sharer must dial us back, quoting our correlation token.
    listener.set_nonblocking(true).expect("non-blocking accept");
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut accepted = None;
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => {
                accepted = Some(stream);
                break;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("accept failed: {e}"),
        }
    }
    let mut p = accepted.expect("the sharer should connect back to us");
    p.set_nonblocking(false).expect("blocking peer socket");
    p.set_read_timeout(Some(Duration::from_secs(20)))
        .expect("read timeout");

    let mut pierce = read_framed(&mut p).expect("a PierceFirewall frame");
    assert_eq!(
        pierce.get_init_code(),
        0,
        "the brokered connect-back must start with a PierceFirewall"
    );
    pierce.set_pointer(5); // length prefix (4) + int8 code (1)
    assert_eq!(
        pierce.read_int32(),
        token,
        "the PierceFirewall must quote the token we brokered with"
    );

    p.write_all(&MessageFactory::build_get_share_file_list().get_buffer())
        .expect("request the share list");
    p.flush().expect("flush share list request");

    let mut response = expect_code(&mut p, 5, Duration::from_secs(20))
        .expect("a SharedFileListResponse over the brokered connection");
    response.set_pointer(8);
    let directories =
        soulseek_rs::message::peer::parse_shared_file_list(&mut response);

    assert!(
        directories
            .iter()
            .any(|d| d.files.iter().any(|f| f.name == "brokered.flac")),
        "the brokered listing should include the shared file, got {directories:?}"
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

// Real peers connect to our listener and go away again (a dropped dial, a port
// scan, a client that gave up). None of that may cost us the listener: the next
// client to come along must still be able to browse.
#[test]
fn a_stalled_peer_connection_does_not_wedge_the_listener() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    std::fs::create_dir_all(share_dir.join("album")).unwrap();
    std::fs::write(share_dir.join("album").join("still.flac"), b"xxxx")
        .unwrap();

    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.listening_settings("e2e_wedge_sharer", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));

    let sharer_addr = format!("127.0.0.1:{sharer_port}");

    // A peer that connects and hangs up without ever sending a PeerInit.
    drop(
        connect_retry(&sharer_addr, Duration::from_secs(5))
            .expect("dial the listener"),
    );
    // A peer that connects and then just sits there, saying nothing.
    let _silent = connect_retry(&sharer_addr, Duration::from_secs(5))
        .expect("dial the listener");

    let server_addr = format!("{}:{}", server.host, server.port);
    let _qt = login_raw(&server_addr, "e2e_wedge_browser", "pw")
        .expect("third-party client logs in");

    let directories = third_party_browse(
        &sharer_addr,
        "e2e_wedge_browser",
        Duration::ZERO,
        Duration::from_secs(15),
    )
    .expect("the listener must still serve browse requests");

    assert!(
        directories
            .iter()
            .any(|d| d.files.iter().any(|f| f.name == "still.flac")),
        "the listing should include the shared file, got {directories:?}"
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

// The same direct browse against a share big enough that the response cannot
// fit in one socket buffer. This is the realistic case: a shared music library
// serialises to hundreds of KB, and the payload is zlib-STORED (not actually
// compressed), so the listing goes out at full size.
#[test]
fn a_third_party_client_browses_a_large_share() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir().join("big");
    let _ = std::fs::remove_dir_all(&share_dir);
    std::fs::create_dir_all(&share_dir).unwrap();
    // ~4000 entries with long names: well over any socket buffer once framed.
    let padding = "x".repeat(150);
    for i in 0..4000 {
        std::fs::write(share_dir.join(format!("{padding}-{i:05}.flac")), b"z")
            .unwrap();
    }

    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.listening_settings("e2e_qt_big_sharer", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));

    let server_addr = format!("{}:{}", server.host, server.port);
    let _qt = login_raw(&server_addr, "e2e_qt_big_browser", "pw")
        .expect("third-party client logs in");

    let directories = third_party_browse(
        &format!("127.0.0.1:{sharer_port}"),
        "e2e_qt_big_browser",
        Duration::from_secs(1),
        Duration::from_secs(20),
    )
    .expect("third-party client should receive the full large listing");

    let file_count: usize = directories.iter().map(|d| d.files.len()).sum();
    assert_eq!(
        file_count, 4000,
        "the whole listing must arrive, not just what fit in one socket buffer"
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

#[test]
fn the_server_reports_another_users_status_and_share_counts() {
    let server = server_or_skip!();

    // A sharer with a known number of files, so the statistics are checkable.
    let share_dir = unique_download_dir();
    std::fs::create_dir_all(share_dir.join("album")).unwrap();
    for name in ["one.flac", "two.flac"] {
        std::fs::write(share_dir.join("album").join(name), b"xxxx").unwrap();
    }
    let mut subject = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.settings("e2e_info_subject", "pw")
    });
    subject.connect().expect("subject connect");
    assert!(subject.login().expect("subject login"));

    let mut asker =
        Client::with_settings(server.settings("e2e_info_asker", "pw"));
    asker.connect().expect("asker connect");
    assert!(asker.login().expect("asker login"));

    asker
        .request_user_info("e2e_info_subject")
        .expect("ask about the subject");

    // Status and statistics arrive as two separate replies.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut info = None;
    while Instant::now() < deadline {
        match asker.user_info("e2e_info_subject") {
            Some(found) if found.is_complete() => {
                info = Some(found);
                break;
            }
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    let info = info.expect("the server should answer with status and stats");

    assert_eq!(info.username, "e2e_info_subject");
    let presence = info.presence.expect("presence should have arrived");
    assert!(
        presence.status.is_reachable(),
        "a logged-in user should not read as offline, got {}",
        presence.status
    );
    let stats = info.stats.expect("stats should have arrived");
    assert_eq!(stats.shared_files, 2, "the subject shares two files");
    assert!(stats.shared_folders >= 1, "and at least one folder");

    let _ = std::fs::remove_dir_all(share_dir);
}

// --- upload slots and privilege recognition --------------------------------
//
// Soulseek's rules ask an alternative client to *recognise* privileges. That is
// only observable if there is a queue to jump, so these tests hold the single
// upload slot open with a peer that queues a file and then never answers the
// TransferRequest — the same thing a stalled client does, and deterministic in
// a way that racing real 4 KB transfers is not.

/// A raw peer that logs in, dials our listener, queues `filename`, and then
/// goes quiet. Returned so the caller can keep it alive: dropping the streams
/// would release the slot it is holding.
struct QueueingPeer {
    _server: TcpStream,
    peer: TcpStream,
}

impl QueueingPeer {
    /// Wait for the upload offer (peer code 40) and deliberately not answer it.
    ///
    /// Receiving the offer is the only positive proof that this peer was given a
    /// slot — an unaccepted offer never reaches `Client::uploads()`, and "has no
    /// place in the queue" is also true of a peer that never asked, so polling
    /// for that would pass against a completely broken pump.
    fn takes_a_slot(&mut self) -> bool {
        self.takes_a_slot_within(Duration::from_secs(10))
    }

    fn takes_a_slot_within(&mut self, timeout: Duration) -> bool {
        let _ = self.peer.set_read_timeout(Some(timeout));
        read_until_code(&mut self.peer, 40, timeout).is_some()
    }
}

fn queue_as(
    server_addr: &str,
    listen_addr: &str,
    username: &str,
    filename: &str,
) -> std::io::Result<QueueingPeer> {
    let server = login_raw(server_addr, username, "pw")?;
    let mut peer = connect_retry(listen_addr, Duration::from_secs(5))?;
    peer.set_read_timeout(Some(Duration::from_secs(10)))?;
    peer.write_all(&peer_init_bytes(username, "P", 0))?;
    let mut queue = Message::new();
    queue.write_int32(43).write_string(filename);
    peer.write_all(&queue.get_buffer())?;
    peer.flush()?;
    Ok(QueueingPeer {
        _server: server,
        peer,
    })
}

/// Wait for `check` to hold, polling the client's view of its own queue.
fn wait_for(mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Where `username` sits in `sharer`'s upload queue, read the way the CLI reads
/// it — through `uploads()` — rather than through a point query kept alive only
/// for tests.
fn place_of(sharer: &Client, username: &str, filename: &str) -> Option<u32> {
    sharer.uploads().into_iter().find_map(|upload| {
        match (upload.status, upload.username == username) {
            (UploadStatus::Queued(place), true)
                if upload.filename == filename =>
            {
                Some(place)
            }
            _ => None,
        }
    })
}

/// Set `column` ("privileges", or "admin" for the user whose private
/// messages to "server" are commands) on `username` in soulfind's own
/// database, good for a day: soulfind stores both as the unix timestamp
/// they expire at.
///
/// `false` means it could not be done — an external server, or no `sqlite3`
/// on this machine — which the caller treats as a skip rather than a failure.
fn grant(server: &TestServer, username: &str, column: &str) -> bool {
    let Some(db) = server.db.as_ref() else {
        return false;
    };
    let expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() + 86_400);
    Command::new("sqlite3")
        .arg(db)
        .arg(format!(
            "UPDATE users SET {column} = {expiry} WHERE username = '{username}';"
        ))
        .status()
        .is_ok_and(|status| status.success())
}

/// A share directory of its own, so the virtual paths a peer has to name are
/// predictable: soulfind-facing paths are `<folder name>\<file>`.
fn queue_share(label: &str, files: &[&str]) -> (PathBuf, String) {
    // Keyed on the thread as well as the process, matching
    // `unique_download_dir`: two tests are two threads, and a shared folder
    // name would make their virtual paths collide.
    let dir = std::env::temp_dir().join(format!(
        "soulseek-e2e-queue-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("share dir");
    for name in files {
        std::fs::write(dir.join(name), vec![9u8; 4096]).expect("share file");
    }
    let folder = dir
        .file_name()
        .and_then(|n| n.to_str())
        .expect("utf-8 folder")
        .to_string();
    (dir, folder)
}

#[test]
fn a_second_request_waits_when_the_only_upload_slot_is_taken() {
    let server = server_or_skip!();
    let (share, folder) =
        queue_share("slots", &["blocker.mp3", "waiter.mp3", "third.mp3"]);
    let server_addr = format!("{}:{}", server.host, server.port);

    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share.display().to_string()],
        ..server.listening_settings("e2e_q_sharer", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));
    sharer.set_upload_slots(1);
    let listen_addr = format!("127.0.0.1:{sharer_port}");

    let blocker_file = format!("{folder}\\blocker.mp3");
    let mut blocker =
        queue_as(&server_addr, &listen_addr, "e2e_q_blocker", &blocker_file)
            .expect("blocker queues");
    assert!(
        blocker.takes_a_slot(),
        "the first request should be offered the free slot"
    );

    let waiter_file = format!("{folder}\\waiter.mp3");
    let _waiter =
        queue_as(&server_addr, &listen_addr, "e2e_q_waiter", &waiter_file)
            .expect("waiter queues");
    assert!(
        wait_for(|| place_of(&sharer, "e2e_q_waiter", &waiter_file) == Some(1)),
        "with the slot held, the next request must wait at place 1, got {:?}",
        place_of(&sharer, "e2e_q_waiter", &waiter_file)
    );

    let third_file = format!("{folder}\\third.mp3");
    let _third =
        queue_as(&server_addr, &listen_addr, "e2e_q_third", &third_file)
            .expect("third queues");
    assert!(
        wait_for(|| place_of(&sharer, "e2e_q_third", &third_file) == Some(2)),
        "and the one after that at place 2, got {:?}",
        place_of(&sharer, "e2e_q_third", &third_file)
    );

    // Both waiters show up as queued uploads with their places, which is what
    // `serve` reports to an operator.
    let queued: Vec<(String, u32)> = sharer
        .uploads()
        .into_iter()
        .filter_map(|upload| match upload.status {
            UploadStatus::Queued(place) => Some((upload.username, place)),
            _ => None,
        })
        .collect();
    assert_eq!(
        queued,
        [
            ("e2e_q_waiter".to_string(), 1),
            ("e2e_q_third".to_string(), 2)
        ]
    );

    let _ = std::fs::remove_dir_all(share);
}

#[test]
fn a_privileged_peer_overtakes_one_already_waiting_for_a_slot() {
    let server = server_or_skip!();
    let server_addr = format!("{}:{}", server.host, server.port);

    // The account has to exist before it can be given privileges, and the
    // privileges have to exist before we log in: the server sends the
    // privileged-user list once, at login.
    drop(
        login_raw(&server_addr, "e2e_q_donor", "pw")
            .expect("the donor registers"),
    );
    if !grant(&server, "e2e_q_donor", "privileges") {
        eprintln!(
            "privilege e2e skipped: cannot write soulfind's database (needs a \
             locally spawned server and sqlite3)"
        );
        return;
    }

    let (share, folder) =
        queue_share("privileged", &["blocker.mp3", "plain.mp3", "donor.mp3"]);
    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share.display().to_string()],
        ..server.listening_settings("e2e_q_psharer", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));
    sharer.set_upload_slots(1);
    let listen_addr = format!("127.0.0.1:{sharer_port}");

    // Not every soulfind build answers login with a populated code-69 list, so
    // a server that names nobody is a skip rather than a failure — the
    // re-ranking itself is covered by `client::upload_queue::context_tests`.
    if !wait_for(|| sharer.is_privileged("e2e_q_donor")) {
        eprintln!(
            "privilege ordering e2e skipped: this server declared no \
             privileged users even after granting them in its database"
        );
        let _ = std::fs::remove_dir_all(share);
        return;
    }
    assert!(!sharer.is_privileged("e2e_q_plain"), "and nobody else");

    let blocker_file = format!("{folder}\\blocker.mp3");
    let mut blocker =
        queue_as(&server_addr, &listen_addr, "e2e_q_pblocker", &blocker_file)
            .expect("blocker queues");
    assert!(
        blocker.takes_a_slot(),
        "the blocker should be offered the only slot"
    );

    // A plain user asks first and is alone in the queue…
    let plain_file = format!("{folder}\\plain.mp3");
    let _plain =
        queue_as(&server_addr, &listen_addr, "e2e_q_plain", &plain_file)
            .expect("plain queues");
    assert!(
        wait_for(|| place_of(&sharer, "e2e_q_plain", &plain_file) == Some(1)),
        "got {:?}",
        place_of(&sharer, "e2e_q_plain", &plain_file)
    );

    // …and then the donor asks, and goes ahead of them.
    let donor_file = format!("{folder}\\donor.mp3");
    let _donor =
        queue_as(&server_addr, &listen_addr, "e2e_q_donor", &donor_file)
            .expect("donor queues");
    assert!(
        wait_for(|| place_of(&sharer, "e2e_q_donor", &donor_file) == Some(1)),
        "the privileged peer should take place 1, got {:?}",
        place_of(&sharer, "e2e_q_donor", &donor_file)
    );
    assert_eq!(
        place_of(&sharer, "e2e_q_plain", &plain_file),
        Some(2),
        "and the plain user should be told they were overtaken"
    );

    let _ = std::fs::remove_dir_all(share);
}

#[test]
fn a_peer_asking_where_it_sits_is_answered_with_its_place() {
    let server = server_or_skip!();
    let (share, folder) = queue_share("place", &["blocker.mp3", "asker.mp3"]);
    let server_addr = format!("{}:{}", server.host, server.port);

    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share.display().to_string()],
        ..server.listening_settings("e2e_q_asharer", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));
    sharer.set_upload_slots(1);
    let listen_addr = format!("127.0.0.1:{sharer_port}");

    let blocker_file = format!("{folder}\\blocker.mp3");
    let mut blocker =
        queue_as(&server_addr, &listen_addr, "e2e_q_ablocker", &blocker_file)
            .expect("blocker queues");
    assert!(
        blocker.takes_a_slot(),
        "the blocker should be offered the only slot"
    );

    // The asker keeps its own connection so it can read the answer back.
    let asker_file = format!("{folder}\\asker.mp3");
    let mut asker =
        queue_as(&server_addr, &listen_addr, "e2e_q_asker", &asker_file)
            .expect("asker queues");

    assert!(
        wait_for(|| place_of(&sharer, "e2e_q_asker", &asker_file) == Some(1)),
        "the asker should be waiting at place 1 first"
    );

    let mut request = Message::new();
    request.write_int32(51).write_string(&asker_file);
    asker.peer.write_all(&request.get_buffer()).expect("ask");
    asker.peer.flush().expect("flush");

    let mut answer = expect_code(&mut asker.peer, 44, Duration::from_secs(10))
        .expect("a PlaceInQueueResponse should come back");
    answer.set_pointer(8);
    assert_eq!(answer.read_string(), asker_file);
    assert_eq!(answer.read_int32(), 1, "and it should carry the real place");

    let _ = std::fs::remove_dir_all(share);
}

#[test]
fn the_server_answers_how_much_privilege_time_we_have() {
    let server = server_or_skip!();
    let mut client =
        Client::with_settings(server.settings("e2e_priv_asker", "pw"));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    client.check_privileges().expect("ask");
    assert!(
        wait_for(|| client.own_privilege_seconds().is_some()),
        "the server should answer code 92"
    );
    assert_eq!(
        client.own_privilege_seconds(),
        Some(0),
        "a fresh account has no privileges, which is zero rather than unknown"
    );
}

#[test]
fn a_user_who_never_logged_in_reads_as_offline() {
    let server = server_or_skip!();
    let mut asker =
        Client::with_settings(server.settings("e2e_info_asker_b", "pw"));
    asker.connect().expect("asker connect");
    assert!(asker.login().expect("asker login"));

    asker.request_user_info("nobody_by_this_name").expect("ask");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut info = None;
    while Instant::now() < deadline && info.is_none() {
        info = asker
            .user_info("nobody_by_this_name")
            .filter(|found| found.presence.is_some());
        if info.is_none() {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    let presence = info
        .expect("the server should answer for an unknown user")
        .presence
        .expect("with a presence");
    assert!(!presence.status.is_reachable(), "got {}", presence.status);
}

// ---------------------------------------------------------------------------
// Concurrent sessions: several one-shot invocations running at once.
// ---------------------------------------------------------------------------

/// Ask the server what address it hands out for `username`.
fn advertised_port(server_addr: &str, asker: &str, username: &str) -> u32 {
    let mut peer = login_raw(server_addr, asker, "pw").expect("asker login");
    peer.write_all(
        &MessageFactory::build_get_peer_address(username).get_buffer(),
    )
    .expect("ask for the address");
    peer.flush().expect("flush");
    let mut reply = expect_code(&mut peer, 3, Duration::from_secs(5))
        .expect("the server should answer with an address");
    reply.set_pointer(8);
    let _name = reply.read_string();
    for _ in 0..4 {
        let _ = reply.read_int8();
    }
    reply.read_int32()
}

#[test]
fn a_taken_listen_port_does_not_cost_us_the_listener() {
    let server = server_or_skip!();
    let addr = format!("{}:{}", server.host, server.port);

    // A sibling process already holds the configured port: the ordinary case
    // when several invocations run at once with one config file between them.
    let taken = free_port().expect("port to occupy");
    let _squatter =
        std::net::TcpListener::bind(("0.0.0.0", taken)).expect("occupy");

    let mut client = Client::with_settings(server.listening_settings(
        "e2e_port_clash",
        "pw",
        taken,
    ));
    client.connect().expect("connect despite the taken port");
    assert!(client.login().expect("login"));

    let bound = client.listen_port().expect("a listener was still bound");
    assert_ne!(bound, taken, "a port someone else holds cannot be ours");
    std::thread::sleep(Duration::from_secs(1));

    // Whatever port we ended up on, the server must hand peers that one:
    // advertising a port held by someone else sends our search responses to a
    // stranger, and we wait out the search seeing nothing.
    let advertised = advertised_port(&addr, "e2e_port_asker", "e2e_port_clash");
    assert_eq!(
        advertised,
        u32::from(bound),
        "the server must advertise the port we really bound"
    );
}

#[test]
fn a_listening_client_reports_the_port_it_bound() {
    let server = server_or_skip!();
    let port = free_port().expect("free port");
    let mut client = Client::with_settings(server.listening_settings(
        "e2e_port_ok",
        "pw",
        port,
    ));
    client.connect().expect("connect");
    assert_eq!(client.listen_port(), Some(port));

    let mut quiet =
        Client::with_settings(server.settings("e2e_port_none", "pw"));
    quiet.connect().expect("connect");
    assert_eq!(
        quiet.listen_port(),
        None,
        "a client that does not listen has no port to report"
    );
}

#[test]
fn a_second_login_under_one_name_reports_the_first_session_as_lost() {
    let server = server_or_skip!();

    let mut first =
        Client::with_settings(server.settings("e2e_displaced", "pw"));
    first.connect().expect("first connect");
    assert!(first.login().expect("first login"));
    assert_eq!(first.session_loss(), None, "a fresh session is alive");
    std::thread::sleep(Duration::from_secs(1));

    // The server allows one session per account, so this login evicts the one
    // above. Silently seeing nothing from then on is what made concurrent runs
    // report files as unavailable when they were not.
    let mut second =
        Client::with_settings(server.settings("e2e_displaced", "pw"));
    second.connect().expect("second connect");
    assert!(second.login().expect("second login"));

    let deadline = Instant::now() + Duration::from_secs(15);
    while first.session_loss().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        first.session_loss(),
        Some(SessionLoss::Displaced),
        "the evicted session must say why it went quiet"
    );
    assert_eq!(second.session_loss(), None, "the winner keeps its session");
}

#[test]
fn clients_with_distinct_names_search_the_same_sharer_at_once() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    let content: Vec<u8> = (0..2048u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(share_dir.join("e2e_probe_parallel.bin"), &content).unwrap();
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.listening_settings(
            "e2e_par_sharer",
            "pw",
            free_port().expect("sharer port"),
        )
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));

    // Three sessions on one machine, all configured with the same listen port:
    // exactly what several agents running the CLI at once look like.
    let shared_port = free_port().expect("shared port");
    let seekers: Vec<Client> = (0..3)
        .map(|i| {
            let mut client = Client::with_settings(server.listening_settings(
                &format!("e2e_par_seeker_{i}"),
                "pw",
                shared_port,
            ));
            client.connect().expect("seeker connect");
            assert!(client.login().expect("seeker login"));
            client
        })
        .collect();

    let ports: Vec<u16> =
        seekers.iter().filter_map(Client::listen_port).collect();
    assert_eq!(ports.len(), 3, "every seeker keeps a listener");
    let mut unique = ports.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 3, "each seeker holds its own port: {ports:?}");
    std::thread::sleep(Duration::from_secs(1));

    std::thread::scope(|scope| {
        for client in &seekers {
            scope.spawn(move || {
                let _ = client.search("parallel", Duration::from_secs(5));
            });
        }
    });

    for (i, client) in seekers.iter().enumerate() {
        assert_eq!(
            client.session_loss(),
            None,
            "seeker {i} should still hold its session"
        );
        let hit = client
            .get_search_results("parallel")
            .into_iter()
            .flat_map(|result| result.files)
            .find(|file| file.name.contains("e2e_probe_parallel"));
        assert!(hit.is_some(), "seeker {i} found nothing");
    }

    let _ = std::fs::remove_dir_all(share_dir);
}

#[test]
fn a_peer_that_hangs_up_its_control_connection_still_delivers_the_file() {
    let server = server_or_skip!();

    // The bytes travel over their own F connection, so a peer is free to close
    // the control connection first — other clients drop idle peer sockets as a
    // matter of course. Treating that hangup as a failure used to fail every
    // download queued with that peer, the one streaming included.
    let client_port = free_port().expect("free client listen port");
    let mut client = Client::with_settings(server.listening_settings(
        "e2e_fin_dl",
        "pw",
        client_port,
    ));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    let mock_port = free_port().expect("free mock listen port");
    let filename = "hangup_song.mp3";
    let content: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
    let size = content.len() as u64;
    let download_dir = unique_download_dir();

    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let cfg = MockDirectUpload {
        server_addr: format!("{}:{}", server.host, server.port),
        username: "e2e_finpeer".to_string(),
        password: "pw".to_string(),
        listen_port: mock_port,
        downloader_listen_addr: format!("127.0.0.1:{client_port}"),
        downloader_username: "e2e_fin_dl".to_string(),
        filename: filename.to_string(),
        content: content.clone(),
        token: 616_161_u32,
        ready: ready_tx,
        close_control_first: true,
    };
    let uploader = std::thread::spawn(move || {
        if let Err(e) = run_mock_direct_peer(&cfg) {
            eprintln!("[mock hangup peer] {e}");
        }
    });

    ready_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("mock peer ready");
    std::thread::sleep(Duration::from_secs(1));

    let (_download, status_rx) = client
        .download(
            filename.to_string(),
            "e2e_finpeer".to_string(),
            size,
            download_dir.display().to_string(),
        )
        .expect("start download");

    let completed =
        wait_for_completion(&client, &status_rx, Duration::from_secs(20));
    let _ = uploader.join();

    assert!(
        completed,
        "a closed control connection must not fail the transfer"
    );
    let written = std::fs::read(download_dir.join(filename))
        .expect("downloaded file should exist");
    assert_eq!(written, content, "downloaded bytes should match the source");

    let _ = std::fs::remove_dir_all(&download_dir);
}

#[test]
fn a_joined_room_reports_its_members_statistics() {
    let server = server_or_skip!();

    let room = "e2e_room_stats";

    // A member with a known share, so the statistics are checkable rather
    // than merely present.
    let share_dir = unique_download_dir().join("stats_share");
    std::fs::create_dir_all(share_dir.join("album")).unwrap();
    for name in ["one.flac", "two.flac", "three.flac"] {
        std::fs::write(share_dir.join("album").join(name), b"xxxx").unwrap();
    }
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.settings("e2e_stats_sharer", "pw")
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));
    sharer.join_room(room).expect("sharer joins room");

    // Let the server register the sharer's membership and share counts, so
    // the observer's own join reply describes a room that already has them.
    std::thread::sleep(Duration::from_millis(750));

    let mut observer =
        Client::with_settings(server.settings("e2e_stats_observer", "pw"));
    observer.connect().expect("observer connect");
    assert!(observer.login().expect("observer login"));
    observer.join_room(room).expect("observer joins room");

    // The stats ride along with the membership list, so poll for the member
    // rather than for a fixed delay.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut sharer_stats = None;
    while Instant::now() < deadline {
        if let Some(found) = observer
            .room_member_stats(room)
            .into_iter()
            .find(|s| s.username == "e2e_stats_sharer")
        {
            sharer_stats = Some(found);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let stats =
        sharer_stats.expect("the join reply should describe the members");

    assert!(
        stats.status.is_reachable(),
        "a member who is in the room cannot read as offline, got {}",
        stats.status
    );
    assert_eq!(
        stats.shared_files, 3,
        "the member's file count must survive the parallel stat vectors"
    );
    assert_eq!(stats.shared_folders, 1);
    assert!(
        !stats.slots_full,
        "a client that never filled its slots must not read as full"
    );

    // Every member is described, not just the one we looked up: a short or
    // misread vector would drop or shift the rest.
    let members = observer.room_member_stats(room);
    let names: Vec<&str> =
        members.iter().map(|s| s.username.as_str()).collect();
    assert!(
        names.contains(&"e2e_stats_sharer")
            && names.contains(&"e2e_stats_observer"),
        "both members should be described, got {names:?}"
    );
    assert_eq!(
        members.len(),
        observer.room_members(room).len(),
        "the stats must line up one-to-one with the roster"
    );
}

#[test]
fn watching_a_user_returns_their_status_and_share_counts() {
    let server = server_or_skip!();

    // A subject with a known share, so the watch reply's statistics are
    // checkable rather than merely present.
    let share_dir = unique_download_dir().join("watch_share");
    std::fs::create_dir_all(share_dir.join("album")).unwrap();
    for name in ["one.flac", "two.flac"] {
        std::fs::write(share_dir.join("album").join(name), b"xxxx").unwrap();
    }
    let mut subject = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.settings("e2e_watch_subject", "pw")
    });
    subject.connect().expect("subject connect");
    assert!(subject.login().expect("subject login"));

    let mut watcher =
        Client::with_settings(server.settings("e2e_watch_watcher", "pw"));
    watcher.connect().expect("watcher connect");
    assert!(watcher.login().expect("watcher login"));

    // Let the subject's share counts reach the server before watching, so
    // the reply describes a user it already knows the statistics for.
    std::thread::sleep(Duration::from_millis(750));

    watcher
        .watch_user("e2e_watch_subject")
        .expect("watch the subject");
    assert_eq!(
        watcher.watched_users(),
        vec!["e2e_watch_subject".to_string()]
    );

    // One reply carries both halves, unlike the two GetUserStatus and
    // GetUserStats answers request_user_info waits on.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut info = None;
    while Instant::now() < deadline {
        match watcher.user_info("e2e_watch_subject") {
            Some(found) if found.is_complete() => {
                info = Some(found);
                break;
            }
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    let info = info.expect("the watch reply should carry status and stats");

    let presence = info.presence.expect("presence should have arrived");
    assert!(
        presence.status.is_reachable(),
        "a logged-in user should not read as offline, got {}",
        presence.status
    );
    let stats = info.stats.expect("stats should have arrived");
    assert_eq!(
        stats.shared_files, 2,
        "the file count must survive the obsolete fields ahead of it"
    );
    assert_eq!(stats.shared_folders, 1);

    // Unwatching drops the user and what we knew about them, so a later
    // re-watch reports a fresh answer rather than this stale one.
    watcher
        .unwatch_user("e2e_watch_subject")
        .expect("unwatch the subject");
    assert!(watcher.watched_users().is_empty());
    assert!(watcher.user_info("e2e_watch_subject").is_none());
}

#[test]
fn watching_a_user_the_server_does_not_know_reports_their_absence() {
    let server = server_or_skip!();

    let mut watcher =
        Client::with_settings(server.settings("e2e_watch_ghost_asker", "pw"));
    watcher.connect().expect("watcher connect");
    assert!(watcher.login().expect("watcher login"));

    watcher
        .watch_user("e2e_watch_nobody_here")
        .expect("watch an unknown user");

    // The server answers that it has no such user; the watch is dropped
    // rather than left showing a permanently blank row.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && !watcher.watched_users().is_empty() {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        watcher.watched_users().is_empty(),
        "a username the server does not know must not stay watched"
    );
    assert!(watcher.user_info("e2e_watch_nobody_here").is_none());
}

// A query that matches most of a big share must not ship the whole share to
// whoever typed it: a reply is a shortlist, and the searcher's other sources
// cover the rest.
#[test]
fn a_search_reply_carries_at_most_the_file_cap() {
    let server = server_or_skip!();

    let cap = soulseek_rs::types::MAX_SEARCH_REPLY_FILES;
    let share_dir = unique_download_dir();
    for i in 0..cap + 20 {
        std::fs::write(share_dir.join(format!("probe_cap_{i:04}.bin")), b"x")
            .unwrap();
    }
    let (_sharer, searcher) = sharer_and_searcher(
        &server,
        &share_dir,
        "e2e_cap_sharer",
        "e2e_cap_searcher",
    );

    let query = "probe_cap";
    let _ = searcher.search(query, Duration::from_secs(3));

    let reply = reply_from(&searcher, query, "e2e_cap_sharer")
        .expect("the sharer should answer the search");
    assert_eq!(reply.files.len(), cap, "a reply is capped at {cap} files");

    let _ = std::fs::remove_dir_all(share_dir);
}

// Tokens were the first five hex digits of the query's MD5, so two queries
// could share one and a peer's answer to either went to whichever search the
// map handed back first. These two queries collide under that scheme.
#[test]
fn two_live_searches_never_share_a_token() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    std::fs::write(share_dir.join("tokenprobe aeabna.bin"), b"x").unwrap();
    let (_sharer, searcher) = sharer_and_searcher(
        &server,
        &share_dir,
        "e2e_token_sharer",
        "e2e_token_searcher",
    );

    let matching = "tokenprobe aeabna";
    let colliding = "tokenprobe soyvgt";
    let _ = searcher.search(colliding, Duration::from_millis(100));
    let _ = searcher.search(matching, Duration::from_secs(3));

    let live = searcher.get_all_searches();
    assert_ne!(
        live[matching].token, live[colliding].token,
        "two live searches must not share a token"
    );

    assert!(
        reply_from(&searcher, matching, "e2e_token_sharer").is_some(),
        "the sharer's answer should land in the search it answers"
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

// Searchers pick sources by the free-slot flag in a reply, and every reply
// said "free" whatever the queue looked like.
#[test]
fn a_search_reply_reports_no_free_slot_while_the_only_slot_is_taken() {
    let server = server_or_skip!();
    let (share, folder) =
        queue_share("honest", &["blocker.mp3", "probe_honest.bin"]);
    let (sharer, searcher) = sharer_and_searcher(
        &server,
        &share,
        "e2e_honest_sharer",
        "e2e_honest_searcher",
    );
    sharer.set_upload_slots(1);

    let query = "probe_honest";
    let _ = searcher.search(query, Duration::from_secs(1));
    let reply = reply_from(&searcher, query, "e2e_honest_sharer")
        .expect("the sharer answers");
    assert_eq!(reply.slots, 1, "nothing is uploading yet");

    let server_addr = format!("{}:{}", server.host, server.port);
    let listen_addr =
        format!("127.0.0.1:{}", sharer.listen_port().expect("listening"));
    let mut blocker = queue_as(
        &server_addr,
        &listen_addr,
        "e2e_honest_blocker",
        &format!("{folder}\\blocker.mp3"),
    )
    .expect("blocker queues");
    assert!(
        blocker.takes_a_slot(),
        "the blocker is offered the only slot"
    );

    let _ = searcher.search(query, Duration::from_secs(1));
    let reply = reply_from(&searcher, query, "e2e_honest_sharer")
        .expect("the sharer answers again");
    assert_eq!(reply.slots, 0, "the only slot is taken");

    let _ = std::fs::remove_dir_all(share);
}

// "Download folder" in SoulseekQt and Nicotine+ asks the sharer for that one
// folder (peer code 36) instead of the whole share; a sharer that never
// answers leaves the folder download hanging.
#[test]
fn a_third_party_client_fetches_one_folder_of_our_shares() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    std::fs::create_dir_all(share_dir.join("album")).unwrap();
    std::fs::create_dir_all(share_dir.join("other")).unwrap();
    std::fs::write(share_dir.join("album").join("one.flac"), b"xxxx").unwrap();
    std::fs::write(share_dir.join("album").join("two.flac"), b"yy").unwrap();
    std::fs::write(share_dir.join("other").join("three.flac"), b"z").unwrap();
    let root = share_dir.file_name().unwrap().to_str().unwrap().to_string();

    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.listening_settings("e2e_folder_sharer", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));
    let server_addr = format!("{}:{}", server.host, server.port);
    let _qt = login_raw(&server_addr, "e2e_folder_browser", "pw")
        .expect("third-party client logs in");

    let folder = format!("{root}\\album");
    let mut p = connect_retry(
        &format!("127.0.0.1:{sharer_port}"),
        Duration::from_secs(5),
    )
    .expect("dial the sharer");
    p.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    p.write_all(&peer_init_bytes("e2e_folder_browser", "P", 0))
        .unwrap();
    let mut request = Message::new();
    request
        .write_int32(36)
        .write_int32(77)
        .write_string(&folder);
    p.write_all(&request.get_buffer()).unwrap();
    p.flush().unwrap();

    let mut response = expect_code(&mut p, 37, Duration::from_secs(15))
        .expect("the sharer answers with a FolderContentsResponse");
    response.set_pointer(8);
    let (token, echoed, directories) =
        soulseek_rs::message::peer::parse_folder_contents(&mut response)
            .expect("a well-formed folder listing");
    assert_eq!(token, 77);
    assert_eq!(echoed, folder);
    let mut names: Vec<(String, Vec<String>)> = directories
        .into_iter()
        .map(|d| (d.name, d.files.into_iter().map(|f| f.name).collect()))
        .collect();
    for (_, files) in &mut names {
        files.sort();
    }
    assert_eq!(
        names,
        [(folder, vec!["one.flac".to_string(), "two.flac".to_string()])],
        "only the requested folder, with its files"
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

// "User info" in SoulseekQt and Nicotine+ asks the peer directly (peer code
// 15); a peer that never answers shows an endless spinner.
#[test]
fn a_third_party_client_reads_our_user_info() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    std::fs::write(share_dir.join("probe.flac"), b"xxxx").unwrap();
    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.listening_settings("e2e_info_sharer", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));
    sharer.set_upload_slots(3);
    let server_addr = format!("{}:{}", server.host, server.port);
    let _qt = login_raw(&server_addr, "e2e_info_asker", "pw")
        .expect("third-party client logs in");

    let mut p = connect_retry(
        &format!("127.0.0.1:{sharer_port}"),
        Duration::from_secs(5),
    )
    .expect("dial the sharer");
    p.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    p.write_all(&peer_init_bytes("e2e_info_asker", "P", 0))
        .unwrap();
    p.write_all(&Message::new().write_int32(15).get_buffer())
        .unwrap();
    p.flush().unwrap();

    let mut reply = expect_code(&mut p, 16, Duration::from_secs(15))
        .expect("the sharer answers with a UserInfoResponse");
    reply.set_pointer(8);
    let description = reply.read_string();
    let has_picture = reply.read_bool();
    let upload_slots = reply.read_int32();
    let queue_size = reply.read_int32();
    let slots_free = reply.read_bool();
    assert!(description.is_empty(), "no description is configured");
    assert!(!has_picture);
    assert_eq!(upload_slots, 3, "the configured slot count");
    assert_eq!(queue_size, 0, "nobody is waiting");
    assert!(slots_free, "nothing is uploading");

    let _ = std::fs::remove_dir_all(share_dir);
}

// Progress assumed every read filled the buffer, so a peer that trickles
// small chunks was reported at several times its real rate.
#[test]
fn download_progress_reports_the_rate_actually_received() {
    let server = server_or_skip!();

    let listen_port = free_port().expect("free listen port");
    let mut client = Client::with_settings(server.listening_settings(
        "e2e_rate_dl",
        "pw",
        listen_port,
    ));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    let filename = "trickle.mp3";
    let content: Vec<u8> = (0..400_000u32).map(|i| (i % 251) as u8).collect();
    let download_dir = unique_download_dir();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let cfg = MockUpload {
        listen_addr: format!("127.0.0.1:{listen_port}"),
        peer_username: "e2e_rate_up".to_string(),
        filename: filename.to_string(),
        content: content.clone(),
        token: 424_260_u32,
        ready: ready_tx,
        token_delay: Duration::ZERO,
    };
    let uploader =
        std::thread::spawn(move || run_mock_slow_uploader(&cfg, None));
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("mock uploader P connection");
    std::thread::sleep(Duration::from_millis(1500));

    let (_download, status_rx) = client
        .download(
            filename.to_string(),
            "e2e_rate_up".to_string(),
            content.len() as u64,
            download_dir.display().to_string(),
        )
        .expect("start download");

    let mut fastest = 0.0f64;
    let mut completed = false;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !completed {
        match status_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(DownloadStatus::InProgress {
                speed_bytes_per_sec,
                ..
            }) => fastest = fastest.max(speed_bytes_per_sec),
            Ok(DownloadStatus::Completed) => completed = true,
            Ok(DownloadStatus::Failed(_) | DownloadStatus::TimedOut) => break,
            _ => {}
        }
    }
    assert!(completed, "the trickled download should complete");
    assert!(fastest > 0.0, "progress should report a rate");
    // The mock sends 4 KiB every 20 ms.
    let sent_rate = 4096.0 / 0.020;
    assert!(
        fastest < 1.5 * sent_rate,
        "reported {fastest} B/s for a peer sending {sent_rate} B/s"
    );

    let _ = uploader.join();
    let _ = std::fs::remove_dir_all(&download_dir);
}

// A listing went out in zlib "stored" blocks, so a big share cost every
// browser its full uncompressed size; names repeat enough to shrink well.
#[test]
fn a_large_listing_travels_compressed() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    let album = share_dir.join("Artist - Album");
    std::fs::create_dir_all(&album).unwrap();
    let mut raw_bytes = 0;
    for n in 0..3000 {
        let name = format!("Artist - Album - {n:04} - Track Title.flac");
        raw_bytes += name.len() + 21;
        std::fs::write(album.join(name), b"x").unwrap();
    }

    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.listening_settings("e2e_zlib_sharer", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));
    let server_addr = format!("{}:{}", server.host, server.port);
    let _qt = login_raw(&server_addr, "e2e_zlib_browser", "pw")
        .expect("third-party client logs in");

    let mut p = connect_retry(
        &format!("127.0.0.1:{sharer_port}"),
        Duration::from_secs(5),
    )
    .expect("dial the sharer");
    p.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    p.write_all(&peer_init_bytes("e2e_zlib_browser", "P", 0))
        .unwrap();
    p.write_all(&MessageFactory::build_get_share_file_list().get_buffer())
        .unwrap();
    p.flush().unwrap();
    let mut response = expect_code(&mut p, 5, Duration::from_secs(15))
        .expect("the sharer answers the browse");

    let frame = response.get_size();
    assert!(
        frame < raw_bytes / 4,
        "a {raw_bytes}-byte listing travelled as {frame} bytes"
    );
    response.set_pointer(8);
    let directories =
        soulseek_rs::message::peer::parse_shared_file_list(&mut response);
    assert_eq!(
        directories.iter().map(|d| d.files.len()).sum::<usize>(),
        3000
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

// The server's announcements (code 66) were dropped on the floor; they are
// how a server tells everyone it is going down.
#[test]
fn a_server_announcement_arrives_as_a_message_from_the_server() {
    let server = server_or_skip!();

    let mut admin = Client::with_settings(server.settings("e2e_admin", "pw"));
    let mut bob =
        Client::with_settings(server.settings("e2e_announce_bob", "pw"));
    admin.connect().expect("admin connect");
    bob.connect().expect("bob connect");
    assert!(admin.login().expect("admin login"));
    assert!(bob.login().expect("bob login"));
    if !grant(&server, "e2e_admin", "admin") {
        println!("e2e skipped: cannot make an admin on this server");
        return;
    }

    admin
        .send_private_message("server", "announcement going down at nine")
        .expect("send the admin command");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut received = Vec::new();
    while Instant::now() < deadline {
        received.extend(bob.take_private_messages());
        if received.iter().any(|m| m.username() == "server") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let announcement = received
        .iter()
        .find(|m| m.username() == "server")
        .expect("bob should hear the announcement");
    assert_eq!(announcement.message(), "going down at nine");
}

// A peer that is offered a slot and never answers must not keep it: with
// two slots by default, two such peers shut uploads down for the session.
#[test]
fn an_unanswered_upload_offer_frees_its_slot() {
    let server = server_or_skip!();
    let (share, folder) = queue_share("expiry", &["silent.mp3", "patient.mp3"]);
    let server_addr = format!("{}:{}", server.host, server.port);

    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share.display().to_string()],
        ..server.listening_settings("e2e_expiry_sharer", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));
    sharer.set_upload_slots(1);
    let listen_addr = format!("127.0.0.1:{sharer_port}");

    let mut silent = queue_as(
        &server_addr,
        &listen_addr,
        "e2e_expiry_silent",
        &format!("{folder}\\silent.mp3"),
    )
    .expect("silent peer queues");
    assert!(silent.takes_a_slot(), "the silent peer is offered the slot");

    let mut patient = queue_as(
        &server_addr,
        &listen_addr,
        "e2e_expiry_patient",
        &format!("{folder}\\patient.mp3"),
    )
    .expect("patient peer queues");
    assert!(
        patient.takes_a_slot_within(Duration::from_mins(1)),
        "the slot must come free once the silent peer's offer expires"
    );
    assert!(
        sharer.take_upload_events().iter().any(|event| {
            event.username == "e2e_expiry_silent"
                && matches!(event.status, UploadStatus::Failed(_))
        }),
        "the operator sees the unanswered offer fail"
    );

    let _ = std::fs::remove_dir_all(share);
}

/// A constant-bitrate MP3: `frames` MPEG-1 Layer III frames at 128 kbps,
/// 44.1 kHz, so every searcher sees a bitrate to filter and sort on.
fn cbr_mp3(frames: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..frames {
        out.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
        out.resize(out.len() + 413, 0);
    }
    out
}

// A shared file with no bitrate fails every `--min-bitrate` filter and sorts
// last in SoulseekQt; the attributes come from the file's own headers.
#[test]
fn a_shared_mp3_advertises_its_bitrate_and_duration() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    // 1000 frames is 26 seconds.
    std::fs::write(share_dir.join("attrprobe song.mp3"), cbr_mp3(1000))
        .unwrap();
    let (_sharer, searcher) = sharer_and_searcher(
        &server,
        &share_dir,
        "e2e_attr_sharer",
        "e2e_attr_searcher",
    );

    let query = "attrprobe";
    let _ = searcher.search(query, Duration::from_secs(3));
    let reply = reply_from(&searcher, query, "e2e_attr_sharer")
        .expect("the sharer answers");
    let file = &reply.files[0];
    assert_eq!(file.attribs.get(&0), Some(&128), "bitrate in kbps");
    assert_eq!(file.attribs.get(&1), Some(&26), "duration in seconds");
    assert_eq!(file.attribs.get(&2), Some(&0), "constant bitrate");

    let _ = std::fs::remove_dir_all(share_dir);
}

// A browse shows what a search shows: the bitrate and duration a searcher
// filters on are in the listing too, as Nicotine+ sends them.
#[test]
fn a_browse_listing_carries_the_files_attributes() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    std::fs::write(share_dir.join("listed.mp3"), cbr_mp3(1000)).unwrap();
    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.listening_settings("e2e_lattr_sharer", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));
    let server_addr = format!("{}:{}", server.host, server.port);
    let _qt = login_raw(&server_addr, "e2e_lattr_browser", "pw")
        .expect("third-party client logs in");

    let directories = third_party_browse(
        &format!("127.0.0.1:{sharer_port}"),
        "e2e_lattr_browser",
        Duration::ZERO,
        Duration::from_secs(15),
    )
    .expect("a listing");
    let listed = directories
        .iter()
        .flat_map(|d| d.files.iter())
        .find(|f| f.name == "listed.mp3")
        .expect("the mp3 is listed");
    assert!(
        listed.attributes.contains(&(0, 128)),
        "{:?}",
        listed.attributes
    );
    assert!(
        listed.attributes.contains(&(1, 26)),
        "{:?}",
        listed.attributes
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

/// The next inbound connection, or a timeout error: a mock that never gets
/// dialled must fail its test rather than hang it.
fn accept_within(
    listener: &std::net::TcpListener,
    timeout: Duration,
) -> std::io::Result<TcpStream> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "no inbound connection",
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    };
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    Ok(stream)
}

/// The PeerInit that opens an inbound connection: who dialled, and as what.
fn peer_init_of(stream: &mut TcpStream) -> std::io::Result<(String, String)> {
    let mut init = read_framed(stream)?;
    assert_eq!(init.get_init_code(), 1, "expected a PeerInit");
    init.set_pointer(5);
    Ok((init.read_string(), init.read_string()))
}

// Searches also travel peer to peer, down a tree of parents and children the
// server assembles from PossibleParents. A leaf dials the candidates with a D
// connection, adopts the first that passes it a search after stating its
// branch, and answers those searches exactly like ones from the server.
// soulfind never hands out parents, so the parent here is a mock: a logged-in
// user the leaf is pointed at directly.
#[test]
fn a_leaf_adopts_a_parent_and_answers_the_searches_it_passes_down() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    std::fs::create_dir_all(share_dir.join("album")).unwrap();
    std::fs::write(share_dir.join("album").join("treesearch.mp3"), b"xxxx")
        .unwrap();

    // The parent and the searcher are two logged-in users with a listener
    // each. Bind all interfaces: soulfind reports the LAN address.
    let server_addr = format!("{}:{}", server.host, server.port);
    let raw_user = |name: &str| {
        let port = free_port().expect("a port for the raw user");
        let listener = std::net::TcpListener::bind(("0.0.0.0", port)).unwrap();
        let mut srv = login_raw(&server_addr, name, "pw").expect("raw login");
        srv.write_all(
            &MessageFactory::build_set_wait_port_message(port).get_buffer(),
        )
        .unwrap();
        (listener, port, srv)
    };
    let (listener, parent_port, _parent_srv) = raw_user("e2e_parent");
    let (searcher, _, _searcher_srv) = raw_user("e2e_searcher");

    let mut leaf = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.listening_settings(
            "e2e_leaf",
            "pw",
            free_port().expect("leaf port"),
        )
    });
    leaf.connect().expect("leaf connect");
    assert!(leaf.login().expect("leaf login"));
    assert_eq!(
        leaf.distributed_branch(),
        ("e2e_leaf".to_string(), 0),
        "a leaf without a parent is its own branch root"
    );

    leaf.consider_parents(vec![(
        "e2e_parent".to_string(),
        "127.0.0.1".to_string(),
        parent_port,
    )])
    .expect("consider parents");

    let mut link = accept_within(&listener, Duration::from_secs(15))
        .expect("the leaf dials the candidate parent");
    assert_eq!(
        peer_init_of(&mut link).unwrap(),
        ("e2e_leaf".to_string(), "D".to_string()),
        "a parent link opens with a distributed PeerInit"
    );

    // We are a branch root at level 0; the leaf hangs one level below us.
    link.write_all(&distributed::build_branch_level(0).get_buffer())
        .unwrap();
    link.write_all(&distributed::build_branch_root("e2e_parent").get_buffer())
        .unwrap();
    link.write_all(
        &distributed::build_search("e2e_searcher", 4242, "treesearch")
            .get_buffer(),
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while leaf.distributed_branch().1 == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        leaf.distributed_branch(),
        ("e2e_parent".to_string(), 1),
        "the first search from a candidate with a branch adopts it"
    );

    // The answer goes to whoever asked, not to the parent that relayed it:
    // a P connection to the searcher carrying a FileSearchResponse for the
    // token.
    let mut p = accept_within(&searcher, Duration::from_secs(20))
        .expect("the leaf dials the searcher to answer");
    assert_eq!(
        peer_init_of(&mut p).unwrap(),
        ("e2e_leaf".to_string(), "P".to_string())
    );
    let response = expect_code(&mut p, 9, Duration::from_secs(15))
        .expect("a FileSearchResponse for the search from the tree");
    let body = soulseek_rs::utils::zlib::inflate(&response.get_data()[8..])
        .expect("the response body inflates");
    let name_len = u32::from_le_bytes(body[..4].try_into().unwrap()) as usize;
    let token_at = 4 + name_len;
    assert_eq!(&body[4..token_at], b"e2e_leaf");
    assert_eq!(
        u32::from_le_bytes(body[token_at..token_at + 4].try_into().unwrap()),
        4242
    );

    // Losing the parent puts the leaf back on its own.
    drop(link);
    let deadline = Instant::now() + Duration::from_secs(10);
    while leaf.distributed_branch().1 != 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(leaf.distributed_branch(), ("e2e_leaf".to_string(), 0));

    let _ = std::fs::remove_dir_all(share_dir);
}

// ---------------------------------------------------------------------------
// Room tickers, the global room feed, interests and the targeted searches.
//
// Every one of these is a message the client can now speak; the point of
// driving them through soulfind is that the wire shapes are right in both
// directions, not just that they parse.
// ---------------------------------------------------------------------------

/// Poll `client`'s room events until `pick` matches one, or time out.
fn await_room_event<T>(
    client: &Client,
    timeout: Duration,
    mut pick: impl FnMut(&soulseek_rs::types::RoomEvent) -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        for event in client.take_room_events() {
            if let Some(found) = pick(&event) {
                return Some(found);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

#[test]
fn a_ticker_set_in_a_room_reaches_the_other_members() {
    use soulseek_rs::types::RoomEvent;
    let server = server_or_skip!();

    let room = "e2e_ticker_room";
    let mut alice =
        Client::with_settings(server.settings("e2e_tick_alice", "pw"));
    let mut bob = Client::with_settings(server.settings("e2e_tick_bob", "pw"));
    alice.connect().expect("alice connect");
    bob.connect().expect("bob connect");
    assert!(alice.login().expect("alice login"));
    assert!(bob.login().expect("bob login"));

    alice.join_room(room).expect("alice joins");
    bob.join_room(room).expect("bob joins");
    std::thread::sleep(Duration::from_millis(500));
    let _ = bob.take_room_events();

    alice
        .set_room_ticker(room, "alice was here")
        .expect("set ticker");

    let seen =
        await_room_event(&bob, Duration::from_secs(5), |event| match event {
            RoomEvent::TickerAdded {
                room: r,
                username,
                ticker,
            } if r == room => Some((username.clone(), ticker.clone())),
            _ => None,
        })
        .expect("bob should see alice's ticker");
    assert_eq!(
        seen,
        ("e2e_tick_alice".to_string(), "alice was here".into())
    );

    // The board is kept, not just the event: a UI reads it back from state.
    let board = bob.room_tickers(room);
    assert_eq!(
        board
            .iter()
            .find(|t| t.username == "e2e_tick_alice")
            .map(|t| t.ticker.as_str()),
        Some("alice was here")
    );
}

#[test]
fn a_client_joining_later_receives_the_whole_ticker_board() {
    use soulseek_rs::types::RoomEvent;
    let server = server_or_skip!();

    let room = "e2e_ticker_board";
    let mut alice =
        Client::with_settings(server.settings("e2e_board_alice", "pw"));
    alice.connect().expect("alice connect");
    assert!(alice.login().expect("alice login"));
    alice.join_room(room).expect("alice joins");
    std::thread::sleep(Duration::from_millis(300));
    alice
        .set_room_ticker(room, "standing message")
        .expect("set ticker");
    std::thread::sleep(Duration::from_millis(500));

    // Carol joins afterwards and must be handed the board that already exists.
    let mut carol =
        Client::with_settings(server.settings("e2e_board_carol", "pw"));
    carol.connect().expect("carol connect");
    assert!(carol.login().expect("carol login"));
    carol.join_room(room).expect("carol joins");

    let tickers =
        await_room_event(&carol, Duration::from_secs(5), |event| match event {
            RoomEvent::Tickers { room: r, tickers } if r == room => {
                Some(tickers.clone())
            }
            _ => None,
        })
        .expect("carol should receive the ticker board on join");
    assert!(
        tickers.iter().any(|t| t.username == "e2e_board_alice"
            && t.ticker == "standing message"),
        "the board should carry alice's standing ticker, got {tickers:?}"
    );
}

#[test]
fn the_global_room_feed_carries_a_room_we_never_joined() {
    use soulseek_rs::types::RoomEvent;
    let server = server_or_skip!();

    let room = "e2e_global_source";
    let mut watcher =
        Client::with_settings(server.settings("e2e_global_watch", "pw"));
    let mut talker =
        Client::with_settings(server.settings("e2e_global_talk", "pw"));
    watcher.connect().expect("watcher connect");
    talker.connect().expect("talker connect");
    assert!(watcher.login().expect("watcher login"));
    assert!(talker.login().expect("talker login"));

    watcher.join_global_room().expect("join global room");
    talker.join_room(room).expect("talker joins a room");
    std::thread::sleep(Duration::from_millis(500));
    let _ = watcher.take_room_events();

    let body = "spoken where nobody is watching";
    talker.say_in_room(room, body).expect("say in room");

    let got =
        await_room_event(
            &watcher,
            Duration::from_secs(5),
            |event| match event {
                RoomEvent::GlobalMessage {
                    room: r,
                    username,
                    message,
                } if message == body => Some((r.clone(), username.clone())),
                _ => None,
            },
        )
        .expect("the global feed should carry the message");
    assert_eq!(got, (room.to_string(), "e2e_global_talk".to_string()));

    // And leaving the feed stops it: the next message must not arrive.
    watcher.leave_global_room().expect("leave global room");
    std::thread::sleep(Duration::from_millis(500));
    let _ = watcher.take_room_events();
    talker
        .say_in_room(room, "after leaving")
        .expect("say again");
    let after = await_room_event(&watcher, Duration::from_secs(2), |event| {
        matches!(event, RoomEvent::GlobalMessage { message, .. }
            if message == "after leaving")
        .then_some(())
    });
    assert!(after.is_none(), "the feed should stop once left");
}

#[test]
fn a_user_search_is_answered_by_the_user_it_names() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    let content: Vec<u8> = (0..1024u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(share_dir.join("e2e_usersearch_grail.bin"), &content)
        .unwrap();

    let (_sharer, searcher) = sharer_and_searcher(
        &server,
        &share_dir,
        "e2e_us_sharer",
        "e2e_us_seeker",
    );

    let query = "grail";
    searcher
        .search_user("e2e_us_sharer", query)
        .expect("user search");

    let reply = reply_from(&searcher, query, "e2e_us_sharer")
        .expect("the named user should answer a user search");
    assert!(
        reply
            .files
            .iter()
            .any(|f| f.name.contains("e2e_usersearch_grail")),
        "the reply should carry the matching file, got {:?}",
        reply.files
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

#[test]
fn a_room_search_is_answered_by_the_rooms_members() {
    let server = server_or_skip!();

    let room = "e2e_search_room";
    let share_dir = unique_download_dir();
    let content: Vec<u8> = (0..1024u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(share_dir.join("e2e_roomsearch_relic.bin"), &content)
        .unwrap();

    let (sharer, searcher) = sharer_and_searcher(
        &server,
        &share_dir,
        "e2e_rs_sharer",
        "e2e_rs_seeker",
    );
    sharer.join_room(room).expect("sharer joins");
    searcher.join_room(room).expect("searcher joins");
    std::thread::sleep(Duration::from_millis(750));

    let query = "relic";
    searcher.search_room(room, query).expect("room search");

    let reply = reply_from(&searcher, query, "e2e_rs_sharer")
        .expect("a member of the room should answer a room search");
    assert!(
        reply
            .files
            .iter()
            .any(|f| f.name.contains("e2e_roomsearch_relic")),
        "the reply should carry the matching file, got {:?}",
        reply.files
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

#[test]
fn shared_interests_make_two_users_similar() {
    let server = server_or_skip!();

    let mut alice =
        Client::with_settings(server.settings("e2e_like_alice", "pw"));
    let mut bob = Client::with_settings(server.settings("e2e_like_bob", "pw"));
    alice.connect().expect("alice connect");
    bob.connect().expect("bob connect");
    assert!(alice.login().expect("alice login"));
    assert!(bob.login().expect("bob login"));

    let item = "e2e_interest_krautrock";
    alice.add_interest(item).expect("alice likes it");
    bob.add_interest(item).expect("bob likes it too");
    alice
        .add_dislike("e2e_interest_muzak")
        .expect("alice dislikes something");
    std::thread::sleep(Duration::from_millis(500));

    // Who else likes this item (code 112).
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut likers = Vec::new();
    while Instant::now() < deadline
        && !likers.iter().any(|u| u == "e2e_like_bob")
    {
        alice
            .request_item_similar_users(item)
            .expect("ask who likes it");
        std::thread::sleep(Duration::from_millis(250));
        likers = alice.item_similar_users(item);
    }
    assert!(
        likers.iter().any(|u| u == "e2e_like_bob"),
        "bob likes the same item, got {likers:?}"
    );

    // The overlap also makes bob a similar user (code 110).
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut similar = Vec::new();
    while Instant::now() < deadline
        && !similar
            .iter()
            .any(|u: &soulseek_rs::SimilarUser| u.username == "e2e_like_bob")
    {
        alice
            .request_similar_users()
            .expect("ask for similar users");
        std::thread::sleep(Duration::from_millis(250));
        similar = alice.similar_users();
    }
    assert!(
        similar
            .iter()
            .any(|u| u.username == "e2e_like_bob" && u.weight > 0),
        "bob should be similar with a non-zero weight, got {similar:?}"
    );

    // And alice's own likes and hates read back over code 57.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut interests = None;
    while Instant::now() < deadline && interests.is_none() {
        bob.request_user_interests("e2e_like_alice")
            .expect("ask about alice");
        std::thread::sleep(Duration::from_millis(250));
        interests = bob.user_interests("e2e_like_alice");
    }
    let interests = interests.expect("bob should learn alice's interests");
    assert!(interests.likes.iter().any(|i| i == item));
    assert!(
        interests.hates.iter().any(|i| i == "e2e_interest_muzak"),
        "hates should come back too, got {interests:?}"
    );

    // A dropped interest stops being reported.
    alice.remove_interest(item).expect("alice drops it");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut still_liked = true;
    while Instant::now() < deadline && still_liked {
        bob.request_user_interests("e2e_like_alice")
            .expect("ask again");
        std::thread::sleep(Duration::from_millis(250));
        still_liked = bob
            .user_interests("e2e_like_alice")
            .is_none_or(|i| i.likes.iter().any(|l| l == item));
    }
    assert!(
        !still_liked,
        "a removed interest should stop being reported"
    );
}

#[test]
fn recommendations_are_returned_for_our_interests() {
    let server = server_or_skip!();

    // Recommendations come from what *other* users who share an interest also
    // like: bob likes both items, so alice, who likes only the first, should
    // be recommended the second.
    let mut alice =
        Client::with_settings(server.settings("e2e_rec_alice", "pw"));
    let mut bob = Client::with_settings(server.settings("e2e_rec_bob", "pw"));
    alice.connect().expect("alice connect");
    bob.connect().expect("bob connect");
    assert!(alice.login().expect("alice login"));
    assert!(bob.login().expect("bob login"));

    let shared = "e2e_rec_shared";
    let other = "e2e_rec_other";
    alice.add_interest(shared).expect("alice likes shared");
    bob.add_interest(shared).expect("bob likes shared");
    bob.add_interest(other).expect("bob likes other");
    std::thread::sleep(Duration::from_millis(500));

    let deadline = Instant::now() + Duration::from_secs(8);
    let mut recommended = Vec::new();
    while Instant::now() < deadline
        && !recommended
            .iter()
            .any(|r: &soulseek_rs::Recommendation| r.item == other)
    {
        alice
            .request_recommendations()
            .expect("ask for recommendations");
        std::thread::sleep(Duration::from_millis(250));
        recommended =
            alice.recommendations().map(|(r, _)| r).unwrap_or_default();
    }
    assert!(
        recommended.iter().any(|r| r.item == other),
        "the other item bob likes should be recommended, got {recommended:?}"
    );

    // The server-wide list (code 56) answers too, with the same shape.
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut global = None;
    while Instant::now() < deadline && global.is_none() {
        alice
            .request_global_recommendations()
            .expect("ask for global recommendations");
        std::thread::sleep(Duration::from_millis(250));
        global = alice.global_recommendations();
    }
    let (global_items, _) = global.expect("the server should answer code 56");
    assert!(
        global_items.iter().any(|r| r.item == shared),
        "an item two users like should show server-wide, got {global_items:?}"
    );

    // And per-item recommendations (code 111) answer for a named item.
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut item_recs = Vec::new();
    while Instant::now() < deadline && item_recs.is_empty() {
        alice
            .request_item_recommendations(shared)
            .expect("ask about one item");
        std::thread::sleep(Duration::from_millis(250));
        item_recs = alice.item_recommendations(shared);
    }
    assert!(
        item_recs.iter().any(|r| r.item == other),
        "people who like {shared} also like {other}, got {item_recs:?}"
    );
}

#[test]
fn a_ping_leaves_the_session_usable() {
    let server = server_or_skip!();

    // A ping has no reply, so what it must not do is upset the session: the
    // very next request has to still be answered.
    let mut client =
        Client::with_settings(server.settings("e2e_ping_user", "pw"));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    for _ in 0..3 {
        client.ping_server().expect("ping");
        std::thread::sleep(Duration::from_millis(100));
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut listed = false;
    while Instant::now() < deadline && !listed {
        client.request_room_list().expect("request room list");
        std::thread::sleep(Duration::from_millis(250));
        listed = !client.room_list().is_empty()
            || client.session_loss().is_none() && Instant::now() > deadline;
    }
    assert!(
        client.session_loss().is_none(),
        "pings must not cost us the session"
    );
}

#[test]
fn one_message_reaches_several_users_at_once() {
    let server = server_or_skip!();

    let mut sender =
        Client::with_settings(server.settings("e2e_many_sender", "pw"));
    let mut first =
        Client::with_settings(server.settings("e2e_many_first", "pw"));
    let mut second =
        Client::with_settings(server.settings("e2e_many_second", "pw"));
    for client in [&mut sender, &mut first, &mut second] {
        client.connect().expect("connect");
        assert!(client.login().expect("login"));
    }

    let body = "one message, two recipients";
    sender
        .send_private_message_to_many(
            &["e2e_many_first".to_string(), "e2e_many_second".to_string()],
            body,
        )
        .expect("send to many");

    for client in [&first, &second] {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got = false;
        while Instant::now() < deadline && !got {
            got = client
                .take_private_messages()
                .iter()
                .any(|m| m.message() == body);
            if !got {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        assert!(got, "every named recipient should receive the message");
    }
}

#[test]
fn a_changed_password_is_what_the_next_login_needs() {
    let server = server_or_skip!();

    let user = "e2e_pwchange_user";
    let mut client = Client::with_settings(server.settings(user, "first-pw"));
    client.connect().expect("connect");
    assert!(client.login().expect("login registers the account"));

    client
        .change_password("second-pw")
        .expect("change password");
    std::thread::sleep(Duration::from_millis(500));
    drop(client);

    let mut stale = Client::with_settings(server.settings(user, "first-pw"));
    stale.connect().expect("connect with the old password");
    assert!(
        !matches!(stale.login(), Ok(true)),
        "the old password must stop working"
    );
    drop(stale);

    let mut fresh = Client::with_settings(server.settings(user, "second-pw"));
    fresh.connect().expect("connect with the new password");
    assert!(
        fresh.login().expect("login with the new password"),
        "the new password must be accepted"
    );
}

#[test]
fn a_brokered_dial_we_cannot_complete_tells_the_waiting_peer() {
    // The peer asks the server to broker a connection to us but is itself
    // unreachable — it advertises a port nothing listens on. Our dial fails,
    // and the peer must be told (CantConnectToPeer, code 1001) rather than
    // left waiting for a connection that is never coming.
    let server = server_or_skip!();
    let addr = format!("{}:{}", server.host, server.port);

    let mut client =
        Client::with_settings(server.settings("e2e_cantconn_us", "pw"));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    let mut peer =
        login_raw(&addr, "e2e_cantconn_peer", "pw").expect("peer login");
    // A port that is free is a port nothing answers on: the dial is refused.
    let dead_port = free_port().expect("free port");
    peer.write_all(
        &MessageFactory::build_set_wait_port_message(dead_port).get_buffer(),
    )
    .expect("set wait port");
    peer.flush().expect("flush wait port");

    let token = 424_242_u32;
    peer.write_all(
        &MessageFactory::build_connect_to_peer(
            token,
            "e2e_cantconn_us",
            ConnectionType::P,
        )
        .get_buffer(),
    )
    .expect("ask the server to broker");
    peer.flush().expect("flush broker request");

    let mut reply = read_until_code(&mut peer, 1001, Duration::from_secs(20))
        .expect("the peer should be told the connection failed");
    reply.set_pointer(8);
    assert_eq!(
        reply.read_int32(),
        token,
        "the reply should quote the token the peer asked with"
    );
}

// ---------------------------------------------------------------------------
// Features that were implemented but never driven end to end: the away
// status, the phrases the server refuses to search for, and asking a peer
// where our queued file sits.
// ---------------------------------------------------------------------------

#[test]
fn going_away_is_visible_to_another_user() {
    use soulseek_rs::types::UserStatus;
    let server = server_or_skip!();

    let mut alice =
        Client::with_settings(server.settings("e2e_away_alice", "pw"));
    let mut bob = Client::with_settings(server.settings("e2e_away_bob", "pw"));
    alice.connect().expect("alice connect");
    bob.connect().expect("bob connect");
    assert!(alice.login().expect("alice login"));
    assert!(bob.login().expect("bob login"));

    // Online to begin with, as the post-login handshake announces.
    let status_of = |client: &Client| -> Option<UserStatus> {
        client
            .user_info("e2e_away_alice")
            .and_then(|info| info.presence)
            .map(|presence| presence.status)
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline
        && status_of(&bob) != Some(UserStatus::Online)
    {
        bob.request_user_info("e2e_away_alice").expect("ask");
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(status_of(&bob), Some(UserStatus::Online));

    alice.set_away(true).expect("alice goes away");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && status_of(&bob) != Some(UserStatus::Away)
    {
        bob.request_user_info("e2e_away_alice").expect("ask again");
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(
        status_of(&bob),
        Some(UserStatus::Away),
        "the away status should reach another user"
    );

    // And coming back is visible too.
    alice.set_away(false).expect("alice comes back");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline
        && status_of(&bob) != Some(UserStatus::Online)
    {
        bob.request_user_info("e2e_away_alice")
            .expect("ask once more");
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(status_of(&bob), Some(UserStatus::Online));
}

/// A minimal server that answers a login and then sends `after_login`.
///
/// soulfind keeps its search filters in its database, which the suite has no
/// way to seed, so the excluded-phrase path is driven from a stub that speaks
/// just enough of the protocol to deliver code 160 on the wire.
fn stub_server_sending(
    after_login: Vec<Message>,
) -> (String, std::thread::JoinHandle<()>) {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub server");
    let addr = listener.local_addr().expect("stub addr");
    let handle = std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("stub read timeout");
        // Wait for the login (code 1) before answering it.
        loop {
            match read_framed(&mut stream) {
                Ok(msg) if msg.get_message_code() == 1 => break,
                Ok(_) => {}
                Err(_) => return,
            }
        }
        let mut ok = Message::new();
        ok.write_int32(1)
            .write_int8(1)
            .write_string("stub greeting")
            .write_int32(0)
            .write_string("")
            .write_int8(0);
        if stream.write_all(&ok.get_buffer()).is_err() {
            return;
        }
        for message in after_login {
            if stream.write_all(&message.get_buffer()).is_err() {
                return;
            }
        }
        let _ = stream.flush();
        // Hold the connection open so the client stays logged in.
        std::thread::sleep(Duration::from_secs(10));
    });
    (format!("{}:{}", addr.ip(), addr.port()), handle)
}

#[test]
fn phrases_the_server_excludes_are_kept_for_our_replies() {
    // The server tells clients which phrases are excluded from the search
    // network (code 160). Nicotine+ applies them to the files it offers in a
    // search reply, not to the searches it sends, and so do we — this pins
    // that the announced list arrives and is kept for that use.
    let mut phrases = Message::new();
    phrases
        .write_int32(160)
        .write_int32(2)
        .write_string("banned-phrase")
        .write_string("blocked");
    let (addr, stub) = stub_server_sending(vec![phrases]);
    let (host, port) = addr.rsplit_once(':').expect("stub addr");

    let mut client = Client::with_settings(ClientSettings {
        username: "e2e_excluded".to_string(),
        password: "pw".to_string(),
        server_address: PeerAddress::new(
            host.to_string(),
            port.parse().expect("stub port"),
        ),
        enable_listen: false,
        listen_port: 0,
        shared_directories: Vec::new(),
        accept_children: false,
        version: ClientVersion::default(),
    });
    client.connect().expect("connect to stub");
    assert!(client.login().expect("login to stub"));

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline
        && client.excluded_search_phrases().is_empty()
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        client.excluded_search_phrases(),
        ["banned-phrase", "blocked"],
        "the announced phrases should be kept"
    );

    // Our own searches are not policed by them: that is the server's job, and
    // no reference client refuses a query locally.
    assert!(
        client
            .search("a banned-phrase query", Duration::from_millis(100))
            .is_ok()
    );

    drop(client);
    let _ = stub.join();
}

#[test]
fn a_queued_download_can_ask_where_it_sits() {
    // We queue a file with a peer that never starts it, then ask where it
    // sits (peer code 51). The peer's answer (code 44) must land on the
    // download as its queue position.
    let server = server_or_skip!();

    let listen_port = free_port().expect("free listen port");
    let mut client = Client::with_settings(server.listening_settings(
        "e2e_place_asker",
        "pw",
        listen_port,
    ));
    client.connect().expect("connect");
    assert!(client.login().expect("login"));

    let filename = "waiting_in_line.mp3";
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (asked_tx, asked_rx) = std::sync::mpsc::channel();
    let listen_addr = format!("127.0.0.1:{listen_port}");
    let peer = std::thread::spawn(move || -> std::io::Result<()> {
        let mut p = connect_retry(&listen_addr, Duration::from_secs(5))?;
        p.set_read_timeout(Some(Duration::from_secs(15)))?;
        p.write_all(&peer_init_bytes("e2e_place_peer", "P", 0))?;
        p.flush()?;
        let _ = ready_tx.send(());

        // Sit on the queue request, then answer the place request with 7.
        loop {
            let mut msg = read_framed(&mut p)?;
            // A QueueUpload (43) is accepted but never started: the file
            // stays in line, which is the state being asked about.
            if msg.get_message_code() == 51 {
                msg.set_pointer(8);
                let asked = msg.read_string();
                let mut reply = Message::new();
                reply.write_int32(44).write_string(&asked).write_int32(7);
                p.write_all(&reply.get_buffer())?;
                p.flush()?;
                let _ = asked_tx.send(asked);
                break;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
        Ok(())
    });
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("mock peer P connection");
    std::thread::sleep(Duration::from_millis(1000));

    let download_dir = unique_download_dir();
    let (_download, _status_rx) = client
        .download(
            filename.to_string(),
            "e2e_place_peer".to_string(),
            10,
            download_dir.display().to_string(),
        )
        .expect("start download");

    // Ask, retrying until the control connection carries it.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut asked = None;
    while Instant::now() < deadline && asked.is_none() {
        client
            .request_place_in_queue("e2e_place_peer", filename)
            .expect("ask for our place");
        asked = asked_rx.recv_timeout(Duration::from_millis(500)).ok();
    }
    assert_eq!(
        asked.as_deref(),
        Some(filename),
        "the peer should be asked about the file we queued"
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut place = None;
    while Instant::now() < deadline && place.is_none() {
        place = client
            .get_all_downloads()
            .into_iter()
            .find(|d| d.filename == filename)
            .and_then(|d| d.queue_position);
        if place.is_none() {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    assert_eq!(
        place,
        Some(7),
        "the peer's answer should land on the download"
    );

    let _ = peer.join();
    let _ = std::fs::remove_dir_all(download_dir);
}

#[test]
fn a_private_room_is_owned_granted_and_revoked() {
    use soulseek_rs::types::RoomEvent;
    let server = server_or_skip!();

    let room = "e2e_private_club";
    let mut owner =
        Client::with_settings(server.settings("e2e_priv_owner", "pw"));
    let mut guest =
        Client::with_settings(server.settings("e2e_priv_guest", "pw"));
    owner.connect().expect("owner connect");
    guest.connect().expect("guest connect");
    assert!(owner.login().expect("owner login"));
    assert!(guest.login().expect("guest login"));

    guest
        .set_room_invitations_enabled(true)
        .expect("guest accepts invitations");
    owner
        .join_private_room(room)
        .expect("owner creates the room");
    std::thread::sleep(Duration::from_millis(750));

    // The guest is invited, and hears about it (code 139).
    owner
        .add_room_member(room, "e2e_priv_guest")
        .expect("owner invites the guest");
    let granted = await_room_event(&guest, Duration::from_secs(5), |event| {
        matches!(event, RoomEvent::OwnStandingChanged {
            room: r, members: true, granted: true
        } if r == room)
        .then_some(())
    });
    assert!(
        granted.is_some(),
        "the guest should be told they are a member"
    );

    // The owner's own roster now carries the guest (code 134).
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline
        && !owner
            .private_room_members(room)
            .iter()
            .any(|u| u == "e2e_priv_guest")
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        owner
            .private_room_members(room)
            .iter()
            .any(|u| u == "e2e_priv_guest"),
        "the member roster should carry the guest, got {:?}",
        owner.private_room_members(room)
    );

    // A member can be made an operator (code 145 for them, 143 for the room).
    owner
        .add_room_operator(room, "e2e_priv_guest")
        .expect("owner promotes the guest");
    let promoted = await_room_event(&guest, Duration::from_secs(5), |event| {
        matches!(event, RoomEvent::OwnStandingChanged {
            room: r, members: false, granted: true
        } if r == room)
        .then_some(())
    });
    assert!(
        promoted.is_some(),
        "the guest should be told they run the room"
    );

    // And membership can be taken away again (code 140).
    owner
        .remove_room_member(room, "e2e_priv_guest")
        .expect("owner removes the guest");
    let revoked = await_room_event(&guest, Duration::from_secs(5), |event| {
        matches!(event, RoomEvent::OwnStandingChanged {
            room: r, members: true, granted: false
        } if r == room)
        .then_some(())
    });
    assert!(revoked.is_some(), "the guest should be told they are out");
}

#[test]
fn a_private_room_someone_else_owns_cannot_be_taken() {
    use soulseek_rs::types::RoomEvent;
    let server = server_or_skip!();

    let room = "e2e_private_taken";
    let mut owner =
        Client::with_settings(server.settings("e2e_taken_owner", "pw"));
    owner.connect().expect("owner connect");
    assert!(owner.login().expect("owner login"));
    owner
        .join_private_room(room)
        .expect("owner creates the room");
    std::thread::sleep(Duration::from_millis(750));

    // An outsider asking for the same name must be refused (code 1003), not
    // handed the room.
    let mut outsider =
        Client::with_settings(server.settings("e2e_taken_outsider", "pw"));
    outsider.connect().expect("outsider connect");
    assert!(outsider.login().expect("outsider login"));
    outsider
        .join_private_room(room)
        .expect("outsider asks for the same room");

    let refused =
        await_room_event(&outsider, Duration::from_secs(5), |event| {
            matches!(event, RoomEvent::CantCreate { room: r } if r == room)
                .then_some(())
        });
    assert!(
        refused.is_some(),
        "a private room owned by someone else must be refused"
    );
    assert!(
        outsider.private_rooms().is_empty(),
        "a refused room must not appear as one we belong to"
    );
}

#[test]
fn an_acknowledged_offline_message_is_not_delivered_twice() {
    // Private messages sent to an offline user are stored by the server and
    // handed over at the next login; the client acknowledges each one
    // (MessageAcked, code 23) so the server drops it. Without that ack the
    // same message arrives again on every login, which is what this pins.
    let server = server_or_skip!();

    // Register the recipient so the server will hold mail for them.
    let mut recipient =
        Client::with_settings(server.settings("e2e_ack_recipient", "pw"));
    recipient.connect().expect("recipient connect");
    assert!(recipient.login().expect("recipient login"));
    drop(recipient);
    // The dropped session's socket must actually be gone before the mail is
    // sent, or the server delivers it live to a connection nobody is reading.
    std::thread::sleep(Duration::from_secs(2));

    let mut sender =
        Client::with_settings(server.settings("e2e_ack_sender", "pw"));
    sender.connect().expect("sender connect");
    assert!(sender.login().expect("sender login"));
    let body = "left while you were out";
    sender
        .send_private_message("e2e_ack_recipient", body)
        .expect("send to an offline user");
    std::thread::sleep(Duration::from_millis(500));

    let received_once = |label: &str| -> bool {
        let mut client =
            Client::with_settings(server.settings("e2e_ack_recipient", "pw"));
        client.connect().unwrap_or_else(|e| panic!("{label}: {e}"));
        assert!(client.login().expect("recipient login"), "{label}");
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut got = false;
        while Instant::now() < deadline && !got {
            got = client
                .take_private_messages()
                .iter()
                .any(|m| m.message() == body);
            if !got {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        std::thread::sleep(Duration::from_millis(500));
        got
    };

    assert!(
        received_once("first login"),
        "the stored message should arrive at the next login"
    );
    assert!(
        !received_once("second login"),
        "an acknowledged message must not be delivered again"
    );
}

#[test]
fn a_wishlist_search_is_answered_like_any_other() {
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    let content: Vec<u8> = (0..1024u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(share_dir.join("e2e_wishlist_talisman.bin"), &content)
        .unwrap();

    let (_sharer, searcher) = sharer_and_searcher(
        &server,
        &share_dir,
        "e2e_wish_sharer",
        "e2e_wish_seeker",
    );

    // A wish returns at once; its results accumulate under the query.
    let query = "talisman";
    searcher
        .start_wishlist_search(query)
        .expect("start a wishlist search");

    let reply = reply_from(&searcher, query, "e2e_wish_sharer")
        .expect("a wishlist search should be answered like a plain one");
    assert!(
        reply
            .files
            .iter()
            .any(|f| f.name.contains("e2e_wishlist_talisman")),
        "the reply should carry the matching file, got {:?}",
        reply.files
    );

    // The server also announces how often it will accept one.
    assert!(
        searcher.wishlist_interval() > Duration::ZERO,
        "the wishlist interval should be a real wait"
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

// ---------------------------------------------------------------------------
// The other half of the tree: serving children.
//
// A client that accepts children takes `D` connections from peers, tells each
// where our branch sits, and passes every search it receives down to them.
// ---------------------------------------------------------------------------

/// Read distributed frames from a child link until `pick` matches or time runs
/// out.
fn read_distributed_until<T>(
    stream: &mut TcpStream,
    timeout: Duration,
    mut pick: impl FnMut(&distributed::Distributed) -> Option<T>,
) -> Option<T> {
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .ok()?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // read_framed already returns the frame with its length prefix,
        // which is exactly what the parser expects.
        let Ok(mut frame) = read_framed(stream) else {
            continue;
        };
        if let Some(parsed) = distributed::parse(&mut frame)
            && let Some(found) = pick(&parsed)
        {
            return Some(found);
        }
    }
    None
}

/// Dial `port` as a distributed child called `username`.
fn dial_as_child(port: u16, username: &str) -> std::io::Result<TcpStream> {
    let mut child =
        connect_retry(&format!("127.0.0.1:{port}"), Duration::from_secs(5))?;
    child.write_all(&peer_init_bytes(username, "D", 0))?;
    child.flush()?;
    Ok(child)
}

#[test]
fn a_parent_tells_a_child_where_it_sits_and_passes_searches_down() {
    let server = server_or_skip!();

    let listen_port = free_port().expect("free listen port");
    let mut parent = Client::with_settings(ClientSettings {
        accept_children: true,
        ..server.listening_settings("e2e_par_parent", "pw", listen_port)
    });
    parent.connect().expect("parent connect");
    assert!(parent.login().expect("parent login"));

    // A client takes children only once something feeds it the search
    // stream — here the server, which relays every search to us. Until then
    // a child would hang from a branch that receives nothing, which is why
    // Nicotine+ refuses one too.
    let addr = format!("{}:{}", server.host, server.port);
    let mut searcher =
        login_raw(&addr, "e2e_par_searcher", "pw").expect("searcher login");
    let search = |searcher: &mut TcpStream, token: u32| {
        searcher
            .write_all(
                &MessageFactory::build_file_search_message(
                    token,
                    "e2e_child_relay_probe",
                )
                .get_buffer(),
            )
            .expect("send a search");
        searcher.flush().expect("flush the search");
    };
    search(&mut searcher, 515_150);
    std::thread::sleep(Duration::from_secs(2));

    let mut child =
        dial_as_child(listen_port, "e2e_par_child").expect("child dials");

    // The child is told our branch before anything else: without it, it has
    // nothing to report to the server as its own place.
    let root = read_distributed_until(
        &mut child,
        Duration::from_secs(10),
        |f| match f {
            distributed::Distributed::BranchRoot(root) => Some(root.clone()),
            _ => None,
        },
    )
    .expect("a child should be told our branch root");
    assert_eq!(
        root, "e2e_par_parent",
        "a parentless client is its own root"
    );
    let level = read_distributed_until(
        &mut child,
        Duration::from_secs(10),
        |f| match f {
            distributed::Distributed::BranchLevel(level) => Some(*level),
            _ => None,
        },
    )
    .expect("a child should be told our branch level");
    assert_eq!(level, 0);

    let deadline = Instant::now() + Duration::from_secs(5);
    while parent.children().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(parent.children(), vec!["e2e_par_child".to_string()]);

    // A search the server hands us must reach the child, whether or not we
    // can answer it ourselves — carrying the stream is the parent's duty.
    let token = 515_151_u32;
    search(&mut searcher, token);

    let relayed = read_distributed_until(
        &mut child,
        Duration::from_secs(20),
        |f| match f {
            distributed::Distributed::Search {
                username,
                token: got,
                query,
            } if *got == token => Some((username.clone(), query.clone())),
            _ => None,
        },
    )
    .expect("the search should be passed down to the child");
    assert_eq!(
        relayed,
        (
            "e2e_par_searcher".to_string(),
            "e2e_child_relay_probe".to_string()
        )
    );

    // A second link from the same child is refused: the one we hold is live.
    let mut duplicate =
        dial_as_child(listen_port, "e2e_par_child").expect("child dials again");
    let told_again =
        read_distributed_until(&mut duplicate, Duration::from_secs(3), |f| {
            matches!(f, distributed::Distributed::BranchRoot(_)).then_some(())
        });
    assert!(told_again.is_none(), "a child we carry is not taken twice");
    assert_eq!(parent.children(), vec!["e2e_par_child".to_string()]);
    drop(duplicate);

    // A child that hangs up loses its place.
    drop(child);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut next_token = token + 1;
    while !parent.children().is_empty() && Instant::now() < deadline {
        search(&mut searcher, next_token);
        next_token += 1;
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(
        parent.children().is_empty(),
        "a child that went away should not keep its slot"
    );
}

#[test]
fn a_client_that_does_not_serve_children_turns_one_away() {
    let server = server_or_skip!();

    // The default: a leaf. A peer that dials with a D connection is dropped,
    // and never told a branch it could hang from.
    let listen_port = free_port().expect("free listen port");
    let mut leaf = Client::with_settings(server.listening_settings(
        "e2e_leafonly",
        "pw",
        listen_port,
    ));
    leaf.connect().expect("leaf connect");
    assert!(leaf.login().expect("leaf login"));

    let mut child =
        dial_as_child(listen_port, "e2e_leafonly_child").expect("child dials");

    let told =
        read_distributed_until(&mut child, Duration::from_secs(3), |f| {
            matches!(f, distributed::Distributed::BranchRoot(_)).then_some(())
        });
    assert!(told.is_none(), "a leaf must not take on a child");
    assert!(leaf.children().is_empty());
}

#[test]
fn one_folder_of_a_peers_shares_can_be_asked_for_by_itself() {
    // "Download folder" in other clients asks for one folder (peer code 36)
    // rather than pulling the peer's whole listing.
    let server = server_or_skip!();

    let share_dir = unique_download_dir();
    std::fs::create_dir_all(share_dir.join("wanted")).unwrap();
    std::fs::create_dir_all(share_dir.join("unwanted")).unwrap();
    std::fs::write(share_dir.join("wanted").join("keep.bin"), b"abcd").unwrap();
    std::fs::write(share_dir.join("unwanted").join("skip.bin"), b"efgh")
        .unwrap();

    let (_sharer, asker) = sharer_and_searcher(
        &server,
        &share_dir,
        "e2e_folder_sharer",
        "e2e_folder_asker",
    );

    // The folder is named the way the sharer advertises it: the share root's
    // basename, then the subfolder, backslash-separated.
    let root = share_dir.file_name().unwrap().to_string_lossy().to_string();
    let folder = format!("{root}\\wanted");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut listing = None;
    while Instant::now() < deadline && listing.is_none() {
        asker
            .request_folder_contents("e2e_folder_sharer", &folder)
            .expect("ask for one folder");
        std::thread::sleep(Duration::from_millis(500));
        listing = asker.take_folder_contents("e2e_folder_sharer", &folder);
    }
    let listing = listing.expect("the peer should answer with that folder");

    assert!(
        listing
            .iter()
            .any(|dir| dir.files.iter().any(|f| f.name.contains("keep.bin"))),
        "the folder we asked for should be in the answer, got {listing:?}"
    );
    assert!(
        !listing
            .iter()
            .any(|dir| dir.files.iter().any(|f| f.name.contains("skip.bin"))),
        "no other folder should come with it, got {listing:?}"
    );

    let _ = std::fs::remove_dir_all(share_dir);
}

#[test]
fn our_interests_are_sent_again_after_a_new_login() {
    // The server keeps interests only for the session that set them, so a
    // client that does not send them again comes back with none — which is
    // why Nicotine+ re-sends its list at every login, from its config. Ours
    // holds the list itself: an interest set before there is a connection is
    // still on the server once one is made.
    let server = server_or_skip!();

    let item = "e2e_relike_shoegaze";
    let mut alice =
        Client::with_settings(server.settings("e2e_relike_alice", "pw"));
    // Set before connecting: there is nothing to send it over yet, and the
    // login is what puts it on the wire.
    let _ = alice.add_interest(item);
    assert_eq!(alice.own_interests().likes, [item]);

    alice.connect().expect("alice connect");
    assert!(alice.login().expect("alice login"));
    std::thread::sleep(Duration::from_millis(500));

    let mut watcher =
        Client::with_settings(server.settings("e2e_relike_watch", "pw"));
    watcher.connect().expect("watcher connect");
    assert!(watcher.login().expect("watcher login"));

    let likes_of_alice = |watcher: &Client| -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut likes = Vec::new();
        while Instant::now() < deadline && !likes.iter().any(|i| i == item) {
            watcher
                .request_user_interests("e2e_relike_alice")
                .expect("ask about alice");
            std::thread::sleep(Duration::from_millis(250));
            likes = watcher
                .user_interests("e2e_relike_alice")
                .map(|i| i.likes)
                .unwrap_or_default();
        }
        likes
    };

    let likes = likes_of_alice(&watcher);
    assert!(
        likes.iter().any(|i| i == item),
        "the login should have carried the interest with it, got {likes:?}"
    );

    // The same client, in a new session: its list goes back up by itself.
    drop(alice);
    std::thread::sleep(Duration::from_secs(1));
    let mut again =
        Client::with_settings(server.settings("e2e_relike_alice", "pw"));
    let _ = again.add_interest(item);
    again.connect().expect("reconnect");
    assert!(again.login().expect("re-login"));
    std::thread::sleep(Duration::from_millis(500));

    let likes = likes_of_alice(&watcher);
    assert!(
        likes.iter().any(|i| i == item),
        "a new session should carry the interests too, got {likes:?}"
    );
}
