use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
};

/// `CantConnectToPeer` (code 1001): a brokered peer gave up reaching us.
///
/// The token is the one we quoted in our own `ConnectToPeer` (code 18), and it
/// is the only field the server relays — the username is not repeated, so the
/// token is what a client matches the pending attempt on.
///
/// Without this the download sits in "connecting" until a timeout; with it the
/// wait ends the moment the peer gives up.
pub struct CantConnectToPeerHandler;

impl MessageHandler<ServerMessage> for CantConnectToPeerHandler {
    fn get_code(&self) -> u32 {
        1001
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let token = message.read_int32();
        let _ = sender.send(ServerMessage::CantConnectToPeer { token });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn the_token_of_the_failed_attempt_is_reported() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(4242);
        });

        CantConnectToPeerHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::CantConnectToPeer { token }) => {
                assert_eq!(token, 4242);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
