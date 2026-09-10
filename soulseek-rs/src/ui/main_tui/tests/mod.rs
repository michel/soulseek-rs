//! The window driven the way a terminal drives it: keys and mouse events in,
//! a rendered screen out, over a stand-in session.
//!
//! This file is the harness the topic modules below share — the session
//! double, the window constructors, and the press/screen helpers.

mod browse;
mod layout;
mod navigation;
mod rooms;
mod searches;
mod session;
mod settings;
mod transfers;

use super::*;

use crate::daemon::proto::ChatMessageDto;

use crate::models::{FocusedPane, MessageDirection, RoomsView, SearchStatus};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A stand-in for a daemon: it already holds a conversation, a transfer
/// queue, and a search cache, none of which this window created.
#[derive(Default)]
struct TalkativeSession {
    history: Vec<ChatMessageDto>,
    downloads: Vec<soulseek_rs::types::Download>,
    searches: std::sync::Mutex<Vec<crate::api::SessionSearch>>,
    /// What `try_get_search_results` hands back to the window.
    results: std::sync::Mutex<Vec<soulseek_rs::SearchResult>>,
    /// Every query a window put on the wire, in order.
    searched: std::sync::Mutex<Vec<String>>,
    /// How many times a window asked for the search list.
    all_searches_calls: std::sync::atomic::AtomicUsize,
    /// How many times a window pulled a full result set across. Against a
    /// daemon each pull is the whole set over the socket, so the tests
    /// count them.
    result_fetches: std::sync::atomic::AtomicUsize,
    /// Whether this session is a daemon's. Local sessions do not sync.
    shared: bool,
    /// What `set_download_directory` was last asked for, if anything.
    download_dir_set: std::sync::Mutex<Option<String>>,
    /// What `set_shared_directories` was last asked for, if anything.
    shares_set: std::sync::Mutex<Option<Vec<String>>>,
    /// What `change_password` was last asked for, if anything.
    password_set: std::sync::Mutex<Option<String>>,
    /// The share index this session serves to peers.
    listing: std::sync::Mutex<Vec<soulseek_rs::SharedDirectory>>,
    download_cancelled: std::sync::Mutex<Vec<(String, String)>>,
    upload_cancelled: std::sync::Mutex<Vec<(String, String)>>,
    download_removed: std::sync::Mutex<Vec<(String, String)>>,
}

fn queued(username: &str, filename: &str) -> soulseek_rs::types::Download {
    at_status(username, filename, soulseek_rs::DownloadStatus::Queued)
}

fn completed(username: &str, filename: &str) -> soulseek_rs::types::Download {
    at_status(username, filename, soulseek_rs::DownloadStatus::Completed)
}

fn uploading(username: &str, filename: &str) -> soulseek_rs::types::UploadInfo {
    soulseek_rs::types::UploadInfo {
        username: username.to_string(),
        filename: filename.to_string(),
        size: 10,
        bytes_sent: 1,
        speed_bytes_per_sec: 1.0,
        status: soulseek_rs::types::UploadStatus::InProgress,
    }
}

fn at_status(
    username: &str,
    filename: &str,
    status: soulseek_rs::DownloadStatus,
) -> soulseek_rs::types::Download {
    let (sender, _) = std::sync::mpsc::channel();
    soulseek_rs::types::Download {
        username: username.to_string(),
        filename: filename.to_string(),
        token: 1,
        size: 4096,
        download_directory: "/tmp".to_string(),
        status,
        sender,
        queue_position: None,
        metadata: soulseek_rs::types::DownloadMetadata::default(),
    }
}

fn key(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
}

