pub use crate::{debug, error, info, trace, warn};

pub mod handlers;
mod message_reader;
pub mod peer;
pub mod server;

pub use handlers::{Handlers, MessageHandler};
pub use message_reader::MessageReader;

use std::{str, usize};

#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub enum MessageType {
    Server,
    Peer,
    PeerInit,
    Distributed,
}

#[derive(Debug)]
pub struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Error {}

#[derive(Debug, PartialEq, Clone)]
pub struct Message {
    data: Vec<u8>,
    pointer: usize,
}

impl Default for Message {
    fn default() -> Self {
        Self::new()
    }
}

impl Message {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            pointer: 0,
        }
    }

    pub fn print_hex(&self) {
        let data = &self.data;
        const BYTES_PER_LINE: usize = 16;

        let chunks = data.chunks(BYTES_PER_LINE);
        for (i, chunk) in chunks.enumerate() {
            // Print the offset
            print!("{:04x}  ", i * BYTES_PER_LINE);

            // Print the hexadecimal part
            for j in 0..BYTES_PER_LINE {
                if j < chunk.len() {
                    print!("{:02x} ", chunk[j]);
                } else {
                    print!(" ");
                }

                // Add extra space in the middle
                if j == 7 {
                    print!(" ");
                }
            }

            print!("  ");

            // Print the ASCII part
            let mut i = 0;
            for &byte in chunk {
                i += 1;
                if byte.is_ascii_graphic() || byte.is_ascii_whitespace() {
                    print!("{}", byte as char);
                } else {
                    print!(".");
                }

                if i == 8 {
                    print!(" ");
                }
            }

            trace!("");
        }
    }
    //
    // pub fn print_hex2(&self) -> String {
    //     let data = &self.data;
    //     let mut out = String::from("");
    //     const BYTES_PER_LINE: usize = 16;
    //
    //     let chunks = data.chunks(BYTES_PER_LINE);
    //     for (i, chunk) in chunks.enumerate() {
    //         // Print the offset
    //         // out += &format!("{:04x}  ", i * BYTES_PER_LINE);
    //
    //         // Print the hexadecimal part
    //         for j in 0..BYTES_PER_LINE {
    //             if j < chunk.len() {
    //                 out += &format!("{:02x} ", chunk[j]);
    //             } else {
    //                 // out += &format!(" ");
    //             }
    //
    //             // Add extra space in the middle
    //             // if j == 7 {
    //             //     out += &format!(" ");
    //             // }
    //         }
    //
    //         // // Print the ASCII part
    //         // let mut i = 0;
    //         // for &byte in chunk {
    //         //     i = i + 1;
    //         //     if byte.is_ascii_graphic() || byte.is_ascii_whitespace() {
    //         //         print!("{}", byte as char);
    //         //     } else {messa
    //         //         print!(".");
    //         //     }
    //         //
    //         //     if i == 8 {
    //         //         print!(" ");
    //         //     }
    //         // }
    //         //
    //     }
    //     println!("{:?}", out.trim());
    //     return out.trim().into();
    // }

    #[allow(dead_code)]
    pub fn get_message_code_u32(&self) -> u32 {
        u32::from_le_bytes(self.data[4..8].try_into().unwrap())
    }

    pub fn get_message_code(&self) -> u8 {
        self.data[4]
    }

    pub fn new_with_data(data: Vec<u8>) -> Self {
        Self { data, pointer: 0 }
    }
    #[allow(dead_code)]
    pub fn reset_pointer(&mut self) {
        self.pointer = 0;
    }
    pub fn set_pointer(&mut self, pointer: usize) {
        self.pointer = pointer;
    }

    pub fn get_pointer(&mut self) -> usize {
        self.pointer
    }

    pub fn get_size(&mut self) -> usize {
        self.data.len()
    }

    pub fn get_data(&self) -> Vec<u8> {
        self.data.clone()
    }

    pub fn get_slice(&self, from: usize, to: usize) -> Vec<u8> {
        self.data[from..to].to_vec()
    }

    /// gets buffer with the message length prepended
    pub fn get_buffer(&self) -> Vec<u8> {
        let mut b = vec![0u8; 4];
        let length = self.data.len() as u32;
        b[0..4].copy_from_slice(&length.to_le_bytes());
        let mut combined = b;
        combined.extend(&self.data);
        combined
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

    pub fn read_int8(&mut self) -> u8 {
        let val = self.data[self.pointer];
        self.pointer += 1;
        val
    }

    #[allow(dead_code)]
    pub fn read_int64(&mut self) -> u64 {
        let val = u64::from_le_bytes([
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

    pub fn read_raw_byte(&mut self) -> Vec<u8> {
        if self.pointer + 4 > self.data.len() {
            return vec![];
        }

        let val = vec![
            self.data[self.pointer],
            self.data[self.pointer + 1],
            self.data[self.pointer + 2],
            self.data[self.pointer + 3],
        ];
        self.pointer += 4;
        val
    }

    pub fn read_int32(&mut self) -> u32 {
        if self.pointer + 4 > self.data.len() {
            return 0;
        }

        let val = u32::from_le_bytes([
            self.data[self.pointer],
            self.data[self.pointer + 1],
            self.data[self.pointer + 2],
            self.data[self.pointer + 3],
        ]);
        self.pointer += 4;
        val
    }

    pub fn read_bool(&mut self) -> bool {
        let val = self.data[self.pointer] == 1;
        self.pointer += 1;
        val
    }

    pub fn write_string(&mut self, val: &str) -> &mut Self {
        let length = val.len() as u32;
        self.data.extend_from_slice(&length.to_le_bytes());
        self.data.extend_from_slice(val.as_bytes());
        self
    }

    pub fn write_int8(&mut self, value: u8) -> &mut Self {
        self.data.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn write_int32(&mut self, value: u32) -> &mut Self {
        self.data.extend_from_slice(&value.to_le_bytes());
        self
    }

    #[allow(dead_code)]
    pub fn write_int64(&mut self, value: u64) -> &mut Self {
        self.data.extend_from_slice(&value.to_le_bytes());
        self
    }
    pub fn write_raw_bytes(&mut self, value: Vec<u8>) -> &mut Self {
        self.data.extend_from_slice(&value);
        self
    }

    pub fn write_bool(&mut self, value: bool) -> &mut Self {
        self.data.push(if value { 1 } else { 0 });
        self
    }

    #[allow(dead_code)]
    pub fn write_raw_hex_string(&mut self, val: &str) -> &mut Self {
        let mut b = Vec::new();
        for i in (0..val.len()).step_by(2) {
            let byte_str = &val[i..i + 2];
            let byte =
                u8::from_str_radix(byte_str, 16).expect("Invalid hex string");
            b.push(byte);
        }
        self.data.extend_from_slice(&b);
        self.pointer += b.len();
        self
    }

    pub fn get_message_name(
        &self,
        msg_type: MessageType,
        code: u32,
    ) -> Result<&str, Error> {
        match msg_type {
            MessageType::Server => match code {
                1 => Ok("Login"),
                2 => Ok("SetWaitPort"),
                3 => Ok("GetPeerAddress"),
                5 => Ok("WatchUser"),
                6 => Ok("UnwatchUser"),
                7 => Ok("GetUserStatus"),
                13 => Ok("SayChatroom"),
                14 => Ok("JoinRoom"),
                15 => Ok("LeaveRoom"),
                18 => Ok("ConnectToPeer"),
                22 => Ok("MessageUser"),
                23 => Ok("MessageAcked"),
                26 => Ok("FileSearch"),
                28 => Ok("SetStatus"),
                32 => Ok("ServerPing"),
                35 => Ok("SharedFoldersFiles"),
                36 => Ok("GetUserStats"),
                41 => Ok("Relogged"),
                42 => Ok("UserSearch"),
                64 => Ok("RoomList"),
                92 => Ok("CheckPrivileges"),
                100 => Ok("AcceptChildren"),
                102 => Ok("PossibleParents"),
                1001 => Ok("CantConnectToPeer"),
                _ => {
                    Err(Error(format!("Unknown server message code: {}", code)))
                }
            },
            MessageType::PeerInit => match code {
                0 => Ok("PierceFireWall"),
                1 => Ok("PeerInit"),
                _ => Err(Error(format!(
                    "Unknown peer init message code: {}",
                    code
                ))),
            },
            MessageType::Peer => match code {
                4 => Ok("GetShareFileList"),
                5 => Ok("SharedFileListResponse"),
                9 => Ok("FileSearchResponse"),
                15 => Ok("UserInfoRequest"),
                16 => Ok("UserInfoResponse"),
                36 => Ok("FolderContentsRequest"),
                37 => Ok("FolderContentsResponse"),
                40 => Ok("TransferRequest"),
                41 => Ok("TransferResponse"),
                43 => Ok("QueueUpload"),
                44 => Ok("PlaceInQueueResponse"),
                46 => Ok("UploadFailed"),
                50 => Ok("UploadDenied"),
                51 => Ok("PlaceInQueueRequest"),
                _ => Err(Error(format!("Unknown peer message code: {}", code))),
            },
            MessageType::Distributed => match code {
                3 => Ok("SearchRequest"),
                4 => Ok("BranchLevel"),
                5 => Ok("BranchRoot"),
                93 => Ok("EmbeddedMessage"),
                _ => Err(Error(format!(
                    "Unknown distributed message code: {}",
                    code
                ))),
            },
        }
    }
    // pub fn decode(&self) {
    //     println!("{:?}", self.data);
    // }
}

#[test]
fn test_get_buffer() {
    let message = Message::new_with_data(
        [
            26, 0, 0, 0, 219, 178, 47, 28, 11, 0, 0, 0, 116, 104, 101, 32, 119,
            101, 101, 107, 101, 110, 100,
        ]
        .to_vec(),
    );
    assert_eq!(
        message.get_buffer(),
        [
            23, 0, 0, 0, 26, 0, 0, 0, 219, 178, 47, 28, 11, 0, 0, 0, 116, 104,
            101, 32, 119, 101, 101, 107, 101, 110, 100,
        ]
        .to_vec()
    );

    let message = Message::new_with_data(
        [
            1, 0, 0, 0, 20, 0, 0, 0, 105, 110, 115, 97, 110, 101, 95, 105, 110,
            95, 116, 104, 101, 95, 98, 114, 97, 105, 110, 50, 8, 0, 0, 0, 49,
            51, 51, 55, 53, 49, 51, 55, 160, 0, 0, 0, 32, 0, 0, 0, 50, 101,
            100, 102, 53, 49, 100, 48, 51, 55, 57, 52, 51, 55, 56, 102, 56, 98,
            98, 54, 51, 49, 48, 100, 52, 54, 48, 99, 50, 50, 98, 49, 17, 0, 0,
            0,
        ]
        .to_vec(),
    );
    assert_eq!(
        message.get_buffer(),
        [
            84, 0, 0, 0, 1, 0, 0, 0, 20, 0, 0, 0, 105, 110, 115, 97, 110, 101,
            95, 105, 110, 95, 116, 104, 101, 95, 98, 114, 97, 105, 110, 50, 8,
            0, 0, 0, 49, 51, 51, 55, 53, 49, 51, 55, 160, 0, 0, 0, 32, 0, 0, 0,
            50, 101, 100, 102, 53, 49, 100, 48, 51, 55, 57, 52, 51, 55, 56,
            102, 56, 98, 98, 54, 51, 49, 48, 100, 52, 54, 48, 99, 50, 50, 98,
            49, 17, 0, 0, 0,
        ]
        .to_vec()
    );
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
        50, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 81, 170, 162, 77, 32, 0, 0, 0,
        101, 102, 99, 97, 51, 52, 102, 99, 52, 99, 56, 98, 101, 56, 98, 55,
        101, 102, 51, 56, 97, 102, 50, 54, 50, 52, 100, 101, 53, 52, 54, 52, 0,
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
