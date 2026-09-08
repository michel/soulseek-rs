use super::name_scroll::{column_width, end_offset, scroll_text};
use crate::models::FileDisplayData;
use crate::ui::{
    BYTES_PER_MB, HIGHLIGHT_SYMBOL, body_style, dimmed_style, format_bytes,
    header_style, info_style, page_of, pane_block, pane_title,
    row_highlight_style, success_style, visible_range, warning_style,
};
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    text::{Line, Span},
    widgets::{Cell, HighlightSpacing, Paragraph, Row, Table, TableState},
};
use std::collections::HashSet;

const VERSION: &str = env!("CARGO_PKG_VERSION");

// The file name and its folder share whatever width the fixed columns leave,
// the name getting the larger part: it is what a search is after, and a
// folder's tail is a scroll away.
const WIDTHS: [Constraint; 8] = [
    Constraint::Length(3),
    Constraint::Fill(3),
    Constraint::Fill(2),
    Constraint::Length(12),
    Constraint::Length(15),
    Constraint::Length(10),
    Constraint::Length(12),
    Constraint::Length(6),
];

pub struct ResultsPaneParams<'a> {
    pub items: &'a [FileDisplayData],
    pub table_state: &'a mut TableState,
    pub selected_indices: &'a HashSet<usize>,
    /// Maps a rendered row index to its index in the unfiltered results list.
    /// `None` means the rendered rows are the unfiltered list (identity map).
    pub original_indices: Option<&'a [usize]>,
    pub filter_query: &'a str,
    pub is_filtering: bool,
    pub focused: bool,
    pub active_search_query: Option<&'a str>,
    pub name_offset: usize,
}

/// The file name is the second column, the folder it sits in the third.
const NAME_COLUMN: usize = 1;
const FOLDER_COLUMN: usize = 2;

/// A shared path as `(folder, name)`. Peers send backslashes, but a slash
/// counts too.
fn split_path(path: &str) -> (&str, &str) {
    path.rsplit_once(['\\', '/']).unwrap_or(("", path))
}

/// The offset that brings the end of a result's longer column into view.
/// One offset scrolls the name and the folder together, so it runs until
/// both tails are showing.
#[must_use]
pub fn name_end_offset(path: &str, area: Rect) -> usize {
    let (folder, name) = split_path(path);
    end_offset(name, area, &WIDTHS, NAME_COLUMN).max(end_offset(
        folder,
        area,
        &WIDTHS,
        FOLDER_COLUMN,
    ))
}

/// Whether the rendered row `display_idx` is selected. `selected_indices` holds
/// indices into the *unfiltered* results, so under an active filter the display
/// index must be translated through `original_indices` first.
fn row_is_selected(
    display_idx: usize,
    original_indices: Option<&[usize]>,
    selected_indices: &HashSet<usize>,
) -> bool {
    let original = match original_indices {
        Some(map) => match map.get(display_idx) {
            Some(&o) => o,
            None => return false,
        },
        None => display_idx,
    };
    selected_indices.contains(&original)
}