impl SessionApi for TalkativeSession {
    fn message_history(&self) -> Vec<ChatMessageDto> {
        self.history.clone()
    }
    fn daemon_endpoint(&self) -> Option<String> {
        self.shared.then(|| "test-daemon".to_string())
    }
    fn username(&self) -> String {
        "tester".to_string()
    }
    fn listen_port(&self) -> Option<u16> {
        None
    }
    fn session_loss(&self) -> Option<soulseek_rs::SessionLoss> {
        None
    }
    fn search_with_cancel(
        &self,
        query: &str,
        _timeout: Duration,
        _cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    ) -> soulseek_rs::Result<Vec<soulseek_rs::SearchResult>> {
        self.searched
            .lock()
            .expect("not poisoned")
            .push(query.to_string());
        Ok(Vec::new())
    }
    fn get_search_results(&self, _key: &str) -> Vec<soulseek_rs::SearchResult> {
        Vec::new()
    }
    fn all_searches(&self) -> Vec<crate::api::SessionSearch> {
        self.all_searches_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.searches.lock().expect("not poisoned").clone()
    }
    fn forget_search(&self, query: &str) -> bool {
        let mut searches = self.searches.lock().expect("not poisoned");
        let before = searches.len();
        searches.retain(|search| search.query != query);
        searches.len() != before
    }
    fn get_search_results_count(&self, _key: &str) -> usize {
        0
    }
    fn try_get_search_results(
        &self,
        _key: &str,
    ) -> Option<Vec<soulseek_rs::SearchResult>> {
        self.result_fetches
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Some(self.results.lock().expect("not poisoned").clone())
    }
    fn start_wishlist_search(&self, _query: &str) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn wishlist_interval(&self) -> Duration {
        Duration::from_mins(12)
    }
    fn download(
        &self,
        _filename: String,
        _username: String,
        _size: u64,
        _directory: String,
    ) -> soulseek_rs::Result<(
        soulseek_rs::types::Download,
        std::sync::mpsc::Receiver<soulseek_rs::DownloadStatus>,
    )> {
        Err(soulseek_rs::SoulseekRs::NotConnected)
    }
    fn download_with_metadata(
        &self,
        _filename: String,
        _username: String,
        _size: u64,
        _directory: String,
        _metadata: soulseek_rs::types::DownloadMetadata,
    ) -> soulseek_rs::Result<(
        soulseek_rs::types::Download,
        std::sync::mpsc::Receiver<soulseek_rs::DownloadStatus>,
    )> {
        Err(soulseek_rs::SoulseekRs::NotConnected)
    }
    fn get_all_downloads(&self) -> Vec<soulseek_rs::types::Download> {
        self.downloads.clone()
    }
    fn pause_download(&self, _u: &str, _f: &str) -> bool {
        false
    }
    fn resume_download(&self, _u: &str, _f: &str) -> bool {
        false
    }
    fn remove_queued_download(&self, _u: &str, _f: &str) -> bool {
        false
    }
    fn remove_download(&self, u: &str, f: &str) -> bool {
        self.download_removed
            .lock()
            .expect("not poisoned")
            .push((u.to_string(), f.to_string()));
        true
    }
    fn uploads(&self) -> Vec<soulseek_rs::UploadInfo> {
        Vec::new()
    }
    fn take_upload_events(&self) -> Vec<soulseek_rs::UploadInfo> {
        Vec::new()
    }
    fn cancel_upload(&self, u: &str, f: &str) -> bool {
        self.upload_cancelled
            .lock()
            .expect("not poisoned")
            .push((u.to_string(), f.to_string()));
        true
    }
    fn cancel_download(&self, u: &str, f: &str) -> bool {
        self.download_cancelled
            .lock()
            .expect("not poisoned")
            .push((u.to_string(), f.to_string()));
        true
    }
    fn set_upload_slots(&self, _slots: usize) {}
    fn check_privileges(&self) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn own_privilege_seconds(&self) -> Option<u32> {
        None
    }
    fn request_room_list(&self) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn join_room(&self, _room: &str) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn leave_room(&self, _room: &str) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn say_in_room(
        &self,
        _room: &str,
        _message: &str,
    ) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn room_members(&self, _room: &str) -> Vec<String> {
        Vec::new()
    }
    fn room_member_stats(
        &self,
        _room: &str,
    ) -> Vec<soulseek_rs::types::RoomUserStats> {
        Vec::new()
    }
    fn take_room_events(&self) -> Vec<soulseek_rs::RoomEvent> {
        Vec::new()
    }
    fn send_private_message(
        &self,
        _u: &str,
        _m: &str,
    ) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn take_private_messages(&self) -> Vec<soulseek_rs::UserMessage> {
        Vec::new()
    }
    fn browse_user(&self, _username: &str) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn take_browse_result(
        &self,
        _username: &str,
    ) -> Option<Vec<soulseek_rs::SharedDirectory>> {
        None
    }
    fn request_user_info(&self, _username: &str) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn watch_user(&self, _username: &str) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn unwatch_user(&self, _username: &str) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn watched_users(&self) -> Vec<String> {
        Vec::new()
    }
    fn user_info(&self, _username: &str) -> Option<soulseek_rs::UserInfo> {
        None
    }
    fn shared_counts(&self) -> (u32, u32) {
        (3, 12)
    }
    fn change_password(&self, password: &str) -> soulseek_rs::Result<()> {
        *self.password_set.lock().expect("not poisoned") =
            Some(password.to_string());
        Ok(())
    }
    fn shared_listing(&self) -> Vec<soulseek_rs::SharedDirectory> {
        self.listing.lock().expect("not poisoned").clone()
    }
    fn shared_directories(&self) -> Vec<String> {
        Vec::new()
    }
    fn set_shared_directories(
        &self,
        directories: Vec<String>,
    ) -> soulseek_rs::Result<()> {
        *self.shares_set.lock().expect("not poisoned") = Some(directories);
        Ok(())
    }
    fn set_download_directory(
        &self,
        directory: String,
    ) -> soulseek_rs::Result<()> {
        *self.download_dir_set.lock().expect("not poisoned") = Some(directory);
        Ok(())
    }
}

