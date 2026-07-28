use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::client::ClientContext;
use crate::message::server::MessageFactory;
use crate::trace;
use crate::types::{Download, DownloadStatus};
use crate::utils::path::{PART_SUFFIX, expand_tilde};

const READ_BUFFER_SIZE: usize = 8192;
const PROGRESS_UPDATE_CHUNKS: usize = 15; // ~120KB (15 * 8192 bytes)

#[derive(Debug)]
pub enum DownloadError {
    ConnectionFailed(io::Error),
    InvalidAddress(String),
    HandshakeFailed(io::Error),
    StreamReadError(io::Error),
    StreamWriteError(io::Error),
    TokenNotFound(u32),
    DownloadInfoMissing(u32),
    FileWriteError(io::Error),
    PathResolutionError(String),
    InvalidTokenBytes,
    LockPoisoned,
    IncompleteDownload { received: usize, expected: usize },
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionFailed(e) => write!(f, "Connection failed: {e}"),
            Self::InvalidAddress(addr) => {
                write!(f, "Invalid address: {addr}")
            }
            Self::HandshakeFailed(e) => write!(f, "Handshake failed: {e}"),
            Self::StreamReadError(e) => write!(f, "Stream read error: {e}"),
            Self::StreamWriteError(e) => write!(f, "Stream write error: {e}"),
            Self::TokenNotFound(token) => {
                write!(f, "Token not found: {token}")
            }
            Self::DownloadInfoMissing(token) => {
                write!(f, "Download info missing for token: {token}")
            }
            Self::FileWriteError(e) => write!(f, "File write error: {e}"),
            Self::PathResolutionError(msg) => {
                write!(f, "Path resolution error: {msg}")
            }
            Self::InvalidTokenBytes => {
                write!(f, "Invalid token bytes received")
            }
            Self::LockPoisoned => write!(f, "Lock poisoned"),
            Self::IncompleteDownload { received, expected } => write!(
                f,
                "Incomplete download: received {received} of {expected} bytes"
            ),
        }
    }
}

impl std::error::Error for DownloadError {}

fn extract_filename_from_path(full_path: &str) -> &str {
    full_path
        .split(['/', '\\'])
        .next_back()
        .unwrap_or(full_path)
}

/// Where a download's bytes end up: its configured directory (falling back
/// to the parent if it names a file) joined with the remote basename.
fn resolve_download_path(download: &Download) -> Result<String, DownloadError> {
    let mut expanded_path = expand_tilde(&download.download_directory);

    if !expanded_path.is_dir() {
        expanded_path = expanded_path
            .parent()
            .ok_or_else(|| {
                DownloadError::PathResolutionError(format!(
                    "Cannot resolve parent directory for: {}",
                    expanded_path.display()
                ))
            })?
            .to_path_buf();
    }

    let final_path =
        expanded_path.join(extract_filename_from_path(&download.filename));

    final_path
        .to_str()
        .ok_or_else(|| {
            DownloadError::PathResolutionError(format!(
                "Path contains invalid UTF-8: {}",
                final_path.display()
            ))
        })
        .map(String::from)
}

/// The `.part` file a transfer streams into, so an interrupted transfer leaves
/// behind what it managed to fetch and the next attempt can pick up from there.
struct PartFile {
    file: File,
    final_path: String,
    written: u64,
    expected: u64,
}

impl PartFile {
    fn open(download: &Download) -> Result<Self, DownloadError> {
        let final_path = resolve_download_path(download)?;
        let path = PathBuf::from(format!("{final_path}{PART_SUFFIX}"));

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(DownloadError::FileWriteError)?;
        }

        // A `.part` that already meets or exceeds the expected size is left over
        // from a different (or corrupted) transfer, not a resume point.
        let on_disk = fs::metadata(&path).map_or(0, |m| m.len());
        let written = if on_disk < download.size { on_disk } else { 0 };

        let file = if written > 0 {
            OpenOptions::new().append(true).open(&path)
        } else {
            File::create(&path)
        }
        .map_err(DownloadError::FileWriteError)?;

