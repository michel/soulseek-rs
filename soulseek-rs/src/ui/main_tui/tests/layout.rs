//! The window itself: which panes are up, how big, and what the chrome
//! says while they move.

use super::*;

#[test]
fn the_results_pane_has_the_whole_width_to_itself() {
    let mut tui = furnished_tui();
    let screen = screen_of(&mut tui);

    let results = row_with(&screen, "[2] Results");
    assert!(results.starts_with('╭'), "{results}");
    assert!(results.trim_end().ends_with('╮'), "{results}");
    assert!(!results.contains("[1]"), "nothing beside it: {results}");

    // The other three share the row underneath, in this order.
    let row = row_with(&screen, "[1] Searches");
    let searches = row.find("[1]").expect("searches");
    let downloads = row.find("[3]").expect("downloads");
    let info = row.find("[Info]").expect("info");
    assert!(searches < downloads && downloads < info, "{row}");
}

#[test]
fn tab_and_shift_tab_walk_the_panes_in_legend_order() {
    let mut tui = furnished_tui();
    tui.state.focused_pane = FocusedPane::Searches;
    press(&mut tui, KeyCode::Tab);
    assert_eq!(tui.state.focused_pane, FocusedPane::Results);
    press(&mut tui, KeyCode::Tab);
    assert_eq!(tui.state.focused_pane, FocusedPane::Downloads);
    press(&mut tui, KeyCode::Tab);
    assert_eq!(tui.state.focused_pane, FocusedPane::Searches);
    press(&mut tui, KeyCode::BackTab);
    assert_eq!(tui.state.focused_pane, FocusedPane::Downloads);
}

#[test]
fn w_hides_the_focused_pane_and_its_number_brings_it_back() {
    let mut tui = furnished_tui();
    tui.state.focused_pane = FocusedPane::Searches;

    press(&mut tui, KeyCode::Char('w'));

    let screen = screen_of(&mut tui);
    assert!(!screen.contains("[1] Searches"), "{screen}");
    assert!(screen.contains("[2] Results"), "{screen}");
    assert!(screen.contains("[3] Downloads"), "{screen}");
    assert_eq!(tui.state.focused_pane, FocusedPane::Results);
    assert_eq!(tui.state.searches_pane_area, None, "nothing to click");
    // The transfers widen into the space.
    let row = row_with(&screen, "[3] Downloads");
    assert!(row.starts_with('╭'), "{row}");

    press(&mut tui, KeyCode::Char('1'));

    let screen = screen_of(&mut tui);
    assert!(screen.contains("[1] Searches"), "{screen}");
    assert_eq!(tui.state.focused_pane, FocusedPane::Searches);
}

#[test]
fn hiding_the_results_gives_the_row_the_whole_height() {
    let mut tui = furnished_tui();
    press(&mut tui, KeyCode::Char('w'));
    let screen = screen_of(&mut tui);
    assert!(!screen.contains("[2] Results"), "{screen}");
    // The row now starts right under the status bar.
    let status = screen
        .lines()
        .position(|line| line.contains("Status"))
        .expect("status bar");
    let row = screen
        .lines()
        .position(|line| line.contains("[1] Searches"))
        .expect("searches");
    assert_eq!(row, status + 3, "{screen}");
}

#[test]
fn the_last_pane_standing_cannot_be_hidden() {
    // Results goes, then Downloads; Searches is what is left, and a
    // third press leaves it there.
    let mut tui = furnished_tui();
    press(&mut tui, KeyCode::Char('w'));
    press(&mut tui, KeyCode::Char('w'));
    press(&mut tui, KeyCode::Char('w'));
    let screen = screen_of(&mut tui);
    assert!(screen.contains("[1] Searches"), "{screen}");
    assert!(!screen.contains("[3] Downloads"), "{screen}");
    assert_eq!(tui.state.focused_pane, FocusedPane::Searches);
}

