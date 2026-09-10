//! What the transfer lists do when a key lands on a row.

use super::*;

#[test]
fn x_cancels_the_highlighted_download() {
    let session = Arc::new(TalkativeSession::default());
    let mut tui = attach(session.clone());
    tui.state.downloads.push(crate::models::DownloadEntry {
        download: at_status(
            "bob",
            "song.mp3",
            soulseek_rs::DownloadStatus::InProgress {
                bytes_downloaded: 1,
                total_bytes: 4096,
                speed_bytes_per_sec: 1.0,
            },
        ),
        receiver: None,
    });
    tui.state.downloads_table_state.select(Some(0));
    tui.state.focused_pane = FocusedPane::Downloads;

    tui.handle_key_event(key('x'));

    assert_eq!(
        *session.download_cancelled.lock().expect("not poisoned"),
        vec![("bob".to_string(), "song.mp3".to_string())]
    );
    assert!(
        session
            .upload_cancelled
            .lock()
            .expect("not poisoned")
            .is_empty(),
        "the download row cancels no upload"
    );
}

#[test]
fn shift_c_clears_the_whole_transfer_list() {
    let session = Arc::new(TalkativeSession::default());
    let mut tui = attach(session.clone());
    tui.state.downloads.push(crate::models::DownloadEntry {
        download: queued("bob", "waiting.mp3"),
        receiver: None,
    });
    tui.state.downloads.push(crate::models::DownloadEntry {
        download: at_status(
            "carol",
            "live.mp3",
            soulseek_rs::DownloadStatus::InProgress {
                bytes_downloaded: 1,
                total_bytes: 4096,
                speed_bytes_per_sec: 1.0,
            },
        ),
        receiver: None,
    });
    tui.state.downloads.push(crate::models::DownloadEntry {
        download: completed("dave", "done.mp3"),
        receiver: None,
    });
    tui.state.uploads.push(uploading("alice", "served.mp3"));
    tui.state.downloads_table_state.select(Some(0));
    tui.state.focused_pane = FocusedPane::Downloads;

    tui.handle_key_event(key('C'));

    assert!(tui.state.downloads.is_empty(), "every row goes");
    assert_eq!(
        *session.download_cancelled.lock().expect("not poisoned"),
        vec![
            ("bob".to_string(), "waiting.mp3".to_string()),
            ("carol".to_string(), "live.mp3".to_string()),
        ],
        "live downloads are cancelled; the finished one is not"
    );
    assert_eq!(
        *session.download_removed.lock().expect("not poisoned"),
        vec![
            ("bob".to_string(), "waiting.mp3".to_string()),
            ("carol".to_string(), "live.mp3".to_string()),
            ("dave".to_string(), "done.mp3".to_string()),
        ],
        "the session forgets every download, not just this window's rows"
    );
    assert_eq!(
        *session.upload_cancelled.lock().expect("not poisoned"),
        vec![("alice".to_string(), "served.mp3".to_string())],
        "a streaming upload is cancelled too"
    );
}

#[test]
fn d_clears_a_cancelled_row() {
    let mut tui = with_session(TalkativeSession::default());
    tui.state.downloads.push(crate::models::DownloadEntry {
        download: at_status(
            "bob",
            "gone.mp3",
            soulseek_rs::DownloadStatus::Cancelled,
        ),
        receiver: None,
    });
    tui.state.downloads_table_state.select(Some(0));
    tui.state.focused_pane = FocusedPane::Downloads;

    tui.handle_key_event(key('d'));

    assert!(tui.state.downloads.is_empty(), "the row goes");
}

#[test]
fn x_on_a_finished_download_asks_nothing() {
    let session = Arc::new(TalkativeSession::default());
    let mut tui = attach(session.clone());
    tui.state.downloads.push(crate::models::DownloadEntry {
        download: completed("bob", "done.mp3"),
        receiver: None,
    });
    tui.state.downloads_table_state.select(Some(0));
    tui.state.focused_pane = FocusedPane::Downloads;

    tui.handle_key_event(key('x'));

    assert!(
        session
            .download_cancelled
            .lock()
            .expect("not poisoned")
            .is_empty(),
        "a finished download is not cancelled"
    );
}

#[test]
fn x_on_an_upload_row_still_cancels_the_upload() {
    let session = Arc::new(TalkativeSession::default());
    let mut tui = attach(session.clone());
    tui.state.downloads.push(crate::models::DownloadEntry {
        download: queued("bob", "song.mp3"),
        receiver: None,
    });
    tui.state.uploads.push(uploading("alice", "served.mp3"));
    tui.state.downloads_table_state.select(Some(1));
    tui.state.focused_pane = FocusedPane::Downloads;

    tui.handle_key_event(key('x'));

    assert_eq!(
        *session.upload_cancelled.lock().expect("not poisoned"),
        vec![("alice".to_string(), "served.mp3".to_string())]
    );
    assert!(
        session
            .download_cancelled
            .lock()
            .expect("not poisoned")
            .is_empty(),
        "the upload row cancels no download"
    );
}

#[test]
fn b_browses_the_user_of_the_highlighted_download() {
    let mut tui = with_session(TalkativeSession::default());
    tui.state.downloads.push(crate::models::DownloadEntry {
        download: queued("bob", "song.mp3"),
        receiver: None,
    });
    tui.state.downloads_table_state.select(Some(0));
    tui.state.focused_pane = FocusedPane::Downloads;

    tui.handle_key_event(key('b'));

    assert!(tui.state.show_browse, "the browse popup opens");
    assert_eq!(
        tui.state.browse.active_tab().map(|b| b.username.as_str()),
        Some("bob"),
        "the download's user is the one browsed"
    );
    assert!(
        !tui.state.command_bar_active,
        "b on a transfer goes straight to the user, not the prompt"
    );
}

#[test]
fn b_on_an_upload_row_browses_that_user() {
    let mut tui = with_session(TalkativeSession::default());
    tui.state.uploads.push(uploading("alice", "served.mp3"));
    tui.state.downloads_table_state.select(Some(0));
    tui.state.focused_pane = FocusedPane::Downloads;

    tui.handle_key_event(key('b'));

    assert_eq!(
        tui.state.browse.active_tab().map(|b| b.username.as_str()),
        Some("alice")
    );
}

#[test]
fn d_clears_a_finished_row_but_not_a_running_one() {
    // Finished transfers are what a list fills up with after a restart, so
    // `d` has to be able to dismiss one — restored or not. A running
    // transfer is not ours to discard, and stays put.
    let mut tui = with_session(TalkativeSession::default());
    tui.state.downloads.push(crate::models::DownloadEntry {
        download: completed("bob", "done.mp3"),
        receiver: None,
    });
    tui.state.downloads_table_state.select(Some(0));
    tui.state.focused_pane = FocusedPane::Downloads;

    tui.handle_key_event(key('d'));

    assert!(tui.state.downloads.is_empty(), "the row goes too");
    assert_eq!(tui.state.downloads_table_state.selected(), None);

    tui.state.downloads.push(crate::models::DownloadEntry {
        download: at_status(
            "bob",
            "still.mp3",
            soulseek_rs::DownloadStatus::InProgress {
                bytes_downloaded: 1,
                total_bytes: 2,
                speed_bytes_per_sec: 1.0,
            },
        ),
        receiver: None,
    });
    tui.state.downloads_table_state.select(Some(0));

    tui.handle_key_event(key('d'));

    assert_eq!(tui.state.downloads.len(), 1, "it is not disposable");
}
