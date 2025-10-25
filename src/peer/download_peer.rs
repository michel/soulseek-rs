use crate::client::ClientContext;
use crate::message::server::MessageFactory;
use crate::trace;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    fn create_download_paths(
        output_path: Option<String>,
        username: &str,
        token: u32,
    ) -> DownloadPaths {
        let final_path = match output_path {
            Some(path) if !path.is_empty() => path,
            _ => format!("/tmp/{}_{}.mp3", username, token),
        };
        let incomplete_path = format!("{}.incomplete", final_path);

        DownloadPaths {
            final_path,
            incomplete_path,
        }
    }

    fn create_temp_file(path: &str) -> Result<BufWriter<File>, io::Error> {
        // Create directory if needed
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
        // Split on both forward slashes and backslashes to handle Windows and Unix paths
        full_path.split(['/', '\\']).next_back().unwrap_or(full_path).to_string()
    }

    fn create_download_path_from_filename(
        output_directory: Option<&str>,
        remote_username: &str,
        token: u32,
        filename: Option<&str>
    ) -> String {
        let base_dir = output_directory.unwrap_or("/tmp");

        if let Some(filename) = filename {
            let clean_filename = Self::extract_filename_from_path(filename);
            format!("{}/{}", base_dir, clean_filename)
        } else {
            format!("{}/{}_{}.mp3", base_dir, remote_username, token)
        }
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
        Ok(false)
    }

    fn process_data_chunk(
        &mut self,
        data: &[u8],
    ) -> Result<(), io::Error> {
        self.buffer.extend_from_slice(data);
        self.total_bytes += data.len();
        Ok(())
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
    #[allow(dead_code)]
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
            let message = MessageFactory::build_peer_init_message(
                &self.own_username,
                super::ConnectionType::F,
                self.token,
            );
            trace!(
                "[default_peer:{}] sending peer init, token: {} message: {:?}
                ",
                self.token,
                self.username,
                message.get_buffer(),
            );
            stream.write_all(&message.get_buffer())?;
            stream.flush()?;
            // stream
            //     .write_all(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])?;
            // stream.flush()?;
            //
        } else {
            stream.write_all(
                &MessageFactory::build_pierce_firewall_message(self.token)
                    .get_buffer(),
            )?;
            trace!(
                "[default_peer:{}] sending pierce firewall message token: {}: {:?}",
                self.username,
                self.token,
                MessageFactory::build_pierce_firewall_message(self.token)
                    .get_buffer()
            );
            stream.flush()?;
        }
        Ok(())
    }

    pub fn download_file(
        self,
        client_context: Arc<Mutex<ClientContext>>,
        mut expected_size: Option<usize>,
        output_path: Option<String>,
    ) -> Result<(usize, String), io::Error> {
        let mut stream = self.establish_connection()?;
        trace!("[download_peer:{}] connected", self.username);

        self.perform_handshake(&mut stream)?;
        trace!("[download_peer:{}] handshake completed", self.username);

        let mut processor = StreamProcessor::new(self.no_pierce, self.token);
        let mut read_buffer = [1u8; 8192];
        let mut download_info: Option<crate::types::Download> = None;

        trace!(
            "[download_peer:{}] Starting to read data from peer",
            self.username
        );

        loop {
            match stream.read(&mut read_buffer) {
                Ok(0) => {
                    trace!(
                        "[download_peer:{}] connection closed by peer",
                        self.username
                    );
                    break; // Connection closed
                }
                Ok(bytes_read) => {
                    let data = &read_buffer[..bytes_read];

                    if !self.no_pierce
                        && !processor.received
                        && processor.handle_pierce_token(data, &mut stream)?
                    {
                        let token = data.get(0..4).unwrap();
                        let token_u32 =
                            u32::from_le_bytes(token.try_into().unwrap());
                        trace!(
                            "[download_peer:{}] received token: {:?} ",
                            self.username,
                            token
                        );

                        download_info = client_context
                            .lock()
                            .unwrap()
                            .download_tokens
                            .get(&token_u32)
                            .cloned();

                        match &download_info {
                            Some(download) => {
                                trace!(
                                    "[download_peer:{}] got download info for token: {} - filename: {}",
                                    self.username,
                                    token_u32,
                                    download.filename
                                );
                                expected_size = Some(download.size as usize);
                            }
                            None => {
                                trace!(
                                    "[download_peer:{}] no download info for token: {}",
                                    self.username,
                                    self.token
                                );
                            }
                        }

                        continue; // Skip this data chunk
                    }

                    processor.process_data_chunk(data)?;

                    if !processor.should_continue(expected_size) {
                        break;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    continue;
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

        // Now determine the final file path using download info
        let output_directory = output_path.as_ref().map(|path| {
            if Path::new(path).is_dir() {
                path.as_str()
            } else if let Some(parent) = Path::new(path).parent() {
                parent.to_str().unwrap_or("/tmp")
            } else {
                "/tmp"
            }
        });

        let filename = download_info.as_ref().map(|d| d.filename.as_str());

        let final_path = if let Some(output_path) = &output_path {
            if Path::new(output_path).is_dir() || output_path.ends_with('/') {
                // output_path is a directory, append filename
                FileManager::create_download_path_from_filename(
                    Some(output_path),
                    &self.username,
                    self.token,
                    filename
                )
            } else {
                // output_path is a full file path, use it directly
                output_path.clone()
            }
        } else {
            // No output path provided, use default logic
            FileManager::create_download_path_from_filename(
                output_directory,
                &self.username,
                self.token,
                filename
            )
        };

        // Create directory if needed
        if let Some(parent) = Path::new(&final_path).parent() {
            fs::create_dir_all(parent)?;
        }

        // Write the buffer to the final file
        fs::write(&final_path, &processor.buffer)?;

        trace!(
            "[download_peer:{}] download completed successfully: {} bytes, saved to: {}",
            self.username,
            processor.total_bytes,
            final_path
        );

        Ok((processor.total_bytes, final_path))
    }
}

#[cfg(test)]
mod tests {
    use super::{DownloadPeer, FileManager, StreamProcessor};
    use crate::client::ClientContext;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    pub fn build_test_server() -> (u16, Arc<Mutex<Vec<Vec<u8>>>>) {
        let messages = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let messages_clone = Arc::clone(&messages);

        // Find an available port by binding to port 0
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => {
                        let mut buffer = [0u8; 1024];
                        match stream.read(&mut buffer) {
                            Ok(bytes_read) if bytes_read > 0 => {
                                let data = buffer[..bytes_read].to_vec();
                                messages_clone.lock().unwrap().push(data);
                            }
                            _ => {}
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Give the server time to start
        thread::sleep(Duration::from_millis(10));

        (port, messages)
    }

    #[test]
    pub fn test_connect_no_pierce() {
        let (port, messages) = build_test_server();
        let token = 33;
        let download_peer = DownloadPeer::new(
            "test_user".to_string(),
            "127.0.0.1".to_string(),
            port as u32,
            token,
            false,
            "own_username".to_string(),
        );
        let dummy_context = Arc::new(Mutex::new(ClientContext::new()));
        let _ = download_peer
            .download_file(dummy_context, None, None)
            .unwrap();

        // Give the client time to send messages
        thread::sleep(Duration::from_millis(10));

        let received_messages = messages.lock().unwrap();
        assert_eq!(received_messages.len(), 1);
        assert_eq!(received_messages[0], vec![0, 0, 0, 0, 33, 0, 0, 0]);
    }

    #[test]
    pub fn test_connect_with_pierce() {
        let (port, messages) = build_test_server();
        let token = 33;
        let download_peer = DownloadPeer::new(
            "test_user".to_string(),
            "127.0.0.1".to_string(),
            port as u32,
            token,
            true, // no_pierce = true, should send init message
            "own_username".to_string(),
        );
        let dummy_context = Arc::new(Mutex::new(ClientContext::new()));
        let _ = download_peer
            .download_file(dummy_context, None, None)
            .unwrap();

        // Give the client time to send messages
        thread::sleep(Duration::from_millis(10));

        let received_messages = messages.lock().unwrap();
        assert_eq!(received_messages.len(), 1); // Expect one init message
        assert_eq!(
            received_messages[0],
            vec![
                1, 12, 0, 0, 0, 111, 119, 110, 95, 117, 115, 101, 114, 110, 97,
                109, 101, 1, 0, 0, 0, 70, 33, 0, 0, 0
            ]
        );

        // No messages expected when no_pierce is false
    }

    #[test]
    pub fn test_download_file() {
        let test_data = b"test file content";
        let messages = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let messages_clone = Arc::clone(&messages);

        // Create a test server that sends test data
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => {
                        // Set non-blocking to avoid hanging
                        stream.set_nonblocking(true).unwrap();

                        // Read handshake data
                        let mut buffer = [0u8; 1024];
                        std::thread::sleep(Duration::from_millis(10));
                        if let Ok(bytes_read) = stream.read(&mut buffer) {
                            if bytes_read > 0 {
                                messages_clone
                                    .lock()
                                    .unwrap()
                                    .push(buffer[..bytes_read].to_vec());
                            }
                        }

                        // Send pierce token (4 bytes) then test data
                        stream.set_nonblocking(false).unwrap();
                        let _ = stream.write_all(&[42, 0, 0, 0]);
                        std::thread::sleep(Duration::from_millis(10));
                        let _ = stream.write_all(test_data);
                    }
                    Err(_) => break,
                }
            }
        });

        thread::sleep(Duration::from_millis(10));

        let download_peer = DownloadPeer::new(
            "remote_user".to_string(),
            "127.0.0.1".to_string(),
            port as u32,
            42,
            false,
            "test_user".to_string(),
        );

        let dummy_context = Arc::new(Mutex::new(ClientContext::new()));
        let result = download_peer.download_file(
            dummy_context,
            Some(test_data.len()),
            Some("test_download.mp3".to_string()),
        );

        assert!(result.is_ok());
        let (bytes_downloaded, file_path) = result.unwrap();
        assert_eq!(bytes_downloaded, test_data.len());
        assert_eq!(file_path, "test_download.mp3");

        // Verify the file was written correctly
        let downloaded_data = fs::read("test_download.mp3").unwrap();
        assert_eq!(downloaded_data, test_data);

        // Clean up test file
        let _ = fs::remove_file("test_download.mp3");
    }

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
            Some("".to_string()),
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

        let result = processor.process_data_chunk(data);
        assert!(result.is_ok());
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

    #[test]
    fn test_create_download_path_from_filename() {
        // Test with filename provided
        let path = FileManager::create_download_path_from_filename(
            Some("/downloads"),
            "remote_user",
            123,
            Some("song.mp3")
        );
        assert_eq!(path, "/downloads/song.mp3");

        // Test with full path filename
        let path = FileManager::create_download_path_from_filename(
            Some("/downloads"),
            "remote_user",
            123,
            Some("/remote/path/to/song.mp3")
        );
        assert_eq!(path, "/downloads/song.mp3");

        // Test without filename (fallback)
        let path = FileManager::create_download_path_from_filename(
            Some("/downloads"),
            "remote_user",
            123,
            None
        );
        assert_eq!(path, "/downloads/remote_user_123.mp3");

        // Test with default directory
        let path = FileManager::create_download_path_from_filename(
            None,
            "remote_user",
            123,
            Some("song.mp3")
        );
        assert_eq!(path, "/tmp/song.mp3");
    }
}
