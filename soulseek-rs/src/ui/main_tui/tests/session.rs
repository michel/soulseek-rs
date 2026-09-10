//! A window over a session it did not start: what a daemon's state
//! looks like arriving in a window, and what the window pushes back.

use super::*;

#[test]
fn an_attached_session_opens_with_the_conversation_already_in_it() {
    // A TUI attached to a daemon has no state file of its own; the
    // conversation has to come from the session, or it starts blank while
    // the daemon has been collecting messages for hours.
    let tui = tui(vec![
        ChatMessageDto {
            peer: "bob".into(),
            outgoing: false,
            text: "hi".into(),
            at: 1,
        },
        ChatMessageDto {
            peer: "bob".into(),
            outgoing: true,
            text: "hello".into(),
            at: 2,
        },
    ]);

    assert_eq!(tui.state.messages.len(), 2);
    assert_eq!(tui.state.messages[0].peer, "bob");
    assert_eq!(tui.state.messages[0].direction, MessageDirection::Incoming);
    assert_eq!(
        tui.state.messages[1].direction,
        MessageDirection::Outgoing,
        "both halves of the conversation, not just what arrived"
    );
    assert_eq!(tui.state.chat_peers(), ["bob"]);
}

#[test]
fn a_window_shows_transfers_it_did_not_start() {
    // Two windows on one daemon are two views of one queue. A transfer
    // queued in the other one has to appear here, or closing a window
    // would look like losing its downloads.
    let mut tui = with_session(TalkativeSession {
        downloads: vec![queued("bob", "@@x\\theirs.mp3")],
        shared: true,
        ..TalkativeSession::default()
    });
    assert!(
        tui.state.downloads.is_empty(),
        "nothing until the first poll"
    );

    tui.update_downloads();

    assert_eq!(tui.state.downloads.len(), 1);
    assert_eq!(tui.state.downloads[0].download.filename, "@@x\\theirs.mp3");
    assert!(
        tui.state.downloads[0].receiver.is_none(),
        "someone else's transfer has no channel of ours"
    );

    // Polling again must not double it up.
    tui.update_downloads();
    assert_eq!(tui.state.downloads.len(), 1);
}

#[test]
fn a_transfer_the_session_forgets_leaves_every_window() {
    let mut tui = with_session(TalkativeSession {
        downloads: vec![queued("bob", "gone.mp3")],
        shared: true,
        ..TalkativeSession::default()
    });
    tui.update_downloads();
    assert_eq!(tui.state.downloads.len(), 1);

    // The session no longer has it — cancelled from the other window.
    tui.client = Arc::new(TalkativeSession {
        shared: true,
        ..TalkativeSession::default()
    });
    tui.update_downloads();
    assert!(
        tui.state.downloads.is_empty(),
        "a queue is shared in both directions"
    );
}

#[test]
fn a_window_shows_searches_run_in_another() {
    let mut tui =
        with_session(shared(vec![search_of("their query", 7, 9_999)]));
    tui.update_search_results();

    assert_eq!(tui.state.searches.len(), 1);
    assert_eq!(tui.state.searches[0].query, "their query");

    // And it is not added a second time on the next frame.
    tui.update_search_results();
    assert_eq!(tui.state.searches.len(), 1);
}

#[test]
fn a_search_another_window_is_still_running_shows_as_active() {
    // It had said "Done" the moment it appeared, because an imported
    // search carried no notion of still collecting — so the other window
    // looked finished while results were still arriving.
    let mut tui = with_session(shared(vec![search_of("still going", 3, 0)]));
    tui.update_search_results();

    assert_eq!(tui.state.searches[0].status, SearchStatus::Active);
    assert_eq!(
        tui.state.searches[0].known_files, 3,
        "and its count comes across before the results do"
    );
}

#[test]
fn a_search_that_has_run_its_window_shows_as_done() {
    let mut tui = with_session(shared(vec![search_of("finished", 12, 9_999)]));
    tui.update_search_results();
    assert_eq!(tui.state.searches[0].status, SearchStatus::Completed);
}

#[test]
fn removing_a_search_removes_it_from_the_session() {
    // Removing it only from this window would last until the next sync,
    // which would put it straight back — and in a shared session it would
    // come back for every other window too.
    let mut tui = with_session(shared(vec![search_of("unwanted", 1, 9_999)]));
    tui.update_search_results();
    assert_eq!(tui.state.searches.len(), 1);

    tui.remove_search_at_index(0);
    tui.update_search_results();

    assert!(tui.state.searches.is_empty(), "and it stays gone");
}

