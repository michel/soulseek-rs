use super::MainTui;
use crate::models::{CommandBarMode, FocusedPane, RoomsView};
use crate::ui::panes::{
    InfoSubject, ResultsPaneParams, render_browse_pane, render_chat_pane,
    render_download_info_pane, render_downloads_pane, render_results_pane,
    render_rooms_pane, render_searches_pane, selected_transfer,
};
use crate::ui::{
    accent_style, body_style, dimmed_style, info_style, pack_shortcuts,
    pane_block, plain_title, primary_style, render_download_stats,
    warning_style,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
};

/// The keys overlay, grouped by where a key applies. Two columns, because a
/// single list runs longer than most terminals are tall.
const HELP_LEFT: &[(&str, &[(&str, &str)])] = &[
    (
        "Panes",
        &[
            ("Tab / Shift-Tab", "next / previous pane"),
            ("1 2 3", "focus a pane, hidden or not"),
            ("z", "zoom the focused pane"),
            ("w", "hide the focused pane"),
            ("Esc", "leave zoom"),
            ("click", "focus a pane"),
        ],
    ),
    (
        "Anywhere",
        &[
            ("s", "search the network"),
            ("b", "browse a user's files"),
            ("m", "compose a private message"),
            ("i", "inbox"),
            ("o", "settings"),
            ("?", "this list"),
            ("q", "quit"),
        ],
    ),
    (
        "Any list",
        &[
            ("↑ ↓ / k j", "move one row"),
            ("Home End / g G", "first / last row"),
            ("PgUp PgDn / ^b ^f", "one page"),
            ("^u ^d", "half a page"),
            ("h l / ← →", "scroll a long name sideways"),
            ("0 / $", "its start / its end"),
        ],
    ),
];

const HELP_RIGHT: &[(&str, &[(&str, &str)])] = &[
    (
        "Searches",
        &[
            ("Enter", "show its results"),
            ("d", "remove the search"),
            ("C", "clear every search"),
            ("c", "chat rooms"),
        ],
    ),
    (
        "Results",
        &[
            ("Space", "select / deselect"),
            ("a / A", "select all / none"),
            ("Enter", "download the selection"),
            ("/", "filter, Enter keeps it, Esc clears"),
            ("b", "browse the owner"),
            ("c", "chat rooms"),
        ],
    ),
    (
        "Downloads / Uploads",
        &[
            ("p", "pause / resume"),
            ("x", "cancel"),
            ("r", "retry a failed one"),
            ("d", "delete a queued or finished one"),
            ("c", "clear every finished one"),
        ],
    ),
    (
        "Chat and browse popups",
        &[
            ("PgUp PgDn / ^b ^f", "a page of messages, or of the tree"),
            ("^u ^d", "half a page of them"),
            ("Home End / g G", "oldest / newest, or first / last row"),
            ("Tab / Shift-Tab", "next room, chat or user"),
        ],
    ),
];

impl MainTui {
    pub(super) fn render(&mut self, frame: &mut Frame) {
        let shortcuts = self.shortcut_rows(frame.area().width);
        let mut constraints = vec![
            Constraint::Length(3), // Status bar
            Constraint::Fill(1),   // Main content
        ];
        if self.state.command_bar_active {
            constraints.push(Constraint::Length(3)); // Command bar
        }
        constraints.push(Constraint::Length(shortcuts.len() as u16 + 2));
        let main_chunks = Layout::vertical(constraints).split(frame.area());

        let daemon = self.client.daemon_endpoint();
        render_download_stats(
            frame,
            main_chunks[0],
            &self.state.downloads,
            self.state.active_downloads_count,
            daemon.as_deref(),
        );

        self.render_content(frame, main_chunks[1]);

        // Render command bar if active (vim-style, above shortcuts)
        if self.state.command_bar_active {
            self.render_command_bar(frame, main_chunks[2]);
            self.render_shortcuts(frame, main_chunks[3], shortcuts);
        } else {
            self.render_shortcuts(frame, main_chunks[2], shortcuts);
        }

        self.render_overlays(frame);
    }