pub fn render_results_pane(
    frame: &mut Frame,
    area: Rect,
    params: ResultsPaneParams,
) {
    let ResultsPaneParams {
        items,
        table_state,
        selected_indices,
        original_indices,
        filter_query,
        is_filtering,
        focused,
        active_search_query,
        name_offset,
    } = params;
    if items.is_empty() {
        let title = match active_search_query {
            Some(query) => format!("Results: {query}"),
            None => "Results".to_string(),
        };

        let empty_block =
            pane_block(focused).title(pane_title("2", &title, focused));

        let message = if is_filtering {
            vec![Line::from(Span::styled(
                format!("No results match filter: '{filter_query}'"),
                dimmed_style(),
            ))]
        } else {
            vec![
                Line::from(vec![
                    Span::styled("soulseek-rs", body_style()),
                    Span::styled(format!(" v{VERSION}"), dimmed_style()),
                ]),
                Line::from(""),
                Line::from(Span::styled("No results yet.", body_style())),
                Line::from(Span::styled(
                    "Pick a search in [1] Searches, or start a new one.",
                    dimmed_style(),
                )),
            ]
        };

        let paragraph = Paragraph::new(message).block(empty_block);
        frame.render_widget(paragraph, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from("✓").style(header_style()),
        Cell::from("Filename").style(header_style()),
        Cell::from("Folder").style(header_style()),
        Cell::from("Size").style(header_style()),
        Cell::from("User").style(header_style()),
        Cell::from("Bitrate").style(header_style()),
        Cell::from("Speed").style(header_style()),
        Cell::from("Slots").style(header_style()),
    ])
    .height(1);

    let name_width = column_width(area, &WIDTHS, NAME_COLUMN);
    let folder_width = column_width(area, &WIDTHS, FOLDER_COLUMN);
    // A selection past the end, after a filter shrank the list, lands on the
    // last row, as the table would have put it.
    if let Some(selected) = table_state.selected_mut() {
        *selected = (*selected).min(items.len() - 1);
    }
    let window = visible_range(
        table_state.offset(),
        table_state.selected(),
        items.len(),
        page_of(Some(area), 1),
    );
    let start = window.start;
    let rows: Vec<Row> = (start..)
        .zip(&items[window])
        .map(|(idx, file)| {
            let (folder, name) = split_path(&file.filename);
            let checkbox =
                if row_is_selected(idx, original_indices, selected_indices) {
                    "[✓]"
                } else {
                    "[ ]"
                };

            let bitrate_str = file
                .bitrate
                .map_or_else(|| "-".to_string(), |br| format!("{br} kbps"));

            let speed_str = if file.speed > 0 {
                let speed_mb = (f64::from(file.speed) / BYTES_PER_MB * 100.0)
                    .round()
                    / 100.0;
                format!("{speed_mb} MB/s")
            } else {
                "-".to_string()
            };

            let checkbox_style = if checkbox == "[✓]" {
                success_style()
            } else {
                dimmed_style()
            };

            Row::new(vec![
                Cell::from(checkbox).style(checkbox_style),
                Cell::from(scroll_text(name, name_offset, name_width))
                    .style(body_style()),
                Cell::from(scroll_text(folder, name_offset, folder_width))
                    .style(dimmed_style()),
                Cell::from(format_bytes(file.size)).style(warning_style()),
                Cell::from(file.username.clone()).style(info_style()),
                Cell::from(bitrate_str).style(dimmed_style()),
                Cell::from(speed_str).style(dimmed_style()),
                Cell::from(file.slots.to_string()).style(dimmed_style()),
            ])
        })
        .collect();

    let title = if is_filtering {
        format!("Results · filter: '{filter_query}'")
    } else if let Some(query) = active_search_query {
        format!("Results: {query}")
    } else {
        "Results".to_string()
    };

    let table = Table::new(rows, WIDTHS)
        .header(header)
        .row_highlight_style(row_highlight_style())
        .highlight_symbol(HIGHLIGHT_SYMBOL)
        .highlight_spacing(HighlightSpacing::Always)
        .block(pane_block(focused).title(pane_title("2", &title, focused)));

    // The table holds one page, so the state it gets is shifted onto it.
    let mut page_state = TableState::default()
        .with_selected(table_state.selected().map(|selected| selected - start));
    frame.render_stateful_widget(table, area, &mut page_state);
    *table_state.offset_mut() = start;
}

#[cfg(test)]
mod tests {
    use super::{ResultsPaneParams, render_results_pane, row_is_selected};
    use crate::models::FileDisplayData;
    use ratatui::{Terminal, backend::TestBackend, widgets::TableState};
    use std::collections::HashSet;

    fn render_rows(items: &[FileDisplayData], name_offset: usize) -> String {
        let mut state = TableState::default();
        state.select(Some(0));
        render_with(items, &mut state, name_offset)
    }

    fn render_with(
        items: &[FileDisplayData],
        state: &mut TableState,
        name_offset: usize,
    ) -> String {
        // Wide enough for the name column to hold 11 cells, with 7 for the
        // folder beside it, and tall enough for three rows.
        let mut terminal =
            Terminal::new(TestBackend::new(88, 6)).expect("backend");
        terminal
            .draw(|frame| {
                render_results_pane(
                    frame,
                    frame.area(),
                    ResultsPaneParams {
                        items,
                        table_state: state,
                        selected_indices: &HashSet::new(),
                        original_indices: None,
                        filter_query: "",
                        is_filtering: false,
                        focused: true,
                        active_search_query: None,
                        name_offset,
                    },
                );
            })
            .expect("draw");
        terminal.backend().to_string()
    }

