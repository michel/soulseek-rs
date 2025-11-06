use std::env;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::client::ClientContext;
use crate::message::server::MessageFactory;
use crate::trace;
use crate::types::Download;

#[derive(Debug)]
#[allow(dead_code)]
struct DownloadPaths {
    final_path: String,
    incomplete_path: String,
}

#[allow(dead_code)]
struct FileManager;

#[allow(dead_code)]
impl FileManager {
    fn expand_path(path: &str) -> String {
        if let Some(stripped) = path.strip_prefix('~') {
            let home = env::var("HOME").expect("HOME not set");
            format!("{}{}", home, stripped)
        } else {
            path.to_string()
        }
    }
    fn create_download_paths(
        output_path: Option<String>,
        username: &str,
        token: u32,
    ) -> DownloadPaths {
        let final_path = match output_path {
            Some(path) if !path.is_empty() => path,
            _ => format!("/tmp/{username}_{token}.mp3"),
        };
        let incomplete_path = format!("{final_path}.incomplete");

        DownloadPaths {
            final_path,
            incomplete_path,
        }
    }

    fn create_temp_file(path: &str) -> Result<BufWriter<File>, io::Error> {
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent)?;
        }

        let temp_file = File::create(path)?;
        Ok(BufWriter::new(temp_file))
    }

    fn finalize_download(
        incomplete_path: &str,
        final_path: &str,
    ) -> Result<(), io::Error> {
        fs::rename(incomplete_path, final_path)
    }

    fn cleanup_on_error(incomplete_path: Option<&str>) {
        if let Some(path) = incomplete_path {
            let _ = fs::remove_file(path);
        }
    }

    fn extract_filename_from_path(full_path: &str) -> String {
        full_path
            .split(['/', '\\'])
            .next_back()
            .unwrap_or(full_path)
            .to_string()
    }

    fn create_download_path_from_filename(
        output_directory: String,
        filename: String,
    ) -> String {
        format!(
            "{}/{}",
            output_directory.trim_end_matches('/'),
            Self::extract_filename_from_path(&filename)
        )
    }
}

#[allow(dead_code)]
struct StreamProcessor {
    no_pierce: bool,
    token: u32,
    total_bytes: usize,
    received: bool,
    buffer: Vec<u8>,
}

#[allow(dead_code)]
impl StreamProcessor {
    fn new(no_pierce: bool, token: u32) -> Self {
        Self {
            no_pierce,
            token,
            total_bytes: 0,
            received: false,
            buffer: Vec::new(),
        }
    }

    fn handle_pierce_token(
        &mut self,
        data: &[u8],
        stream: &mut TcpStream,
    ) -> Result<bool, io::Error> {
        if !self.no_pierce && !self.received && data.len() >= 4 {
            stream
                .write_all(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])?;
            self.received = true;
            return Ok(true); // Skip this data chunk
        }
        self.received = true;
        Ok(false)
    }

    fn process_data_chunk(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
        self.total_bytes += data.len();
    }

    fn should_continue(&self, expected_size: Option<usize>) -> bool {
        if let Some(size) = expected_size {
            self.total_bytes < size
        } else {
            true
        }
    }
}

#[allow(dead_code)]
pub struct DownloadPeer {
    username: String,
    host: String,
    port: u32,
    own_username: String,
    token: u32,
    no_pierce: bool,
}

#[allow(dead_code)]
impl DownloadPeer {
    #[must_use]
    pub fn new(
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

    fn establish_connection(&self) -> Result<TcpStream, io::Error> {
        let socket_address = format!("{}:{}", self.host, self.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Invalid address")
            })?;

        let stream = TcpStream::connect_timeout(
            &socket_address,
            Duration::from_secs(20),
        )?;

        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        stream.set_nodelay(true)?;

        Ok(stream)
    }

    fn perform_handshake(
        &self,
        stream: &mut TcpStream,
    ) -> Result<(), io::Error> {
        trace!(
            "[download_peer:{}] performing handshake no_pierce: {}",
            self.username,
            self.no_pierce
        );
        if self.no_pierce {
            // let message = MessageFactory::build_peer_init_message(
            //     &self.own_username,
            //     super::ConnectionType::F,
            //     self.token,
            // );
            // trace!(
            //     "[download_peer:{}] sending peer init, token: {} message: {:?}
            //     ",
            //     self.username,
            //     self.token,
            //     message.get_buffer(),
            // );
            // stream.write_all(&message.get_buffer())?;
            // stream.flush()?;
        } else {
            let message =
                MessageFactory::build_pierce_firewall_message(self.token);

            stream.write_all(&message.get_buffer())?;
            trace!(
                "[download_peer:{}] sending pierce firewall message token: {}: {:?}",
                self.username,
                self.token,
                &message.get_buffer()
            );
            stream.flush()?;
        }
        Ok(())
    }

