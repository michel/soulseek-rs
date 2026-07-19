use crate::models::{BrowseState, BrowseStatus};
use crate::ui::{
    HIGHLIGHT_SYMBOL, border_style, border_type, error_style, format_bytes,
    get_spinner_char, highlight_style, primary_style,
};
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    text::Line,
    widgets::{
        Block, Borders, Cell, HighlightSpacing, Paragraph, Row, Table,
        TableState,
    },
};

/// Render the browse popup: a collapsible tree of a peer's shared files.
pub fn render_browse_pane(
    frame: &mut Frame,
    area: Rect,
    browse: &BrowseState,
    table_state: &mut TableState,
    spinner_state: usize,
) {
    let title = match browse.status {
        BrowseStatus::Loaded => format!(
            " Browse {} — {} files, {} folders  (Enter/d: download, Esc: close) ",
            browse.username, browse.file_count, browse.folder_count
        ),
        _ => format!(" Browse {} ", browse.username),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(true))
        .border_type(border_type(true))
        .title(title);

    match browse.status {
        BrowseStatus::Loading => {
            let text = format!(
                "{} Requesting shared files from {}…",
                get_spinner_char(spinner_state),
                browse.username
            );
            frame.render_widget(Paragraph::new(text).block(block), area);
        }
        BrowseStatus::Empty => {
            let text = format!("{} is not sharing any files.", browse.username);
            frame.render_widget(Paragraph::new(text).block(block), area);
        }
        BrowseStatus::TimedOut => {
            let text = Line::styled(
                "Timed out waiting for the file list. Press b to retry.",
                error_style(),
            );
            frame.render_widget(Paragraph::new(text).block(block), area);
        }
        BrowseStatus::Loaded => {
            let rows: Vec<Row> = browse
                .rows()
                .iter()
                .map(|row| {
                    let indent = "  ".repeat(row.depth);
                    let (label, size) = if row.is_folder {
                        let glyph = if row.expanded { "▾" } else { "▸" };
                        (
                            Cell::from(format!("{indent}{glyph} {}", row.name))
                                .style(primary_style()),
                            Cell::from(String::new()),
                        )
                    } else {
                        (
                            Cell::from(format!("{indent}  {}", row.name)),
                            Cell::from(
                                row.size.map(format_bytes).unwrap_or_default(),
                            ),
                        )
                    };
                    Row::new(vec![label, size])
                })
                .collect();

            let table =
                Table::new(rows, [Constraint::Fill(1), Constraint::Length(12)])
                    .row_highlight_style(highlight_style())
                    .highlight_symbol(HIGHLIGHT_SYMBOL)
                    .highlight_spacing(HighlightSpacing::Always)
                    .block(block);

            table_state.select(Some(browse.selected_row));
            frame.render_stateful_widget(table, area, table_state);
        }
    }
}
