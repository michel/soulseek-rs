//! Serving a shared file to a peer over an F (file transfer) connection.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

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
/// aborts the stream with an [`io::ErrorKind::Interrupted`] error. Returns the
/// bytes streamed by this call, which a resumed transfer's prefix is not.
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
) -> io::Result<u64> {
    let mut file = File::open(path)?;

    let socket = format!("{host}:{port}")
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "no address")
        })?;
    let mut stream =
        TcpStream::connect_timeout(&socket, Duration::from_secs(20))?;
    // A downloader that connects and then goes silent must cost seconds, not
    // the rest of the session: without deadlines a wedged `read_exact` (or a
    // zero-window `write_all`) holds this thread, the socket, the open file,
    // and — because the upload stays InProgress — an upload slot, forever.
    // Thirty seconds of total silence is a dead transfer, not a slow one: even
    // a slow reader drains some buffer within that. The write deadline is
    // deliberately looser than the download side's 5s: bulk data flows this
    // way, and a congested but living peer may hold the send buffer full far
    // longer than a peer draining our tiny control frames ever would.
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
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

    // The downloader replies with an 8-byte START_DOWNLOAD offset first. A
    // non-zero offset means it is resuming and already holds that prefix, so
    // seek past it and count it as sent to keep progress absolute.
    let mut offset = [0u8; 8];
    stream.read_exact(&mut offset)?;
    let offset = u64::from_le_bytes(offset);
    let file_size = file.metadata()?.len();
    if offset > file_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("download offset {offset} exceeds file size {file_size}"),
        ));
    }
    file.seek(SeekFrom::Start(offset))?;
    bytes_sent.fetch_add(offset, Ordering::Relaxed);

    let mut buffer = vec![0u8; 64 * 1024];
    let mut streamed = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        stream.write_all(&buffer[..read])?;
        bytes_sent.fetch_add(read as u64, Ordering::Relaxed);
        streamed += read as u64;
    }
    stream.flush()?;

    // Soulseek assigns closing the file-transfer connection to the downloader
    // after it has read exactly the advertised number of bytes. Poll while
    // waiting for that EOF so cancelling a finished-but-unacknowledged upload
    // remains prompt rather than waiting for the full socket timeout.
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    let close_deadline = Instant::now() + Duration::from_secs(30);
    let mut unexpected = [0u8; 1];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        match stream.read(&mut unexpected) {
            Ok(0) => break,
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected data after download offset",
                ));
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if Instant::now() >= close_deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "downloader did not close the file connection",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }
    trace!("[upload] served {} to {}:{}", path.display(), host, port);
    Ok(streamed)
}

#[cfg(test)]
mod tests {
    use super::serve_file;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    /// Play the downloader: accept the F connection, drain the PeerInit frame
    /// and the raw token, then reply with the 8-byte START_DOWNLOAD offset.
    /// Returns the connected stream and the token the uploader announced.
    fn accept_and_send_offset(
        listener: &TcpListener,
        offset: u64,
    ) -> (TcpStream, u32) {
        let (mut stream, _) = listener.accept().unwrap();
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).unwrap();
        let mut payload = vec![0u8; u32::from_le_bytes(len_buf) as usize];
        stream.read_exact(&mut payload).unwrap();
        let mut token = [0u8; 4];
        stream.read_exact(&mut token).unwrap();