    pub fn download_file(
        self,
        client_context: Arc<RwLock<ClientContext>>,
        mut download: Option<Download>,
        stream: Option<TcpStream>,
    ) -> Result<(Download, String), io::Error> {
        trace!(
            "[download_peer:{}] download_file: download is present?: {:?}, stream is present?: {:?}, no_pierce: {}",
            self.username,
            download.is_some(),
            stream.is_some(),
            self.no_pierce
        );
        let mut stream = stream.unwrap_or_else(|| {
            self.establish_connection().unwrap_or_else(|e| {
                panic!("Failed to establish connection: {:?}", e)
            })
        });

        trace!("[download_peer:{}] connected", self.username);

        self.perform_handshake(&mut stream)?;
        trace!("[download_peer:{}] handshake completed", self.username);

        let mut processor = StreamProcessor::new(self.no_pierce, self.token);
        let mut read_buffer = [1u8; 8192];

        trace!(
            "[download_peer:{}] Starting to read data from peer",
            self.username
        );

        if download.is_some() {
            stream
                .write_all(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])?;
        }

        loop {
            match stream.read(&mut read_buffer) {
                Ok(0) => {
                    trace!(
                        "[download_peer:{}] connection closed by peer. bytes read: {}",
                        self.username,
                        processor.total_bytes
                    );
                    break;
                }
                Ok(bytes_read) => {
                    let data = &read_buffer[..bytes_read];

                    if !self.no_pierce && !processor.received {
                        let token = data.get(0..4).unwrap();
                        let token_u32 = u32::from_le_bytes(
                            token
                                .try_into()
                                .unwrap_or_else(|_| panic!("[download_peer:{}] slice with incorrect length", self.username)),
                        );
                        trace!(
                            "[download_peer:{}] got token: {} from data chunk",
                            self.username,
                            token_u32
                        );
                        processor.received = true;

                        stream.write_all(&[
                            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                        ])?;

                        let read = client_context.read().unwrap();

                        let download_info =
                            read.download_tokens.get(&token_u32);

                        match download_info {
                            Some(d) => {
                                trace!(
                                    "[download_peer:{}] got download info for token: {} - filename: {}",
                                    self.username,
                                    token_u32,
                                    d.filename
                                );
                                trace!("[download_peer:{}] setting download info for token: {}",
                                    self.username,
                                    token_u32);
                                download = Some(d.clone());
                            }
                            None => {
                                let tokens = read
                                    .download_tokens
                                    .keys()
                                    .collect::<Vec<_>>();
                                panic!(
                                    "[download_peer:{}] No download info for token {token_u32}, tokens: {:?}", self.username, tokens
                                );
                            }
                        }

                        continue;
                    }

                    processor.process_data_chunk(data);

                    if !processor.should_continue(Some(
                        download.as_ref().unwrap().size as usize,
                    )) {
                        break;
                    }
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        trace!(
            "[download_peer:{}] finished reading data from peer",
            self.username
        );

        // let final_path = FileManager::create_download_path_from_filename(
        //     "/tmp/".to_string(),
        //     "test.mp3".to_string(),
        // );
        //
        // if let Some(parent) = Path::new(&final_path).parent() {
        //     fs::create_dir_all(parent)?;
        // }
        //
        // fs::write(&final_path, &processor.buffer).unwrap_or_else(|e| {
        //     panic!(
        //         "[download_peer] Failed to write to file {}: {}",
        //         final_path, e
        //     )
        // });

        let read = client_context.read().unwrap();
        let tokens = read.download_tokens.keys().collect::<Vec<_>>();

        let download = download.clone().unwrap_or_else(|| {
            panic!(
                "[download_peer] No download info found for token: {}, tokens: {:?} ",
                self.token, tokens
            )
        });

        let download_directory = download.download_directory.clone();
        let mut expaned_path = FileManager::expand_path(&download_directory);
        if !Path::new(&expaned_path).is_dir() {
            expaned_path = Path::new(&expaned_path)
                .parent()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        }

        let final_path = FileManager::create_download_path_from_filename(
            expaned_path,
            download.filename.clone(),
        );

        if let Some(parent) = Path::new(&final_path).parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&final_path, &processor.buffer).unwrap_or_else(|e| {
            panic!(
                "[download_peer] Failed to write to file {}: {}",
                final_path, e
            )
        });

        trace!(
            "[download_peer:{}] download completed successfully: {} bytes, saved to: {}",
            self.username,
            processor.total_bytes,
            final_path
        );

        Ok((download.clone(), final_path))
    }
}

#[cfg(test)]
mod tests {
    use super::{DownloadPeer, FileManager, StreamProcessor};
    use std::fs;

