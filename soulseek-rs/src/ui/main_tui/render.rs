use super::MainTui;
use crate::models::{CommandBarMode, FocusedPane, RoomsView};
use crate::ui::panes::{
    ResultsPaneParams, render_browse_pane, render_chat_pane,
    render_download_info_pane, render_downloads_pane, render_results_pane,
    render_rooms_pane, render_searches_pane, selected_transfer,
};
use crate::ui::{
    accent_style, dimmed_style, format_shortcuts_styled, pane_block,
    plain_title, primary_style, render_download_stats, warning_style,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
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
            &self.state.uploads,
            &mut self.state.downloads_table_state,
            self.state.focused_pane == FocusedPane::Downloads,
        );

        let selected_transfer = selected_transfer(
            self.state.downloads_table_state.selected(),
            &self.state.downloads,
            &self.state.uploads,
        );
        render_download_info_pane(
            frame,
            downloads_chunks[1],
            selected_transfer,
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

        // Settings popup overlays everything when open.
        if self.state.settings.is_some() {
            self.render_settings_popup(frame);
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

    fn render_settings_popup(&self, frame: &mut Frame) {
        use crate::models::SettingsMode;
        let Some(settings) = self.state.settings.as_ref() else {
            return;
        };
        let area = centered_rect(70, 60, frame.area());
        frame.render_widget(ratatui::widgets::Clear, area);

        let mut lines: Vec<Line> = Vec::new();
        let marker = |selected: bool| if selected { "> " } else { "  " };
        let entry = |selected: bool, label: &str, value: String| {
            Line::from(vec![
                Span::styled(marker(selected).to_string(), accent_style()),
                Span::styled(label.to_string(), dimmed_style()),
                Span::styled(value, primary_style()),
            ])
        };

        lines.push(if settings.mode == SettingsMode::EditingDownloadDir {
            Line::from(vec![
                Span::styled("> ", accent_style()),
                Span::styled("Download folder: ", dimmed_style()),
                Span::styled(settings.input.clone(), primary_style()),
                Span::styled("▏", accent_style()),
            ])
        } else {
            entry(
                settings.selected == 0,
                "Download folder: ",
                settings.download_dir.clone(),
            )
        });
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!("Shared folders ({}):", settings.share_dirs.len()),
            dimmed_style(),
        ));
        for (i, dir) in settings.share_dirs.iter().enumerate() {
            lines.push(entry(settings.selected == i + 1, "", dir.clone()));
        }
        if settings.share_dirs.is_empty() {
            lines.push(Line::styled(
                "  (nothing shared — press 'a' to add a folder)",
                dimmed_style(),
            ));
        }
        if settings.mode == SettingsMode::AddingShare {
            lines.push(Line::from(vec![
                Span::styled("  Add share: ", dimmed_style()),
                Span::styled(settings.input.clone(), primary_style()),
                Span::styled("▏", accent_style()),
            ]));
        }
        if let Some(status) = &settings.status {
            lines.push(Line::from(""));
            lines.push(Line::styled(status.clone(), warning_style()));
        }

        let block = pane_block(true).title(plain_title("Settings", true));
        frame.render_widget(
            ratatui::widgets::Paragraph::new(lines)
                .block(block)
                .wrap(ratatui::widgets::Wrap { trim: false }),
            area,
        );
    }

    fn render_messages_popup(&self, frame: &mut Frame) {
        // The per-conversation chat box supersedes the old flat message list;
        // it renders the messages and the compose line itself.
        let area = centered_rect(70, 60, frame.area());
        frame.render_widget(ratatui::widgets::Clear, area);
        render_chat_pane(frame, area, &self.state, &self.client.username());
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

    /// Which shortcuts the bar offers, which is purely a question of what is
    /// open and where the focus sits. Sibling of [`Self::rooms_shortcuts`].
    fn shortcuts(&self) -> Vec<(&'static str, &'static str)> {
        if self.state.settings.is_some() {
            vec![
                ("↑↓", "move"),
                ("Enter/e", "edit download dir"),
                ("a", "add share"),
                ("d", "remove share"),
                ("r", "re-index"),
                ("Esc", "close"),
            ]
        } else if self.state.show_messages {
            if self.state.chat_composing {
                vec![
                    ("Type", "message"),
                    ("Enter", "send"),
                    ("Esc", "stop typing"),
                ]
            } else {
                vec![
                    ("Enter", "type"),
                    ("↑↓/Tab", "switch chat"),
                    ("m", "new chat"),
                    ("i/Esc", "close"),
                ]
            }
        } else if self.state.show_rooms {
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
                    ("i", "inbox"),
                    ("c", "chat"),
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
                    ("c", "chat"),
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
        }
    }

    fn render_shortcuts(&self, frame: &mut Frame, area: Rect) {
        let shortcuts_line = format_shortcuts_styled(&self.shortcuts());
        // Surface our own sharing status in the block title.
        let shared = self.client.shared_directories();
        let sharing = match shared.as_slice() {
            [] => "off".to_string(),
            [only] => only.clone(),
            more => format!("{} folders", more.len()),
        };
        let title = format!("Shortcuts · Sharing: {sharing}");
        // Themed block (master's design system), but we take its inner area so
        // the unread badge can be split off to the right below.
        let block = pane_block(false).title(plain_title(title, false));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // A prominent unread badge, right-aligned so the context shortcuts on
        // the left never crowd it out. It rides in the always-visible shortcuts
        // bar, so unread mail/chat shows in every pane, not just some. The
        // spinner tick (0..10, ~100ms each) drives a ~1s blink: bright for the
        // first half of the cycle, dim for the second.
        let unread = unread_indicator(
            self.state.unread_messages,
            self.state.rooms.total_unread(),
            self.spinner_state < 5,
        );
        if unread.spans.is_empty() {
            frame.render_widget(Paragraph::new(shortcuts_line), inner);
        } else {
            let badge_width = unread.width() as u16;
            let cols = Layout::horizontal([
                Constraint::Fill(1),
                Constraint::Length(badge_width),
            ])
            .split(inner);
            frame.render_widget(Paragraph::new(shortcuts_line), cols[0]);
            frame.render_widget(
                Paragraph::new(unread).alignment(Alignment::Right),
                cols[1],
            );
        }
    }

    fn render_command_bar(&self, frame: &mut Frame, area: Rect) {
        let prefix = command_bar_prefix(self.state.command_bar_mode);
        let block = pane_block(true);
        // Derive from the block's inner rect so borders and padding stay in
        // one place; the cursor follows whatever the pane reserves.
        let inner = block.inner(area);
        let prefix_width = prefix.chars().count() as u16;
        let input_width = inner.width.saturating_sub(prefix_width);
        let (visible_input, cursor_column) = visible_input_at_cursor(
            &self.state.command_bar_input,
            self.state.command_bar_cursor_position,
            input_width,
        );
        let command_line = Line::from(vec![
            Span::styled(prefix, accent_style()),
            Span::styled(visible_input, primary_style()),
        ]);

        frame.render_widget(Paragraph::new(command_line).block(block), area);

        if inner.width > 0 && inner.height > 0 {
            let cursor_x = inner
                .x
                .saturating_add(prefix_width)
                .saturating_add(cursor_column)
                .min(inner.x.saturating_add(inner.width.saturating_sub(1)));
            frame.set_cursor_position(Position::new(cursor_x, inner.y));
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

/// Build the shortcuts-bar unread badge: `✉ N` for unread private messages and
/// `💬 N` for unread chat-room messages. Returns an empty line when nothing is
/// unread so the caller can skip it.
///
/// `bright` drives a self-drawn blink: the caller flips it on a timer so the
/// badge pulses between bold warning colour and dim. We do the blink ourselves
/// rather than lean on the terminal's SLOW_BLINK attribute, which tmux and most
/// macOS terminals silently drop. The text is identical in both phases, so the
/// badge keeps its width and the bar never reflows.
fn unread_indicator(
    messages: usize,
    rooms: usize,
    bright: bool,
) -> Line<'static> {
    let mut counts: Vec<String> = Vec::new();
    if messages > 0 {
        counts.push(format!("✉ {messages}"));
    }
    if rooms > 0 {
        counts.push(format!("💬 {rooms}"));
    }
    if counts.is_empty() {
        return Line::from(Vec::new());
    }
    let style = if bright {
        warning_style().add_modifier(Modifier::BOLD)
    } else {
        dimmed_style()
    };
    Line::from(Span::styled(format!("⟨ {} ⟩", counts.join("  ")), style))
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

#[cfg(test)]
mod tests {
    use super::unread_indicator;

    fn text(messages: usize, rooms: usize) -> String {
        unread_indicator(messages, rooms, true)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    #[test]
    fn unread_indicator_is_empty_when_nothing_is_unread() {
        // Empty in both blink phases — no badge means no badge.
        assert!(unread_indicator(0, 0, true).spans.is_empty());
        assert!(unread_indicator(0, 0, false).spans.is_empty());
    }

    #[test]
    fn unread_indicator_shows_each_source_and_both_together() {
        // Bracketed so the badge reads as one distinct block.
        assert_eq!(text(3, 0), "⟨ ✉ 3 ⟩");
        assert_eq!(text(0, 2), "⟨ 💬 2 ⟩");
        // Both present: DMs first, rooms second, separated so they don't run
        // together.
        assert_eq!(text(3, 2), "⟨ ✉ 3  💬 2 ⟩");
    }

    #[test]
    fn blink_phases_share_text_but_differ_in_style() {
        // The self-drawn blink must keep the same glyphs (so the bar never
        // reflows) while changing style, or it would not read as blinking.
        let bright = unread_indicator(1, 0, true);
        let dim = unread_indicator(1, 0, false);
        assert_eq!(bright.spans[0].content, dim.spans[0].content);
        assert_ne!(
            bright.spans[0].style, dim.spans[0].style,
            "bright and dim phases must look different"
        );
    }
}
