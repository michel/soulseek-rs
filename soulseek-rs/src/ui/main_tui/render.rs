use super::MainTui;
use crate::models::{CommandBarMode, FocusedPane, MessageDirection, RoomsView};
use crate::ui::panes::{
    ResultsPaneParams, render_browse_pane, render_download_info_pane,
    render_downloads_pane, render_results_pane, render_rooms_pane,
    render_searches_pane,
};
use crate::ui::{
    border_style, border_type, format_shortcuts_styled, render_download_stats,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Position, Rect},
    widgets::{Block, Borders, Paragraph},
};

const COMMAND_BAR_PREFIX: &str = "search: ";
const MESSAGE_BAR_PREFIX: &str = "message (to: recipient text): ";
const BROWSE_BAR_PREFIX: &str = "browse user: ";

const fn command_bar_prefix(mode: CommandBarMode) -> &'static str {
    match mode {
        CommandBarMode::Search => COMMAND_BAR_PREFIX,
        CommandBarMode::Message => MESSAGE_BAR_PREFIX,
        CommandBarMode::Browse => BROWSE_BAR_PREFIX,
    }
}

impl MainTui {
    pub(super) fn render(&mut self, frame: &mut Frame) {
        let mut constraints = vec![
            Constraint::Length(3), // Status bar
            Constraint::Fill(1),   // Main content
        ];
        if self.state.command_bar_active {
            constraints.push(Constraint::Length(3)); // Command bar
        }
        constraints.push(Constraint::Length(3)); // Shortcuts
        let main_chunks = Layout::vertical(constraints).split(frame.area());

        render_download_stats(
            frame,
            main_chunks[0],
            &self.state.downloads,
            self.state.active_downloads_count,
        );

        self.render_content(frame, main_chunks[1]);

        // Render command bar if active (vim-style, above shortcuts)
        if self.state.command_bar_active {
            self.render_command_bar(frame, main_chunks[2]);
            self.render_shortcuts(frame, main_chunks[3]);
        } else {
            self.render_shortcuts(frame, main_chunks[2]);
        }

        self.render_overlays(frame);
    }

    fn render_content(&mut self, frame: &mut Frame, area: Rect) {
        // Split main content area
        let content_chunks = Layout::horizontal([
            Constraint::Percentage(30), // Searches pane
            Constraint::Percentage(70), // Results + Downloads
        ])
        .split(area);

        // Store searches pane area
        self.state.searches_pane_area = Some(content_chunks[0]);

        // Render Searches pane (left)
        render_searches_pane(
            frame,
            content_chunks[0],
            &self.state.searches,
            &mut self.state.searches_table_state,
            self.state.focused_pane == FocusedPane::Searches,
        );

        // Split right side into Results (top) and Downloads (bottom)
        let right_chunks = Layout::vertical([
            Constraint::Percentage(60), // Results
            Constraint::Percentage(40), // Downloads
        ])
        .split(content_chunks[1]);

        // Split bottom-right into Downloads list and Info pane
        let downloads_chunks = Layout::horizontal([
            Constraint::Percentage(60), // Downloads list
            Constraint::Percentage(40), // Info pane
        ])
        .split(right_chunks[1]);

        // Store results and downloads pane areas
        self.state.results_pane_area = Some(right_chunks[0]);
        self.state.downloads_pane_area = Some(downloads_chunks[0]);

        // Render Results pane. When a filter is active the rendered rows are a
        // subset, so the pane also needs the mapping back to unfiltered indices
        // to render the selection checkboxes correctly.
        let (results_items, results_original_indices) =
            if self.state.results_filter_query.is_empty() {
                (&self.state.results_items, None)
            } else {
                (
                    &self.state.results_filtered_items,
                    Some(self.state.results_filtered_indices.as_slice()),
                )
            };

        let active_search_query = self
            .state
            .selected_search_index
            .and_then(|idx| self.state.searches.get(idx))
            .map(|search| search.query.as_str());

        render_results_pane(
            frame,
            right_chunks[0],
            ResultsPaneParams {
                items: results_items,
                table_state: &mut self.state.results_table_state,
                selected_indices: &self.state.results_selected_indices,
                original_indices: results_original_indices,
                filter_query: &self.state.results_filter_query,
                is_filtering: self.state.results_is_filtering,
                focused: self.state.focused_pane == FocusedPane::Results,
                active_search_query,
            },
        );

        render_downloads_pane(
            frame,
            downloads_chunks[0],
            &self.state.downloads,
            &mut self.state.downloads_table_state,
            self.state.focused_pane == FocusedPane::Downloads,
        );

        let selected_download = self
            .state
            .downloads_table_state
            .selected()
            .and_then(|idx| self.state.downloads.get(idx));
        render_download_info_pane(
            frame,
            downloads_chunks[1],
            selected_download,
            self.state.focused_pane == FocusedPane::Downloads,
        );
    }

