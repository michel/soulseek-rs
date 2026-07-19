use crate::models::{RoomsState, RoomsView};
use crate::ui::{
    HIGHLIGHT_SYMBOL, accent_style, border_style, border_type, dimmed_style,
    highlight_style, info_style, primary_style,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, HighlightSpacing, Paragraph, Row, Table,
        TableState, Wrap,
    },
};

/// Render the chat-rooms popup: either the browsable room list or the open
/// rooms' tabbed chat view.
pub fn render_rooms_pane(
    frame: &mut Frame,
    area: Rect,
    rooms: &RoomsState,
    list_table_state: &mut TableState,
) {
    match rooms.view {
        RoomsView::List => render_list(frame, area, rooms, list_table_state),
        RoomsView::Chat => render_chat(frame, area, rooms),
    }
}

fn render_list(
    frame: &mut Frame,
    area: Rect,
    rooms: &RoomsState,
    table_state: &mut TableState,
) {
    let title = if rooms.list_is_filtering || !rooms.list_filter.is_empty() {
        format!(
            " Rooms · filter: {}_  (Enter: join, Esc: clear) ",
            rooms.list_filter
        )
    } else {
        " Rooms  (Enter: join, /: filter, Tab: open rooms, Esc: close) "
            .to_string()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(true))
        .border_type(border_type(true))
        .title(title);

    let filtered = rooms.filtered_rooms();
    if filtered.is_empty() {
        let text = if rooms.available.is_empty() {
            "Fetching room list…"
        } else {
            "No rooms match the filter."
        };
        frame.render_widget(Paragraph::new(text).block(block), area);
        return;
    }

    let header = Row::new(vec![
        Cell::from("room").style(dimmed_style()),
        Cell::from("users").style(dimmed_style()),
    ]);
    let table_rows: Vec<Row> = filtered
        .iter()
        .map(|r| {
            let open = rooms.open_index(&r.name).is_some();
            let name = if open {
                format!("● {}", r.name)
            } else {
                format!("  {}", r.name)
            };
            let name_style = if open {
                accent_style()
            } else {
                primary_style()
            };
            Row::new(vec![
                Cell::from(name).style(name_style),
                Cell::from(r.user_count.to_string()),
            ])
        })
        .collect();

    let table =
        Table::new(table_rows, [Constraint::Fill(1), Constraint::Length(8)])
            .header(header)
            .row_highlight_style(highlight_style())
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always)
            .block(block);

    table_state.select(Some(rooms.list_selected.min(filtered.len() - 1)));
    frame.render_stateful_widget(table, area, table_state);
}

fn render_chat(frame: &mut Frame, area: Rect, rooms: &RoomsState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(true))
        .border_type(border_type(true))
        .title(
            " Chat rooms  (Tab: switch, l: room list, x: leave, Esc: back) ",
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1), // tab bar
        Constraint::Fill(1),   // body
        Constraint::Length(1), // compose / hint
    ])
    .split(inner);

    render_tab_bar(frame, chunks[0], rooms);

    let Some(active) = rooms.active_room() else {
        frame.render_widget(
            Paragraph::new("No open rooms. Press l for the room list.")
                .style(dimmed_style()),
            chunks[1],
        );
        return;
    };

    // Body: messages (left) + user list (right).
    let body =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(22)])
            .split(chunks[1]);

    render_messages(frame, body[0], active.lines.as_slice());
    render_users(frame, body[1], &active.users);

    // Compose line or hint.
    if rooms.composing {
        let line = Line::from(vec![
            Span::styled("› ", accent_style()),
            Span::styled(active.input.clone(), primary_style()),
            Span::styled("▏", accent_style()),
        ]);
        frame.render_widget(Paragraph::new(line), chunks[2]);
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Enter: type a message",
                dimmed_style(),
            ))),
            chunks[2],
        );
    }
}

fn render_tab_bar(frame: &mut Frame, area: Rect, rooms: &RoomsState) {
    let mut spans: Vec<Span> = Vec::new();
    for (i, room) in rooms.open.iter().enumerate() {
        let active = i == rooms.active;
        let label = if room.unread > 0 {
            format!(" {} ({}) ", room.name, room.unread)
        } else {
            format!(" {} ", room.name)
        };
        let mut style = if active {
            highlight_style()
        } else {
            primary_style()
        };
        if room.unread > 0 {
            style = style.add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_messages(
    frame: &mut Frame,
    area: Rect,
    lines: &[crate::models::RoomLine],
) {
    // Auto-scroll: keep the most recent lines that fit the height.
    let height = area.height as usize;
    let start = lines.len().saturating_sub(height.max(1));
    let rendered: Vec<Line> = lines[start..]
        .iter()
        .map(|l| match &l.username {
            Some(user) => Line::from(vec![
                Span::styled(format!("<{user}> "), info_style()),
                Span::styled(l.text.clone(), primary_style()),
            ]),
            None => Line::from(Span::styled(l.text.clone(), dimmed_style())),
        })
        .collect();
    frame.render_widget(
        Paragraph::new(rendered).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_users(frame: &mut Frame, area: Rect, users: &[String]) {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(dimmed_style())
        .title(format!(" Users ({}) ", users.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let height = inner.height as usize;
    let shown: Vec<Line> = users
        .iter()
        .take(height)
        .map(|u| Line::from(Span::styled(u.clone(), primary_style())))
        .collect();
    frame.render_widget(Paragraph::new(shown), inner);
}