    #[test]
    fn test_file_manager_create_paths_with_custom_path() {
        let paths = FileManager::create_download_paths(
            Some("custom/path.mp3".to_string()),
            "user",
            123,
        );
        assert_eq!(paths.final_path, "custom/path.mp3");
        assert_eq!(paths.incomplete_path, "custom/path.mp3.incomplete");
    }

    #[test]
    fn test_file_manager_create_paths_with_default() {
        let paths = FileManager::create_download_paths(None, "testuser", 456);
        assert_eq!(paths.final_path, "/tmp/testuser_456.mp3");
        assert_eq!(paths.incomplete_path, "/tmp/testuser_456.mp3.incomplete");
    }

    #[test]
    fn test_file_manager_create_paths_with_empty_string() {
        let paths = FileManager::create_download_paths(
            Some(String::new()),
            "user",
            789,
        );
        assert_eq!(paths.final_path, "/tmp/user_789.mp3");
        assert_eq!(paths.incomplete_path, "/tmp/user_789.mp3.incomplete");
    }

    #[test]
    fn test_file_manager_create_temp_file() {
        let temp_path = "test_temp.txt.incomplete";
        let result = FileManager::create_temp_file(temp_path);
        assert!(result.is_ok());

        // Verify file was created
        assert!(fs::metadata(temp_path).is_ok());

        // Clean up
        let _ = fs::remove_file(temp_path);
    }

    #[test]
    fn test_file_manager_finalize_download() {
        // Create a temporary file
        let temp_path = "test_incomplete.txt";
        let final_path = "test_final.txt";
        fs::write(temp_path, "test content").unwrap();

        let result = FileManager::finalize_download(temp_path, final_path);
        assert!(result.is_ok());

        // Verify rename worked
        assert!(fs::metadata(final_path).is_ok());
        assert!(fs::metadata(temp_path).is_err()); // Original should be gone

        // Clean up
        let _ = fs::remove_file(final_path);
    }

    #[test]
    fn test_stream_processor_new() {
        let processor = StreamProcessor::new(true, 123);
        assert!(processor.no_pierce);
        assert_eq!(processor.token, 123);
        assert_eq!(processor.total_bytes, 0);
        assert!(!processor.received);
    }

    #[test]
    fn test_stream_processor_should_continue() {
        let processor = StreamProcessor::new(false, 123);

        // Without expected size, should always continue
        assert!(processor.should_continue(None));

        // With expected size, should continue if under limit
        assert!(processor.should_continue(Some(100)));
    }

    #[test]
    fn test_stream_processor_should_continue_with_limit() {
        let mut processor = StreamProcessor::new(false, 123);
        processor.total_bytes = 150;

        // Should not continue if over limit
        assert!(!processor.should_continue(Some(100)));

        // Should continue if under limit
        assert!(processor.should_continue(Some(200)));
    }

    #[test]
    fn test_stream_processor_process_data_chunk() {
        let mut processor = StreamProcessor::new(false, 123);
        let data = b"test data";

        processor.process_data_chunk(data);
        assert_eq!(processor.total_bytes, data.len());
        assert_eq!(processor.buffer, data);
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
        // Test with Unix-style path
        assert_eq!(
            FileManager::extract_filename_from_path("/path/to/file.mp3"),
            "file.mp3"
        );

        // Test with Windows-style path
        assert_eq!(
            FileManager::extract_filename_from_path("C:\\path\\to\\file.mp3"),
            "file.mp3"
        );

        // Test with Soulseek Windows path (the actual failing case)
        assert_eq!(
            FileManager::extract_filename_from_path("@@bhfrv\\Soulseek Downloads\\complete\\Beatport Top Deep House (2021)\\michel test file.mp3"),
            "michel test file.mp3"
        );

        // Test with just filename
        assert_eq!(
            FileManager::extract_filename_from_path("file.mp3"),
            "file.mp3"
        );
    }
}
