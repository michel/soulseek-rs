//! Server-independent end-to-end coverage for downloads that resume.
//!
//! The subject is a real `Client` with its real peer listener and actor stack.
//! A loopback socket stands in for a Soulseek peer and speaks the public wire
//! format.  The configured server is deliberately idle: peer transfers do not
//! require a login once the control connection is registered.

use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use soulseek_rs::message::Message;
use soulseek_rs::{
    Client, ClientSettings, ClientVersion, DownloadStatus, PeerAddress,
};

const IO_TIMEOUT: Duration = Duration::from_secs(10);

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "soulseek-resume-e2e-{}-{label}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create scratch directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Subject {
    client: Client,
    peer_addr: String,
    // Keeping a server listener alive is enough for ServerActor's TCP connect.
    // No login or server messages are needed by the peer-transfer path.
    _idle_server: TcpListener,
}

impl Subject {
    fn new(username: &str) -> Self {
        let idle_server =
            TcpListener::bind("127.0.0.1:0").expect("bind idle server");
        let server_addr =
            idle_server.local_addr().expect("idle server address");
        let mut client = Client::with_settings(ClientSettings {
            username: username.to_string(),
            password: "unused".to_string(),
            server_address: PeerAddress::new(
                "127.0.0.1".to_string(),
                server_addr.port(),
            ),
            enable_listen: true,
            listen_port: 0,
            shared_directories: Vec::new(),
            accept_children: false,
            version: ClientVersion::default(),
        });
        client.connect().expect("connect subject");
        let port = client.listen_port().expect("subject listener port");
        Self {
            client,
            peer_addr: format!("127.0.0.1:{port}"),
            _idle_server: idle_server,
        }
    }
}

struct ControlPeer {
    username: String,
    subject_addr: String,
    stream: TcpStream,
}

impl ControlPeer {
    fn register(subject: &Subject, username: &str) -> Self {
        let mut stream = TcpStream::connect(&subject.peer_addr)
            .expect("connect peer control socket");
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .expect("control read timeout");
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .expect("control write timeout");

        // Code 15/16 is a positive registration fence. Receiving the response
        // proves the inbound P connection has reached the peer actor and can
        // receive QueueUpload messages from `Client::download`.
        stream
            .write_all(&peer_init_bytes(username, "P", 0))
            .expect("send P PeerInit");
        stream
            .write_all(&Message::new().write_int32(15).get_buffer())
            .expect("send UserInfoRequest registration fence");
        stream.flush().expect("flush registration fence");
        let _ = expect_code(&mut stream, 16);

        Self {
            username: username.to_string(),
            subject_addr: subject.peer_addr.clone(),
            stream,
        }
    }

    fn accept_queue_and_offer(&mut self, filename: &str, size: u64) -> u32 {
        let mut queue = expect_code(&mut self.stream, 43);
        queue.set_pointer(8);
        assert_eq!(queue.read_string(), filename, "queued remote filename");

        let token = 700_000;
        let mut request = Message::new();
        request
            .write_int32(40)
            .write_int32(1) // upload from the peer to the subject
            .write_int32(token)
            .write_string(filename)
            .write_int64(size);
        self.stream
            .write_all(&request.get_buffer())
            .expect("send TransferRequest");
        self.stream.flush().expect("flush TransferRequest");

        let mut response = expect_code(&mut self.stream, 41);
        response.set_pointer(8);
        assert_eq!(response.read_int32(), token, "response transfer token");
        assert_eq!(response.read_int8(), 1, "transfer should be allowed");
        token
    }

    fn open_file_transfer(&self, token: u32) -> (TcpStream, u64) {
        let mut stream = TcpStream::connect(&self.subject_addr)
            .expect("connect peer file socket");
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .expect("file read timeout");
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .expect("file write timeout");

        let mut init = peer_init_bytes(&self.username, "F", token);
        init.extend_from_slice(&token.to_le_bytes());
        stream.write_all(&init).expect("send F PeerInit and token");
        stream.flush().expect("flush F PeerInit and token");

        let mut offset = [0u8; 8];
        stream
            .read_exact(&mut offset)
            .expect("read START_DOWNLOAD offset");
        (stream, u64::from_le_bytes(offset))
    }
}

