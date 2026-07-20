use crate::models::{BrowseTabs, FileDisplayData, RoomsState, SettingsState};
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

/// A private message shown in the inbox popup.
pub struct ChatMessage {
    pub direction: MessageDirection,
    /// The other party: the sender for incoming, the recipient for outgoing.
    pub peer: String,
    pub text: String,
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