        stream.write_all(&offset.to_le_bytes()).unwrap();
        stream.flush().unwrap();
        (stream, u32::from_le_bytes(token))
    }

    fn scratch_file(name: &str, content: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("soulseek-upload-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("probe.bin");
        std::fs::write(&path, content).unwrap();
        path
    }

    fn spawn_uploader(
        listener: &TcpListener,
        path: std::path::PathBuf,
        token: u32,
        cancel: Arc<AtomicBool>,
    ) -> (
        Arc<AtomicU64>,
        std::thread::JoinHandle<std::io::Result<u64>>,
    ) {
        let port = u32::from(listener.local_addr().unwrap().port());
        let bytes_sent = Arc::new(AtomicU64::new(0));
        let sent_counter = bytes_sent.clone();
        let uploader = std::thread::spawn(move || {
            serve_file(
                "127.0.0.1",
                port,
                "me",
                token,
                &path,
                &sent_counter,
                &cancel,
            )
        });
        (bytes_sent, uploader)
    }

    #[test]
    fn serve_file_streams_the_file_over_an_f_connection() {
        let content: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let path = scratch_file("stream", &content);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dir = path.parent().unwrap().to_path_buf();
        let (bytes_sent, uploader) =
            spawn_uploader(&listener, path, 777, Arc::default());

        let (mut stream, token) = accept_and_send_offset(&listener, 0);
        assert_eq!(token, 777);

        let mut received = vec![0; content.len()];
        stream.read_exact(&mut received).unwrap();
        assert_eq!(received, content);

        assert!(
            !uploader.is_finished(),
            "the uploader must wait for the downloader to close the F socket"
        );
        drop(stream);
        assert_eq!(uploader.join().unwrap().unwrap(), 4096);
        assert_eq!(bytes_sent.load(Ordering::Relaxed), 4096);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn serve_file_accepts_valid_resume_offsets() {
        let content: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        for offset in [1000, content.len()] {
            let path =
                scratch_file(&format!("valid-offset-{offset}"), &content);
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let dir = path.parent().unwrap().to_path_buf();
            let (bytes_sent, uploader) =
                spawn_uploader(&listener, path, 779, Arc::default());

            let (mut stream, _) =
                accept_and_send_offset(&listener, offset as u64);
            let mut received = vec![0; content.len() - offset];
            stream.read_exact(&mut received).unwrap();
            assert_eq!(received, content[offset..]);
            drop(stream);

            assert_eq!(
                uploader.join().unwrap().unwrap(),
                (content.len() - offset) as u64
            );
            assert_eq!(
                bytes_sent.load(Ordering::Relaxed),
                content.len() as u64
            );
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn serve_file_rejects_an_offset_past_end_of_file() {
        let content: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let path = scratch_file("invalid-offset", &content);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dir = path.parent().unwrap().to_path_buf();
        let (bytes_sent, uploader) =
            spawn_uploader(&listener, path, 780, Arc::default());

        let (mut stream, _) = accept_and_send_offset(&listener, 4097);
        let mut received = Vec::new();
        stream.read_to_end(&mut received).unwrap();
        assert!(received.is_empty());

        let error = uploader.join().unwrap().unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(
            bytes_sent.load(Ordering::Relaxed),
            0,
            "an invalid prefix must not be counted as uploaded"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn serve_file_cancels_while_waiting_for_the_downloader_to_close() {
        let content: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let path = scratch_file("cancel-close-wait", &content);
        let dir = path.parent().unwrap().to_path_buf();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = u32::from(listener.local_addr().unwrap().port());
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_flag = cancel.clone();
        let (result_tx, result_rx) = mpsc::channel();

        let uploader = std::thread::spawn(move || {
            let result = serve_file(
                "127.0.0.1",
                port,
                "me",
                782,
                &path,
                &AtomicU64::new(0),
                &cancel_flag,
            );
            let _ = result_tx.send(result);
        });

        let (mut stream, _) = accept_and_send_offset(&listener, 0);
        let mut received = vec![0; content.len()];
        stream.read_exact(&mut received).unwrap();
        assert_eq!(received, content);

        cancel.store(true, Ordering::Relaxed);
        let error = result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("cancellation must wake the close wait")
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);

        drop(stream);
        let _ = uploader.join();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn serve_file_stops_when_cancelled() {
        let path = scratch_file("cancel", &vec![7u8; 1024 * 1024]);
        let dir = path.parent().unwrap().to_path_buf();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        // Cancelled before the copy loop starts: the stream must abort with
        // Interrupted instead of serving the whole file.
        let cancel = Arc::new(AtomicBool::new(true));
        let (_, uploader) = spawn_uploader(&listener, path, 778, cancel);

        let (mut stream, _) = accept_and_send_offset(&listener, 0);

        let mut received = Vec::new();
        let _ = stream.read_to_end(&mut received);
        assert!(received.is_empty(), "no file bytes after cancellation");

        let err = uploader.join().unwrap().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
        let _ = std::fs::remove_dir_all(dir);
    }
}
