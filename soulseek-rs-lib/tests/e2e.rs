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
//! No external crates are used — the library forbids dependencies, so the
//! harness sticks to `std` and the library's own public API.

#![allow(clippy::doc_markdown)]

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use soulseek_rs::message::Message;
use soulseek_rs::message::server::MessageFactory;
use soulseek_rs::peer::ConnectionType;
use soulseek_rs::{Client, ClientSettings, DownloadStatus, PeerAddress};

/// A Soulseek server to test against: either a child soulfind process we
/// spawned, or an external server referenced by `SOULSEEK_TEST_SERVER`.
struct TestServer {
    host: String,
    port: u16,
    child: Option<Child>,
    db: Option<PathBuf>,
}

impl TestServer {
    /// Resolve a server to test against, or `None` if the suite should skip.
    fn resolve() -> Option<Self> {
        if let Ok(addr) = std::env::var("SOULSEEK_TEST_SERVER") {
            let (host, port) = split_host_port(&addr)?;
            wait_until_listening(&host, port, Duration::from_secs(2))?;
            return Some(Self {
                host,
                port,
                child: None,
                db: None,
            });
        }
        Self::spawn()
    }

    /// Spawn a local soulfind on an ephemeral port with a throwaway database.
    fn spawn() -> Option<Self> {
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

fn split_host_port(addr: &str) -> Option<(String, u16)> {
    let (host, port) = addr.rsplit_once(':')?;
    Some((host.to_string(), port.parse().ok()?))
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

#[test]
fn two_clients_can_be_logged_in_together() {
    let server = server_or_skip!();

    // The server must handle several independent sessions at once — this is
    // the precondition for any peer-to-peer feature routed through it.
    let mut alice = Client::with_settings(server.settings("e2e_alice", "pw_a"));
    let mut bob = Client::with_settings(server.settings("e2e_bob", "pw_b"));

    alice.connect().expect("alice connect");
    bob.connect().expect("bob connect");

    assert!(alice.login().expect("alice login"));
    assert!(bob.login().expect("bob login"));
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

fn run_mock_uploader(cfg: &MockUpload) -> std::io::Result<()> {
    // 1. P (control) connection: register ourselves with the downloader.
    let mut p = connect_retry(&cfg.listen_addr, Duration::from_secs(5))?;
    p.set_read_timeout(Some(Duration::from_secs(10)))?;
    p.write_all(&peer_init_bytes(&cfg.peer_username, "P", 0))?;
    p.flush()?;
    let _ = cfg.ready.send(());

    // 2. Wait for the downloader's QueueUpload (peer code 43).
    loop {
        let mut msg = read_framed(&mut p)?;
        if msg.get_message_code() == 43 {
            msg.set_pointer(8);
            let _requested = msg.read_string();
            break;
        }
    }

    // 3. Offer the transfer with a TransferRequest (peer code 40). The size here
    //    becomes the download's expected size, so it must match the content.
    let mut tr = Message::new();
    tr.write_int32(40)
        .write_int32(1) // direction: upload
        .write_int32(cfg.token)
        .write_string(&cfg.filename)
        .write_int64(cfg.content.len() as u64);
    p.write_all(&tr.get_buffer())?;
    p.flush()?;

    // 4. Wait for the downloader to allow it (TransferResponse, peer code 41).
    loop {
        let msg = read_framed(&mut p)?;
        if msg.get_message_code() == 41 {
            break;
        }
    }

    // 5. Open the F (file) connection to the downloader's listener and stream.
    serve_file_over_f(
        &cfg.listen_addr,
        &cfg.peer_username,
        cfg.token,
        &cfg.content,
    )
}

/// Open an F (file transfer) connection to `downloader_addr` and stream
/// `content`. PeerInit(F) and the 4-byte transfer token are written together so
/// the token lands in the listener's read buffer, where the download is looked
/// up by token; the downloader then sends an 8-byte START_DOWNLOAD offset before
/// we send the bytes.
fn serve_file_over_f(
    downloader_addr: &str,
    username: &str,
    token: u32,
    content: &[u8],
) -> std::io::Result<()> {
    let mut f = connect_retry(downloader_addr, Duration::from_secs(5))?;
    f.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut init = peer_init_bytes(username, "F", token);
    init.extend_from_slice(&token.to_le_bytes());
    f.write_all(&init)?;
    f.flush()?;

    let mut start = [0u8; 8];
    f.read_exact(&mut start)?;
    f.write_all(content)?;
    f.flush()?;

    // Keep the connection open briefly so the reader drains everything.
    std::thread::sleep(Duration::from_millis(500));
    Ok(())
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
        &MessageFactory::build_login_message(username, password).get_buffer(),
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
    code: u8,
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
    code: u8,
    timeout: Duration,
) -> std::io::Result<Message> {
    read_until_code(stream, code, timeout).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("timed out waiting for message code {code}"),
        )
    })
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

#[test]
fn a_file_downloads_from_a_peer_over_p_and_f_connections() {
    let server = server_or_skip!();

    // Downloader: connected to the server with its peer listener enabled.
    let listen_port = free_port().expect("free listen port");
    let mut client = Client::with_settings(server.listening_settings(
        "e2e_downloader",
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
        peer_username: "e2e_mockpeer".to_string(),
        filename: filename.to_string(),
        content: content.clone(),
        token,
        ready: ready_tx,
    };
    let uploader = std::thread::spawn(move || {
        if let Err(e) = run_mock_uploader(&cfg) {
            eprintln!("[mock uploader] {e}");
        }
    });

    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("mock uploader P connection");

    // The listener registers an incoming peer under "<username>:direct"; give
    // that registration a moment to complete before queuing the download.
    std::thread::sleep(Duration::from_millis(1500));

    let (_download, status_rx) = client
        .download(
            filename.to_string(),
            "e2e_mockpeer:direct".to_string(),
            size,
            download_dir.display().to_string(),
        )
        .expect("start download");

    // Wait for completion via the per-download status channel.
    let mut completed = false;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match status_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(DownloadStatus::Completed) => {
                completed = true;
                break;
            }
            Ok(DownloadStatus::Failed(_) | DownloadStatus::TimedOut) => {
                break;
            }
            _ => {}
        }
        if client
            .get_all_downloads()
            .iter()
            .any(|d| matches!(d.status, DownloadStatus::Completed))
        {
            completed = true;
            break;
        }
    }
    let _ = uploader.join();

    assert!(completed, "the download should reach Completed");

    let written = std::fs::read(download_dir.join(filename))
        .expect("downloaded file should exist");
    assert_eq!(written, content, "downloaded bytes should match the source");

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
}

fn run_mock_direct_peer(cfg: &MockDirectUpload) -> std::io::Result<()> {
    // 1. Log in to the server so it knows this user is online.
    let mut srv = connect_retry(&cfg.server_addr, Duration::from_secs(5))?;
    srv.set_read_timeout(Some(Duration::from_secs(10)))?;
    srv.write_all(
        &MessageFactory::build_login_message(&cfg.username, &cfg.password)
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
    assert_eq!(init.get_message_code(), 1, "expected inbound PeerInit");
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

    // 5. Stream the bytes over an F connection to the downloader's listener.
    //    `srv` stays in scope so the peer remains online for the whole transfer.
    serve_file_over_f(
        &cfg.downloader_listen_addr,
        &cfg.username,
        cfg.token,
        &cfg.content,
    )
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

    let mut completed = false;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match status_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(DownloadStatus::Completed) => {
                completed = true;
                break;
            }
            Ok(DownloadStatus::Failed(_) | DownloadStatus::TimedOut) => break,
            _ => {}
        }
        if client
            .get_all_downloads()
            .iter()
            .any(|d| matches!(d.status, DownloadStatus::Completed))
        {
            completed = true;
            break;
        }
    }
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
    )
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

    let mut completed = false;
    let deadline = Instant::now() + Duration::from_secs(25);
    while Instant::now() < deadline {
        match status_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(DownloadStatus::Completed) => {
                completed = true;
                break;
            }
            Ok(DownloadStatus::Failed(_) | DownloadStatus::TimedOut) => break,
            _ => {}
        }
        if client
            .get_all_downloads()
            .iter()
            .any(|d| matches!(d.status, DownloadStatus::Completed))
        {
            completed = true;
            break;
        }
    }
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

