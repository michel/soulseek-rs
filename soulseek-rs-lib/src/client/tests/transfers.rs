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

// A peer may repeat its offer, naming the token the download already has.
#[test]
fn a_repeated_transfer_offer_keeps_the_download_listed_and_claimable() {
    let client = Client::new("u", "p");
    let (sender, _receiver) = mpsc::channel();
    client.context.write().unwrap().add_download(download(
        "peer",
        "f.mp3",
        9,
        DownloadStatus::Queued,
        sender,
    ));

    let (ops_tx, ops_rx) = mpsc::channel();
    Client::listen_to_client_operations(
        ops_rx,
        client.context.clone(),
        "u".to_string(),
    );
    for _ in 0..2 {
        ops_tx
            .send(ClientOperation::UpdateDownloadTokens(
                Transfer {
                    direction: 1,
                    token: 7,
                    filename: "f.mp3".to_string(),
                    size: 200,
                },
                "peer".to_string(),
            ))
            .unwrap();
    }
    ops_tx.send(ClientOperation::OwnPrivileges(1)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.own_privilege_seconds() != Some(1) {
        assert!(Instant::now() < deadline, "ops loop never caught up");
        thread::yield_now();
    }

    assert_eq!(client.get_all_downloads().len(), 1);
    let claimed = client
        .context
        .write()
        .unwrap()
        .downloads
        .claim_for_peer(7, "peer")
        .expect("the repeated offer must leave the download claimable");
    assert_eq!((claimed.filename.as_str(), claimed.size), ("f.mp3", 200));
}

// The token we give a download before it is offered and the token a peer
// chose for another one are drawn from the same small numbers.
#[test]
fn an_offer_re_keys_its_own_download_and_leaves_the_rest() {
    let client = Client::new("u", "p");
    let (sender, _receiver) = mpsc::channel();
    for (username, filename) in [("alice", "a.mp3"), ("bob", "b.mp3")] {
        client.context.write().unwrap().add_download(download(
            username,
            filename,
            7,
            DownloadStatus::Queued,
            sender.clone(),
        ));
    }

    let (ops_tx, ops_rx) = mpsc::channel();
    Client::listen_to_client_operations(
        ops_rx,
        client.context.clone(),
        "u".to_string(),
    );
    ops_tx
        .send(ClientOperation::UpdateDownloadTokens(
            Transfer {
                direction: 1,
                token: 99,
                filename: "b.mp3".to_string(),
                size: 200,
            },
            "bob".to_string(),
        ))
        .unwrap();
    ops_tx.send(ClientOperation::OwnPrivileges(1)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.own_privilege_seconds() != Some(1) {
        assert!(Instant::now() < deadline, "ops loop never caught up");
        thread::yield_now();
    }

    let mut tokens: Vec<(String, u32)> = client
        .get_all_downloads()
        .into_iter()
        .map(|download| (download.username, download.token))
        .collect();
    tokens.sort();
    assert_eq!(
        tokens,
        vec![("alice".to_string(), 7), ("bob".to_string(), 99)]
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

// An uploader that restarts, or drops an idle connection together with its
// in-memory queue, no longer knows what we asked it for. The queue poll is
// what asks again.

/// A client running its operations loop, with a peer registry reporting to
/// that loop and the server played by the returned receiver.
fn client_with_server()
-> (Client, Sender<ClientOperation>, Receiver<ServerMessage>) {
    let client = Client::new("me", "pw");
    let (ops, ops_rx) = mpsc::channel();
    let (server, server_rx) = mpsc::channel();
    {
        let mut ctx = client.context.write().unwrap();
        ctx.peer_registry = Some(PeerRegistry::new(
            ctx.actor_system.clone(),
            ops.clone(),
            "me".to_string(),
        ));
        ctx.server_sender = Some(server);
    }
    Client::listen_to_client_operations(
        ops_rx,
        client.context.clone(),
        "me".to_string(),
    );
    (client, ops, server_rx)
}

/// Return once the operations loop has handled everything sent before.
fn caught_up(client: &Client, ops: &Sender<ClientOperation>) {
    let fence = client.own_privilege_seconds().unwrap_or(0) + 1;
    ops.send(ClientOperation::OwnPrivileges(fence)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.own_privilege_seconds() != Some(fence) {
        assert!(Instant::now() < deadline, "ops loop never caught up");
        thread::yield_now();
    }
}

fn address_of(username: &str, host: &str, port: u32) -> ClientOperation {
    ClientOperation::GetPeerAddressResponse {
        username: username.to_string(),
        host: host.to_string(),
        port,
        obfuscation_type: 0,
        obfuscated_port: 0,
    }
}

fn looked_up(server: &Receiver<ServerMessage>) -> Option<String> {
    match server.try_recv() {
        Ok(ServerMessage::GetPeerAddress(username)) => Some(username),
        _ => None,
    }
}

/// The file named by the next QueueUpload (peer code 43) read from `stream`.
fn next_queue_request(stream: &mut TcpStream) -> String {
    use std::io::Read;
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    loop {
        let mut length = [0u8; 4];
        stream
            .read_exact(&mut length)
            .expect("a QueueUpload in time");
        let mut frame = length.to_vec();
        frame.resize(4 + u32::from_le_bytes(length) as usize, 0);
        stream.read_exact(&mut frame[4..]).unwrap();
        let mut message = crate::message::Message::new_with_data(frame);
        if message.get_message_code() == 43 {
            message.set_pointer(8);
            return message.read_string();
        }
    }
}

fn idle_port() -> u32 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    u32::from(listener.local_addr().unwrap().port())
}

/// Register an inbound control connection from `username`, returning the
/// peer's end of it.
fn connected_peer(client: &Client, username: &str) -> TcpStream {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let ours = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    ours.set_nonblocking(true).unwrap();
    let peer = Peer::new(
        username.to_string(),
        ConnectionType::P,
        "127.0.0.1".to_string(),
        0,
        None,
        0,
        0,
        0,
    );
    let registry = client.context.read().unwrap().peer_registry.clone();
    registry
        .unwrap()
        .register_peer(peer, Some(ours), None)
        .unwrap();
    listener.accept().unwrap().0
}

/// Queue `f.mp3` with `username`, returning its status channel.
fn queue_with(client: &Client, username: &str) -> Receiver<DownloadStatus> {
    let (sender, status) = mpsc::channel();
    client.context.write().unwrap().add_download(download(
        username,
        "f.mp3",
        1,
        DownloadStatus::Queued,
        sender,
    ));
    status
}

/// The token of the broker request (ConnectToPeer, code 18) sent next.
fn broker_request(server: &Receiver<ServerMessage>) -> u32 {
    let Ok(ServerMessage::SendMessage(mut request)) =
        server.recv_timeout(Duration::from_secs(5))
    else {
        panic!("expected a broker request");
    };
    assert_eq!(request.read_int32(), 18, "ConnectToPeer");
    request.read_int32()
}

fn assert_still_queued(client: &Client, status: &Receiver<DownloadStatus>) {
    assert!(status.try_recv().is_err(), "nothing failed the download");
    assert!(matches!(
        client.get_all_downloads()[0].status,
        DownloadStatus::Queued
    ));
}

#[test]
fn the_queue_poll_looks_up_each_peer_it_holds_no_connection_to() {
    let (client, _ops, server) = client_with_server();
    let _here = connected_peer(&client, "here");
    for (username, filename, token) in [
        ("gone", "a.mp3", 1),
        ("gone", "b.mp3", 2),
        ("here", "c.mp3", 3),
    ] {
        client.context.write().unwrap().add_download(download(
            username,
            filename,
            token,
            DownloadStatus::Queued,
            mpsc::channel().0,
        ));
    }

    Client::poll_queue_positions(&client.context);

    let sent: Vec<ServerMessage> = server.try_iter().collect();
    assert!(
        matches!(
            sent.as_slice(),
            [ServerMessage::GetPeerAddress(username)] if username == "gone"
        ),
        "one lookup for the peer we lost, none for the one we hold, got {sent:?}"
    );
}

#[test]
fn a_queue_poll_looks_up_a_fixed_number_and_the_next_takes_the_rest() {
    use crate::client::operations::QUEUE_POLL_LOOKUPS;

    let (client, _ops, server) = client_with_server();
    let lost = QUEUE_POLL_LOOKUPS as u32 * 3 / 2;
    for peer in 0..lost {
        client.context.write().unwrap().add_download(download(
            &format!("gone-{peer}"),
            "f.mp3",
            peer,
            DownloadStatus::Queued,
            mpsc::channel().0,
        ));
    }
    let mut asked = std::collections::HashSet::new();
    let mut poll = || {
        Client::poll_queue_positions(&client.context);
        let lookups: Vec<ServerMessage> = server.try_iter().collect();
        for lookup in &lookups {
            if let ServerMessage::GetPeerAddress(username) = lookup {
                asked.insert(username.clone());
            }
        }
        lookups.len()
    };

    assert_eq!(poll(), QUEUE_POLL_LOOKUPS);
    assert_eq!(poll(), QUEUE_POLL_LOOKUPS);
    assert_eq!(asked.len(), lost as usize, "two polls reach all of them");
}

#[test]
fn a_queued_download_is_requested_again_after_its_uploader_hangs_up() {
    let (client, ops, server) = client_with_server();
    let mut first = connected_peer(&client, "uploader");
    let _ = client
        .download(
            "f.mp3".to_string(),
            "uploader".to_string(),
            100,
            "test".to_string(),
        )
        .unwrap();
    assert_eq!(next_queue_request(&mut first), "f.mp3");

    drop(first);
    let registry = client.context.read().unwrap().peer_registry.clone();
    let registry = registry.unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while registry.contains("uploader") {
        assert!(Instant::now() < deadline, "the hang-up was never noticed");
        thread::yield_now();
    }

    let uploader = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    uploader.set_nonblocking(true).unwrap();
    let port = u32::from(uploader.local_addr().unwrap().port());
    Client::poll_queue_positions(&client.context);
    assert_eq!(looked_up(&server).as_deref(), Some("uploader"));
    ops.send(address_of("uploader", "127.0.0.1", port)).unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut second = loop {
        match uploader.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "the uploader was never dialled again"
                );
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("accept: {e}"),
        }
    };
    assert_eq!(
        next_queue_request(&mut second),
        "f.mp3",
        "the new connection carries the queue request again"
    );
}

#[test]
fn a_peer_offline_at_the_queue_poll_keeps_its_downloads_queued() {
    let (client, ops, server) = client_with_server();
    let status = queue_with(&client, "ghost");

    Client::poll_queue_positions(&client.context);
    assert_eq!(looked_up(&server).as_deref(), Some("ghost"));
    // The server's answer for a user who is not logged in.
    ops.send(address_of("ghost", "0.0.0.0", 0)).unwrap();

    let broker = server.recv_timeout(Duration::from_secs(1));
    assert!(
        broker.is_err(),
        "an offline peer is not dialled, so no broker request follows, got {broker:?}"
    );
    caught_up(&client, &ops);
    assert_still_queued(&client, &status);
}

#[test]
fn a_peer_the_queue_poll_cannot_reach_keeps_its_downloads_queued() {
    let (client, ops, server) = client_with_server();
    let status = queue_with(&client, "walled");

    Client::poll_queue_positions(&client.context);
    assert_eq!(looked_up(&server).as_deref(), Some("walled"));
    ops.send(address_of("walled", "127.0.0.1", idle_port()))
        .unwrap();
    // The dial is refused, so the server is asked to broker it, and the peer
    // cannot connect back either.
    let token = broker_request(&server);
    ops.send(ClientOperation::CantConnectToPeer { token })
        .unwrap();

    caught_up(&client, &ops);
    assert_still_queued(&client, &status);
}

#[test]
fn an_offline_peer_still_fails_a_download_the_poll_never_looked_up() {
    let (client, ops, server) = client_with_server();
    let status = queue_with(&client, "ghost");

    ops.send(address_of("ghost", "0.0.0.0", 0)).unwrap();
    // What the server answers when the user it is to broker is offline.
    let token = broker_request(&server);
    ops.send(ClientOperation::CantConnectToPeer { token })
        .unwrap();

    assert!(matches!(
        status.recv_timeout(Duration::from_secs(5)),
        Ok(DownloadStatus::Failed(_))
    ));
}
