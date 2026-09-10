//! Downloads through the context: what is queued, paused, cancelled,
//! and what a peer's answer is allowed to start.

use super::*;

#[test]
fn test_client_context_downloads() {
    let mut context = ClientContext::new();
    let token = 123;
    let new_token = 1234;
    context.add_download(download(
        "test",
        "test.txt",
        token,
        DownloadStatus::Queued,
        mpsc::channel().0,
    ));
    assert!(context.get_download_by_token(123).is_some());
    assert_eq!(context.get_download_tokens(), vec![123]);
    assert_eq!(context.get_downloads().len(), 1);
    if let Some(download) = context.get_download_by_token_mut(token) {
        assert_eq!(download.token, token);
        download.token = new_token;
    }
    assert!(context.get_download_by_token(new_token).is_some());
    assert_eq!(context.get_download_tokens(), vec![new_token]);
    context.remove_download(new_token);
    assert_eq!(context.get_downloads().len(), 0);
    assert!(context.get_download_by_token(1234).is_none());
}

#[test]
fn test_client_pause_and_resume_download() {
    let client = Client::new("test-user", "test-password");
    let (download_sender, download_receiver) = mpsc::channel();

    client.context.write().unwrap().add_download(download(
        "peer",
        "song.mp3",
        123,
        DownloadStatus::InProgress {
            bytes_downloaded: 25,
            total_bytes: 100,
            speed_bytes_per_sec: 10.0,
        },
        download_sender,
    ));

    assert!(client.pause_download("peer", "song.mp3"));
    assert!(matches!(
        client
            .context
            .read()
            .unwrap()
            .get_download_by_token(123)
            .unwrap()
            .status,
        DownloadStatus::Paused {
            bytes_downloaded: 25,
            total_bytes: 100
        }
    ));
    assert!(matches!(
        download_receiver.try_recv().unwrap(),
        DownloadStatus::Paused {
            bytes_downloaded: 25,
            total_bytes: 100
        }
    ));

    assert!(client.resume_download("peer", "song.mp3"));
    assert!(matches!(
        client
            .context
            .read()
            .unwrap()
            .get_download_by_token(123)
            .unwrap()
            .status,
        DownloadStatus::InProgress {
            bytes_downloaded: 25,
            total_bytes: 100,
            speed_bytes_per_sec: 0.0
        }
    ));
}

#[test]
fn download_without_a_connection_resolves_failed() {
    // A client that never connected has no server handle and no peer registry,
    // so it cannot open a connection to the peer: the download must resolve to
    // Failed rather than hang Queued forever.
    let client = Client::new("test-user", "test-password");
    let (_download, receiver) = client
        .download(
            "song.mp3".to_string(),
            "peer".to_string(),
            100,
            "test".to_string(),
        )
        .expect("download() should return a handle");
    assert!(matches!(
        receiver.recv_timeout(Duration::from_secs(1)),
        Ok(DownloadStatus::Failed(_))
    ));
}

#[test]
fn fail_queued_downloads_notifies_receiver_and_store() {
    // When a brokered connect times out, every Queued download for the peer
    // must resolve to Failed both on its channel and in the store.
    let client = Client::new("u", "p");
    let (sender, receiver) = mpsc::channel();
    client.context.write().unwrap().add_download(download(
        "peer",
        "f.mp3",
        7,
        DownloadStatus::Queued,
        sender,
    ));

    Client::fail_queued_downloads(&client.context, "peer");

    assert!(matches!(receiver.try_recv(), Ok(DownloadStatus::Failed(_))));
    assert!(matches!(
        client
            .context
            .read()
            .unwrap()
            .get_download_by_token(7)
            .unwrap()
            .status,
        DownloadStatus::Failed(_)
    ));
}

#[test]
fn test_client_removes_only_queued_downloads() {
    let client = Client::new("test-user", "test-password");
    {
        let mut context = client.context.write().unwrap();
        context.add_download(download(
            "peer",
            "queued.mp3",
            123,
            DownloadStatus::Queued,
            mpsc::channel().0,
        ));
        context.add_download(download(
            "peer",
            "active.mp3",
            456,
            DownloadStatus::InProgress {
                bytes_downloaded: 25,
                total_bytes: 100,
                speed_bytes_per_sec: 10.0,
            },
            mpsc::channel().0,
        ));
    }

    assert!(client.remove_queued_download("peer", "queued.mp3"));
    assert!(!client.remove_queued_download("peer", "active.mp3"));
    let context = client.context.read().unwrap();
    assert!(context.get_download_by_token(123).is_none());
    assert!(context.get_download_by_token(456).is_some());
}

