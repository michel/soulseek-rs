use crate::models::{SearchEntry, SearchStatus};
use crate::ui::{
    HIGHLIGHT_SYMBOL, body_style, dimmed_style, header_style, highlight_style,
    pane_block, pane_title, success_style, warning_style,
};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Cell, HighlightSpacing, Row, Table, TableState},
};

pub fn render_searches_pane(
    frame: &mut Frame,
    area: Rect,
    searches: &[SearchEntry],
    table_state: &mut TableState,
    focused: bool,
) {
    let header = Row::new(vec![
        Cell::from("Status").style(header_style()),
        Cell::from("Query").style(header_style()),
        Cell::from("Results").style(header_style()),
    ])
    .height(1);

    let rows: Vec<Row> = searches
        .iter()
        .map(|search| {
            let status_cell = match &search.status {
                SearchStatus::Active => {
                    Cell::from("Active").style(warning_style())
                }
                SearchStatus::Completed => {
                    Cell::from("Done").style(success_style())
                }
            };

            let results_count = search.results.len();
            let results_text = if results_count == 0
                && search.status == SearchStatus::Active
            {
                "Searching...".to_string()
            } else {
                format!("{results_count}")
            };

            Row::new(vec![
                status_cell,
                Cell::from(search.query.clone()).style(body_style()),
                Cell::from(results_text).style(dimmed_style()),
            ])
        })
        .collect();

    let widths = [
        ratatui::layout::Constraint::Length(15),
        ratatui::layout::Constraint::Fill(1),
        ratatui::layout::Constraint::Length(10),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(highlight_style())
        .highlight_symbol(HIGHLIGHT_SYMBOL)
        .highlight_spacing(HighlightSpacing::Always)
        .block(pane_block(focused).title(pane_title("1", "Searches", focused)));

    frame.render_stateful_widget(table, area, table_state);
}
