//! Sideways scrolling for a table column that holds text wider than it is:
//! a file name, a query. Every list scrolls the same way, so the arithmetic
//! lives here and each pane only says which of its columns it applies to.

use crate::ui::{HIGHLIGHT_SYMBOL, pane_block};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

/// Cells column `column` of a table laid out with `widths` gets inside a pane
/// drawn over `area`: what the pane keeps for its border, padding and the
/// highlight symbol comes off first.
#[must_use]
pub fn column_width(area: Rect, widths: &[Constraint], column: usize) -> usize {
    let inner = pane_block(false).inner(area);
    let symbol = Line::from(HIGHLIGHT_SYMBOL).width() as u16;
    let [_, columns] =
        Layout::horizontal([Constraint::Length(symbol), Constraint::Fill(0)])
            .areas(inner);
    usize::from(
        Layout::horizontal(widths).spacing(1).split(columns)[column].width,
    )
}

/// How far a text of `cells` can scroll before its tail is in view, keeping
/// one cell for the ellipsis that marks the cut.
const fn overflow(cells: usize, width: usize) -> usize {
    if cells <= width { 0 } else { cells + 1 - width }
}

/// The offset that brings the end of `text` into view in the column.
#[must_use]
pub fn end_offset(
    text: &str,
    area: Rect,
    widths: &[Constraint],
    column: usize,
) -> usize {
    overflow(Span::raw(text).width(), column_width(area, widths, column))
}

/// `text` scrolled `offset` cells to the left, with an ellipsis where it was
/// cut. Scrolls by cells, so a wide glyph counts double, and never starts on
/// a combining mark. Beyond the end it shows the tail.
#[must_use]
pub fn scroll_text(text: &str, offset: usize, width: usize) -> Line<'_> {
    if offset == 0 {
        return Line::from(text);
    }
    let mut tail_cells = 0;
    let mut furthest = text.len();
    for (index, ch) in text.char_indices().rev() {
        let cells = ch.width().unwrap_or(0);
        if tail_cells + cells >= width {
            if index == 0 && tail_cells + cells == width {
                return Line::from(text);
            }
            break;
        }
        tail_cells += cells;
        furthest = index;
    }
    if furthest == 0 {
        return Line::from(text);
    }
    let mut skipped = 0;
    let mut start = furthest;
    for (index, ch) in text.char_indices() {
        if index >= furthest || skipped >= offset {
            start = index;
            break;
        }
        skipped += ch.width().unwrap_or(0);
    }
    while let Some(ch) = text[start..].chars().next()
        && ch.width() == Some(0)
    {
        start += ch.len_utf8();
    }
    Line::from(vec![Span::raw("…"), Span::raw(&text[start..])])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn a_text_that_fits_never_scrolls() {
        assert_eq!(text_of(&scroll_text("short", 8, 10)), "short");
        assert_eq!(
            end_offset(
                "short",
                Rect::new(0, 0, 40, 5),
                &[Constraint::Fill(1)],
                0
            ),
            0
        );
    }

    #[test]
    fn the_end_offset_leaves_room_for_the_ellipsis() {
        // Column of 40 - 2 border - 2 padding - 1 symbol = 35 cells.
        let area = Rect::new(0, 0, 40, 5);
        let widths = [Constraint::Fill(1)];
        let fits = "x".repeat(35);
        let over = "x".repeat(36);
        assert_eq!(end_offset(&fits, area, &widths, 0), 0);
        assert_eq!(end_offset(&over, area, &widths, 0), 2);
        let line = scroll_text(&over, 2, 35);
        assert_eq!(text_of(&line).chars().count(), 35);
        assert!(text_of(&line).starts_with('…'));
    }

    #[test]
    fn scrolling_past_the_end_shows_the_tail() {
        let line = scroll_text("abcdefghij", usize::MAX, 5);
        assert_eq!(text_of(&line), "…ghij");
    }
}
