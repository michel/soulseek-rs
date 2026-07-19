use crate::{
    message::Message, peer::ConnectionType, types::Transfer, utils::md5::md5,
};

pub struct MessageFactory;
impl MessageFactory {
    #[must_use]
    pub fn build_get_peer_address(username: &str) -> Message {
        let mut message = Message::new();

        message.write_int32(3);
        message.write_string(username);
        message
    }
    #[must_use]
    pub fn build_login_message(username: &str, password: &str) -> Message {
        // Message::new_with_data(
        //     [
        //         1, 0, 0, 0, 20, 0, 0, 0, 105, 110, 115, 97, 110, 101, 95, 105, 110, 95, 116, 104, 101,
        //         95, 98, 114, 97, 105, 110, 50, 8, 0, 0, 0, 49, 51, 51, 55, 53, 49, 51, 55, 160, 0, 0,
        //         0, 32, 0, 0, 0, 50, 101, 100, 102, 53, 49, 100, 48, 51, 55, 57, 52, 51, 55, 56, 102,
        //         56, 98, 98, 54, 51, 49, 48, 100, 52, 54, 48, 99, 50, 50, 98, 49, 17, 0, 0,
        //         0,
        //         //0, // 84, 0, 0, 0, 1, 0, 0, 0, 20, 0, 0, 0, 105, 110, 115, 97, 110, 101, 95, 105, 110, 95,
        //         // 116, 104, 101, 95, 98, 114, 97, 105, 110, 50, 8, 0, 0, 0, 49, 51, 51, 55, 53, 49, 51,
        //         // 55, 160, 0, 0, 0, 32, 0, 0, 0, 50, 101, 100, 102, 53, 49, 100, 48, 51, 55, 57, 52, 51,
        //         // 55, 56, 102, 56, 98, 98, 54, 51, 49, 48, 100, 52, 54, 48, 99, 50, 50, 98, 49, 17, 0, 0,
        //         // 0,
        //     ]
        //     .to_vec(),
        // )
        // .clone()fac
        let hash = md5([username, password].join("").as_str());

        let mut message = Message::new();

        message
            .write_int32(1)
            .write_string(username)
            .write_string(password)
            .write_int32(157) // version
            .write_string(&hash)
            .write_int32(100)
            .clone()
    }

    #[must_use]
    pub fn build_shared_folders_message(
        folder_count: u32,
        file_count: u32,
    ) -> Message {
        Message::new()
            .write_int32(35)
            .write_int32(folder_count)
            .write_int32(file_count)
            .clone()
    }
    #[must_use]
    pub fn build_file_search_message(token: u32, query: &str) -> Message {
        Message::new()
            .write_int32(26)
            .write_int32(token)
            .write_string(query)
            .clone()
    }
    /// Build a private message (server code 22) to send to another user.
    #[must_use]
    pub fn build_message_user(username: &str, message: &str) -> Message {
        Message::new()
            .write_int32(22)
            .write_string(username)
            .write_string(message)
            .clone()
    }

    /// Acknowledge a received private message (server code 23) so the server
    /// stops re-delivering it.
    #[must_use]
    pub fn build_message_acked(id: u32) -> Message {
        Message::new().write_int32(23).write_int32(id).clone()
    }

    /// Ask the server (code 18) to broker a connection to a firewalled peer:
    /// the server tells that peer to connect back to us, quoting `token`.
    #[must_use]
    pub fn build_connect_to_peer(
        token: u32,
        username: &str,
        connection_type: ConnectionType,
    ) -> Message {
        Message::new()
            .write_int32(18)
            .write_int32(token)
            .write_string(username)
            .write_string(&connection_type.to_string())
            .clone()
    }

