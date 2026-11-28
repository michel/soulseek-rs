//! What a session must not accumulate when it stays up for days.
//!
//! Each of these held something for the life of the process: nothing went
//! wrong in an hour, and all of it showed in a daemon left running.

use super::*;
use crate::message::Message;
use crate::message::peer::PeerInfo;

// The daemon logs back in by itself. It did so with the password it was
// started with, so one `account.password` meant every reconnect after it was
// refused until the daemon was restarted.
#[test]
fn a_changed_password_is_sent_with_the_next_login() {
    use crate::actor::ActorMessage;

    fn assert_traits<
        T: Send + Sync + std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    >() {
    }
    assert_traits::<Client>();

    let mut client = Client::new("me", "first-pw");
    let (sender, receiver) = mpsc::channel();
    client.server_handle = Some(ActorHandle { sender });

    client
        .change_password("second-pw")
        .expect("change password");
    let ActorMessage::UserMessage(ServerMessage::SendMessage(mut change)) =
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("password change")
    else {
        panic!("expected a password change");
    };
    assert_eq!(change.read_int32(), 142);
    assert_eq!(change.read_string(), "second-pw");

    thread::scope(|scope| {
        let login = scope.spawn(|| client.login());
        let ActorMessage::UserMessage(ServerMessage::Login {
            password,
            response,
            ..
        }) = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("login request")
        else {
            panic!("expected a login request");
        };
        response.send(Ok(true)).expect("login verdict");
        assert!(login.join().expect("login thread").expect("login"));
        assert_eq!(
            password, "second-pw",
            "the next login uses the changed password"
        );
    });

    client.session.record(SessionLoss::Disconnected);
    assert!(
        client.change_password("third-pw").is_err(),
        "a change the server cannot hear must not become our password"
    );
}

// Issue #116: one expiry clock per peer, restarted by every message, never
// ran out for a peer that kept being sent things.
#[test]
fn a_peer_message_expires_on_its_own_clock_whatever_is_queued_after_it() {
    let mut ctx = ClientContext::for_user("me");
    ctx.queue_peer_message("ghost", Message::new());
    let first_queued = Instant::now();

    // The searcher keeps asking, so replies keep arriving for it: one more
    // just inside the first one's lifetime.
    let later = first_queued + (PENDING_PEER_TTL / 2);
    ctx.pending_peer_messages
        .get_mut("ghost")
        .expect("queued")
        .push_back((later, Message::new()));

    ctx.expire_pending_peer_messages(first_queued + PENDING_PEER_TTL);
    assert_eq!(
        ctx.pending_peer_messages["ghost"].len(),
        1,
        "the first message is out of time, however fresh the second is"
    );

    ctx.expire_pending_peer_messages(later + PENDING_PEER_TTL);
    assert!(
        ctx.pending_peer_messages.is_empty(),
        "and a peer with nothing left waiting is forgotten"
    );
}

#[test]
fn the_messages_held_for_one_peer_are_capped() {
    let mut ctx = ClientContext::for_user("me");
    for _ in 0..(MAX_PENDING_PEER_MESSAGES * 4) {
        ctx.queue_peer_message("ghost", Message::new());
    }
    assert_eq!(
        ctx.take_peer_messages("ghost").len(),
        MAX_PENDING_PEER_MESSAGES
    );
}

// Issue #117: replies were stored whether or not anyone had asked, under
// whatever name and folder the peer chose.
#[test]
fn a_listing_nobody_asked_for_is_not_kept() {
    let mut ctx = ClientContext::new();
    ctx.store_browse_result("stranger".to_string(), Vec::new());
    assert!(ctx.take_browse_result("stranger").is_none());

    ctx.mark_browse_pending("asked");
    ctx.store_browse_result("asked".to_string(), Vec::new());
    assert!(ctx.take_browse_result("asked").is_some());

    // One request buys one answer.
    ctx.store_browse_result("asked".to_string(), Vec::new());
    assert!(ctx.take_browse_result("asked").is_none());
}

