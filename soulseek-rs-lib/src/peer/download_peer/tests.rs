use super::{
    DownloadError, DownloadPeer, PartFile, extract_filename_from_path,
    part_path_of,
};
use crate::client::ClientContext;
use crate::types::{Download, DownloadMetadata, DownloadStatus};
use std::sync::{Arc, RwLock, mpsc};

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("soulseek-part-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn part_of(download: &Download) -> std::path::PathBuf {
    part_path_of(download).expect("a part path").into()
}

fn download_into(dir: &std::path::Path, size: u64) -> Download {
    Download {
        username: "peer".to_string(),
        filename: "file.bin".to_string(),
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
fn nested_download_directory_creates_parents_and_resumes_part_file() {
    let dir = scratch_dir("nested").join("share").join("album");
    let download = download_into(&dir, 8);
    let mut first = PartFile::open(&download).unwrap();
    first.write(b"data").unwrap();
    drop(first);

    let mut resumed = PartFile::open(&download).unwrap();
    assert_eq!(resumed.written, 4);
    resumed.write(b"1234").unwrap();
    let final_path = resumed.finish().unwrap();
    assert_eq!(std::fs::read(&final_path).unwrap(), b"data1234");
    assert!(!part_of(&download).exists());
    let _ = std::fs::remove_dir_all(dir.ancestors().nth(2).unwrap());
}

#[test]
fn a_discarded_part_file_is_removed_from_disk() {
    let dir = scratch_dir("discard");
    let mut part = PartFile::open(&download_into(&dir, 8)).unwrap();
    part.write(b"data").unwrap();
    let path = part_of(&download_into(&dir, 8));
    assert!(path.exists());

    part.discard();

    assert!(!path.exists());
    assert!(!dir.join("file.bin").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pause_is_reported_so_the_stall_budget_restarts() {
    let dir = scratch_dir("paused");
    let mut download = download_into(&dir, 8);
    download.status = DownloadStatus::Paused {
        bytes_downloaded: 0,
        total_bytes: 8,
    };
    let context = Arc::new(RwLock::new(ClientContext::new()));
    context.write().unwrap().add_download(download.clone());
    let resumer = {
        let context = context.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            assert!(
                context
                    .write()
                    .unwrap()
                    .downloads
                    .resume_by_file("peer", "file.bin")
            );
        })
    };

    assert!(matches!(
        DownloadPeer::wait_while_paused(&context, &download),
        Ok(true)
    ));
    assert!(matches!(
        DownloadPeer::wait_while_paused(&context, &download),
        Ok(false)
    ));
    let _ = resumer.join();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_cancelled_download_stops_the_transfer_loop() {
    let dir = scratch_dir("cancelled");
    let mut download = download_into(&dir, 8);
    download.status = DownloadStatus::Cancelled;
    let context = Arc::new(RwLock::new(ClientContext::new()));
    context.write().unwrap().add_download(download.clone());

    assert!(matches!(
        DownloadPeer::wait_while_paused(&context, &download),
        Err(DownloadError::Cancelled)
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_status_after_cancel_is_not_relayed_on_the_channel() {
    let dir = scratch_dir("relay");
    let (tx, rx) = mpsc::channel();
    let mut download = download_into(&dir, 8);
    download.sender = tx;
    download.status = DownloadStatus::Cancelled;
    let context = Arc::new(RwLock::new(ClientContext::new()));
    context.write().unwrap().add_download(download.clone());

    DownloadPeer::send_download_status(
        &context,
        &download,
        DownloadStatus::InProgress {
            bytes_downloaded: 4,
            total_bytes: 8,
            speed_bytes_per_sec: 1.0,
        },
    );

    assert!(rx.try_recv().is_err(), "a cancelled row hears nothing more");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pierced_short_transfer_reports_failure() {
    let dir = scratch_dir("pierced-short-transfer");
    let (sender, receiver) = mpsc::channel();
    let mut download = download_into(&dir, 10);
    download.sender = sender;
    let context = Arc::new(RwLock::new(ClientContext::new()));
    context.write().unwrap().add_download(download.clone());

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client_stream =
        std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut uploader, _) = listener.accept().unwrap();
    uploader
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    let peer = DownloadPeer::new(
        download.username.clone(),
        "127.0.0.1".into(),
        0,
        99,
        false,
        "me".into(),
    );
    let worker_context = context.clone();
    let worker = std::thread::spawn(move || {
        peer.download_file(worker_context, None, Some(client_stream))
    });

    // Drain the framed PierceFirewall request before sending the raw token.
    let mut frame_len = [0u8; 4];
    std::io::Read::read_exact(&mut uploader, &mut frame_len).unwrap();
    let mut frame = vec![0; u32::from_le_bytes(frame_len) as usize];
    std::io::Read::read_exact(&mut uploader, &mut frame).unwrap();

    std::io::Write::write_all(&mut uploader, &download.token.to_le_bytes())
        .unwrap();

    let mut offset = [0u8; 8];
    std::io::Read::read_exact(&mut uploader, &mut offset).unwrap();
    assert_eq!(u64::from_le_bytes(offset), 0);
    std::io::Write::write_all(&mut uploader, b"0123").unwrap();
    std::net::TcpStream::shutdown(&uploader, std::net::Shutdown::Write)
        .unwrap();
    drop(uploader);

    assert!(matches!(
        worker.join().unwrap(),
        Err(DownloadError::IncompleteDownload {
            received: 4,
            expected: 10
        })
    ));
    assert!(matches!(
        receiver.recv_timeout(std::time::Duration::from_secs(2)),
        Ok(DownloadStatus::InProgress { .. })
    ));
    assert!(matches!(
        receiver.recv_timeout(std::time::Duration::from_secs(2)),
        Ok(DownloadStatus::Failed(_))
    ));
    assert!(matches!(
        context
            .read()
            .unwrap()
            .get_download_by_token(download.token)
            .unwrap()
            .status,
        DownloadStatus::Failed(_)
    ));
    assert_eq!(std::fs::read(part_of(&download)).unwrap(), b"0123");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_pierced_token_cannot_claim_another_peers_download() {
    let dir = scratch_dir("token-peer-mismatch");
    let mut download = download_into(&dir, 10);
    download.username = "victim".into();
    let context = Arc::new(RwLock::new(ClientContext::new()));
    context.write().unwrap().add_download(download.clone());

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client_stream =
        std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut attacker, _) = listener.accept().unwrap();
    let peer = DownloadPeer::new(
        "attacker".into(),
        "127.0.0.1".into(),
        0,
        100,
        false,
        "me".into(),
    );
    let worker_context = context.clone();
    let worker = std::thread::spawn(move || {
        peer.download_file(worker_context, None, Some(client_stream))
    });

    let mut frame_len = [0u8; 4];
    std::io::Read::read_exact(&mut attacker, &mut frame_len).unwrap();
    let mut frame = vec![0; u32::from_le_bytes(frame_len) as usize];
    std::io::Read::read_exact(&mut attacker, &mut frame).unwrap();
    std::io::Write::write_all(&mut attacker, &download.token.to_le_bytes())
        .unwrap();

    assert!(matches!(
        worker.join().unwrap(),
        Err(DownloadError::TokenNotFound(1))
    ));
    assert!(!part_of(&download).exists());
    assert!(matches!(
        context
            .read()
            .unwrap()
            .get_download_by_token(download.token)
            .unwrap()
            .status,
        DownloadStatus::Queued
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn start_transfer_refuses_a_cancelled_download() {
    let dir = scratch_dir("refuse");
    let mut download = download_into(&dir, 8);
    download.status = DownloadStatus::Cancelled;
    let context = Arc::new(RwLock::new(ClientContext::new()));
    context.write().unwrap().add_download(download.clone());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut stream =
        std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let _peer = listener.accept().unwrap();
    let path = part_of(&download);

    let outcome = DownloadPeer::start_transfer(download, &mut stream, &context);

    assert!(matches!(outcome, Err(DownloadError::Cancelled)));
    assert!(!path.exists(), "no .part is opened");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_directory_that_does_not_exist_yet_is_created_not_dodged() {
    let dir = scratch_dir("fresh").join("Soulseek");

    let mut part = PartFile::open(&download_into(&dir, 4)).unwrap();
    part.write(b"data").unwrap();
    let final_path = part.finish().unwrap();

    assert_eq!(
        std::path::Path::new(&final_path).parent(),
        Some(dir.as_path()),
        "the bytes belong in the configured directory, not its parent"
    );
    assert_eq!(std::fs::read(&final_path).unwrap(), b"data");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn a_partial_file_sets_the_resume_offset() {
    let dir = scratch_dir("resume");
    let download = download_into(&dir, 10);
    let path = part_of(&download);
    std::fs::write(&path, b"0123").unwrap();

    let mut part = PartFile::open(&download).unwrap();
    assert_eq!(part.written, 4, "resume from what is already on disk");

    part.write(b"456789").unwrap();
    let final_path = part.finish().unwrap();

    assert_eq!(std::fs::read(&final_path).unwrap(), b"0123456789");
    assert!(!path.exists(), "the .part is renamed, not left behind");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_partial_file_resumes_only_for_the_same_transfer() {
    let dir = scratch_dir("transfer-identity");
    let mut original = download_into(&dir, 8);
    original.filename = "album-a\\file.bin".into();
    let mut first = PartFile::open(&original).unwrap();
    first.write(b"AAAA").unwrap();
    drop(first);

    for (username, filename, content) in [
        ("other-peer", "album-a\\file.bin", b"BBBBBBBB".as_slice()),
        ("peer", "album-b\\file.bin", b"CCCCCCCC".as_slice()),
        ("peer", "album-a\\file.bin", b"DDDDDDDDDD".as_slice()),
    ] {
        let mut other = original.clone();
        other.username = username.into();
        other.filename = filename.into();
        other.size = content.len() as u64;

        let mut part = PartFile::open(&other).unwrap();
        assert_eq!(
            part.written, 0,
            "a different transfer must not inherit the original prefix"
        );
        part.write(content).unwrap();
        assert_eq!(std::fs::read(part.finish().unwrap()).unwrap(), content);
    }

    original.token = 2;
    let mut resumed = PartFile::open(&original).unwrap();
    assert_eq!(resumed.written, 4);
    resumed.write(b"1234").unwrap();
    assert_eq!(
        std::fs::read(resumed.finish().unwrap()).unwrap(),
        b"AAAA1234"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_legacy_partial_does_not_supply_bytes_to_a_new_transfer() {
    let dir = scratch_dir("legacy");
    std::fs::write(dir.join("file.bin.part"), b"AAAA").unwrap();

    let mut part = PartFile::open(&download_into(&dir, 8)).unwrap();
    assert_eq!(part.written, 0);
    part.write(b"BBBBBBBB").unwrap();
    assert_eq!(std::fs::read(part.finish().unwrap()).unwrap(), b"BBBBBBBB");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_exact_size_partial_restarts_from_zero() {
    let dir = scratch_dir("complete");
    let download = download_into(&dir, 10);
    let path = part_of(&download);
    std::fs::write(&path, b"0123456789").unwrap();

    let mut part = PartFile::open(&download).unwrap();
    assert_eq!(part.written, 0);
    assert!(!part.is_complete());

    part.write(b"abcdefghij").unwrap();
    let final_path = part.finish().unwrap();
    assert_eq!(std::fs::read(final_path).unwrap(), b"abcdefghij");
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cancellation_while_a_complete_transfer_is_paused_discards_its_part() {
    let dir = scratch_dir("cancel-paused-finish");
    let mut download = download_into(&dir, 10);
    let mut part = PartFile::open(&download).unwrap();
    part.write(b"0123456789").unwrap();
    download.status = DownloadStatus::Paused {
        bytes_downloaded: 10,
        total_bytes: 10,
    };
    let context = Arc::new(RwLock::new(ClientContext::new()));
    context.write().unwrap().add_download(download.clone());
    let worker_context = context.clone();
    let worker_download = download.clone();

    let worker = std::thread::spawn(move || {
        DownloadPeer::finish_transfer(&worker_context, &worker_download, part)
    });

    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        context
            .write()
            .unwrap()
            .downloads
            .cancel_by_file(&download.username, &download.filename)
            .is_some()
    );

    assert!(matches!(
        worker.join().unwrap(),
        Err(DownloadError::Cancelled)
    ));
    assert!(!part_of(&download).exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_oversized_partial_file_restarts_from_zero() {
    let dir = scratch_dir("stale");
    let download = download_into(&dir, 10);
    std::fs::write(part_of(&download), vec![9u8; 40]).unwrap();

    let mut part = PartFile::open(&download).unwrap();
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
        std::fs::read(part_of(&download_into(&dir, 10))).unwrap(),
        b"01234",
        "the partial stays on disk for the next attempt"
    );
    assert!(!dir.join("file.bin").exists());
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

// Streams the listener accepted arrive with no deadlines; without these a
// peer that sends a token and goes silent parks the transfer thread —
// socket and open .part file included — for the rest of the session.
#[test]
fn supplied_streams_get_transfer_deadlines() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let stream =
        std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();

    DownloadPeer::apply_transfer_deadlines(&stream).unwrap();

    let secs = std::time::Duration::from_secs;
    assert_eq!(stream.read_timeout().unwrap(), Some(secs(30)));
    assert_eq!(stream.write_timeout().unwrap(), Some(secs(5)));
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