fn peer_init_bytes(
    username: &str,
    connection_type: &str,
    token: u32,
) -> Vec<u8> {
    Message::new()
        .write_int8(1)
        .write_string(username)
        .write_string(connection_type)
        .write_int32(token)
        .get_buffer()
}

fn read_framed(stream: &mut TcpStream) -> std::io::Result<Message> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let mut payload = vec![0u8; u32::from_le_bytes(len) as usize];
    stream.read_exact(&mut payload)?;
    let mut bytes = len.to_vec();
    bytes.extend_from_slice(&payload);
    Ok(Message::new_with_data(bytes))
}

fn expect_code(stream: &mut TcpStream, expected: u32) -> Message {
    let deadline = Instant::now() + IO_TIMEOUT;
    loop {
        let message = read_framed(stream).unwrap_or_else(|error| {
            panic!("read peer code {expected}: {error}")
        });
        if message.get_message_code() == expected {
            return message;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for peer code {expected}"
        );
    }
}

fn start_download(
    subject: &Subject,
    control: &mut ControlPeer,
    filename: &str,
    content_len: usize,
    destination: &Path,
) -> (Receiver<DownloadStatus>, TcpStream, u64) {
    let (_download, statuses) = subject
        .client
        .download(
            filename.to_string(),
            control.username.clone(),
            content_len as u64,
            destination.display().to_string(),
        )
        .expect("queue download");
    let token = control.accept_queue_and_offer(filename, content_len as u64);
    let (file_stream, offset) = control.open_file_transfer(token);
    (statuses, file_stream, offset)
}

fn wait_for_status(
    statuses: &Receiver<DownloadStatus>,
    description: &str,
    wanted: impl Fn(&DownloadStatus) -> bool,
) -> DownloadStatus {
    let deadline = Instant::now() + IO_TIMEOUT;
    loop {
        if let Ok(status) = statuses.recv_timeout(Duration::from_millis(100))
            && wanted(&status)
        {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "status channel did not report {description}"
        );
    }
}

fn assert_stored_status(
    subject: &Subject,
    filename: &str,
    wanted: impl Fn(&DownloadStatus) -> bool,
) {
    let status = subject
        .client
        .get_all_downloads()
        .into_iter()
        .find(|download| download.filename == filename)
        .unwrap_or_else(|| panic!("download {filename} is missing from store"))
        .status;
    assert!(wanted(&status), "unexpected stored status: {status:?}");
}

fn expect_downloader_close(stream: &mut TcpStream) {
    let mut unexpected = [0u8; 1];
    assert_eq!(
        stream
            .read(&mut unexpected)
            .expect("wait for downloader to close the F socket"),
        0,
        "the downloader sends no bytes after its offset"
    );
}

fn binary_payload(len: usize) -> Vec<u8> {
    (0..len).map(|index| index as u8).collect()
}

