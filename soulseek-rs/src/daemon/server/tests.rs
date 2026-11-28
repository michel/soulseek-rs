use super::super::proto::{CODE_METHOD_NOT_FOUND, CODE_PARSE};
use super::transport::{accepts, answer, constant_time_eq};
use super::*;
use serde_json::json;
use std::sync::mpsc::Receiver;

#[test]
fn a_unix_socket_needs_no_token_but_tcp_does() {
    let daemon = daemon_with_token("secret");
    assert!(
        accepts(&daemon, true, None),
        "socket permissions are the auth"
    );
    assert!(!accepts(&daemon, false, None), "TCP must demand a token");
}

#[test]
fn a_wrong_token_is_refused_on_every_transport() {
    let daemon = daemon_with_token("secret");
    assert!(!accepts(&daemon, false, Some("guess")));
    assert!(
        !accepts(&daemon, true, Some("guess")),
        "a token that is offered is always checked, even locally"
    );
    assert!(accepts(&daemon, false, Some("secret")));
}

#[test]
fn token_comparison_does_not_short_circuit_on_length_or_content() {
    assert!(constant_time_eq("abc", "abc"));
    assert!(!constant_time_eq("abc", "abd"));
    assert!(!constant_time_eq("abc", "abcd"));
    assert!(!constant_time_eq("", "a"));
    assert!(constant_time_eq("", ""));
}

#[test]
fn an_unknown_method_is_reported_rather_than_ignored() {
    let daemon = daemon_with_token("t");
    let response = answer(
        r#"{"jsonrpc":"2.0","id":1,"method":"search.nope"}"#,
        &daemon,
    )
    .expect("a request with an id is always answered");
    let error = response.error.expect("an unknown method is an error");
    assert_eq!(error.code, CODE_METHOD_NOT_FOUND);
}

