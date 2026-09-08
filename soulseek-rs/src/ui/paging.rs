//! How a pane's table shows one page of a list that may run to tens of
//! thousands of rows: which rows those are, and how ratatui is handed just
//! those. Paging and drawing both count rows here, so they agree on what a
//! page is.

use crate::ui::pane_block;
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Table, TableState},
};
use std::ops::Range;

/// Rows a list in `area` shows at once, less the `chrome` rows above it: a
/// table header, a popup's tab bar. At least one, so a page key always
/// moves, even before the first draw.
#[must_use]
pub fn page_of(area: Option<Rect>, chrome: u16) -> usize {
    area.map_or(0, |area| {
        usize::from(pane_block(false).inner(area).height.saturating_sub(chrome))
    })
    .max(1)
}

/// The page a table over `area` shows of `len` rows, `chrome` rows of the
/// area being above them: where the last frame left off, moved only as far
/// as keeps the selection in view. A selection past the end lands on the
/// last row, as the table would have put it.
pub fn page_window(
    state: &mut TableState,
    len: usize,
    area: Rect,
    chrome: u16,
) -> Range<usize> {
    if let Some(selected) = state.selected_mut() {
        *selected = (*selected).min(len.saturating_sub(1));
    }
    visible_range(
        state.offset(),
        state.selected(),
        len,
        page_of(Some(area), chrome),
    )
}

/// Draw `table`, holding the rows of the page that starts at `start`, with
/// `state` shifted onto that page. The page start is kept for the next
/// frame.
pub fn render_page(
    frame: &mut Frame,
    table: Table,
    area: Rect,
    state: &mut TableState,
    start: usize,
) {
    let mut page_state = TableState::default()
        .with_selected(state.selected().map(|selected| selected - start));
    frame.render_stateful_widget(table, area, &mut page_state);
    *state.offset_mut() = start;
}

/// The page a `height`-tall table shows, by ratatui's rule (its
/// `visible_rows` is private): the offset, moved only as far as keeps the
/// selection in view. A selection past the end counts as the last row.
fn visible_range(
    offset: usize,
    selected: Option<usize>,
    len: usize,
    height: usize,
) -> Range<usize> {
    let height = height.max(1);
    let last = len.saturating_sub(1);
    let start = match selected.map(|selected| selected.min(last)) {
        Some(selected) => {
            offset.clamp((selected + 1).saturating_sub(height), selected)
        }
        None => offset.min(last),
    };
    start..(start + height).min(len)
}

#[cfg(test)]
mod tests {
    use super::{page_window, visible_range};
    use ratatui::{layout::Rect, widgets::TableState};

    #[test]
    fn the_page_follows_the_selection_and_is_never_longer_than_the_pane() {
        assert_eq!(visible_range(500, Some(505), 10_000, 10), 500..510);
        assert_eq!(visible_range(0, Some(9_999), 10_000, 10), 9_990..10_000);
        assert_eq!(visible_range(500, Some(499), 10_000, 10), 499..509);
        assert_eq!(visible_range(9_995, None, 10_000, 10), 9_995..10_000);
        assert_eq!(visible_range(0, Some(9), 3, 2), 1..3);
    }

    #[test]
    fn a_window_pulls_a_selection_past_the_end_onto_the_last_row() {
        // 6 rows tall: 2 border, 1 header, 3 of the list.
        let area = Rect::new(0, 0, 40, 6);
        let mut state = TableState::default().with_selected(Some(9));
        assert_eq!(page_window(&mut state, 5, area, 1), 2..5);
        assert_eq!(state.selected(), Some(4));
    }
}
