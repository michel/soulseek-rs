use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

/// `ChangePassword` (code 142): the server has stored the password we asked
/// for, and echoes it back.
pub struct ChangePasswordHandler;

impl MessageHandler<ServerMessage> for ChangePasswordHandler {
    fn get_code(&self) -> u32 {
        142
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let password = message.read_string();
        let _ = sender.send(ServerMessage::PasswordChanged(password));
    }
}
