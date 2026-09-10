//! The keys the window advertises: the shortcut bar along the foot, the `?`
//! overlay behind it, and the unread badge that rides in the bar.
//!
//! A child module of `render`, so it still reaches the window's state and
//! `render`'s own helpers directly; only what `render` calls back into is
//! widened to `pub(super)`.

use super::{
    Alignment, CommandBarMode, Constraint, FocusedPane, Frame, Layout, Line,
    MainTui, Modifier, PaneLayout, Paragraph, Rect, RoomsView, Span,
    accent_style, body_style, dimmed_style, info_style, pack_shortcuts,
    pane_block, plain_title, warning_style,
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
            ("W", "every pane back, zoom off, sizes reset"),
            ("click", "focus a pane"),
            ("drag", "resize panes by their borders"),
        ],
    ),
    (
        "Anywhere",
        &[
            ("s", "search the network"),
            ("b", "browse a user's files"),
            ("B", "browse what you share yourself"),
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
    (
        "Settings popup",
        &[
            ("Enter", "change the password, log out, edit the folder"),
            ("a / d", "add / remove a shared folder"),
            ("r", "re-index the shares"),
        ],
    ),
];

const HELP_RIGHT: &[(&str, &[(&str, &str)])] = &[
    (
        "Searches",
        &[
            ("Enter", "show its results"),
            ("S", "run the search again"),
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
            ("C", "clear every one, cancelling the live ones"),
            ("b", "browse that user"),
        ],
    ),
    (
        "Chat and browse popups",
        &[
            ("PgUp PgDn / ^b ^f", "a page of messages, or of the tree"),
            ("^u ^d", "half a page of them"),
            ("Home End / g G", "oldest / newest, or first / last row"),
            ("Tab / Shift-Tab", "next room, chat or user"),
            ("/", "filter the messages, the room list or the tree"),
            ("u", "filter a room's member list"),
        ],
    ),
    (
        "Browse popup",
        &[
            ("← → / h l", "close / open a folder, or step out / in"),
            ("J K", "next / previous folder"),
            ("H L", "close / open every folder"),
            ("/", "filter by path, Enter keeps it, Esc clears"),
            ("Enter", "open a folder, or download a file"),
            ("d", "download a file, or a folder's files"),
            ("r", "ask again after a timeout, re-index your own"),
            ("o", "shared folders (your own tab)"),
        ],
    ),
];

/// What the bar offers while a filter is being typed, wherever that is,
/// with the keys that still move around there.
fn filter_keys(
    moves: &[(&'static str, &'static str)],
) -> Vec<(&'static str, &'static str)> {
    let mut keys = vec![("Type", "filter")];
    keys.extend_from_slice(moves);
    keys.extend([("Enter", "keep filter"), ("Esc", "clear filter")]);
    keys
}

/// What the bar offers in the settings popup: the keys that always apply,
/// plus what Enter does on the row the selection rests on.
fn settings_shortcuts(
    settings: &crate::models::SettingsState,
) -> Vec<(&'static str, &'static str)> {
    use crate::models::{SettingsMode, SettingsRow};
    match settings.mode {
        SettingsMode::ConfirmingLogout => {
            vec![("y", "log out"), ("n/Esc", "stay")]
        }
        SettingsMode::NewPassword => {
            vec![
                ("Type", "password"),
                ("Enter", "repeat it"),
                ("Esc", "cancel"),
            ]
        }
        SettingsMode::RepeatPassword(_) => {
            vec![
                ("Type", "password"),
                ("Enter", "change it"),
                ("Esc", "cancel"),
            ]
        }
        SettingsMode::EditingDownloadDir | SettingsMode::AddingShare => {
            vec![("Type", "path"), ("Enter", "save"), ("Esc", "cancel")]
        }
        SettingsMode::Navigate => vec![
            ("↑↓", "move"),
            match settings.selected_row() {
                SettingsRow::ChangePassword => ("Enter", "change password"),
                SettingsRow::Logout => ("Enter", "log out"),
                SettingsRow::DownloadDir => ("Enter", "edit folder"),
                SettingsRow::Share(_) => ("d", "remove share"),
            },
            ("a", "add share"),
            ("r", "re-index"),
            ("Esc", "close"),
        ],
    }
}

impl MainTui {
    /// The keys list, two columns side by side where the terminal is wide
    /// enough and one on top of the other where it is not, scrolling when
    /// even that is taller than the window.
    pub(super) fn render_help_popup(&mut self, frame: &mut Frame) {
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
            RoomsView::Chat if self.state.rooms.filtering.is_some() => {
                filter_keys(&[("PgUp/PgDn", "scroll"), ("↑↓", "pick user")])
            }
            RoomsView::Chat => vec![
                ("Enter", "say"),
                ("PgUp/PgDn", "scroll"),
                ("↑↓", "pick user"),
                ("/", "find in chat"),
                ("u", "find user"),
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
        let mut keys = vec![
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
        ];
        // Only once a pane is hidden or zoomed is there a layout to reset.
        if self.state.layout != PaneLayout::default() {
            keys.push(("W", "reset layout"));
        }
        keys.extend([("?", "keys"), ("q", "quit")]);
        keys
    }

    /// Which shortcuts the bar offers, which is purely a question of what is
    /// open and where the focus sits. Sibling of [`Self::rooms_shortcuts`].
    fn shortcuts(&self) -> Vec<(&'static str, &'static str)> {
        if self.state.show_help {
            vec![("?/Esc", "close")]
        } else if let Some(settings) = self.state.settings.as_ref() {
            settings_shortcuts(settings)
        } else if self.state.show_messages {
            if self.state.chat_composing {
                vec![
                    ("Type", "message"),
                    ("Enter", "send"),
                    ("Esc", "stop typing"),
                ]
            } else if self.state.chat_filtering {
                filter_keys(&[("PgUp/PgDn", "scroll")])
            } else {
                vec![
                    ("Enter", "type"),
                    ("PgUp/PgDn", "scroll"),
                    ("↑↓/Tab", "switch chat"),
                    ("/", "find"),
                    ("m", "new chat"),
                    ("i/Esc", "close"),
                ]
            }
        } else if self.state.show_rooms {
            self.rooms_shortcuts()
        } else if self.state.show_browse {
            if self.state.browse.active_tab().is_some_and(|b| b.filtering) {
                // Typing goes to the filter, so no letter keys are on offer.
                filter_keys(&[("↑/↓", "navigate"), ("PgUp/PgDn", "page")])
            } else {
                // Nothing of ours is downloadable from ourselves, and `r`
                // re-scans the disk instead of re-asking a peer.
                let acts = if self.state.browse.active_is_own() {
                    [
                        ("Enter", "open folder"),
                        ("r", "re-index"),
                        ("o", "shared folders"),
                    ]
                } else {
                    [
                        ("Enter", "open/download"),
                        ("d", "download folder"),
                        ("r", "retry"),
                    ]
                };
                let mut keys = vec![
                    ("↑↓", "move"),
                    ("J/K", "next/prev folder"),
                    ("→←", "expand/collapse"),
                    ("H/L", "collapse/expand all"),
                    ("/", "filter"),
                ];
                keys.extend(acts);
                keys.extend([
                    ("Tab", "switch user"),
                    ("w", "close tab"),
                    ("Esc", "hide"),
                ]);
                keys
            }
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
            filter_keys(&[("↑/↓", "navigate"), ("PgUp/PgDn", "page")])
        } else {
            let mut keys = match self.state.focused_pane {
                FocusedPane::Searches => vec![
                    ("s", "search"),
                    ("S", "search again"),
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
                    ("C", "clear all"),
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
    pub(super) fn shortcut_rows(&self, width: u16) -> Vec<Line<'static>> {
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

    pub(super) fn render_shortcuts(
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
