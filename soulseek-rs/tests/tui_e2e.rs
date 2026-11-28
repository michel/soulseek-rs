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

use common::{CLI_ENV_VARS, Soulfind, free_port, settle};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use soulseek_rs::{Client, DownloadStatus, UploadStatus};
use soulseek_rs_tui::MainTui;
use soulseek_rs_tui::api::SessionApi;
use soulseek_rs_tui::models::SearchStatus;
use soulseek_rs_tui::persist::state::StateStore;
use soulseek_rs_tui::remote::{Endpoint, RemoteSession};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// How long a search collects for. Short, because every run here has to wait
/// it out; long enough for a peer on loopback to answer many times over.
const SEARCH_WINDOW: Duration = Duration::from_secs(3);

macro_rules! server_or_skip {
    () => {
        match Soulfind::start() {
            Some(server) => server,
            None => {
                common::no_server("tui e2e");
                return;
            }
        }
    };
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
    fn open(
        client: Arc<dyn SessionApi>,
        state: &Path,
        downloads: &Path,
    ) -> Self {
        let tui = MainTui::new(
            client,
            downloads.display().to_string(),
            false,
            false,
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

    /// Keep the window running until `done` says so, and hand back the
    /// screen it stopped on. Past `timeout` the test fails showing the last
    /// screen, what the user would have been looking at, under `what`.
    fn run_until(
        &mut self,
        timeout: Duration,
        what: &str,
        mut done: impl FnMut(&MainTui, &str) -> bool,
    ) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let screen = self.frame();
            if done(&self.tui, &screen) {
                return screen;
            }
            assert!(Instant::now() <= deadline, "{what}:\n{screen}");
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

/// Whether the one search has run its window out.
fn finished(tui: &MainTui, _: &str) -> bool {
    tui.state().searches[0].status == SearchStatus::Completed
}

/// The window's answer to "am I actually sharing that?", against a real
/// scan of a real folder rather than a stand-in listing.
#[test]
fn shift_b_shows_the_scanned_index_the_way_a_peer_would_get_it() {
    let server = server_or_skip!();

    let share = tempfile::tempdir().expect("share dir");
    std::fs::create_dir_all(share.path().join("album")).expect("subfolder");
    std::fs::write(
        share.path().join("album").join("tui_probe_own.bin"),
        [3u8; 32],
    )
    .expect("share file");

    let state = tempfile::tempdir().expect("state dir");
    let downloads = tempfile::tempdir().expect("download dir");
    let me = Arc::new(server.client(
        "tui_e2e_own_shares",
        vec![share.path().display().to_string()],
    ));
    let mut window = Window::open(me, state.path(), downloads.path());

    window.press(KeyCode::Char('B'));
    window.press(KeyCode::Char('L'));
    let screen = window.frame();
    assert!(screen.contains("My shares"), "{screen}");
    assert!(
        screen.contains("album") && screen.contains("tui_probe_own.bin"),
        "the scanned tree, folder by folder:\n{screen}"
    );

    window.press(KeyCode::Esc);
    assert!(!window.tui.state().show_browse, "Esc hides the popup");
}

#[test]
fn shift_s_runs_a_restored_query_again_and_the_answers_land_on_screen() {
    let server = server_or_skip!();

    // The other end of the wire: a peer sharing one file with a name only
    // this test searches for.
    let share = tempfile::tempdir().expect("share dir");
    std::fs::write(share.path().join("tui_probe_rerun_vexq.bin"), [7u8; 64])
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

    // One key, no typing: the window opens on the list with the query
    // highlighted.
    window.press(KeyCode::Char('S'));
    window.run_until(
        Duration::from_secs(15),
        "the sharer's file should reach the screen",
        shows_the_shared_file,
    );
    // The search runs its window out and settles, keeping what it found.
    let screen = window.run_until(
        SEARCH_WINDOW + Duration::from_secs(5),
        "the search should finish",
        finished,
    );
    assert!(shows_the_shared_file(&window.tui, &screen));
    let first_run = window.result_names();

    // Again, now that the query has been on the wire once in this session:
    // the same peer has to answer the same searcher a second time, and the
    // window has to start the display over rather than stack the answers.
    window.press(KeyCode::Char('1'));
    window.press(KeyCode::Char('S'));
    window.run_until(
        Duration::from_secs(15),
        "the second run should be answered too",
        shows_the_shared_file,
    );
    window.run_until(
        SEARCH_WINDOW + Duration::from_secs(5),
        "the second run should finish",
        finished,
    );
    assert_eq!(
        window.result_names(),
        first_run,
        "the same answer, once, not the old set with the new one on top"
    );
    assert_eq!(window.tui.state().searches.len(), 1, "still one row");
}

/// Short enough that the Downloads pane shows the whole name in its row.
const SERVED: &str = "up_qzvw.bin";

fn share_one_file() -> tempfile::TempDir {
    let share = tempfile::tempdir().expect("share dir");
    std::fs::write(share.path().join(SERVED), [5u8; 2048]).expect("share file");
    share
}

/// `leecher` downloads the one file `sharer` offers. The transfer runs for as
/// long as the returned channel lives.
fn fetch(
    leecher: &Client,
    sharer: &str,
    into: &Path,
) -> Receiver<DownloadStatus> {
    let file = leecher
        .search("qzvw", SEARCH_WINDOW)
        .expect("search")
        .into_iter()
        .filter(|reply| reply.username == sharer)
        .flat_map(|reply| reply.files)
        .find(|file| file.name.contains(SERVED))
        .expect("the leecher should find the served file");
    leecher
        .download(
            file.name,
            sharer.to_string(),
            file.size,
            into.display().to_string(),
        )
        .expect("start download")
        .1
}

fn shows_a_finished_upload(tui: &MainTui, screen: &str) -> bool {
    tui.state()
        .uploads
        .iter()
        .any(|upload| upload.status == UploadStatus::Completed)
        && screen.contains(SERVED)
}

fn c_clears_the_finished_upload(window: &mut Window) {
    window.press(KeyCode::Char('3'));
    window.run_until(
        Duration::from_secs(20),
        "the finished upload should be listed",
        shows_a_finished_upload,
    );

    window.press(KeyCode::Char('c'));
    for _ in 0..20 {
        let screen = window.frame();
        assert!(
            window.tui.state().uploads.is_empty() && !screen.contains(SERVED),
            "the cleared upload should stay gone:\n{screen}"
        );
    }
}

#[test]
fn c_clears_a_finished_upload_for_good() {
    let server = server_or_skip!();
    let share = share_one_file();
    let state = tempfile::tempdir().expect("state dir");
    let downloads = tempfile::tempdir().expect("download dir");
    let me =
        Arc::new(server.client(
            "tui_e2e_uploader",
            vec![share.path().display().to_string()],
        ));
    let mut window = Window::open(me, state.path(), downloads.path());

    let leecher = server.client("tui_e2e_leecher", Vec::new());
    settle();
    let _transfer = fetch(&leecher, "tui_e2e_uploader", downloads.path());

    c_clears_the_finished_upload(&mut window);
}

/// A `soulseek-rs daemon` process, killed when dropped.
struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start a daemon logged in as `user` and sharing `share`, and attach to it.
fn attach_to_daemon(
    server: &Soulfind,
    user: &str,
    share: &Path,
    state: &Path,
) -> (Daemon, RemoteSession) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_soulseek-rs"));
    for name in CLI_ENV_VARS {
        command.env_remove(name);
    }
    let listener_port = free_port().expect("listener port").to_string();
    let daemon = Daemon(
        command
            .env("SOULSEEK_STATE_DIR", state)
            .current_dir(state)
            .args(["--no-config", "--server", &server.address()])
            .args(["--username", user, "--password", "pw"])
            .args(["--listener-port", &listener_port])
            .arg("--shared-dir")
            .arg(share)
            .arg("--download-dir")
            .arg(state.join("downloads"))
            .arg("daemon")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the daemon should start"),
    );
    let socket = Endpoint::Socket(state.join("daemon.sock"));
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Ok(session) = RemoteSession::connect(&socket, None) {
            return (daemon, session);
        }
        assert!(Instant::now() <= deadline, "the daemon never answered");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn c_clears_a_finished_upload_the_daemon_served_for_good() {
    let server = server_or_skip!();
    let share = share_one_file();
    let daemon_state = tempfile::tempdir().expect("daemon state dir");
    let state = tempfile::tempdir().expect("state dir");
    let downloads = tempfile::tempdir().expect("download dir");
    let (_daemon, session) = attach_to_daemon(
        &server,
        "tui_e2e_daemon",
        share.path(),
        daemon_state.path(),
    );
    let mut window =
        Window::open(Arc::new(session), state.path(), downloads.path());

    let leecher = server.client("tui_e2e_daemon_leecher", Vec::new());
    settle();
    let _transfer = fetch(&leecher, "tui_e2e_daemon", downloads.path());

    c_clears_the_finished_upload(&mut window);
}

/// The tests here run in parallel, each with clients whose listeners land on
/// whatever port is free, so one of them can take the port a soulfind was
/// handed before that soulfind binds it.
#[test]
fn a_soulfind_whose_port_a_client_listener_holds_is_not_reported_up() {
    let mut server = server_or_skip!();
    server.stop();
    let _squatter = std::net::TcpListener::bind(("0.0.0.0", server.port()))
        .expect("the port soulfind let go of");

    assert!(
        server.restart().is_none(),
        "soulfind cannot bind a port another listener holds, so nobody can \
         log in there"
    );
}
