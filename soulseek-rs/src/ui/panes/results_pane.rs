use crate::models::FileDisplayData;
use crate::ui::{
    border_style, border_type, format_bytes, header_style, highlight_style,
    HIGHLIGHT_SYMBOL,
};
use ratatui::{
    layout::Rect,
    widgets::{
        Block, Borders, Cell, HighlightSpacing, Paragraph, Row, Table,
        TableState,
    },
    Frame,
};
use std::collections::HashSet;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct ResultsPaneParams<'a> {
    pub items: &'a [FileDisplayData],
    pub table_state: &'a mut TableState,
    pub selected_indices: &'a HashSet<usize>,
    pub filter_query: &'a str,
    pub is_filtering: bool,
    pub focused: bool,
    pub active_search_query: Option<&'a str>,
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
        filter_query,
        is_filtering,
        focused,
        active_search_query,
    } = params;
    if items.is_empty() {
        let title = if let Some(query) = active_search_query {
            format!("[2] Results: {}", query)
        } else {
            "[2] Results".to_string()
        };

        let empty_block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style(focused))
            .border_type(border_type(focused))
            .title(title);

        let message = if is_filtering {
            format!("No results match filter: '{}'", filter_query)
        } else {
            format!(
                "soulseek-rs 🦀 v{}
Michel de Graaf 2025\n
No results. Select a search from the Searches pane [1]. Or start new search [s → search] \n",
                VERSION
            )
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
            let checkbox = if selected_indices.contains(&idx) {
                "[✓]"
            } else {
                "[ ]"
            };

            let bitrate_str = file
                .bitrate
                .map(|br| format!("{} kbps", br))
                .unwrap_or_else(|| "-".to_string());

            let speed_str = if file.speed > 0 {
                let speed_mb =
                    (file.speed as f64 / 1_048_576.0 * 100.0).round() / 100.0;
                format!("{} MB/s", speed_mb)
            } else {
                "-".to_string()
            };

            Row::new(vec![
                Cell::from(checkbox),
                Cell::from(file.filename.clone()),
                Cell::from(format_bytes(file.size)),
                Cell::from(file.username.clone()),
                Cell::from(bitrate_str),
                Cell::from(speed_str),
                Cell::from(file.slots.to_string()),
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
        format!("[2] Results - Filter: '{}'", filter_query)
    } else if let Some(query) = active_search_query {
        format!("[2] Results: {}", query)
    } else {
        "[2] Results".to_string()
    };

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(highlight_style())
        .highlight_symbol(HIGHLIGHT_SYMBOL)
        .highlight_spacing(HighlightSpacing::Always)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style(focused))
                .border_type(border_type(focused))
                .title(title),
        );

    frame.render_stateful_widget(table, area, table_state);
}
