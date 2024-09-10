use crate::{
    dispatcher::MessageHandler,
    message::Message,
    server::{Context, Room},
};
use std::sync::{Arc, Mutex};
pub struct RoomListHandler;

impl MessageHandler for RoomListHandler {
    fn handle(&self, message: &mut Message, context: Arc<Mutex<Context>>) {
        let mut binding = context.lock().unwrap();
        let rooms = binding.get_rooms();

        let num_public_rooms = message.read_int32();
        for _ in 0..num_public_rooms {
            rooms.public_rooms.push(Room::new(message.read_string(), 0));
        }

        let num_public_rooms = message.read_int32();
        for i in 0..num_public_rooms {
            rooms.public_rooms[i as usize].set_number_of_users(message.read_int32());
        }

        let num_owned_private_rooms = message.read_int32();
        for _ in 0..num_owned_private_rooms {
            rooms
                .owned_private_rooms
                .push(Room::new(message.read_string(), 0));
        }

        let num_owned_private_rooms = message.read_int32();
        for i in 0..num_owned_private_rooms {
            rooms.owned_private_rooms[i as usize].set_number_of_users(message.read_int32());
        }

        let num_private_rooms = message.read_int32();
        for _ in 0..num_private_rooms {
            rooms
                .private_rooms
                .push(Room::new(message.read_string(), 0));
        }

        let num_private_rooms = message.read_int32();
        for i in 0..num_private_rooms {
            rooms.private_rooms[i as usize].set_number_of_users(message.read_int32());
        }

        let num_operated_private_rooms = message.read_int32();
        for _ in 0..num_operated_private_rooms {
            rooms
                .operated_private_rooms
                .push(Room::new(message.read_string(), 0));
        }
        let num_operated_private_rooms = message.read_int32();
        for i in 0..num_operated_private_rooms {
            rooms.operated_private_rooms[i as usize].set_number_of_users(message.read_int32());
        }
        rooms.print();
    }
}
