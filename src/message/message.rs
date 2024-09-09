use std::str;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum MessageType {
    Login,
    PrivilegedUsers,
    ExcludedSearchPhrases,
    RoomList,
    Unknown(u8),
}
impl From<u8> for MessageType {
    fn from(value: u8) -> Self {
        match value {
            1 => MessageType::Login,
            69 => MessageType::PrivilegedUsers,
            160 => MessageType::ExcludedSearchPhrases,
            64 => MessageType::RoomList,
            _ => MessageType::Unknown(value),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Message {
    data: Vec<u8>,
    pointer: usize,
}

impl Message {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            pointer: 0,
        }
    }

    pub fn get_message_type(&self) -> MessageType {
        MessageType::from(self.data[4])
    }

    pub fn new_with_data(data: Vec<u8>) -> Self {
        Self { data, pointer: 0 }
    }
    // pub fn reset_pointer(&mut self) {
    //     self.pointer = 0;
    // }
    pub fn set_pointer(&mut self, pointer: usize) {
        self.pointer = pointer;
    }

    pub fn get_data(&self) -> Vec<u8> {
        self.data.clone()
    }

    pub fn read_string(&mut self) -> String {
        let size = u32::from_le_bytes([
            self.data[self.pointer],
            self.data[self.pointer + 1],
            self.data[self.pointer + 2],
            self.data[self.pointer + 3],
        ]) as usize;

        self.pointer += 4;
        let data = &self.data[self.pointer..self.pointer + size];
        self.pointer += size;

        String::from_utf8(data.to_vec()).expect("Failed to read string")
    }

    pub fn read_int8(&mut self) -> i8 {
        let val = self.data[self.pointer] as i8;
        self.pointer += 1;
        val
    }

    pub fn read_int64(&mut self) -> i64 {
        let val = i64::from_le_bytes([
            self.data[self.pointer],
            self.data[self.pointer + 1],
            self.data[self.pointer + 2],
            self.data[self.pointer + 3],
            self.data[self.pointer + 4],
            self.data[self.pointer + 5],
            self.data[self.pointer + 6],
            self.data[self.pointer + 7],
        ]);
        self.pointer += 8;
        val
    }

    pub fn read_int32(&mut self) -> i32 {
        if self.pointer + 4 > self.data.len() {
            return 0;
        }

        let val = i32::from_le_bytes([
            self.data[self.pointer],
            self.data[self.pointer + 1],
            self.data[self.pointer + 2],
            self.data[self.pointer + 3],
        ]);
        self.pointer += 4;
        val
    }

    pub fn write_string(&mut self, val: &str) -> &mut Self {
        let length = val.len() as u32;
        self.data.extend_from_slice(&length.to_le_bytes());
        self.data.extend_from_slice(val.as_bytes());
        self
    }

    pub fn write_int32(&mut self, value: i32) -> &mut Self {
        self.data.extend_from_slice(&value.to_le_bytes());
        self
    }

    // pub fn decode(&self) {
    //     let size =
    //         u32::from_le_bytes([self.data[0], self.data[1], self.data[2], self.data[3]]) as usize;
    //     println!("Size: {}", size);
    //     if size >= 4 {
    //         let code = u32::from_le_bytes([self.data[4], self.data[5], self.data[6], self.data[7]]);
    //         println!("Code: {}", code);
    //     }
    // }
}

#[test]
fn test_read_string() {
    let data = vec![
        5, 0, 0, 0, // size = 5
        72, 101, 108, 108, 111, // "Hello"
    ];
    let mut test_data = Message::new_with_data(data);
    assert_eq!(test_data.read_string(), "Hello");
}

#[test]
fn test_read_string_2() {
    let data = vec![
        50, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 81, 170, 162, 77, 32, 0, 0, 0, 101, 102, 99, 97,
        51, 52, 102, 99, 52, 99, 56, 98, 101, 56, 98, 55, 101, 102, 51, 56, 97, 102, 50, 54, 50,
        52, 100, 101, 53, 52, 54, 52, 0,
    ];
    let mut test_data = Message::new_with_data(data);
    test_data.set_pointer(9);
    assert_eq!(test_data.read_string(), "");
}

#[test]
#[should_panic(expected = "Failed to read string")]
fn test_read_string_invalid_utf8() {
    let data = vec![
        3, 0, 0, 0, // size = 3
        255, 255, 255, // invalid UTF-8 sequence
    ];
    let mut test_data = Message::new_with_data(data);
    test_data.read_string();
}