    /// Results across the top, where a long file name has the whole width;
    /// Searches, Downloads and Info in a row underneath. A hidden pane drops
    /// out of its row and the rest widen; a zoomed one has the area alone.
    fn render_content(&mut self, frame: &mut Frame, area: Rect) {
        // Only what is drawn can be clicked.
        self.state.searches_pane_area = None;
        self.state.results_pane_area = None;
        self.state.downloads_pane_area = None;

        if self.state.layout.zoomed {
            self.render_pane(frame, area, self.state.focused_pane);
            return;
        }

        // Transfers carry the most columns, so they get the most width.
        let row_panes: Vec<(FocusedPane, u16)> =
            [(FocusedPane::Searches, 5), (FocusedPane::Downloads, 10)]
                .into_iter()
                .filter(|(pane, _)| self.state.layout.is_visible(*pane))
                .collect();

        let row_area = if self.state.layout.is_visible(FocusedPane::Results) {
            // With only Info left in the row, the results deserve more of
            // the height.
            let row_weight = if row_panes.is_empty() { 1 } else { 2 };
            let [top, bottom] = Layout::vertical([
                Constraint::Fill(3),
                Constraint::Fill(row_weight),
            ])
            .areas(area);
            self.render_pane(frame, top, FocusedPane::Results);
            bottom
        } else {
            area
        };

        let mut constraints: Vec<Constraint> = row_panes
            .iter()
            .map(|(_, weight)| Constraint::Fill(*weight))
            .collect();
        constraints.push(Constraint::Fill(6)); // Info
        let chunks = Layout::horizontal(constraints).split(row_area);
        for (chunk, (pane, _)) in chunks.iter().zip(&row_panes) {
            self.render_pane(frame, *chunk, *pane);
        }
        self.render_info_pane(frame, chunks[row_panes.len()]);
    }

    fn render_pane(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        pane: FocusedPane,
    ) {
        let focused = self.state.focused_pane == pane;
        match pane {
            FocusedPane::Searches => {
                self.state.searches_pane_area = Some(area);
                render_searches_pane(
                    frame,
                    area,
                    &self.state.searches,
                    &mut self.state.searches_table_state,
                    focused,
                    self.state.searches_query_offset,
                );
            }
            FocusedPane::Results => {
                self.state.results_pane_area = Some(area);
                self.render_results_pane(frame, area, focused);
            }
            FocusedPane::Downloads => {
                self.state.downloads_pane_area = Some(area);
                render_downloads_pane(
                    frame,
                    area,
                    &self.state.downloads,
                    &self.state.uploads,
                    &mut self.state.downloads_table_state,
                    focused,
                    self.state.downloads_name_offset,
                );
            }
        }
    }

