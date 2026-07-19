//! `SharedFileListResponse` (peer code 5): a peer's full shared-file listing,
//! sent in reply to `GetShareFileList` (code 4). The payload is zlib-compressed
//! and groups files by their virtual directory.

use crate::message::{Message, MessageHandler};
use crate::peer::PeerMessage;
use crate::utils::zlib::{compress_stored, deflate};
use std::sync::mpsc::Sender;

/// One shared directory and the files directly in it (basename + size).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedDirectory {
    pub name: String,
    pub files: Vec<(String, u64)>,
}

/// Receives a peer's `SharedFileListResponse` (peer code 5) when browsing them.
pub struct SharedFileListResponseHandler;
impl MessageHandler<PeerMessage> for SharedFileListResponseHandler {
    fn get_code(&self) -> u8 {
        5
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let directories = parse_shared_file_list(message);
        let _ = sender.send(PeerMessage::ShareListReceived(directories));
    }
}

/// Build a `SharedFileListResponse` (peer code 5) from the directory listing.
#[must_use]
pub fn build_shared_file_list(dirs: &[SharedDirectory]) -> Message {
    let mut payload = Message::new();
    payload.write_int32(dirs.len() as u32);
    for dir in dirs {
        payload
            .write_string(&dir.name)
            .write_int32(dir.files.len() as u32);
        for (name, size) in &dir.files {
            payload
                .write_int8(1)
                .write_string(name)
                .write_int64(*size)
                .write_string("") // extension
                .write_int32(0); // attribute count
        }
    }
    payload.write_int32(0); // unknown
    payload.write_int32(0); // number of private directories

    let compressed = compress_stored(&payload.get_data());
    Message::new()
        .write_int32(5)
        .write_raw_bytes(compressed)
        .clone()
}

/// Parse the (zlib-compressed) `SharedFileListResponse` payload. `message` must
/// be positioned at the compressed blob (the dispatcher sets pointer 8).
///
/// Returns an empty listing if the payload is malformed.
#[must_use]
pub fn parse_shared_file_list(message: &mut Message) -> Vec<SharedDirectory> {
    let pointer = message.get_pointer();
    let size = message.get_size();
    let compressed = message.get_slice(pointer, size);
    let Ok(data) = deflate(&compressed) else {
        return Vec::new();
    };

    let mut body = Message::new_with_data(data);
    let dir_count = body.read_int32();
    let mut dirs = Vec::new();
    for _ in 0..dir_count {
        // Stop if a hostile count outruns the (decompressed) payload, so a
        // bogus length can't spin us into a huge allocation loop.
        if body.get_pointer() >= body.get_size() {
            break;
        }
        let name = body.read_string();
        let file_count = body.read_int32();
        let mut files = Vec::new();
        for _ in 0..file_count {
            if body.get_pointer() >= body.get_size() {
                break;
            }
            body.read_int8(); // code
            let filename = body.read_string();
            let file_size = body.read_int64();
            body.read_string(); // extension
            let attr_count = body.read_int32();
            for _ in 0..attr_count {
                // Each attribute is two int32s (8 bytes); read_int32 does not
                // advance past the end, so bound the loop explicitly.
                if body.get_pointer() + 8 > body.get_size() {
                    break;
                }
                body.read_int32();
                body.read_int32();
            }
            files.push((filename, file_size));
        }
        dirs.push(SharedDirectory { name, files });
    }
    dirs
}

#[test]
fn hostile_dir_count_does_not_hang() {
    // A compressed body claiming ~4 billion directories with no data must
    // parse to empty promptly rather than looping into an OOM.
    let compressed =
        crate::utils::zlib::compress_stored(&u32::MAX.to_le_bytes());
    let mut message = Message::new();
    message.write_raw_bytes(vec![0u8; 8]);
    message.write_raw_bytes(compressed);
    message.set_pointer(8);
    assert!(parse_shared_file_list(&mut message).is_empty());
}

#[test]
fn shared_file_list_roundtrips() {
    let dirs = vec![
        SharedDirectory {
            name: "music\\album".to_string(),
            files: vec![
                ("song one.flac".to_string(), 123),
                ("song two.flac".to_string(), 456),
            ],
        },
        SharedDirectory {
            name: "music".to_string(),
            files: vec![("top.mp3".to_string(), 789)],
        },
    ];
    let message = build_shared_file_list(&dirs);

    // Decode via the same offset the dispatcher would use.
    let mut decoded = Message::new_with_data(message.get_buffer());
    decoded.set_pointer(8);
    assert_eq!(parse_shared_file_list(&mut decoded), dirs);
}
