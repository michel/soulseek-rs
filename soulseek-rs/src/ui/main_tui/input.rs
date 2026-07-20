use super::MainTui;
use crate::models::{CommandBarMode, FocusedPane};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

impl MainTui {
    pub(super) fn handle_key_event(&mut self, key: KeyEvent) {
        // Command bar takes priority
        if self.state.command_bar_active {
            return self.handle_command_bar_input(key);
        }

        // Messages popup: any of i/Esc/q closes it.
        if self.state.show_messages {
            if matches!(key.code, KeyCode::Char('i' | 'q') | KeyCode::Esc) {
                self.state.show_messages = false;
            }
            return;
        }

        // Browse popup takes over navigation while open.
        if self.state.show_browse {
            return self.handle_browse_input(key);
        }

        // Rooms popup takes over navigation while open.
        if self.state.show_rooms {
            return self.handle_rooms_input(key);
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

    fn handle_command_bar_input(&mut self, key: KeyEvent) {
        self.state.command_bar_cursor_position = clamp_cursor_to_char_boundary(
            &self.state.command_bar_input,
            self.state.command_bar_cursor_position,
        );

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
                    let previous_position = previous_char_boundary(
                        &self.state.command_bar_input,
                        cursor_position,
                    );
                    self.state
                        .command_bar_input
                        .drain(previous_position..cursor_position);
                    self.state.command_bar_cursor_position = previous_position;
                }
            }
            KeyCode::Delete => {
                let cursor_position = self.state.command_bar_cursor_position;
                if cursor_position < self.state.command_bar_input.len() {
                    let next_position = next_char_boundary(
                        &self.state.command_bar_input,
                        cursor_position,
                    );
                    self.state
                        .command_bar_input
                        .drain(cursor_position..next_position);
                }
            }
            KeyCode::Left => {
                self.state.command_bar_cursor_position = previous_char_boundary(
                    &self.state.command_bar_input,
                    self.state.command_bar_cursor_position,
                );
            }
            KeyCode::Right => {
                self.state.command_bar_cursor_position = next_char_boundary(
                    &self.state.command_bar_input,
                    self.state.command_bar_cursor_position,
                );
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
            _ => {}
        }
    }

    fn handle_searches_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k')
                if !self.state.searches.is_empty() =>
            {
                let current =
                    self.state.searches_table_state.selected().unwrap_or(0);
                let new = if current == 0 {
                    self.state.searches.len() - 1
                } else {
                    current - 1
                };
                self.state.searches_table_state.select(Some(new));
            }
            KeyCode::Down | KeyCode::Char('j')
                if !self.state.searches.is_empty() =>
            {
                let current =
                    self.state.searches_table_state.selected().unwrap_or(0);
                let new = (current + 1) % self.state.searches.len();
                self.state.searches_table_state.select(Some(new));
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
            KeyCode::Up | KeyCode::Char('k') if items_count > 0 => {
                let current =
                    self.state.results_table_state.selected().unwrap_or(0);
                let new = if current == 0 {
                    items_count - 1
                } else {
                    current - 1
                };
                self.state.results_table_state.select(Some(new));
            }
            KeyCode::Down | KeyCode::Char('j') if items_count > 0 => {
                let current =
                    self.state.results_table_state.selected().unwrap_or(0);
                let new = (current + 1) % items_count;
                self.state.results_table_state.select(Some(new));
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
        match key.code {
            KeyCode::Up | KeyCode::Char('k')
                if !self.state.downloads.is_empty() =>
            {
                let current =
                    self.state.downloads_table_state.selected().unwrap_or(0);
                let new = if current == 0 {
                    self.state.downloads.len() - 1
                } else {
                    current - 1
                };
                self.state.downloads_table_state.select(Some(new));
            }
            KeyCode::Down | KeyCode::Char('j')
                if !self.state.downloads.is_empty() =>
            {
                let current =
                    self.state.downloads_table_state.selected().unwrap_or(0);
                let new = (current + 1) % self.state.downloads.len();
                self.state.downloads_table_state.select(Some(new));
            }
            KeyCode::Char('p') => {
                self.toggle_selected_download_pause();
            }
            KeyCode::Char('d') => {
                self.remove_selected_queued_download();
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
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            let (col, row) = (mouse.column, mouse.row);

            // Check if click is within searches pane
            if let Some(area) = self.state.searches_pane_area
                && col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.state.focused_pane = FocusedPane::Searches;
                return;
            }

            // Check if click is within results pane
            if let Some(area) = self.state.results_pane_area
                && col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.state.focused_pane = FocusedPane::Results;
                return;
            }

            // Check if click is within downloads pane
            if let Some(area) = self.state.downloads_pane_area
                && col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                self.state.focused_pane = FocusedPane::Downloads;
            }
        }
    }
}

pub(super) const fn clamp_cursor_to_char_boundary(
    input: &str,
    cursor_position: usize,
) -> usize {
    if cursor_position >= input.len() {
        return input.len();
    }

    let mut cursor_position = cursor_position;
    while cursor_position > 0 && !input.is_char_boundary(cursor_position) {
        cursor_position -= 1;
    }
    cursor_position
}

fn previous_char_boundary(input: &str, cursor_position: usize) -> usize {
    let cursor_position = clamp_cursor_to_char_boundary(input, cursor_position);

    input[..cursor_position]
        .char_indices()
        .last()
        .map_or(0, |(index, _)| index)
}

fn next_char_boundary(input: &str, cursor_position: usize) -> usize {
    let cursor_position = clamp_cursor_to_char_boundary(input, cursor_position);

    if cursor_position >= input.len() {
        return input.len();
    }

    cursor_position
        + input[cursor_position..]
            .chars()
            .next()
            .map_or(0, char::len_utf8)
}
