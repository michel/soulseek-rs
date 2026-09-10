mod shortcuts;

use super::MainTui;
use crate::models::{CommandBarMode, FocusedPane, PaneLayout, RoomsView};
use crate::ui::panes::{
    InfoSubject, ResultsPaneParams, render_browse_pane, render_chat_pane,
    render_download_info_pane, render_downloads_pane, render_results_pane,
    render_rooms_pane, render_searches_pane, selected_transfer,
};
use crate::ui::{
    GLYPH_CURSOR, accent_style, body_style, dimmed_style, info_style, mask,
    pack_shortcuts, pane_block, plain_title, primary_style,
    render_download_stats, warning_style,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
};
impl MainTui {
    /// Draw the whole window.
    pub fn render(&mut self, frame: &mut Frame) {
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
        self.state.info_pane_area = None;
        self.state.hsplit_divider = None;
        self.state.vsplit_dividers.clear();
        self.state.row_area = None;

        if self.state.layout.zoomed {
            self.render_pane(frame, area, self.state.focused_pane);
            return;
        }
        self.state.content_area = Some(area);

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
            let top = match self.state.layout.top_height {
                Some(rows) => Constraint::Length(shrink(rows, area.height)),
                None => Constraint::Fill(3),
            };
            let [top, bottom] =
                Layout::vertical([top, Constraint::Fill(row_weight)])
                    .areas(area);
            // The two touching border rows between the halves; either can
            // be dragged.
            self.state.hsplit_divider = Some(Rect::new(
                area.x,
                top.y.saturating_add(top.height).saturating_sub(1),
                area.width,
                2,
            ));
            self.render_pane(frame, top, FocusedPane::Results);
            bottom
        } else {
            area
        };
        self.state.row_area = Some(row_area);

        let mut constraints: Vec<Constraint> = row_panes
            .iter()
            .map(|(pane, weight)| {
                let dragged = match *pane {
                    FocusedPane::Searches => self.state.layout.searches_width,
                    FocusedPane::Downloads => self.state.layout.downloads_width,
                    FocusedPane::Results => None,
                };
                match dragged {
                    Some(cols) => {
                        Constraint::Length(shrink(cols, row_area.width))
                    }
                    None => Constraint::Fill(*weight),
                }
            })
            .collect();
        constraints.push(Constraint::Fill(6)); // Info
        let chunks = Layout::horizontal(constraints).split(row_area);
        for (chunk, (pane, _)) in chunks.iter().zip(&row_panes) {
            // The seam at this pane's right edge: its border column and the
            // next pane's, both draggable.
            self.state.vsplit_dividers.push((
                *pane,
                Rect::new(
                    chunk.x.saturating_add(chunk.width).saturating_sub(1),
                    row_area.y,
                    2,
                    row_area.height,
                ),
            ));
            self.render_pane(frame, *chunk, *pane);
        }
        self.state.info_pane_area = Some(chunks[row_panes.len()]);
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
                &self.state.downloads,
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
        use crate::models::{SettingsMode, SettingsRow};
        let Some(settings) = self.state.settings.as_ref() else {
            return;
        };
        let area = centered_rect(70, 60, frame.area());
        frame.render_widget(ratatui::widgets::Clear, area);

        let selected = settings.selected_row();
        let mut lines: Vec<Line> = Vec::new();
        let entry = |on: bool, label: &str, value: String| {
            Line::from(vec![
                Span::styled(if on { "> " } else { "  " }, accent_style()),
                Span::styled(label.to_string(), dimmed_style()),
                Span::styled(value, primary_style()),
            ])
        };
        let typing = |label: &str, value: String| {
            Line::from(vec![
                Span::styled("> ", accent_style()),
                Span::styled(label.to_string(), dimmed_style()),
                Span::styled(value, primary_style()),
                Span::styled(GLYPH_CURSOR, accent_style()),
            ])
        };
        let section = |name: &str| Line::styled(name.to_string(), info_style());

        // One column of labels, padded here rather than by hand in each
        // string.
        let field = |label: &str, value: String| {
            entry(false, &format!("  {label:<14}"), value)
        };
        let account = &settings.account;
        lines.push(section("Account"));
        lines.push(field("Signed in as", account.username.clone()));
        lines.push(field("Sharing", account.shares.clone()));
        lines.push(field(
            "Reachable on",
            account.listen_port.map_or_else(
                || "not accepting connections".to_string(),
                |port| format!("port {port}"),
            ),
        ));
        if let Some(daemon) = &account.daemon {
            lines.push(field("Daemon", daemon.clone()));
        }
        lines.push(Line::from(""));

        lines.push(match settings.mode {
            SettingsMode::NewPassword => {
                typing("New password: ", mask(&settings.input))
            }
            SettingsMode::RepeatPassword(_) => {
                typing("Repeat it: ", mask(&settings.input))
            }
            _ => entry(
                selected == SettingsRow::ChangePassword,
                "Change password…",
                String::new(),
            ),
        });
        if settings.mode == SettingsMode::ConfirmingLogout {
            lines.push(entry(
                true,
                &format!("Log out of {}? ", account.username),
                "y / n".to_string(),
            ));
        } else if settings.can_log_out() {
            lines.push(entry(
                selected == SettingsRow::Logout,
                "Log out…",
                String::new(),
            ));
        }
        lines.push(Line::from(""));

        lines.push(section("Folders"));
        lines.push(if settings.mode == SettingsMode::EditingDownloadDir {
            typing("Download folder: ", settings.input.clone())
        } else {
            entry(
                selected == SettingsRow::DownloadDir,
                "Download folder: ",
                settings.download_dir.clone(),
            )
        });
        lines.push(Line::styled(
            format!("Shared folders ({}):", settings.share_dirs.len()),
            dimmed_style(),
        ));
        for (i, dir) in settings.share_dirs.iter().enumerate() {
            lines.push(entry(
                selected == SettingsRow::Share(i),
                "",
                dir.clone(),
            ));
        }
        if settings.share_dirs.is_empty() {
            lines.push(Line::styled(
                "  (nothing shared — press 'a' to add a folder)",
                dimmed_style(),
            ));
        }
        if settings.mode == SettingsMode::AddingShare {
            lines.push(typing("  Add share: ", settings.input.clone()));
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

/// A dragged pane size kept inside the space the terminal now offers: never
/// below [`MIN_PANE`], and never so big that what shares the split with it
/// drops below `MIN_PANE` too.
pub(super) fn shrink(dragged: u16, total: u16) -> u16 {
    let max = total.saturating_sub(MIN_PANE).max(MIN_PANE);
    dragged.max(MIN_PANE).min(max)
}

/// The smallest pane a drag can leave anyone: a border and a row or column
/// of content each side.
pub(super) const MIN_PANE: u16 = 3;
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
