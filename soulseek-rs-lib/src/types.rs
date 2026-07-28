use std::{collections::HashMap, sync::mpsc::Sender};

use crate::{error::Result, message::Message, utils::zlib::deflate};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct File {
    pub username: String,
    pub name: String,
    pub size: u64,
    pub attribs: HashMap<u32, u32>,
}
/// The client version sent to the server on login.
///
/// Major versions identify client projects on the Soulseek network and are
/// reserved per project for the project's lifetime:
/// <https://nicotine-plus.org/doc/SLSKPROTOCOL.html#major-versions>.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientVersion {
    pub major: u32,
    pub minor: u32,
}

impl ClientVersion {
    /// The major version reserved for the soulseek-rs project. Clients and
    /// bots built on this library keep this major and pick their own non-zero
    /// minor version.
    pub const MAJOR: u32 = 176;

    /// The version the soulseek-rs reference client (CLI/TUI) logs in with.
    pub const REFERENCE_CLIENT: Self = Self {
        major: Self::MAJOR,
        minor: 100,
    };
}

impl Default for ClientVersion {
    fn default() -> Self {
        Self {
            major: Self::MAJOR,
            minor: 1,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SearchResult {
    pub token: u32,
    pub files: Vec<File>,
    pub slots: u8,
    pub speed: u32,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct Search {
    pub token: u32,
    pub results: Vec<SearchResult>,
}

impl SearchResult {
    pub fn new_from_message(message: &mut Message) -> Result<Self> {
        let pointer = message.get_pointer();
        let size = message.get_size();
        let data: Vec<u8> = message.get_slice(pointer, size);
        let deflated = deflate(&data)?;
        let mut message = Message::new_with_data(deflated);

        let username = message.read_string();
        let token = message.read_int32();
        let n_files = message.read_int32();
        let mut files: Vec<File> = Vec::new();
        for _ in 0..n_files {
            // Stop if a hostile n_files count outruns the payload, so a bogus
            // length can't spin us into a huge allocation loop.
            if message.get_pointer() >= message.get_size() {
                break;
            }
            message.read_int8();
            let name = message.read_string();
            let size = message.read_int64();
            message.read_string();
            let n_attribs = message.read_int32();
            let mut attribs: HashMap<u32, u32> = HashMap::new();

            for _ in 0..n_attribs {
                // Each attribute is two int32s (8 bytes); guard against a bogus
                // count since read_int32 does not advance past the buffer end.
                if message.get_pointer() + 8 > message.get_size() {
                    break;
                }
                attribs.insert(message.read_int32(), message.read_int32());
            }
            files.push(File {
                username: username.clone(),
                name,
                size,
                attribs,
            });
        }
        let slots = message.read_int8();
        let speed = message.read_int32();

        Ok(Self {
            token,
            files,
            slots,
            speed,
            username,
        })
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Transfer {
    pub direction: u32,
    pub token: u32,
    pub filename: String,
    pub size: u64,
}
#[derive(Debug, Clone, Default)]
pub struct DownloadMetadata {
    pub bitrate: Option<u32>,
    pub length_seconds: Option<u32>,
    pub peer_upload_speed: Option<u32>,
    pub peer_free_slots: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct Download {
    pub username: String,
    pub filename: String,
    pub token: u32,
    pub size: u64,
    pub download_directory: String,
    pub status: DownloadStatus,
    pub sender: Sender<DownloadStatus>,
    pub queue_position: Option<u32>,
    pub metadata: DownloadMetadata,
}

impl Download {
    #[must_use]
    pub const fn is_finished(&self) -> bool {
        matches!(
            self.status,
            DownloadStatus::Completed
                | DownloadStatus::Failed(_)
                | DownloadStatus::TimedOut
        )
    }

    #[must_use]
    pub const fn bytes_downloaded(&self) -> u64 {
        match &self.status {
            DownloadStatus::InProgress {
                bytes_downloaded, ..
            }
            | DownloadStatus::Paused {
                bytes_downloaded, ..
            } => *bytes_downloaded,
            DownloadStatus::Completed => self.size,
            _ => 0,
        }
    }

    #[must_use]
    pub const fn speed_bytes_per_sec(&self) -> f64 {
        match &self.status {
            DownloadStatus::InProgress {
                speed_bytes_per_sec,
                ..
            } => *speed_bytes_per_sec,
            _ => 0.0,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum DownloadStatus {
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
    /// Failed, optionally with a human-readable reason.
    Failed(Option<String>),
    TimedOut,
}

/// Whether a user is reachable, as the server reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UserStatus {
    #[default]
    Offline,
    Away,
    Online,
}

impl UserStatus {
    /// Map the wire value (`GetUserStatus`, code 7). Anything unrecognised is
    /// treated as offline, which is the safe reading for "cannot be reached".
    #[must_use]
    pub const fn from_code(code: u32) -> Self {
        match code {
            1 => Self::Away,
            2 => Self::Online,
            _ => Self::Offline,
        }
    }

    /// True when the user can be reached — online or away, but not offline.
    #[must_use]
    pub const fn is_reachable(self) -> bool {
        matches!(self, Self::Away | Self::Online)
    }
}

impl std::fmt::Display for UserStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Offline => "offline",
            Self::Away => "away",
            Self::Online => "online",
        })
    }
}

/// A user's presence, from `GetUserStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserPresence {
    pub status: UserStatus,
    /// Whether the server grants this user queue priority.
    pub privileged: bool,
}

/// A user's sharing statistics, from `GetUserStats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserStats {
    /// Average upload speed in bytes per second, as the server records it.
    pub average_speed: u32,
    pub shared_files: u32,
    pub shared_folders: u32,
}

/// What the server knows about another user.
///
/// The server answers presence and statistics as two separate messages, so
/// each part is `None` until its reply lands. A snapshot therefore cannot
/// report a status the server never sent — ask for
/// [`UserInfo::presence`] and handle `None` rather than reading a default.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UserInfo {
    pub username: String,
    /// `None` until the status reply arrives.
    pub presence: Option<UserPresence>,
    /// `None` until the statistics reply arrives.
    pub stats: Option<UserStats>,
}

impl UserInfo {
    /// An empty snapshot for `username`, before any reply has arrived.
    #[must_use]
    pub const fn pending(username: String) -> Self {
        Self {
            username,
            presence: None,
            stats: None,
        }
    }

    /// True once both replies have landed.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.presence.is_some() && self.stats.is_some()
    }
}

/// A public chat room advertised by the server (`RoomList`, code 64).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomInfo {
    pub name: String,
    pub user_count: u32,
}

