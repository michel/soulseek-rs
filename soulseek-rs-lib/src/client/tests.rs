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

    let response =
        build_search_response(&shares, "me", 99, "xyzzy", true, 0, 0, &[])
            .expect("a matching share yields a response");
    let mut decoded =
        crate::message::Message::new_with_data(response.get_buffer());
    decoded.set_pointer(8);
    let result = SearchResult::new_from_message(&mut decoded).unwrap();
    assert_eq!(result.username, "me");
    assert_eq!(result.token, 99);
    assert!(result.files.iter().any(|f| f.name.contains("probe_xyzzy")));

    assert!(
        build_search_response(&shares, "me", 1, "nomatch", true, 0, 0, &[])
            .is_none()
    );
    let _ = std::fs::remove_dir_all(dir);
}

fn peer_files(count: usize) -> SearchResult {
    SearchResult {
        token: 1,
        files: (0..count)
            .map(|i| crate::types::File {
                username: "bob".to_string(),
                name: format!("song-{i}.mp3"),
                size: 1,
                attribs: std::collections::HashMap::new(),
            })
            .collect(),
        slots: 1,
        speed: 0,
        username: "bob".to_string(),
    }
}

#[test]
fn search_file_counts_cover_every_search_without_the_results() {
    // The daemon polls these counts many times a second; they must come from
    // a walk of the cache, never a copy of it.
    let client = Client::new("u", "p");
    {
        let mut context = client.context.write().unwrap();
        context.searches.insert(
            "aphex twin".to_string(),
            Search {
                token: 1,
                results: vec![peer_files(2), peer_files(1)],
            },
        );
        context.searches.insert(
            "nothing yet".to_string(),
            Search {
                token: 2,
                results: Vec::new(),
            },
        );
    }

    let mut counts = client.search_file_counts();
    counts.sort();
    assert_eq!(
        counts,
        [
            ("aphex twin".to_string(), 3),
            ("nothing yet".to_string(), 0)
        ]
    );
}

#[test]
fn a_search_stops_collecting_once_it_has_enough_responses() {
    // A popular query on the live network draws answers for minutes —
    // nearly a million files and gigabytes of memory for one search.
    // Surplus responders are dropped, not archived.
    let mut search = Search {
        token: 1,
        results: Vec::new(),
    };
    for _ in 0..(crate::types::MAX_SEARCH_RESPONSES + 50) {
        search.accept(peer_files(1));
    }
    assert_eq!(search.results.len(), crate::types::MAX_SEARCH_RESPONSES);
}

