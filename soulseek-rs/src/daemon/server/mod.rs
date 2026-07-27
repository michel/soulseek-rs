//! The control listener: who may connect, and what they may ask for.
//!
//! One thread per connection, which is noise next to the thousand-odd peer
//! threads the client already runs. Each connection owns a writer thread fed by
//! a channel, so request replies and pushed events interleave onto the socket
//! without a lock and without either being able to block the other.

use super::hub::{Hub, PendingBrowses, PendingDownload};
use super::proto::{
    Ack, AuthResult, CODE_APPLICATION, CODE_INVALID_PARAMS, ChatMessageDto,
    DaemonStatus, DirectoriesParams, DownloadDto, DownloadStartParams,
    DownloadStarted, Downloads, IntervalSeconds, Members, MessageParams,
    Messages, Method, OPENRPC, PROTOCOL_VERSION, QueryParams, RoomRef,
    RpcError, SayParams, SearchResultDto, SearchResults, SearchSummary,
    Searches, Seconds, SharesStatus, SlotsParams, TransferRef, UploadInfoDto,
    Uploads, UserInfoDto, UserRef, UserResult,
};
use crate::api::SessionApi;
use crate::output::Exit;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Everything a connection needs to answer a request.
pub struct Daemon {
    pub session: Arc<dyn SessionApi>,
    pub hub: Arc<Hub>,
    pub browses: Arc<PendingBrowses>,
    pub downloads: Arc<Mutex<Vec<PendingDownload>>>,
    /// Where transfers land, on *this* machine. A remote caller cannot see
    /// this filesystem, so it does not get to choose.
    pub download_dir: String,
    pub server: String,
    pub token: String,
    pub started: Instant,
    pub stop: Arc<AtomicBool>,
    /// Connections currently being served, capped so an unauthenticated peer
    /// cannot spawn threads without bound.
    pub open: Arc<std::sync::atomic::AtomicUsize>,
    /// Rooms this session has joined, so a restart can rejoin them.
    pub rooms: Mutex<Vec<String>>,
    /// When each search went out. The window a search collects for belongs to
    /// the client, so a second client can only tell a running search from a
    /// finished one if the daemon remembers when it started.
    pub searches: Mutex<std::collections::HashMap<String, Instant>>,
}

