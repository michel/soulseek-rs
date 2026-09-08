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

use common::{Soulfind, settle};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use soulseek_rs::Client;
use soulseek_rs_tui::MainTui;
use soulseek_rs_tui::models::{FocusedPane, SearchStatus};
use soulseek_rs_tui::persist::state::StateStore;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a search collects for. Short, because every run here has to wait
/// it out; long enough for a peer on loopback to answer many times over.
const SEARCH_WINDOW: Duration = Duration::from_secs(3);

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

/// Whether the one search has run its window out.
fn finished(tui: &MainTui, _: &str) -> bool {
    tui.state().searches[0].status == SearchStatus::Completed
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
    // The search runs its window out and settles, keeping what it found.
    let (done, screen) =
        window.run_until(SEARCH_WINDOW + Duration::from_secs(5), finished);
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
    let (done, _) =
        window.run_until(SEARCH_WINDOW + Duration::from_secs(5), finished);
    assert!(done);
    assert_eq!(
        window.result_names(),
        first_run,
        "the same answer, once, not the old set with the new one on top"
    );
    assert_eq!(window.tui.state().searches.len(), 1, "still one row");
}
