//! End-to-end tests of the window itself against a real Soulseek server:
//! key presses in, a screen out, with soulfind and a real sharing peer on the
//! other end of the wire.
//!
//! The CLI suite proves the command surface; the unit tests under `ui` prove
//! the window's keys against a stand-in session. What neither covers is the
//! two together — a key that is supposed to put a search on the network and
//! bring its answers back onto the screen. The window is driven in-process
//! through the same handlers the terminal feeds, rendered onto a test
//! backend, and the session is a real client logged in to a real server.
//!
//! Like the rest of the e2e suite this needs a soulfind server and skips
//! without one; `SOULSEEK_E2E_REQUIRED=1` turns that skip into a failure.

mod common;

use common::{free_port, soulfind_binary};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use soulseek_rs::{Client, ClientSettings, PeerAddress};
use soulseek_rs_tui::MainTui;
use soulseek_rs_tui::models::{FocusedPane, SearchStatus};
use soulseek_rs_tui::persist::state::StateStore;
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a search collects for. Short, because every run here has to wait
/// it out; long enough for a peer on loopback to answer many times over.
const SEARCH_WINDOW: Duration = Duration::from_secs(3);

/// A soulfind spawned for one test, on a port nobody else holds.
struct Soulfind {
    child: Child,
    db: std::path::PathBuf,
    port: u16,
}

impl Soulfind {
    fn start() -> Option<Self> {
        let bin = soulfind_binary()?;
        let port = free_port()?;
        let db = std::env::temp_dir().join(format!("soulfind-tui-{port}.db"));
        let _ = std::fs::remove_file(&db);

        let mut child = Command::new(bin)
            .args(["-d", db.to_str()?, "-p", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(Self { child, db, port });
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        None
    }

    /// A client logged in to this server as `user`, sharing `shares`.
    ///
    /// The listener stays on: a peer delivers search results over a
    /// connection it opens back to the searcher, so a searcher without one
    /// hears nothing.
    fn client(&self, user: &str, shares: Vec<String>) -> Client {
        let mut client = Client::with_settings(ClientSettings {
            username: user.to_string(),
            password: "pw".to_string(),
            server_address: PeerAddress::new(
                "127.0.0.1".to_string(),
                self.port,
            ),
            enable_listen: true,
            listen_port: free_port().expect("peer port"),
            shared_directories: shares,
            version: soulseek_rs::ClientVersion::default(),
        });
        client.connect().expect("peer connect");
        assert!(client.login().expect("peer login"), "peer should log in");
        client
    }
}

impl Drop for Soulfind {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.db);
    }
}

macro_rules! server_or_skip {
    () => {
        match Soulfind::start() {
            Some(server) => server,
            None => {
                let required = std::env::var("SOULSEEK_E2E_REQUIRED")
                    .is_ok_and(|v| v != "0" && !v.is_empty());
                assert!(
                    !required,
                    "SOULSEEK_E2E_REQUIRED is set but no soulfind server could \
                     be started (set SOULFIND_BIN)"
                );
                println!(
                    "tui e2e skipped: no soulfind server (set SOULFIND_BIN to \
                     run)"
                );
                return;
            }
        }
    };
}

/// Wait out the SetWaitPort registrations so peer lookups resolve.
fn settle() {
    std::thread::sleep(Duration::from_secs(1));
}

/// The window under test: the real `MainTui` over a real session, drawn onto
/// a buffer instead of a terminal.
struct Window {
    tui: MainTui,
    terminal: Terminal<TestBackend>,
}

impl Window {
    /// Open the window as the terminal would on start-up, over `state` — the
    /// directory a previous run left its search list in.
    fn open(client: Arc<Client>, state: &Path, downloads: &Path) -> Self {
        let tui = MainTui::new(
            client,
            downloads.display().to_string(),
            SEARCH_WINDOW,
            Some(StateStore::new(state.to_path_buf())),
            None,
        );
        let terminal = Terminal::new(TestBackend::new(160, 40))
            .expect("a test backend always opens");
        Self { tui, terminal }
    }

