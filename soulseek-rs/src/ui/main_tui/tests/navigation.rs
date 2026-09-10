//! Moving through a list: rows, pages, and the sideways scroll that
//! walks a name too long for its column.

use super::*;

#[test]
fn end_and_home_jump_to_the_last_and_first_result() {
    let mut tui = results_tui(30);
    press(&mut tui, KeyCode::End);
    assert_eq!(selected(&tui), Some(29));
    press(&mut tui, KeyCode::Home);
    assert_eq!(selected(&tui), Some(0));
    press(&mut tui, KeyCode::Char('G'));
    assert_eq!(selected(&tui), Some(29));
    press(&mut tui, KeyCode::Char('g'));
    assert_eq!(selected(&tui), Some(0));
}

#[test]
fn page_keys_move_a_pane_of_rows_and_stop_at_the_edges() {
    let mut tui = results_tui(30);
    press(&mut tui, KeyCode::PageDown);
    assert_eq!(selected(&tui), Some(10));
    ctrl(&mut tui, 'f');
    assert_eq!(selected(&tui), Some(20));
    press(&mut tui, KeyCode::PageDown);
    assert_eq!(selected(&tui), Some(29), "clamps instead of wrapping");
    press(&mut tui, KeyCode::PageUp);
    assert_eq!(selected(&tui), Some(19));
    ctrl(&mut tui, 'b');
    assert_eq!(selected(&tui), Some(9));
    press(&mut tui, KeyCode::PageUp);
    assert_eq!(selected(&tui), Some(0));
    assert!(
        !tui.state.show_browse && !tui.state.command_bar_active,
        "ctrl-b pages up instead of opening browse"
    );
}

#[test]
fn ctrl_d_and_ctrl_u_move_half_a_pane() {
    let mut tui = results_tui(30);
    ctrl(&mut tui, 'd');
    assert_eq!(selected(&tui), Some(5));
    ctrl(&mut tui, 'u');
    assert_eq!(selected(&tui), Some(0));
}

#[test]
fn navigation_keys_do_nothing_without_results() {
    let mut tui = results_tui(0);
    tui.state.results_table_state.select(None);
    press(&mut tui, KeyCode::End);
    press(&mut tui, KeyCode::PageDown);
    ctrl(&mut tui, 'd');
    assert_eq!(selected(&tui), None);
}

#[test]
fn l_and_h_scroll_the_highlighted_name_up_to_its_end() {
    let mut tui = results_tui(3);
    tui.state.results_items[1].filename = "x".repeat(40);
    tui.state.results_table_state.select(Some(1));
    press(&mut tui, KeyCode::Char('l'));
    assert_eq!(tui.state.results_name_offset, 8);
    press(&mut tui, KeyCode::Right);
    assert_eq!(tui.state.results_name_offset, 16);
    press(&mut tui, KeyCode::Char('$'));
    assert_eq!(tui.state.results_name_offset, 30);
    press(&mut tui, KeyCode::Char('l'));
    assert_eq!(tui.state.results_name_offset, 30, "stops at the end");
    press(&mut tui, KeyCode::Char('h'));
    assert_eq!(tui.state.results_name_offset, 22);
    press(&mut tui, KeyCode::Left);
    assert_eq!(tui.state.results_name_offset, 14);
    press(&mut tui, KeyCode::Char('0'));
    assert_eq!(tui.state.results_name_offset, 0);
    press(&mut tui, KeyCode::Char('h'));
    assert_eq!(tui.state.results_name_offset, 0);
}

#[test]
fn scrolling_follows_the_highlighted_row_not_the_longest_one() {
    let mut tui = results_tui(3);
    tui.state.results_items[0].filename = "short.mp3".to_string();
    tui.state.results_items[1].filename = "x".repeat(40);
    tui.state.results_items[2].filename = "short.mp3".to_string();
    press(&mut tui, KeyCode::Char('$'));
    assert_eq!(tui.state.results_name_offset, 0, "row 0 fits already");
    tui.state.results_table_state.select(Some(1));
    press(&mut tui, KeyCode::Char('$'));
    assert_eq!(tui.state.results_name_offset, 30);
    tui.state.results_table_state.select(Some(2));
    press(&mut tui, KeyCode::Char('h'));
    assert_eq!(
        tui.state.results_name_offset, 0,
        "a step left lands within the row now highlighted"
    );
}