impl Daemon {
    /// Answer one request.
    ///
    /// The match is exhaustive over [`Method`], so a new method cannot be added
    /// to the protocol without being implemented here.
    #[allow(clippy::too_many_lines)]
    pub fn call(
        &self,
        method: Method,
        params: Value,
    ) -> Result<Value, RpcError> {
        match method {
            // Handled during the handshake; reaching here means a second
            // `auth` on an established connection, which is answered with the
            // shape the contract publishes rather than a bare acknowledgement.
            Method::Auth => ok(self.identify()),
            Method::DaemonStatus => ok(self.status()),
            Method::DaemonStop => {
                self.stop.store(true, Ordering::Relaxed);
                ok(Ack::OK)
            }
            Method::RpcDiscover => serde_json::from_str(OPENRPC).map_err(|e| {
                RpcError::new(
                    CODE_APPLICATION,
                    format!("the contract is unreadable: {e}"),
                )
            }),

            Method::SearchStart => {
                let query: QueryParams = parse(params)?;
                // Zero window: put the search on the wire and return. The
                // caller decides how long to let answers accumulate, which is
                // what keeps one client's search from blocking the daemon.
                self.session
                    .search_with_cancel(&query.query, Duration::ZERO, None)
                    .map_err(|e| {
                        RpcError::application(
                            Exit::Connection,
                            format!("search failed: {e}"),
                        )
                    })?;
                self.record_search_start(&query.query);
                ok(Ack::OK)
            }
            Method::SearchResults => {
                let query: QueryParams = parse(params)?;
                ok(SearchResults {
                    results: self
                        .session
                        .get_search_results(&query.query)
                        .iter()
                        .map(SearchResultDto::from)
                        .collect(),
                })
            }
            Method::SearchList => ok(Searches {
                searches: self
                    .session
                    .all_searches()
                    .into_iter()
                    .map(|search| SearchSummary {
                        files: search.files,
                        started_secs_ago: self.search_age(&search.query),
                        query: search.query,
                    })
                    .collect(),
            }),
            Method::SearchForget => {
                let query: QueryParams = parse(params)?;
                self.forget_search_start(&query.query);
                ok(Ack {
                    ok: self.session.forget_search(&query.query),
                })
            }
            Method::SearchWishlist => {
                let query: QueryParams = parse(params)?;
                self.session.start_wishlist_search(&query.query).map_err(
                    |e| RpcError::application(Exit::Connection, e.to_string()),
                )?;
                ok(Ack::OK)
            }
            Method::SearchWishlistInterval => ok(IntervalSeconds {
                seconds: self.session.wishlist_interval().as_secs(),
            }),

            Method::DownloadStart => self.start_download(parse(params)?),
            Method::DownloadList => ok(Downloads {
                downloads: self
                    .session
                    .get_all_downloads()
                    .iter()
                    .map(DownloadDto::from)
                    .collect(),
            }),
            Method::DownloadPause => {
                let transfer: TransferRef = parse(params)?;
                ok(Ack {
                    ok: self
                        .session
                        .pause_download(&transfer.username, &transfer.filename),
                })
            }
            Method::DownloadResume => {
                let transfer: TransferRef = parse(params)?;
                ok(Ack {
                    ok: self.session.resume_download(
                        &transfer.username,
                        &transfer.filename,
                    ),
                })
            }
            Method::DownloadRemove => {
                let transfer: TransferRef = parse(params)?;
                ok(Ack {
                    ok: self.session.remove_download(
                        &transfer.username,
                        &transfer.filename,
                    ),
                })
            }
            Method::DownloadRemoveQueued => {
                let transfer: TransferRef = parse(params)?;
                ok(Ack {
                    ok: self.session.remove_queued_download(
                        &transfer.username,
                        &transfer.filename,
                    ),
                })
            }

            Method::UploadList => ok(Uploads {
                uploads: self
                    .session
                    .uploads()
                    .iter()
                    .map(UploadInfoDto::from)
                    .collect(),
            }),
            Method::UploadCancel => {
                let transfer: TransferRef = parse(params)?;
                ok(Ack {
                    ok: self
                        .session
                        .cancel_upload(&transfer.username, &transfer.filename),
                })
            }
            Method::UploadSlots => {
                let slots: SlotsParams = parse(params)?;
                self.session.set_upload_slots(slots.slots);
                ok(Ack::OK)
            }

            Method::PrivilegesCheck => {
                self.session.check_privileges().map_err(|e| {
                    RpcError::application(Exit::Connection, e.to_string())
                })?;
                ok(Ack::OK)
            }
            Method::PrivilegesOwn => ok(Seconds {
                seconds: self.session.own_privilege_seconds(),
            }),

            Method::RoomListRequest => {
                self.session.request_room_list().map_err(|e| {
                    RpcError::application(Exit::Connection, e.to_string())
                })?;
                ok(Ack::OK)
            }
            Method::RoomJoin => {
                let room: RoomRef = parse(params)?;
                self.session.join_room(&room.room).map_err(|e| {
                    RpcError::application(Exit::Connection, e.to_string())
                })?;
                self.joined(&room.room, true);
                ok(Ack::OK)
            }
            Method::RoomLeave => {
                let room: RoomRef = parse(params)?;
                self.session.leave_room(&room.room).map_err(|e| {
                    RpcError::application(Exit::Connection, e.to_string())
                })?;
                self.joined(&room.room, false);
                ok(Ack::OK)
            }
            Method::RoomSay => {
                let say: SayParams = parse(params)?;
                self.session.say_in_room(&say.room, &say.message).map_err(
                    |e| RpcError::application(Exit::Connection, e.to_string()),
                )?;
                ok(Ack::OK)
            }
            Method::RoomMembers => {
                let room: RoomRef = parse(params)?;
                ok(Members {
                    users: self.session.room_members(&room.room),
                })
            }

            Method::MessageSend => {
                let message: MessageParams = parse(params)?;
                self.session
                    .send_private_message(&message.username, &message.message)
                    .map_err(|e| {
                        RpcError::application(Exit::Connection, e.to_string())
                    })?;
                // Record our own half of the conversation: the drainer only
                // ever sees incoming messages, so without this a client that
                // attaches later reads a one-sided chat.
                self.hub.remember(ChatMessageDto {
                    peer: message.username,
                    outgoing: true,
                    text: message.message,
                    at: now_seconds(),
                });
                ok(Ack::OK)
            }
            Method::MessageHistory => ok(Messages {
                messages: self.hub.history(),
            }),

            Method::BrowseUser => {
                let user: UserRef = parse(params)?;
                // Register before asking: the answer can land on the very next
                // drainer tick, and an unexpected listing is discarded.
                self.browses.expect(&user.username);
                if let Err(e) = self.session.browse_user(&user.username) {
                    self.browses.forget(&user.username);
                    return Err(RpcError::application(
                        Exit::Connection,
                        e.to_string(),
                    ));
                }
                ok(Ack::OK)
            }
            Method::UserRequest => {
                let user: UserRef = parse(params)?;
                self.session.request_user_info(&user.username).map_err(
                    |e| RpcError::application(Exit::Connection, e.to_string()),
                )?;
                ok(Ack::OK)
            }
            Method::UserInfoOf => {
                let user: UserRef = parse(params)?;
                ok(UserResult {
                    user: self
                        .session
                        .user_info(&user.username)
                        .as_ref()
                        .map(UserInfoDto::from),
                })
            }

            Method::SharesStatusOf => ok(self.shares()),
            Method::SharesSet => {
                let directories: DirectoriesParams = parse(params)?;
                self.session
                    .set_shared_directories(directories.directories)
                    .map_err(|e| {
                        RpcError::application(Exit::Usage, e.to_string())
                    })?;
                ok(self.shares())
            }
            Method::SharesReindex => {
                self.session
                    .set_shared_directories(self.session.shared_directories())
                    .map_err(|e| {
                        RpcError::application(Exit::Usage, e.to_string())
                    })?;
                ok(self.shares())
            }
        }
    }