    #[must_use]
    pub fn build_set_status_message(status_code: u32) -> Message {
        Message::new()
            .write_int32(28)
            .write_int32(status_code)
            .clone()
    }
    #[must_use]
    pub fn build_no_parent_message() -> Message {
        Message::new().write_int32(71).write_bool(true).clone()
    }
    #[must_use]
    pub fn build_set_wait_port_message(port: u16) -> Message {
        Message::new()
            .write_int32(2)
            .write_int32(port.into())
            .clone()
    }
    #[must_use]
    pub fn build_watch_user(token: u32) -> Message {
        Message::new()
            .write_raw_bytes([5, 0, 0, 0, 0].to_vec())
            .write_int32(token)
            .clone()
    }

    /// Ask the server (code 64) for the list of public chat rooms.
    #[must_use]
    pub fn build_room_list_request() -> Message {
        Message::new().write_int32(64).clone()
    }

    /// Join a chat room (server code 14). `private` requests a private room.
    #[must_use]
    pub fn build_join_room(room: &str, private: bool) -> Message {
        Message::new()
            .write_int32(14)
            .write_string(room)
            .write_int32(u32::from(private))
            .clone()
    }

    /// Leave a chat room (server code 15).
    #[must_use]
    pub fn build_leave_room(room: &str) -> Message {
        Message::new().write_int32(15).write_string(room).clone()
    }

    /// Say `message` in chat room `room` (server code 13).
    #[must_use]
    pub fn build_say_chatroom(room: &str, message: &str) -> Message {
        Message::new()
            .write_int32(13)
            .write_string(room)
            .write_string(message)
            .clone()
    }

    /// Ask a peer for their shared-file listing (peer code 4, no body).
    #[must_use]
    pub fn build_get_share_file_list() -> Message {
        Message::new().write_int32(4).clone()
    }

    #[must_use]
    pub fn build_queue_upload_message(filename: &str) -> Message {
        Message::new()
            .write_int32(43)
            .write_string(filename)
            .clone()
    }

    #[must_use]
    pub fn build_transfer_request_message(
        filename: &str,
        token: u32,
    ) -> Message {
        Message::new()
            .write_int32(40) // code
            .write_int32(0) // direction
            .write_int32(token)
            .write_string(filename)
            .clone()
    }
    /// A TransferRequest (peer code 40) initiating an *upload*: we offer a file
    /// to a peer who queued it, quoting our transfer token and its size.
    #[must_use]
    pub fn build_upload_transfer_request(
        filename: &str,
        token: u32,
        size: u64,
    ) -> Message {
        Message::new()
            .write_int32(40)
            .write_int32(1) // direction: upload
            .write_int32(token)
            .write_string(filename)
            .write_int64(size)
            .clone()
    }

    #[must_use]
    pub fn build_transfer_response_message(transfer: Transfer) -> Message {
        Message::new()
            .write_int32(41)
            .write_int32(transfer.token)
            .write_bool(true)
            .clone()
    }
    #[must_use]
    pub fn build_pierce_firewall_message(token: u32) -> Message {
        Message::new()
            .write_int8(0) // PierceFirewall message code
            .write_int32(token)
            .clone()
    }

    #[must_use]
    pub fn build_peer_init_message(
        own_username: &str,
        connection_type: ConnectionType,
        token: u32,
    ) -> Message {
        Message::new()
            .write_int8(1)
            .write_string(own_username)
            .write_string(&connection_type.to_string())
            .write_int32(token)
            .clone()
    }
}

#[test]
fn test_build_watch_user() {
    let token: u32 = 223;
    let message = MessageFactory::build_watch_user(token);
    let expect: Vec<u8> = [5, 0, 0, 0, 0, 223, 0, 0, 0].to_vec();

    assert_eq!(expect, message.get_data());
}

#[test]
fn test_build_login_message() {
    let message =
        MessageFactory::build_login_message("insane_in_the_brain2", "13375137");

    let expect: Vec<u8> = [
        1, 0, 0, 0, 20, 0, 0, 0, 105, 110, 115, 97, 110, 101, 95, 105, 110, 95,
        116, 104, 101, 95, 98, 114, 97, 105, 110, 50, 8, 0, 0, 0, 49, 51, 51,
        55, 53, 49, 51, 55, 157, 0, 0, 0, 32, 0, 0, 0, 50, 101, 100, 102, 53,
        49, 100, 48, 51, 55, 57, 52, 51, 55, 56, 102, 56, 98, 98, 54, 51, 49,
        48, 100, 52, 54, 48, 99, 50, 50, 98, 49, 100, 0, 0, 0,
    ]
    .to_vec();

    assert_eq!(expect, message.get_data());
}