#[test]
fn a_folder_nobody_asked_for_is_not_kept() {
    let mut ctx = ClientContext::new();
    for unasked in 0..100 {
        ctx.store_folder_contents(
            "peer".to_string(),
            format!("made-up-{unasked}"),
            Vec::new(),
        );
    }
    assert!(ctx.folder_contents.is_empty());

    ctx.expect_folder_contents("peer", "music\\album");
    ctx.store_folder_contents(
        "peer".to_string(),
        "music\\album".to_string(),
        Vec::new(),
    );
    assert!(ctx.take_folder_contents("peer", "music\\album").is_some());
}

#[test]
fn a_self_description_nobody_asked_for_is_not_kept() {
    let mut ctx = ClientContext::new();
    ctx.store_peer_info("stranger".to_string(), PeerInfo::default());
    assert!(ctx.peer_info("stranger").is_none());

    ctx.expect_peer_info("asked");
    ctx.store_peer_info("asked".to_string(), PeerInfo::default());
    assert!(ctx.peer_info("asked").is_some());
}

#[test]
fn the_self_descriptions_kept_are_capped() {
    let mut ctx = ClientContext::new();
    for peer in 0..(MAX_PEER_INFOS + 20) {
        let name = format!("peer{peer}");
        ctx.expect_peer_info(&name);
        ctx.store_peer_info(name, PeerInfo::default());
    }
    assert_eq!(ctx.peer_infos.len(), MAX_PEER_INFOS);
}

#[test]
fn an_answer_nobody_collects_is_dropped_in_the_end() {
    let mut ctx = ClientContext::new();
    ctx.mark_browse_pending("slow");
    ctx.store_browse_result("slow".to_string(), Vec::new());
    ctx.expect_folder_contents("slow", "f");
    ctx.store_folder_contents("slow".to_string(), "f".to_string(), Vec::new());
    ctx.expect_folder_contents("silent", "f");
    ctx.expect_peer_info("silent");

    let now = Instant::now();
    ctx.expire_peer_caches(now);
    assert_eq!(ctx.browse_results.len(), 1, "still fresh");

    ctx.expire_peer_caches(now + PEER_REPLY_TTL);
    assert!(ctx.browse_results.is_empty());
    assert!(ctx.folder_contents.is_empty());
    assert!(ctx.pending_folders.is_empty(), "nor is the wait kept");
    assert!(ctx.pending_peer_infos.is_empty());
}

// Issue #118: only an explicit forget ever dropped a search.
#[test]
fn the_oldest_search_makes_room_once_there_are_too_many() {
    let mut ctx = ClientContext::new();
    for query in 0..(MAX_SEARCHES + 10) {
        ctx.register_search(&format!("query {query}"), query as u32);
    }

    assert_eq!(ctx.searches.len(), MAX_SEARCHES);
    assert!(!ctx.searches.contains_key("query 0"), "the oldest went");
    assert!(
        ctx.searches
            .contains_key(&format!("query {}", MAX_SEARCHES + 9))
    );
}

#[test]
fn searching_again_keeps_that_search_among_the_newest() {
    let mut ctx = ClientContext::new();
    ctx.register_search("wishlist", 1);
    for query in 0..(MAX_SEARCHES - 1) {
        ctx.register_search(&format!("query {query}"), 100 + query as u32);
    }
    // Run again, as a wishlist search is on every interval.
    ctx.register_search("wishlist", 2);
    ctx.register_search("one more", 3);

    assert_eq!(ctx.searches.len(), MAX_SEARCHES);
    assert_eq!(ctx.searches["wishlist"].token, 2);
    assert!(!ctx.searches.contains_key("query 0"));
}

#[test]
fn forgetting_a_search_frees_its_place() {
    let mut ctx = ClientContext::new();
    ctx.register_search("a", 1);
    assert!(ctx.forget_search("a"));
    assert!(!ctx.forget_search("a"));
    assert!(ctx.search_order.is_empty());
}

