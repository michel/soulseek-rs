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
pub struct UploadFailed {
    pub filename: String,
}
impl UploadFailed {
    pub fn new_from_message(message: &mut Message) -> Self {
        let filename = message.read_string();

        Self { filename }
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