#[test]
fn a_flood_of_files_fills_a_search_before_the_response_cap() {
    // A handful of whales with huge matching collections must not add up
    // to an unbounded set just because the responses are few.
    let mut search = Search {
        token: 1,
        results: Vec::new(),
    };
    for _ in 0..10 {
        search.accept(peer_files(crate::types::MAX_SEARCH_FILES / 2));
    }
    assert_eq!(
        search.results.len(),
        2,
        "two half-cap responses fill the search; the rest are dropped"
    );
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

#[test]
fn a_pending_connect_expires_after_the_broker_timeout() {
    let mut context = ClientContext::new();
    context.add_pending_connect(7, "ghost".to_string());

    assert!(context.take_expired_connects(Instant::now()).is_empty());

    let after_deadline =
        Instant::now() + BROKER_CONNECT_TIMEOUT + Duration::from_secs(1);
    assert_eq!(
        context.take_expired_connects(after_deadline),
        vec!["ghost".to_string()]
    );
    assert!(context.take_pending_connect(7).is_none());
}

#[test]
fn protected_peers_covers_downloads_uploads_and_pending_serves() {
    let mut context = ClientContext::new();
    context.add_download(download(
        "downloader",
        "song.mp3",
        1,
        DownloadStatus::Queued,
        mpsc::channel().0,
    ));
    context.add_download(download(
        "done",
        "old.mp3",
        2,
        DownloadStatus::Completed,
        mpsc::channel().0,
    ));
    context
        .pending_serves
        .insert("waiting".to_string(), vec![9]);
    context.mark_browse_pending("browsed");

    let protected = context.protected_peers();
    assert!(protected.contains("downloader"));
    assert!(protected.contains("waiting"));
    assert!(protected.contains("browsed"));
    assert!(!protected.contains("done"));

    context.store_browse_result("browsed".to_string(), Vec::new());
    assert!(!context.protected_peers().contains("browsed"));
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

#[test]
fn room_member_stats_are_returned_sorted_and_scoped_to_their_room() {
    use crate::types::{RoomUserStats, UserStatus};
    let stat = |username: &str| RoomUserStats {
        username: username.to_string(),
        status: UserStatus::Online,
        average_speed: 1,
        shared_files: 2,
        shared_folders: 3,
        slots_full: true,
        country: None,
    };

    let mut context = ClientContext::new();
    context.apply_room_member_stats(
        "jazz".to_string(),
        vec![stat("carol"), stat("alice")],
    );

    let names: Vec<String> = context
        .room_member_stats("jazz")
        .into_iter()
        .map(|s| s.username)
        .collect();
    assert_eq!(names, vec!["alice", "carol"]);
    assert!(context.room_member_stats("unjoined").is_empty());
}

#[test]
fn rejoining_replaces_the_previous_member_stats() {
    use crate::types::{RoomUserStats, UserStatus};
    let stat = |username: &str| RoomUserStats {
        username: username.to_string(),
        status: UserStatus::Online,
        average_speed: 1,
        shared_files: 2,
        shared_folders: 3,
        slots_full: false,
        country: None,
    };

    let mut context = ClientContext::new();
    context.apply_room_member_stats("jazz".to_string(), vec![stat("alice")]);
    context.apply_room_member_stats("jazz".to_string(), vec![stat("bob")]);

    let stats = context.room_member_stats("jazz");
    assert_eq!(stats.len(), 1, "a rejoin supersedes the old snapshot");
    assert_eq!(stats[0].username, "bob");
}

#[test]
fn a_watch_reply_fills_both_halves_of_the_snapshot() {
    let mut context = ClientContext::new();
    context.add_watched_user("alice");
    context.apply_watched_user(
        "alice".to_string(),
        true,
        Some(2),
        Some(1024),
        Some(20),
        Some(3),
    );

    let info = context.user_info("alice").expect("a snapshot");
    assert!(info.is_complete(), "a watch reply carries status and stats");
    assert_eq!(info.stats.map(|s| s.average_speed), Some(1024));
    assert_eq!(context.watched_users(), vec!["alice".to_string()]);
}

#[test]
fn a_watch_reply_keeps_a_privileged_flag_it_cannot_carry() {
    // WatchUser has no privileged field, so it must not overwrite what
    // GetUserStatus already told us with a fabricated `false`.
    let mut context = ClientContext::new();
    context.apply_user_status("bob".to_string(), 1, true);
    context.apply_watched_user(
        "bob".to_string(),
        true,
        Some(2),
        Some(1),
        Some(2),
        Some(3),
    );

    let info = context.user_info("bob").expect("a snapshot");
    assert_eq!(info.presence.map(|p| p.privileged), Some(true));
}

#[test]
fn watching_an_unknown_user_drops_them_from_the_list() {
    // The server will never push status for a name it does not know, so
    // keeping it in the watch list would show a permanently blank row.
    let mut context = ClientContext::new();
    context.add_watched_user("ghost");
    context.apply_watched_user(
        "ghost".to_string(),
        false,
        None,
        None,
        None,
        None,
    );

    assert!(context.watched_users().is_empty());
    assert!(context.user_info("ghost").is_none());
}

#[test]
fn unwatching_forgets_the_users_snapshot() {
    let mut context = ClientContext::new();
    context.add_watched_user("alice");
    context.apply_user_status("alice".to_string(), 2, false);

    context.remove_watched_user("alice");
    assert!(context.watched_users().is_empty());
    assert!(
        context.user_info("alice").is_none(),
        "a later re-watch must report a fresh answer"
    );
}

#[test]
fn watched_users_are_listed_in_a_stable_order() {
    let mut context = ClientContext::new();
    context.add_watched_user("carol");
    context.add_watched_user("alice");
    context.add_watched_user("bob");
    assert_eq!(context.watched_users(), vec!["alice", "bob", "carol"]);
}

// A reply queued for a searcher the server cannot place waits a minute,
// not forever: every such search would otherwise pin a map entry and a
// protected slot in the peer registry.
#[test]
fn a_peer_message_nobody_could_deliver_expires() {
    let mut ctx = ClientContext::for_user("me");
    ctx.queue_peer_message("ghost", crate::message::Message::new());
    ctx.expire_pending_peer_messages(Instant::now());
    assert_eq!(ctx.take_peer_messages("ghost").len(), 1, "still fresh");

    ctx.queue_peer_message("ghost", crate::message::Message::new());
    ctx.expire_pending_peer_messages(Instant::now() + PENDING_PEER_TTL);
    assert!(ctx.take_peer_messages("ghost").is_empty(), "expired");
}

#[test]
fn a_new_ticker_replaces_that_users_previous_one() {
    // The server treats a user's ticker as singular: a second one from the
    // same user supersedes the first rather than stacking beside it.
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::Tickers {
        room: "jazz".into(),
        tickers: vec![
            RoomTicker {
                username: "alice".into(),
                ticker: "first".into(),
            },
            RoomTicker {
                username: "bob".into(),
                ticker: "bobs".into(),
            },
        ],
    });
    ctx.apply_room_event(RoomEvent::TickerAdded {
        room: "jazz".into(),
        username: "alice".into(),
        ticker: "second".into(),
    });

    let board = ctx.room_tickers("jazz");
    assert_eq!(board.len(), 2);
    assert_eq!(
        board.iter().find(|t| t.username == "alice").unwrap().ticker,
        "second"
    );
}

#[test]
fn a_removed_ticker_leaves_the_rest_of_the_board() {
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::Tickers {
        room: "jazz".into(),
        tickers: vec![
            RoomTicker {
                username: "alice".into(),
                ticker: "a".into(),
            },
            RoomTicker {
                username: "bob".into(),
                ticker: "b".into(),
            },
        ],
    });
    ctx.apply_room_event(RoomEvent::TickerRemoved {
        room: "jazz".into(),
        username: "alice".into(),
    });

    let board = ctx.room_tickers("jazz");
    assert_eq!(board.len(), 1);
    assert_eq!(board[0].username, "bob");
}