#[test]
fn picking_another_search_starts_its_results_unscrolled() {
    let mut tui = results_tui(3);
    tui.state.results_items[1].filename = "x".repeat(40);
    tui.state.results_table_state.select(Some(1));
    press(&mut tui, KeyCode::Char('$'));
    assert_eq!(tui.state.results_name_offset, 30);

    tui.state.searches.push(crate::models::SearchEntry {
        query: "other".to_string(),
        status: SearchStatus::Active,
        results: results(2),
        known_files: 2,
        owned: true,
        start_time: std::time::Instant::now(),
        cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    tui.state.searches_table_state.select(Some(0));
    tui.state.focused_pane = FocusedPane::Searches;
    press(&mut tui, KeyCode::Enter);
    assert_eq!(tui.state.results_name_offset, 0);
}

#[test]
fn ctrl_keys_are_ignored_outside_the_results_pane() {
    let mut tui = results_tui(3);
    tui.state.searches.push(crate::models::SearchEntry {
        query: "keep me".to_string(),
        status: SearchStatus::Active,
        results: Vec::new(),
        known_files: 0,
        owned: true,
        start_time: std::time::Instant::now(),
        cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    tui.state.searches_table_state.select(Some(0));
    tui.state.focused_pane = FocusedPane::Searches;
    ctrl(&mut tui, 'd');
    assert_eq!(tui.state.searches.len(), 1, "ctrl-d is not d");
    ctrl(&mut tui, 'b');
    assert!(!tui.state.command_bar_active, "ctrl-b is not b");
    ctrl(&mut tui, 'q');
    assert!(tui.state.exit.is_none(), "ctrl-q is not q");
}

#[test]
fn ctrl_keys_page_while_a_filter_is_being_typed() {
    let mut tui = results_tui(30);
    press(&mut tui, KeyCode::Char('/'));
    press(&mut tui, KeyCode::Char('m'));
    ctrl(&mut tui, 'd');
    assert_eq!(selected(&tui), Some(5));
    assert_eq!(tui.state.results_filter_query, "m", "no d appended");
    press(&mut tui, KeyCode::End);
    assert_eq!(selected(&tui), Some(29));
}

#[test]
fn space_after_a_filter_that_matches_nothing_does_not_panic() {
    let mut tui = results_tui(3);
    press(&mut tui, KeyCode::Char('/'));
    press(&mut tui, KeyCode::Char('z'));
    press(&mut tui, KeyCode::Enter);
    assert!(tui.state.results_filtered_items.is_empty());
    press(&mut tui, KeyCode::Char(' '));
    assert!(tui.state.results_selected_indices.is_empty());
}

#[test]
fn l_and_h_scroll_a_transfer_name_and_a_query_too() {
    let mut tui = furnished_tui();
    tui.state.downloads[0].download.filename = "y".repeat(120);
    tui.state.searches[0].query = "z".repeat(60);
    let _ = screen_of(&mut tui);

    tui.state.focused_pane = FocusedPane::Downloads;
    press(&mut tui, KeyCode::Char('l'));
    assert_eq!(tui.state.downloads_name_offset, 8);
    press(&mut tui, KeyCode::Char('$'));
    let end = tui.state.downloads_name_offset;
    assert!(end > 8, "the end is past a step: {end}");
    let screen = screen_of(&mut tui);
    assert!(row_with(&screen, "…yyyy").contains("…yyyy"), "{screen}");
    press(&mut tui, KeyCode::Char('0'));
    assert_eq!(tui.state.downloads_name_offset, 0);

    tui.state.focused_pane = FocusedPane::Searches;
    press(&mut tui, KeyCode::Right);
    assert_eq!(tui.state.searches_query_offset, 8);
    press(&mut tui, KeyCode::End);
    assert_eq!(
        tui.state.searches_table_state.selected(),
        Some(0),
        "End is still a row key, $ is the name key"
    );
    press(&mut tui, KeyCode::Char('$'));
    assert!(tui.state.searches_query_offset > 8);
    let screen = screen_of(&mut tui);
    assert!(screen.contains("…zzzz"), "{screen}");
    assert!(
        tui.state.results_name_offset == 0,
        "each list scrolls on its own"
    );
}

#[test]
fn page_keys_work_in_the_transfers_and_searches_lists_too() {
    let mut tui = furnished_tui();
    for i in 0..30 {
        tui.state.downloads.push(crate::models::DownloadEntry {
            download: queued("bob", &format!("{i}.mp3")),
            receiver: None,
        });
    }
    tui.state.focused_pane = FocusedPane::Downloads;
    tui.state.downloads_table_state.select(Some(0));
    let _ = screen_of(&mut tui); // lays the panes out, which sets the page size

    press(&mut tui, KeyCode::End);
    assert_eq!(tui.state.downloads_table_state.selected(), Some(30));
    press(&mut tui, KeyCode::Home);
    assert_eq!(tui.state.downloads_table_state.selected(), Some(0));
    press(&mut tui, KeyCode::PageDown);
    let page = tui.state.downloads_table_state.selected().expect("row");
    assert!(page > 1 && page < 30, "a pane of rows, not one: {page}");
    ctrl(&mut tui, 'd');
    assert_eq!(tui.state.downloads.len(), 31, "ctrl-d pages, d deletes");
    assert!(tui.state.downloads_table_state.selected() > Some(page));

    tui.state.focused_pane = FocusedPane::Searches;
    press(&mut tui, KeyCode::Char('G'));
    assert_eq!(tui.state.searches_table_state.selected(), Some(0));
    press(&mut tui, KeyCode::Char('g'));
    assert_eq!(tui.state.searches_table_state.selected(), Some(0));
    assert_eq!(tui.state.searches.len(), 1, "g and G only move");
}
