use super::MainTui;
use super::render::{MIN_PANE, shrink};
use crate::models::{CommandBarMode, FocusedPane, LogView, PaneLayout};
use crate::ui::page_of;
use crate::ui::panes::{
    InfoSubject, name_end_offset, query_end_offset, selected_transfer,
    transfer_name_end_offset, upload_display_name,
};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;
use ratatui::widgets::TableState;

const NAME_SCROLL_STEP: isize = 8;

impl MainTui {
    /// One key press, routed to whatever owns the keyboard right now.
    pub fn handle_key_event(&mut self, key: KeyEvent) {
        // Command bar takes priority
        if self.state.command_bar_active {
            return self.handle_command_bar_input(key);
        }

        // The keys list closes on the key that opened it, or the usual two,
        // and scrolls like any log for a terminal too short to show it all.
        if self.state.show_help {
            let view = &mut self.state.help_view;
            match key.code {
                KeyCode::Char('?' | 'q') | KeyCode::Esc => {
                    self.state.show_help = false;
                }
                KeyCode::Down | KeyCode::Char('j') => view.scroll(1),
                KeyCode::Up | KeyCode::Char('k') => view.scroll(-1),
                _ => {
                    scroll_log(view, key);
                }
            }
            return;
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

        // Control combinations only ever page a list, so they never reach
        // the letter keys below: ctrl-d is not d.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            self.navigate_focused_list(key);
            return;
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
            KeyCode::Char('?') => {
                self.state.show_help = true;
                self.state.help_view.to_oldest();
                return;
            }
            // A pane's number is its place in the legend order.
            KeyCode::Char(digit @ '1'..='3') => {
                let index = usize::from(digit as u8 - b'1');
                self.state.focus_pane(FocusedPane::ALL[index]);
                return;
            }
            KeyCode::Tab => {
                self.state.cycle_focus(true);
                return;
            }
            KeyCode::BackTab => {
                self.state.cycle_focus(false);
                return;
            }
            KeyCode::Char('z') => {
                self.state.toggle_zoom();
                return;
            }
            KeyCode::Char('w') => {
                self.state.hide_focused_pane();
                return;
            }
            KeyCode::Char('W') => {
                // The universal undo for the window: every pane back, zoom
                // off, and any mouse-dragged pane sizes back to the default
                // proportions — they all live in the layout.
                self.state.layout = PaneLayout::default();
                return;
            }
            KeyCode::Esc if self.state.layout.zoomed => {
                self.state.layout.zoomed = false;
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
                } else if self.state.focused_pane == FocusedPane::Downloads
                    && let Some(user) = self.selected_transfer_user()
                {
                    self.start_browse(user);
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

        // Typing a filter captures the keys that edit it; the rest still
        // move around.
        if self.state.chat_filtering
            && let Some(edit) = edit_filter(&mut self.state.chat_filter, key)
        {
            if edit != FilterEdit::Changed {
                self.state.chat_filtering = false;
            }
            if edit != FilterEdit::Kept {
                self.state.chat_view.to_newest();
            }
            return;
        }

        // Paging keys move through the conversation's history.
        if scroll_log(&mut self.state.chat_view, key) {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return;
        }

        match key.code {
            // Esc peels back one level: a filter first, then the popup.
            KeyCode::Esc if !self.state.chat_filter.is_empty() => {
                self.state.chat_filter.clear();
                self.state.chat_view.to_newest();
            }
            KeyCode::Char('i' | 'q') | KeyCode::Esc => {
                self.state.show_messages = false;
            }
            KeyCode::Char('/') => self.state.chat_filtering = true,
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
        let Some(edit) = edit_filter(&mut self.state.results_filter_query, key)
        else {
            return self.handle_results_input(key);
        };
        // A kept query stays: the normal Results keys act on the filtered
        // list. An empty one shows everything without a rebuild.
        if edit == FilterEdit::Changed {
            self.apply_filter();
        } else {
            self.state.results_is_filtering = false;
        }
    }

    /// The list the focus is on and how far one page moves it.
    fn navigate_focused_list(&mut self, key: KeyEvent) -> bool {
        let focused = self.state.focused_pane;
        let page = self.page_size();
        let (table, len) = match focused {
            FocusedPane::Searches => (
                &mut self.state.searches_table_state,
                self.state.searches.len(),
            ),
            FocusedPane::Results => {
                let len = self.visible_results().len();
                (&mut self.state.results_table_state, len)
            }
            FocusedPane::Downloads => (
                &mut self.state.downloads_table_state,
                self.state.downloads.len() + self.state.uploads.len(),
            ),
        };
        navigate_list(key, table, len, page)
    }

    /// Rows a popup's list shows at once: its inner height less the `chrome`
    /// rows above the list, a tab bar or a header.
    pub(super) fn popup_page(&self, chrome: u16) -> usize {
        page_of(self.state.popup_area, chrome)
    }

    /// Rows the focused pane shows at once: its inner height less the table
    /// header.
    fn page_size(&self) -> usize {
        page_of(self.state.pane_area(self.state.focused_pane), 1)
    }

    fn handle_searches_input(&mut self, key: KeyEvent) {
        if self.navigate_focused_list(key) || self.scroll_focused_name(key) {
            return;
        }
        let selected = self.state.searches_table_state.selected();
        match key.code {
            KeyCode::Enter => {
                if let Some(selected) = selected {
                    self.state.selected_search_index = Some(selected);
                    if let Some(search) = self.state.searches.get(selected) {
                        self.state.results_items = search.results.clone();
                        self.state.results_filtered_items =
                            search.results.clone();
                        self.state.results_filtered_indices =
                            (0..search.results.len()).collect();
                        self.state.results_selected_indices.clear();
                        self.state.results_name_offset = 0;
                        self.state.results_table_state.select(Some(0));
                        self.state.focus_pane(FocusedPane::Results);
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(selected) = selected {
                    self.remove_search_at_index(selected);
                }
            }
            // `s` asks for a query; `S` reuses the highlighted one.
            KeyCode::Char('S') => {
                if let Some(selected) = selected {
                    self.rerun_search_at_index(selected);
                }
            }
            KeyCode::Char('C') => {
                self.clear_all_searches();
            }
            _ => {}
        }
    }

    fn handle_results_input(&mut self, key: KeyEvent) {
        if self.navigate_focused_list(key) || self.scroll_focused_name(key) {
            return;
        }
        match key.code {
            KeyCode::Char(' ') => {
                let index = self
                    .state
                    .results_table_state
                    .selected()
                    .and_then(|row| self.original_index(row));
                if let Some(index) = index
                    && !self.state.results_selected_indices.remove(&index)
                {
                    self.state.results_selected_indices.insert(index);
                }
            }
            KeyCode::Char('/') => self.state.results_is_filtering = true,
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

    /// The keys that scroll the focused list's long column sideways, the
    /// same in every list: a file name in Results and Downloads, the query
    /// in Searches. Says whether `key` was one of them.
    fn scroll_focused_name(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Right | KeyCode::Char('l') => {
                self.scroll_names(NAME_SCROLL_STEP);
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.scroll_names(-NAME_SCROLL_STEP);
            }
            KeyCode::Char('0') => {
                *self.state.name_offset_mut(self.state.focused_pane) = 0;
            }
            KeyCode::Char('$') => {
                let end = self.highlighted_name_end();
                *self.state.name_offset_mut(self.state.focused_pane) = end;
            }
            _ => return false,
        }
        true
    }

    fn scroll_names(&mut self, delta: isize) {
        let end = self.highlighted_name_end();
        let offset = self.state.name_offset_mut(self.state.focused_pane);
        let current = (*offset).min(end);
        *offset = current.saturating_add_signed(delta).min(end);
    }

    /// How far the highlighted row's long column can scroll before its end
    /// is in view. Zero when nothing is highlighted or the pane is not drawn.
    fn highlighted_name_end(&self) -> usize {
        let pane = self.state.focused_pane;
        let Some(area) = self.state.pane_area(pane) else {
            return 0;
        };
        match pane {
            FocusedPane::Searches => self
                .state
                .searches_table_state
                .selected()
                .and_then(|row| self.state.searches.get(row))
                .map_or(0, |search| query_end_offset(&search.query, area)),
            FocusedPane::Results => self
                .highlighted_result()
                .map_or(0, |file| name_end_offset(&file.filename, area)),
            FocusedPane::Downloads => {
                let shown = match selected_transfer(
                    self.state.downloads_table_state.selected(),
                    &self.state.downloads,
                    &self.state.uploads,
                ) {
                    Some(InfoSubject::Download(entry)) => {
                        entry.download.filename.as_str()
                    }
                    Some(InfoSubject::Upload(upload)) => {
                        upload_display_name(upload)
                    }
                    Some(InfoSubject::Result(_)) | None => return 0,
                };
                transfer_name_end_offset(shown, area)
            }
        }
    }

    fn handle_downloads_input(&mut self, key: KeyEvent) {
        // The pane lists downloads first, then uploads; navigation spans both.
        if self.navigate_focused_list(key) || self.scroll_focused_name(key) {
            return;
        }
        match key.code {
            KeyCode::Char('x') => {
                self.cancel_selected_transfer();
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
            KeyCode::Char('C') => {
                self.clear_all_transfers();
            }
            _ => {}
        }
    }

    pub(super) fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let at = Position::new(mouse.column, mouse.row);
                // A divider wins over the panes it separates; grabbing one
                // starts a drag, and the click alone already resizes.
                if let Some(drag) = self.grabbed_divider(at) {
                    self.resize_drag = Some(drag);
                    self.apply_resize(drag, at);
                    return;
                }
                // The areas are from the last draw. A pane hidden since then, in the
                // same batch of events, still has one, and is not there to click.
                let hit = FocusedPane::ALL.into_iter().find(|pane| {
                    self.state.layout.is_visible(*pane)
                        && self
                            .state
                            .pane_area(*pane)
                            .is_some_and(|area| area.contains(at))
                });
                if let Some(pane) = hit {
                    self.state.focused_pane = pane;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(drag) = self.resize_drag {
                    self.apply_resize(
                        drag,
                        Position::new(mouse.column, mouse.row),
                    );
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.resize_drag = None;
            }
            _ => {}
        }
    }

    /// Which dragged divider, if any, the pointer is on. Nothing is grabbable
    /// while a popup or a zoom covers the dividers.
    fn grabbed_divider(&self, at: Position) -> Option<ResizeDrag> {
        if self.state.layout.zoomed
            || self.state.show_help
            || self.state.show_messages
            || self.state.show_browse
            || self.state.show_rooms
            || self.state.settings.is_some()
        {
            return None;
        }
        if self
            .state
            .hsplit_divider
            .is_some_and(|rect| rect.contains(at))
        {
            return Some(ResizeDrag::Rows);
        }
        self.state
            .vsplit_dividers
            .iter()
            .find(|(_, rect)| rect.contains(at))
            .map(|(pane, _)| ResizeDrag::Columns(*pane))
    }

    /// Move the divider under the pointer to where the pointer is, keeping
    /// every pane at least [`MIN_PANE`] cells.
    fn apply_resize(&mut self, drag: ResizeDrag, at: Position) {
        match drag {
            ResizeDrag::Rows => {
                let Some(area) = self.state.content_area else {
                    return;
                };
                // The dragged row is the pane's last row, so the height is
                // everything from the top of the content down to it.
                let height = at.y.saturating_sub(area.y).saturating_add(1);
                self.state.layout.top_height =
                    Some(shrink(height, area.height));
            }
            ResizeDrag::Columns(pane) => {
                let Some(row) = self.state.row_area else {
                    return;
                };
                let left = if pane == FocusedPane::Downloads
                    && self.state.layout.is_visible(FocusedPane::Searches)
                {
                    // Downloads starts where Searches ends.
                    self.state
                        .searches_pane_area
                        .map_or(row.x, |area| area.x.saturating_add(area.width))
                } else {
                    row.x
                };
                let width = at.x.saturating_sub(left).saturating_add(1);
                // Info shares the row, so the drag may not crowd it (and any
                // other visible pane) out of `MIN_PANE` cells.
                let others = MIN_PANE
                    * u16::from(
                        self.state.layout.is_visible(FocusedPane::Searches)
                            && pane == FocusedPane::Downloads,
                    )
                    + MIN_PANE;
                let max = row.width.saturating_sub(others).max(MIN_PANE);
                let width = width.max(MIN_PANE).min(max);
                match pane {
                    FocusedPane::Searches => {
                        self.state.layout.searches_width = Some(width);
                    }
                    FocusedPane::Downloads => {
                        self.state.layout.downloads_width = Some(width);
                    }
                    FocusedPane::Results => {}
                }
            }
        }
    }
}

/// What a divider does while the left button is held on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResizeDrag {
    /// The border between Results and the row underneath: sets that row's height.
    Rows,
    /// The seam at the right of the named row pane: gives it that width and
    /// takes it from the panes to its right.
    Columns(FocusedPane),
}

/// What a key did to a filter being typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FilterEdit {
    Changed,
    Kept,
    Cleared,
}

/// Apply `key` to `filter` the way every typed filter takes it: characters
/// and Backspace edit it, Enter keeps it, Esc clears it. A control or alt
/// chord is not typing. `None` when the key was none of those.
pub(super) fn edit_filter(
    filter: &mut String,
    key: KeyEvent,
) -> Option<FilterEdit> {
    match key.code {
        KeyCode::Esc => {
            filter.clear();
            Some(FilterEdit::Cleared)
        }
        KeyCode::Enter => Some(FilterEdit::Kept),
        KeyCode::Backspace => {
            filter.pop();
            Some(FilterEdit::Changed)
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            filter.push(c);
            Some(FilterEdit::Changed)
        }
        _ => None,
    }
}

/// A move over a whole list at once: to either end, or by a number of rows
/// (negative is up).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListJump {
    First,
    Last,
    Rows(isize),
}

/// The keys every list and log answers alike for moving further than a row:
/// the two ends, a page, half a page. `page` is how many rows the list shows.
pub(super) fn list_jump(key: KeyEvent, page: usize) -> Option<ListJump> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let page = isize::try_from(page).unwrap_or(isize::MAX);
    let half = (page / 2).max(1);
    Some(match key.code {
        KeyCode::Home | KeyCode::Char('g') if !ctrl => ListJump::First,
        KeyCode::End | KeyCode::Char('G') if !ctrl => ListJump::Last,
        KeyCode::PageDown => ListJump::Rows(page),
        KeyCode::PageUp => ListJump::Rows(-page),
        KeyCode::Char('f') if ctrl => ListJump::Rows(page),
        KeyCode::Char('b') if ctrl => ListJump::Rows(-page),
        KeyCode::Char('d') if ctrl => ListJump::Rows(half),
        KeyCode::Char('u') if ctrl => ListJump::Rows(-half),
        _ => return None,
    })
}

/// Apply the jump keys to a log, moving by the screenful it last showed.
/// Says whether `key` was one of them.
pub(super) fn scroll_log(view: &mut LogView, key: KeyEvent) -> bool {
    match list_jump(key, view.page()) {
        Some(ListJump::First) => view.to_oldest(),
        Some(ListJump::Last) => view.to_newest(),
        Some(ListJump::Rows(rows)) => view.scroll(rows),
        None => return false,
    }
    true
}

/// `selected` moved by `jump` over a list `len` long, stopping at the ends.
pub(super) fn jumped(selected: usize, len: usize, jump: ListJump) -> usize {
    let last = len.saturating_sub(1);
    match jump {
        ListJump::First => 0,
        ListJump::Last => last,
        ListJump::Rows(rows) => selected.saturating_add_signed(rows).min(last),
    }
}

/// The keys every list answers alike: a row at a time wrapping at either
/// end, and the jumps of [`list_jump`]. Says whether `key` was one of them.
fn navigate_list(
    key: KeyEvent,
    table: &mut TableState,
    len: usize,
    page: usize,
) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Up | KeyCode::Char('k') if !ctrl => cycle(table, len, false),
        KeyCode::Down | KeyCode::Char('j') if !ctrl => cycle(table, len, true),
        _ => {
            let Some(jump) = list_jump(key, page) else {
                return false;
            };
            if len > 0 {
                let current = table.selected().unwrap_or(0);
                table.select(Some(jumped(current, len, jump)));
            }
        }
    }
    true
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