    fn press(&mut self, code: KeyCode) {
        self.tui
            .handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// One turn of the event loop: catch up with the session, then draw.
    /// Returns the screen as the user would see it.
    fn frame(&mut self) -> String {
        self.tui.poll_session();
        self.terminal
            .draw(|frame| self.tui.render(frame))
            .expect("draw");
        self.terminal.backend().to_string()
    }

    /// Keep the window running until `done` says so, or `timeout` runs out.
    /// Returns the last screen either way, so a failing test shows what the
    /// user would have been looking at.
    fn run_until(
        &mut self,
        timeout: Duration,
        mut done: impl FnMut(&MainTui, &str) -> bool,
    ) -> (bool, String) {
        let deadline = Instant::now() + timeout;
        loop {
            let screen = self.frame();
            if done(&self.tui, &screen) {
                return (true, screen);
            }
            if Instant::now() > deadline {
                return (false, screen);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// File names in the Results pane right now.
    fn result_names(&self) -> Vec<String> {
        self.tui
            .state()
            .results_items
            .iter()
            .map(|file| file.filename.clone())
            .collect()
    }

    fn search_status(&self, index: usize) -> SearchStatus {
        self.tui.state().searches[index].status.clone()
    }
}

/// Whether the file the sharer offers has reached the Results pane, on
/// screen as well as in the state behind it.
fn shows_the_shared_file(tui: &MainTui, screen: &str) -> bool {
    tui.state()
        .results_items
        .iter()
        .any(|file| file.filename.contains("tui_probe_rerun"))
        && screen.contains("tui_probe_rerun")
}

#[test]
fn shift_s_runs_a_restored_query_again_and_the_answers_land_on_screen() {
    let server = server_or_skip!();

    // The other end of the wire: a peer sharing one file with a name only
    // this test searches for.
    let share = tempfile::tempdir().expect("share dir");
    std::fs::write(
        share.path().join("tui_probe_rerun_vexq.bin"),
        (0..4096u32).map(|i| (i % 251) as u8).collect::<Vec<u8>>(),
    )
    .expect("share file");
    let _sharer = server
        .client("tui_e2e_sharer", vec![share.path().display().to_string()]);
    settle();

    // Last time this user ran the window they searched for "vexq" and quit.
    // The list came back with them; the results did not.
    let state = tempfile::tempdir().expect("state dir");
    let downloads = tempfile::tempdir().expect("download dir");
    StateStore::new(state.path().to_path_buf())
        .save_search_queries(&["vexq".to_string()])
        .expect("a previous run's search list");

    let seeker = Arc::new(server.client("tui_e2e_seeker", Vec::new()));
    let mut window = Window::open(seeker, state.path(), downloads.path());

    let screen = window.frame();
    assert!(
        screen.contains("vexq"),
        "the old query is listed:\n{screen}"
    );
    assert_eq!(window.tui.state().searches.len(), 1);
    assert_eq!(window.search_status(0), SearchStatus::Completed);
    assert!(
        window.result_names().is_empty(),
        "and it has nothing under it"
    );
    assert_eq!(
        window.tui.state().focused_pane,
        FocusedPane::Searches,
        "the window opens on the list, so the query is already highlighted"
    );

    // One key, no typing.
    window.press(KeyCode::Char('S'));

    assert_eq!(window.tui.state().searches.len(), 1, "no second row");
    assert_eq!(window.search_status(0), SearchStatus::Active);
    assert_eq!(window.tui.state().focused_pane, FocusedPane::Results);

    let (found, screen) =
        window.run_until(Duration::from_secs(15), shows_the_shared_file);
    assert!(
        found,
        "the sharer's file should reach the screen:\n{screen}"
    );
    assert!(
        window
            .result_names()
            .iter()
            .all(|name| name.contains("tui_probe_rerun")),
        "only what the network answered: {:?}",
        window.result_names()
    );

    // The search runs its window out and settles, keeping what it found.
    let (done, screen) = window
        .run_until(SEARCH_WINDOW + Duration::from_secs(5), |tui, _| {
            tui.state().searches[0].status == SearchStatus::Completed
        });
    assert!(done, "the search should finish:\n{screen}");
    assert!(shows_the_shared_file(&window.tui, &screen));
    let first_run = window.result_names();

    // Again, now that the query has been on the wire once in this session:
    // the same peer has to answer the same searcher a second time, and the
    // window has to start the display over rather than stack the answers.
    window.press(KeyCode::Char('1'));
    assert_eq!(window.tui.state().focused_pane, FocusedPane::Searches);
    window.press(KeyCode::Char('S'));
    assert_eq!(window.search_status(0), SearchStatus::Active);
    assert!(
        window.result_names().is_empty(),
        "the display starts over: {:?}",
        window.result_names()
    );

    let (found, screen) =
        window.run_until(Duration::from_secs(15), shows_the_shared_file);
    assert!(found, "the second run should be answered too:\n{screen}");
    let (done, _) = window
        .run_until(SEARCH_WINDOW + Duration::from_secs(5), |tui, _| {
            tui.state().searches[0].status == SearchStatus::Completed
        });
    assert!(done);
    assert_eq!(
        window.result_names(),
        first_run,
        "the same answer, once, not the old set with the new one on top"
    );
    assert_eq!(window.tui.state().searches.len(), 1, "still one row");
}
