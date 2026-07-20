use super::MainTui;
use crate::models::{CommandBarMode, RoomsView};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl MainTui {
    /// Open the chat-rooms popup and refresh the room list. If rooms are
    /// already open, jump straight to the chat view; otherwise show the list.
    pub(super) fn start_rooms(&mut self) {
        let _ = self.client.request_room_list();
        self.state.show_rooms = true;
        if self.state.rooms.open.is_empty() {
            self.state.rooms.view = RoomsView::List;
        } else {
            self.state.rooms.view = RoomsView::Chat;
            self.state.rooms.mark_active_read();
        }
    }

    pub(super) fn handle_rooms_input(&mut self, key: KeyEvent) {
        // Composing a message captures typing.
        if self.state.rooms.composing {
            self.handle_room_compose_input(key);
            return;
        }
        // Filtering the room list captures typing.
        if self.state.rooms.view == RoomsView::List
            && self.state.rooms.list_is_filtering
        {
            self.handle_room_filter_input(key);
            return;
        }

        match self.state.rooms.view {
            RoomsView::List => self.handle_rooms_list_input(key),
            RoomsView::Chat => self.handle_rooms_chat_input(key),
        }
    }

    fn handle_rooms_list_input(&mut self, key: KeyEvent) {
        let len = self.state.rooms.filtered_rooms().len();
        match key.code {
            // Esc peels back one level: clear a lingering filter first (as the
            // title's "Esc: clear" promises), otherwise close the popup.
            KeyCode::Esc if !self.state.rooms.list_filter.is_empty() => {
                self.state.rooms.list_filter.clear();
                self.state.rooms.list_selected = 0;
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.show_rooms = false;
            }
            KeyCode::Char('/') => {
                self.state.rooms.list_is_filtering = true;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.rooms.list_selected =
                    self.state.rooms.list_selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') if len > 0 => {
                self.state.rooms.list_selected =
                    (self.state.rooms.list_selected + 1).min(len - 1);
            }
            KeyCode::Enter => self.join_selected_room(),
            KeyCode::Tab if !self.state.rooms.open.is_empty() => {
                self.state.rooms.view = RoomsView::Chat;
                self.state.rooms.mark_active_read();
            }
            _ => {}
        }
    }

    fn handle_room_filter_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.state.rooms.list_is_filtering = false;
                self.state.rooms.list_filter.clear();
                self.state.rooms.list_selected = 0;
            }
            KeyCode::Enter => {
                self.state.rooms.list_is_filtering = false;
                self.join_selected_room();
            }
            KeyCode::Char(c) => {
                self.state.rooms.list_filter.push(c);
                self.state.rooms.list_selected = 0;
            }
            KeyCode::Backspace => {
                self.state.rooms.list_filter.pop();
                self.state.rooms.list_selected = 0;
            }
            _ => {}
        }
    }

    fn handle_rooms_chat_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.state.show_rooms = false,
            KeyCode::Esc | KeyCode::Char('l') => {
                self.state.rooms.view = RoomsView::List;
            }
            KeyCode::Tab => self.state.rooms.next_tab(),
            KeyCode::BackTab => self.state.rooms.prev_tab(),
            KeyCode::Char('x') => self.leave_active_room(),
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.rooms.select_user_up();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.rooms.select_user_down();
            }
            // Act on the highlighted member of the room.
            KeyCode::Char('b') => self.browse_selected_room_user(),
            KeyCode::Char('m') => self.message_selected_room_user(),
            KeyCode::Enter if self.state.rooms.active_room().is_some() => {
                self.state.rooms.composing = true;
            }
            _ => {}
        }
    }

    /// Browse the shares of the member highlighted in the active room. Closes
    /// the rooms popup so the browse tree is the sole overlay.
    fn browse_selected_room_user(&mut self) {
        if let Some(user) = self.state.rooms.selected_user() {
            self.state.show_rooms = false;
            self.start_browse(user);
        }
    }

    /// Compose a private message to the member highlighted in the active room.
    /// Closes the rooms popup and opens the message command bar pre-filled with
    /// the recipient.
    fn message_selected_room_user(&mut self) {
        if let Some(user) = self.state.rooms.selected_user() {
            self.state.show_rooms = false;
            self.state.command_bar_active = true;
            self.state.command_bar_mode = CommandBarMode::Message;
            self.state.command_bar_input = format!("{user} ");
            self.state.command_bar_cursor_position =
                self.state.command_bar_input.len();
        }
    }

    fn handle_room_compose_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.send_room_message(),
            KeyCode::Esc => {
                self.state.rooms.composing = false;
                if let Some(room) =
                    self.state.rooms.open.get_mut(self.state.rooms.active)
                {
                    room.input.clear();
                }
            }
            KeyCode::Backspace => {
                if let Some(room) =
                    self.state.rooms.open.get_mut(self.state.rooms.active)
                {
                    room.input.pop();
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(room) =
                    self.state.rooms.open.get_mut(self.state.rooms.active)
                {
                    room.input.push(c);
                }
            }
            _ => {}
        }
    }

    /// Join (or focus) the room highlighted in the list view.
    fn join_selected_room(&mut self) {
        if let Some(name) = self.state.rooms.selected_room_name() {
            let newly_opened = self.state.rooms.focus_or_open(&name);
            if newly_opened && let Err(e) = self.client.join_room(&name) {
                soulseek_rs::warn!("Failed to join {name}: {e}");
            }
        }
    }

    /// Leave the active room and close its tab.
    fn leave_active_room(&mut self) {
        if let Some(name) = self.state.rooms.close_active()
            && let Err(e) = self.client.leave_room(&name)
        {
            soulseek_rs::warn!("Failed to leave {name}: {e}");
        }
    }

    /// Send the active room's compose buffer. The server echoes the message
    /// back as a RoomEvent, which is what actually renders it in the log.
    fn send_room_message(&mut self) {
        let active = self.state.rooms.active;
        let (room, text) = match self.state.rooms.open.get(active) {
            Some(room) if !room.input.trim().is_empty() => {
                (room.name.clone(), room.input.trim().to_string())
            }
            _ => {
                self.state.rooms.composing = false;
                return;
            }
        };
        if let Err(e) = self.client.say_in_room(&room, &text) {
            soulseek_rs::warn!("Failed to say in {room}: {e}");
        }
        if let Some(room) = self.state.rooms.open.get_mut(active) {
            room.input.clear();
        }
        self.state.rooms.composing = false;
    }

    /// Drain chat-room events into the rooms state, tracking unread badges.
    pub(super) fn poll_room_events(&mut self) {
        let viewing = if self.state.show_rooms
            && self.state.rooms.view == RoomsView::Chat
        {
            self.state.rooms.active_room().map(|r| r.name.clone())
        } else {
            None
        };
        for event in self.client.take_room_events() {
            self.state.rooms.apply_event(event, viewing.as_deref());
        }
    }
}