    fn file(filename: &str) -> FileDisplayData {
        FileDisplayData {
            filename: filename.to_string(),
            username: "bob".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn a_scrolled_name_shows_its_tail_up_to_the_column_edge() {
        let items = [
            file("abcdefghijklmnopqrstuvwxyz0123456789ABCD"),
            file("short.mp3"),
        ];
        let screen = render_rows(&items, usize::MAX);
        assert!(screen.contains("…456789ABCD"), "{screen}");
        assert!(!screen.contains("3456789ABCD"), "{screen}");
        assert!(screen.contains(" short.mp3 "), "{screen}");

        let screen = render_rows(&items, 8);
        assert!(screen.contains("…ijklmnopqr"), "{screen}");
        assert!(!screen.contains("hijklmnopqr"), "{screen}");
    }

    #[test]
    fn a_wide_glyph_name_scrolls_by_cells_not_chars() {
        let items = [file("零一二三四五六七八九十百千万亿東西南北中")];
        let screen = render_rows(&items, usize::MAX);
        assert!(screen.contains("…東西南北中"), "{screen}");
        assert!(!screen.contains("亿東"), "{screen}");
    }

    #[test]
    fn a_scrolled_name_never_starts_with_a_combining_mark() {
        let name = format!("{}e\u{301}{}", "x".repeat(5), "y".repeat(20));
        let items = [file(&name)];
        let screen = render_rows(&items, 6);
        assert!(screen.contains("…yyyyyyyyyy"), "{screen}");
        assert!(!screen.contains("…\u{301}"), "{screen}");
    }

    #[test]
    fn a_path_shows_its_name_before_its_folder() {
        let items = [file("@@abc\\Music\\Album\\01.flac"), file("cover.jpg")];
        let screen = render_rows(&items, 0);
        assert!(screen.contains("01.flac     @@abc\\M"), "{screen}");
        assert!(screen.contains("cover.jpg   "), "{screen}");
    }

    #[test]
    fn one_offset_scrolls_the_name_and_the_folder_together() {
        let items = [file("@@abc\\Music\\Album\\abcdefghijklmnopqrstuvwxyz")];
        let screen = render_rows(&items, 4);
        assert!(screen.contains("…efghijklmn …c\\Musi"), "{screen}");
    }

    #[test]
    fn the_end_offset_is_the_longer_columns() {
        // 88 cells wide: 11 for the name, 7 for the folder.
        let area = ratatui::layout::Rect::new(0, 0, 88, 6);
        let folder = "f".repeat(20);
        assert_eq!(super::name_end_offset(&format!("{folder}\\a"), area), 14);
        let name = "x".repeat(40);
        assert_eq!(
            super::name_end_offset(&format!("{folder}\\{name}"), area),
            30
        );
    }

    #[test]
    fn an_unscrolled_name_starts_at_its_beginning() {
        let items = [file("abcdefghijklmnopqrstuvwxyz0123456789ABCD")];
        let screen = render_rows(&items, 0);
        assert!(screen.contains("abcdefghijk"), "{screen}");
        assert!(!screen.contains("…"), "{screen}");
    }

    #[test]
    fn a_far_selection_is_drawn_from_its_own_page() {
        let items: Vec<FileDisplayData> =
            (0..10_000).map(|i| file(&format!("{i}.mp3"))).collect();
        let mut state = TableState::default();
        state.select(Some(9_999));
        let screen = render_with(&items, &mut state, 0);
        assert!(screen.contains("›[ ] 9999.mp3"), "{screen}");
        assert!(screen.contains(" [ ] 9997.mp3"), "{screen}");
        assert!(!screen.contains(" [ ] 0.mp3"), "{screen}");
        assert_eq!(state.offset(), 9_997, "the next frame starts here");
        assert_eq!(state.selected(), Some(9_999));
    }

    #[test]
    fn a_selection_off_the_end_lands_on_the_last_row() {
        let items = [file("a.mp3"), file("b.mp3")];
        let mut state = TableState::default();
        state.select(Some(7));
        let screen = render_with(&items, &mut state, 0);
        assert!(screen.contains("›[ ] b.mp3"), "{screen}");
        assert_eq!(state.selected(), Some(1));
    }

    #[test]
    fn identity_mapping_used_when_no_filter() {
        let selected: HashSet<usize> = std::iter::once(1).collect();
        assert!(row_is_selected(1, None, &selected));
        assert!(!row_is_selected(0, None, &selected));
    }

    #[test]
    fn filtered_rows_resolve_through_original_indices() {
        // Filter shows original rows 2, 5, 7 as display rows 0, 1, 2.
        // The user selected original row 5.
        let original_indices = [2usize, 5, 7];
        let selected: HashSet<usize> = std::iter::once(5).collect();
        assert!(!row_is_selected(0, Some(&original_indices), &selected));
        assert!(row_is_selected(1, Some(&original_indices), &selected));
        assert!(!row_is_selected(2, Some(&original_indices), &selected));
    }

    #[test]
    fn out_of_range_display_index_is_not_selected() {
        let original_indices = [2usize, 5];
        let selected: HashSet<usize> = [2, 5].into_iter().collect();
        assert!(!row_is_selected(9, Some(&original_indices), &selected));
    }
}
