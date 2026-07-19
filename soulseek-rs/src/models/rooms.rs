use soulseek_rs::types::{RoomEvent, RoomInfo};

/// One line in a room's chat log. A `None` username marks a system line
/// (joins/leaves), rendered dimmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomLine {
    pub username: Option<String>,
    pub text: String,
}

impl RoomLine {
    const fn chat(username: String, text: String) -> Self {
        Self {
            username: Some(username),
            text,
        }
    }
    const fn system(text: String) -> Self {
        Self {
            username: None,
            text,
        }
    }
}

/// A chat room the user has open (a tab).
#[derive(Debug, Clone, Default)]
pub struct OpenRoom {
    pub name: String,
    pub users: Vec<String>,
    pub lines: Vec<RoomLine>,
    /// Unread messages received while this room was not being viewed.
    pub unread: usize,
    /// Per-room compose buffer.
    pub input: String,
}

impl OpenRoom {
    fn new(name: String) -> Self {
        Self {
            name,
            ..Default::default()
        }
    }
}

/// Which sub-view of the rooms popup is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomsView {
    /// The browsable/filterable list of public rooms.
    List,
    /// The open-room tabs and their chat logs.
    Chat,
}

/// All chat-room UI state: the discoverable room list plus the set of open
/// rooms (tabs).
pub struct RoomsState {
    /// Latest public-room snapshot from the server.
    pub available: Vec<RoomInfo>,
    pub list_filter: String,
    pub list_is_filtering: bool,
    /// Selected index into the *filtered* room list.
    pub list_selected: usize,
    /// Rooms the user has joined (tabs).
    pub open: Vec<OpenRoom>,
    /// Index into `open` of the active tab.
    pub active: usize,
    pub view: RoomsView,
    /// Whether the compose input for the active room is capturing keys.
    pub composing: bool,
}

impl Default for RoomsState {
    fn default() -> Self {
        Self::new()
    }
}

