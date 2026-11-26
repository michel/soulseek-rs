use std::io::{self, Read};
use std::{collections::VecDeque, net::TcpStream};

use crate::message::Message;

// Soulseek messages are length-prefixed (u32 LE size, then payload). TCP gives us
// arbitrary-sized chunks, so we accumulate into a buffer and only emit a Message
// once size + 4 bytes are available.

pub struct MessageReader {
    buffer: VecDeque<u8>,
    /// The largest payload a frame may declare, when the other end is not
    /// trusted to declare a sane one.
    max_frame: Option<usize>,
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
            max_frame: None,
        }
    }

    /// Refuse any frame declaring more than `max` bytes of payload.
    ///
    /// The length prefix is whatever the other end wrote. Without a limit a
    /// frame declaring four gigabytes is simply waited for, and everything
    /// sent towards it is buffered: one connection can hold as much memory
    /// as its peer cares to send, for as long as it keeps sending.
    pub const fn limit_frames(&mut self, max: usize) {
        self.max_frame = Some(max);
    }

    #[cfg(test)]
    #[must_use]
    pub fn new_with_buffer(buffer: Vec<u8>) -> Self {
        Self {
            buffer: buffer.into(),
            max_frame: None,
        }
    }

    pub fn read_from_socket(
        &mut self,
        stream: &mut TcpStream,
    ) -> io::Result<()> {
        let mut temp_buffer = [0; 1024]; // Temporary buffer for reading from the socket
        let bytes_read = stream.read(&mut temp_buffer)?;
        // On a TCP socket a zero-byte read is the other end hanging up. Passing
        // that back as success left the caller polling a dead socket forever,
        // which is how a server that closes cleanly went unnoticed.
        if bytes_read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the remote end closed the connection",
            ));
        }

        // Add the read bytes to the internal buffer
        self.buffer.extend(&temp_buffer[..bytes_read]);

        Ok(())
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

        if let Some(max) = self.max_frame
            && message_size > max
        {
            // Nothing in the buffer can be framed any more; let it go.
            self.buffer = VecDeque::new();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "frame declares {message_size} bytes, over the \
                     {max}-byte limit"
                ),
            ));
        }

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

    // Issue #115: the declared length was trusted, so a peer announcing a
    // huge frame had everything it then sent buffered while the reader
    // waited for the rest.
    #[test]
    fn a_frame_over_the_limit_is_refused_instead_of_waited_for() {
        let mut declared_4gib = u32::MAX.to_le_bytes().to_vec();
        declared_4gib.extend([0u8; 64]);

        let mut trusting =
            MessageReader::new_with_buffer(declared_4gib.clone());
        assert_eq!(
            trusting.extract_message().unwrap(),
            None,
            "without a limit the reader waits for four gigabytes"
        );

        let mut limited = MessageReader::new_with_buffer(declared_4gib);
        limited.limit_frames(1024);
        let refused = limited.extract_message().unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_frame_at_the_limit_still_arrives() {
        let mut frame = 8u32.to_le_bytes().to_vec();
        frame.extend([7u8; 8]);
        let mut reader = MessageReader::new_with_buffer(frame);
        reader.limit_frames(8);
        assert!(reader.extract_message().unwrap().is_some());
    }

    /// A hangup has to reach the caller as an error. Reported as success it
    /// leaves the actors polling a socket that will never speak again, which is
    /// how a server that closed cleanly went unnoticed.
    #[test]
    fn a_closed_connection_reads_as_an_error_not_as_success() {
        use std::io::Write;
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut ours = TcpStream::connect(addr).unwrap();
        let (mut theirs, _) = listener.accept().unwrap();

        theirs.write_all(&[1, 2, 3]).unwrap();
        theirs.flush().unwrap();
        drop(theirs);

        let mut reader = MessageReader::new();
        // Whatever was sent before the close still arrives.
        while reader.buffer_len() < 3 {
            reader.read_from_socket(&mut ours).unwrap();
        }
        let error = reader
            .read_from_socket(&mut ours)
            .expect_err("a closed connection is not a successful read");
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
