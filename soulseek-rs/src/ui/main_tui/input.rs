use super::MainTui;
use crate::models::{CommandBarMode, FocusedPane};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;
use ratatui::widgets::TableState;

impl MainTui {
    pub(super) fn handle_key_event(&mut self, key: KeyEvent) {
        // Command bar takes priority
        if self.state.command_bar_active {
            return self.handle_command_bar_input(key);
        }

        // Chat popup takes over navigation while open.
        if self.state.show_messages {
            return self.handle_chat_input(key);
        }

        // An open popup owns the keyboard, so these come before pane routing.
        if self.state.show_browse {
            return self.handle_browse_input(key);
        }
        if self.state.show_rooms {
            return self.handle_rooms_input(key);
        }
        if self.state.settings.is_some() {
            return self.handle_settings_input(key);
        }

        // Filter mode in Results pane
        if self.state.results_is_filtering
            && self.state.focused_pane == FocusedPane::Results
        {
            return self.handle_filter_input(key);
        }

        // Global shortcuts
        match key.code {
            KeyCode::Char('q') => {
                self.state.should_exit = true;
                return;
            }
            KeyCode::Char('1') => {
                self.state.focused_pane = FocusedPane::Searches;
                return;
            }
            KeyCode::Char('2') => {
                self.state.focused_pane = FocusedPane::Results;
                return;
            }
            KeyCode::Char('3') => {
                self.state.focused_pane = FocusedPane::Downloads;
                return;
            }
            KeyCode::Char('s') => {
                self.state.command_bar_active = true;
                self.state.command_bar_mode = CommandBarMode::Search;
                self.state.command_bar_input.clear();
                self.state.command_bar_cursor_position = 0;
                return;
            }
            KeyCode::Char('m') => {
                self.state.command_bar_active = true;
                self.state.command_bar_mode = CommandBarMode::Message;
                self.state.command_bar_input.clear();
                self.state.command_bar_cursor_position = 0;
                return;
            }
            KeyCode::Char('i') => {
                self.state.show_messages = true;
                self.state.unread_messages = 0;
                return;
            }
            // Chat rooms. In the Downloads pane `c` clears finished downloads,
            // so only open chat from the other panes (like `b` is contextual).
            KeyCode::Char('c')
                if self.state.focused_pane != FocusedPane::Downloads =>
            {
                self.start_rooms();
                return;
            }
            KeyCode::Char('o') => {
                self.open_settings();
                return;
            }
            KeyCode::Char('b') => {
                // From a highlighted search result, browse its owner directly;
                // otherwise prompt for a username.
                if self.state.focused_pane == FocusedPane::Results
                    && let Some(owner) = self.highlighted_result_owner()
                {
                    self.start_browse(owner);
                } else {
                    self.state.command_bar_active = true;
                    self.state.command_bar_mode = CommandBarMode::Browse;
                    self.state.command_bar_input.clear();
                    self.state.command_bar_cursor_position = 0;
                }
                return;
            }
            _ => {}
        }

        // Pane-specific shortcuts
        match self.state.focused_pane {
            FocusedPane::Searches => self.handle_searches_input(key),
            FocusedPane::Results => self.handle_results_input(key),
            FocusedPane::Downloads => self.handle_downloads_input(key),
        }
    }

