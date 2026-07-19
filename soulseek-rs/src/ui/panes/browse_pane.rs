use crate::models::{BrowseState, BrowseStatus, BrowseTabs};
use crate::ui::{
    HIGHLIGHT_SYMBOL, border_style, border_type, dimmed_style, error_style,
    format_bytes, get_spinner_char, highlight_style, primary_style,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, HighlightSpacing, Paragraph, Row, Table,
        TableState,
    },
};

/// Render the browse popup: a tab bar of browsed users (when more than one is
/// open) above the active user's collapsible shared-file tree.
pub fn render_browse_pane(
    frame: &mut Frame,
    area: Rect,
    tabs: &BrowseTabs,
    table_state: &mut TableState,
    spinner_state: usize,
) {
    let Some(active) = tabs.active_tab() else {
        return;
    };

    // With multiple users open, reserve the top row for a tab bar.
    let tree_area = if tabs.tabs.len() > 1 {
        let chunks =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)])
                .split(area);
        render_browse_tabs(frame, chunks[0], tabs);
        chunks[1]
    } else {
        area
    };

    render_browse_one(frame, tree_area, active, table_state, spinner_state);
}

fn render_browse_tabs(frame: &mut Frame, area: Rect, tabs: &BrowseTabs) {
    let mut spans: Vec<Span> = Vec::new();
    for (i, tab) in tabs.tabs.iter().enumerate() {
        let style = if i == tabs.active {
            highlight_style().add_modifier(Modifier::BOLD)
        } else {
            dimmed_style()
        };
        spans.push(Span::styled(format!(" {} ", tab.username), style));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Render one browsed user's shared-file tree.
fn render_browse_one(
    frame: &mut Frame,
    area: Rect,
    browse: &BrowseState,
    table_state: &mut TableState,
    spinner_state: usize,
) {
    let title = match browse.status {
        BrowseStatus::Loaded => format!(
            " Browse {} — {} files, {} folders  (Enter/d: download, Tab: user, w: close, Esc: hide) ",
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
            let text = vec![
                Line::styled(
                    format!("Couldn't reach {}.", browse.username),
                    error_style(),
                ),
                Line::raw(""),
                Line::raw(
                    "They may be offline, or their connection can't be \
                     reached (both of you may be behind a router/firewall).",
                ),
                Line::raw("Press r to try again."),
            ];
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
