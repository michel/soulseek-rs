//! What each method takes and returns, and what each pushed event carries.

use super::dto::{
    DownloadDto, DownloadMetadataDto, DownloadStatusDto, SearchResultDto,
    SessionLossDto, SharedDirectoryDto, UploadInfoDto, UserInfoDto,
};
use serde::{Deserialize, Serialize};

/// A void result. JSON-RPC allows a null result, but an object leaves room to
/// add a field later without changing the shape a client already parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Ack {
    pub ok: bool,
}

impl Ack {
    pub const OK: Self = Self { ok: true };
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct QueryParams {
    pub query: String,
}

/// Names one transfer, which is how both directions are addressed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct TransferRef {
    pub username: String,
    pub filename: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct RoomRef {
    pub room: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct UserRef {
    pub username: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct SayParams {
    pub room: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct MessageParams {
    pub username: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct DownloadStartParams {
    pub username: String,
    pub filename: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub metadata: DownloadMetadataDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct SlotsParams {
    pub slots: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct DirectoriesParams {
    pub directories: Vec<String>,
}

/// Every search this session has run, newest last. Shared by all clients, so
/// a second one attaching sees what the first has been looking for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Searches {
    pub searches: Vec<SearchSummary>,
}

/// One search as the daemon sees it, for a client that did not run it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct SearchSummary {
    pub query: String,
    /// Files found so far, across every peer that has answered.
    pub files: usize,
    /// How long ago the query went out. A client compares this with its own
    /// search window to decide whether the search is still collecting.
    pub started_secs_ago: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct SearchResults {
    pub results: Vec<SearchResultDto>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Downloads {
    pub downloads: Vec<DownloadDto>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct DownloadStarted {
    pub download: DownloadDto,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Uploads {
    pub uploads: Vec<UploadInfoDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Members {
    pub users: Vec<String>,
}

/// One line of chat history. Unlike a live [`UserMessageDto`] this carries a
/// direction, because a conversation the daemon collected includes what this
/// account sent as well as what it received.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ChatMessageDto {
    /// The other party, whichever way the message went.
    pub peer: String,
    pub outgoing: bool,
    pub text: String,
    /// Unix seconds.
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Messages {
    pub messages: Vec<ChatMessageDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct UserResult {
    #[serde(default)]
    pub user: Option<UserInfoDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Seconds {
    #[serde(default)]
    pub seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct IntervalSeconds {
    pub seconds: u64,
}

/// A peer answered a browse request. Carried whole rather than as a "go ask
/// for it" nudge, because the daemon's own copy is consumed when it drains.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct BrowseEvent {
    pub username: String,
    pub directories: Vec<SharedDirectoryDto>,
}

/// One step in a transfer this client started.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct DownloadStatusEvent {
    pub username: String,
    pub filename: String,
    pub status: DownloadStatusDto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct SessionLossEvent {
    pub loss: SessionLossDto,
}
