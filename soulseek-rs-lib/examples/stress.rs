//! Concurrency stress harness for the soulseek-rs client.
//!
//! Spins up a local soulfind, surrounds one real `Client` with a swarm of
//! mock peers, and drives searches, downloads, uploads and browses at the same
//! time. Prints a metrics table and a single 0-100 score, and compares that
//! score against a stored baseline so regressions are visible.
//!
//! Run it with `scripts/stress.sh` (sets SOULFIND_BIN and release mode).
//!
//! Knobs (all env vars, defaults in `Config::from_env`):
//!   STRESS_PEERS      mock seeders online, each answering searches
//!   STRESS_SEARCHES   searches fired at once
//!   STRESS_DOWNLOADS  downloads started at once
//!   STRESS_LEECHERS   mock peers pulling uploads from us at once
//!   STRESS_BROWSES    browses fired at once
//!   STRESS_UPLOAD_SLOTS  concurrent upload slots (default: every leecher)
//!   STRESS_FILE_KB    size of every transferred file
//!   STRESS_TIMEOUT    seconds before the run is called off
//!   STRESS_SAVE=1     store this run as the new baseline

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use soulseek_rs::message::Message;
use soulseek_rs::message::peer::{
    FileEntry, SharedDirectory, build_file_search_response,
    build_shared_file_list,
};
use soulseek_rs::message::server::MessageFactory;
use soulseek_rs::{Client, ClientSettings, ClientVersion, PeerAddress};

/// Files matching this tag are what every mock seeder answers searches for.
const TAG: &str = "zqxstress";

/// Aggregate transfer rate (MiB/s) that scores full marks. Set well above any
/// real network link: the point is to keep the client, not the wire, as the
/// limiting factor, so the score still moves when transfer handling regresses.
const TARGET_MBPS: f64 = 500.0;

const BASELINE: &str = "benchmarks/stress-baseline.json";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

struct Config {
    peers: usize,
    searches: usize,
    downloads: usize,
    leechers: usize,
    browses: usize,
    upload_slots: usize,
    file_bytes: usize,
    timeout: Duration,
    save: bool,
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

impl Config {
    fn from_env() -> Self {
        Self {
            peers: env_usize("STRESS_PEERS", 1024),
            searches: env_usize("STRESS_SEARCHES", 128),
            downloads: env_usize("STRESS_DOWNLOADS", 512),
            leechers: env_usize("STRESS_LEECHERS", 256),
            browses: env_usize("STRESS_BROWSES", 128),
            // Serve every leecher at once by default: this profile measures
            // concurrency, and the shipped slot cap would otherwise make it a
            // measurement of the queue. Lower it deliberately to stress that.
            upload_slots: env_usize(
                "STRESS_UPLOAD_SLOTS",
                env_usize("STRESS_LEECHERS", 256).max(1),
            ),
            file_bytes: env_usize("STRESS_FILE_KB", 4096) * 1024,
            timeout: Duration::from_secs(env_usize("STRESS_TIMEOUT", 60) as u64),
            save: std::env::var("STRESS_SAVE").is_ok_and(|v| v == "1"),
        }
    }
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Metrics {
    peers_online: AtomicU64,
    searches_issued: AtomicU64,
    searches_answered: AtomicU64,
    results_total: AtomicU64,
    downloads_issued: AtomicU64,
    downloads_completed: AtomicU64,
    bytes_downloaded: AtomicU64,
    uploads_requested: AtomicU64,
    uploads_completed: AtomicU64,
    bytes_uploaded: AtomicU64,
    browses_issued: AtomicU64,
    browses_answered: AtomicU64,
    peer_connect_failures: AtomicU64,
    /// Mock peers that never got online (server refused the login, no free
    /// port, ...). Counted separately so a harness-side limit shows up as a
    /// number instead of silently shrinking a denominator.
    mocks_offline: AtomicU64,
}

impl Metrics {
    fn bump(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
    fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }
}

fn ratio(done: u64, total: u64) -> f64 {
    if total == 0 {
        1.0
    } else {
        done as f64 / total as f64
    }
}

// ---------------------------------------------------------------------------
// soulfind
// ---------------------------------------------------------------------------

struct TestServer {
    host: String,
    port: u16,
    child: Option<Child>,
    db: Option<PathBuf>,
}

impl TestServer {
    fn spawn() -> Option<Self> {
        if let Ok(addr) = std::env::var("SOULSEEK_TEST_SERVER") {
            let (host, port) = addr.rsplit_once(':')?;
            let port = port.parse().ok()?;
            wait_until_listening(host, port, Duration::from_secs(2))?;
            return Some(Self {
                host: host.to_string(),
                port,
                child: None,
                db: None,
            });
        }

        let bin = PathBuf::from(std::env::var("SOULFIND_BIN").ok()?);
        let port = free_port()?;
        let db =
            std::env::temp_dir().join(format!("soulfind-stress-{port}.db"));
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
        })
    }

    fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(db) = self.db.take() {
            let _ = std::fs::remove_file(&db);
        }
    }
}

fn free_port() -> Option<u16> {
    TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()
        .map(|a| a.port())
}

fn wait_until_listening(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Option<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect((host, port)).is_ok() {
            return Some(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

// ---------------------------------------------------------------------------
// Raw peer protocol helpers
// ---------------------------------------------------------------------------

fn peer_init_bytes(username: &str, conn_type: &str, token: u32) -> Vec<u8> {
    let mut m = Message::new();
    m.write_int8(1)
        .write_string(username)
        .write_string(conn_type)
        .write_int32(token);
    m.get_buffer()
}

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
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// Log in to the server and drain up to the login response, leaving the user
/// online for as long as the returned stream lives.
fn login_raw(server_addr: &str, username: &str) -> std::io::Result<TcpStream> {
    let mut srv = connect_retry(server_addr, Duration::from_secs(10))?;
    srv.set_read_timeout(Some(Duration::from_secs(20)))?;
    srv.write_all(
        &MessageFactory::build_login_message(
            username,
            "pw",
            ClientVersion::default(),
        )
        .get_buffer(),
    )?;
    srv.flush()?;
    loop {
        let msg = read_framed(&mut srv)?;
        if msg.get_message_code() == 1 {
            return Ok(srv);
        }
    }
}

// ---------------------------------------------------------------------------
// Mock seeder: answers searches, serves browses, uploads files to the client
// ---------------------------------------------------------------------------

struct Seeder {
    name: String,
    server_addr: String,
    client_addr: String,
    files: Vec<(String, u64)>,
    content: Arc<Vec<u8>>,
    metrics: Arc<Metrics>,
    stop: Arc<AtomicBool>,
    login_delay: Duration,
}

impl Seeder {
    fn run(self) {
        // soulfind logs every login through SQLite, and a thousand of them
        // arriving in the same millisecond starves it badly enough that the
        // client under test cannot log in either. Real swarms do not arrive in
        // lockstep; spreading the herd keeps the server out of the measurement.
        std::thread::sleep(self.login_delay);
        let Ok(mut srv) = login_raw(&self.server_addr, &self.name) else {
            Metrics::bump(&self.metrics.mocks_offline);
            return;
        };
        let Some(port) = free_port() else {
            Metrics::bump(&self.metrics.mocks_offline);
            return;
        };
        let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) else {
            Metrics::bump(&self.metrics.mocks_offline);
            return;
        };
        if srv
            .write_all(
                &MessageFactory::build_set_wait_port_message(port).get_buffer(),
            )
            .is_err()
        {
            return;
        }
        let _ = srv.flush();
        Metrics::bump(&self.metrics.peers_online);

        // Inbound P connections (the client dialing us to queue a download or
        // browse our shares) each get their own handler thread, exactly as a
        // real peer would.
        let inbound = Arc::new(self.clone_state());
        let stop = self.stop.clone();
        std::thread::spawn(move || {
            let _ = listener.set_nonblocking(true);
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let state = inbound.clone();
                        std::thread::spawn(move || {
                            let _ = stream.set_nonblocking(false);
                            state.serve_connection(stream, true);
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });

        // Server connection: watch for relayed searches and answer the ones
        // that match our tag by dialling the searcher directly.
        let _ = srv.set_read_timeout(Some(Duration::from_millis(500)));
        let state = Arc::new(self.clone_state());
        while !self.stop.load(Ordering::Relaxed) {
            let Ok(mut msg) = read_framed(&mut srv) else {
                continue;
            };
            if msg.get_message_code() != 26 {
                continue;
            }
            msg.set_pointer(8);
            let _searcher = msg.read_string();
            let token = msg.read_int32();
            let query = msg.read_string();
            if !query.contains(TAG) {
                continue;
            }
            state.answer_search(token);
        }
    }

    fn clone_state(&self) -> SeederState {
        SeederState {
            name: self.name.clone(),
            client_addr: self.client_addr.clone(),
            files: self.files.clone(),
            content: self.content.clone(),
            metrics: self.metrics.clone(),
            stop: self.stop.clone(),
            conn: Mutex::new(None),
        }
    }
}

struct SeederState {
    name: String,
    client_addr: String,
    files: Vec<(String, u64)>,
    content: Arc<Vec<u8>>,
    metrics: Arc<Metrics>,
    stop: Arc<AtomicBool>,
    /// The one control connection we hold open to the client, as a real peer
    /// does. Dialling afresh per search response would be both unrealistic and
    /// a self-inflicted load: it makes the harness, not the client, the thing
    /// under strain.
    conn: Mutex<Option<TcpStream>>,
}

impl SeederState {
    /// Push a `FileSearchResponse` over our control connection, opening it on
    /// first use and reusing it thereafter.
    fn answer_search(self: &Arc<Self>, token: u32) {
        let entries: Vec<FileEntry> = self
            .files
            .iter()
            .map(|(name, size)| FileEntry {
                name,
                size: *size,
                attribs: &[],
            })
            .collect();
        let response =
            build_file_search_response(&self.name, token, &entries, 1, 0);

        let Ok(mut guard) = self.conn.lock() else {
            return;
        };
        if guard.is_none() {
            let Ok(mut p) =
                connect_retry(&self.client_addr, Duration::from_secs(10))
            else {
                Metrics::bump(&self.metrics.peer_connect_failures);
                return;
            };
            if p.write_all(&peer_init_bytes(&self.name, "P", 0)).is_err() {
                Metrics::bump(&self.metrics.peer_connect_failures);
                return;
            }
            // Read side runs on its own thread so browses and queued downloads
            // arriving on this connection are still served.
            let Ok(read_half) = p.try_clone() else {
                Metrics::bump(&self.metrics.peer_connect_failures);
                return;
            };
            let state = self.clone();
            std::thread::spawn(move || {
                state.serve_connection(read_half, false);
            });
            *guard = Some(p);
        }

        if let Some(p) = guard.as_mut()
            && p.write_all(&response.get_buffer()).is_err()
        {
            Metrics::bump(&self.metrics.peer_connect_failures);
            *guard = None;
        }
    }

    /// Write a reply on the connection `p` belongs to. When `p` is the read
    /// half of the shared control connection, the write half lives behind
    /// `conn` and every writer must take that lock — two threads writing the
    /// same socket would interleave and corrupt the framing.
    fn reply(&self, local: &mut TcpStream, shared: bool, bytes: &[u8]) -> bool {
        if !shared {
            return local.write_all(bytes).and_then(|()| local.flush()).is_ok();
        }
        let Ok(mut guard) = self.conn.lock() else {
            return false;
        };
        let Some(p) = guard.as_mut() else {
            return false;
        };
        p.write_all(bytes).and_then(|()| p.flush()).is_ok()
    }

    /// Handle one P connection for its lifetime: browses, queued downloads and
    /// the F-connection file transfers they lead to. `expect_init` marks a
    /// connection the client dialled (so it owns its own write half); anything
    /// else is our shared control connection.
    fn serve_connection(&self, mut p: TcpStream, expect_init: bool) {
        let shared = !expect_init;
        let _ = p.set_read_timeout(Some(Duration::from_millis(500)));
        if expect_init {
            // The client announces itself first; anything else is a stray dial.
            let Ok(init) = read_framed(&mut p) else {
                return;
            };
            if init.get_message_code() != 1 {
                return;
            }
        }

        while !self.stop.load(Ordering::Relaxed) {
            let mut msg = match read_framed(&mut p) {
                Ok(m) => m,
                Err(ref e) if is_timeout(e) => continue,
                Err(_) => return,
            };
            match msg.get_message_code() {
                // GetShareFileList: reply with everything we hold.
                4 => {
                    let dir = SharedDirectory {
                        name: format!("stress\\{}", self.name),
                        files: self.files.clone(),
                    };
                    let list =
                        build_shared_file_list(std::slice::from_ref(&dir));
                    if !self.reply(&mut p, shared, &list.get_buffer()) {
                        return;
                    }
                }
                // QueueUpload: offer the transfer, then stream it once allowed.
                43 => {
                    msg.set_pointer(8);
                    let requested = msg.read_string();
                    let Some((name, size)) =
                        self.files.iter().find(|(n, _)| *n == requested)
                    else {
                        continue;
                    };
                    let token = next_token();
                    let mut tr = Message::new();
                    tr.write_int32(40)
                        .write_int32(1)
                        .write_int32(token)
                        .write_string(name)
                        .write_int64(*size);
                    if !self.reply(&mut p, shared, &tr.get_buffer()) {
                        return;
                    }
                    if !self.wait_for_allow(&mut p, token) {
                        return;
                    }
                    let content = self.content.clone();
                    let addr = self.client_addr.clone();
                    let name = self.name.clone();
                    std::thread::spawn(move || {
                        let _ =
                            serve_file_over_f(&addr, &name, token, &content);
                    });
                }
                _ => {}
            }
        }
    }

    /// Wait for the client's `TransferResponse` allowing `token`.
    fn wait_for_allow(&self, p: &mut TcpStream, token: u32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline && !self.stop.load(Ordering::Relaxed) {
            match read_framed(p) {
                Ok(mut msg) if msg.get_message_code() == 41 => {
                    msg.set_pointer(8);
                    if msg.read_int32() == token {
                        return true;
                    }
                }
                Ok(_) => {}
                Err(ref e) if is_timeout(e) => {}
                Err(_) => return false,
            }
        }
        false
    }
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(0x1000);

fn next_token() -> u32 {
    NEXT_TOKEN.fetch_add(1, Ordering::Relaxed) as u32
}

/// Open an F connection to the downloader's listener and stream the file.
fn serve_file_over_f(
    downloader_addr: &str,
    username: &str,
    token: u32,
    content: &[u8],
) -> std::io::Result<()> {
    let mut f = connect_retry(downloader_addr, Duration::from_secs(10))?;
    f.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut init = peer_init_bytes(username, "F", token);
    init.extend_from_slice(&token.to_le_bytes());
    f.write_all(&init)?;
    f.flush()?;

    let mut start = [0u8; 8];
    f.read_exact(&mut start)?;
    let offset = u64::from_le_bytes(start) as usize;
    if offset < content.len() {
        f.write_all(&content[offset..])?;
    }
    f.flush()?;
    std::thread::sleep(Duration::from_millis(300));
    Ok(())
}

// ---------------------------------------------------------------------------
// Mock leecher: pulls an upload out of the client under test
// ---------------------------------------------------------------------------

struct Leecher {
    name: String,
    server_addr: String,
    client_addr: String,
    filename: String,
    metrics: Arc<Metrics>,
    /// The run's global deadline. Every blocking wait honours it so a stuck
    /// transfer costs the iteration nothing beyond the budget already spent.
    deadline: Instant,
    login_delay: Duration,
    /// Raised once the client is up. Leechers come online with the seeders so
    /// the server has really registered their listening port by the time they
    /// ask for a file — a real peer is online long before it queues anything.
    start: Arc<AtomicBool>,
}

impl Leecher {
    fn run(self) {
        std::thread::sleep(self.login_delay);
        Metrics::bump(&self.metrics.uploads_requested);
        let Ok(mut srv) = login_raw(&self.server_addr, &self.name) else {
            Metrics::bump(&self.metrics.mocks_offline);
            return;
        };
        let Some(port) = free_port() else {
            Metrics::bump(&self.metrics.mocks_offline);
            return;
        };
        let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) else {
            Metrics::bump(&self.metrics.mocks_offline);
            return;
        };
        // The client dials this port for the F leg, so the server must know it.
        if srv
            .write_all(
                &MessageFactory::build_set_wait_port_message(port).get_buffer(),
            )
            .is_err()
        {
            return;
        }
        let _ = srv.flush();

        while !self.start.load(Ordering::Relaxed) {
            if Instant::now() >= self.deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        if let Err(e) = self.pull(&listener) {
            eprintln!("[leecher {}] {e}", self.name);
        }
    }

    fn pull(&self, listener: &TcpListener) -> std::io::Result<()> {
        let mut p = connect_retry(&self.client_addr, Duration::from_secs(10))?;
        p.set_read_timeout(Some(Duration::from_secs(30)))?;
        p.write_all(&peer_init_bytes(&self.name, "P", 0))?;
        p.write_all(
            &MessageFactory::build_queue_upload_message(&self.filename)
                .get_buffer(),
        )?;
        p.flush()?;

        // The client offers the transfer; accept it so it starts streaming.
        let token = loop {
            if Instant::now() >= self.deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "client never offered the queued upload",
                ));
            }
            match read_framed(&mut p) {
                Ok(mut msg) if msg.get_message_code() == 40 => {
                    msg.set_pointer(8);
                    let _direction = msg.read_int32();
                    break msg.read_int32();
                }
                Ok(_) => {}
                Err(ref e) if is_timeout(e) => {}
                Err(e) => return Err(e),
            }
        };
        let mut ok = Message::new();
        ok.write_int32(41).write_int32(token).write_int8(1);
        p.write_all(&ok.get_buffer())?;
        p.flush()?;

        // Accept the client's inbound F connection and drain the file. The
        // client also dials our P port, so keep accepting until the PeerInit
        // announces "F" — otherwise we block reading a transfer token that a
        // control connection is never going to send.
        let mut f = accept_file_connection(listener, self.deadline)?;
        f.write_all(&0u64.to_le_bytes())?;
        f.flush()?;

        let mut sink = Vec::new();
        let received = f.read_to_end(&mut sink)? as u64;
        self.metrics
            .bytes_uploaded
            .fetch_add(received, Ordering::Relaxed);
        if received > 0 {
            Metrics::bump(&self.metrics.uploads_completed);
        }
        Ok(())
    }
}

/// Accept connections until one announces itself as an F (file) connection,
/// and return it positioned just past its PeerInit and transfer token.
fn accept_file_connection(
    listener: &TcpListener,
    deadline: Instant,
) -> std::io::Result<TcpStream> {
    listener.set_nonblocking(true)?;
    let mut held = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "no inbound F connection",
            ));
        }
        let Ok((mut stream, _)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        let Ok(mut init) = read_framed(&mut stream) else {
            continue;
        };
        if init.get_message_code() != 1 {
            continue;
        }
        init.set_pointer(5);
        let _username = init.read_string();
        if init.read_string() != "F" {
            // A control connection: hold it open so the client does not see a
            // reset, and keep waiting for the transfer.
            held.push(stream);
            continue;
        }
        let mut token_buf = [0u8; 4];
        stream.read_exact(&mut token_buf)?;
        drop(held);
        return Ok(stream);
    }
}