        Ok(Self {
            file,
            final_path,
            written,
            expected: download.size,
        })
    }

    /// Peers can coalesce trailing bytes into the last chunk, so overshoot past
    /// the expected size is dropped rather than written.
    fn write(&mut self, data: &[u8]) -> Result<(), DownloadError> {
        let remaining = (self.expected - self.written) as usize;
        let data = &data[..data.len().min(remaining)];
        self.file
            .write_all(data)
            .map_err(DownloadError::FileWriteError)?;
        self.written += data.len() as u64;
        Ok(())
    }

    const fn is_complete(&self) -> bool {
        self.written >= self.expected
    }

    /// A short transfer is a failure, and the `.part` stays put so the next
    /// attempt can resume it.
    fn finish(self) -> Result<String, DownloadError> {
        if !self.is_complete() {
            return Err(DownloadError::IncompleteDownload {
                received: self.written as usize,
                expected: self.expected as usize,
            });
        }
        drop(self.file);
        fs::rename(
            format!("{}{PART_SUFFIX}", self.final_path),
            &self.final_path,
        )
        .map_err(DownloadError::FileWriteError)?;
        Ok(self.final_path)
    }
}

pub struct DownloadPeer {
    username: String,
    host: String,
    port: u32,
    #[allow(dead_code)]
    own_username: String,
    token: u32,
    no_pierce: bool,
}

impl DownloadPeer {
    #[must_use]
    pub const fn new(
        username: String,
        host: String,
        port: u32,
        token: u32,
        no_pierce: bool,
        own_username: String,
    ) -> Self {
        Self {
            username,
            host,
            port,
            own_username,
            token,
            no_pierce,
        }
    }

    fn establish_connection(&self) -> Result<TcpStream, DownloadError> {
        let socket_address = format!("{}:{}", self.host, self.port)
            .to_socket_addrs()
            .map_err(DownloadError::ConnectionFailed)?
            .next()
            .ok_or_else(|| {
                DownloadError::InvalidAddress(format!(
                    "{}:{}",
                    self.host, self.port
                ))
            })?;

        let stream = TcpStream::connect_timeout(
            &socket_address,
            Duration::from_secs(20),
        )
        .map_err(DownloadError::ConnectionFailed)?;

        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(DownloadError::ConnectionFailed)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(DownloadError::ConnectionFailed)?;
        stream
            .set_nodelay(true)
            .map_err(DownloadError::ConnectionFailed)?;

        Ok(stream)
    }

    fn perform_handshake(
        &self,
        stream: &mut TcpStream,
    ) -> Result<(), DownloadError> {
        trace!(
            "[download_peer:{}] performing handshake no_pierce: {}",
            self.username, self.no_pierce
        );

        if !self.no_pierce {
            let message =
                MessageFactory::build_pierce_firewall_message(self.token);
            stream
                .write_all(&message.get_buffer())
                .map_err(DownloadError::HandshakeFailed)?;
            trace!(
                "[download_peer:{}] sending pierce firewall message token: {}: {:?}",
                self.username,
                self.token,
                &message.get_buffer()
            );
            stream.flush().map_err(DownloadError::HandshakeFailed)?;
        }
        Ok(())
    }

    /// Soulseek's START_DOWNLOAD is an 8-byte little-endian offset, so anything
    /// already in the `.part` file is skipped by asking the peer to start past it.
    fn start_transfer(
        download: Download,
        stream: &mut TcpStream,
        client_context: &Arc<RwLock<ClientContext>>,
    ) -> Result<(Download, PartFile), DownloadError> {
        let part = PartFile::open(&download)?;
        stream
            .write_all(&part.written.to_le_bytes())
            .map_err(DownloadError::StreamWriteError)?;
        Self::report_progress(client_context, &download, part.written, 0.0);
        Ok((download, part))
    }

    fn begin_pierced_download(
        &self,
        data: &[u8],
        stream: &mut TcpStream,
        client_context: &Arc<RwLock<ClientContext>>,
    ) -> Result<(Download, PartFile), DownloadError> {
        let token_bytes =
            data.get(0..4).ok_or(DownloadError::InvalidTokenBytes)?;
        let token_array: [u8; 4] = token_bytes
            .try_into()
            .map_err(|_| DownloadError::InvalidTokenBytes)?;
        let token_u32 = u32::from_le_bytes(token_array);

        trace!(
            "[download_peer:{}] got token: {} from data chunk",
            self.username, token_u32
        );

        let client_guard = client_context
            .read()
            .map_err(|_| DownloadError::LockPoisoned)?;
        let download_info =
            client_guard.get_download_by_token(token_u32).cloned();
        drop(client_guard);

        let download =
            download_info.ok_or(DownloadError::TokenNotFound(token_u32))?;

        Self::start_transfer(download, stream, client_context)
    }

