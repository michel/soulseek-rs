//! `FolderContentsRequest` (peer code 36) and its reply (code 37): one folder
//! of a share in the listing form code 5 uses, which is what "download
//! folder" in other clients asks for.

use crate::message::peer::SharedDirectory;
#[cfg(test)]
use crate::message::peer::SharedFileEntry;
use crate::message::peer::shared_file_list::{
    decompress_body, read_directories, write_directories,
};
use crate::message::{Message, MessageHandler};
use crate::peer::PeerMessage;
use crate::utils::zlib::deflate;
use std::sync::mpsc::Sender;

pub struct FolderContentsRequest;
impl MessageHandler<PeerMessage> for FolderContentsRequest {
    fn get_code(&self) -> u32 {
        36
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let token = message.read_int32();
        let folder = message.read_string();
        let _ =
            sender.send(PeerMessage::FolderContentsRequested { token, folder });
    }
}

/// `FolderContentsResponse` (peer code 37): the reply to a folder we asked
/// for. The token is the one we quoted, so a client with two folders in
/// flight can tell the answers apart.
pub struct FolderContentsResponseHandler;
impl MessageHandler<PeerMessage> for FolderContentsResponseHandler {
    fn get_code(&self) -> u32 {
        37
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let Some((token, folder, directories)) = parse_folder_contents(message)
        else {
            return;
        };
        let _ = sender.send(PeerMessage::FolderContentsReceived {
            token,
            folder,
            directories,
        });
    }
}

/// Build a `FolderContentsRequest` (peer code 36).
#[must_use]
pub fn build_folder_contents_request(token: u32, folder: &str) -> Message {
    Message::new()
        .write_int32(36)
        .write_int32(token)
        .write_string(folder)
        .clone()
}

/// Build a `FolderContentsResponse` (peer code 37).
#[must_use]
pub fn build_folder_contents(
    token: u32,
    folder: &str,
    dirs: &[SharedDirectory],
) -> Message {
    let mut payload = Message::new();
    payload.write_int32(token).write_string(folder);
    write_directories(&mut payload, dirs);
    Message::new()
        .write_int32(37)
        .write_raw_bytes(deflate(&payload.get_data()))
        .clone()
}

/// Parse a `FolderContentsResponse` positioned at its compressed blob, or
/// `None` when it does not inflate.
#[must_use]
pub fn parse_folder_contents(
    message: &mut Message,
) -> Option<(u32, String, Vec<SharedDirectory>)> {
    let mut body = decompress_body(message)?;
    let token = body.read_int32();
    let folder = body.read_string();
    Some((token, folder, read_directories(&mut body)))
}

#[test]
fn a_folder_listing_round_trips() {
    let dirs = vec![SharedDirectory {
        name: "music\\album".to_string(),
        files: vec![
            SharedFileEntry {
                name: "one.flac".to_string(),
                size: 40,
                attributes: vec![(0, 992)],
            },
            SharedFileEntry {
                name: "two.flac".to_string(),
                size: 50,
                attributes: Vec::new(),
            },
        ],
    }];
    let built = build_folder_contents(7, "music\\album", &dirs);
    let mut message = Message::new_with_data(built.get_data());
    message.set_pointer(4);
    assert_eq!(
        parse_folder_contents(&mut message),
        Some((7, "music\\album".to_string(), dirs))
    );
}
