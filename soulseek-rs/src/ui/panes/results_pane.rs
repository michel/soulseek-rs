use crate::models::FileDisplayData;
use crate::ui::{
    BYTES_PER_MB, HIGHLIGHT_SYMBOL, body_style, dimmed_style, format_bytes,
    header_style, info_style, pane_block, pane_title, row_highlight_style,
    success_style, warning_style,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Cell, HighlightSpacing, Paragraph, Row, Table, TableState},
};
use std::collections::HashSet;
use unicode_width::UnicodeWidthChar;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const WIDTHS: [Constraint; 7] = [
    Constraint::Length(3),
    Constraint::Fill(3),
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

fn name_column_width(area: Rect) -> usize {
    let inner = pane_block(false).inner(area);
    let symbol = Line::from(HIGHLIGHT_SYMBOL).width() as u16;
    let [_, columns] =
        Layout::horizontal([Constraint::Length(symbol), Constraint::Fill(0)])
            .areas(inner);
    usize::from(Layout::horizontal(WIDTHS).spacing(1).split(columns)[1].width)
}

const fn overflow(cells: usize, width: usize) -> usize {
    if cells <= width { 0 } else { cells + 1 - width }
}

#[must_use]
pub fn name_end_offset(name: &str, area: Rect) -> usize {
    overflow(Span::raw(name).width(), name_column_width(area))
}

fn scroll_name(name: &str, offset: usize, width: usize) -> Line<'_> {
    if offset == 0 {
        return Line::from(name);
    }
    let mut tail_cells = 0;
    let mut furthest = name.len();
    for (index, ch) in name.char_indices().rev() {
        let cells = ch.width().unwrap_or(0);
        if tail_cells + cells >= width {
            if index == 0 && tail_cells + cells == width {
                return Line::from(name);
            }
            break;
        }
        tail_cells += cells;
        furthest = index;
    }
    if furthest == 0 {
        return Line::from(name);
    }
    let mut skipped = 0;
    let mut start = furthest;
    for (index, ch) in name.char_indices() {
        if index >= furthest || skipped >= offset {
            start = index;
            break;
        }
        skipped += ch.width().unwrap_or(0);
    }
    while let Some(ch) = name[start..].chars().next()
        && ch.width() == Some(0)
    {
        start += ch.len_utf8();
    }
    Line::from(vec![Span::raw("…"), Span::raw(&name[start..])])
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
        Cell::from("Size").style(header_style()),
        Cell::from("User").style(header_style()),
        Cell::from("Bitrate").style(header_style()),
        Cell::from("Speed").style(header_style()),
        Cell::from("Slots").style(header_style()),
    ])
    .height(1);

    let name_width = name_column_width(area);
    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(idx, file)| {
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
                Cell::from(scroll_name(
                    &file.filename,
                    name_offset,
                    name_width,
                ))
                .style(body_style()),
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

    frame.render_stateful_widget(table, area, table_state);
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
        let mut terminal =
            Terminal::new(TestBackend::new(80, 6)).expect("backend");
        terminal
            .draw(|frame| {
                render_results_pane(
                    frame,
                    frame.area(),
                    ResultsPaneParams {
                        items,
                        table_state: &mut state,
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
    fn an_unscrolled_name_starts_at_its_beginning() {
        let items = [file("abcdefghijklmnopqrstuvwxyz0123456789ABCD")];
        let screen = render_rows(&items, 0);
        assert!(screen.contains("abcdefghijk"), "{screen}");
        assert!(!screen.contains("…"), "{screen}");
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
