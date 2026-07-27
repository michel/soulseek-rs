//! The wire contract: JSON-RPC 2.0 framing and the data types that cross it.
//!
//! Library types never travel as themselves. Every payload is a DTO defined
//! here with an explicit conversion, so the wire format is versioned
//! independently of the library: a refactor inside `soulseek-rs-lib` cannot
//! silently change what a third-party script receives, and the compiler points
//! at this file when one would.
//!
//! The schemas published in `docs/openrpc.json` are derived from these types
//! (see the tests at the bottom), which is what keeps the advertised contract
//! and the running daemon in agreement.

use crate::output::Exit;
use serde::{Deserialize, Serialize};
use soulseek_rs::types::{
    Download, DownloadMetadata, UserPresence, UserStats, UserStatus,
};
use soulseek_rs::{
    DownloadStatus, File, RoomEvent, RoomInfo, SearchResult, SessionLoss,
    SharedDirectory, UploadInfo, UploadStatus, UserInfo, UserMessage,
};
use std::collections::HashMap;

/// Bumped when a change would break a client written against the old shape.
/// Sent in the `auth` reply so a mismatch is caught at connect time rather
/// than as a confusing failure three calls later.
pub const PROTOCOL_VERSION: u32 = 1;

/// The OpenRPC document, served by `rpc.discover` so a generator can point at
/// a running daemon instead of needing this repository.
pub const OPENRPC: &str = include_str!("../../../docs/openrpc.json");

// ---------------------------------------------------------------------------
// JSON-RPC envelope
// ---------------------------------------------------------------------------

/// A request from a client. `id` is absent for a notification, which by
/// JSON-RPC rule gets no reply.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Request {
    #[serde(default = "jsonrpc_version")]
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// A reply. Exactly one of `result`/`error` is set; `id` echoes the request,
/// and is absent on a daemon-initiated notification.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Response {
    #[serde(default = "jsonrpc_version")]
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

fn jsonrpc_version() -> String {
    "2.0".to_string()
}

/// A failure, carrying the exit code the same command would have produced
/// locally so a remote run exits exactly as a local one does.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErrorData {
    /// The `Exit` discriminant this failure maps to.
    pub exit: u8,
}

/// Application errors all share one code; the exit status in `data` is what a
/// caller branches on.
pub const CODE_APPLICATION: i32 = -32000;
pub const CODE_PARSE: i32 = -32700;
pub const CODE_INVALID_REQUEST: i32 = -32600;
pub const CODE_METHOD_NOT_FOUND: i32 = -32601;
pub const CODE_INVALID_PARAMS: i32 = -32602;

impl RpcError {
    #[must_use]
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// An application failure that should reproduce `exit` on the client.
    #[must_use]
    pub fn application(exit: Exit, message: impl Into<String>) -> Self {
        Self {
            code: CODE_APPLICATION,
            message: message.into(),
            data: Some(ErrorData { exit: exit.code() }),
        }
    }

    /// The exit status this error asks the client to produce, defaulting to a
    /// generic failure when the daemon did not say.
    #[must_use]
    pub fn exit(&self) -> Exit {
        match self.data.as_ref().map(|data| data.exit) {
            Some(0) => Exit::Ok,
            Some(2) => Exit::Usage,
            Some(3) => Exit::Connection,
            Some(4) => Exit::NoResults,
            Some(5) => Exit::Timeout,
            Some(6) => Exit::Transfer,
            Some(7) => Exit::SessionLost,
            _ => Exit::Failure,
        }
    }
}

impl Response {
    #[must_use]
    pub fn result(id: Option<serde_json::Value>, value: serde_json::Value) -> Self {
        Self {
            jsonrpc: jsonrpc_version(),
            id,
            method: None,
            params: None,
            result: Some(value),
            error: None,
        }
    }

    #[must_use]
    pub fn failure(id: Option<serde_json::Value>, error: RpcError) -> Self {
        Self {
            jsonrpc: jsonrpc_version(),
            id,
            method: None,
            params: None,
            result: None,
            error: Some(error),
        }
    }

