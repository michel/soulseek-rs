use crate::models::FileDisplayData;
use crate::ui::{
    BYTES_PER_MB, HIGHLIGHT_SYMBOL, body_style, dimmed_style, format_bytes,
    header_style, highlight_style, info_style, pane_block, pane_title,
    success_style, warning_style,
};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Cell, HighlightSpacing, Paragraph, Row, Table, TableState},
};
use std::collections::HashSet;

const VERSION: &str = env!("CARGO_PKG_VERSION");

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
                Cell::from(file.filename.clone()).style(body_style()),
                Cell::from(format_bytes(file.size)).style(warning_style()),
                Cell::from(file.username.clone()).style(info_style()),
                Cell::from(bitrate_str).style(dimmed_style()),
                Cell::from(speed_str).style(dimmed_style()),
                Cell::from(file.slots.to_string()).style(dimmed_style()),
            ])
        })
        .collect();

    let widths = [
        ratatui::layout::Constraint::Length(3),
        ratatui::layout::Constraint::Fill(3),
        ratatui::layout::Constraint::Length(12),
        ratatui::layout::Constraint::Length(15),
        ratatui::layout::Constraint::Length(10),
        ratatui::layout::Constraint::Length(12),
        ratatui::layout::Constraint::Length(6),
    ];

    let title = if is_filtering {
        format!("Results · filter: '{filter_query}'")
    } else if let Some(query) = active_search_query {
        format!("Results: {query}")
    } else {
        "Results".to_string()
    };

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(highlight_style())
        .highlight_symbol(HIGHLIGHT_SYMBOL)
        .highlight_spacing(HighlightSpacing::Always)
        .block(pane_block(focused).title(pane_title("2", &title, focused)));

    frame.render_stateful_widget(table, area, table_state);
}

#[cfg(test)]
mod tests {
    use super::row_is_selected;
    use std::collections::HashSet;

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
