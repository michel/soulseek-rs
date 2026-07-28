//! The seam between "what this program does" and "where the session lives".
//!
//! Commands and the TUI hold an `Arc<dyn SessionApi>` rather than an
//! `Arc<Client>`. Locally that *is* the client — the trait is implemented on
//! `Client` itself, so an `Arc<Client>` coerces straight to the trait object
//! and the local path keeps calling the same methods it always did. In daemon
//! mode the same handle is a [`crate::remote::RemoteSession`] that answers over
//! a socket.
//!
//! The signatures deliberately mirror `Client`'s, because every one of them is
//! already load-bearing somewhere in the UI or the command layer. The one
//! change is [`SessionApi::username`], which returns an owned `String`: a
//! remote session has no borrowed client to hand a `&str` out of.

use crate::daemon::proto::ChatMessageDto;
use soulseek_rs::types::{Download, DownloadMetadata};
use soulseek_rs::{
    Client, DownloadStatus, Result, RoomEvent, SearchResult, SessionLoss,
    SharedDirectory, UploadInfo, UserInfo, UserMessage,
};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// What a session knows about one search it has run.
///
/// `started_secs_ago` is what lets a second window tell a search that is still
/// collecting from one that has finished: the collecting window is the
/// client's, so without the daemon saying when a search began, every window
/// but the one that started it would show the result as final immediately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSearch {
    pub query: String,
    /// Files found so far, across every peer that has answered.
    pub files: usize,
    /// How long ago the search went out, or `None` when the session cannot
    /// say — which is the local case, where the window that ran it already
    /// knows.
    pub started_secs_ago: Option<u64>,
}

/// Everything the command layer and the TUI ask of a logged-in session.
///
/// Two rules keep the remote implementation honest:
///
/// * Non-destructive getters (`uploads`, `room_members`, `user_info`, …) are
///   plain request/response — a caller may poll them as often as it likes.
/// * The `take_*` drains are destructive, so with more than one client
///   attached they cannot be served by asking the daemon to drain on demand:
///   whoever asked first would steal the events. The daemon drains once and
///   fans the events out, and a remote session's `take_*` empties its own
///   local mirror.
pub trait SessionApi: Send + Sync {
    fn username(&self) -> String;
    /// The daemon this session is borrowed from, for the UI to name. `None`
    /// when the session belongs to this process.
    fn daemon_endpoint(&self) -> Option<String> {
        None
    }
    fn listen_port(&self) -> Option<u16>;
    fn session_loss(&self) -> Option<SessionLoss>;

    /// Search and wait out the window. Every implementation is
    /// `search_with_cancel` with nothing to cancel on.
    fn search(
        &self,
        query: &str,
        timeout: Duration,
    ) -> Result<Vec<SearchResult>> {
        self.search_with_cancel(query, timeout, None)
    }

