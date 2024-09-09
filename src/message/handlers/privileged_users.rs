use crate::{
    dispatcher::MessageHandler,
    message::{factory::build_shared_folders_message, Message},
};
pub struct PrivilegedUsersHandler;

impl MessageHandler for PrivilegedUsersHandler {
    fn handle(&self, message: &mut Message, _context: crate::server::Context) {
        let number = message.read_int32();
        println!("Number of privilaged users: {}", number);
    }
}
