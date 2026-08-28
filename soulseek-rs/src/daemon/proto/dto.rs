//! The data types that cross the wire.
//!
//! Library types never travel as themselves. Each has an explicit DTO and a
//! mechanical conversion, so the published shape is versioned independently of
//! `soulseek-rs-lib` — a refactor there cannot silently change what a
//! third-party script receives, and the compiler points here when one would.

use serde::{Deserialize, Serialize};
use soulseek_rs::types::{
    Download, DownloadMetadata, RoomUserStats, UserPresence, UserStats,
    UserStatus,
};
use soulseek_rs::{
    DownloadStatus, File, RoomEvent, RoomInfo, SearchResult, SessionLoss,
    SharedDirectory, UploadInfo, UploadStatus, UserInfo, UserMessage,
};
use std::collections::HashMap;

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
    #[serde(default)]
    pub bitrate: Option<u32>,
    #[serde(default)]
    pub length_seconds: Option<u32>,
    #[serde(default)]
    pub peer_upload_speed: Option<u32>,
    #[serde(default)]
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
    #[serde(default)]
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
    Queued {
        place: u32,
    },
    InProgress,
    Completed,
    Cancelled,
    Failed {
        reason: String,
    },
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
            RoomEventDto::Joined { room, users } => {
                Self::Joined { room, users }
            }
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

/// The inverse of [`UserStatus`]'s `Display`. An unrecognised word reads as
/// offline, matching how the wire code is decoded.
fn status_from_str(status: &str) -> UserStatus {
    match status {
        "online" => UserStatus::Online,
        "away" => UserStatus::Away,
        _ => UserStatus::Offline,
    }
}

/// One member of a room, as the JoinRoom reply described them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct RoomUserStatsDto {
    pub username: String,
    /// `online`, `away`, or `offline`.
    pub status: String,
    pub average_speed: u32,
    pub shared_files: u32,
    pub shared_folders: u32,
    pub slots_free: u32,
    pub country: Option<String>,
}

impl From<&RoomUserStats> for RoomUserStatsDto {
    fn from(stats: &RoomUserStats) -> Self {
        Self {
            username: stats.username.clone(),
            status: stats.status.to_string(),
            average_speed: stats.average_speed,
            shared_files: stats.shared_files,
            shared_folders: stats.shared_folders,
            slots_free: stats.slots_free,
            country: stats.country.clone(),
        }
    }
}

impl From<RoomUserStatsDto> for RoomUserStats {
    fn from(dto: RoomUserStatsDto) -> Self {
        Self {
            username: dto.username,
            status: status_from_str(&dto.status),
            average_speed: dto.average_speed,
            shared_files: dto.shared_files,
            shared_folders: dto.shared_folders,
            slots_free: dto.slots_free,
            country: dto.country,
        }
    }
}

/// Each half is `None` until its reply lands, so a consumer can tell "the
/// server said offline" from "the server has not answered yet".
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct UserInfoDto {
    pub username: String,
    #[serde(default)]
    pub presence: Option<UserPresenceDto>,
    #[serde(default)]
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
            status: status_from_str(&presence.status),
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
    #[serde(default)]
    pub listen_port: Option<u16>,
    pub shared_folders: u32,
    pub shared_files: u32,
    pub download_dir: String,
    #[serde(default)]
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
