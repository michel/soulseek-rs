use crate::message::{Message, MessageHandler};
use crate::peer::PeerMessage;
use crate::types::SearchResult;
use crate::utils::zlib::compress_stored;
use std::sync::mpsc::Sender;

/// A borrowed view of one file to advertise in a search response, kept
/// independent of the shares module so the message layer stays decoupled.
pub struct FileEntry<'a> {
    pub name: &'a str,
    pub size: u64,
    pub attribs: &'a [(u32, u32)],
}

/// Build a `FileSearchResponse` (peer code 9): the zlib-compressed reply we send
/// to a peer whose search matched our shared files. The payload is the exact
/// inverse of [`SearchResult::new_from_message`].
#[must_use]
pub fn build_file_search_response(
    own_username: &str,
    token: u32,
    files: &[FileEntry],
    slots: u8,
    speed: u32,
) -> Message {
    let mut payload = Message::new();
    payload
        .write_string(own_username)
        .write_int32(token)
        .write_int32(files.len() as u32);
    for file in files {
        payload
            .write_int8(1)
            .write_string(file.name)
            .write_int64(file.size)
            .write_string("") // extension (decoder reads and discards)
            .write_int32(file.attribs.len() as u32);
        for &(code, value) in file.attribs {
            payload.write_int32(code).write_int32(value);
        }
    }
    payload.write_int8(slots).write_int32(speed).write_int32(0); // free upload slots / queue length (well-formed trailer)

    let compressed = compress_stored(&payload.get_data());
    Message::new()
        .write_int32(9)
        .write_raw_bytes(compressed)
        .clone()
}

pub struct FileSearchResponse;
impl MessageHandler<PeerMessage> for FileSearchResponse {
    fn get_code(&self) -> u8 {
        9
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        // Skip malformed search results
        let Ok(file_search) = SearchResult::new_from_message(message) else {
            return;
        };

        let _ = sender.send(PeerMessage::FileSearchResult(file_search));
    }
}

#[test]
fn test_new_from_message() {
    let data: Vec<u8> = [
        229, 0, 0, 0, 9, 0, 0, 0, 120, 156, 99, 103, 96, 96, 72, 201, 79, 201,
        76, 79, 204, 203, 213, 158, 98, 194, 4, 228, 50, 250, 3, 9, 7, 135,
        162, 156, 148, 194, 188, 152, 228, 252, 220, 130, 156, 212, 146, 212,
        24, 231, 196, 188, 228, 204, 252, 188, 212, 226, 152, 144, 162, 210,
        226, 226, 212, 28, 93, 67, 75, 115, 75, 93, 119, 160, 144, 130, 91,
        126, 145, 66, 72, 70, 170, 66, 120, 106, 106, 118, 106, 94, 138, 174,
        161, 89, 82, 102, 137, 174, 137, 137, 142, 161, 119, 70, 149, 94, 90,
        78, 98, 114, 203, 175, 243, 32, 163, 193, 128, 25, 100, 7, 16, 23, 0,
        9, 22, 32, 237, 178, 134, 129, 129, 21, 72, 11, 128, 196, 243, 176,
        217, 29, 156, 153, 151, 158, 147, 90, 12, 54, 95, 193, 216, 84, 193,
        200, 192, 200, 36, 198, 45, 181, 168, 40, 53, 57, 91, 193, 37, 177, 60,
        79, 71, 193, 55, 177, 44, 181, 40, 19, 200, 13, 78, 76, 42, 74, 85, 80,
        83, 240, 75, 45, 7, 10, 38, 103, 100, 2, 221, 167, 139, 238, 66, 5, 13,
        144, 17, 154, 96, 167, 173, 228, 215, 98, 68, 119, 218, 74, 6, 76, 167,
        49, 60, 153, 202, 200, 160, 199, 128, 0, 0, 161, 99, 76, 142,
    ]
    .to_vec();
    let mut message = Message::new_with_data(data);
    message.set_pointer(8);

    let file_search = SearchResult::new_from_message(&mut message).unwrap();
    assert_eq!(file_search.token, 882125677);
    assert_eq!(file_search.files.len(), 2);
    let file = &file_search.files[0];
    assert_eq!(
        file.name,
        "@@rldqn\\complete\\Canciones\\Trussel-1979-Gone For The Weekend-16bit-44,1Khz.flac"
    );
    assert_eq!(file.username, "dodigan");
    assert_eq!(file.size, 47184516);
}

#[test]
fn build_file_search_response_roundtrips_through_the_decoder() {
    let files = [
        FileEntry {
            name: "music\\album\\song.mp3",
            size: 47_184_516,
            attribs: &[(1, 320), (4, 44100), (5, 16)],
        },
        FileEntry {
            name: "b.flac",
            size: 456,
            attribs: &[],
        },
    ];
    let message = build_file_search_response("e2e_sharer", 42, &files, 1, 0);

    // Decode via the exact production decoder used for real peer responses:
    // the dispatcher positions the pointer at 8 (past length + code).
    let mut decoded = Message::new_with_data(message.get_buffer());
    decoded.set_pointer(8);
    let result = SearchResult::new_from_message(&mut decoded).unwrap();

    assert_eq!(result.username, "e2e_sharer");
    assert_eq!(result.token, 42);
    assert_eq!(result.files.len(), 2);
    assert_eq!(result.files[0].name, "music\\album\\song.mp3");
    assert_eq!(result.files[0].size, 47_184_516);
    assert_eq!(result.files[0].attribs.get(&1), Some(&320));
    assert_eq!(result.files[0].attribs.get(&4), Some(&44100));
    assert_eq!(result.files[1].name, "b.flac");
    assert_eq!(result.files[1].size, 456);
    assert!(result.files[1].attribs.is_empty());
    assert_eq!(result.slots, 1);
}
