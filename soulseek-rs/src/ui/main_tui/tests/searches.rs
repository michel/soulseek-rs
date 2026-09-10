//! Running a search the list already holds.

use super::*;

#[test]
fn shift_s_runs_the_highlighted_search_again_in_its_place() {
    // The list survives a restart, but a query on it was only ever a
    // reminder: seeing what the network has now meant typing it again.
    let session = Arc::new(TalkativeSession::default());
    let mut tui =
        restored(session.clone(), &["boards of canada", "aphex twin"]);
    tui.state.searches_table_state.select(Some(1));

    press(&mut tui, KeyCode::Char('S'));

    assert_eq!(
        searched(&session, 1),
        vec!["aphex twin".to_string()],
        "the highlighted query went out again, untyped"
    );
    let queries: Vec<&str> = tui
        .state
        .searches
        .iter()
        .map(|s| s.query.as_str())
        .collect();
    assert_eq!(
        queries,
        ["boards of canada", "aphex twin"],
        "it keeps its row: no duplicate, nothing moved"
    );
    assert_eq!(tui.state.searches[1].status, SearchStatus::Active);
    assert_eq!(tui.state.selected_search_index, Some(1));
    assert_eq!(
        tui.state.focused_pane,
        FocusedPane::Results,
        "and the window turns to where the results will land"
    );
    assert!(tui.state.results_items.is_empty());
}

#[test]
fn shift_s_stops_a_run_still_collecting_before_starting_over() {
    let mut tui = furnished_tui();
    tui.state.searches[0].status = SearchStatus::Active;
    tui.state.searches[0].start_time = std::time::Instant::now()
        .checked_sub(Duration::from_secs(2))
        .expect("two seconds ago");
    let previous = tui.state.searches[0].cancel_flag.clone();
    tui.state.focused_pane = FocusedPane::Searches;
    tui.state.searches_table_state.select(Some(0));

    press(&mut tui, KeyCode::Char('S'));

    assert!(
        previous.load(std::sync::atomic::Ordering::Relaxed),
        "the worker that was collecting is told to stop"
    );
    let fresh = &tui.state.searches[0];
    assert!(!fresh.cancel_flag.load(std::sync::atomic::Ordering::Relaxed));
    assert!(fresh.results.is_empty(), "stale rows do not linger");
    assert_eq!(tui.state.searches.len(), 1);
}

#[test]
fn shift_s_is_only_a_key_over_the_searches_list() {
    let session = Arc::new(TalkativeSession::default());
    let mut tui = restored(session.clone(), &["aphex twin"]);

    // Over Results the same key means nothing, so a stray shift does not
    // throw a finished search away.
    tui.state.focused_pane = FocusedPane::Results;
    press(&mut tui, KeyCode::Char('S'));
    assert_eq!(tui.state.searches[0].status, SearchStatus::Completed);
    std::thread::sleep(Duration::from_millis(50));
    assert!(searched(&session, 0).is_empty(), "nothing went out");
}

#[test]
fn a_held_shift_s_is_one_search_not_one_per_repeat() {
    let session = Arc::new(TalkativeSession::default());
    let mut tui = restored(session.clone(), &["aphex twin"]);
    press(&mut tui, KeyCode::Char('S'));
    press(&mut tui, KeyCode::Char('1'));
    press(&mut tui, KeyCode::Char('S'));
    assert_eq!(searched(&session, 1).len(), 1);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(searched(&session, 1).len(), 1, "the repeat is ignored");
}
