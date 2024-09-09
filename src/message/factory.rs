use super::Message;

pub fn build_init_message() -> Message {
    let mut message = Message::new();
    message.write_int32(81); // need to send as first message?
    message
}

pub fn build_login_message(username: &str, password: &str) -> Message {
    let mut hasher = md5::Context::new();

    hasher.consume(username);
    hasher.consume(password);

    let hash = format!("{:x}", hasher.compute());
    let hash_str = hash.as_str();

    let mut message = Message::new();

    message
        .write_int32(1)
        .write_string(username)
        .write_string(password)
        .write_int32(160) // version
        .write_string(hash_str)
        .write_int32(17);

    message
}

pub fn build_shared_folders_message(folder_count: i32, file_count: i32) -> Message {
    let mut message = Message::new();
    message
        .write_int32(35)
        .write_int32(folder_count)
        .write_int32(file_count);

    message
}

#[test]
fn test_build_init_message() {
    let message = build_init_message();
    let expect: Vec<u8> = [81, 0, 0, 0].to_vec();
    assert_eq!(expect, message.get_data())
}

#[test]
fn test_build_login_message() {
    let message = build_login_message("username", "password");

    let expect: Vec<u8> = [
        1, 0, 0, 0, 8, 0, 0, 0, 117, 115, 101, 114, 110, 97, 109, 101, 8, 0, 0, 0, 112, 97, 115,
        115, 119, 111, 114, 100, 160, 0, 0, 0, 32, 0, 0, 0, 100, 53, 49, 99, 57, 97, 55, 101, 57,
        51, 53, 51, 55, 52, 54, 97, 54, 48, 50, 48, 102, 57, 54, 48, 50, 100, 52, 53, 50, 57, 50,
        57, 17, 0, 0, 0,
    ]
    .to_vec();
    assert_eq!(expect, message.get_data())
}
