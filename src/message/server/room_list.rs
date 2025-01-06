use crate::{
    message::{Message, MessageHandler},
    server::ServerOperation,
};
use std::sync::mpsc::Sender;

pub struct RoomListHandler;

impl MessageHandler<ServerOperation> for RoomListHandler {
    fn get_code(&self) -> u8 {
        64
    }

    fn handle(&self, _message: &mut Message, _sender: Sender<ServerOperation>) {
        // let num_public_rooms = message.read_int32();
        // for _ in 0..num_public_rooms {
        //     rooms.public_rooms.push(Room::new(message.read_string(), 0));
        // }
        //
        // let num_public_rooms = message.read_int32();
        // for i in 0..num_public_rooms {
        //     rooms.public_rooms[i as usize].set_number_of_users(message.read_int32());
        // }
        //
        // let num_owned_private_rooms = message.read_int32();
        // for _ in 0..num_owned_private_rooms {
        //     rooms
        //         .owned_private_rooms
        //         .push(Room::new(message.read_string(), 0));
        // }
        //
        // let num_owned_private_rooms = message.read_int32();
        // for i in 0..num_owned_private_rooms {
        //     rooms.owned_private_rooms[i as usize].set_number_of_users(message.read_int32());
        // }
        //
        // let num_private_rooms = message.read_int32();
        // for _ in 0..num_private_rooms {
        //     rooms
        //         .private_rooms
        //         .push(Room::new(message.read_string(), 0));
        // }
        //
        // let num_private_rooms = message.read_int32();
        // for i in 0..num_private_rooms {
        //     rooms.private_rooms[i as usize].set_number_of_users(message.read_int32());
        // }
        //
        // let num_operated_private_rooms = message.read_int32();
        // for _ in 0..num_operated_private_rooms {
        //     rooms
        //         .operated_private_rooms
        //         .push(Room::new(message.read_string(), 0));
        // }
        // let num_operated_private_rooms = message.read_int32();
        // for i in 0..num_operated_private_rooms {
        //     rooms.operated_private_rooms[i as usize].set_number_of_users(message.read_int32());
        // }
        // cvat.notify_all();
        // rooms.print();
    }
}
