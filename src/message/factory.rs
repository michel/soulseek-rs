use super::Message;
use crate::utils::md5;

pub fn build_init_message() -> Message {
    let mut message = Message::new();
    message.write_int32(84); // secret init message code (needed for version check)?
    message.print_hex();

    message
}

pub fn build_login_message(username: &str, password: &str) -> Message {
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

pub fn build_shared_folders_message(folder_count: i32, file_count: i32) -> Message {
    Message::new()
        .write_int32(35)
        .write_int32(folder_count)
        .write_int32(file_count)
        .clone()
}
pub fn build_file_search_message(token: &str, query: &str) -> Message {
    Message::new()
        .write_int32(26)
        .write_string(token)
        .write_string(query)
        .clone()
}
#[test]
fn test_build_init_message() {
    let message = build_init_message();
    let expect: Vec<u8> = [81, 0, 0, 0].to_vec();
    assert_eq!(expect, message.get_data())
}

#[test]
fn test_build_login_message() {
    let message = build_login_message("insane_in_the_brain2", "13375137");

    let expect: Vec<u8> = [
        1, 0, 0, 0, 8, 0, 0, 0, 117, 115, 101, 114, 110, 97, 109, 101, 8, 0, 0, 0, 112, 97, 115,
        115, 119, 111, 114, 100, 200, 0, 0, 0, 32, 0, 0, 0, 100, 53, 49, 99, 57, 97, 55, 101, 57,
        51, 53, 51, 55, 52, 54, 97, 54, 48, 50, 48, 102, 57, 54, 48, 50, 100, 52, 53, 50, 57, 50,
        57, 17, 0, 0, 0,
    ]
    .to_vec();
    assert_eq!(expect, message.get_data())
}

fn test_build_file_search_message() {
    let message = build_file_search_message("token", "trance wax");
    let expect: Vec<u8> = [
        26, 0, 0, 0, 116, 111, 107, 101, 110, 113, 117, 101, 114, 121, 0, 0, 0, 113, 117, 101, 114,
        121,
    ]
    .to_vec();
    assert_eq!(expect, message.get_data())
}