    fn read_download_stream(
        &self,
        stream: &mut TcpStream,
        client_context: &Arc<RwLock<ClientContext>>,
        download: Option<Download>,
    ) -> Result<(Download, String), DownloadError> {
        let mut read_buffer = [0u8; READ_BUFFER_SIZE];
        let mut chunk_counter = 0usize;
        let mut last_update_time = Instant::now();

        trace!(
            "[download_peer:{}] Starting to read data from peer",
            self.username
        );

        let mut transfer = match download {
            Some(dl) => Some(Self::start_transfer(dl, stream, client_context)?),
            None => None,
        };

        loop {
            if let Some((ref dl, _)) = transfer {
                Self::wait_while_paused(client_context, dl)?;
            }

            match stream.read(&mut read_buffer) {
                Ok(0) => {
                    trace!(
                        "[download_peer:{}] connection closed by peer",
                        self.username
                    );
                    break;
                }
                Ok(bytes_read) => {
                    let data = &read_buffer[..bytes_read];

                    if transfer.is_none() && !self.no_pierce {
                        transfer = Some(self.begin_pierced_download(
                            data,
                            stream,
                            client_context,
                        )?);
                        continue;
                    }

                    let Some((dl, part)) = transfer.as_mut() else {
                        return Err(DownloadError::DownloadInfoMissing(
                            self.token,
                        ));
                    };

                    part.write(data)?;
                    chunk_counter += 1;

                    if chunk_counter.is_multiple_of(PROGRESS_UPDATE_CHUNKS) {
                        let elapsed = last_update_time.elapsed().as_secs_f64();
                        let bytes_since_last_update =
                            PROGRESS_UPDATE_CHUNKS * READ_BUFFER_SIZE;
                        let speed = if elapsed > 0.0 {
                            bytes_since_last_update as f64 / elapsed
                        } else {
                            0.0
                        };
                        Self::report_progress(
                            client_context,
                            dl,
                            part.written,
                            speed,
                        );
                        last_update_time = Instant::now();
                    }

                    if part.is_complete() {
                        break;
                    }
                }
                Err(e) => {
                    return Err(DownloadError::StreamReadError(e));
                }
            }
        }

        trace!(
            "[download_peer:{}] finished reading data from peer",
            self.username
        );

        let Some((download, part)) = transfer else {
            return Err(DownloadError::DownloadInfoMissing(self.token));
        };

        Ok((download, part.finish()?))
    }

    fn report_progress(
        client_context: &Arc<RwLock<ClientContext>>,
        download: &Download,
        bytes_downloaded: u64,
        speed_bytes_per_sec: f64,
    ) {
        Self::send_download_status(
            client_context,
            download,
            DownloadStatus::InProgress {
                bytes_downloaded,
                total_bytes: download.size,
                speed_bytes_per_sec,
            },
        );
    }

    fn send_download_status(
        client_context: &Arc<RwLock<ClientContext>>,
        download: &Download,
        status: DownloadStatus,
    ) {
        let _ = download.sender.send(status.clone());
        if let Ok(mut context) = client_context.write() {
            context.update_download_with_status(download.token, status);
        }
    }