// ---------------------------------------------------------------------------
// Scoring and baseline
// ---------------------------------------------------------------------------

struct Score {
    total: f64,
    parts: Vec<(&'static str, f64, f64)>, // name, achieved 0-1, weight
}

fn compute_score(m: &Metrics, elapsed: Duration) -> Score {
    let bytes =
        Metrics::get(&m.bytes_downloaded) + Metrics::get(&m.bytes_uploaded);
    let mbps = (bytes as f64 / 1_048_576.0) / elapsed.as_secs_f64().max(0.001);

    let parts = vec![
        (
            "downloads",
            ratio(
                Metrics::get(&m.downloads_completed),
                Metrics::get(&m.downloads_issued),
            ),
            0.30,
        ),
        (
            "uploads",
            ratio(
                Metrics::get(&m.uploads_completed),
                Metrics::get(&m.uploads_requested),
            ),
            0.20,
        ),
        (
            "searches",
            ratio(
                Metrics::get(&m.searches_answered),
                Metrics::get(&m.searches_issued),
            ),
            0.20,
        ),
        (
            "browses",
            ratio(
                Metrics::get(&m.browses_answered),
                Metrics::get(&m.browses_issued),
            ),
            0.15,
        ),
        ("throughput", (mbps / TARGET_MBPS).min(1.0), 0.15),
    ];

    let total = 100.0 * parts.iter().map(|(_, v, w)| v * w).sum::<f64>();
    Score { total, parts }
}

fn read_baseline(path: &Path) -> Option<f64> {
    let text = std::fs::read_to_string(path).ok()?;
    let idx = text.find("\"score\"")?;
    text[idx + 7..]
        .trim_start()
        .trim_start_matches(':')
        .trim_start()
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .find(|s| !s.is_empty())?
        .parse()
        .ok()
}

fn write_baseline(path: &Path, score: &Score, cfg: &Config, m: &Metrics) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let parts = score
        .parts
        .iter()
        .map(|(n, v, _)| format!("    \"{n}\": {v:.4}"))
        .collect::<Vec<_>>()
        .join(",\n");
    let body = format!(
        "{{\n  \"score\": {:.2},\n  \"parts\": {{\n{}\n  }},\n  \"load\": {{\n    \
         \"peers\": {},\n    \"searches\": {},\n    \"downloads\": {},\n    \
         \"leechers\": {},\n    \"browses\": {},\n    \"file_bytes\": {}\n  }},\n  \
         \"observed\": {{\n    \"downloads_completed\": {},\n    \
         \"uploads_completed\": {},\n    \"bytes\": {}\n  }}\n}}\n",
        score.total,
        parts,
        cfg.peers,
        cfg.searches,
        cfg.downloads,
        cfg.leechers,
        cfg.browses,
        cfg.file_bytes,
        Metrics::get(&m.downloads_completed),
        Metrics::get(&m.uploads_completed),
        Metrics::get(&m.bytes_downloaded) + Metrics::get(&m.bytes_uploaded),
    );
    let _ = std::fs::write(path, body);
}

