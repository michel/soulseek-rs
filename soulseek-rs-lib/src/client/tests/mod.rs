//! The client's context, exercised directly.
//!
//! This file is the harness the topic modules below share.

mod peers;
mod rooms;
mod search;
mod transfers;
mod users;

use super::*;

fn download(
    username: &str,
    filename: &str,
    token: u32,
    status: DownloadStatus,
    sender: Sender<DownloadStatus>,
) -> Download {
    Download {
        username: username.to_string(),
        filename: filename.to_string(),
        token,
        size: 100,
        download_directory: "test".to_string(),
        status,
        sender,
        queue_position: None,
        metadata: DownloadMetadata::default(),
    }
}

fn peer_files(count: usize) -> SearchResult {
    SearchResult {
        token: 1,
        files: (0..count)
            .map(|i| crate::types::File {
                username: "bob".to_string(),
                name: format!("song-{i}.mp3"),
                size: 1,
                attribs: std::collections::HashMap::new(),
            })
            .collect(),
        slots: 1,
        speed: 0,
        username: "bob".to_string(),
    }
}

use crate::types::{RoomEvent, UploadStatus};

use std::time::{Duration, Instant};