fn tui(history: Vec<ChatMessageDto>) -> MainTui {
    with_session(TalkativeSession {
        history,
        shared: true,
        ..TalkativeSession::default()
    })
}

fn shared(searches: Vec<crate::api::SessionSearch>) -> TalkativeSession {
    TalkativeSession {
        searches: std::sync::Mutex::new(searches),
        shared: true,
        ..TalkativeSession::default()
    }
}

fn search_of(query: &str, files: usize, age: u64) -> crate::api::SessionSearch {
    crate::api::SessionSearch {
        query: query.to_string(),
        files,
        started_secs_ago: Some(age),
    }
}

fn with_session(session: TalkativeSession) -> MainTui {
    attach(Arc::new(session))
}

fn attach(session: Arc<TalkativeSession>) -> MainTui {
    MainTui::new(
        session,
        "/tmp".to_string(),
        Duration::from_secs(1),
        None,
        None,
    )
}

fn files_from_bob(names: &[&str]) -> soulseek_rs::SearchResult {
    soulseek_rs::SearchResult {
        token: 0,
        files: names
            .iter()
            .map(|name| soulseek_rs::types::File {
                username: "bob".to_string(),
                name: (*name).to_string(),
                size: 42,
                attribs: std::collections::HashMap::new(),
            })
            .collect(),
        slots: 1,
        speed: 0,
        username: "bob".to_string(),
    }
}

/// An attached window that selected a two-file search and has already
/// ticked past its one necessary fetch.
fn window_that_pulled_two_files() -> (Arc<TalkativeSession>, MainTui) {
    let session = Arc::new(TalkativeSession {
        results: std::sync::Mutex::new(vec![files_from_bob(&[
            "@@x\\a.mp3",
            "@@x\\b.mp3",
        ])]),
        ..shared(vec![search_of("aphex twin", 2, 9_999)])
    });
    let mut tui = attach(session.clone());
    tui.state.selected_search_index = Some(0);
    tui.update_search_results();
    tui.update_search_results();
    (session, tui)
}

fn results(n: usize) -> Vec<crate::models::FileDisplayData> {
    (0..n)
        .map(|i| crate::models::FileDisplayData {
            filename: format!("@@x\\Music\\{i:02}.mp3"),
            username: "bob".to_string(),
            ..Default::default()
        })
        .collect()
}

fn results_tui(n: usize) -> MainTui {
    let mut tui = with_session(TalkativeSession::default());
    tui.state.results_items = results(n);
    tui.state.focused_pane = FocusedPane::Results;
    // 88 cells wide gives the name column 11 cells and the folder 7.
    tui.state.results_pane_area =
        Some(ratatui::layout::Rect::new(0, 0, 88, 13));
    tui
}

