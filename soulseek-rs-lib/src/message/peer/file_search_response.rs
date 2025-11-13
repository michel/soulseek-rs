use crate::message::{Message, MessageHandler};
use crate::peer::PeerMessage;
use crate::types::SearchResult;
use std::sync::mpsc::Sender;

pub struct FileSearchResponse;
impl MessageHandler<PeerMessage> for FileSearchResponse {
    fn get_code(&self) -> u8 {
        9
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let file_search = match SearchResult::new_from_message(message) {
            Ok(result) => result,
            Err(_) => return, // Skip malformed search results
        };

        sender
            .send(PeerMessage::FileSearchResult(file_search))
            .unwrap();
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