/// Something that happened in the chat-room subsystem, surfaced to the client
/// so a UI can react to it. Drained via `Client::take_room_events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomEvent {
    /// The full list of public rooms (supersedes any previous snapshot).
    List(Vec<RoomInfo>),
    /// We successfully joined `room`; carries the current member list.
    Joined { room: String, users: Vec<String> },
    /// We left `room`.
    Left { room: String },
    /// `username` said `message` in `room`.
    Message {
        room: String,
        username: String,
        message: String,
    },
    /// `username` joined `room`.
    UserJoined { room: String, username: String },
    /// `username` left `room`.
    UserLeft { room: String, username: String },
}

impl Transfer {
    pub fn new_from_message(message: &mut Message) -> Self {
        let direction = message.read_int32();
        let token = message.read_int32();
        let filename = message.read_string();
        let size = message.read_int64();

        Self {
            direction,
            token,
            filename,
            size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A FileSearchResponse whose n_files claims ~4 billion entries with no
    // file data must parse to an empty result promptly, not loop into an OOM.
    #[test]
    fn search_result_hostile_file_count_does_not_hang() {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_le_bytes()); // username "" (len 0)
        body.extend_from_slice(&7u32.to_le_bytes()); // token
        body.extend_from_slice(&u32::MAX.to_le_bytes()); // n_files (hostile)
        let compressed = crate::utils::zlib::compress_stored(&body);
        let mut message = Message::new_with_data(compressed);
        let result = SearchResult::new_from_message(&mut message)
            .expect("hostile count should parse, not error");
        assert_eq!(result.token, 7);
        assert!(result.files.is_empty());
    }

    // A truncated TransferRequest from an untrusted peer must parse to defaults
    // rather than panic (the read_* primitives are bounds-checked).
    #[test]
    fn transfer_new_from_truncated_message_does_not_panic() {
        let mut message = Message::new_with_data(vec![1, 0, 0]);
        let transfer = Transfer::new_from_message(&mut message);
        assert_eq!(transfer.direction, 0);
        assert_eq!(transfer.token, 0);
        assert_eq!(transfer.filename, "");
        assert_eq!(transfer.size, 0);
    }
}

/// Lifecycle of a file we are serving to a peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadStatus {
    /// Waiting for a free upload slot, at this 1-based place in the queue.
    Queued(u32),
    InProgress,
    Completed,
    Cancelled,
    Failed(String),
}

/// A snapshot of one upload for display purposes.
#[derive(Debug, Clone)]
pub struct UploadInfo {
    /// The peer receiving the file.
    pub username: String,
    /// The peer-facing virtual path being served.
    pub filename: String,
    pub size: u64,
    pub bytes_sent: u64,
    pub status: UploadStatus,
    /// Transfer rate, so an upload row can fill the same Speed column a
    /// download does. Zero unless the upload is in progress.
    pub speed_bytes_per_sec: f64,
}

/// Why a server session ended before the client was done with it.
///
/// A session that ends stops seeing the network entirely, which is not the
/// same thing as the network having nothing to show — telling the two apart is
/// the difference between "retry" and "give up".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SessionLoss {
    /// Another login claimed this username. The server keeps one session per
    /// name, so the older one is cut off.
    Displaced = 1,
    /// The connection to the server dropped.
    Disconnected = 2,
}

impl std::fmt::Display for SessionLoss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Displaced => f.write_str(
                "another login took over this username; the server allows one \
                 session per account",
            ),
            Self::Disconnected => {
                f.write_str("the connection to the server dropped")
            }
        }
    }
}

/// A lock-free view, shared with the server actor, of whether the session is
/// still alive.
///
/// The first loss recorded wins: a displaced session also drops its socket a
/// moment later, and "displaced" is the reason worth reporting.
#[derive(Debug, Clone, Default)]
pub struct SessionWatch(std::sync::Arc<std::sync::atomic::AtomicU8>);

impl SessionWatch {
    const LIVE: u8 = 0;

    pub fn record(&self, loss: SessionLoss) {
        let _ = self.0.compare_exchange(
            Self::LIVE,
            loss as u8,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    #[must_use]
    pub fn loss(&self) -> Option<SessionLoss> {
        match self.0.load(std::sync::atomic::Ordering::Relaxed) {
            1 => Some(SessionLoss::Displaced),
            2 => Some(SessionLoss::Disconnected),
            _ => None,
        }
    }
}
