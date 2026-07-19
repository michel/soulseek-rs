//! Serving a shared file to a peer over an F (file transfer) connection.

use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use crate::message::server::MessageFactory;
use crate::peer::ConnectionType;
use crate::trace;

/// Connect to the downloader's file listener and stream `path`'s bytes.
///
/// We announce ourselves with a `PeerInit(F)` immediately followed by the raw
/// transfer token (so it lands in the downloader's read buffer, where the
/// download is matched by token), then the downloader sends an 8-byte
/// START_DOWNLOAD offset before we stream the file.
///
/// # Errors
/// Returns any I/O error opening the file or talking to the peer.
pub fn serve_file(
    host: &str,
    port: u32,
    own_username: &str,
    token: u32,
    path: &Path,
) -> io::Result<()> {
    let mut file = File::open(path)?;

    let socket = format!("{host}:{port}")
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "no address")
        })?;
    let mut stream =
        TcpStream::connect_timeout(&socket, Duration::from_secs(20))?;
    stream.set_nodelay(true).ok();

    // PeerInit(F) + the 4-byte token in a single write so they coalesce.
    let mut init = MessageFactory::build_peer_init_message(
        own_username,
        ConnectionType::F,
        token,
    )
    .get_buffer();
    init.extend_from_slice(&token.to_le_bytes());
    stream.write_all(&init)?;
    stream.flush()?;

    // The downloader replies with an 8-byte START_DOWNLOAD offset first.
    let mut offset = [0u8; 8];
    stream.read_exact(&mut offset)?;

    io::copy(&mut file, &mut stream)?;
    stream.flush()?;

    // Linger so the downloader drains everything before the socket closes.
    std::thread::sleep(Duration::from_millis(500));
    trace!("[upload] served {} to {}:{}", path.display(), host, port);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::serve_file;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn serve_file_streams_the_file_over_an_f_connection() {
        let content: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let dir = std::env::temp_dir()
            .join(format!("soulseek-upload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("probe.bin");
        std::fs::write(&path, &content).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = u32::from(listener.local_addr().unwrap().port());

        let uploader = std::thread::spawn(move || {
            serve_file("127.0.0.1", port, "me", 777, &path)
        });

        // Downloader side: accept, read PeerInit(F) frame + raw token, send the
        // 8-byte START_DOWNLOAD offset, then read the streamed bytes.
        let (mut stream, _) = listener.accept().unwrap();
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).unwrap();
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).unwrap();
        let mut token = [0u8; 4];
        stream.read_exact(&mut token).unwrap();
        assert_eq!(u32::from_le_bytes(token), 777);

        stream.write_all(&[0u8; 8]).unwrap();
        stream.flush().unwrap();

        let mut received = Vec::new();
        stream.read_to_end(&mut received).unwrap();
        assert_eq!(received, content);

        let _ = uploader.join();
        let _ = std::fs::remove_dir_all(dir);
    }
}
