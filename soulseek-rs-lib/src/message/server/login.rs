use crate::{
    actor::server_actor::ServerMessage, debug, info, message::Message,
};
use std::sync::mpsc::Sender;

use crate::message::MessageHandler;

pub struct LoginHandler;

impl MessageHandler<ServerMessage> for LoginHandler {
    fn get_code(&self) -> u8 {
        1
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let response = message.read_int8();

        if response != 1 {
            return sender.send(ServerMessage::LoginStatus(false)).unwrap();
        }

        info!("Login successful");
        let greeting = message.read_string();
        debug!("Server greeting: {:?}", greeting);

        sender.send(ServerMessage::LoginStatus(true)).unwrap();
    }
}

// fn build_login_response_message() -> Message {
//     return Message::new_with_data([
//         50, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 81, 170, 162, 77, 32, 0, 0, 0, 101, 102, 99, 97,
//         51, 52, 102, 99, 52, 99, 56, 98, 101, 56, 98, 55, 101, 102, 51, 56, 97, 102, 50, 54, 50,
//         52, 100, 101, 53, 52, 54, 52, 0,
//     ]);
// }
// #[test]
// fn test_can_handle() {
//     assert_eq!(true, LoginHandler.can_handle(build_login_response_message());
//     );
// }