#[test]
fn z_zooms_the_focused_pane_and_esc_or_z_leaves_it() {
    let mut tui = furnished_tui();
    press(&mut tui, KeyCode::Char('z'));

    let screen = screen_of(&mut tui);
    assert!(screen.contains("[2] Results"), "{screen}");
    assert!(!screen.contains("[1] Searches"), "{screen}");
    assert!(!screen.contains("[3] Downloads"), "{screen}");
    assert!(!screen.contains("[Info]"), "{screen}");
    assert!(screen.contains("[z → unzoom]"), "{screen}");

    // Tab while zoomed switches which pane fills the window.
    press(&mut tui, KeyCode::Tab);
    let screen = screen_of(&mut tui);
    assert!(screen.contains("[3] Downloads"), "{screen}");
    assert!(!screen.contains("[2] Results"), "{screen}");

    press(&mut tui, KeyCode::Esc);
    assert!(!tui.state.layout.zoomed);
    let screen = screen_of(&mut tui);
    assert!(screen.contains("[2] Results"), "{screen}");
    assert!(screen.contains("[Info]"), "{screen}");

    press(&mut tui, KeyCode::Char('z'));
    press(&mut tui, KeyCode::Char('z'));
    assert!(!tui.state.layout.zoomed, "z toggles");
}

#[test]
fn a_search_brings_a_hidden_results_pane_back() {
    let mut tui = furnished_tui();
    press(&mut tui, KeyCode::Char('w'));
    assert!(!tui.state.layout.is_visible(FocusedPane::Results));

    press(&mut tui, KeyCode::Char('s'));
    press(&mut tui, KeyCode::Char('x'));
    press(&mut tui, KeyCode::Enter);

    assert!(tui.state.layout.is_visible(FocusedPane::Results));
    assert_eq!(tui.state.focused_pane, FocusedPane::Results);
}

#[test]
fn question_mark_opens_the_keys_list_and_closes_it_again() {
    let mut tui = furnished_tui();
    press(&mut tui, KeyCode::Char('?'));

    let screen = screen_of(&mut tui);
    assert!(screen.contains("Keys"), "{screen}");
    assert!(screen.contains("Tab / Shift-Tab"), "{screen}");
    assert!(screen.contains("clear every finished one"), "{screen}");

    // Other keys are swallowed while it is open.
    press(&mut tui, KeyCode::Char('q'));
    assert!(tui.state.exit.is_none(), "q closes the list, not the app");
    assert!(!tui.state.show_help);

    press(&mut tui, KeyCode::Char('?'));
    press(&mut tui, KeyCode::Char('w'));
    assert!(tui.state.show_help, "w did not reach the panes");
    assert!(tui.state.layout.is_visible(FocusedPane::Results));
    press(&mut tui, KeyCode::Esc);
    assert!(!tui.state.show_help);
}

#[test]
fn on_a_small_terminal_the_keys_list_stacks_and_scrolls() {
    let mut tui = furnished_tui();
    press(&mut tui, KeyCode::Char('?'));

    let screen = screen_sized(&mut tui, 80, 24);
    let panes = row_with(&screen, "Panes");
    assert!(!panes.contains("Searches"), "one column: {panes}");
    assert!(screen.contains("focus a pane, hidden or not"), "{screen}");
    assert!(!screen.contains("next room, chat or user"), "{screen}");

    press(&mut tui, KeyCode::End);
    let screen = screen_sized(&mut tui, 80, 24);
    assert!(screen.contains("next room, chat or user"), "{screen}");
    assert!(!screen.contains("next / previous pane"), "{screen}");

    press(&mut tui, KeyCode::Home);
    let screen = screen_sized(&mut tui, 80, 24);
    assert!(screen.contains("Tab / Shift-Tab"), "{screen}");

    // A wide window shows both columns at once, nothing to scroll. It
    // has to be tall enough to hold the taller column outright; in a
    // shorter one End follows the tail and the headings scroll off.
    press(&mut tui, KeyCode::End);
    let screen = screen_sized(&mut tui, 160, 44);
    let panes = row_with(&screen, "Panes");
    assert!(panes.contains("Searches"), "two columns: {panes}");
    assert!(screen.contains("Tab / Shift-Tab"), "{screen}");
}

