use crate::{message::Message, types::Transfer, utils::md5};

pub struct MessageFactory;
impl MessageFactory {
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
            .write_int32(100); //minor version

        message
    }

    pub fn build_shared_folders_message(folder_count: u32, file_count: u32) -> Message {
        Message::new()
            .write_int32(35)
            .write_int32(folder_count)
            .write_int32(file_count)
            .clone()
    }
    pub fn build_file_search_message(token: u32, query: &str) -> Message {
        // Message::new_with_data(
        //     [
        //         26, 0, 0, 0, 219, 178, 47, 28, 11, 0, 0, 0, 116, 104, 101, 32, 119, 101, 101, 107, 101,
        //         110, 100, 0, 0,
        //     ]
        //     .to_vec(),
        // )
        // .clone()
        Message::new()
            .write_int32(26)
            .write_int32(token)
            .write_string(query)
            .clone()
    }
    pub fn build_set_status_message(status_code: u32) -> Message {
        Message::new()
            .write_int32(28)
            .write_int32(status_code)
            .clone()
    }
    pub fn build_no_parent_message() -> Message {
        Message::new().write_int32(71).write_int32(1).clone()
    }
    pub fn build_set_wait_port_message() -> Message {
        Message::new().write_int32(2).write_int32(2234).clone()
    }
    pub fn build_watch_user(token: u32) -> Message {
        Message::new()
            .write_raw_bytes([5, 0, 0, 0, 0].to_vec())
            .write_int32(token)
            .clone()
    }

    pub fn build_queue_upload_message(filename: &str) -> Message {
        Message::new()
            .write_int32(43)
            .write_string(filename)
            .clone()
    }

    pub fn build_transfer_response_message(transfer: Transfer) -> Message {
        Message::new()
            .write_int32(41)
            .write_int32(transfer.token)
            .write_bool(true)
            .clone()
    }
    pub fn build_pierce_firewall_message(token: u32) -> Message {
        Message::new()
            .write_int32(0)  // PierceFirewall message code
            .write_int32(token)
            .clone()
    }
}

#[test]
fn test_build_watch_user() {
    let token: u32 = 223;
    let message = MessageFactory::build_watch_user(token);
    let expect: Vec<u8> = [5, 0, 0, 0, 0, 91, 88, 21, 0].to_vec();

    assert_eq!(expect, message.get_data())
}

#[test]
fn test_build_login_message() {
    let message = MessageFactory::build_login_message("insane_in_the_brain2", "13375137");

    let expect: Vec<u8> = [
        1, 0, 0, 0, 20, 0, 0, 0, 105, 110, 115, 97, 110, 101, 95, 105, 110, 95, 116, 104, 101, 95,
        98, 114, 97, 105, 110, 50, 8, 0, 0, 0, 49, 51, 51, 55, 53, 49, 51, 55, 157, 0, 0, 0, 32, 0,
        0, 0, 50, 101, 100, 102, 53, 49, 100, 48, 51, 55, 57, 52, 51, 55, 56, 102, 56, 98, 98, 54,
        51, 49, 48, 100, 52, 54, 48, 99, 50, 50, 98, 49, 100, 0, 0, 0,
    ]
    .to_vec();

    // println!("{:?}", print_hex(message.get_data()));
    // assert_eq!(expect, message.get_data());
    assert_eq!(expect, message.get_data())
}

#[test]
fn test_build_file_search_message() {
    let message = MessageFactory::build_file_search_message(12, "trance wax");
    let expect: Vec<u8> = [
        26, 0, 0, 0, 12, 0, 0, 0, 10, 0, 0, 0, 116, 114, 97, 110, 99, 101, 32, 119, 97, 120,
    ]
    .to_vec();
    assert_eq!(expect, message.get_data())
}