#[test]
fn two_real_clients_search_and_download() {
    let server = server_or_skip!();

    // Sharer with one distinctively named file.
    let share_dir = unique_download_dir();
    let content: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
    let filename = "e2e_probe_xyzzy.bin";
    std::fs::write(share_dir.join(filename), &content).unwrap();

    let sharer_port = free_port().expect("sharer port");
    let mut sharer = Client::with_settings(ClientSettings {
        shared_directories: vec![share_dir.display().to_string()],
        ..server.listening_settings("e2e_sharer", "pw", sharer_port)
    });
    sharer.connect().expect("sharer connect");
    assert!(sharer.login().expect("sharer login"));

    let leecher_port = free_port().expect("leecher port");
    let mut leecher = Client::with_settings(server.listening_settings(
        "e2e_leecher",
        "pw",
        leecher_port,
    ));
    leecher.connect().expect("leecher connect");
    assert!(leecher.login().expect("leecher login"));

    // Let soulfind register both SetWaitPorts before the search resolves peers.
    std::thread::sleep(Duration::from_secs(1));

    let query = "xyzzy";
    let _ = leecher.search(query, Duration::from_secs(3));

    // Poll until the sharer's response for our file arrives.
    let mut hit: Option<(String, u64)> = None;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline && hit.is_none() {
        for result in leecher.get_search_results(query) {
            if result.username == "e2e_sharer" {
                for file in &result.files {
                    if file.name.contains("e2e_probe_xyzzy") {
                        hit = Some((file.name.clone(), file.size));
                    }
                }
            }
        }
        if hit.is_none() {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let (result_path, size) =
        hit.expect("leecher should find the sharer's file");
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

    let mut completed = false;
    let deadline = Instant::now() + Duration::from_secs(25);
    while Instant::now() < deadline {
        match status_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(DownloadStatus::Completed) => {
                completed = true;
                break;
            }
            Ok(DownloadStatus::Failed(_) | DownloadStatus::TimedOut) => break,
            _ => {}
        }
        if leecher
            .get_all_downloads()
            .iter()
            .any(|d| matches!(d.status, DownloadStatus::Completed))
        {
            completed = true;
            break;
        }
    }
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
            .any(|d| d.files.iter().any(|(name, _)| name == "late.mp3")),
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
            .any(|d| { d.files.iter().any(|(name, _)| name == "track.flac") }),
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
            .any(|d| d.files.iter().any(|(name, _)| name == "hidden.flac")),
        "the brokered listing should include the shared file"
    );

    let _ = std::fs::remove_dir_all(share_dir);
}
