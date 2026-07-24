use crate::models::{BrowseTabs, FileDisplayData, RoomsState, SettingsState};
use chrono::{DateTime, Local};
use ratatui::{layout::Rect, widgets::TableState};
use soulseek_rs::{DownloadStatus, types::Download};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc::Receiver, mpsc::Sender};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchStatus {
    Active,
    Completed,
}

pub struct SearchEntry {
    pub query: String,
    pub status: SearchStatus,
    pub results: Vec<FileDisplayData>,
    pub start_time: Instant,
    #[allow(dead_code)]
    pub cancel_flag: Arc<AtomicBool>,
}

pub struct DownloadEntry {
    pub download: Download,
    pub receiver: Option<Receiver<DownloadStatus>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPane {
    Searches,
    Results,
    Downloads,
}

/// What the shared command bar is currently capturing input for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandBarMode {
    Search,
    Message,
    Browse,
}

/// Direction of a private message relative to the local user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageDirection {
    Incoming,
    Outgoing,
}

/// A private message shown in the chat popup.
pub struct ChatMessage {
    pub direction: MessageDirection,
    /// The other party: the sender for incoming, the recipient for outgoing.
    pub peer: String,
    pub text: String,
    /// Local wall-clock time the message was sent or received.
    pub at: DateTime<Local>,
}

#[allow(clippy::struct_excessive_bools)]
pub struct AppState {
    // Searches
    pub searches: Vec<SearchEntry>,
    pub searches_table_state: TableState,
    pub selected_search_index: Option<usize>,

    // Results
    pub results_items: Vec<FileDisplayData>,
    pub results_filtered_items: Vec<FileDisplayData>,
    pub results_filtered_indices: Vec<usize>,
    pub results_table_state: TableState,
    pub results_selected_indices: std::collections::HashSet<usize>,
    pub results_filter_query: String,
    pub results_is_filtering: bool,

    // Downloads
    pub downloads: Vec<DownloadEntry>,
    pub downloads_table_state: TableState,
    pub downloads_receiver_channel:
        Option<Receiver<(Download, Receiver<DownloadStatus>)>>,
    pub downloads_sender_channel:
        Option<Sender<(Download, Receiver<DownloadStatus>)>>,
    pub active_downloads_count: usize,

    // UI State
    pub focused_pane: FocusedPane,
    pub should_exit: bool,
    pub command_bar_active: bool,
    pub command_bar_input: String,
    pub command_bar_cursor_position: usize,
    pub command_bar_mode: CommandBarMode,

    // Private messages
    pub messages: Vec<ChatMessage>,
    pub show_messages: bool,
    /// Incoming private messages received while the inbox was closed.
    pub unread_messages: usize,
    /// Conversation the chat popup is pinned to; unset follows the newest.
    pub chat_peer: Option<String>,
    /// Compose buffer for the active conversation.
    pub chat_input: String,
    pub chat_composing: bool,

    // Browse users' shared files (one tab per user)
    pub browse: BrowseTabs,
    pub show_browse: bool,
    pub browse_table_state: TableState,

    // Chat rooms
    pub rooms: RoomsState,
    pub show_rooms: bool,
    pub rooms_list_table_state: TableState,

    // Settings popup (download folder + share paths)
    pub settings: Option<SettingsState>,

    // Uploads we are serving (refreshed from the client every tick)
    pub uploads: Vec<soulseek_rs::types::UploadInfo>,

    // Pane areas for mouse interaction
    pub searches_pane_area: Option<Rect>,
    pub results_pane_area: Option<Rect>,
    pub downloads_pane_area: Option<Rect>,
}

