use crate::{dispatcher::MessageHandler, message::Message, server::Room};
pub struct RoomListHandler;

impl MessageHandler for RoomListHandler {
    fn handle(&self, message: &mut Message, mut context: crate::server::Context) {
        let mut rooms: Vec<Room> = Vec::new();
        let mut owned_private_rooms: Vec<Room> = Vec::new();
        let mut private_rooms: Vec<Room> = Vec::new();
        let mut operated_private_rooms: Vec<Room> = Vec::new();

        // Number of rooms
        let num_rooms = message.read_int32();
        for _ in 0..num_rooms {
            rooms.push(Room::new(message.read_string(), 0));
        }

        // Number of rooms
        let num_rooms = message.read_int32();
        // get the number of users in each room
        for i in 0..num_rooms {
            rooms[i as usize].set_number_of_users(message.read_int32());
        }

        // Number of owned private rooms
        let num_owned_private_rooms = message.read_int32();
        for _ in 0..num_owned_private_rooms {
            owned_private_rooms.push(Room::new(message.read_string(), 0));
        }

        // Number of owned private rooms
        let num_owned_private_rooms = message.read_int32();
        for i in 0..num_owned_private_rooms {
            owned_private_rooms[i as usize].set_number_of_users(message.read_int32());
        }

        // Number of private rooms (except owned)
        let num_private_rooms = message.read_int32();
        for _ in 0..num_private_rooms {
            private_rooms.push(Room::new(message.read_string(), 0));
        }

        // Number of private rooms (except owned)
        let num_private_rooms = message.read_int32();
        for i in 0..num_private_rooms {
            // Number of users in private rooms (except owned)
            private_rooms[i as usize].set_number_of_users(message.read_int32());
        }

        // Number of operated private rooms
        let num_operated_private_rooms = message.read_int32();
        for _ in 0..num_operated_private_rooms {
            operated_private_rooms.push(Room::new(message.read_string(), 0));
        }
        let num_operated_private_rooms = message.read_int32();
        for i in 0..num_operated_private_rooms {
            // Number of users in private rooms (except owned)
            operated_private_rooms[i as usize].set_number_of_users(message.read_int32());
        }
        println!("Rooms: {:?}", rooms.len());
        println!("Owned private rooms: {:?}", owned_private_rooms.len());
        println!("Private rooms: {:?}", private_rooms.len());
        println!("Operated private rooms: {:?}", operated_private_rooms.len());
        context.set_rooms(rooms);
    }
}