#[test]
fn clearing_every_search_clears_them_in_the_session_too() {
    let mut tui = with_session(shared(vec![
        search_of("one", 1, 9_999),
        search_of("two", 2, 9_999),
    ]));
    tui.update_search_results();
    assert_eq!(tui.state.searches.len(), 2);

    tui.clear_all_searches();
    tui.update_search_results();

    assert!(
        tui.state.searches.is_empty(),
        "clearing has to reach the session, or every row comes back"
    );
}

#[test]
fn a_search_dismissed_in_another_window_leaves_this_one() {
    let session = shared(vec![search_of("theirs", 4, 9_999)]);
    let mut tui = with_session(session);
    tui.update_search_results();
    assert_eq!(tui.state.searches.len(), 1);

    // The other window dismissed it.
    tui.client.forget_search("theirs");
    tui.update_search_results();

    assert!(tui.state.searches.is_empty());
}

#[test]
fn a_tick_asks_the_session_for_its_searches_once() {
    // Against a daemon every call is a round trip, and the daemon answers
    // it by walking its whole search cache — twice per frame was half the
    // cost of attaching at all.
    let session = Arc::new(shared(vec![search_of("q", 0, 9_999)]));
    let mut tui = attach(session.clone());

    tui.update_search_results();

    assert_eq!(
        session
            .all_searches_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "one sync means one listing, not one per use of it"
    );
}

#[test]
fn a_selected_search_with_nothing_new_is_not_pulled_across_again() {
    // The result set only accumulates, so an unchanged count means a
    // fetch would hand back what this window already holds — against a
    // daemon that is the entire result set over the socket, every frame,
    // forever.
    let (session, tui) = window_that_pulled_two_files();

    assert_eq!(tui.state.searches[0].results.len(), 2);
    assert_eq!(
        session
            .result_fetches
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "the set crosses once; after that the unchanged count spares it"
    );
}

#[test]
fn fresh_results_still_reach_a_window_that_skipped_a_fetch() {
    let (session, mut tui) = window_that_pulled_two_files();

    // A late responder lands in the session.
    *session.searches.lock().expect("not poisoned") =
        vec![search_of("aphex twin", 3, 9_999)];
    *session.results.lock().expect("not poisoned") =
        vec![files_from_bob(&["@@x\\a.mp3", "@@x\\b.mp3", "@@x\\c.mp3"])];
    tui.update_search_results();

    assert_eq!(
        tui.state.searches[0].results.len(),
        3,
        "a changed count must bring the new results across"
    );
}

#[test]
fn a_local_window_fetches_results_without_counting_on_the_session() {
    // Locally nothing maintains known_files, so the count gate must stay
    // out of the way: a search whose count still reads zero has to get
    // its results anyway.
    let session = Arc::new(TalkativeSession {
        results: std::sync::Mutex::new(vec![files_from_bob(&[
            "@@x\\a.mp3",
            "@@x\\b.mp3",
        ])]),
        shared: false,
        ..TalkativeSession::default()
    });
    let mut tui = attach(session);
    tui.state.searches.push(crate::models::SearchEntry {
        query: "mine".to_string(),
        status: SearchStatus::Active,
        results: Vec::new(),
        known_files: 0,
        owned: true,
        start_time: std::time::Instant::now(),
        cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });

    tui.update_search_results();

    assert_eq!(
        tui.state.searches[0].results.len(),
        2,
        "a zero count matching zero held rows must not read as \
         nothing-to-fetch on a local session"
    );
}

#[test]
fn a_window_with_its_own_session_is_left_alone() {
    // Nothing to share, so nothing is synced — and in particular the
    // completed transfers restored from disk, which the session has never
    // heard of, are not mistaken for cancelled and deleted.
    let mut tui = with_session(TalkativeSession {
        downloads: Vec::new(),
        shared: false,
        ..TalkativeSession::default()
    });
    tui.state.downloads.push(crate::models::DownloadEntry {
        download: queued("bob", "from-disk.mp3"),
        receiver: None,
    });

    tui.update_downloads();

    assert_eq!(
        tui.state.downloads.len(),
        1,
        "a local window's history must survive the tick"
    );
}

#[test]
fn a_local_session_starts_from_its_own_snapshot_instead() {
    // Locally the session knows no history and the state file is the whole
    // record, so nothing is invented.
    assert!(tui(Vec::new()).state.messages.is_empty());
}