#[test]
fn a_notification_gets_no_reply_even_when_it_fails() {
    let daemon = daemon_with_token("t");
    assert!(
        answer(r#"{"jsonrpc":"2.0","method":"search.nope"}"#, &daemon)
            .is_none(),
        "a request without an id must be answered with silence"
    );
}

#[test]
fn unparseable_input_is_a_parse_error_not_a_dropped_connection() {
    let daemon = daemon_with_token("t");
    let response =
        answer("this is not json", &daemon).expect("garbage is answered");
    assert_eq!(response.error.expect("an error").code, CODE_PARSE);
}

#[test]
fn bad_params_are_reported_as_such() {
    let daemon = daemon_with_token("t");
    let error = daemon
        .call(Method::RoomJoin, json!({ "not_a_room": 1 }))
        .expect_err("the params do not match");
    assert_eq!(error.code, CODE_INVALID_PARAMS);
}

#[test]
fn discover_serves_the_published_contract() {
    let daemon = daemon_with_token("t");
    let document = daemon
        .call(Method::RpcDiscover, Value::Null)
        .expect("the contract is compiled in");
    assert_eq!(document["openrpc"], "1.3.2");
    assert!(
        document["methods"]
            .as_array()
            .is_some_and(|methods| methods.len()
                == Method::ALL.len() + super::super::proto::Event::ALL.len()),
        "discover must describe every method and event"
    );
}

#[test]
fn member_stats_are_published_for_a_joined_room() {
    let daemon = daemon_with_token("t");
    let stats = daemon
        .call(Method::RoomMemberStats, json!({ "room": STUB_ROOM }))
        .expect("member stats");
    assert_eq!(stats["members"][0]["username"], "alice");
    assert_eq!(stats["members"][0]["status"], "online");
    assert_eq!(stats["members"][0]["slots_full"], true);
    assert_eq!(stats["members"][0]["country"], "NL");
}

#[test]
fn a_room_that_was_never_joined_has_no_member_stats() {
    // Empty, not an error: asking about a room this session is not in is
    // an ordinary thing for a client to do.
    let daemon = daemon_with_token("t");
    let stats = daemon
        .call(Method::RoomMemberStats, json!({ "room": "elsewhere" }))
        .expect("member stats");
    assert_eq!(stats["members"], json!([]));
}

#[test]
fn a_password_the_server_never_confirmed_is_not_stored() {
    let session = SilentSession {
        never_confirms: true,
        ..SilentSession::default()
    };
    let store = crate::persist::secret::MemoryStore::default();
    store.set("tester", "the-one-that-works").expect("stored");

    let changed = crate::persist::secret::change_password_within(
        &session,
        &store,
        "never-arrived",
        Duration::from_millis(60),
    );

    assert!(matches!(changed, Err(soulseek_rs::SoulseekRs::Timeout)));
    assert_eq!(
        store.get("tester").expect("the store answers").as_deref(),
        Some("the-one-that-works"),
        "the next start must still find the password that logs in"
    );
}

/// The daemon owns the login, so a password it changes it also stores —
/// its next start logs in from that store, not from the client that
/// asked.
#[test]
fn a_password_change_reaches_the_session_and_this_hosts_store() {
    let session = Arc::new(SilentSession::default());
    let daemon = daemon_with("t", Arc::clone(&session));

    daemon
        .call(Method::AccountPassword, json!({ "password": "hunter2" }))
        .expect("the password change is served");

    assert_eq!(
        *session.password.lock().expect("not poisoned"),
        Some("hunter2".to_string())
    );
    assert_eq!(
        daemon
            .secrets
            .get(&session.username())
            .expect("the store answers")
            .as_deref(),
        Some("hunter2")
    );
}

#[test]
fn a_watch_is_readable_back_and_survives_until_unwatched() {
    // The three methods are only useful together: what `user.watch`
    // records, `user.watched` must report, and `user.unwatch` must undo.
    let daemon = daemon_with_token("t");
    let user = |name: &str| json!({ "username": name });

    daemon
        .call(Method::UserWatch, user("alice"))
        .expect("watch");
    daemon.call(Method::UserWatch, user("bob")).expect("watch");
    let watched = daemon
        .call(Method::UserWatched, Value::Null)
        .expect("watched");
    assert_eq!(watched["users"], json!(["alice", "bob"]));

    daemon
        .call(Method::UserUnwatch, user("alice"))
        .expect("unwatch");
    let watched = daemon
        .call(Method::UserWatched, Value::Null)
        .expect("watched");
    assert_eq!(
        watched["users"],
        json!(["bob"]),
        "unwatching one must not clear the rest"
    );
}

#[test]
fn watching_the_same_user_twice_lists_them_once() {
    let daemon = daemon_with_token("t");
    let alice = json!({ "username": "alice" });
    daemon
        .call(Method::UserWatch, alice.clone())
        .expect("watch");
    daemon.call(Method::UserWatch, alice).expect("watch");
    let watched = daemon
        .call(Method::UserWatched, Value::Null)
        .expect("watched");
    assert_eq!(watched["users"], json!(["alice"]));
}

#[test]
fn stop_is_recorded_so_the_daemon_can_wind_down() {
    let daemon = daemon_with_token("t");
    assert!(!daemon.stop.load(Ordering::Relaxed));
    daemon
        .call(Method::DaemonStop, Value::Null)
        .expect("stop is always accepted");
    assert!(daemon.stop.load(Ordering::Relaxed));
}

// A session that answers nothing: enough to exercise dispatch, parameter
// validation, and the auth gate without a Soulseek server.
/// A session that answers nothing, with two exceptions the methods under
/// test need: it remembers watches, so what one call records the next can
/// read back, and it describes the members of one room, so the stats
/// method can return something for a joined room and nothing for any
/// other.
#[derive(Default)]
struct SilentSession {
    watched: Mutex<Vec<String>>,
    /// What `account.password` asked the session to set, if anything.
    password: Mutex<Option<String>>,
    /// A server that never confirms a password change.
    never_confirms: bool,
}

/// The one room `SilentSession` claims to have joined.
const STUB_ROOM: &str = "jazz";

pub(super) fn daemon_with_token(token: &str) -> Daemon {
    daemon_with(token, Arc::new(SilentSession::default()))
}

/// A daemon over a session the caller can read back, with an in-memory
/// secret store: no test may touch the machine's keychain.
fn daemon_with(token: &str, session: Arc<SilentSession>) -> Daemon {
    Daemon {
        session,
        secrets: Arc::new(crate::persist::secret::MemoryStore::default()),
        hub: Arc::new(Hub::new()),
        browses: Arc::new(PendingBrowses::default()),
        downloads: Arc::new(Mutex::new(Vec::new())),
        download_dir: Mutex::new("/tmp".to_string()),
        use_username_dirs: false,
        preserve_remote_dirs: false,
        server: "server.invalid:2416".to_string(),
        token: token.to_string(),
        started: Instant::now(),
        stop: Arc::new(AtomicBool::new(false)),
        open: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        rooms: Mutex::new(Vec::new()),
        searches: Mutex::new(std::collections::HashMap::new()),
    }
}

impl SessionApi for SilentSession {
    fn username(&self) -> String {
        "tester".to_string()
    }
    fn change_password(&self, password: &str) -> soulseek_rs::Result<()> {
        *self.password.lock().expect("not poisoned") =
            Some(password.to_string());
        Ok(())
    }
    fn password_change_confirmed(&self, _password: &str) -> bool {
        !self.never_confirms
    }
    fn listen_port(&self) -> Option<u16> {
        None
    }
    fn session_loss(&self) -> Option<soulseek_rs::SessionLoss> {
        None
    }
    fn search(
        &self,
        _query: &str,
        _timeout: Duration,
    ) -> soulseek_rs::Result<Vec<soulseek_rs::SearchResult>> {
        Ok(Vec::new())
    }
    fn search_with_cancel(
        &self,
        _query: &str,
        _timeout: Duration,
        _cancel: Option<Arc<AtomicBool>>,
    ) -> soulseek_rs::Result<Vec<soulseek_rs::SearchResult>> {
        Ok(Vec::new())
    }
    fn get_search_results(&self, _key: &str) -> Vec<soulseek_rs::SearchResult> {
        Vec::new()
    }
    fn all_searches(&self) -> Vec<crate::api::SessionSearch> {
        Vec::new()
    }
    fn forget_search(&self, _query: &str) -> bool {
        false
    }
    fn get_search_results_count(&self, _key: &str) -> usize {
        0
    }
    fn try_get_search_results(
        &self,
        _key: &str,
    ) -> Option<Vec<soulseek_rs::SearchResult>> {
        None
    }
    fn start_wishlist_search(&self, _query: &str) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn wishlist_interval(&self) -> Duration {
        Duration::from_mins(12)
    }
    fn download(
        &self,
        filename: String,
        username: String,
        size: u64,
        directory: String,
    ) -> soulseek_rs::Result<(
        soulseek_rs::types::Download,
        Receiver<soulseek_rs::DownloadStatus>,
    )> {
        self.download_with_metadata(
            filename,
            username,
            size,
            directory,
            soulseek_rs::types::DownloadMetadata::default(),
        )
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
        Receiver<soulseek_rs::DownloadStatus>,
    )> {
        Err(soulseek_rs::SoulseekRs::NotConnected)
    }
    fn get_all_downloads(&self) -> Vec<soulseek_rs::types::Download> {
        Vec::new()
    }
    fn pause_download(&self, _username: &str, _filename: &str) -> bool {
        false
    }
    fn resume_download(&self, _username: &str, _filename: &str) -> bool {
        false
    }
    fn remove_queued_download(&self, _username: &str, _filename: &str) -> bool {
        false
    }
    fn remove_download(&self, _username: &str, _filename: &str) -> bool {
        false
    }
    fn uploads(&self) -> Vec<soulseek_rs::UploadInfo> {
        Vec::new()
    }
    fn take_upload_events(&self) -> Vec<soulseek_rs::UploadInfo> {
        Vec::new()
    }
    fn cancel_upload(&self, _username: &str, _filename: &str) -> bool {
        false
    }
    fn remove_upload(&self, _username: &str, _filename: &str) -> bool {
        false
    }
    fn cancel_download(&self, _username: &str, _filename: &str) -> bool {
        false
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
        room: &str,
    ) -> Vec<soulseek_rs::types::RoomUserStats> {
        if room != STUB_ROOM {
            return Vec::new();
        }
        vec![soulseek_rs::types::RoomUserStats {
            username: "alice".to_string(),
            status: soulseek_rs::types::UserStatus::Online,
            average_speed: 1024,
            shared_files: 20,
            shared_folders: 3,
            slots_full: true,
            country: Some("NL".to_string()),
        }]
    }
    fn take_room_events(&self) -> Vec<soulseek_rs::RoomEvent> {
        Vec::new()
    }
    fn send_private_message(
        &self,
        _username: &str,
        _message: &str,
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
    fn watch_user(&self, username: &str) -> soulseek_rs::Result<()> {
        let mut watched = self.watched.lock().expect("watch list");
        if !watched.iter().any(|u| u == username) {
            watched.push(username.to_string());
            watched.sort();
        }
        Ok(())
    }
    fn unwatch_user(&self, username: &str) -> soulseek_rs::Result<()> {
        self.watched
            .lock()
            .expect("watch list")
            .retain(|u| u != username);
        Ok(())
    }
    fn watched_users(&self) -> Vec<String> {
        self.watched.lock().expect("watch list").clone()
    }
    fn user_info(&self, _username: &str) -> Option<soulseek_rs::UserInfo> {
        None
    }
    fn shared_counts(&self) -> (u32, u32) {
        (0, 0)
    }
    fn shared_listing(&self) -> Vec<soulseek_rs::SharedDirectory> {
        Vec::new()
    }
    fn shared_directories(&self) -> Vec<String> {
        Vec::new()
    }
    fn set_shared_directories(
        &self,
        _directories: Vec<String>,
    ) -> soulseek_rs::Result<()> {
        Ok(())
    }
    fn set_download_directory(
        &self,
        _directory: String,
    ) -> soulseek_rs::Result<()> {
        Ok(())
    }
}