#[test]
fn a_global_message_is_queued_without_touching_room_membership() {
    // The global feed carries messages from rooms we have not joined; they
    // must not invent a roster for those rooms.
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::GlobalMessage {
        room: "elsewhere".into(),
        username: "alice".into(),
        message: "hi".into(),
    });
    assert!(ctx.room_members("elsewhere").is_empty());
    assert_eq!(ctx.take_room_events().len(), 1);
}

#[test]
fn own_and_global_recommendations_are_kept_apart() {
    let mut ctx = ClientContext::new();
    ctx.apply_recommendations(
        false,
        vec![Recommendation {
            item: "jazz".into(),
            rating: 2,
        }],
        Vec::new(),
    );
    ctx.apply_recommendations(
        true,
        vec![Recommendation {
            item: "pop".into(),
            rating: 9,
        }],
        Vec::new(),
    );

    assert_eq!(ctx.recommendations(false).unwrap().0[0].item, "jazz");
    assert_eq!(ctx.recommendations(true).unwrap().0[0].item, "pop");
}

#[test]
fn asking_again_about_a_users_interests_drops_the_previous_answer() {
    let mut ctx = ClientContext::new();
    ctx.apply_user_interests(UserInterests {
        username: "alice".into(),
        likes: vec!["jazz".into()],
        hates: Vec::new(),
    });
    assert!(ctx.user_interests("alice").is_some());

    ctx.invalidate_user_interests("alice");
    assert!(
        ctx.user_interests("alice").is_none(),
        "a stale answer must not be mistaken for the next one"
    );
}

#[test]
fn a_private_room_roster_tracks_who_is_added_and_removed() {
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::PrivateMembers {
        room: "club".into(),
        users: vec!["bob".into(), "alice".into(), "bob".into()],
    });
    assert_eq!(
        ctx.private_room_members("club"),
        vec!["alice".to_string(), "bob".to_string()],
        "a roster is sorted and free of repeats"
    );

    ctx.apply_room_event(RoomEvent::PrivateRosterChanged {
        room: "club".into(),
        username: "carol".into(),
        members: true,
        added: true,
    });
    ctx.apply_room_event(RoomEvent::PrivateRosterChanged {
        room: "club".into(),
        username: "alice".into(),
        members: true,
        added: false,
    });
    assert_eq!(
        ctx.private_room_members("club"),
        vec!["bob".to_string(), "carol".to_string()]
    );

    // Operators are a separate roster in the same room.
    ctx.apply_room_event(RoomEvent::PrivateRosterChanged {
        room: "club".into(),
        username: "bob".into(),
        members: false,
        added: true,
    });
    assert_eq!(ctx.private_room_operators("club"), vec!["bob".to_string()]);
    assert_eq!(ctx.private_rooms(), vec!["club".to_string()]);
}

#[test]
fn revoked_membership_drops_the_room_we_can_no_longer_see() {
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::PrivateMembers {
        room: "club".into(),
        users: vec!["alice".into()],
    });
    ctx.apply_room_event(RoomEvent::PrivateOperators {
        room: "club".into(),
        users: vec!["alice".into()],
    });

    ctx.apply_room_event(RoomEvent::OwnStandingChanged {
        room: "club".into(),
        members: true,
        granted: false,
    });
    assert!(ctx.private_rooms().is_empty());
    assert!(ctx.private_room_operators("club").is_empty());
}

