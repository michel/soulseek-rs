use super::handlers::MessageHandler;
use crate::{
    message::Message,
    server::{Context, UserMessage},
};
use std::sync::{Arc, Mutex};

pub struct MessageUser;

impl MessageHandler for MessageUser {
    fn get_code(&self) -> u8 {
        22
    }

    fn handle(&self, message: &mut Message, context: Arc<Mutex<Context>>) {
        let id = message.read_int32();
        let timestamp = message.read_int32();
        let username = message.read_string();
        let message_content = message.read_string();
        let new_message = message.read_bool();
        let user_message = UserMessage::new(
            id,
            timestamp,
            username.clone(),
            message_content,
            new_message,
        );

        context
            .lock()
            .unwrap()
            .add_message_for_user(username.clone(), user_message.clone());
        println!("User message received:",);
        user_message.print()
    }
}

#[cfg(test)]
use std::sync::mpsc;
#[cfg(test)]
use std::sync::mpsc::{Receiver, Sender};

#[test]
fn test_handle() {
    let (message_sender, _message_reader): (Sender<Message>, Receiver<Message>) = mpsc::channel();

    let context = Arc::new(Mutex::new(Context::new(Arc::new(Mutex::new(
        message_sender,
    )))));
    let mut message = Message::new_with_data(vec![
        254, 0, 0, 0, 22, 0, 0, 0, 153, 102, 47, 0, 58, 65, 228, 102, 6, 0, 0, 0, 115, 101, 114,
        118, 101, 114, 227, 0, 0, 0, 89, 111, 117, 114, 32, 99, 111, 110, 110, 101, 99, 116, 105,
        111, 110, 32, 105, 115, 32, 114, 101, 115, 116, 114, 105, 99, 116, 101, 100, 58, 32, 89,
        111, 117, 32, 99, 97, 110, 110, 111, 116, 32, 115, 101, 97, 114, 99, 104, 32, 111, 114, 32,
        99, 104, 97, 116, 46, 32, 89, 111, 117, 114, 32, 99, 108, 105, 101, 110, 116, 32, 118, 101,
        114, 115, 105, 111, 110, 32, 105, 115, 32, 116, 111, 111, 32, 111, 108, 100, 46, 32, 89,
        111, 117, 32, 110, 101, 101, 100, 32, 116, 111, 32, 117, 112, 103, 114, 97, 100, 101, 32,
        116, 111, 32, 116, 104, 101, 32, 108, 97, 116, 101, 115, 116, 32, 118, 101, 114, 115, 105,
        111, 110, 46, 32, 67, 108, 111, 115, 101, 32, 116, 104, 105, 115, 32, 99, 108, 105, 101,
        110, 116, 44, 32, 100, 111, 119, 110, 108, 111, 97, 100, 32, 110, 101, 119, 32, 118, 101,
        114, 115, 105, 111, 110, 32, 102, 114, 111, 109, 32, 104, 116, 116, 112, 58, 47, 47, 119,
        119, 119, 46, 115, 108, 115, 107, 110, 101, 116, 46, 111, 114, 103, 44, 32, 105, 110, 115,
        116, 97, 108, 108, 32, 105, 116, 32, 97, 110, 100, 32, 114, 101, 99, 111, 110, 110, 101,
        99, 116, 46, 1,
    ]);
    message.set_pointer(8);
    MessageUser.handle(&mut message.clone(), context.clone());
    assert_eq!(1, context.lock().unwrap().get_user_messages().len());
}
// }
