use std::collections::VecDeque;
use std::io::{self, Read};

use crate::message::Message;

// Soulseek messages are length-prefixed (u32 LE size, then payload). TCP gives us
// arbitrary-sized chunks, so we accumulate into a buffer and only emit a Message
// once size + 4 bytes are available.

pub struct MessageReader {
    buffer: VecDeque<u8>,
}

impl Default for MessageReader {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageReader {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffer: VecDeque::new(),
        }
    }

    #[cfg(test)]
    #[must_use]
    pub fn new_with_buffer(buffer: Vec<u8>) -> Self {
        Self {
            buffer: buffer.into(),
        }
    }

    /// Drain the (non-blocking) socket into the internal buffer until it
    /// reports `WouldBlock`. mio readiness is edge-triggered: stopping after a
    /// partial read would strand the rest in the kernel buffer with no further
    /// event. A remote close (0-byte read) surfaces as `UnexpectedEof` so the
    /// caller can tear the connection down; bytes received before the close
    /// remain buffered for extraction.
    pub fn read_from_socket<R: Read>(
        &mut self,
        stream: &mut R,
    ) -> io::Result<()> {
        let mut temp_buffer = [0; 4096];
        loop {
            match stream.read(&mut temp_buffer) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed by peer",
                    ));
                }
                Ok(bytes_read) => {
                    self.buffer.extend(&temp_buffer[..bytes_read]);
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        }
    }

    #[must_use]
    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    pub fn get_buffer(&mut self) -> Vec<u8> {
        self.buffer.drain(..).collect()
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
    fn read_from_socket_drains_until_would_block_and_reports_eof() {
        use std::io::Read;

        // A stream yielding two chunks, then WouldBlock, then EOF — the shape
        // a non-blocking socket presents under edge-triggered readiness.
        struct Script(Vec<std::io::Result<Vec<u8>>>);
        impl Read for Script {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                match self.0.remove(0) {
                    Ok(bytes) => {
                        buf[..bytes.len()].copy_from_slice(&bytes);
                        Ok(bytes.len())
                    }
                    Err(e) => Err(e),
                }
            }
        }

        let mut reader = MessageReader::new();
        let mut stream = Script(vec![
            Ok(vec![1; 10]),
            Ok(vec![2; 5]),
            Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
            Ok(vec![3; 4]),
            Ok(Vec::new()), // EOF
        ]);

        // First call drains both chunks and stops at WouldBlock.
        reader.read_from_socket(&mut stream).unwrap();
        assert_eq!(reader.buffer_len(), 15);

        // Second call reads the tail, then surfaces the close as
        // UnexpectedEof while keeping the bytes received before it.
        let err = reader.read_from_socket(&mut stream).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
        assert_eq!(reader.buffer_len(), 19);
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