#[test]
fn an_actual_short_transfer_resumes_opaque_bytes() {
    let subject = Subject::new("resume_binary_subject");
    let mut control = ControlPeer::register(&subject, "resume_binary_peer");
    let destination = ScratchDir::new("binary-tracer");
    let content = binary_payload(3 * 64 * 1024 + 257);
    let filename = "opaque.bin";
    let interrupt_at = 64 * 1024 + 1;
    let part_path = destination.path().join(format!("{filename}.part"));
    let final_path = destination.path().join(filename);

    assert!(content.contains(&0));
    assert!(content.contains(&u8::MAX));
    assert!(std::str::from_utf8(&content).is_err());

    let (first_statuses, mut first, first_offset) = start_download(
        &subject,
        &mut control,
        filename,
        content.len(),
        destination.path(),
    );
    assert_eq!(first_offset, 0);
    first
        .write_all(&content[..interrupt_at])
        .expect("write short first transfer");
    first.flush().expect("flush short first transfer");
    first
        .shutdown(Shutdown::Write)
        .expect("interrupt first transfer");
    drop(first);

    let _ = wait_for_status(&first_statuses, "Failed", |status| {
        matches!(status, DownloadStatus::Failed(_))
    });
    assert_stored_status(&subject, filename, |status| {
        matches!(status, DownloadStatus::Failed(_))
    });
    assert!(!final_path.exists());
    assert_eq!(
        fs::read(&part_path).expect("read retained partial file"),
        content[..interrupt_at]
    );

    let (retry_statuses, mut retry, retry_offset) = start_download(
        &subject,
        &mut control,
        filename,
        content.len(),
        destination.path(),
    );
    assert_eq!(retry_offset, interrupt_at as u64);
    retry
        .write_all(&content[interrupt_at..])
        .expect("write retry tail");
    retry.flush().expect("flush retry tail");
    expect_downloader_close(&mut retry);
    drop(retry);

    let _ = wait_for_status(&retry_statuses, "Completed", |status| {
        matches!(status, DownloadStatus::Completed)
    });
    assert_stored_status(&subject, filename, |status| {
        matches!(status, DownloadStatus::Completed)
    });
    assert_eq!(fs::read(&final_path).unwrap(), content);
    assert!(!part_path.exists());
}

#[test]
fn a_live_download_pauses_and_resumes_on_the_same_connection() {
    let subject = Subject::new("resume_pause_subject");
    let mut control = ControlPeer::register(&subject, "resume_pause_peer");
    let destination = ScratchDir::new("pause-resume");
    let filename = "paused.bin";
    let content = binary_payload(5 * 64 * 1024 + 257);
    let part_path = destination.path().join(format!("{filename}.part"));

    let (statuses, mut peer, offset) = start_download(
        &subject,
        &mut control,
        filename,
        content.len(),
        destination.path(),
    );
    assert_eq!(offset, 0);

    let first_chunk = 2 * 64 * 1024;
    peer.write_all(&content[..first_chunk])
        .expect("send first chunk");
    peer.flush().expect("flush first chunk");
    let _ = wait_for_status(&statuses, "initial progress", |status| {
        matches!(
            status,
            DownloadStatus::InProgress {
                bytes_downloaded,
                ..
            } if *bytes_downloaded >= first_chunk as u64
        )
    });

    assert!(subject.client.pause_download(&control.username, filename));
    let _ = wait_for_status(&statuses, "Paused", |status| {
        matches!(status, DownloadStatus::Paused { .. })
    });

    // The pause is checked before every read, so once it lands only the read
    // already in flight can finish: one 64 KiB buffer at most. Queue more
    // than that, which a transfer ignoring the pause would take all of.
    let stable_len = fs::metadata(&part_path).expect("paused partial").len();
    let in_flight = 64 * 1024;
    let queued_while_paused = in_flight + 8 * 1024;
    peer.write_all(&content[first_chunk..first_chunk + queued_while_paused])
        .expect("queue bytes while paused");
    peer.flush().expect("flush bytes while paused");
    std::thread::sleep(Duration::from_millis(300));
    let paused_len = fs::metadata(&part_path).expect("paused partial").len();
    assert!(
        paused_len <= stable_len + in_flight as u64,
        "a paused transfer must stop consuming its socket: {paused_len} \
         bytes after pausing at {stable_len}"
    );

    assert!(subject.client.resume_download(&control.username, filename));
    peer.write_all(&content[first_chunk + queued_while_paused..])
        .expect("send tail after resume");
    peer.flush().expect("flush resumed tail");
    expect_downloader_close(&mut peer);

    let _ = wait_for_status(&statuses, "Completed", |status| {
        matches!(status, DownloadStatus::Completed)
    });
    assert_eq!(
        fs::read(destination.path().join(filename)).unwrap(),
        content
    );
}
