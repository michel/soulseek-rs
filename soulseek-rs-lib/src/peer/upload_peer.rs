//! Serving a shared file to a peer over an F (file transfer) connection.

use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
/// `bytes_sent` is updated as the transfer progresses, and setting `cancel`
/// aborts the stream with an [`io::ErrorKind::Interrupted`] error.
///
/// # Errors
/// Returns any I/O error opening the file or talking to the peer.
pub fn serve_file(
    host: &str,
    port: u32,
    own_username: &str,
    token: u32,
    path: &Path,
    bytes_sent: &AtomicU64,
    cancel: &AtomicBool,
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

    let mut buffer = [0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "upload cancelled",
            ));
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        stream.write_all(&buffer[..read])?;
        bytes_sent.fetch_add(read as u64, Ordering::Relaxed);
    }
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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

        let bytes_sent = Arc::new(AtomicU64::new(0));
        let sent_counter = bytes_sent.clone();
        let uploader = std::thread::spawn(move || {
            serve_file(
                "127.0.0.1",
                port,
                "me",
                777,
                &path,
                &sent_counter,
                &AtomicBool::new(false),
            )
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
        assert_eq!(bytes_sent.load(Ordering::Relaxed), 4096);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn serve_file_stops_when_cancelled() {
        let dir = std::env::temp_dir()
            .join(format!("soulseek-upload-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.bin");
        std::fs::write(&path, vec![7u8; 1024 * 1024]).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = u32::from(listener.local_addr().unwrap().port());

        // Cancelled before the copy loop starts: the stream must abort with
        // Interrupted instead of serving the whole file.
        let cancel = Arc::new(AtomicBool::new(true));
        let cancel_flag = cancel.clone();
        let uploader = std::thread::spawn(move || {
            serve_file(
                "127.0.0.1",
                port,
                "me",
                778,
                &path,
                &AtomicU64::new(0),
                &cancel_flag,
            )
        });

        let (mut stream, _) = listener.accept().unwrap();
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).unwrap();
        let mut payload = vec![0u8; u32::from_le_bytes(len_buf) as usize];
        stream.read_exact(&mut payload).unwrap();
        let mut token = [0u8; 4];
        stream.read_exact(&mut token).unwrap();
        stream.write_all(&[0u8; 8]).unwrap();
        stream.flush().unwrap();

        let mut received = Vec::new();
        let _ = stream.read_to_end(&mut received);
        assert!(received.is_empty(), "no file bytes after cancellation");

        let err = uploader.join().unwrap().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
        let _ = std::fs::remove_dir_all(dir);
    }
}
