//! `SharedFileListResponse` (peer code 5): a peer's full shared-file listing,
//! sent in reply to `GetShareFileList` (code 4). The payload is zlib-compressed
//! and groups files by their virtual directory.

use crate::message::{Message, MessageHandler};
use crate::peer::PeerMessage;
use crate::utils::zlib::{deflate, inflate};
use std::sync::mpsc::Sender;

/// One file in a shared directory: its basename, size and the `(code, value)`
/// audio attributes the owner advertises for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedFileEntry {
    pub name: String,
    pub size: u64,
    pub attributes: Vec<(u32, u32)>,
}

impl SharedFileEntry {
    /// The value of attribute `code` (0 bitrate, 1 duration, 2 VBR, 4 sample
    /// rate, 5 bit depth), if the owner advertised it.
    #[must_use]
    pub fn attribute(&self, code: u32) -> Option<u32> {
        self.attributes
            .iter()
            .find(|&&(c, _)| c == code)
            .map(|&(_, v)| v)
    }
}

/// One shared directory and the files directly in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedDirectory {
    pub name: String,
    pub files: Vec<SharedFileEntry>,
}

/// Receives a peer's `SharedFileListResponse` (peer code 5) when browsing them.
pub struct SharedFileListResponseHandler;
impl MessageHandler<PeerMessage> for SharedFileListResponseHandler {
    fn get_code(&self) -> u32 {
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
    write_directories(&mut payload, dirs);
    payload.write_int32(0); // unknown
    payload.write_int32(0); // number of private directories

    let compressed = deflate(&payload.get_data());
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
    decompress_body(message)
        .map_or_else(Vec::new, |mut body| read_directories(&mut body))
}

/// Write a directory listing in the form both code 5 and code 37 carry.
pub fn write_directories(payload: &mut Message, dirs: &[SharedDirectory]) {
    payload.write_int32(dirs.len() as u32);
    for dir in dirs {
        payload
            .write_string(&dir.name)
            .write_int32(dir.files.len() as u32);
        for file in &dir.files {
            payload
                .write_int8(1)
                .write_string(&file.name)
                .write_int64(file.size)
                .write_string("") // extension
                .write_int32(file.attributes.len() as u32);
            for &(code, value) in &file.attributes {
                payload.write_int32(code).write_int32(value);
            }
        }
    }
}

/// Read back what [`write_directories`] wrote, stopping early when a hostile
/// count outruns the payload so a bogus length cannot spin into a huge
/// allocation loop.
pub fn read_directories(body: &mut Message) -> Vec<SharedDirectory> {
    let dir_count = body.read_int32();
    let mut dirs = Vec::new();
    for _ in 0..dir_count {
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
            let name = body.read_string();
            let size = body.read_int64();
            body.read_string(); // extension
            let attr_count = body.read_int32();
            let mut attributes = Vec::new();
            for _ in 0..attr_count {
                // Each attribute is two int32s (8 bytes); read_int32 does not
                // advance past the end, so bound the loop explicitly.
                if body.get_pointer() + 8 > body.get_size() {
                    break;
                }
                attributes.push((body.read_int32(), body.read_int32()));
            }
            files.push(SharedFileEntry {
                name,
                size,
                attributes,
            });
        }
        dirs.push(SharedDirectory { name, files });
    }
    dirs
}

/// Inflate a compressed peer payload positioned at the blob.
pub fn decompress_body(message: &mut Message) -> Option<Message> {
    let pointer = message.get_pointer();
    let size = message.get_size();
    inflate(&message.get_slice(pointer, size))
        .ok()
        .map(Message::new_with_data)
}

#[test]
fn hostile_dir_count_does_not_hang() {
    // A compressed body claiming ~4 billion directories with no data must
    // parse to empty promptly rather than looping into an OOM.
    let compressed = crate::utils::zlib::deflate(&u32::MAX.to_le_bytes());
    let mut message = crate::message::framed(|m| {
        m.write_raw_bytes(compressed);
    });
    assert!(parse_shared_file_list(&mut message).is_empty());
}

#[test]
fn a_listing_carries_each_files_attributes() {
    let dirs = vec![SharedDirectory {
        name: "music\\album".to_string(),
        files: vec![
            SharedFileEntry {
                name: "one.mp3".to_string(),
                size: 40,
                attributes: vec![(0, 320), (1, 187)],
            },
            SharedFileEntry {
                name: "two.flac".to_string(),
                size: 50,
                attributes: Vec::new(),
            },
        ],
    }];
    let mut message = crate::message::framed(|m| {
        m.write_raw_bytes(
            build_shared_file_list(&dirs).get_data()[4..].to_vec(),
        );
    });
    assert_eq!(parse_shared_file_list(&mut message), dirs);
}

#[test]
fn shared_file_list_roundtrips() {
    let dirs = vec![
        SharedDirectory {
            name: "music\\album".to_string(),
            files: vec![
                SharedFileEntry {
                    name: "song one.flac".to_string(),
                    size: 123,
                    attributes: vec![(1, 300), (4, 44_100), (5, 16)],
                },
                SharedFileEntry {
                    name: "song two.flac".to_string(),
                    size: 456,
                    attributes: Vec::new(),
                },
            ],
        },
        SharedDirectory {
            name: "music".to_string(),
            files: vec![SharedFileEntry {
                name: "top.mp3".to_string(),
                size: 789,
                attributes: Vec::new(),
            }],
        },
    ];
    let message = build_shared_file_list(&dirs);

    // Decode via the same offset the dispatcher would use.
    let mut decoded = Message::new_with_data(message.get_buffer());
    decoded.set_pointer(8);
    assert_eq!(parse_shared_file_list(&mut decoded), dirs);
}