impl AppState {
    #[must_use]
    pub fn new() -> Self {
        let mut searches_table_state = TableState::default();
        searches_table_state.select(Some(0));

        let mut results_table_state = TableState::default();
        results_table_state.select(Some(0));

        let mut downloads_table_state = TableState::default();
        downloads_table_state.select(Some(0));

        Self {
            searches: Vec::new(),
            searches_table_state,
            selected_search_index: None,

            results_items: Vec::new(),
            results_filtered_items: Vec::new(),
            results_filtered_indices: Vec::new(),
            results_table_state,
            results_selected_indices: std::collections::HashSet::new(),
            results_filter_query: String::new(),
            results_is_filtering: false,

            downloads: Vec::new(),
            downloads_table_state,
            downloads_receiver_channel: None,
            downloads_sender_channel: None,
            active_downloads_count: 0,

            focused_pane: FocusedPane::Searches,
            should_exit: false,
            command_bar_active: false,
            command_bar_input: String::new(),
            command_bar_cursor_position: 0,
            command_bar_mode: CommandBarMode::Search,

            messages: Vec::new(),
            show_messages: false,
            unread_messages: 0,
            chat_peer: None,
            chat_input: String::new(),
            chat_composing: false,

            browse: BrowseTabs::new(),
            show_browse: false,
            browse_table_state: TableState::default(),

            rooms: RoomsState::new(),
            show_rooms: false,
            rooms_list_table_state: TableState::default(),

            settings: None,

            uploads: Vec::new(),

            searches_pane_area: None,
            results_pane_area: None,
            downloads_pane_area: None,
        }
    }

    /// Open the chat popup on a conversation with `peer`, ready to type.
    pub fn open_chat(&mut self, peer: String) {
        self.chat_peer = Some(peer);
        self.chat_input.clear();
        self.chat_composing = true;
        self.show_messages = true;
        self.unread_messages = 0;
    }

    /// The conversation the chat popup shows: the pinned peer, else whoever
    /// was last talked to.
    #[must_use]
    pub fn active_chat_peer(&self) -> Option<&str> {
        self.chat_peer
            .as_deref()
            .or_else(|| self.messages.last().map(|m| m.peer.as_str()))
    }

    /// Conversation partners in first-contact order. ponytail: rescans every
    /// message per frame; an inbox is never big enough to care.
    #[must_use]
    pub fn chat_peers(&self) -> Vec<&str> {
        let mut peers: Vec<&str> = Vec::new();
        for message in &self.messages {
            if !peers.contains(&message.peer.as_str()) {
                peers.push(&message.peer);
            }
        }
        // A conversation opened but not yet written to has no messages.
        if let Some(peer) = self.chat_peer.as_deref()
            && !peers.contains(&peer)
        {
            peers.push(peer);
        }
        peers
    }

    /// Pin the chat popup to the next (or previous) conversation.
    pub fn cycle_chat_peer(&mut self, forward: bool) {
        let peers = self.chat_peers();
        if peers.is_empty() {
            return;
        }
        let current = self
            .active_chat_peer()
            .and_then(|active| peers.iter().position(|p| *p == active))
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % peers.len()
        } else {
            (current + peers.len() - 1) % peers.len()
        };
        let peer = peers[next].to_string();
        self.chat_peer = Some(peer);
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn get_selected_search(&self) -> Option<&SearchEntry> {
        self.selected_search_index
            .and_then(|idx| self.searches.get(idx))
    }

    #[allow(dead_code)]
    pub fn get_selected_search_mut(&mut self) -> Option<&mut SearchEntry> {
        self.selected_search_index
            .and_then(|idx| self.searches.get_mut(idx))
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn say(state: &mut AppState, peer: &str) {
        state.messages.push(ChatMessage {
            direction: MessageDirection::Outgoing,
            peer: peer.to_string(),
            text: "hi".to_string(),
            at: Local::now(),
        });
    }

    #[test]
    fn chat_defaults_to_the_newest_conversation_and_cycles() {
        let mut state = AppState::new();
        assert_eq!(state.active_chat_peer(), None);

        say(&mut state, "alice");
        say(&mut state, "bob");
        say(&mut state, "alice");
        assert_eq!(state.chat_peers(), vec!["alice", "bob"]);
        assert_eq!(state.active_chat_peer(), Some("alice"));

        state.cycle_chat_peer(true);
        assert_eq!(state.active_chat_peer(), Some("bob"));
        state.cycle_chat_peer(true); // wraps
        assert_eq!(state.active_chat_peer(), Some("alice"));
        state.cycle_chat_peer(false);
        assert_eq!(state.active_chat_peer(), Some("bob"));
    }

    #[test]
    fn opening_a_chat_pins_a_peer_with_no_messages_yet() {
        let mut state = AppState::new();
        say(&mut state, "alice");
        state.open_chat("carol".to_string());
        assert_eq!(state.active_chat_peer(), Some("carol"));
        assert!(state.show_messages && state.chat_composing);
        assert_eq!(state.unread_messages, 0);
        assert_eq!(state.chat_peers(), vec!["alice", "carol"]);
    }
}