    fn wait_while_paused(
        client_context: &Arc<RwLock<ClientContext>>,
        download: &Download,
    ) -> Result<(), DownloadError> {
        loop {
            let status = client_context
                .read()
                .map_err(|_| DownloadError::LockPoisoned)?
                .get_download_by_token(download.token)
                .map(|download| download.status.clone())
                .ok_or(DownloadError::TokenNotFound(download.token))?;

            if !matches!(status, DownloadStatus::Paused { .. }) {
                return Ok(());
            }

            thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn download_file(
        self,
        client_context: Arc<RwLock<ClientContext>>,
        download: Option<Download>,
        stream: Option<TcpStream>,
    ) -> Result<(Download, String), DownloadError> {
        trace!(
            "[download_peer:{}] download_file: download is present?: {:?}, stream is present?: {:?}, no_pierce: {}",
            self.username,
            download.is_some(),
            stream.is_some(),
            self.no_pierce
        );

        if let Some(ref dl) = download {
            let _ = dl.sender.send(DownloadStatus::Queued);
            client_context
                .write()
                .map_err(|_| DownloadError::LockPoisoned)?
                .update_download_with_status(dl.token, DownloadStatus::Queued);
        }

        let mut stream = match stream {
            Some(s) => s,
            None => self.establish_connection()?,
        };

        trace!("[download_peer:{}] connected", self.username);

        self.perform_handshake(&mut stream)?;
        trace!("[download_peer:{}] handshake completed", self.username);

        let (download, final_path) =
            self.read_download_stream(&mut stream, &client_context, download)?;

        trace!(
            "[download_peer:{}] download completed successfully: {} bytes, saved to: {}",
            self.username, download.size, final_path
        );

        Ok((download, final_path))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DownloadError, DownloadPeer, PartFile, extract_filename_from_path,
    };
    use crate::types::{Download, DownloadMetadata, DownloadStatus};
    use std::sync::mpsc;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("soulseek-part-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn download_into(dir: &std::path::Path, size: u64) -> Download {
        Download {
            username: "peer".to_string(),
            filename: "song.mp3".to_string(),
            token: 1,
            size,
            download_directory: dir.display().to_string(),
            status: DownloadStatus::Queued,
            sender: mpsc::channel().0,
            queue_position: None,
            metadata: DownloadMetadata::default(),
        }
    }

    #[test]
    fn a_partial_file_sets_the_resume_offset() {
        let dir = scratch_dir("resume");
        std::fs::write(dir.join("song.mp3.part"), b"0123").unwrap();

        let mut part = PartFile::open(&download_into(&dir, 10)).unwrap();
        assert_eq!(part.written, 4, "resume from what is already on disk");

        part.write(b"456789").unwrap();
        let final_path = part.finish().unwrap();

        assert_eq!(std::fs::read(&final_path).unwrap(), b"0123456789");
        assert!(
            !dir.join("song.mp3.part").exists(),
            "the .part is renamed, not left behind"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_oversized_partial_file_restarts_from_zero() {
        // Left over from a different transfer — not a valid resume point.
        let dir = scratch_dir("stale");
        std::fs::write(dir.join("song.mp3.part"), vec![9u8; 40]).unwrap();

        let mut part = PartFile::open(&download_into(&dir, 10)).unwrap();
        assert_eq!(part.written, 0);

        part.write(&(0..10).collect::<Vec<u8>>()).unwrap();
        let final_path = part.finish().unwrap();
        assert_eq!(
            std::fs::read(final_path).unwrap(),
            (0..10).collect::<Vec<u8>>()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_truncated_transfer_fails_and_keeps_the_partial_file() {
        let dir = scratch_dir("truncated");
        let mut part = PartFile::open(&download_into(&dir, 10)).unwrap();
        part.write(b"01234").unwrap();

        assert!(matches!(
            part.finish(),
            Err(DownloadError::IncompleteDownload {
                received: 5,
                expected: 10
            })
        ));
        assert_eq!(
            std::fs::read(dir.join("song.mp3.part")).unwrap(),
            b"01234",
            "the partial stays on disk for the next attempt"
        );
        assert!(!dir.join("song.mp3").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn overshoot_past_the_expected_size_is_dropped() {
        let dir = scratch_dir("overshoot");
        let mut part = PartFile::open(&download_into(&dir, 10)).unwrap();
        part.write(&(0..12).collect::<Vec<u8>>()).unwrap();

        assert!(part.is_complete());
        assert_eq!(
            std::fs::read(part.finish().unwrap()).unwrap(),
            (0..10).collect::<Vec<u8>>()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_establish_connection_invalid_address() {
        let download_peer = DownloadPeer::new(
            "user".to_string(),
            "invalid-host".to_string(),
            9999,
            123,
            false,
            "own_user".to_string(),
        );
        let result = download_peer.establish_connection();
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_filename_from_path() {
        assert_eq!(extract_filename_from_path("/path/to/file.mp3"), "file.mp3");
        assert_eq!(
            extract_filename_from_path("C:\\path\\to\\file.mp3"),
            "file.mp3"
        );
        assert_eq!(
            extract_filename_from_path(
                "@@bhfrv\\Soulseek Downloads\\complete\\Beatport Top Deep House (2021)\\michel test file.mp3"
            ),
            "michel test file.mp3"
        );
        assert_eq!(extract_filename_from_path("file.mp3"), "file.mp3");
    }
}
