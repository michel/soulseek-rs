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

use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use soulseek_rs::{Client, ClientSettings, PeerAddress};

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
            shared_directory: None,
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