    fn search_with_cancel(
        &self,
        query: &str,
        timeout: Duration,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<Vec<SearchResult>>;
    fn get_search_results(&self, key: &str) -> Vec<SearchResult>;
    /// Every search the session knows about.
    ///
    /// Daemon-side this is shared, so a second client sees what the first has
    /// been searching for rather than starting from a blank list.
    fn all_searches(&self) -> Vec<SessionSearch>;
    /// Drop a search from the session, so every window loses it rather than
    /// only the one that dismissed it.
    fn forget_search(&self, query: &str) -> bool;
    fn get_search_results_count(&self, key: &str) -> usize;
    fn try_get_search_results(&self, key: &str) -> Option<Vec<SearchResult>>;
    fn start_wishlist_search(&self, query: &str) -> Result<()>;
    fn wishlist_interval(&self) -> Duration;

    fn download(
        &self,
        filename: String,
        username: String,
        size: u64,
        download_directory: String,
    ) -> Result<(Download, Receiver<DownloadStatus>)>;
    fn download_with_metadata(
        &self,
        filename: String,
        username: String,
        size: u64,
        download_directory: String,
        metadata: DownloadMetadata,
    ) -> Result<(Download, Receiver<DownloadStatus>)>;
    fn get_all_downloads(&self) -> Vec<Download>;
    fn pause_download(&self, username: &str, filename: &str) -> bool;
    fn resume_download(&self, username: &str, filename: &str) -> bool;
    fn remove_queued_download(&self, username: &str, filename: &str) -> bool;
    fn remove_download(&self, username: &str, filename: &str) -> bool;

    fn uploads(&self) -> Vec<UploadInfo>;
    fn take_upload_events(&self) -> Vec<UploadInfo>;
    fn cancel_upload(&self, username: &str, filename: &str) -> bool;
    fn set_upload_slots(&self, slots: usize);
    fn check_privileges(&self) -> Result<()>;
    fn own_privilege_seconds(&self) -> Option<u32>;

    fn request_room_list(&self) -> Result<()>;
    fn join_room(&self, room: &str) -> Result<()>;
    fn leave_room(&self, room: &str) -> Result<()>;
    fn say_in_room(&self, room: &str, message: &str) -> Result<()>;
    fn room_members(&self, room: &str) -> Vec<String>;
    fn take_room_events(&self) -> Vec<RoomEvent>;

    fn send_private_message(&self, username: &str, message: &str)
    -> Result<()>;
    fn take_private_messages(&self) -> Vec<UserMessage>;

    fn browse_user(&self, username: &str) -> Result<()>;
    fn take_browse_result(
        &self,
        username: &str,
    ) -> Option<Vec<SharedDirectory>>;
    fn request_user_info(&self, username: &str) -> Result<()>;
    fn user_info(&self, username: &str) -> Option<UserInfo>;

    /// The conversation this session already knows about.
    ///
    /// Empty locally, where the TUI's own snapshot on disk is the record. A
    /// daemon has been collecting private messages the whole time it has been
    /// running, including while nothing was attached, so an attaching client
    /// asks for the backlog rather than starting a conversation half-way
    /// through.
    fn message_history(&self) -> Vec<ChatMessageDto> {
        Vec::new()
    }

    fn shared_counts(&self) -> (u32, u32);
    fn shared_directories(&self) -> Vec<String>;
    fn set_shared_directories(&self, directories: Vec<String>) -> Result<()>;

    /// Point future transfers at `directory`.
    ///
    /// Only a daemon holds a download folder of its own — locally every
    /// download call names its directory, so the local session has nothing to
    /// store and accepts silently.
    fn set_download_directory(&self, directory: String) -> Result<()>;
}

/// The local session: the library client itself, so `Arc<Client>` is already an
/// `Arc<dyn SessionApi>` and the in-process path costs nothing but a vtable.
impl SessionApi for Client {
    fn username(&self) -> String {
        Self::username(self).to_string()
    }

    fn listen_port(&self) -> Option<u16> {
        Self::listen_port(self)
    }

    fn session_loss(&self) -> Option<SessionLoss> {
        Self::session_loss(self)
    }

