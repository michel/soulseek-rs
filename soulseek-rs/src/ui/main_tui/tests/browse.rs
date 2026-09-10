//! The browse popup, over a peer's shares and over our own.

use super::*;

#[test]
fn shift_b_shows_what_this_session_shares_and_r_re_indexes_it() {
    let session = Arc::new(TalkativeSession::default());
    *session.listing.lock().expect("not poisoned") =
        vec![soulseek_rs::SharedDirectory {
            name: "Music\\Album".to_string(),
            files: vec![soulseek_rs::SharedFileEntry {
                name: "track.mp3".to_string(),
                size: 4096,
                attributes: Vec::new(),
            }],
        }];
    let mut tui = attach(session.clone());

    press(&mut tui, KeyCode::Char('B'));
    assert!(tui.state.show_browse, "the popup opens");
    let tab = tui.state.browse.active_tab().expect("tab");
    assert!(tab.own, "on this session's own index");
    assert_eq!(tab.username, "tester");
    assert_eq!(tab.file_count, 1);
    let screen = screen_of(&mut tui);
    assert!(screen.contains("My shares"), "{screen}");
    press(&mut tui, KeyCode::Char('L'));
    let screen = screen_of(&mut tui);
    assert!(
        screen.contains("Album") && screen.contains("track.mp3"),
        "the virtual path a peer would see, folder by folder: {screen}"
    );

    // Nothing here is downloadable from ourselves.
    press(&mut tui, KeyCode::Char('d'));
    assert!(tui.state.downloads.is_empty());

    // `r` re-scans the disk rather than re-asking a peer, and the tab
    // picks up what the fresh scan found.
    session.listing.lock().expect("not poisoned").clear();
    press(&mut tui, KeyCode::Char('r'));
    assert!(
        session.shares_set.lock().expect("not poisoned").is_some(),
        "the session was asked to re-index"
    );
    assert_eq!(
        tui.state.browse.active_tab().expect("tab").file_count,
        0,
        "the view follows the new index"
    );

    // A second B focuses the tab it already opened.
    press(&mut tui, KeyCode::Esc);
    press(&mut tui, KeyCode::Char('B'));
    assert_eq!(tui.state.browse.tabs.len(), 1);
}

#[test]
fn o_leaves_the_share_view_for_the_folders_that_feed_it() {
    let mut tui = attach(Arc::new(TalkativeSession::default()));
    press(&mut tui, KeyCode::Char('B'));
    press(&mut tui, KeyCode::Char('o'));
    assert!(!tui.state.show_browse, "the popup steps aside");
    assert!(tui.state.settings.is_some(), "for the settings pane");
}

#[test]
fn page_keys_walk_the_browse_tree_and_ctrl_d_does_not_download() {
    let session = Arc::new(TalkativeSession::default());
    let mut tui = attach(session);
    browsed(&mut tui, 40);
    let rows = tui.state.browse.active_tab().expect("tab").rows().len();
    assert_eq!(rows, 41, "the folder and its files");

    press(&mut tui, KeyCode::End);
    let selected = |tui: &MainTui| {
        tui.state.browse.active_tab().expect("tab").selected_row
    };
    assert_eq!(selected(&tui), 40);
    press(&mut tui, KeyCode::Home);
    assert_eq!(selected(&tui), 0);
    press(&mut tui, KeyCode::PageDown);
    let page = selected(&tui);
    assert!(page > 1 && page < 40, "a page of rows: {page}");
    ctrl(&mut tui, 'd');
    assert!(selected(&tui) > page, "ctrl-d pages");
    assert!(
        tui.state.downloads.is_empty()
            && tui.state.downloads_receiver_channel.is_none(),
        "and does not download the folder"
    );
    press(&mut tui, KeyCode::Char('G'));
    assert_eq!(selected(&tui), 40);
    assert!(tui.state.show_browse, "still open");
}

#[test]
fn folder_keys_and_the_filter_move_through_a_browsed_tree() {
    let mut tui = attach(Arc::new(TalkativeSession::default()));
    tui.state.browse.open("bob");
    let listing = vec![
        soulseek_rs::SharedDirectory {
            name: "music\\alpha".to_string(),
            files: vec![
                soulseek_rs::SharedFileEntry {
                    name: "a1.mp3".to_string(),
                    size: 1,
                    attributes: Vec::new(),
                },
                soulseek_rs::SharedFileEntry {
                    name: "a2.flac".to_string(),
                    size: 2,
                    attributes: Vec::new(),
                },
            ],
        },
        soulseek_rs::SharedDirectory {
            name: "music\\beta".to_string(),
            files: vec![soulseek_rs::SharedFileEntry {
                name: "b1.mp3".to_string(),
                size: 3,
                attributes: Vec::new(),
            }],
        },
    ];
    tui.state
        .browse
        .active_tab_mut()
        .expect("tab")
        .load(&listing);
    tui.state.show_browse = true;
    let _ = screen_of(&mut tui);
    let selected = |tui: &MainTui| {
        tui.state.browse.active_tab().expect("tab").selected_row
    };
    let rows = |tui: &MainTui| {
        tui.state.browse.active_tab().expect("tab").rows().len()
    };

    press(&mut tui, KeyCode::Char('L'));
    assert_eq!(rows(&tui), 6, "every folder open");
    press(&mut tui, KeyCode::Char('J'));
    press(&mut tui, KeyCode::Char('J'));
    assert_eq!(selected(&tui), 4, "beta, skipping alpha's files");
    press(&mut tui, KeyCode::Char('K'));
    assert_eq!(selected(&tui), 1, "back to alpha");
    press(&mut tui, KeyCode::Char('H'));
    assert_eq!(rows(&tui), 1, "just music");

    press(&mut tui, KeyCode::Char('/'));
    for c in "flac".chars() {
        press(&mut tui, KeyCode::Char(c));
    }
    assert_eq!(rows(&tui), 3, "music, alpha, a2.flac");
    let screen = screen_of(&mut tui);
    assert!(screen.contains("filter: flac_"), "{screen}");
    ctrl(&mut tui, 'd');
    assert_eq!(
        tui.state.browse.active_tab().expect("tab").filter(),
        "flac",
        "a control chord pages, it does not type"
    );
    press(&mut tui, KeyCode::Enter);
    press(&mut tui, KeyCode::Char('j'));
    press(&mut tui, KeyCode::Char('j'));
    assert_eq!(selected(&tui), 2, "j moves again once the filter is kept");
    press(&mut tui, KeyCode::Esc);
    assert_eq!(rows(&tui), 1, "Esc clears the filter first");
    assert!(tui.state.show_browse);
    press(&mut tui, KeyCode::Esc);
    assert!(!tui.state.show_browse, "then hides the popup");
}