// The cap on finished downloads was only applied when one was added, so a
// batch queued first and finished later stayed above it.
#[test]
fn a_finished_batch_is_capped_and_every_download_gets_its_terminal_status() {
    use crate::download_store::MAX_FINISHED_DOWNLOADS;

    let mut retained = Vec::new();
    for status in [
        DownloadStatus::Completed,
        DownloadStatus::Failed(None),
        DownloadStatus::TimedOut,
        DownloadStatus::Cancelled,
    ] {
        let client = Client::new("u", "p");
        let mut batch = Vec::new();
        for token in 0..=(MAX_FINISHED_DOWNLOADS as u32) {
            let (sender, receiver) = mpsc::channel();
            let download = download(
                "peer",
                &format!("file-{token}.mp3"),
                token,
                DownloadStatus::Queued,
                sender,
            );
            client
                .context
                .write()
                .unwrap()
                .add_download(download.clone());
            batch.push((download, receiver));
        }

        // The oldest finishes last, so pruning evicts the row being notified.
        for (download, receiver) in batch.into_iter().rev() {
            if matches!(status, DownloadStatus::Cancelled) {
                assert!(client.cancel_download("peer", &download.filename));
            } else {
                DownloadPeer::send_download_status(
                    &client.context,
                    &download,
                    status.clone(),
                );
            }
            assert_eq!(
                std::mem::discriminant(&receiver.try_recv().unwrap()),
                std::mem::discriminant(&status),
                "an evicted download is still told how it ended"
            );
        }
        retained.push(client.get_all_downloads().len());
    }
    assert_eq!(retained, vec![MAX_FINISHED_DOWNLOADS; 4]);
}

// Issue #120: an address was kept, and used, for as long as the process
// lived; and every user the server mentioned got a snapshot.
#[test]
fn a_peer_address_is_looked_up_again_once_it_is_old() {
    let mut ctx = ClientContext::new();
    ctx.cache_peer_address("amy", "10.0.0.1".to_string(), 2234);
    assert_eq!(
        ctx.peer_address("amy"),
        Some(("10.0.0.1".to_string(), 2234))
    );

    let learned = Instant::now()
        .checked_sub(PEER_ADDRESS_TTL)
        .expect("the clock is past the TTL");
    ctx.peer_addresses.get_mut("amy").expect("cached").2 = learned;
    assert_eq!(
        ctx.peer_address("amy"),
        None,
        "a stale address is a dial that times out"
    );

    ctx.expire_peer_caches(Instant::now());
    assert!(ctx.peer_addresses.is_empty());
}

#[test]
fn server_pushes_about_users_we_never_asked_about_are_not_kept() {
    let mut ctx = ClientContext::new();
    for user in 0..1000 {
        let name = format!("passer-by-{user}");
        ctx.apply_user_status(name.clone(), 2, false);
        ctx.apply_user_stats(name, 1, 2, 3);
    }
    assert!(ctx.user_info.is_empty());

    ctx.add_watched_user("friend");
    ctx.apply_user_status("friend".to_string(), 2, false);
    ctx.expect_user_info("asked");
    ctx.apply_user_stats("asked".to_string(), 1, 2, 3);
    assert!(ctx.user_info("friend").is_some());
    assert!(ctx.user_info("asked").is_some());
}

#[test]
fn the_users_we_asked_about_are_capped_and_the_watch_list_is_spared() {
    let mut ctx = ClientContext::new();
    ctx.add_watched_user("friend");
    ctx.expect_user_info("friend");
    ctx.apply_user_status("friend".to_string(), 2, false);

    for user in 0..(MAX_USER_INFO_REQUESTS + 50) {
        let name = format!("user{user}");
        ctx.expect_user_info(&name);
        ctx.apply_user_status(name, 2, false);
    }

    assert_eq!(ctx.user_info.len(), MAX_USER_INFO_REQUESTS + 1);
    assert!(ctx.user_info("user0").is_none(), "the oldest request went");
    assert!(ctx.user_info("friend").is_some(), "a watched user stays");
}
