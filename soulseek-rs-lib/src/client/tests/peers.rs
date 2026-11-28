//! Uploads, broker connects, and the peers a disconnect may not drop.

use super::*;

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

/// A client under the parent "dad" whose child slots are all taken, that
/// has told the server so, and one of whose children is gone. Feeding
/// `relay` to its operations loop makes it write to every child.
///
/// The gone child's link is shut down at our end rather than dropped at
/// theirs, so the write to it fails at once instead of after the RST has
/// crossed the loopback.
fn the_server_hears_of_room_after(relay: impl FnOnce(u64) -> ClientOperation) {
    const ACCEPT_CHILDREN: u32 = 100;

    let (server, server_rx) = mpsc::channel();
    let (ops, ops_rx) = mpsc::channel();
    let context = Arc::new(RwLock::new(ClientContext::for_user("me")));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let mut child_ends = Vec::new();
    let mut gone = None;
    let link = {
        let mut ctx = context.write().unwrap();
        ctx.server_sender = Some(server);
        ctx.operations = Some(ops.clone());
        ctx.children = children::Children::new(true);
        ctx.set_fed_by(true);
        let dial = ctx
            .leaf
            .consider(vec![("dad".into(), "127.0.0.1".into(), 2234)])
            .remove(0);
        ctx.leaf.branch_level("dad", dial.link, 0);
        assert!(ctx.leaf.search_from("dad", dial.link).is_some());
        for i in 0..children::MAX_CHILDREN {
            let ours = TcpStream::connect(listener.local_addr().unwrap())
                .expect("connect");
            child_ends.push(listener.accept().expect("accept").0);
            gone.get_or_insert_with(|| ours.try_clone().expect("clone"));
            assert!(ctx.children.accept(&format!("kid{i}"), ours));
        }
        assert_eq!(child_ends.len(), children::MAX_CHILDREN, "links held");
        assert_eq!(ctx.accept_children_change(), Some(false), "told: full");
        dial.link
    };
    gone.expect("a child")
        .shutdown(std::net::Shutdown::Both)
        .expect("shutdown");

    Client::listen_to_client_operations(ops_rx, context, "me".to_string());
    ops.send(relay(link)).unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        let Ok(message) = server_rx.recv_timeout(wait) else {
            panic!("the server never heard that a slot opened");
        };
        let ServerMessage::SendMessage(message) = message else {
            continue;
        };
        let data = message.get_data();
        if data[..4] == ACCEPT_CHILDREN.to_le_bytes() && data[4] == 1 {
            break;
        }
    }
}

#[test]
fn a_child_lost_relaying_our_parents_search_tells_the_server_we_have_room() {
    the_server_hears_of_room_after(|link| ClientOperation::ParentSearch {
        parent: "dad".to_string(),
        link,
        username: "seeker".to_string(),
        token: 1,
        query: "jazz".to_string(),
    });
}

#[test]
fn a_child_lost_hearing_our_new_place_tells_the_server_we_have_room() {
    the_server_hears_of_room_after(|link| ClientOperation::ParentBranchLevel {
        parent: "dad".to_string(),
        link,
        level: 3,
    });
}
