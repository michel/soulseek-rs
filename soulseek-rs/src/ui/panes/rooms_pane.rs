use crate::models::{
    ChatFilter, LogView, OpenRoom, RoomLine, RoomsState, RoomsView, WrappedLog,
    contains_filter, matching_users,
};
use crate::ui::{
    HIGHLIGHT_SYMBOL, PANE_PADDING, accent_style, dimmed_style,
    highlight_style, info_style, page_of, pane_block, plain_title,
    primary_style, row_highlight_style, visible_range, wrap_chat_line,
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

/// Render the chat-rooms popup: either the browsable room list or the open
/// rooms' tabbed chat view.
pub fn render_rooms_pane(
    frame: &mut Frame,
    area: Rect,
    rooms: &mut RoomsState,
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
    let block = pane_block(true).title(title);

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
    let window = visible_range(
        table_state.offset(),
        Some(rooms.list_selected),
        filtered.len(),
        page_of(Some(area), 1),
    );
    let start = window.start;
    let table_rows: Vec<Row> = filtered[window]
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
            .row_highlight_style(row_highlight_style())
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always)
            .block(block);

    // The table holds one page, so the state it gets is shifted onto it.
    let mut page_state = TableState::default().with_selected(Some(
        rooms.list_selected.min(filtered.len() - 1) - start,
    ));
    frame.render_stateful_widget(table, area, &mut page_state);
    *table_state.offset_mut() = start;
}

fn render_chat(frame: &mut Frame, area: Rect, rooms: &mut RoomsState) {
    let typing_log = rooms.filtering == Some(ChatFilter::Log);
    let title = if typing_log || !rooms.log_filter.is_empty() {
        format!(
            " Chat rooms · filter: {}{}  (Enter: keep, Esc: clear) ",
            rooms.log_filter,
            if typing_log { "_" } else { "" }
        )
    } else {
        " Chat rooms  (Tab: switch, /: find, u: find user, l: room list, x: leave, Esc: back) "
            .to_string()
    };
    let block = pane_block(true).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1), // tab bar
        Constraint::Fill(1),   // body
        Constraint::Length(1), // compose / hint
    ])
    .split(inner);

    render_tab_bar(frame, chunks[0], rooms);

    let RoomsState {
        open,
        active: active_index,
        user_selected,
        composing,
        log_filter,
        user_filter,
        filtering,
        ..
    } = rooms;
    let Some(active) = open.get_mut(*active_index) else {
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

    let OpenRoom {
        lines,
        view,
        wrapped,
        users,
        ..
    } = active;
    render_messages(frame, body[0], lines, view, wrapped, log_filter);
    let shown = matching_users(users, user_filter);
    let typing_users = *filtering == Some(ChatFilter::Users);
    let members = if typing_users || !user_filter.is_empty() {
        format!(
            "Users {}/{} · {}{}",
            shown.len(),
            users.len(),
            user_filter,
            if typing_users { "_" } else { "" }
        )
    } else {
        format!("Users ({})", users.len())
    };
    render_users(frame, body[1], &shown, *user_selected, members);

    // Compose line or hint.
    if *composing {
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
    lines: &[RoomLine],
    view: &mut LogView,
    wrapped: &mut WrappedLog,
    filter: &str,
) {
    // Each message wrapped to the pane width, then the rows the reader is
    // at: the newest ones, unless they are holding a place in the history.
    let width = area.width as usize;
    let rendered = wrapped.rows(width, filter, lines, |l| {
        let sender = l.username.as_deref().unwrap_or_default();
        if !contains_filter(&l.text, filter) && !contains_filter(sender, filter)
        {
            return Vec::new();
        }
        let time =
            Span::styled(l.at.format("%H:%M ").to_string(), dimmed_style());
        match &l.username {
            Some(user) => wrap_chat_line(
                vec![time, Span::styled(format!("<{user}> "), info_style())],
                &l.text,
                primary_style(),
                width,
            ),
            None => wrap_chat_line(vec![time], &l.text, dimmed_style(), width),
        }
    });
    let window = view.window(rendered.len(), usize::from(area.height));
    frame.render_widget(Paragraph::new(rendered[window].to_vec()), area);
}

fn render_users(
    frame: &mut Frame,
    area: Rect,
    users: &[&str],
    selected: usize,
    title: String,
) {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(dimmed_style())
        .padding(PANE_PADDING)
        .title(plain_title(title, false));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let height = inner.height as usize;
    // Keep the highlighted user in view by scrolling the window to it.
    let start = selected.saturating_sub(height.saturating_sub(1));
    let shown: Vec<Line> = users
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, u)| {
            if i == selected {
                Line::from(Span::styled(format!("▸ {u}"), highlight_style()))
            } else {
                Line::from(Span::styled(format!("  {u}"), primary_style()))
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(shown), inner);
}
