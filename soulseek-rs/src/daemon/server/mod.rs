//! The control listener: who may connect, and what they may ask for.
//!
//! One thread per connection, which is noise next to the thousand-odd peer
//! threads the client already runs. Each connection owns a writer thread fed by
//! a channel, so request replies and pushed events interleave onto the socket
//! without a lock and without either being able to block the other.

use super::hub::{Hub, PendingBrowses, PendingDownload};
use super::proto::{
    Ack, AuthResult, CODE_APPLICATION, CODE_INVALID_PARAMS, ChatMessageDto,
    DaemonStatus, DirectoriesParams, DirectoryParams, DownloadDto,
    DownloadStartParams, DownloadStarted, Downloads, IntervalSeconds,
    MemberStats, Members, MessageParams, Messages, Method, OPENRPC,
    PROTOCOL_VERSION, PasswordParams, QueryParams, RoomRef, RoomUserStatsDto,
    RpcError, SayParams, SearchResultDto, SearchResults, SearchSummary,
    Searches, Seconds, SharedDirectoryDto, SharedListing, SharesStatus,
    SlotsParams, TransferRef, UploadInfoDto, Uploads, UserInfoDto, UserRef,
    UserResult, Watched,
};
use crate::api::SessionApi;
use crate::output::Exit;
use crate::persist::secret::SecretStore;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Everything a connection needs to answer a request.
pub struct Daemon {
    pub session: Arc<dyn SessionApi>,
    /// Where this host keeps the account's password. The daemon owns the
    /// login, so a password it changes it also has to store.
    pub secrets: Arc<dyn SecretStore>,
    pub hub: Arc<Hub>,
    pub browses: Arc<PendingBrowses>,
    pub downloads: Arc<Mutex<Vec<PendingDownload>>>,
    /// Where transfers land, on *this* machine. A caller may point it at a
    /// different folder with `download.set_dir`, but never per download.
    pub download_dir: Mutex<String>,
    pub use_username_dirs: bool,
    pub preserve_remote_dirs: bool,
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
                // Without a start time every other window would render this
                // search as finished the moment it goes out.
                self.record_search_start(&query.query);
                ok(Ack::OK)
            }
            Method::SearchWishlistInterval => ok(IntervalSeconds {
                seconds: self.session.wishlist_interval().as_secs(),
            }),

            Method::DownloadStart => self.start_download(parse(params)?),
            Method::DownloadSetDir => {
                let dir: DirectoryParams = parse(params)?;
                self.set_download_dir(dir.directory)?;
                ok(Ack::OK)
            }
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
            Method::DownloadCancel => {
                let transfer: TransferRef = parse(params)?;
                ok(Ack {
                    ok: self.session.cancel_download(
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
            Method::UploadRemove => {
                let transfer: TransferRef = parse(params)?;
                ok(Ack {
                    ok: self
                        .session
                        .remove_upload(&transfer.username, &transfer.filename),
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

            Method::RoomMemberStats => {
                let room: RoomRef = parse(params)?;
                ok(MemberStats {
                    members: self
                        .session
                        .room_member_stats(&room.room)
                        .iter()
                        .map(RoomUserStatsDto::from)
                        .collect(),
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

            Method::UserWatch => {
                let user: UserRef = parse(params)?;
                self.session.watch_user(&user.username).map_err(|e| {
                    RpcError::application(Exit::Connection, e.to_string())
                })?;
                ok(Ack::OK)
            }
            Method::UserUnwatch => {
                let user: UserRef = parse(params)?;
                self.session.unwatch_user(&user.username).map_err(|e| {
                    RpcError::application(Exit::Connection, e.to_string())
                })?;
                ok(Ack::OK)
            }
            Method::UserWatched => ok(Watched {
                users: self.session.watched_users(),
            }),

            Method::AccountPassword => {
                let params: PasswordParams = parse(params)?;
                // The daemon logs in again from this host's store when it
                // restarts, so a password it changed but never stored would
                // lock it out of its own account.
                let stored = crate::persist::secret::change_password(
                    self.session.as_ref(),
                    self.secrets.as_ref(),
                    &params.password,
                )
                .map_err(|e| {
                    RpcError::application(Exit::Connection, e.to_string())
                })?;
                if let Some(e) = stored {
                    soulseek_rs::warn!(
                        "Password changed, but not stored on this host: {e}"
                    );
                }
                ok(Ack::OK)
            }

            Method::SharesStatusOf => ok(self.shares()),
            Method::SharesFiles => ok(SharedListing {
                directories: self
                    .session
                    .shared_listing()
                    .into_iter()
                    .map(SharedDirectoryDto::from)
                    .collect(),
            }),
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
        // per download would scatter transfers in ways `daemon.status` cannot
        // report; changing the folder is a deliberate session-wide act,
        // `download.set_dir`.
        let destination = crate::directories::download_path_for(
            &self.download_dir(),
            &params.username,
            &params.filename,
            self.use_username_dirs,
            self.preserve_remote_dirs,
        );
        let directory = destination
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .display()
            .to_string();
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

    fn download_dir(&self) -> String {
        self.download_dir
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Point future transfers at `directory`, creating it on *this* host.
    /// The same trust boundary as `shares.set`: authenticated means owner.
    fn set_download_dir(&self, directory: String) -> Result<(), RpcError> {
        if directory.trim().is_empty() {
            return Err(RpcError::new(
                CODE_INVALID_PARAMS,
                "the download directory cannot be empty".to_string(),
            ));
        }
        std::fs::create_dir_all(&directory).map_err(|e| {
            RpcError::application(
                Exit::Usage,
                format!("cannot create {directory}: {e}"),
            )
        })?;
        *self
            .download_dir
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = directory;
        Ok(())
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
        // The session drops its oldest searches to make room for new ones.
        // Forget their start times with them, or this map is the one thing
        // that still grows with every query ever run.
        let known: std::collections::HashSet<String> = self
            .session
            .all_searches()
            .into_iter()
            .map(|search| search.query)
            .collect();
        if let Ok(mut searches) = self.searches.lock() {
            searches.retain(|started, _| known.contains(started));
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
            daemon_version: crate::cli::VERSION.to_string(),
            username: self.session.username(),
        }
    }

    fn status(&self) -> DaemonStatus {
        let (folders, files) = self.session.shared_counts();
        DaemonStatus {
            username: self.session.username(),
            server: self.server.clone(),
            daemon_version: crate::cli::VERSION.to_string(),
            protocol: PROTOCOL_VERSION,
            listen_port: self.session.listen_port(),
            shared_folders: folders,
            shared_files: files,
            download_dir: self.download_dir(),
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
mod tests;