#[test]
fn a_click_where_a_pane_just_hid_does_not_focus_it() {
    let mut tui = furnished_tui();
    let _ = screen_of(&mut tui);
    let searches = tui.state.searches_pane_area.expect("laid out");
    // Hidden, but not yet redrawn: the old area is still on record.
    tui.state.focused_pane = FocusedPane::Searches;
    press(&mut tui, KeyCode::Char('w'));
    assert_eq!(tui.state.focused_pane, FocusedPane::Results);
    tui.handle_mouse_event(ratatui::crossterm::event::MouseEvent {
        kind: ratatui::crossterm::event::MouseEventKind::Down(
            ratatui::crossterm::event::MouseButton::Left,
        ),
        column: searches.x + 1,
        row: searches.y + 1,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(tui.state.focused_pane, FocusedPane::Results);
}

#[test]
fn the_shortcut_bar_wraps_instead_of_cutting_keys_off() {
    let mut tui = furnished_tui();
    let screen = screen_of(&mut tui);
    assert!(screen.contains("[q → quit]"), "{screen}");
    let bar_rows = screen.lines().filter(|line| line.contains(" → ")).count();
    assert_eq!(bar_rows, 2, "{screen}");

    let wide = screen_sized(&mut tui, 300, 40);
    let bar_rows = wide.lines().filter(|line| line.contains(" → ")).count();
    assert_eq!(bar_rows, 1, "{wide}");
}

#[test]
fn the_searches_pane_lists_its_clear_all_and_rerun_keys() {
    let mut tui = furnished_tui();
    tui.state.focused_pane = FocusedPane::Searches;
    let screen = screen_sized(&mut tui, 300, 40);
    assert!(screen.contains("[C → clear all]"), "{screen}");
    assert!(screen.contains("[S → search again]"), "{screen}");
    press(&mut tui, KeyCode::Char('?'));
    let screen = screen_of(&mut tui);
    assert!(screen.contains("run the search again"), "{screen}");
}

#[test]
fn the_downloads_pane_lists_its_clear_everything_key() {
    let mut tui = furnished_tui();
    tui.state.focused_pane = FocusedPane::Downloads;
    let screen = screen_sized(&mut tui, 300, 40);
    assert!(screen.contains("[c → clear finished]"), "{screen}");
    assert!(screen.contains("[C → clear all]"), "{screen}");
    press(&mut tui, KeyCode::Char('?'));
    let screen = screen_of(&mut tui);
    assert!(screen.contains("cancelling the live ones"), "{screen}");
}

#[test]
fn shift_w_brings_every_pane_back_and_leaves_zoom() {
    let mut tui = furnished_tui();
    tui.state.focused_pane = FocusedPane::Searches;
    assert!(!screen_sized(&mut tui, 300, 40).contains("[W → "));
    press(&mut tui, KeyCode::Char('w'));
    press(&mut tui, KeyCode::Char('z'));
    assert!(tui.state.layout.zoomed);
    assert!(!tui.state.layout.is_visible(FocusedPane::Searches));
    let screen = screen_sized(&mut tui, 300, 40);
    assert!(screen.contains("[W → reset layout]"), "{screen}");
    press(&mut tui, KeyCode::Char('W'));
    assert!(!tui.state.layout.zoomed);
    assert!(
        FocusedPane::ALL
            .iter()
            .all(|p| tui.state.layout.is_visible(*p))
    );
}

#[test]
fn clicking_where_a_hidden_pane_was_focuses_whatever_is_there_now() {
    let mut tui = furnished_tui();
    let _ = screen_of(&mut tui);
    let searches = tui.state.searches_pane_area.expect("laid out");
    let inside = ratatui::crossterm::event::MouseEvent {
        kind: ratatui::crossterm::event::MouseEventKind::Down(
            ratatui::crossterm::event::MouseButton::Left,
        ),
        column: searches.x + 1,
        row: searches.y + 1,
        modifiers: KeyModifiers::NONE,
    };
    tui.handle_mouse_event(inside);
    assert_eq!(tui.state.focused_pane, FocusedPane::Searches);

    press(&mut tui, KeyCode::Char('w'));
    let _ = screen_of(&mut tui);
    tui.handle_mouse_event(inside);
    assert_eq!(tui.state.focused_pane, FocusedPane::Downloads);
}

#[test]
fn dragging_the_border_under_results_resizes_its_row() {
    let mut tui = furnished_tui();
    let _ = screen_of(&mut tui);
    let results = tui.state.results_pane_area.expect("laid out");
    let divider = tui.state.hsplit_divider.expect("a divider");
    let before = results.height;
    let focused = tui.state.focused_pane;

    // Two drags: the row shrinks by the rows the pointer moved up.
    mouse_drag_row(
        &mut tui,
        (results.x + 1, divider.y),
        (results.x + 1, divider.y - 2),
    );
    assert_eq!(tui.state.layout.top_height, Some(before - 2));
    assert_eq!(
        tui.state.focused_pane, focused,
        "a grab is not a focus click"
    );

    let _ = screen_of(&mut tui);
    assert_eq!(
        tui.state.results_pane_area.expect("laid out").height,
        before - 2
    );

    // A drag far past the row stops at the smallest pane, not zero.
    mouse_drag_row(
        &mut tui,
        (results.x + 1, divider.y - 2),
        (results.x + 1, 0),
    );
    assert!(tui.state.layout.top_height.expect("resized") >= 3);

    press(&mut tui, KeyCode::Char('W'));
    assert_eq!(tui.state.layout.top_height, None, "W resets the layout");
}

#[test]
fn dragging_a_row_panes_right_edge_gives_it_width() {
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};

    let mut tui = furnished_tui();
    let _ = screen_of(&mut tui);
    let searches = tui.state.searches_pane_area.expect("laid out");
    let seam = tui
        .state
        .vsplit_dividers
        .iter()
        .find(|(pane, _)| *pane == FocusedPane::Searches)
        .map(|(_, rect)| *rect)
        .expect("a seam beside Searches");
    tui.state.focused_pane = FocusedPane::Downloads;

    mouse_drag_row(
        &mut tui,
        (seam.x, searches.y + 1),
        (seam.x + 4, searches.y + 1),
    );
    assert_eq!(tui.state.layout.searches_width, Some(searches.width + 4));
    assert_eq!(tui.state.focused_pane, FocusedPane::Downloads);

    // The widths are applied, and Info narrows to whatever is left.
    let _ = screen_of(&mut tui);
    assert_eq!(
        tui.state.searches_pane_area.expect("laid out").width,
        searches.width + 4
    );
    assert!(tui.state.info_pane_area.expect("laid out").width >= 3);

    // Releasing the button ends the drag: later motions just move the
    // pointer.
    mouse(
        &mut tui,
        MouseEventKind::Drag(MouseButton::Left),
        seam.x + 40,
        searches.y + 1,
    );
    assert_eq!(tui.state.layout.searches_width, Some(searches.width + 4));
}

#[test]
fn the_info_pane_describes_the_highlighted_result() {
    let mut tui = results_tui(3);
    tui.state.results_items[1].filename =
        "@@x\\Music\\Album\\Second Track.mp3".to_string();
    tui.state.results_items[1].bitrate = Some(320);
    tui.state.results_table_state.select(Some(1));

    let screen = screen_of(&mut tui);
    assert!(screen.contains("@@x/Music/Album"), "{screen}");
    assert!(screen.contains("Bitrate"), "{screen}");

    tui.state.focused_pane = FocusedPane::Downloads;
    let screen = screen_of(&mut tui);
    assert!(!screen.contains("@@x/Music/Album"), "{screen}");
}
