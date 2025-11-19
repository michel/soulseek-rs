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
            message.read_int8();
            let name = message.read_string();
            let size = message.read_int64();
            message.read_string();
            let n_attribs = message.read_int32();
            let mut attribs: HashMap<u32, u32> = HashMap::new();

            for _ in 0..n_attribs {
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
#[derive(Debug, Clone)]
pub struct Download {
    pub username: String,
    pub filename: String,
    pub token: u32,
    pub size: u64,
    pub download_directory: String,
    pub status: DownloadStatus,
    pub sender: Sender<DownloadStatus>,
}

impl Download {
    pub fn is_finished(&self) -> bool {
        matches!(
            self.status,
            DownloadStatus::Completed
                | DownloadStatus::Failed
                | DownloadStatus::TimedOut
        )
    }

    pub fn bytes_downloaded(&self) -> u64 {
        match &self.status {
            DownloadStatus::InProgress {
                bytes_downloaded, ..
            } => *bytes_downloaded,
            DownloadStatus::Completed => self.size,
            _ => 0,
        }
    }

    pub fn speed_bytes_per_sec(&self) -> f64 {
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
    Completed,
    Failed,
    TimedOut,
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
