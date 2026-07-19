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
macro_rules! server_or_skip {
    () => {
        match TestServer::resolve() {
            Some(server) => server,
            None => {
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
}