    fn render_overlays(&mut self, frame: &mut Frame) {
        // Messages inbox overlays everything when open.
        if self.state.show_messages {
            self.render_messages_popup(frame);
        }

        // Browse tree overlays everything when open.
        if self.state.show_browse && !self.state.browse.is_empty() {
            let area = centered_rect(80, 80, frame.area());
            frame.render_widget(ratatui::widgets::Clear, area);
            render_browse_pane(
                frame,
                area,
                &self.state.browse,
                &mut self.state.browse_table_state,
                self.spinner_state,
            );
        }

        // Chat rooms overlay everything when open.
        if self.state.show_rooms {
            let area = centered_rect(85, 80, frame.area());
            frame.render_widget(ratatui::widgets::Clear, area);
            render_rooms_pane(
                frame,
                area,
                &self.state.rooms,
                &mut self.state.rooms_list_table_state,
            );
        }
    }

    fn render_messages_popup(&self, frame: &mut Frame) {
        let area = centered_rect(70, 60, frame.area());

        let lines: Vec<ratatui::text::Line> = if self.state.messages.is_empty()
        {
            vec![ratatui::text::Line::from(
                "No messages yet. Press 'm' to send one.",
            )]
        } else {
            self.state
                .messages
                .iter()
                .map(|m| {
                    let (arrow, who) = match m.direction {
                        MessageDirection::Incoming => ("⇦ from", &m.peer),
                        MessageDirection::Outgoing => ("⇨ to  ", &m.peer),
                    };
                    ratatui::text::Line::from(format!(
                        "{arrow} {who}: {}",
                        m.text
                    ))
                })
                .collect()
        };

        let popup = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style(true))
                .border_type(border_type(true))
                .title(" Messages  (m: compose, i/Esc: close) "),
        );

        frame.render_widget(ratatui::widgets::Clear, area);
        frame.render_widget(popup, area);
    }

    /// Context shortcuts for the chat-rooms popup.
    fn rooms_shortcuts(&self) -> Vec<(&'static str, &'static str)> {
        if self.state.rooms.composing {
            return vec![
                ("Type", "message"),
                ("Enter", "send"),
                ("Esc", "cancel"),
            ];
        }
        match self.state.rooms.view {
            RoomsView::List => {
                if self.state.rooms.list_is_filtering {
                    vec![
                        ("Type", "filter"),
                        ("Enter", "join match"),
                        ("Esc", "clear filter"),
                    ]
                } else {
                    vec![
                        ("↑↓", "move"),
                        ("Enter", "join"),
                        ("/", "filter"),
                        ("Tab", "open rooms"),
                        ("Esc", "close"),
                    ]
                }
            }
            RoomsView::Chat => vec![
                ("Enter", "say"),
                ("↑↓", "pick user"),
                ("b", "browse user"),
                ("m", "message user"),
                ("Tab", "switch room"),
                ("l", "rooms"),
                ("x", "leave"),
            ],
        }
    }

    fn render_shortcuts(&self, frame: &mut Frame, area: Rect) {
        // Unread badges for the inbox and chat shortcuts.
        let inbox_label = if self.state.unread_messages > 0 {
            format!("inbox ({})", self.state.unread_messages)
        } else {
            "inbox".to_string()
        };
        let chat_unread = self.state.rooms.total_unread();
        let chat_label = if chat_unread > 0 {
            format!("chat ({chat_unread})")
        } else {
            "chat".to_string()
        };

        let shortcuts = if self.state.show_rooms {
            self.rooms_shortcuts()
        } else if self.state.show_browse {
            vec![
                ("↑↓", "move"),
                ("→←", "expand/collapse"),
                ("Enter", "open/download"),
                ("d", "download folder"),
                ("Tab", "switch user"),
                ("r", "retry"),
                ("w", "close tab"),
                ("Esc", "hide"),
            ]
        } else if self.state.command_bar_active {
            match self.state.command_bar_mode {
                CommandBarMode::Search => vec![
                    ("Type", "search term"),
                    ("←→", "move cursor"),
                    ("Backspace/Del", "edit"),
                    ("Enter", "search"),
                    ("Esc", "cancel"),
                ],
                CommandBarMode::Message => vec![
                    ("Type", "recipient then message"),
                    ("Enter", "send"),
                    ("Esc", "cancel"),
                ],
                CommandBarMode::Browse => vec![
                    ("Type", "username"),
                    ("Enter", "browse"),
                    ("Esc", "cancel"),
                ],
            }
        } else {
            match self.state.focused_pane {
                FocusedPane::Searches => vec![
                    ("s", "search"),
                    ("m", "message"),
                    ("i", inbox_label.as_str()),
                    ("c", chat_label.as_str()),
                    ("b", "browse user"),
                    ("1-3", "focus pane"),
                    ("↑↓", "navigate"),
                    ("Enter", "results"),
                    ("q", "quit"),
                ],
                FocusedPane::Results if self.state.results_is_filtering => {
                    vec![
                        ("Type", "filter"),
                        ("Esc", "clear filter"),
                        ("1-3", "focus pane"),
                        ("q", "quit"),
                    ]
                }
                FocusedPane::Results => vec![
                    ("Space", "select"),
                    ("Enter", "download"),
                    ("b", "browse owner"),
                    ("c", chat_label.as_str()),
                    ("/", "filter"),
                    ("a/A", "select all/none"),
                    ("1-3", "focus pane"),
                    ("q", "quit"),
                ],
                FocusedPane::Downloads => {
                    vec![
                        ("p", "pause/resume"),
                        ("r", "retry failed"),
                        ("d", "delete queued"),
                        ("c", "clear finished"),
                        ("b", "browse user"),
                        ("1-3", "focus pane"),
                        ("q", "quit"),
                    ]
                }
            }
        };

        let shortcuts_line = format_shortcuts_styled(&shortcuts);
        // Surface our own sharing status in the block title.
        let shared = self.client.shared_directories();
        let sharing = match shared.as_slice() {
            [] => "off".to_string(),
            [only] => only.clone(),
            more => format!("{} folders", more.len()),
        };
        let title = format!("Shortcuts · Sharing: {sharing}");
        let shortcuts_widget = Paragraph::new(shortcuts_line).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(border_type(false))
                .title(title),
        );

        frame.render_widget(shortcuts_widget, area);
    }

    fn render_command_bar(&self, frame: &mut Frame, area: Rect) {
        let prefix = command_bar_prefix(self.state.command_bar_mode);
        let content_width = area.width.saturating_sub(2);
        let prefix_width = prefix.chars().count() as u16;
        let input_width = content_width.saturating_sub(prefix_width);
        let (visible_input, cursor_column) = visible_input_at_cursor(
            &self.state.command_bar_input,
            self.state.command_bar_cursor_position,
            input_width,
        );
        let command_text = format!("{prefix}{visible_input}");

        let paragraph = Paragraph::new(command_text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style(true))
                .border_type(border_type(true)),
        );

        frame.render_widget(paragraph, area);

        if area.width > 2 && area.height > 2 {
            let cursor_x = area
                .x
                .saturating_add(1)
                .saturating_add(prefix_width)
                .saturating_add(cursor_column)
                .min(area.x.saturating_add(area.width.saturating_sub(2)));
            frame.set_cursor_position(Position::new(cursor_x, area.y + 1));
        }
    }
}

/// A `Rect` centered within `area`, sized to the given percentages.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

fn visible_input_at_cursor(
    input: &str,
    cursor_position: usize,
    width: u16,
) -> (String, u16) {
    if width == 0 {
        return (String::new(), 0);
    }

    let cursor_position = input.floor_char_boundary(cursor_position);
    let cursor_character_index = input[..cursor_position].chars().count();
    let max_cursor_column = usize::from(width.saturating_sub(1));
    let start_character_index =
        cursor_character_index.saturating_sub(max_cursor_column);

    let visible_input = input
        .chars()
        .skip(start_character_index)
        .take(usize::from(width))
        .collect();
    let cursor_column = cursor_character_index
        .saturating_sub(start_character_index)
        .min(max_cursor_column);

    (visible_input, cursor_column as u16)
}