impl RoomsState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            available: Vec::new(),
            list_filter: String::new(),
            list_is_filtering: false,
            list_selected: 0,
            open: Vec::new(),
            active: 0,
            view: RoomsView::List,
            composing: false,
        }
    }

    /// The public rooms matching the current filter, busiest first then by name.
    #[must_use]
    pub fn filtered_rooms(&self) -> Vec<RoomInfo> {
        let needle = self.list_filter.to_lowercase();
        let mut rooms: Vec<RoomInfo> = self
            .available
            .iter()
            .filter(|r| {
                needle.is_empty() || r.name.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect();
        rooms.sort_by(|a, b| {
            b.user_count
                .cmp(&a.user_count)
                .then_with(|| a.name.cmp(&b.name))
        });
        rooms
    }

    /// The room name currently highlighted in the list view, if any.
    #[must_use]
    pub fn selected_room_name(&self) -> Option<String> {
        let rooms = self.filtered_rooms();
        rooms.get(self.list_selected).map(|r| r.name.clone())
    }

    #[must_use]
    pub fn open_index(&self, name: &str) -> Option<usize> {
        self.open.iter().position(|r| r.name == name)
    }

    #[must_use]
    pub fn active_room(&self) -> Option<&OpenRoom> {
        self.open.get(self.active)
    }

    /// Open (or focus) a room by name, switching to the Chat view. Returns
    /// `true` if the room was newly opened (so the caller should `join_room`).
    pub fn focus_or_open(&mut self, name: &str) -> bool {
        self.view = RoomsView::Chat;
        if let Some(idx) = self.open_index(name) {
            self.active = idx;
            self.mark_active_read();
            false
        } else {
            self.open.push(OpenRoom::new(name.to_string()));
            self.active = self.open.len() - 1;
            true
        }
    }

    /// Close the active tab, returning its room name (for `leave_room`).
    pub fn close_active(&mut self) -> Option<String> {
        if self.active >= self.open.len() {
            return None;
        }
        let room = self.open.remove(self.active);
        if self.active >= self.open.len() {
            self.active = self.open.len().saturating_sub(1);
        }
        if self.open.is_empty() {
            self.view = RoomsView::List;
        }
        Some(room.name)
    }

    pub fn next_tab(&mut self) {
        if !self.open.is_empty() {
            self.active = (self.active + 1) % self.open.len();
            self.mark_active_read();
        }
    }

    pub fn prev_tab(&mut self) {
        if !self.open.is_empty() {
            self.active = (self.active + self.open.len() - 1) % self.open.len();
            self.mark_active_read();
        }
    }

    /// Clear the unread badge on the active room.
    pub fn mark_active_read(&mut self) {
        if let Some(room) = self.open.get_mut(self.active) {
            room.unread = 0;
        }
    }

    /// Total unread messages across all open rooms (for the closed-popup badge).
    #[must_use]
    pub fn total_unread(&self) -> usize {
        self.open.iter().map(|r| r.unread).sum()
    }

    /// Apply a room event. `viewing` is the room the user is actively looking
    /// at (Chat view open on that tab), whose incoming messages should not
    /// accrue an unread badge.
    pub fn apply_event(&mut self, event: RoomEvent, viewing: Option<&str>) {
        match event {
            RoomEvent::List(rooms) => {
                self.available = rooms;
                let max = self.filtered_rooms().len().saturating_sub(1);
                self.list_selected = self.list_selected.min(max);
            }
            RoomEvent::Joined { room, users } => {
                let idx = self.ensure_open(&room);
                self.open[idx].users = users;
                self.open[idx]
                    .lines
                    .push(RoomLine::system("— joined —".to_string()));
            }
            RoomEvent::Left { room } => {
                if let Some(idx) = self.open_index(&room) {
                    self.open[idx]
                        .lines
                        .push(RoomLine::system("— left —".to_string()));
                }
            }
            RoomEvent::Message {
                room,
                username,
                message,
            } => {
                let idx = self.ensure_open(&room);
                self.open[idx].lines.push(RoomLine::chat(username, message));
                if viewing != Some(room.as_str()) {
                    self.open[idx].unread += 1;
                }
            }
            RoomEvent::UserJoined { room, username } => {
                if let Some(idx) = self.open_index(&room) {
                    if !self.open[idx].users.contains(&username) {
                        self.open[idx].users.push(username.clone());
                    }
                    self.open[idx]
                        .lines
                        .push(RoomLine::system(format!("→ {username}")));
                }
            }
            RoomEvent::UserLeft { room, username } => {
                if let Some(idx) = self.open_index(&room) {
                    self.open[idx].users.retain(|u| u != &username);
                    self.open[idx]
                        .lines
                        .push(RoomLine::system(format!("← {username}")));
                }
            }
        }
    }

    /// Index of the open room `name`, creating an empty tab if necessary.
    fn ensure_open(&mut self, name: &str) -> usize {
        if let Some(idx) = self.open_index(name) {
            idx
        } else {
            self.open.push(OpenRoom::new(name.to_string()));
            self.open.len() - 1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(name: &str, users: u32) -> RoomInfo {
        RoomInfo {
            name: name.to_string(),
            user_count: users,
        }
    }

    #[test]
    fn filtered_rooms_sorts_busiest_first_then_name() {
        let mut state = RoomsState::new();
        state.apply_event(
            RoomEvent::List(vec![
                room("blues", 3),
                room("jazz", 10),
                room("ambient", 10),
            ]),
            None,
        );
        let names: Vec<String> =
            state.filtered_rooms().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["ambient", "jazz", "blues"]);
    }

    #[test]
    fn filter_matches_case_insensitively() {
        let mut state = RoomsState::new();
        state.apply_event(
            RoomEvent::List(vec![room("Jazz", 1), room("Blues", 1)]),
            None,
        );
        state.list_filter = "JAZ".to_string();
        let names: Vec<String> =
            state.filtered_rooms().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["Jazz"]);
    }

    #[test]
    fn joining_creates_a_tab_with_members() {
        let mut state = RoomsState::new();
        state.apply_event(
            RoomEvent::Joined {
                room: "jazz".to_string(),
                users: vec!["alice".to_string(), "bob".to_string()],
            },
            None,
        );
        assert_eq!(state.open.len(), 1);
        assert_eq!(state.open[0].name, "jazz");
        assert_eq!(state.open[0].users, vec!["alice", "bob"]);
    }

    #[test]
    fn message_to_unviewed_room_increments_unread() {
        let mut state = RoomsState::new();
        state.focus_or_open("jazz");
        state.apply_event(
            RoomEvent::Message {
                room: "jazz".to_string(),
                username: "bob".to_string(),
                message: "hi".to_string(),
            },
            None, // not viewing anything
        );
        assert_eq!(state.open[0].unread, 1);
        assert_eq!(state.total_unread(), 1);
    }

    #[test]
    fn message_to_viewed_room_does_not_increment_unread() {
        let mut state = RoomsState::new();
        state.focus_or_open("jazz");
        state.apply_event(
            RoomEvent::Message {
                room: "jazz".to_string(),
                username: "bob".to_string(),
                message: "hi".to_string(),
            },
            Some("jazz"),
        );
        assert_eq!(state.open[0].unread, 0);
        assert_eq!(state.open[0].lines.len(), 1);
    }

    #[test]
    fn focus_or_open_is_idempotent_and_marks_read() {
        let mut state = RoomsState::new();
        assert!(state.focus_or_open("jazz")); // newly opened
        state.open[0].unread = 4;
        assert!(!state.focus_or_open("jazz")); // already open
        assert_eq!(state.open.len(), 1);
        assert_eq!(state.open[0].unread, 0, "focusing clears unread");
    }

    #[test]
    fn closing_active_tab_returns_name_and_adjusts_index() {
        let mut state = RoomsState::new();
        state.focus_or_open("a");
        state.focus_or_open("b");
        assert_eq!(state.active, 1);
        assert_eq!(state.close_active().as_deref(), Some("b"));
        assert_eq!(state.active, 0);
        assert_eq!(state.open.len(), 1);
        assert_eq!(state.close_active().as_deref(), Some("a"));
        assert!(state.open.is_empty());
        assert_eq!(state.view, RoomsView::List);
    }

    #[test]
    fn tab_cycling_wraps_and_marks_read() {
        let mut state = RoomsState::new();
        state.focus_or_open("a");
        state.focus_or_open("b");
        state.open[0].unread = 2;
        state.next_tab(); // 1 -> 0
        assert_eq!(state.active, 0);
        assert_eq!(state.open[0].unread, 0);
        state.prev_tab(); // 0 -> 1
        assert_eq!(state.active, 1);
    }

    #[test]
    fn user_joined_and_left_update_membership() {
        let mut state = RoomsState::new();
        state.apply_event(
            RoomEvent::Joined {
                room: "jazz".to_string(),
                users: vec!["alice".to_string()],
            },
            None,
        );
        state.apply_event(
            RoomEvent::UserJoined {
                room: "jazz".to_string(),
                username: "bob".to_string(),
            },
            None,
        );
        assert_eq!(state.open[0].users, vec!["alice", "bob"]);
        state.apply_event(
            RoomEvent::UserLeft {
                room: "jazz".to_string(),
                username: "alice".to_string(),
            },
            None,
        );
        assert_eq!(state.open[0].users, vec!["bob"]);
    }
}