#[test]
fn test_build_upload_transfer_request() {
    use crate::types::Transfer;
    let message =
        MessageFactory::build_upload_transfer_request("song.mp3", 555, 4096);
    // Decode via the production Transfer parser (dispatcher starts at offset 8).
    let mut decoded = Message::new_with_data(message.get_buffer());
    decoded.set_pointer(8);
    let transfer = Transfer::new_from_message(&mut decoded);
    assert_eq!(transfer.direction, 1); // upload
    assert_eq!(transfer.token, 555);
    assert_eq!(transfer.filename, "song.mp3");
    assert_eq!(transfer.size, 4096);
}

#[test]
fn test_build_peer_init_message() {
    use crate::peer::ConnectionType;
    let message =
        MessageFactory::build_peer_init_message("bob", ConnectionType::P, 7);
    // [1][len=3]"bob"[len=1]"P"[token=7] — no length prefix in get_data()
    let expect: Vec<u8> = [
        1, // PeerInit code (int8)
        3, 0, 0, 0, 98, 111, 98, // username "bob"
        1, 0, 0, 0, 80, // connection type "P"
        7, 0, 0, 0, // token
    ]
    .to_vec();
    assert_eq!(expect, message.get_data());
}

#[test]
fn test_build_message_user() {
    let message = MessageFactory::build_message_user("bob", "hi");
    let expect: Vec<u8> = [
        22, 0, 0, 0, // code
        3, 0, 0, 0, 98, 111, 98, // username "bob"
        2, 0, 0, 0, 104, 105, // message "hi"
    ]
    .to_vec();
    assert_eq!(expect, message.get_data());
}

#[test]
fn test_build_message_acked() {
    let message = MessageFactory::build_message_acked(7);
    let expect: Vec<u8> = [23, 0, 0, 0, 7, 0, 0, 0].to_vec();
    assert_eq!(expect, message.get_data());
}

#[test]
fn test_build_join_room() {
    let message = MessageFactory::build_join_room("nicotine", false);
    let expect: Vec<u8> = [
        14, 0, 0, 0, // code
        8, 0, 0, 0, 110, 105, 99, 111, 116, 105, 110, 101, // "nicotine"
        0, 0, 0, 0, // private = 0
    ]
    .to_vec();
    assert_eq!(expect, message.get_data());
}

#[test]
fn test_build_leave_room() {
    let message = MessageFactory::build_leave_room("nicotine");
    let expect: Vec<u8> = [
        15, 0, 0, 0, // code
        8, 0, 0, 0, 110, 105, 99, 111, 116, 105, 110, 101, // "nicotine"
    ]
    .to_vec();
    assert_eq!(expect, message.get_data());
}

#[test]
fn test_build_say_chatroom() {
    let message = MessageFactory::build_say_chatroom("room", "hi");
    let expect: Vec<u8> = [
        13, 0, 0, 0, // code
        4, 0, 0, 0, 114, 111, 111, 109, // "room"
        2, 0, 0, 0, 104, 105, // "hi"
    ]
    .to_vec();
    assert_eq!(expect, message.get_data());
}

#[test]
fn test_build_room_list_request() {
    let message = MessageFactory::build_room_list_request();
    assert_eq!(vec![64, 0, 0, 0], message.get_data());
}

#[test]
fn test_build_file_search_message() {
    let message = MessageFactory::build_file_search_message(12, "trance wax");
    let expect: Vec<u8> = [
        26, 0, 0, 0, 12, 0, 0, 0, 10, 0, 0, 0, 116, 114, 97, 110, 99, 101, 32,
        119, 97, 120,
    ]
    .to_vec();
    assert_eq!(expect, message.get_data());
}
