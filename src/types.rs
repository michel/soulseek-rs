use std::collections::HashMap;

use crate::{message::Message, utils::zlib::deflate};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct File {
    pub username: String,
    pub name: String,
    pub size: i32,
    pub attribs: HashMap<i32, i32>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FileSearchResult {
    pub token: String,
    pub files: Vec<File>,
    pub slots: i8,
    pub speed: i32,
}

impl FileSearchResult {
    pub fn new_from_message(message: &mut Message) -> Self {
        let pointer = message.get_pointer();
        let size = message.get_size();
        let data: Vec<u8> = message.get_slice(pointer, size);
        let deflated = deflate(&data).unwrap();
        let mut message = Message::new_with_data(deflated);

        let username = message.read_string();
        let token = message.read_raw_hex_str(4);
        let n_files = message.read_int32();
        let mut files: Vec<File> = Vec::new();
        for _ in 0..n_files {
            message.read_int8();
            let name = message.read_string();
            let size = message.read_int32();
            message.read_int32();
            message.read_string();
            let n_attribs = message.read_int32();
            let mut attribs: HashMap<i32, i32> = HashMap::new();

            for _ in 0..n_attribs {
                attribs.insert(message.read_int32(), message.read_int32());
            }
            files.push(File {
                username: username.clone(),
                name,
                size,
                attribs,
            });
        }
        let slots = message.read_int8();
        let speed = message.read_int32();

        Self {
            token,
            files,
            slots,
            speed,
        }
    }
}
pub struct Transfer {
    pub direction: i32,
    pub token: i32,
    pub filename: String,
    pub size: i64,
}
impl Transfer {
    pub fn new_from_message(message: &mut Message) -> Self {
        let direction = message.read_int32();
        let token = message.read_int32();
        let filename = message.read_string();
        let size = message.read_int64();

        Self {
            direction,
            token,
            filename,
            size,
        }
    }
}