    /// Chat popup: composing captures typing, otherwise these are navigation
    /// keys over the conversation list.
    fn handle_chat_input(&mut self, key: KeyEvent) {
        if self.state.chat_composing {
            match key.code {
                KeyCode::Enter => self.send_chat_message(),
                KeyCode::Esc => {
                    self.state.chat_composing = false;
                    self.state.chat_input.clear();
                }
                KeyCode::Backspace => {
                    self.state.chat_input.pop();
                }
                KeyCode::Char(c)
                    if !key.modifiers.intersects(
                        KeyModifiers::CONTROL | KeyModifiers::ALT,
                    ) =>
                {
                    self.state.chat_input.push(c);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('i' | 'q') | KeyCode::Esc => {
                self.state.show_messages = false;
            }
            // Move through the conversation list on the right.
            KeyCode::Tab | KeyCode::Down | KeyCode::Char('j') => {
                self.state.cycle_chat_peer(true);
            }
            KeyCode::BackTab | KeyCode::Up | KeyCode::Char('k') => {
                self.state.cycle_chat_peer(false);
            }
            KeyCode::Char('m') => {
                self.state.show_messages = false;
                self.state.command_bar_active = true;
                self.state.command_bar_mode = CommandBarMode::Message;
                self.state.command_bar_input.clear();
                self.state.command_bar_cursor_position = 0;
            }
            KeyCode::Enter if self.state.active_chat_peer().is_some() => {
                self.state.chat_composing = true;
            }
            _ => {}
        }
    }

    fn handle_command_bar_input(&mut self, key: KeyEvent) {
        self.state.command_bar_cursor_position = self
            .state
            .command_bar_input
            .floor_char_boundary(self.state.command_bar_cursor_position);

        match key.code {
            KeyCode::Enter => {
                let input = self.state.command_bar_input.trim().to_string();
                if !input.is_empty() {
                    match self.state.command_bar_mode {
                        CommandBarMode::Search => self.start_search(input),
                        CommandBarMode::Message => {
                            self.send_message_from_input(&input);
                        }
                        CommandBarMode::Browse => self.start_browse(input),
                    }
                }
                self.state.command_bar_active = false;
                self.state.command_bar_input.clear();
                self.state.command_bar_cursor_position = 0;
            }
            KeyCode::Esc => {
                self.state.command_bar_active = false;
                self.state.command_bar_input.clear();
                self.state.command_bar_cursor_position = 0;
            }
            KeyCode::Backspace => {
                let cursor_position = self.state.command_bar_cursor_position;
                if cursor_position > 0 {
                    let previous_position = self
                        .state
                        .command_bar_input
                        .floor_char_boundary(cursor_position.saturating_sub(1));
                    self.state
                        .command_bar_input
                        .drain(previous_position..cursor_position);
                    self.state.command_bar_cursor_position = previous_position;
                }
            }
            KeyCode::Delete => {
                let cursor_position = self.state.command_bar_cursor_position;
                if cursor_position < self.state.command_bar_input.len() {
                    let next_position = self
                        .state
                        .command_bar_input
                        .ceil_char_boundary(cursor_position + 1);
                    self.state
                        .command_bar_input
                        .drain(cursor_position..next_position);
                }
            }
            KeyCode::Left => {
                let previous =
                    self.state.command_bar_cursor_position.saturating_sub(1);
                self.state.command_bar_cursor_position =
                    self.state.command_bar_input.floor_char_boundary(previous);
            }
            KeyCode::Right => {
                let input = &self.state.command_bar_input;
                let next = self.state.command_bar_cursor_position + 1;
                self.state.command_bar_cursor_position =
                    input.ceil_char_boundary(next.min(input.len()));
            }
            KeyCode::Home => {
                self.state.command_bar_cursor_position = 0;
            }
            KeyCode::End => {
                self.state.command_bar_cursor_position =
                    self.state.command_bar_input.len();
            }
            KeyCode::Char('a')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.state.command_bar_cursor_position = 0;
            }
            KeyCode::Char('e')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.state.command_bar_cursor_position =
                    self.state.command_bar_input.len();
            }
            KeyCode::Char('u')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.state.command_bar_input.clear();
                self.state.command_bar_cursor_position = 0;
            }
            KeyCode::Char(c)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL
                        | KeyModifiers::ALT
                        | KeyModifiers::SUPER
                        | KeyModifiers::HYPER
                        | KeyModifiers::META,
                ) =>
            {
                let cursor_position = self.state.command_bar_cursor_position;
                self.state.command_bar_input.insert(cursor_position, c);
                self.state.command_bar_cursor_position =
                    cursor_position + c.len_utf8();
            }
            _ => {}
        }
    }

    fn handle_filter_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.state.results_is_filtering = false;
                self.state.results_filter_query.clear();
                self.state.results_filtered_items =
                    self.state.results_items.clone();
                self.state.results_filtered_indices =
                    (0..self.state.results_items.len()).collect();
            }
            KeyCode::Char(c) => {
                self.state.results_filter_query.push(c);
                self.apply_filter();
            }
            KeyCode::Backspace => {
                self.state.results_filter_query.pop();
                self.apply_filter();
            }
            // Confirm the filter: leave typing mode but keep the query, so
            // the normal Results keys (j/k, space, enter) act on the
            // filtered list.
            KeyCode::Enter => {
                self.state.results_is_filtering = false;
            }
            KeyCode::Up => {
                cycle(
                    &mut self.state.results_table_state,
                    self.state.results_filtered_items.len(),
                    false,
                );
            }
            KeyCode::Down => {
                cycle(
                    &mut self.state.results_table_state,
                    self.state.results_filtered_items.len(),
                    true,
                );
            }
            _ => {}
        }
    }

    fn handle_searches_input(&mut self, key: KeyEvent) {
        let rows = self.state.searches.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                cycle(&mut self.state.searches_table_state, rows, false);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                cycle(&mut self.state.searches_table_state, rows, true);
            }
            KeyCode::Enter => {
                if let Some(selected) =
                    self.state.searches_table_state.selected()
                {
                    self.state.selected_search_index = Some(selected);
                    if let Some(search) = self.state.searches.get(selected) {
                        self.state.results_items = search.results.clone();
                        self.state.results_filtered_items =
                            search.results.clone();
                        self.state.results_filtered_indices =
                            (0..search.results.len()).collect();
                        self.state.results_selected_indices.clear();
                        self.state.results_table_state.select(Some(0));
                        self.state.focused_pane = FocusedPane::Results;
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(selected) =
                    self.state.searches_table_state.selected()
                {
                    self.remove_search_at_index(selected);
                }
            }
            KeyCode::Char('C') => {
                self.clear_all_searches();
            }
            _ => {}
        }
    }

    fn handle_results_input(&mut self, key: KeyEvent) {
        let items_count = if self.state.results_filter_query.is_empty() {
            self.state.results_items.len()
        } else {
            self.state.results_filtered_items.len()
        };

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                cycle(&mut self.state.results_table_state, items_count, false);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                cycle(&mut self.state.results_table_state, items_count, true);
            }
            KeyCode::Char(' ') => {
                if let Some(current) = self.state.results_table_state.selected()
                {
                    let actual_index =
                        if self.state.results_filter_query.is_empty() {
                            current
                        } else {
                            self.state.results_filtered_indices[current]
                        };

                    if self
                        .state
                        .results_selected_indices
                        .contains(&actual_index)
                    {
                        self.state
                            .results_selected_indices
                            .remove(&actual_index);
                    } else {
                        self.state
                            .results_selected_indices
                            .insert(actual_index);
                    }
                }
            }
            KeyCode::Char('/') => {
                self.state.results_is_filtering = true;
                self.state.results_filter_query.clear();
            }
            KeyCode::Char('a') => {
                let indices: Vec<usize> =
                    if self.state.results_filter_query.is_empty() {
                        (0..self.state.results_items.len()).collect()
                    } else {
                        self.state.results_filtered_indices.clone()
                    };
                self.state.results_selected_indices.extend(indices);
            }
            KeyCode::Char('A') => {
                self.state.results_selected_indices.clear();
            }
            KeyCode::Enter => {
                self.queue_selected_downloads();
            }
            _ => {}
        }
    }

    fn handle_downloads_input(&mut self, key: KeyEvent) {
        // The pane lists downloads first, then uploads; navigation spans both.
        let rows = self.state.downloads.len() + self.state.uploads.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                cycle(&mut self.state.downloads_table_state, rows, false);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                cycle(&mut self.state.downloads_table_state, rows, true);
            }
            KeyCode::Char('x') => {
                self.cancel_selected_upload();
            }
            KeyCode::Char('p') => {
                self.toggle_selected_download_pause();
            }
            KeyCode::Char('d') => {
                self.remove_selected_download();
            }
            KeyCode::Char('r') => {
                self.retry_selected_download();
            }
            KeyCode::Char('c') => {
                self.clear_finished_downloads();
            }
            _ => {}
        }
    }

    pub(super) fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }

        let clicked = Position::new(mouse.column, mouse.row);
        let panes = [
            (self.state.searches_pane_area, FocusedPane::Searches),
            (self.state.results_pane_area, FocusedPane::Results),
            (self.state.downloads_pane_area, FocusedPane::Downloads),
        ];
        for (area, pane) in panes {
            if area.is_some_and(|area| area.contains(clicked)) {
                self.state.focused_pane = pane;
                return;
            }
        }
    }
}

/// Move a table's selection one row, wrapping at either end.
fn cycle(table: &mut TableState, len: usize, forward: bool) {
    if len == 0 {
        return;
    }
    let current = table.selected().unwrap_or(0);
    let next = if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    };
    table.select(Some(next));
}
