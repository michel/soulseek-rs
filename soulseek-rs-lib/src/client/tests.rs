use super::*;

fn download(
    username: &str,
    filename: &str,
    token: u32,
    status: DownloadStatus,
    sender: Sender<DownloadStatus>,
) -> Download {
    Download {
        username: username.to_string(),
        filename: filename.to_string(),
        token,
        size: 100,
        download_directory: "test".to_string(),
        status,
        sender,
        queue_position: None,
        metadata: DownloadMetadata::default(),
    }
}

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
fn build_search_response_matches_shares_and_echoes_token() {
    let dir = std::env::temp_dir()
        .join(format!("soulseek-searchresp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("probe_xyzzy.bin"), b"data").unwrap();
    let shares = Shares::scan(&dir).unwrap();

    let response = build_search_response(&shares, "me", 99, "xyzzy")
        .expect("a matching share yields a response");
    let mut decoded =
        crate::message::Message::new_with_data(response.get_buffer());
    decoded.set_pointer(8);
    let result = SearchResult::new_from_message(&mut decoded).unwrap();
    assert_eq!(result.username, "me");
    assert_eq!(result.token, 99);
    assert!(result.files.iter().any(|f| f.name.contains("probe_xyzzy")));

    assert!(build_search_response(&shares, "me", 1, "nomatch").is_none());
    let _ = std::fs::remove_dir_all(dir);
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

use crate::types::{RoomEvent, UploadStatus};
use std::time::{Duration, Instant};

#[test]
fn the_room_roster_follows_the_joins_and_leaves_the_server_reports() {
    let mut context = ClientContext::new();

    // Joining hands us the current membership in one event.
    context.apply_room_event(RoomEvent::Joined {
        room: "lobby".to_string(),
        users: vec!["bob".to_string(), "alice".to_string()],
    });
    assert_eq!(context.room_members("lobby"), ["alice", "bob"]);

    context.apply_room_event(RoomEvent::UserJoined {
        room: "lobby".to_string(),
        username: "carol".to_string(),
    });
    context.apply_room_event(RoomEvent::UserLeft {
        room: "lobby".to_string(),
        username: "bob".to_string(),
    });
    assert_eq!(context.room_members("lobby"), ["alice", "carol"]);

    // Events for other rooms do not leak into this one.
    context.apply_room_event(RoomEvent::UserJoined {
        room: "elsewhere".to_string(),
        username: "dave".to_string(),
    });
    assert_eq!(context.room_members("lobby"), ["alice", "carol"]);

    // A room we never joined has no roster, and leaving forgets it.
    assert!(context.room_members("unknown").is_empty());
    context.apply_room_event(RoomEvent::Left {
        room: "lobby".to_string(),
    });
    assert!(context.room_members("lobby").is_empty());
}

#[test]
fn a_fresh_request_discards_the_previous_answer() {
    // Without this, a poll after a second request returns the old
    // snapshot immediately and the caller cannot tell stale from fresh.
    let mut context = ClientContext::new();
    context.apply_user_status("alice".to_string(), 2, false);
    context.apply_user_stats("alice".to_string(), 10, 20, 30);
    assert!(context.user_info("alice").is_some_and(|i| i.is_complete()));

    context.invalidate_user_info("alice");
    assert!(
        context.user_info("alice").is_none(),
        "a new request must not be answerable from the old reply"
    );
}

#[test]
fn each_reply_fills_only_its_own_half() {
    let mut context = ClientContext::new();
    context.apply_user_status("bob".to_string(), 1, true);

    let info = context.user_info("bob").expect("a snapshot");
    assert!(!info.is_complete(), "stats have not arrived");
    assert_eq!(
        info.presence.map(|p| p.status),
        Some(crate::types::UserStatus::Away)
    );
    assert!(info.stats.is_none(), "must not invent statistics");

    context.apply_user_stats("bob".to_string(), 5, 6, 7);
    let info = context.user_info("bob").expect("a snapshot");
    assert!(info.is_complete());
    assert_eq!(info.stats.map(|s| s.shared_files), Some(6));
    assert_eq!(
        info.presence.map(|p| p.privileged),
        Some(true),
        "the earlier half must survive the merge"
    );
}

#[test]
fn a_user_joining_twice_is_listed_once() {
    let mut context = ClientContext::new();
    context.apply_room_event(RoomEvent::Joined {
        room: "lobby".to_string(),
        users: vec!["alice".to_string()],
    });
    context.apply_room_event(RoomEvent::UserJoined {
        room: "lobby".to_string(),
        username: "alice".to_string(),
    });
    assert_eq!(context.room_members("lobby"), ["alice"]);
}

#[test]
fn upload_speed_is_reported_only_while_running() {
    let two_seconds_ago = Instant::now()
        .checked_sub(Duration::from_secs(2))
        .expect("clock supports a 2s offset");

    // 1 KiB over ~2s is ~512 B/s.
    let rate = upload_speed(&UploadStatus::InProgress, 1024, two_seconds_ago);
    assert!((rate - 512.0).abs() < 50.0, "unexpected rate {rate}");

    // A finished upload reports no rate, exactly as a finished download
    // does, so the Speed column renders "-" rather than a stale figure.
    for status in [
        UploadStatus::Completed,
        UploadStatus::Cancelled,
        UploadStatus::Failed("nope".to_string()),
    ] {
        let rate = upload_speed(&status, 1024, two_seconds_ago);
        assert!(rate.abs() < f64::EPSILON, "unexpected rate {rate}");
    }

    // A just-started upload must not divide by a zero elapsed time.
    let rate = upload_speed(&UploadStatus::InProgress, 0, Instant::now());
    assert!(
        rate.is_finite() && rate.abs() < f64::EPSILON,
        "unexpected rate {rate}"
    );
}

// A clean disconnect — the idle reaper, or a remote client tidying an idle
// socket while it waits in our queue — must keep the peer's queued uploads;
// only an error disconnect is evidence the peer is gone.
#[test]
fn a_clean_disconnect_keeps_queued_uploads_an_error_drops_them() {
    let client = Client::new("test-user", "test-password");
    let (ops, ops_rx) = mpsc::channel();
    Client::listen_to_client_operations(
        ops_rx,
        client.context.clone(),
        "me".to_string(),
    );

    client.context.write().unwrap().enqueue_upload(
        "amy",
        "@@share\\f.mp3",
        std::path::PathBuf::from("/tmp/f.mp3"),
        4096,
    );

    ops.send(ClientOperation::PeerDisconnected(
        1,
        "amy".to_string(),
        None,
    ))
    .unwrap();
    // The loop is serial, so once this fence op is visible the disconnect
    // before it has been handled.
    ops.send(ClientOperation::OwnPrivileges(7)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.own_privilege_seconds() != Some(7) {
        assert!(Instant::now() < deadline, "ops loop never caught up");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        client
            .context
            .read()
            .unwrap()
            .place_in_queue("amy", "@@share\\f.mp3"),
        Some(1),
        "a clean disconnect must keep the queue"
    );

    ops.send(ClientOperation::PeerDisconnected(
        1,
        "amy".to_string(),
        Some(crate::error::SoulseekRs::NotConnected),
    ))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let place = client
            .context
            .read()
            .unwrap()
            .place_in_queue("amy", "@@share\\f.mp3");
        if place.is_none() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "an error disconnect must drop the queue"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
