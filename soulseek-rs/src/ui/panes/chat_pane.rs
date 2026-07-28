use crate::models::{AppState, MessageDirection};
use crate::ui::{
    accent_style, border_style, dimmed_style, highlight_style, info_style,
    primary_style,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

/// Render the private-message chat box: title is the conversation partner,
/// messages left, the list of people you have chatted with on the right, and
/// the compose line pinned to the bottom. `own_username` labels our own
/// messages.
pub fn render_chat_pane(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    own_username: &str,
) {
    let peer = state.active_chat_peer();
    let title = peer.map_or_else(
        || " Messages  (m: compose, i/Esc: close) ".to_string(),
        |peer| format!(" {peer}  (↑↓/Tab: switch, m: to…, i/Esc: close) "),
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(true))
        .border_type(BorderType::Rounded)
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)])
        .split(inner);

    // Body: messages (left) + conversation list (right).
    let body =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(22)])
            .split(chunks[0]);
    render_peers(frame, body[1], state, peer);

    let Some(peer) = peer else {
        frame.render_widget(
            Paragraph::new("No messages yet. Press 'm' to send one.")
                .style(dimmed_style()),
            body[0],
        );
        return;
    };

    render_messages(frame, body[0], state, peer, own_username);
    render_compose(frame, chunks[1], state);
}

fn render_peers(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    active: Option<&str>,
) {
    let peers = state.chat_peers();
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(dimmed_style())
        .title(format!(" Chats ({}) ", peers.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let height = inner.height as usize;
    let selected = active
        .and_then(|active| peers.iter().position(|p| *p == active))
        .unwrap_or(0);
    // Keep the active conversation in view by scrolling the window to it.
    let start = selected.saturating_sub(height.saturating_sub(1));
    let shown: Vec<Line> = peers
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, peer)| {
            if i == selected {
                Line::from(Span::styled(format!("▸ {peer}"), highlight_style()))
            } else {
                Line::from(Span::styled(format!("  {peer}"), primary_style()))
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(shown), inner);
}

fn render_messages(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    peer: &str,
    own_username: &str,
) {
    let lines: Vec<Line> = state
        .messages
        .iter()
        .filter(|m| m.peer == peer)
        .map(|m| {
            let (sender, sender_style) = match m.direction {
                MessageDirection::Incoming => (peer, info_style()),
                MessageDirection::Outgoing => (own_username, accent_style()),
            };
            Line::from(vec![
                Span::styled(m.at.format("%H:%M ").to_string(), dimmed_style()),
                Span::styled(format!("<{sender}> "), sender_style),
                Span::styled(m.text.clone(), primary_style()),
            ])
        })
        .collect();

    // Auto-scroll: one row per message (no wrapping), so tailing by the pane
    // height keeps the newest message visible.
    let start = lines.len().saturating_sub((area.height as usize).max(1));
    frame.render_widget(Paragraph::new(lines[start..].to_vec()), area);
}

fn render_compose(frame: &mut Frame, area: Rect, state: &AppState) {
    let line = if state.chat_composing {
        Line::from(vec![
            Span::styled("› ", accent_style()),
            Span::styled(state.chat_input.clone(), primary_style()),
            Span::styled("▏", accent_style()),
        ])
    } else {
        Line::from(Span::styled("Enter: type a message", dimmed_style()))
    };
    frame.render_widget(Paragraph::new(line), area);
}