/// Live thread count for this process, so the harness can show whether load is
/// being absorbed or simply turned into threads.
fn thread_count() -> usize {
    Command::new("ps")
        .args(["-M", &std::process::id().to_string()])
        .output()
        .ok()
        .map_or(0, |o| String::from_utf8_lossy(&o.stdout).lines().count())
        .saturating_sub(1)
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

fn main() {
    let cfg = Config::from_env();
    let Some(server) = TestServer::spawn() else {
        eprintln!(
            "stress: no soulfind server. Set SOULFIND_BIN (or SOULSEEK_TEST_SERVER)."
        );
        std::process::exit(2);
    };
    let server_addr = server.addr();
    let metrics = Arc::new(Metrics::default());
    let stop = Arc::new(AtomicBool::new(false));

    // Shares the client under test offers to mock leechers.
    let run_id = std::process::id();
    let share_dir =
        std::env::temp_dir().join(format!("soulseek-stress-share-{run_id}"));
    let download_dir =
        std::env::temp_dir().join(format!("soulseek-stress-dl-{run_id}"));
    let _ = std::fs::remove_dir_all(&share_dir);
    let _ = std::fs::remove_dir_all(&download_dir);
    std::fs::create_dir_all(&share_dir).expect("share dir");
    std::fs::create_dir_all(&download_dir).expect("download dir");

    let payload: Vec<u8> =
        (0..cfg.file_bytes).map(|i| (i % 251) as u8).collect();
    // Peers ask for the backslash-separated virtual path (root dir name first),
    // not the basename — anything else is silently unknown to the share index.
    let share_root = share_dir
        .file_name()
        .expect("share dir name")
        .to_string_lossy()
        .into_owned();
    let shared_names: Vec<String> = (0..cfg.leechers.max(1))
        .map(|i| format!("{share_root}\\{TAG}_own_{i}.bin"))
        .collect();
    for name in &shared_names {
        let basename = name.rsplit('\\').next().expect("basename");
        std::fs::write(share_dir.join(basename), &payload)
            .expect("write share");
    }

    let client_port = free_port().expect("client port");
    let client_name = format!("stress_client_{run_id}");
    let client_addr = format!("127.0.0.1:{client_port}");

    println!(
        "stress: {} peers, {} searches, {} downloads, {} leechers, {} browses, \
         {} KiB files",
        cfg.peers,
        cfg.searches,
        cfg.downloads,
        cfg.leechers,
        cfg.browses,
        cfg.file_bytes / 1024
    );

    // --- mock seeders -----------------------------------------------------
    let content = Arc::new(payload);
    let mut seeder_names = Vec::new();
    for i in 0..cfg.peers {
        let name = format!("stress_seed_{run_id}_{i}");
        seeder_names.push(name.clone());
        let files =
            vec![(format!("share\\{TAG}_{i}.bin"), cfg.file_bytes as u64)];
        let seeder = Seeder {
            name,
            server_addr: server_addr.clone(),
            client_addr: client_addr.clone(),
            files,
            content: content.clone(),
            metrics: metrics.clone(),
            stop: stop.clone(),
            login_delay: Duration::from_micros(1500 * i as u64),
        };
        std::thread::spawn(move || seeder.run());
    }

    // --- mock leechers, brought online alongside the seeders ---------------
    // They wait on `start` before queueing anything, so the server has really
    // registered their listening port by the time we ask it for their address.
    let start = Arc::new(AtomicBool::new(false));
    let run_deadline_hint =
        Instant::now() + cfg.timeout + Duration::from_secs(40);
    let mut leechers = Vec::new();
    for i in 0..cfg.leechers {
        let leecher = Leecher {
            name: format!("stress_leech_{run_id}_{i}"),
            server_addr: server_addr.clone(),
            client_addr: client_addr.clone(),
            filename: shared_names[i % shared_names.len()].clone(),
            metrics: metrics.clone(),
            deadline: run_deadline_hint,
            login_delay: Duration::from_micros(1500 * i as u64),
            start: start.clone(),
        };
        leechers.push(std::thread::spawn(move || leecher.run()));
    }

    // Let the swarm register its wait ports before the client starts.
    let deadline = Instant::now() + Duration::from_secs(30);
    while Metrics::get(&metrics.peers_online) < cfg.peers as u64
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    println!(
        "stress: {}/{} seeders online",
        Metrics::get(&metrics.peers_online),
        cfg.peers
    );

    // --- client under test ------------------------------------------------
    let mut client = Client::with_settings(ClientSettings {
        username: client_name,
        password: "pw".to_string(),
        server_address: PeerAddress::new(server.host.clone(), server.port),
        enable_listen: true,
        listen_port: client_port,
        shared_directories: vec![share_dir.display().to_string()],
        version: ClientVersion::default(),
    });
    client.connect().expect("client connect");
    assert!(client.login().expect("client login"), "client login failed");
    client.set_upload_slots(cfg.upload_slots);
    std::thread::sleep(Duration::from_secs(1));

    let client = Arc::new(client);
    let started = Instant::now();
    let run_deadline = started + cfg.timeout;
    let peak_threads = Arc::new(AtomicU64::new(0));

    // Sample thread count while the run is in flight.
    {
        let peak = peak_threads.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let n = thread_count() as u64;
                peak.fetch_max(n, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(200));
            }
        });
    }

    let mut workers = Vec::new();

    // --- concurrent searches ---------------------------------------------
    let queries: Vec<String> =
        (0..cfg.searches).map(|k| format!("{TAG}_q{k}")).collect();
    for query in queries.clone() {
        let client = client.clone();
        let metrics = metrics.clone();
        workers.push(std::thread::spawn(move || {
            Metrics::bump(&metrics.searches_issued);
            let _ = client.search(&query, Duration::from_secs(3));
        }));
    }

    // --- concurrent uploads: release the leechers already standing by -----
    start.store(true, Ordering::Relaxed);
    workers.extend(leechers);

    // --- concurrent browses ----------------------------------------------
    for name in seeder_names.iter().take(cfg.browses) {
        let client = client.clone();
        let metrics = metrics.clone();
        let name = name.clone();
        workers.push(std::thread::spawn(move || {
            Metrics::bump(&metrics.browses_issued);
            if client.browse_user(&name).is_err() {
                return;
            }
            let deadline = run_deadline;
            while Instant::now() < deadline {
                if client.take_browse_result(&name).is_some() {
                    Metrics::bump(&metrics.browses_answered);
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }));
    }

    // --- concurrent downloads --------------------------------------------
    // Collect distinct (peer, file) hits from the searches, then fire every
    // download at once — that simultaneity is the point of the exercise.
    let hit_deadline = started + cfg.timeout / 3;
    let mut hits: HashMap<String, (String, u64)> = HashMap::new();
    let mut answered = 0;
    let mut results_seen = 0u64;
    // Keep collecting until there is enough to download *and* every search has
    // been answered, so search recall is measured even on download-free runs.
    while Instant::now() < hit_deadline
        && (hits.len() < cfg.downloads || answered < queries.len())
    {
        answered = 0;
        results_seen = 0;
        for query in &queries {
            let results = client.get_search_results(query);
            if !results.is_empty() {
                answered += 1;
            }
            for result in results {
                results_seen += result.files.len() as u64;
                if let Some(file) = result.files.first() {
                    hits.entry(result.username.clone())
                        .or_insert_with(|| (file.name.clone(), file.size));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    metrics
        .searches_answered
        .store(answered as u64, Ordering::Relaxed);
    metrics.results_total.store(results_seen, Ordering::Relaxed);
    println!(
        "stress: {} distinct peers answered searches, firing {} downloads",
        hits.len(),
        hits.len().min(cfg.downloads)
    );

    let completed = Arc::new(Mutex::new(Vec::new()));
    for (peer, (filename, size)) in hits.into_iter().take(cfg.downloads) {
        let client = client.clone();
        let metrics = metrics.clone();
        let dir = download_dir.display().to_string();
        let completed = completed.clone();
        workers.push(std::thread::spawn(move || {
            Metrics::bump(&metrics.downloads_issued);
            let Ok((_download, status_rx)) =
                client.download(filename.clone(), peer.clone(), size, dir)
            else {
                return;
            };
            let deadline = run_deadline;
            while Instant::now() < deadline {
                match status_rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(soulseek_rs::DownloadStatus::Completed) => {
                        Metrics::bump(&metrics.downloads_completed);
                        metrics
                            .bytes_downloaded
                            .fetch_add(size, Ordering::Relaxed);
                        if let Ok(mut c) = completed.lock() {
                            c.push(peer);
                        }
                        return;
                    }
                    Ok(
                        soulseek_rs::DownloadStatus::Failed(_)
                        | soulseek_rs::DownloadStatus::TimedOut,
                    ) => return,
                    _ => {}
                }
            }
        }));
    }

    // --- wait for the run to settle --------------------------------------
    for worker in workers {
        let remaining = run_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let _ = worker.join();
    }
    let elapsed = started.elapsed();
    stop.store(true, Ordering::Relaxed);

    // --- report -----------------------------------------------------------
    let score = compute_score(&metrics, elapsed);
    let bytes = Metrics::get(&metrics.bytes_downloaded)
        + Metrics::get(&metrics.bytes_uploaded);

    println!("\n--- results ({:.1}s) ---", elapsed.as_secs_f64());
    println!(
        "  seeders online       {}/{}",
        Metrics::get(&metrics.peers_online),
        cfg.peers
    );
    println!(
        "  searches answered    {}/{}",
        Metrics::get(&metrics.searches_answered),
        Metrics::get(&metrics.searches_issued)
    );
    println!(
        "  search results       {}",
        Metrics::get(&metrics.results_total)
    );
    println!(
        "  downloads completed  {}/{}",
        Metrics::get(&metrics.downloads_completed),
        Metrics::get(&metrics.downloads_issued)
    );
    println!(
        "  uploads completed    {}/{}",
        Metrics::get(&metrics.uploads_completed),
        Metrics::get(&metrics.uploads_requested)
    );
    println!(
        "  browses answered     {}/{}",
        Metrics::get(&metrics.browses_answered),
        Metrics::get(&metrics.browses_issued)
    );
    println!(
        "  peer dial failures   {}",
        Metrics::get(&metrics.peer_connect_failures)
    );
    println!(
        "  mocks never online   {}  (harness-side, not the client)",
        Metrics::get(&metrics.mocks_offline)
    );
    println!(
        "  transferred          {:.1} MiB ({:.1} MiB/s)",
        bytes as f64 / 1_048_576.0,
        bytes as f64 / 1_048_576.0 / elapsed.as_secs_f64().max(0.001)
    );
    println!(
        "  peak threads         {} (client + mock swarm, same process)",
        peak_threads.load(Ordering::Relaxed)
    );

    // Where the stragglers got stuck is the whole diagnostic: a download parked
    // in Queued never got its transfer negotiated, while one in InProgress is
    // simply slow.
    let mut by_status: HashMap<&'static str, usize> = HashMap::new();
    for download in client.get_all_downloads() {
        let label = match download.status {
            soulseek_rs::DownloadStatus::Queued => "Queued",
            soulseek_rs::DownloadStatus::InProgress { .. } => "InProgress",
            soulseek_rs::DownloadStatus::Completed => "Completed",
            soulseek_rs::DownloadStatus::Failed(_) => "Failed",
            soulseek_rs::DownloadStatus::TimedOut => "TimedOut",
            soulseek_rs::DownloadStatus::Paused { .. } => "Paused",
        };
        *by_status.entry(label).or_default() += 1;
    }
    if !by_status.is_empty() {
        let mut parts: Vec<String> =
            by_status.iter().map(|(k, v)| format!("{k}={v}")).collect();
        parts.sort();
        println!("  download states      {}", parts.join(" "));
    }
    let stuck: Vec<String> = client
        .get_all_downloads()
        .iter()
        .filter(|d| matches!(d.status, soulseek_rs::DownloadStatus::Queued))
        .map(|d| d.username.clone())
        .collect();
    if !stuck.is_empty() {
        println!("  stuck peers          {}", stuck.join(" "));
    }

    println!("\n--- score ---");
    for (name, value, weight) in &score.parts {
        println!("  {name:<12} {:>6.1}%  x{weight:.2}", value * 100.0);
    }
    println!("  TOTAL        {:.2}/100", score.total);

    let baseline_path = PathBuf::from(BASELINE);
    if let Some(prev) = read_baseline(&baseline_path) {
        let delta = score.total - prev;
        let sign = if delta >= 0.0 { "+" } else { "" };
        println!("  baseline     {prev:.2}  ({sign}{delta:.2})");
        if cfg.save && score.total > prev {
            write_baseline(&baseline_path, &score, &cfg, &metrics);
            println!("  baseline updated to {:.2}", score.total);
        } else if cfg.save {
            println!("  baseline kept (run did not beat it)");
        }
    } else {
        write_baseline(&baseline_path, &score, &cfg, &metrics);
        println!("  baseline created at {:.2}", score.total);
    }

    let _ = std::fs::remove_dir_all(&share_dir);
    let _ = std::fs::remove_dir_all(&download_dir);
}
