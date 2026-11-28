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
use crate::utils::md5;
use crate::utils::path::{PART_SUFFIX, expand_tilde};

/// Read in 64 KiB slices: an eighth of the syscalls of the 8 KiB reads before,
/// and what a fast peer fills per read. slskd reads 256 KiB, Nicotine+ ~50 KiB.
const READ_BUFFER_SIZE: usize = 64 * 1024;
const CANCEL_POLL: Duration = Duration::from_secs(1);
const STALL_DEADLINE: Duration = Duration::from_secs(30);
/// Bytes between progress reports. Counting bytes rather than reads keeps
/// the report volume flat however big a read is, and gives a slow peer's
/// rate a window wide enough to mean something.
const PROGRESS_INTERVAL_BYTES: usize = 128 * 1024;

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
    LockPoisoned,
    Cancelled,
    IncompleteDownload { received: u64, expected: u64 },
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
            Self::LockPoisoned => write!(f, "Lock poisoned"),
            Self::Cancelled => write!(f, "Download cancelled"),
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

/// Named for its transfer, not only its basename: names like `01.flac` and
/// `cover.jpg` recur, and resuming another download's partial would put its
/// bytes at the front of this file.
fn part_path(final_path: &str, download: &Download) -> String {
    // Length prefixes keep field boundaries unambiguous for remote names.
    let identity = format!(
        "{}:{}:{}:{}:{}",
        download.username.len(),
        download.username,
        download.filename.len(),
        download.filename,
        download.size
    );
    let digest = md5(&identity);
    format!("{final_path}.{}{PART_SUFFIX}", &digest[..8])
}

pub fn part_path_of(download: &Download) -> Option<String> {
    resolve_download_path(download)
        .ok()
        .map(|path| part_path(&path, download))
}

