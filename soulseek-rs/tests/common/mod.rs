//! Scaffolding both CLI end-to-end binaries need: the environment they scrub,
//! the soulfind they run against, and a port nobody else holds.

#![allow(dead_code)]

use soulseek_rs::message::server::MessageFactory;
use soulseek_rs::{Client, ClientSettings, PeerAddress};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Every environment variable the binary reads, cleared for each run so a
/// developer's shell (or a stray `.env`) cannot change what a test observes —
/// and so no run finds a real daemon.
pub const CLI_ENV_VARS: [&str; 15] = [
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

pub fn soulfind_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SOULFIND_BIN") {
        let path = PathBuf::from(path);
        return path.exists().then_some(path);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|dir| dir.join("soulfind/bin/soulfind"))
        .find(|candidate| candidate.exists())
}

pub fn free_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()
        .map(|addr| addr.port())
}

/// A soulfind spawned for one test, on a port nobody else holds, with a
/// throwaway database that goes with it.
pub struct Soulfind {
    child: Child,
    db: PathBuf,
    port: u16,
}

impl Soulfind {
    /// soulfind exits if its port is taken between `free_port()` handing it
    /// out and soulfind binding it, so a start that fails gets a fresh port.
    pub fn start() -> Option<Self> {
        (0..3).find_map(|_| {
            let port = free_port()?;
            let db = std::env::temp_dir().join(format!("soulfind-{port}.db"));
            let _ = std::fs::remove_file(&db);
            let child = Self::spawn(port, &db)?;
            Some(Self { child, db, port })
        })
    }

    fn spawn(port: u16, db: &Path) -> Option<Child> {
        let mut child = Command::new(soulfind_binary()?)
            .args(["-d", db.to_str()?, "-p", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            // A child that has exited is never taken as up: soulfind exits
            // when its port is gone, and whoever took it may answer instead.
            if child.try_wait().ok().flatten().is_some() {
                return None;
            }
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(child);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        None
    }

    pub const fn port(&self) -> u16 {
        self.port
    }

    pub fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Bring it back on the same port and database.
    pub fn restart(&mut self) -> Option<()> {
        self.stop();
        self.child = Self::spawn(self.port, &self.db)?;
        Some(())
    }

    /// A client logged in to this server as `user`, sharing `shares`.
    pub fn client(&self, user: &str, shares: Vec<String>) -> Client {
        login("127.0.0.1", self.port, user, shares)
    }
}

impl Drop for Soulfind {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_file(&self.db);
    }
}

/// An in-process client logged in to the server at `host:port` as `user`,
/// sharing `shares`: the other end of a test.
///
/// The listener stays on: a peer delivers search results and transfers over
/// a connection it opens back to us, so a client without one hears nothing.
pub fn login(host: &str, port: u16, user: &str, shares: Vec<String>) -> Client {
    let mut client = Client::with_settings(ClientSettings {
        username: user.to_string(),
        password: "pw".to_string(),
        server_address: PeerAddress::new(host.to_string(), port),
        enable_listen: true,
        listen_port: free_port().expect("peer port"),
        shared_directories: shares,
        accept_children: false,
        version: soulseek_rs::ClientVersion::default(),
    });
    client.connect().expect("peer connect");
    assert!(client.login().expect("peer login"), "peer should log in");
    client
}

/// A user who is online and reachable but never answers, held for as long as
/// the value lives.
///
/// Their listener accepts a connection and then says nothing, so a download
/// from them waits in the queue the way one waits behind a busy peer, with no
/// timer to race. A user who does not exist cannot stand in for this: the
/// server refuses to broker a connection to them, and their download fails
/// before a test gets to act on it.
pub struct SilentPeer {
    _session: TcpStream,
    _listener: TcpListener,
}

pub fn silent_peer(host: &str, port: u16, user: &str) -> SilentPeer {
    // Not loopback: soulfind hands a loopback login's address out as the
    // host's LAN address, where a 127.0.0.1 listener is never reached.
    let listener = TcpListener::bind("0.0.0.0:0").expect("silent listener");
    let wait_port = listener.local_addr().expect("listener address").port();
    let mut session = TcpStream::connect((host, port)).expect("server");
    let login = MessageFactory::build_login_message(
        user,
        "pw",
        soulseek_rs::ClientVersion::default(),
    );
    session
        .write_all(&login.get_buffer())
        .expect("silent login");
    // soulfind drops everything but a login until the login has landed, so a
    // port announced any sooner is announced to nobody.
    session
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    loop {
        let mut len = [0; 4];
        session.read_exact(&mut len).expect("login reply");
        let mut body = vec![0; u32::from_le_bytes(len) as usize];
        session.read_exact(&mut body).expect("login reply");
        if body[..4] == 1u32.to_le_bytes() {
            assert_eq!(body[4], 1, "{user} should log in");
            break;
        }
    }
    let wait = MessageFactory::build_set_wait_port_message(wait_port);
    session.write_all(&wait.get_buffer()).expect("wait port");
    settle();
    SilentPeer {
        _session: session,
        _listener: listener,
    }
}

/// Wait out the SetWaitPort registrations so peer lookups resolve.
pub fn settle() {
    std::thread::sleep(Duration::from_secs(1));
}

/// What a suite does without a server: fail when the run said one is
/// required, otherwise say it skipped. The caller returns afterwards.
pub fn no_server(suite: &str) {
    let required = std::env::var("SOULSEEK_E2E_REQUIRED")
        .is_ok_and(|v| v != "0" && !v.is_empty());
    assert!(
        !required,
        "SOULSEEK_E2E_REQUIRED is set but no soulfind server could be \
         started for the {suite} suite (set SOULFIND_BIN)"
    );
    println!("{suite} skipped: no soulfind server (set SOULFIND_BIN to run)");
}