    fn search_with_cancel(
        &self,
        query: &str,
        timeout: Duration,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<Vec<SearchResult>> {
        Self::search_with_cancel(self, query, timeout, cancel)
    }

    fn get_search_results(&self, key: &str) -> Vec<SearchResult> {
        Self::get_search_results(self, key)
    }

    fn forget_search(&self, query: &str) -> bool {
        Self::forget_search(self, query)
    }

    fn all_searches(&self) -> Vec<SessionSearch> {
        Self::get_all_searches(self)
            .into_iter()
            .map(|(query, search)| SessionSearch {
                query,
                files: search.results.iter().map(|r| r.files.len()).sum(),
                // A local window started every search it can see, so it
                // already knows which are still running.
                started_secs_ago: None,
            })
            .collect()
    }

    fn get_search_results_count(&self, key: &str) -> usize {
        Self::get_search_results_count(self, key)
    }

    fn try_get_search_results(&self, key: &str) -> Option<Vec<SearchResult>> {
        Self::try_get_search_results(self, key)
    }

    fn start_wishlist_search(&self, query: &str) -> Result<()> {
        Self::start_wishlist_search(self, query)
    }

    fn wishlist_interval(&self) -> Duration {
        Self::wishlist_interval(self)
    }

    fn download(
        &self,
        filename: String,
        username: String,
        size: u64,
        download_directory: String,
    ) -> Result<(Download, Receiver<DownloadStatus>)> {
        Self::download(self, filename, username, size, download_directory)
    }

    fn download_with_metadata(
        &self,
        filename: String,
        username: String,
        size: u64,
        download_directory: String,
        metadata: DownloadMetadata,
    ) -> Result<(Download, Receiver<DownloadStatus>)> {
        Self::download_with_metadata(
            self,
            filename,
            username,
            size,
            download_directory,
            metadata,
        )
    }

    fn get_all_downloads(&self) -> Vec<Download> {
        Self::get_all_downloads(self)
    }

    fn pause_download(&self, username: &str, filename: &str) -> bool {
        Self::pause_download(self, username, filename)
    }

    fn resume_download(&self, username: &str, filename: &str) -> bool {
        Self::resume_download(self, username, filename)
    }

    fn remove_queued_download(&self, username: &str, filename: &str) -> bool {
        Self::remove_queued_download(self, username, filename)
    }

    fn remove_download(&self, username: &str, filename: &str) -> bool {
        Self::remove_download(self, username, filename)
    }

    fn uploads(&self) -> Vec<UploadInfo> {
        Self::uploads(self)
    }

    fn take_upload_events(&self) -> Vec<UploadInfo> {
        Self::take_upload_events(self)
    }

    fn cancel_upload(&self, username: &str, filename: &str) -> bool {
        Self::cancel_upload(self, username, filename)
    }

    fn set_upload_slots(&self, slots: usize) {
        Self::set_upload_slots(self, slots);
    }

    fn check_privileges(&self) -> Result<()> {
        Self::check_privileges(self)
    }

    fn own_privilege_seconds(&self) -> Option<u32> {
        Self::own_privilege_seconds(self)
    }

    fn request_room_list(&self) -> Result<()> {
        Self::request_room_list(self)
    }

    fn join_room(&self, room: &str) -> Result<()> {
        Self::join_room(self, room)
    }

    fn leave_room(&self, room: &str) -> Result<()> {
        Self::leave_room(self, room)
    }

    fn say_in_room(&self, room: &str, message: &str) -> Result<()> {
        Self::say_in_room(self, room, message)
    }

    fn room_members(&self, room: &str) -> Vec<String> {
        Self::room_members(self, room)
    }

    fn take_room_events(&self) -> Vec<RoomEvent> {
        Self::take_room_events(self)
    }

    fn send_private_message(
        &self,
        username: &str,
        message: &str,
    ) -> Result<()> {
        Self::send_private_message(self, username, message)
    }

    fn take_private_messages(&self) -> Vec<UserMessage> {
        Self::take_private_messages(self)
    }

    fn browse_user(&self, username: &str) -> Result<()> {
        Self::browse_user(self, username)
    }

    fn take_browse_result(
        &self,
        username: &str,
    ) -> Option<Vec<SharedDirectory>> {
        Self::take_browse_result(self, username)
    }

    fn request_user_info(&self, username: &str) -> Result<()> {
        Self::request_user_info(self, username)
    }

    fn user_info(&self, username: &str) -> Option<UserInfo> {
        Self::user_info(self, username)
    }

    fn shared_counts(&self) -> (u32, u32) {
        Self::shared_counts(self)
    }

    fn shared_directories(&self) -> Vec<String> {
        Self::shared_directories(self)
    }

    fn set_shared_directories(&self, directories: Vec<String>) -> Result<()> {
        Self::set_shared_directories(self, directories)
    }

    fn set_download_directory(&self, _directory: String) -> Result<()> {
        Ok(())
    }
}