/// Where a download's bytes end up: its configured directory (falling back
/// to the parent if it names a file) joined with the remote basename.
///
/// A directory that does not exist yet stays the target — `PartFile::open`
/// creates it. Treating it as "not a directory" and dodging to the parent
/// made a daemon whose folder was never created download into `~/Downloads`
/// while reporting `~/Downloads/Soulseek`.
fn resolve_download_path(download: &Download) -> Result<String, DownloadError> {
    let mut expanded_path = expand_tilde(&download.download_directory);

    if expanded_path.is_file() {
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
    path: PathBuf,
    final_path: String,
    written: u64,
    expected: u64,
}

impl PartFile {
    fn open(download: &Download) -> Result<Self, DownloadError> {
        let final_path = resolve_download_path(download)?;
        let path = PathBuf::from(part_path(&final_path, download));

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(DownloadError::FileWriteError)?;
        }

        // A partial at or past the full size is not a resume point: nothing
        // is left to ask the peer for.
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
            path,
            final_path,
            written,
            expected: download.size,
        })
    }

    /// Peers can coalesce trailing bytes into the last chunk, so overshoot past
    /// the expected size is dropped rather than written.
    fn write(&mut self, data: &[u8]) -> Result<(), DownloadError> {
        let remaining = self.expected - self.written;
        let write_len = remaining.min(data.len() as u64) as usize;
        let data = &data[..write_len];
        self.file
            .write_all(data)
            .map_err(DownloadError::FileWriteError)?;
        self.written += data.len() as u64;
        Ok(())
    }

    const fn is_complete(&self) -> bool {
        self.written >= self.expected
    }

    fn discard(self) {
        drop(self.file);
        let _ = fs::remove_file(self.path);
    }

    /// A short transfer is a failure, and the `.part` stays put so the next
    /// attempt can resume it.
    fn finish(self) -> Result<String, DownloadError> {
        if !self.is_complete() {
            return Err(DownloadError::IncompleteDownload {
                received: self.written,
                expected: self.expected,
            });
        }
        drop(self.file);
        fs::rename(self.path, &self.final_path)
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

        Self::apply_transfer_deadlines(&stream)?;

        Ok(stream)
    }

    /// Bound every stall on a transfer socket. This must cover streams the
    /// listener accepted as well as ones we dialled: an accepted F connection
    /// used to arrive here with no read timeout at all, so a peer that sent a
    /// token and went silent parked the transfer thread — holding the socket
    /// and the open `.part` file — for the rest of the session.
    fn apply_transfer_deadlines(
        stream: &TcpStream,
    ) -> Result<(), DownloadError> {
        stream
            .set_read_timeout(Some(STALL_DEADLINE))
            .map_err(DownloadError::ConnectionFailed)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(DownloadError::ConnectionFailed)?;
        stream
            .set_nodelay(true)
            .map_err(DownloadError::ConnectionFailed)?;
        Ok(())
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
        Self::wait_while_paused(client_context, &download)?;
        let part = PartFile::open(&download)?;
        stream
            .write_all(&part.written.to_le_bytes())
            .map_err(DownloadError::StreamWriteError)?;
        Self::report_progress(client_context, &download, part.written, 0.0);
        Ok((download, part))
    }

    fn read_download_stream(
        &self,
        stream: &mut TcpStream,
        client_context: &Arc<RwLock<ClientContext>>,
        download: Option<Download>,
        matched_download: &mut Option<Download>,
    ) -> Result<(Download, String), DownloadError> {
        let mut read_buffer = vec![0u8; READ_BUFFER_SIZE];
        let mut bytes_since_last_update = 0usize;
        let mut last_update_time = Instant::now();

        trace!(
            "[download_peer:{}] Starting to read data from peer",
            self.username
        );

        let (download, mut part) = match download {
            Some(download) => {
                Self::start_transfer(download, stream, client_context)?
            }
            None if !self.no_pierce => {
                let mut token = [0u8; 4];
                stream
                    .read_exact(&mut token)
                    .map_err(DownloadError::StreamReadError)?;
                let token = u32::from_le_bytes(token);
                trace!(
                    "[download_peer:{}] got token: {} from data chunk",
                    self.username, token
                );
                let download = client_context
                    .write()
                    .map_err(|_| DownloadError::LockPoisoned)?
                    .downloads
                    .claim_for_peer(token, &self.username)
                    .ok_or(DownloadError::TokenNotFound(token))?;
                *matched_download = Some(download.clone());
                Self::start_transfer(download, stream, client_context)?
            }
            None => {
                return Err(DownloadError::DownloadInfoMissing(self.token));
            }
        };
        stream
            .set_read_timeout(Some(CANCEL_POLL))
            .map_err(DownloadError::ConnectionFailed)?;
        let speed_limit = client_context
            .read()
            .map_err(|_| DownloadError::LockPoisoned)?
            .download_speed_limit
            .clone();
        let mut last_data = Instant::now();

        loop {
            match Self::wait_while_paused(client_context, &download) {
                Ok(true) => last_data = Instant::now(),
                Ok(false) => {}
                Err(error) => {
                    if matches!(error, DownloadError::Cancelled) {
                        part.discard();
                    }
                    return Err(error);
                }
            }

            // A zero-byte transfer needs no payload bytes. There is no offset
            // acknowledgement in Soulseek's transfer protocol, so the uploader
            // waits for us to close and we must finalize without another read.
            if part.is_complete() {
                break;
            }

            let allowance = speed_limit.take(READ_BUFFER_SIZE);
            let read = stream.read(&mut read_buffer[..allowance]);
            speed_limit.give_back(allowance - read.as_ref().map_or(0, |&n| n));
            match read {
                Ok(0) => {
                    trace!(
                        "[download_peer:{}] connection closed by peer",
                        self.username
                    );
                    break;
                }
                Ok(bytes_read) => {
                    last_data = Instant::now();
                    part.write(&read_buffer[..bytes_read])?;
                    bytes_since_last_update += bytes_read;

                    if bytes_since_last_update >= PROGRESS_INTERVAL_BYTES {
                        let elapsed = last_update_time.elapsed().as_secs_f64();
                        let speed = if elapsed > 0.0 {
                            bytes_since_last_update as f64 / elapsed
                        } else {
                            0.0
                        };
                        Self::report_progress(
                            client_context,
                            &download,
                            part.written,
                            speed,
                        );
                        bytes_since_last_update = 0;
                        last_update_time = Instant::now();
                    }

                    if part.is_complete() {
                        break;
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) && last_data.elapsed() < STALL_DEADLINE => {}
                Err(e) => {
                    return Err(DownloadError::StreamReadError(e));
                }
            }
        }

        trace!(
            "[download_peer:{}] finished reading data from peer",
            self.username
        );

        let final_path =
            Self::finish_transfer(client_context, &download, part)?;
        Ok((download, final_path))
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

    pub(crate) fn send_download_status(
        client_context: &Arc<RwLock<ClientContext>>,
        download: &Download,
        status: DownloadStatus,
    ) {
        let applied = client_context.write().is_ok_and(|mut context| {
            context.downloads.update_status(
                &download.username,
                download.token,
                status.clone(),
            )
        });
        if applied {
            let _ = download.sender.send(status);
        }
    }

    /// Promote a complete partial and publish `Completed` as one operation with
    /// respect to cancellation. Holding the store write lock across the rename
    /// gives both possible races deterministic outcomes: cancellation marks the
    /// row first and discards the partial, or completion lands first and a later
    /// cancellation is refused.
    fn finish_transfer(
        client_context: &Arc<RwLock<ClientContext>>,
        download: &Download,
        part: PartFile,
    ) -> Result<String, DownloadError> {
        loop {
            let mut context = client_context
                .write()
                .map_err(|_| DownloadError::LockPoisoned)?;
            let status = context
                .downloads
                .get(&download.username, download.token)
                .map(|download| download.status.clone())
                .ok_or(DownloadError::TokenNotFound(download.token))?;

            if matches!(status, DownloadStatus::Cancelled) {
                drop(context);
                part.discard();
                return Err(DownloadError::Cancelled);
            }
            if matches!(status, DownloadStatus::Paused { .. }) {
                drop(context);
                if let Err(error) =
                    Self::wait_while_paused(client_context, download)
                {
                    if matches!(error, DownloadError::Cancelled) {
                        part.discard();
                    }
                    return Err(error);
                }
                continue;
            }

            let final_path = part.finish()?;
            if !context.downloads.update_status(
                &download.username,
                download.token,
                DownloadStatus::Completed,
            ) {
                return Err(DownloadError::TokenNotFound(download.token));
            }
            drop(context);
            let _ = download.sender.send(DownloadStatus::Completed);
            return Ok(final_path);
        }
    }

    fn wait_while_paused(
        client_context: &Arc<RwLock<ClientContext>>,
        download: &Download,
    ) -> Result<bool, DownloadError> {
        let mut waited = false;
        loop {
            let status = client_context
                .read()
                .map_err(|_| DownloadError::LockPoisoned)?
                .downloads
                .get(&download.username, download.token)
                .map(|download| download.status.clone())
                .ok_or(DownloadError::TokenNotFound(download.token))?;

            if matches!(status, DownloadStatus::Cancelled) {
                return Err(DownloadError::Cancelled);
            }
            if !matches!(status, DownloadStatus::Paused { .. }) {
                return Ok(waited);
            }

            waited = true;
            thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn download_file(
        self,
        client_context: Arc<RwLock<ClientContext>>,
        download: Option<Download>,
        stream: Option<TcpStream>,
    ) -> Result<(Download, String), DownloadError> {
        let mut matched_download = download.clone();
        let result: Result<_, DownloadError> = (|| {
            trace!(
                "[download_peer:{}] download_file: download is present?: {:?}, stream is present?: {:?}, no_pierce: {}",
                self.username,
                download.is_some(),
                stream.is_some(),
                self.no_pierce
            );

            let mut stream = match stream {
                Some(s) => {
                    Self::apply_transfer_deadlines(&s)?;
                    s
                }
                None => self.establish_connection()?,
            };

            trace!("[download_peer:{}] connected", self.username);

            self.perform_handshake(&mut stream)?;
            trace!("[download_peer:{}] handshake completed", self.username);

            let (download, final_path) = self.read_download_stream(
                &mut stream,
                &client_context,
                download,
                &mut matched_download,
            )?;

            trace!(
                "[download_peer:{}] download completed successfully: {} bytes, saved to: {}",
                self.username, download.size, final_path
            );

            Ok((download, final_path))
        })();
        if let Err(error) = &result
            && !matches!(error, DownloadError::Cancelled)
            && let Some(download) = matched_download
        {
            Self::send_download_status(
                &client_context,
                &download,
                DownloadStatus::Failed(Some(error.to_string())),
            );
        }
        result
    }
}

#[cfg(test)]
mod tests;