#[test]
fn an_expired_broker_connect_fails_the_queued_downloads() {
    let client = Client::new("u", "p");
    let (sender, receiver) = mpsc::channel();
    {
        let mut ctx = client.context.write().unwrap();
        ctx.add_download(download(
            "ghost",
            "f.mp3",
            7,
            DownloadStatus::Queued,
            sender,
        ));
        ctx.pending_connect_tokens
            .insert(7, ("ghost".to_string(), Instant::now()));
    }
    let (_ops_tx, ops_rx) = mpsc::channel();
    Client::listen_to_client_operations(
        ops_rx,
        client.context.clone(),
        "u".to_string(),
    );

    let status = receiver.recv_timeout(Duration::from_secs(5));
    assert!(
        matches!(status, Ok(DownloadStatus::Failed(_))),
        "the sweep must fail the queued download, got {status:?}"
    );
}

#[test]
fn a_replayed_transfer_response_does_not_start_a_second_transfer() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = u32::from(listener.local_addr().unwrap().port());
    let download_dir = std::env::temp_dir()
        .join(format!("soulseek-replayed-transfer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&download_dir);
    std::fs::create_dir_all(&download_dir).unwrap();

    let client = Client::new("u", "p");
    let (sender, _receiver) = mpsc::channel();
    let mut queued =
        download("peer", "f.mp3", 9, DownloadStatus::Queued, sender);
    queued.download_directory = download_dir.display().to_string();
    client.context.write().unwrap().add_download(queued);

    let (ops_tx, ops_rx) = mpsc::channel();
    Client::listen_to_client_operations(
        ops_rx,
        client.context.clone(),
        "u".to_string(),
    );

    let peer = Peer::new(
        "peer".to_string(),
        ConnectionType::F,
        "127.0.0.1".to_string(),
        port,
        None,
        0,
        0,
        0,
    );
    ops_tx
        .send(ClientOperation::DownloadFromPeer(9, peer.clone(), true))
        .unwrap();
    ops_tx
        .send(ClientOperation::DownloadFromPeer(9, peer, true))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut connections = 0;
    while Instant::now() < deadline {
        match listener.accept() {
            Ok(_) => {
                connections += 1;
                if connections == 1 {
                    thread::sleep(Duration::from_millis(500));
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if connections >= 1 {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
    assert_eq!(
        connections, 1,
        "a replayed TransferResponse must not dial the peer again"
    );
    let _ = std::fs::remove_dir_all(download_dir);
}

#[test]
fn a_cancelled_download_is_not_started_when_the_peer_allows_it() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = u32::from(listener.local_addr().unwrap().port());

    let client = Client::new("u", "p");
    let (sender, _receiver) = mpsc::channel();
    client.context.write().unwrap().add_download(download(
        "peer",
        "f.mp3",
        9,
        DownloadStatus::Cancelled,
        sender,
    ));

    let (ops_tx, ops_rx) = mpsc::channel();
    Client::listen_to_client_operations(
        ops_rx,
        client.context.clone(),
        "u".to_string(),
    );
    let peer = Peer::new(
        "peer".to_string(),
        ConnectionType::F,
        "127.0.0.1".to_string(),
        port,
        None,
        0,
        0,
        0,
    );
    ops_tx
        .send(ClientOperation::DownloadFromPeer(9, peer, true))
        .unwrap();

    thread::sleep(Duration::from_millis(500));
    assert!(
        matches!(listener.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "a cancelled download must not dial the peer"
    );
}

#[test]
fn cancel_download_marks_the_store_and_answers_whether_it_matched() {
    let client = Client::new("u", "p");
    let (sender, receiver) = mpsc::channel();
    client.context.write().unwrap().add_download(download(
        "peer",
        "f.mp3",
        9,
        DownloadStatus::Queued,
        sender,
    ));

    assert!(client.cancel_download("peer", "f.mp3"));
    assert!(!client.cancel_download("peer", "f.mp3"));
    assert!(!client.cancel_download("peer", "other.mp3"));
    assert!(matches!(receiver.try_recv(), Ok(DownloadStatus::Cancelled)));
    assert!(matches!(
        client.get_all_downloads()[0].status,
        DownloadStatus::Cancelled
    ));
}
