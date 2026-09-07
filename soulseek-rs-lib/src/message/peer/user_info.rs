//! `UserInfoRequest` (peer code 15) and its reply (code 16): what a peer says
//! about itself when another user opens its profile.

use crate::message::{Message, MessageHandler};
use crate::peer::PeerMessage;
use std::sync::mpsc::Sender;

pub struct UserInfoRequest;
impl MessageHandler<PeerMessage> for UserInfoRequest {
    fn get_code(&self) -> u32 {
        15
    }
    fn handle(&self, _message: &mut Message, sender: Sender<PeerMessage>) {
        let _ = sender.send(PeerMessage::UserInfoRequested);
    }
}

/// Build a `UserInfoResponse` (peer code 16) with no description or picture.
#[must_use]
pub fn build_user_info(
    upload_slots: u32,
    queue_size: u32,
    slots_free: bool,
) -> Message {
    Message::new()
        .write_int32(16)
        .write_string("")
        .write_bool(false)
        .write_int32(upload_slots)
        .write_int32(queue_size)
        .write_bool(slots_free)
        .clone()
}
