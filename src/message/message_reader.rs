use std::io::{self, Read};
use std::{collections::VecDeque, net::TcpStream};

use crate::message::Message;

// # why buffered message reader?
// the stream comes in chunks of bytes: 1024 for example
// the soulseek protocol has binary messages
// the first 4 bytes of the message represent the message size
// the rest of the bytes represent the message itself
//
// # how to implement the buffered message reader?
// read 4 bytes from the stream
// if the stream has less than 4 bytes, buffer then add the rest of the bytes to the buffered
// message reader buffer and return
// if the stream has 4 bytes, read the message size
// if the stream is as long as the message size + 4 bytes, read the message
//  create a new message struct and read the message size from the buffer, thats a message
//  the rest of the bytes are the next message so you call the read function again with the rest of the bytes
// else
//  buffer the rest of the bytes and return
//
//
//

pub struct MessageReader {
    buffer: VecDeque<u8>,
}

impl Default for MessageReader {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageReader {
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::new(),
        }
    }

    #[cfg(test)]
    pub fn new_with_buffer(buffer: Vec<u8>) -> Self {
        Self {
            buffer: buffer.into(),
        }
    }

    pub fn read_from_socket(
        &mut self,
        stream: &mut TcpStream,
    ) -> io::Result<()> {
        let mut temp_buffer = [0; 1024]; // Temporary buffer for reading from the socket
        let bytes_read = stream.read(&mut temp_buffer)?;
        if bytes_read == 0 {
            return Ok(());
        }

        // Add the read bytes to the internal buffer
        self.buffer.extend(&temp_buffer[..bytes_read]);

        Ok(())
    }

    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    pub fn extract_message(&mut self) -> io::Result<Option<Message>> {
        let bytes_read = self.buffer.len();
        if bytes_read < 4 {
            return Ok(None);
        }

        let message_size = u32::from_le_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        let total_size = message_size + 4;

        if bytes_read < total_size {
            return Ok(None);
        }

        let message_buffer: Vec<u8> = self.buffer.drain(..total_size).collect();
        Ok(Some(Message::new_with_data(message_buffer)))
    }
}

#[cfg(test)]
mod tests {
    use crate::message::MessageReader;

    #[test]
    fn test_extract_message() {
        let buffer: Vec<u8> = [
            8, 0, 0, 0, 117, 115, 101, 114, 110, 97, 109, 101, 8, 0, 0, 0, 112,
            97, 115, 115, 119, 111, 114, 100, 160, 0, 0, 0, 32, 0, 0, 0, 100,
            53, 49, 99, 57, 97, 55, 101, 57, 51, 53, 51, 55, 52, 54, 97, 54,
            48, 50, 48, 102, 57, 54, 48, 50, 100, 52, 53, 50, 57, 50, 57, 17,
            0, 0, 0,
        ]
        .to_vec();
        let mut buffered_reader = MessageReader::new_with_buffer(buffer);
        let mut message = buffered_reader.extract_message().unwrap().unwrap();
        assert_eq!(
            message.get_data(),
            vec![8, 0, 0, 0, 117, 115, 101, 114, 110, 97, 109, 101]
        );
        assert_eq!(message.read_string(), "username");
    }
    #[test]
    fn test_extract_message_incomplete_message() {
        let incomplete_buffer = vec![1, 2, 3];
        let mut buffered_reader =
            MessageReader::new_with_buffer(incomplete_buffer);

        let result = buffered_reader.extract_message();
        assert_eq!(None, result.unwrap());

        let rest: Vec<u8> = buffered_reader
            .buffer
            .drain(..buffered_reader.buffer.len())
            .collect();

        assert!(buffered_reader.buffer.is_empty());
        assert_eq!(vec![1, 2, 3], rest);
    }
}