    /// Track membership so a restart can rejoin what was open. The library
    /// has no "which rooms am I in" accessor, so the daemon keeps the list.
    fn joined(&self, room: &str, member: bool) {
        let Ok(mut rooms) = self.rooms.lock() else {
            return;
        };
        rooms.retain(|name| name != room);
        if member {
            rooms.push(room.to_string());
        }
    }

    #[must_use]
    pub fn rooms(&self) -> Vec<String> {
        self.rooms
            .lock()
            .map_or_else(|_| Vec::new(), |rooms| rooms.clone())
    }

    fn start_download(
        &self,
        params: DownloadStartParams,
    ) -> Result<Value, RpcError> {
        // The daemon's own directory, always. Letting a caller name a path
        // would hand any authenticated client an arbitrary write on this
        // host's filesystem, and a remote one cannot see that filesystem to
        // choose sensibly anyway.
        let directory = self.download_dir.clone();
        let (download, updates) = self
            .session
            .download_with_metadata(
                params.filename.clone(),
                params.username.clone(),
                params.size,
                directory,
                params.metadata.into(),
            )
            .map_err(|e| {
                RpcError::application(
                    Exit::Transfer,
                    format!("cannot start: {e}"),
                )
            })?;

        if let Ok(mut pending) = self.downloads.lock() {
            pending.push(PendingDownload {
                username: params.username,
                filename: params.filename,
                updates,
            });
        }
        ok(DownloadStarted {
            download: DownloadDto::from(&download),
        })
    }

    fn shares(&self) -> SharesStatus {
        let (folders, files) = self.session.shared_counts();
        SharesStatus {
            folders,
            files,
            directories: self.session.shared_directories(),
        }
    }

    /// How long ago a search went out, or a long time when we never saw it
    /// start — a search restored or begun before this daemon did.
    fn search_age(&self, query: &str) -> u64 {
        self.searches
            .lock()
            .ok()
            .and_then(|searches| {
                searches.get(query).map(|at| at.elapsed().as_secs())
            })
            .unwrap_or(u64::MAX)
    }

    fn record_search_start(&self, query: &str) {
        if let Ok(mut searches) = self.searches.lock() {
            searches.insert(query.to_string(), Instant::now());
        }
    }

    fn forget_search_start(&self, query: &str) {
        if let Ok(mut searches) = self.searches.lock() {
            searches.remove(query);
        }
    }

    fn identify(&self) -> AuthResult {
        AuthResult {
            protocol: PROTOCOL_VERSION,
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            username: self.session.username(),
        }
    }

    fn status(&self) -> DaemonStatus {
        let (folders, files) = self.session.shared_counts();
        DaemonStatus {
            username: self.session.username(),
            server: self.server.clone(),
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol: PROTOCOL_VERSION,
            listen_port: self.session.listen_port(),
            shared_folders: folders,
            shared_files: files,
            download_dir: self.download_dir.clone(),
            session_loss: self.session.session_loss().map(Into::into),
            clients: self.hub.subscribers(),
            uptime_secs: self.started.elapsed().as_secs(),
        }
    }
}

fn ok<T: serde::Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|e| {
        RpcError::new(
            CODE_APPLICATION,
            format!("cannot serialize the reply: {e}"),
        )
    })
}

fn parse<T: DeserializeOwned>(params: Value) -> Result<T, RpcError> {
    serde_json::from_value(params)
        .map_err(|e| RpcError::new(CODE_INVALID_PARAMS, e.to_string()))
}

/// Wall-clock seconds, to stamp a message the way the server stamps the ones
/// it delivers. A clock before the epoch reads as zero rather than panicking.
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

mod transport;

#[cfg(unix)]
pub use transport::bind_unix;
pub use transport::{Listener, bind_tcp};

#[cfg(test)]
mod tests {
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
                    == Method::ALL.len()
                        + super::super::proto::Event::ALL.len()),
            "discover must describe every method and event"
        );
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
    struct SilentSession;

    fn daemon_with_token(token: &str) -> Daemon {
        Daemon {
            session: Arc::new(SilentSession),
            hub: Arc::new(Hub::new()),
            browses: Arc::new(PendingBrowses::default()),
            downloads: Arc::new(Mutex::new(Vec::new())),
            download_dir: "/tmp".to_string(),
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
        fn get_search_results(
            &self,
            _key: &str,
        ) -> Vec<soulseek_rs::SearchResult> {
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
        fn start_wishlist_search(
            &self,
            _query: &str,
        ) -> soulseek_rs::Result<()> {
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
        fn remove_queued_download(
            &self,
            _username: &str,
            _filename: &str,
        ) -> bool {
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
        fn request_user_info(
            &self,
            _username: &str,
        ) -> soulseek_rs::Result<()> {
            Ok(())
        }
        fn user_info(&self, _username: &str) -> Option<soulseek_rs::UserInfo> {
            None
        }
        fn shared_counts(&self) -> (u32, u32) {
            (0, 0)
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
    }
}