#[test]
fn losing_operatorship_keeps_the_roster_we_can_still_see() {
    // Demotion does not blind us: we are still a member, and the room's other
    // operators are still ours to show. The server narrates our own removal
    // from that roster separately (code 144).
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::PrivateMembers {
        room: "club".into(),
        users: vec!["alice".into()],
    });
    ctx.apply_room_event(RoomEvent::PrivateOperators {
        room: "club".into(),
        users: vec!["alice".into(), "bob".into()],
    });

    ctx.apply_room_event(RoomEvent::OwnStandingChanged {
        room: "club".into(),
        members: false,
        granted: false,
    });
    assert_eq!(ctx.private_room_members("club"), vec!["alice".to_string()]);
    assert_eq!(
        ctx.private_room_operators("club"),
        vec!["alice".to_string(), "bob".to_string()],
        "the other operators are still there to show"
    );

    ctx.apply_room_event(RoomEvent::PrivateRosterChanged {
        room: "club".into(),
        username: "alice".into(),
        members: false,
        added: false,
    });
    assert_eq!(
        ctx.private_room_operators("club"),
        vec!["bob".to_string()],
        "only the demotion the server narrates removes us"
    );
}

#[test]
fn a_file_the_server_excludes_is_left_out_of_a_reply() {
    // The server's excluded phrases (code 160) police what travels the search
    // network: a matching file whose path carries one must not be offered,
    // and a reply with nothing left is not sent at all.
    let dir = std::env::temp_dir()
        .join(format!("soulseek-excluded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Spam_Xyzzy.bin"), b"data").unwrap();
    let shares = Shares::scan(&dir).unwrap();

    assert!(
        build_search_response(&shares, "me", 1, "xyzzy", true, 0, 0, &[])
            .is_some(),
        "with no exclusions the file is offered"
    );
    assert!(
        build_search_response(
            &shares,
            "me",
            1,
            "xyzzy",
            true,
            0,
            0,
            &["spam".to_string()],
        )
        .is_none(),
        "an excluded phrase in the path, matched case-insensitively, \
         withholds the file — and with no files there is no reply"
    );
    assert!(
        build_search_response(
            &shares,
            "me",
            1,
            "xyzzy",
            true,
            0,
            0,
            &["unrelated".to_string()],
        )
        .is_some(),
        "an exclusion the path does not carry changes nothing"
    );

    // The phrases are lowercased where they arrive, so a server that sends
    // one capitalised still matches a path.
    let mut ctx = ClientContext::new();
    ctx.set_excluded_search_phrases(vec!["SPAM".to_string()]);
    assert!(
        build_search_response(
            &shares,
            "me",
            1,
            "xyzzy",
            true,
            0,
            0,
            &ctx.excluded_search_phrases(),
        )
        .is_none(),
        "a capitalised phrase from the server still withholds the file"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn our_own_interests_are_kept_lowercased_and_deduplicated() {
    // The server matches interests case-insensitively and forgets them when
    // the session ends, so they are held here in one spelling to be sent
    // again next login.
    let mut ctx = ClientContext::new();
    assert_eq!(
        ctx.add_own_interest("  Krautrock ", true).as_deref(),
        Some("krautrock")
    );
    assert_eq!(
        ctx.add_own_interest("   ", true),
        None,
        "an empty interest is not stored, and is not sent either"
    );
    ctx.add_own_interest("KRAUTROCK", true);
    ctx.add_own_interest("Muzak", false);

    let interests = ctx.own_interests();
    assert_eq!(interests.likes, ["krautrock"]);
    assert_eq!(interests.hates, ["muzak"]);

    ctx.remove_own_interest("KrautRock", true);
    assert!(ctx.own_interests().likes.is_empty());
    assert_eq!(
        ctx.own_interests().hates,
        ["muzak"],
        "the other list stands"
    );
}

#[test]
fn leaving_a_room_drops_its_ticker_board() {
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::Tickers {
        room: "jazz".into(),
        tickers: vec![RoomTicker {
            username: "alice".into(),
            ticker: "hi".into(),
        }],
    });
    ctx.apply_room_event(RoomEvent::Left {
        room: "jazz".into(),
    });
    assert!(
        ctx.room_tickers("jazz").is_empty(),
        "a board for a room we left is stale"
    );
}

#[test]
fn the_server_is_told_our_child_capacity_only_when_it_changes() {
    // AcceptChildren is a standing state on the server, not a heartbeat.
    let mut ctx = ClientContext::new();
    assert_eq!(
        ctx.accept_children_change(),
        Some(false),
        "the first answer is always worth sending"
    );
    assert_eq!(ctx.accept_children_change(), None, "unchanged, so silent");
}
