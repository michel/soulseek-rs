use crate::{
    message::Message,
    peer::ConnectionType,
    types::{ClientVersion, Transfer},
    utils::md5::md5,
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
    pub fn build_login_message(
        username: &str,
        password: &str,
        version: ClientVersion,
    ) -> Message {
        let hash = md5([username, password].join("").as_str());

        let mut message = Message::new();

        message
            .write_int32(1)
            .write_string(username)
            .write_string(password)
            .write_int32(version.major)
            .write_string(&hash)
            .write_int32(version.minor)
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

    /// Ask the server (code 92) how much of our own privilege time is left.
    #[must_use]
    pub fn build_check_privileges() -> Message {
        Message::new().write_int32(92).clone()
    }

    /// Tell a peer where their queued file sits (peer code 44). Place counts
    /// from 1.
    #[must_use]
    pub fn build_place_in_queue_response(
        filename: &str,
        place: u32,
    ) -> Message {
        Message::new()
            .write_int32(44)
            .write_string(filename)
            .write_int32(place)
            .clone()
    }

    /// A wishlist search (server code 103). Same shape as a plain `FileSearch`,
    /// but the server rate-limits it to the interval it announced in code 104
    /// and does not count it against the normal search allowance.
    #[must_use]
    pub fn build_wishlist_search(token: u32, query: &str) -> Message {
        Message::new()
            .write_int32(103)
            .write_int32(token)
            .write_string(query)
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
    /// Whether we lack a parent in the distributed network (code 71); the
    /// server offers PossibleParents while we say so.
    pub fn build_have_no_parent(no_parent: bool) -> Message {
        Message::new().write_int32(71).write_bool(no_parent).clone()
    }

    /// The root of the branch we hang from (code 127): ourselves while we
    /// have no parent.
    #[must_use]
    pub fn build_branch_root(username: &str) -> Message {
        Message::new()
            .write_int32(127)
            .write_string(username)
            .clone()
    }

    /// How deep in the tree we sit (code 126): 0 without a parent.
    #[must_use]
    pub fn build_branch_level(level: u32) -> Message {
        Message::new().write_int32(126).write_int32(level).clone()
    }

    /// Whether we take children in the distributed network (code 100).
    #[must_use]
    pub fn build_accept_children(accept: bool) -> Message {
        Message::new().write_int32(100).write_bool(accept).clone()
    }
    #[must_use]
    pub fn build_set_wait_port_message(port: u16) -> Message {
        Message::new()
            .write_int32(2)
            .write_int32(port.into())
            .clone()
    }
    /// Ask the server (code 7) for a user's online status.
    #[must_use]
    pub fn build_get_user_status(username: &str) -> Message {
        Message::new().write_int32(7).write_string(username).clone()
    }

    /// Tell the server (code 121) what a finished upload averaged, in bytes
    /// per second; it folds that into the speed other users see for us.
    #[must_use]
    pub fn build_send_upload_speed(bytes_per_second: u32) -> Message {
        Message::new()
            .write_int32(121)
            .write_int32(bytes_per_second)
            .clone()
    }

    /// Ask the server (code 36) for a user's share statistics.
    #[must_use]
    pub fn build_get_user_stats(username: &str) -> Message {
        Message::new()
            .write_int32(36)
            .write_string(username)
            .clone()
    }

    /// Watch `username` (server code 5): the server replies with their
    /// current status and share statistics, then pushes every later status
    /// change as a `GetUserStatus` (code 7) until we unwatch them.
    #[must_use]
    pub fn build_watch_user(username: &str) -> Message {
        Message::new().write_int32(5).write_string(username).clone()
    }

    /// Stop watching `username` (server code 6). The server sends no reply.
    #[must_use]
    pub fn build_unwatch_user(username: &str) -> Message {
        Message::new().write_int32(6).write_string(username).clone()
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
    pub fn build_transfer_denial_message(token: u32, reason: &str) -> Message {
        Message::new()
            .write_int32(41)
            .write_int32(token)
            .write_bool(false)
            .write_string(reason)
            .clone()
    }

    #[must_use]
    pub fn build_pierce_firewall_message(token: u32) -> Message {
        Message::new()
            .write_int8(0) // PierceFirewall message code
            .write_int32(token)
            .clone()
    }

    /// A keepalive ping (server code 32, no body). The server sends no reply;
    /// its only job is to keep a quiet connection from being reaped by a NAT
    /// or the server's own idle timeout, the way other clients ping.
    #[must_use]
    pub fn build_server_ping() -> Message {
        Message::new().write_int32(32).clone()
    }

    /// Search one user's shares (server code 42). Replies come back over a
    /// peer connection as ordinary `FileSearchResponse` frames, so the token
    /// is drawn from the same space as a plain `FileSearch`.
    #[must_use]
    pub fn build_user_search(
        username: &str,
        token: u32,
        query: &str,
    ) -> Message {
        Message::new()
            .write_int32(42)
            .write_string(username)
            .write_int32(token)
            .write_string(query)
            .clone()
    }

    /// Search the shares of everyone in `room` (server code 120).
    #[must_use]
    pub fn build_room_search(room: &str, token: u32, query: &str) -> Message {
        Message::new()
            .write_int32(120)
            .write_string(room)
            .write_int32(token)
            .write_string(query)
            .clone()
    }

    /// Tell the server (code 1001) that the peer it asked us to connect to,
    /// quoting `token`, could not be reached. The server relays that to the
    /// peer so it stops waiting on a connection that is never coming.
    #[must_use]
    pub fn build_cant_connect_to_peer(token: u32, username: &str) -> Message {
        Message::new()
            .write_int32(1001)
            .write_int32(token)
            .write_string(username)
            .clone()
    }

    /// Set our ticker (the scrolling one-line message) in `room`, server code
    /// 116. An empty ticker removes ours.
    #[must_use]
    pub fn build_set_room_ticker(room: &str, ticker: &str) -> Message {
        Message::new()
            .write_int32(116)
            .write_string(room)
            .write_string(ticker)
            .clone()
    }

    /// Subscribe to the global room feed (server code 150): every public room
    /// message on the server, tagged with the room it came from.
    #[must_use]
    pub fn build_join_global_room() -> Message {
        Message::new().write_int32(150).clone()
    }

    /// Stop the global room feed (server code 151).
    #[must_use]
    pub fn build_leave_global_room() -> Message {
        Message::new().write_int32(151).clone()
    }

    /// Add an interest (server code 51). Interests drive the recommendation
    /// and similar-user queries below.
    #[must_use]
    pub fn build_add_thing_i_like(item: &str) -> Message {
        Message::new().write_int32(51).write_string(item).clone()
    }

    /// Drop an interest (server code 52).
    #[must_use]
    pub fn build_remove_thing_i_like(item: &str) -> Message {
        Message::new().write_int32(52).write_string(item).clone()
    }

    /// Add a dislike (server code 117).
    #[must_use]
    pub fn build_add_thing_i_hate(item: &str) -> Message {
        Message::new().write_int32(117).write_string(item).clone()
    }

    /// Drop a dislike (server code 118).
    #[must_use]
    pub fn build_remove_thing_i_hate(item: &str) -> Message {
        Message::new().write_int32(118).write_string(item).clone()
    }

    /// Ask for recommendations based on our own interests (server code 54).
    #[must_use]
    pub fn build_get_recommendations() -> Message {
        Message::new().write_int32(54).clone()
    }

    /// Ask for the server-wide recommendations (server code 56).
    #[must_use]
    pub fn build_global_recommendations() -> Message {
        Message::new().write_int32(56).clone()
    }

    /// Ask what `username` likes and hates (server code 57).
    #[must_use]
    pub fn build_user_interests(username: &str) -> Message {
        Message::new()
            .write_int32(57)
            .write_string(username)
            .clone()
    }

    /// Ask for users whose interests overlap ours (server code 110).
    #[must_use]
    pub fn build_similar_users() -> Message {
        Message::new().write_int32(110).clone()
    }

    /// Ask what else people who like `item` like (server code 111).
    #[must_use]
    pub fn build_item_recommendations(item: &str) -> Message {
        Message::new().write_int32(111).write_string(item).clone()
    }

    /// Ask which users like `item` (server code 112).
    #[must_use]
    pub fn build_item_similar_users(item: &str) -> Message {
        Message::new().write_int32(112).write_string(item).clone()
    }

    /// Hand `days` of our own privileges to `username` (server code 123).
    #[must_use]
    pub fn build_give_privileges(username: &str, days: u32) -> Message {
        Message::new()
            .write_int32(123)
            .write_string(username)
            .write_int32(days)
            .clone()
    }

    /// Change our account password (server code 142). The server echoes the
    /// accepted password back under the same code.
    #[must_use]
    pub fn build_change_password(password: &str) -> Message {
        Message::new()
            .write_int32(142)
            .write_string(password)
            .clone()
    }

    /// Send one private message to several users at once (server code 149).
    #[must_use]
    pub fn build_message_users(usernames: &[String], message: &str) -> Message {
        let mut msg = Message::new();
        msg.write_int32(149);
        msg.write_int32(usernames.len() as u32);
        for username in usernames {
            msg.write_string(username);
        }
        msg.write_string(message);
        msg
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
fn test_build_get_user_status() {
    let message = MessageFactory::build_get_user_status("bob");
    let expect: Vec<u8> = [7, 0, 0, 0, 3, 0, 0, 0, b'b', b'o', b'b'].to_vec();
    assert_eq!(expect, message.get_data());
}

#[test]
fn test_build_get_user_stats() {
    let message = MessageFactory::build_get_user_stats("bob");
    let expect: Vec<u8> = [36, 0, 0, 0, 3, 0, 0, 0, b'b', b'o', b'b'].to_vec();
    assert_eq!(expect, message.get_data());
}

#[test]
fn test_build_login_message() {
    let message = MessageFactory::build_login_message(
        "insane_in_the_brain2",
        "13375137",
        ClientVersion::default(),
    );

    let expect: Vec<u8> = [
        1, 0, 0, 0, 20, 0, 0, 0, 105, 110, 115, 97, 110, 101, 95, 105, 110, 95,
        116, 104, 101, 95, 98, 114, 97, 105, 110, 50, 8, 0, 0, 0, 49, 51, 51,
        55, 53, 49, 51, 55, 176, 0, 0, 0, 32, 0, 0, 0, 50, 101, 100, 102, 53,
        49, 100, 48, 51, 55, 57, 52, 51, 55, 56, 102, 56, 98, 98, 54, 51, 49,
        48, 100, 52, 54, 48, 99, 50, 50, 98, 49, 1, 0, 0, 0,
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
fn a_transfer_denial_carries_the_token_and_the_reason() {
    let message =
        MessageFactory::build_transfer_denial_message(555, "Cancelled");
    let mut decoded = Message::new_with_data(message.get_buffer());
    assert_eq!(decoded.get_message_code(), 41);
    decoded.set_pointer(8);
    assert_eq!(decoded.read_int32(), 555);
    assert!(!decoded.read_bool());
    assert_eq!(decoded.read_string(), "Cancelled");
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
fn test_build_check_privileges() {
    assert_eq!(
        vec![92, 0, 0, 0],
        MessageFactory::build_check_privileges().get_data()
    );
}

#[test]
fn test_build_place_in_queue_response() {
    let message = MessageFactory::build_place_in_queue_response("song.mp3", 3);
    let expect: Vec<u8> = [
        44, 0, 0, 0, // code
        8, 0, 0, 0, 115, 111, 110, 103, 46, 109, 112, 51, // "song.mp3"
        3, 0, 0, 0, // place
    ]
    .to_vec();
    assert_eq!(expect, message.get_data());
}

#[test]
fn test_build_wishlist_search() {
    let message = MessageFactory::build_wishlist_search(12, "trance wax");
    let expect: Vec<u8> = [
        103, 0, 0, 0, 12, 0, 0, 0, 10, 0, 0, 0, 116, 114, 97, 110, 99, 101, 32,
        119, 97, 120,
    ]
    .to_vec();
    assert_eq!(expect, message.get_data());
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

#[test]
fn test_build_watch_user() {
    let message = MessageFactory::build_watch_user("bob");
    let expect: Vec<u8> = [5, 0, 0, 0, 3, 0, 0, 0, b'b', b'o', b'b'].to_vec();
    assert_eq!(expect, message.get_data());
}

#[test]
fn test_build_unwatch_user() {
    let message = MessageFactory::build_unwatch_user("bob");
    let expect: Vec<u8> = [6, 0, 0, 0, 3, 0, 0, 0, b'b', b'o', b'b'].to_vec();
    assert_eq!(expect, message.get_data());
}

#[test]
fn an_upload_speed_report_carries_the_rate() {
    let message = MessageFactory::build_send_upload_speed(900);
    let mut decoded = Message::new_with_data(message.get_buffer());
    assert_eq!(decoded.get_message_code(), 121);
    decoded.set_pointer(8);
    assert_eq!(decoded.read_int32(), 900);
}

#[test]
fn a_ping_is_just_its_code() {
    let message = MessageFactory::build_server_ping();
    assert_eq!(message.get_data(), [32, 0, 0, 0]);
}

#[test]
fn a_user_search_names_the_user_before_its_token() {
    let message = MessageFactory::build_user_search("bob", 99, "jazz");
    let mut decoded = Message::new_with_data(message.get_buffer());
    assert_eq!(decoded.get_message_code(), 42);
    decoded.set_pointer(8);
    assert_eq!(decoded.read_string(), "bob");
    assert_eq!(decoded.read_int32(), 99);
    assert_eq!(decoded.read_string(), "jazz");
}

#[test]
fn a_room_search_names_the_room_before_its_token() {
    let message = MessageFactory::build_room_search("nicotine", 7, "dub");
    let mut decoded = Message::new_with_data(message.get_buffer());
    assert_eq!(decoded.get_message_code(), 120);
    decoded.set_pointer(8);
    assert_eq!(decoded.read_string(), "nicotine");
    assert_eq!(decoded.read_int32(), 7);
    assert_eq!(decoded.read_string(), "dub");
}

#[test]
fn a_cant_connect_carries_the_token_then_the_user() {
    let message = MessageFactory::build_cant_connect_to_peer(4242, "bob");
    let mut decoded = Message::new_with_data(message.get_buffer());
    assert_eq!(decoded.get_message_code(), 1001);
    decoded.set_pointer(8);
    assert_eq!(decoded.read_int32(), 4242);
    assert_eq!(decoded.read_string(), "bob");
}

#[test]
fn a_room_ticker_carries_its_room_and_text() {
    let message = MessageFactory::build_set_room_ticker("jazz", "hello");
    let mut decoded = Message::new_with_data(message.get_buffer());
    assert_eq!(decoded.get_message_code(), 116);
    decoded.set_pointer(8);
    assert_eq!(decoded.read_string(), "jazz");
    assert_eq!(decoded.read_string(), "hello");
}

#[test]
fn the_global_room_is_joined_and_left_by_code_alone() {
    assert_eq!(
        MessageFactory::build_join_global_room().get_data(),
        [150, 0, 0, 0]
    );
    assert_eq!(
        MessageFactory::build_leave_global_room().get_data(),
        [151, 0, 0, 0]
    );
}

#[test]
fn each_interest_message_uses_its_own_code() {
    for (message, code) in [
        (MessageFactory::build_add_thing_i_like("jazz"), 51),
        (MessageFactory::build_remove_thing_i_like("jazz"), 52),
        (MessageFactory::build_add_thing_i_hate("polka"), 117),
        (MessageFactory::build_remove_thing_i_hate("polka"), 118),
        (MessageFactory::build_user_interests("bob"), 57),
        (MessageFactory::build_item_recommendations("jazz"), 111),
        (MessageFactory::build_item_similar_users("jazz"), 112),
    ] {
        let mut decoded = Message::new_with_data(message.get_buffer());
        assert_eq!(decoded.get_message_code(), code);
        decoded.set_pointer(8);
        assert!(!decoded.read_string().is_empty());
    }
}

#[test]
fn the_bodyless_queries_are_just_their_codes() {
    assert_eq!(
        MessageFactory::build_get_recommendations().get_data(),
        [54, 0, 0, 0]
    );
    assert_eq!(
        MessageFactory::build_global_recommendations().get_data(),
        [56, 0, 0, 0]
    );
    assert_eq!(
        MessageFactory::build_similar_users().get_data(),
        [110, 0, 0, 0]
    );
}

#[test]
fn a_privilege_gift_carries_the_recipient_and_days() {
    let message = MessageFactory::build_give_privileges("bob", 3);
    let mut decoded = Message::new_with_data(message.get_buffer());
    assert_eq!(decoded.get_message_code(), 123);
    decoded.set_pointer(8);
    assert_eq!(decoded.read_string(), "bob");
    assert_eq!(decoded.read_int32(), 3);
}

#[test]
fn a_password_change_carries_the_new_password() {
    let message = MessageFactory::build_change_password("hunter2");
    let mut decoded = Message::new_with_data(message.get_buffer());
    assert_eq!(decoded.get_message_code(), 142);
    decoded.set_pointer(8);
    assert_eq!(decoded.read_string(), "hunter2");
}

#[test]
fn a_multi_user_message_counts_its_recipients_first() {
    let recipients = vec!["alice".to_string(), "bob".to_string()];
    let message = MessageFactory::build_message_users(&recipients, "hi all");
    let mut decoded = Message::new_with_data(message.get_buffer());
    assert_eq!(decoded.get_message_code(), 149);
    decoded.set_pointer(8);
    assert_eq!(decoded.read_int32(), 2);
    assert_eq!(decoded.read_string(), "alice");
    assert_eq!(decoded.read_string(), "bob");
    assert_eq!(decoded.read_string(), "hi all");
}
