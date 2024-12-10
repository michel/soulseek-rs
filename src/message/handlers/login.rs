use std::sync::{mpsc::Sender, Arc, Condvar, Mutex};

use super::handlers::MessageHandler;
use crate::{
    message::{factory::build_shared_folders_message, Message},
    server::ServerOperation,
};

pub struct LoginHandler;

impl MessageHandler for LoginHandler {
    fn get_code(&self) -> u8 {
        1
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerOperation>) {
        let response = message.read_int8();

        if response != 1 {
            return sender.send(ServerOperation::LoginStatus(false)).unwrap();
        }

        println!("Login successful");
        let greeting = message.read_string();
        println!("Server geeting: {:?}", greeting);

        // Build the shared folders message and queue it
        let shared_message = build_shared_folders_message(1, 1);

        sender.send(ServerOperation::LoginStatus(true)).unwrap();
        sender
            .send(ServerOperation::SendMessage(shared_message))
            .unwrap();
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