    fn render_results_pane(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        focused: bool,
    ) {
        // When a filter is active the rendered rows are a subset, so the pane
        // also needs the mapping back to unfiltered indices to render the
        // selection checkboxes correctly.
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
            area,
            ResultsPaneParams {
                items: results_items,
                table_state: &mut self.state.results_table_state,
                selected_indices: &self.state.results_selected_indices,
                original_indices: results_original_indices,
                filter_query: &self.state.results_filter_query,
                is_filtering: self.state.results_is_filtering,
                focused,
                active_search_query,
                name_offset: self.state.results_name_offset,
            },
        );
    }

    /// The Info pane follows the focus: the highlighted result, or the
    /// highlighted transfer.
    fn render_info_pane(&self, frame: &mut Frame, area: Rect) {
        let selected = if self.state.focused_pane == FocusedPane::Results {
            self.highlighted_result().map(InfoSubject::Result)
        } else {
            selected_transfer(
                self.state.downloads_table_state.selected(),
                &self.state.downloads,
                &self.state.uploads,
            )
        };
        render_download_info_pane(
            frame,
            area,
            selected,
            matches!(
                self.state.focused_pane,
                FocusedPane::Results | FocusedPane::Downloads
            ),
        );
    }

    /// The keys list, two columns side by side where the terminal is wide
    /// enough and one on top of the other where it is not, scrolling when
    /// even that is taller than the window.
    fn render_help_popup(&mut self, frame: &mut Frame) {
        let screen = frame.area();
        // Borders and padding on either side of the text.
        let frame_width = 4;
        let columns = help_columns(screen.width.saturating_sub(frame_width));
        let gaps = HELP_GAP * (columns.len() as u16 - 1);
        let text_width = columns.iter().map(|lines| widest(lines)).sum::<u16>();
        let width = screen.width.min(text_width + gaps + frame_width);
        let tallest = columns.iter().map(Vec::len).max().unwrap_or(0);
        let height = screen
            .height
            .min(u16::try_from(tallest + 2).unwrap_or(u16::MAX));
        let area = Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(ratatui::widgets::Clear, area);

        let block = pane_block(true).title(plain_title("Keys", true));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Both columns scroll together, and no further than the taller one
        // needs.
        let window = self
            .state
            .help_view
            .window(tallest, usize::from(inner.height));
        let scroll = u16::try_from(window.start).unwrap_or(u16::MAX);

        let areas =
            Layout::horizontal(vec![Constraint::Fill(1); columns.len()])
                .spacing(HELP_GAP)
                .split(inner);
        for (area, lines) in areas.iter().zip(columns) {
            frame.render_widget(
                Paragraph::new(lines).scroll((scroll, 0)),
                *area,
            );
        }
    }

    fn render_overlays(&mut self, frame: &mut Frame) {
        // Messages inbox overlays everything when open.
        if self.state.show_messages {
            self.render_messages_popup(frame);
        }

        // Browse tree overlays everything when open.
        if self.state.show_browse && !self.state.browse.is_empty() {
            let area = centered_rect(80, 80, frame.area());
            self.state.popup_area = Some(area);
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
            self.state.popup_area = Some(area);
            frame.render_widget(ratatui::widgets::Clear, area);
            render_rooms_pane(
                frame,
                area,
                &mut self.state.rooms,
                &mut self.state.rooms_list_table_state,
            );
        }

        // The keys list only opens from the main view, so it is on top of
        // nothing; drawn last so that stays true if that ever changes.
        if self.state.show_help {
            self.render_help_popup(frame);
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

    fn render_messages_popup(&mut self, frame: &mut Frame) {
        // The per-conversation chat box supersedes the old flat message list;
        // it renders the messages and the compose line itself.
        let area = centered_rect(70, 60, frame.area());
        frame.render_widget(ratatui::widgets::Clear, area);
        self.state.popup_area = Some(area);
        let own = self.client.username();
        render_chat_pane(frame, area, &mut self.state, &own);
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
                ("PgUp/PgDn", "scroll"),
                ("↑↓", "pick user"),
                ("b", "browse user"),
                ("m", "message user"),
                ("Tab", "switch room"),
                ("l", "rooms"),
                ("x", "leave"),
            ],
        }
    }

    /// The keys every pane shares for moving between and resizing panes,
    /// ending the bar the same way wherever the focus sits.
    fn pane_shortcuts(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("Tab/1-3", "pane"),
            (
                "z",
                if self.state.layout.zoomed {
                    "unzoom"
                } else {
                    "zoom"
                },
            ),
            ("w", "hide"),
            ("?", "keys"),
            ("q", "quit"),
        ]
    }

    /// Which shortcuts the bar offers, which is purely a question of what is
    /// open and where the focus sits. Sibling of [`Self::rooms_shortcuts`].
    fn shortcuts(&self) -> Vec<(&'static str, &'static str)> {
        if self.state.show_help {
            vec![("?/Esc", "close")]
        } else if self.state.settings.is_some() {
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
                    ("PgUp/PgDn", "scroll"),
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
                ("PgUp/PgDn", "page"),
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
        } else if self.state.results_is_filtering
            && self.state.focused_pane == FocusedPane::Results
        {
            // Typing goes to the filter, so no letter keys are on offer here.
            vec![
                ("Type", "filter"),
                ("↑/↓", "navigate"),
                ("PgUp/PgDn", "page"),
                ("Enter", "keep filter"),
                ("Esc", "clear filter"),
            ]
        } else {
            let mut keys = match self.state.focused_pane {
                FocusedPane::Searches => vec![
                    ("s", "search"),
                    ("Enter", "results"),
                    ("d", "remove"),
                    ("C", "clear all"),
                    ("m", "message"),
                    ("i", "inbox"),
                    ("c", "chat"),
                    ("b", "browse user"),
                    ("h/l", "scroll query"),
                ],
                FocusedPane::Results => vec![
                    ("Space", "select"),
                    ("Enter", "download"),
                    ("b", "browse owner"),
                    ("c", "chat"),
                    ("/", "filter"),
                    ("a/A", "select all/none"),
                    ("h/l", "scroll name"),
                    ("g/G", "top/end"),
                ],
                FocusedPane::Downloads => vec![
                    ("p", "pause/resume"),
                    ("x", "cancel"),
                    ("r", "retry failed"),
                    ("d", "delete queued/done"),
                    ("c", "clear finished"),
                    ("b", "browse user"),
                    ("h/l", "scroll name"),
                ],
            };
            keys.extend(self.pane_shortcuts());
            keys
        }
    }

    /// The legend packed into rows for a window `width` wide, so a narrow
    /// terminal grows the bar instead of cutting keys off its right edge.
    fn shortcut_rows(&self, width: u16) -> Vec<Line<'static>> {
        // Borders and padding, and the unread badge's column when it shows.
        let badge = self.unread_badge().width() as u16;
        let reserved = 4 + if badge > 0 { badge + 1 } else { 0 };
        pack_shortcuts(&self.shortcuts(), width.saturating_sub(reserved))
    }

    /// A prominent unread badge for the shortcuts bar. It rides in the
    /// always-visible bar, so unread mail/chat shows in every pane, not just
    /// some. The spinner tick (0..10, ~100ms each) drives a ~1s blink: bright
    /// for the first half of the cycle, dim for the second.
    fn unread_badge(&self) -> Line<'static> {
        unread_indicator(
            self.state.unread_messages,
            self.state.rooms.total_unread(),
            self.spinner_state < 5,
        )
    }

    fn render_shortcuts(
        &self,
        frame: &mut Frame,
        area: Rect,
        rows: Vec<Line<'static>>,
    ) {
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

        // The unread badge is right-aligned so the context shortcuts on the
        // left never crowd it out.
        let unread = self.unread_badge();
        if unread.spans.is_empty() {
            frame.render_widget(Paragraph::new(rows), inner);
        } else {
            let badge_width = unread.width() as u16;
            let cols = Layout::horizontal([
                Constraint::Fill(1),
                Constraint::Length(badge_width),
            ])
            .spacing(1)
            .split(inner);
            frame.render_widget(Paragraph::new(rows), cols[0]);
            frame.render_widget(
                Paragraph::new(unread).alignment(Alignment::Right),
                cols[1],
            );
        }
    }

    fn render_command_bar(&self, frame: &mut Frame, area: Rect) {
        let prefix = match self.state.command_bar_mode {
            CommandBarMode::Search => "search: ",
            CommandBarMode::Message => "message (to: recipient text): ",
            CommandBarMode::Browse => "browse user: ",
        };
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

/// Columns between the two halves of the keys list.
const HELP_GAP: u16 = 2;

/// The widest key in a column, so the descriptions line up.
fn key_column_width(sections: &[(&str, &[(&str, &str)])]) -> usize {
    sections
        .iter()
        .flat_map(|(_, keys)| keys.iter())
        .map(|(key, _)| Span::raw(*key).width())
        .max()
        .unwrap_or(0)
}

/// Cells the widest of `lines` takes.
fn widest(lines: &[Line<'_>]) -> u16 {
    let cells = lines.iter().map(Line::width).max().unwrap_or(0);
    u16::try_from(cells).unwrap_or(u16::MAX)
}

/// The keys list laid out for `width` cells of text: two columns when both
/// fit beside each other, otherwise the right half under the left.
fn help_columns(width: u16) -> Vec<Vec<Line<'static>>> {
    let left = help_lines(HELP_LEFT);
    let right = help_lines(HELP_RIGHT);
    if width >= widest(&left) + HELP_GAP + widest(&right) {
        return vec![left, right];
    }
    let mut stacked = left;
    stacked.push(Line::from(""));
    stacked.extend(right);
    vec![stacked]
}

fn help_lines(sections: &[(&str, &[(&str, &str)])]) -> Vec<Line<'static>> {
    let key_width = key_column_width(sections);
    let mut lines = Vec::new();
    for (index, (heading, keys)) in sections.iter().enumerate() {
        if index > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::styled(
            (*heading).to_string(),
            accent_style().add_modifier(Modifier::BOLD),
        ));
        for (key, action) in *keys {
            lines.push(Line::from(vec![
                Span::styled(format!("{key:<key_width$}  "), info_style()),
                Span::styled((*action).to_string(), body_style()),
            ]));
        }
    }
    lines
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