    /// A daemon-initiated event: no id, so no reply is expected.
    #[must_use]
    pub fn notification(method: &str, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: jsonrpc_version(),
            id: None,
            method: Some(method.to_string()),
            params: Some(params),
            result: None,
            error: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Handshake
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct AuthParams {
    /// Required on TCP. Omitted on a Unix socket, where the socket's own
    /// permissions already prove the caller is this user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default)]
    pub protocol: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct AuthResult {
    pub protocol: u32,
    pub daemon_version: String,
    /// The Soulseek account the daemon is logged in as.
    pub username: String,
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct FileDto {
    pub name: String,
    pub size: u64,
    /// Soulseek attribute codes (0 = bitrate, 1 = duration in seconds).
    pub attribs: HashMap<u32, u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct SearchResultDto {
    pub token: u32,
    pub username: String,
    pub slots: u8,
    pub speed: u32,
    pub files: Vec<FileDto>,
}

impl From<&SearchResult> for SearchResultDto {
    fn from(result: &SearchResult) -> Self {
        Self {
            token: result.token,
            username: result.username.clone(),
            slots: result.slots,
            speed: result.speed,
            files: result
                .files
                .iter()
                .map(|file| FileDto {
                    name: file.name.clone(),
                    size: file.size,
                    attribs: file.attribs.clone(),
                })
                .collect(),
        }
    }
}

impl From<SearchResultDto> for SearchResult {
    fn from(dto: SearchResultDto) -> Self {
        Self {
            token: dto.token,
            slots: dto.slots,
            speed: dto.speed,
            files: dto
                .files
                .into_iter()
                .map(|file| File {
                    username: dto.username.clone(),
                    name: file.name,
                    size: file.size,
                    attribs: file.attribs,
                })
                .collect(),
            username: dto.username,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DownloadStatusDto {
    Queued,
    InProgress {
        bytes_downloaded: u64,
        total_bytes: u64,
        speed_bytes_per_sec: f64,
    },
    Paused {
        bytes_downloaded: u64,
        total_bytes: u64,
    },
    Completed,
    Failed {
        reason: Option<String>,
    },
    TimedOut,
}

impl From<&DownloadStatus> for DownloadStatusDto {
    fn from(status: &DownloadStatus) -> Self {
        match status {
            DownloadStatus::Queued => Self::Queued,
            DownloadStatus::InProgress {
                bytes_downloaded,
                total_bytes,
                speed_bytes_per_sec,
            } => Self::InProgress {
                bytes_downloaded: *bytes_downloaded,
                total_bytes: *total_bytes,
                speed_bytes_per_sec: *speed_bytes_per_sec,
            },
            DownloadStatus::Paused {
                bytes_downloaded,
                total_bytes,
            } => Self::Paused {
                bytes_downloaded: *bytes_downloaded,
                total_bytes: *total_bytes,
            },
            DownloadStatus::Completed => Self::Completed,
            DownloadStatus::Failed(reason) => Self::Failed {
                reason: reason.clone(),
            },
            DownloadStatus::TimedOut => Self::TimedOut,
        }
    }
}

impl From<DownloadStatusDto> for DownloadStatus {
    fn from(dto: DownloadStatusDto) -> Self {
        match dto {
            DownloadStatusDto::Queued => Self::Queued,
            DownloadStatusDto::InProgress {
                bytes_downloaded,
                total_bytes,
                speed_bytes_per_sec,
            } => Self::InProgress {
                bytes_downloaded,
                total_bytes,
                speed_bytes_per_sec,
            },
            DownloadStatusDto::Paused {
                bytes_downloaded,
                total_bytes,
            } => Self::Paused {
                bytes_downloaded,
                total_bytes,
            },
            DownloadStatusDto::Completed => Self::Completed,
            DownloadStatusDto::Failed { reason } => Self::Failed(reason),
            DownloadStatusDto::TimedOut => Self::TimedOut,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct DownloadMetadataDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_upload_speed: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_free_slots: Option<u8>,
}

impl From<&DownloadMetadata> for DownloadMetadataDto {
    fn from(metadata: &DownloadMetadata) -> Self {
        Self {
            bitrate: metadata.bitrate,
            length_seconds: metadata.length_seconds,
            peer_upload_speed: metadata.peer_upload_speed,
            peer_free_slots: metadata.peer_free_slots,
        }
    }
}

impl From<DownloadMetadataDto> for DownloadMetadata {
    fn from(dto: DownloadMetadataDto) -> Self {
        Self {
            bitrate: dto.bitrate,
            length_seconds: dto.length_seconds,
            peer_upload_speed: dto.peer_upload_speed,
            peer_free_slots: dto.peer_free_slots,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct DownloadDto {
    pub username: String,
    pub filename: String,
    pub token: u32,
    pub size: u64,
    pub download_directory: String,
    pub status: DownloadStatusDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<u32>,
    #[serde(default)]
    pub metadata: DownloadMetadataDto,
}

impl From<&Download> for DownloadDto {
    fn from(download: &Download) -> Self {
        Self {
            username: download.username.clone(),
            filename: download.filename.clone(),
            token: download.token,
            size: download.size,
            download_directory: download.download_directory.clone(),
            status: DownloadStatusDto::from(&download.status),
            queue_position: download.queue_position,
            metadata: DownloadMetadataDto::from(&download.metadata),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UploadStatusDto {
    /// Waiting for a slot, at this 1-based place in the queue.
    Queued { place: u32 },
    InProgress,
    Completed,
    Cancelled,
    Failed { reason: String },
}

impl From<&UploadStatus> for UploadStatusDto {
    fn from(status: &UploadStatus) -> Self {
        match status {
            UploadStatus::Queued(place) => Self::Queued { place: *place },
            UploadStatus::InProgress => Self::InProgress,
            UploadStatus::Completed => Self::Completed,
            UploadStatus::Cancelled => Self::Cancelled,
            UploadStatus::Failed(reason) => Self::Failed {
                reason: reason.clone(),
            },
        }
    }
}

impl From<UploadStatusDto> for UploadStatus {
    fn from(dto: UploadStatusDto) -> Self {
        match dto {
            UploadStatusDto::Queued { place } => Self::Queued(place),
            UploadStatusDto::InProgress => Self::InProgress,
            UploadStatusDto::Completed => Self::Completed,
            UploadStatusDto::Cancelled => Self::Cancelled,
            UploadStatusDto::Failed { reason } => Self::Failed(reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct UploadInfoDto {
    pub username: String,
    pub filename: String,
    pub size: u64,
    pub bytes_sent: u64,
    pub status: UploadStatusDto,
    pub speed_bytes_per_sec: f64,
}

impl From<&UploadInfo> for UploadInfoDto {
    fn from(upload: &UploadInfo) -> Self {
        Self {
            username: upload.username.clone(),
            filename: upload.filename.clone(),
            size: upload.size,
            bytes_sent: upload.bytes_sent,
            status: UploadStatusDto::from(&upload.status),
            speed_bytes_per_sec: upload.speed_bytes_per_sec,
        }
    }
}

impl From<UploadInfoDto> for UploadInfo {
    fn from(dto: UploadInfoDto) -> Self {
        Self {
            username: dto.username,
            filename: dto.filename,
            size: dto.size,
            bytes_sent: dto.bytes_sent,
            status: dto.status.into(),
            speed_bytes_per_sec: dto.speed_bytes_per_sec,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct RoomInfoDto {
    pub name: String,
    pub user_count: u32,
}

impl From<&RoomInfo> for RoomInfoDto {
    fn from(room: &RoomInfo) -> Self {
        Self {
            name: room.name.clone(),
            user_count: room.user_count,
        }
    }
}

impl From<RoomInfoDto> for RoomInfo {
    fn from(dto: RoomInfoDto) -> Self {
        Self {
            name: dto.name,
            user_count: dto.user_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RoomEventDto {
    List {
        rooms: Vec<RoomInfoDto>,
    },
    Joined {
        room: String,
        users: Vec<String>,
    },
    Left {
        room: String,
    },
    Message {
        room: String,
        username: String,
        message: String,
    },
    UserJoined {
        room: String,
        username: String,
    },
    UserLeft {
        room: String,
        username: String,
    },
}

impl From<&RoomEvent> for RoomEventDto {
    fn from(event: &RoomEvent) -> Self {
        match event {
            RoomEvent::List(rooms) => Self::List {
                rooms: rooms.iter().map(RoomInfoDto::from).collect(),
            },
            RoomEvent::Joined { room, users } => Self::Joined {
                room: room.clone(),
                users: users.clone(),
            },
            RoomEvent::Left { room } => Self::Left { room: room.clone() },
            RoomEvent::Message {
                room,
                username,
                message,
            } => Self::Message {
                room: room.clone(),
                username: username.clone(),
                message: message.clone(),
            },
            RoomEvent::UserJoined { room, username } => Self::UserJoined {
                room: room.clone(),
                username: username.clone(),
            },
            RoomEvent::UserLeft { room, username } => Self::UserLeft {
                room: room.clone(),
                username: username.clone(),
            },
        }
    }
}

impl From<RoomEventDto> for RoomEvent {
    fn from(dto: RoomEventDto) -> Self {
        match dto {
            RoomEventDto::List { rooms } => {
                Self::List(rooms.into_iter().map(RoomInfo::from).collect())
            }
            RoomEventDto::Joined { room, users } => Self::Joined { room, users },
            RoomEventDto::Left { room } => Self::Left { room },
            RoomEventDto::Message {
                room,
                username,
                message,
            } => Self::Message {
                room,
                username,
                message,
            },
            RoomEventDto::UserJoined { room, username } => {
                Self::UserJoined { room, username }
            }
            RoomEventDto::UserLeft { room, username } => {
                Self::UserLeft { room, username }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct UserPresenceDto {
    /// `online`, `away`, or `offline`.
    pub status: String,
    pub privileged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct UserStatsDto {
    pub average_speed: u32,
    pub shared_files: u32,
    pub shared_folders: u32,
}

/// Each half is `None` until its reply lands, so a consumer can tell "the
/// server said offline" from "the server has not answered yet".
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct UserInfoDto {
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<UserPresenceDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<UserStatsDto>,
}

impl From<&UserInfo> for UserInfoDto {
    fn from(info: &UserInfo) -> Self {
        Self {
            username: info.username.clone(),
            presence: info.presence.map(|presence| UserPresenceDto {
                status: presence.status.to_string(),
                privileged: presence.privileged,
            }),
            stats: info.stats.map(|stats| UserStatsDto {
                average_speed: stats.average_speed,
                shared_files: stats.shared_files,
                shared_folders: stats.shared_folders,
            }),
        }
    }
}

impl From<UserInfoDto> for UserInfo {
    fn from(dto: UserInfoDto) -> Self {
        let mut info = Self::pending(dto.username);
        info.presence = dto.presence.map(|presence| UserPresence {
            status: match presence.status.as_str() {
                "online" => UserStatus::Online,
                "away" => UserStatus::Away,
                _ => UserStatus::Offline,
            },
            privileged: presence.privileged,
        });
        info.stats = dto.stats.map(|stats| UserStats {
            average_speed: stats.average_speed,
            shared_files: stats.shared_files,
            shared_folders: stats.shared_folders,
        });
        info
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct SharedDirectoryDto {
    pub name: String,
    /// Basename and size; join with the directory to get a downloadable path.
    pub files: Vec<(String, u64)>,
}

impl From<&SharedDirectory> for SharedDirectoryDto {
    fn from(directory: &SharedDirectory) -> Self {
        Self {
            name: directory.name.clone(),
            files: directory.files.clone(),
        }
    }
}

impl From<SharedDirectoryDto> for SharedDirectory {
    fn from(dto: SharedDirectoryDto) -> Self {
        Self {
            name: dto.name,
            files: dto.files,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct UserMessageDto {
    pub id: u32,
    pub timestamp: u32,
    pub username: String,
    pub message: String,
    pub new_message: bool,
}

impl From<&UserMessage> for UserMessageDto {
    fn from(message: &UserMessage) -> Self {
        Self {
            id: message.id(),
            timestamp: message.timestamp(),
            username: message.username().to_string(),
            message: message.message().to_string(),
            new_message: message.is_new(),
        }
    }
}

impl From<UserMessageDto> for UserMessage {
    fn from(dto: UserMessageDto) -> Self {
        Self::new(
            dto.id,
            dto.timestamp,
            dto.username,
            dto.message,
            dto.new_message,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SessionLossDto {
    Displaced,
    Disconnected,
}

impl From<SessionLoss> for SessionLossDto {
    fn from(loss: SessionLoss) -> Self {
        match loss {
            SessionLoss::Displaced => Self::Displaced,
            SessionLoss::Disconnected => Self::Disconnected,
        }
    }
}

impl From<SessionLossDto> for SessionLoss {
    fn from(dto: SessionLossDto) -> Self {
        match dto {
            SessionLossDto::Displaced => Self::Displaced,
            SessionLossDto::Disconnected => Self::Disconnected,
        }
    }
}

/// What the daemon is and what its session is doing, from `daemon.status`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct DaemonStatus {
    pub username: String,
    pub server: String,
    pub daemon_version: String,
    pub protocol: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_port: Option<u16>,
    pub shared_folders: u32,
    pub shared_files: u32,
    pub download_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_loss: Option<SessionLossDto>,
    /// Control connections currently attached.
    pub clients: usize,
    pub uptime_secs: u64,
}

/// What the share index holds, from `shares.status` / `shares.reindex`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct SharesStatus {
    pub folders: u32,
    pub files: u32,
    pub directories: Vec<String>,
}

// ---------------------------------------------------------------------------
// Method parameters
// ---------------------------------------------------------------------------

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
    /// Where the file lands *on the daemon's* filesystem. Defaults to the
    /// daemon's configured download directory, which is the only sensible
    /// choice for a remote caller that cannot see that filesystem.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_directory: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Messages {
    pub messages: Vec<UserMessageDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct UserResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<UserInfoDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Seconds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct IntervalSeconds {
    pub seconds: u64,
}

// ---------------------------------------------------------------------------
// Event notifications
// ---------------------------------------------------------------------------

/// Declare a wire vocabulary once: the enum, the complete list, and the
/// name mapping in both directions.
///
/// Everything that consumes one of these matches on the enum, so the match is
/// exhaustive and adding a line here is a compile error everywhere the new
/// member still has to be handled — the dispatcher, the published schema, the
/// remote client. A `&[&str]` table checked by a test would only fail after
/// the fact, and only if someone remembered to write the test.
macro_rules! wire_vocabulary {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $wire:literal),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant),*
        }

        impl $name {
            /// Every member, in published order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),*];

            #[must_use]
            pub const fn wire_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),*
                }
            }

            #[must_use]
            pub fn from_wire(name: &str) -> Option<Self> {
                match name {
                    $($wire => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.wire_name())
            }
        }
    };
}

wire_vocabulary! {
    /// Every method the daemon answers.
    Method {
        Auth => "auth",
        DaemonStatus => "daemon.status",
        DaemonStop => "daemon.stop",
        RpcDiscover => "rpc.discover",
        SearchStart => "search.start",
        SearchResults => "search.results",
        SearchWishlist => "search.wishlist",
        SearchWishlistInterval => "search.wishlist_interval",
        DownloadStart => "download.start",
        DownloadList => "download.list",
        DownloadPause => "download.pause",
        DownloadResume => "download.resume",
        DownloadRemove => "download.remove",
        DownloadRemoveQueued => "download.remove_queued",
        UploadList => "upload.list",
        UploadCancel => "upload.cancel",
        UploadSlots => "upload.slots",
        PrivilegesCheck => "privileges.check",
        PrivilegesOwn => "privileges.own",
        RoomListRequest => "room.list_request",
        RoomJoin => "room.join",
        RoomLeave => "room.leave",
        RoomSay => "room.say",
        RoomMembers => "room.members",
        MessageSend => "message.send",
        MessageHistory => "message.history",
        BrowseUser => "browse.user",
        UserRequest => "user.request",
        UserInfoOf => "user.info",
        SharesStatusOf => "shares.status",
        SharesSet => "shares.set",
        SharesReindex => "shares.reindex",
    }
}

wire_vocabulary! {
    /// Events the daemon pushes without being asked.
    ///
    /// These exist because the library's `take_*` accessors are destructive:
    /// with two clients attached, whichever drained first would steal the
    /// others' events. The daemon is the single drainer and fans out.
    Event {
        Room => "event.room",
        Message => "event.message",
        Upload => "event.upload",
        DownloadStatus => "event.download_status",
        Browse => "event.browse",
        SessionLoss => "event.session_loss",
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_result_survives_the_round_trip() {
        let mut attribs = HashMap::new();
        attribs.insert(0, 320);
        attribs.insert(1, 210);
        let original = SearchResult {
            token: 7,
            slots: 2,
            speed: 900,
            username: "bob".into(),
            files: vec![File {
                username: "bob".into(),
                name: "@@x\\a.mp3".into(),
                size: 4096,
                attribs,
            }],
        };

        let json = serde_json::to_string(&SearchResultDto::from(&original))
            .expect("a DTO serializes");
        let back: SearchResult =
            serde_json::from_str::<SearchResultDto>(&json).unwrap().into();

        assert_eq!(back.token, original.token);
        assert_eq!(back.username, original.username);
        assert_eq!(back.slots, original.slots);
        assert_eq!(back.speed, original.speed);
        assert_eq!(back.files.len(), 1);
        assert_eq!(back.files[0].name, "@@x\\a.mp3");
        assert_eq!(back.files[0].size, 4096);
        assert_eq!(back.files[0].attribs.get(&0), Some(&320));
        // The per-file username is carried by the parent result, so it has to
        // be restored rather than lost.
        assert_eq!(back.files[0].username, "bob");
    }

    #[test]
    fn every_download_status_survives_the_round_trip() {
        let statuses = [
            DownloadStatus::Queued,
            DownloadStatus::InProgress {
                bytes_downloaded: 10,
                total_bytes: 100,
                speed_bytes_per_sec: 1.5,
            },
            DownloadStatus::Paused {
                bytes_downloaded: 10,
                total_bytes: 100,
            },
            DownloadStatus::Completed,
            DownloadStatus::Failed(Some("gone".into())),
            DownloadStatus::Failed(None),
            DownloadStatus::TimedOut,
        ];
        for status in &statuses {
            let dto = DownloadStatusDto::from(status);
            let json = serde_json::to_string(&dto).unwrap();
            let back: DownloadStatusDto = serde_json::from_str(&json).unwrap();
            assert_eq!(dto, back, "{status:?} must survive the wire");
            assert_eq!(
                DownloadStatusDto::from(&DownloadStatus::from(back)),
                dto,
                "{status:?} must convert back to the same library value"
            );
        }
    }

    #[test]
    fn every_upload_status_survives_the_round_trip() {
        let statuses = [
            UploadStatus::Queued(3),
            UploadStatus::InProgress,
            UploadStatus::Completed,
            UploadStatus::Cancelled,
            UploadStatus::Failed("nope".into()),
        ];
        for status in &statuses {
            let dto = UploadStatusDto::from(status);
            let json = serde_json::to_string(&dto).unwrap();
            let back: UploadStatus =
                serde_json::from_str::<UploadStatusDto>(&json).unwrap().into();
            assert_eq!(&back, status);
        }
    }

    #[test]
    fn every_room_event_survives_the_round_trip() {
        let events = [
            RoomEvent::List(vec![RoomInfo {
                name: "lobby".into(),
                user_count: 42,
            }]),
            RoomEvent::Joined {
                room: "lobby".into(),
                users: vec!["bob".into()],
            },
            RoomEvent::Left {
                room: "lobby".into(),
            },
            RoomEvent::Message {
                room: "lobby".into(),
                username: "bob".into(),
                message: "hi".into(),
            },
            RoomEvent::UserJoined {
                room: "lobby".into(),
                username: "sue".into(),
            },
            RoomEvent::UserLeft {
                room: "lobby".into(),
                username: "sue".into(),
            },
        ];
        for event in &events {
            let json = serde_json::to_string(&RoomEventDto::from(event)).unwrap();
            let back: RoomEvent =
                serde_json::from_str::<RoomEventDto>(&json).unwrap().into();
            assert_eq!(&back, event);
        }
    }

    #[test]
    fn a_partly_answered_user_stays_partly_answered() {
        // "the server has not said" must not arrive as "offline, shares
        // nothing": that is the difference between waiting and giving up.
        let pending = UserInfo::pending("bob".into());
        let json = serde_json::to_string(&UserInfoDto::from(&pending)).unwrap();
        let back: UserInfo =
            serde_json::from_str::<UserInfoDto>(&json).unwrap().into();
        assert_eq!(back.username, "bob");
        assert!(back.presence.is_none());
        assert!(back.stats.is_none());
        assert!(!back.is_complete());
    }

    #[test]
    fn a_complete_user_survives_the_round_trip() {
        let mut info = UserInfo::pending("bob".into());
        info.presence = Some(UserPresence {
            status: UserStatus::Away,
            privileged: true,
        });
        info.stats = Some(UserStats {
            average_speed: 500,
            shared_files: 10,
            shared_folders: 2,
        });

        let json = serde_json::to_string(&UserInfoDto::from(&info)).unwrap();
        let back: UserInfo =
            serde_json::from_str::<UserInfoDto>(&json).unwrap().into();
        assert_eq!(back, info);
    }

    #[test]
    fn a_private_message_survives_the_round_trip() {
        let message = UserMessage::new(9, 1234, "bob".into(), "hi".into(), true);
        let json = serde_json::to_string(&UserMessageDto::from(&message)).unwrap();
        let back: UserMessage =
            serde_json::from_str::<UserMessageDto>(&json).unwrap().into();
        assert_eq!(back.id(), 9);
        assert_eq!(back.timestamp(), 1234);
        assert_eq!(back.username(), "bob");
        assert_eq!(back.message(), "hi");
    }

    #[test]
    fn a_shared_listing_survives_the_round_trip() {
        let directory = SharedDirectory {
            name: "@@x\\Music".into(),
            files: vec![("a.mp3".into(), 4096)],
        };
        let json =
            serde_json::to_string(&SharedDirectoryDto::from(&directory)).unwrap();
        let back: SharedDirectory =
            serde_json::from_str::<SharedDirectoryDto>(&json).unwrap().into();
        assert_eq!(back.name, directory.name);
        assert_eq!(back.files, directory.files);
    }

    #[test]
    fn an_error_carries_the_exit_code_a_local_run_would_have_produced() {
        for exit in [
            Exit::Usage,
            Exit::Connection,
            Exit::NoResults,
            Exit::Timeout,
            Exit::Transfer,
            Exit::SessionLost,
        ] {
            let error = RpcError::application(exit, "nope");
            let json = serde_json::to_string(&error).unwrap();
            let back: RpcError = serde_json::from_str(&json).unwrap();
            assert_eq!(back.exit(), exit);
        }
    }

    #[test]
    fn an_error_without_an_exit_code_is_a_generic_failure() {
        let error = RpcError::new(CODE_METHOD_NOT_FOUND, "no such method");
        assert_eq!(error.exit(), Exit::Failure);
    }

    #[test]
    fn a_request_parses_without_optional_fields() {
        let request: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"daemon.status"}"#)
                .expect("params and id are optional");
        assert_eq!(request.method, "daemon.status");
        assert!(request.id.is_none());
        assert!(request.params.is_null());
    }

    #[test]
    fn a_notification_carries_no_id_so_no_reply_is_expected() {
        let notification =
            Response::notification("event.room", serde_json::json!({"a": 1}));
        let json = serde_json::to_string(&notification).unwrap();
        assert!(!json.contains("\"id\""), "{json}");
        assert!(json.contains("\"method\":\"event.room\""), "{json}");
    }

    #[test]
    fn a_response_serializes_exactly_one_of_result_and_error() {
        let ok = serde_json::to_string(&Response::result(
            Some(serde_json::json!(1)),
            serde_json::json!({}),
        ))
        .unwrap();
        assert!(ok.contains("\"result\""));
        assert!(!ok.contains("\"error\""));

        let failed = serde_json::to_string(&Response::failure(
            Some(serde_json::json!(1)),
            RpcError::application(Exit::NoResults, "nothing"),
        ))
        .unwrap();
        assert!(failed.contains("\"error\""));
        assert!(!failed.contains("\"result\""));
    }
}
