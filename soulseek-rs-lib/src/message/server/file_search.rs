use crate::trace;

use crate::{
    actor::server_actor::ServerMessage, message::Message,
    message::handlers::MessageHandler,
};

pub struct FileSearchHandler;

impl MessageHandler<ServerMessage> for FileSearchHandler {
    fn get_code(&self) -> u8 {
        26
    }
    fn handle(&self, message: &mut Message, out: &mut Vec<ServerMessage>) {
        // The server distributes another user's search to us: [user][token][query].
        let username = message.read_string();
        let token = message.read_int32();
        let query = message.read_string();
        trace!("[server] search from {}: {} ({})", username, query, token);
        out.push(ServerMessage::FileSearchRequest {
            username,
            token,
            query,
        });
    }
}