fn press(tui: &mut MainTui, code: KeyCode) {
    tui.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

fn ctrl(tui: &mut MainTui, ch: char) {
    tui.handle_key_event(KeyEvent::new(
        KeyCode::Char(ch),
        KeyModifiers::CONTROL,
    ));
}

fn selected(tui: &MainTui) -> Option<usize> {
    tui.state.results_table_state.selected()
}

fn screen_sized(tui: &mut MainTui, width: u16, height: u16) -> String {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("backend");
    terminal.draw(|frame| tui.render(frame)).expect("draw");
    terminal.backend().to_string()
}

fn screen_of(tui: &mut MainTui) -> String {
    screen_sized(tui, 160, 40)
}

fn mouse(
    tui: &mut MainTui,
    kind: ratatui::crossterm::event::MouseEventKind,
    column: u16,
    row: u16,
) {
    tui.handle_mouse_event(ratatui::crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

fn mouse_drag_row(tui: &mut MainTui, from: (u16, u16), to: (u16, u16)) {
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    mouse(tui, MouseEventKind::Down(MouseButton::Left), from.0, from.1);
    mouse(tui, MouseEventKind::Drag(MouseButton::Left), to.0, to.1);
    mouse(tui, MouseEventKind::Up(MouseButton::Left), to.0, to.1);
}

/// The screen row carrying a pane's legend, `[2] Results` say, without
/// the quotes the test backend wraps each row in.
fn row_with<'a>(screen: &'a str, needle: &str) -> &'a str {
    let line = screen
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no row with {needle:?} in\n{screen}"));
    let start = line.find('"').map_or(0, |i| i + 1);
    let end = line.rfind('"').unwrap_or(line.len());
    &line[start..end]
}

/// A window with a search, its results, and a transfer, so every pane
/// has something to show.
fn furnished_tui() -> MainTui {
    let mut tui = results_tui(3);
    tui.state.searches.push(crate::models::SearchEntry {
        query: "aphex twin".to_string(),
        status: SearchStatus::Completed,
        results: results(3),
        known_files: 3,
        owned: true,
        start_time: std::time::Instant::now(),
        cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    tui.state.selected_search_index = Some(0);
    tui.state.downloads.push(crate::models::DownloadEntry {
        download: queued("bob", "@@x\\Music\\00.mp3"),
        receiver: None,
    });
    tui
}

fn browsed(tui: &mut MainTui, files: usize) {
    tui.state.browse.open("bob");
    let listing = vec![soulseek_rs::SharedDirectory {
        name: "Music".to_string(),
        files: (0..files)
            .map(|i| soulseek_rs::SharedFileEntry {
                name: format!("{i:02}.mp3"),
                size: 1,
                attributes: Vec::new(),
            })
            .collect(),
    }];
    tui.state
        .browse
        .active_tab_mut()
        .expect("tab")
        .load(&listing);
    tui.state.show_browse = true;
    let _ = screen_of(tui);
}

fn in_a_room(tui: &mut MainTui, messages: usize) {
    tui.state.rooms.apply_event(
        soulseek_rs::RoomEvent::Joined {
            room: "jazz".to_string(),
            users: (0..30).map(|i| format!("user{i:02}")).collect(),
        },
        None,
    );
    tui.state.rooms.focus_or_open("jazz");
    for i in 0..messages {
        tui.state.rooms.apply_event(
            soulseek_rs::RoomEvent::Message {
                room: "jazz".to_string(),
                username: "alice".to_string(),
                message: format!("line {i:03}"),
            },
            Some("jazz"),
        );
    }
    tui.state.show_rooms = true;
    let _ = screen_of(tui);
}

/// The queries the window has sent so far, once at least `count` of them
/// are out: a search goes to the session from a worker thread, so a test
/// has to give it a moment.
fn searched(session: &TalkativeSession, count: usize) -> Vec<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let seen = session.searched.lock().expect("not poisoned").clone();
        if seen.len() >= count || std::time::Instant::now() > deadline {
            return seen;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A window over last session's search list: the queries came back from
/// disk, with nothing under them.
fn restored(session: Arc<TalkativeSession>, queries: &[&str]) -> MainTui {
    let mut tui = attach(session);
    let queries: Vec<String> =
        queries.iter().map(ToString::to_string).collect();
    restore_searches(&mut tui.state, &queries);
    tui.state.focused_pane = FocusedPane::Searches;
    tui
}
