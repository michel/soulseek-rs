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
    /// What the session says this search has found, for a window that has not
    /// fetched the results itself. Fetching every search every frame is what
    /// a shared cache would otherwise cost.
    pub known_files: usize,
    /// Whether this window started the search. Its own worker knows when it
    /// finishes; for the rest the session is the authority.
    pub owned: bool,
    pub start_time: Instant,
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

impl FocusedPane {
    /// Every focusable pane, in the order `Tab` walks them: the number each
    /// carries in its legend.
    pub const ALL: [Self; 3] = [Self::Searches, Self::Results, Self::Downloads];

    const fn index(self) -> usize {
        match self {
            Self::Searches => 0,
            Self::Results => 1,
            Self::Downloads => 2,
        }
    }
}

/// Which panes are on screen. A hidden pane gives its space to the others; a
/// zoomed one takes the whole content area for itself, so `Tab` through the
/// panes while zoomed reads as switching windows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaneLayout {
    hidden: [bool; 3],
    pub zoomed: bool,
}

impl PaneLayout {
    #[must_use]
    pub const fn is_visible(&self, pane: FocusedPane) -> bool {
        !self.hidden[pane.index()]
    }

    /// The panes still on screen, in `Tab` order.
    pub fn visible(&self) -> impl Iterator<Item = FocusedPane> + '_ {
        FocusedPane::ALL
            .into_iter()
            .filter(move |pane| self.is_visible(*pane))
    }

    /// Take `pane` off the screen. Refuses when it is the last one showing —
    /// an empty window has nothing to press a key in — and says so.
    pub fn hide(&mut self, pane: FocusedPane) -> bool {
        if self.visible().count() <= 1 {
            return false;
        }
        self.hidden[pane.index()] = true;
        true
    }

    pub const fn show(&mut self, pane: FocusedPane) {
        self.hidden[pane.index()] = false;
    }

    /// The visible pane after (or before) `from`, wrapping around. `from`
    /// itself may already be hidden, which is how focus leaves a pane that
    /// was just hidden.
    #[must_use]
    pub fn neighbour(&self, from: FocusedPane, forward: bool) -> FocusedPane {
        let count = FocusedPane::ALL.len();
        let step = if forward { 1 } else { count - 1 };
        let mut index = from.index();
        for _ in 0..count {
            index = (index + step) % count;
            let candidate = FocusedPane::ALL[index];
            if self.is_visible(candidate) {
                return candidate;
            }
        }
        from
    }
}

/// Where a scrolling log is being read.
///
/// Either following its newest line, or held at a row of its history so
/// that arriving messages do not push the reader along. The renderer records
/// the log's size at each draw, which is what lets the keys move by a
/// screenful and stop at the ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogView {
    /// The first visible row while held in the history; `None` follows the
    /// tail.
    top: Option<usize>,
    /// Rows the log had, and rows the window showed, at the last draw.
    rows: usize,
    height: usize,
}

impl LogView {
    /// Whether the newest line is on screen, so new messages are being seen.
    #[must_use]
    pub const fn following(&self) -> bool {
        self.top.is_none()
    }

    /// Rows a page key moves: what the window showed last time.
    #[must_use]
    pub const fn page(&self) -> usize {
        self.height
    }

    const fn last_top(&self) -> usize {
        self.rows.saturating_sub(self.height)
    }

    /// The rows of a log `len` long to draw in a window `height` tall. A
    /// held position that has reached the tail goes back to following it.
    pub fn window(
        &mut self,
        len: usize,
        height: usize,
    ) -> std::ops::Range<usize> {
        self.rows = len;
        self.height = height.max(1);
        let top = match self.top {
            Some(top) if top < self.last_top() => top,
            _ => {
                self.top = None;
                self.last_top()
            }
        };
        top..(top + self.height).min(len)
    }

    /// Move the window `rows` down the log (up, when negative), holding
    /// wherever it lands unless that is the tail.
    pub fn scroll(&mut self, rows: isize) {
        let current = self.top.unwrap_or_else(|| self.last_top());
        let next = current.saturating_add_signed(rows);
        self.top = (next < self.last_top()).then_some(next);
    }

    pub const fn to_oldest(&mut self) {
        self.top = Some(0);
    }

    pub const fn to_newest(&mut self) {
        self.top = None;
    }
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
    /// Cells the query column is scrolled to the left.
    pub searches_query_offset: usize,

    // Results
    pub results_items: Vec<FileDisplayData>,
    pub results_filtered_items: Vec<FileDisplayData>,
    pub results_filtered_indices: Vec<usize>,
    pub results_table_state: TableState,
    pub results_selected_indices: std::collections::HashSet<usize>,
    pub results_filter_query: String,
    pub results_is_filtering: bool,
    /// Cells the file-name column is scrolled to the left.
    pub results_name_offset: usize,

    // Downloads
    pub downloads: Vec<DownloadEntry>,
    pub downloads_table_state: TableState,
    /// Cells the transfers' file-name column is scrolled to the left.
    pub downloads_name_offset: usize,
    pub downloads_receiver_channel:
        Option<Receiver<(Download, Receiver<DownloadStatus>)>>,
    pub downloads_sender_channel:
        Option<Sender<(Download, Receiver<DownloadStatus>)>>,
    pub active_downloads_count: usize,

    // UI State
    pub focused_pane: FocusedPane,
    pub layout: PaneLayout,
    /// The keys overlay (`?`) is open.
    pub show_help: bool,
    /// Rows the keys overlay is scrolled down, for a terminal too short to
    /// show it whole. The renderer clamps it to what there is to scroll.
    pub help_scroll: usize,
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
    /// Where the conversation is being read.
    pub chat_view: LogView,

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
    /// Where the open popup was last drawn, for its page size. Only one is
    /// ever open, so one record serves them all.
    pub popup_area: Option<Rect>,
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
            searches_query_offset: 0,

            results_items: Vec::new(),
            results_filtered_items: Vec::new(),
            results_filtered_indices: Vec::new(),
            results_table_state,
            results_selected_indices: std::collections::HashSet::new(),
            results_filter_query: String::new(),
            results_is_filtering: false,
            results_name_offset: 0,

            downloads: Vec::new(),
            downloads_table_state,
            downloads_name_offset: 0,
            downloads_receiver_channel: None,
            downloads_sender_channel: None,
            active_downloads_count: 0,

            focused_pane: FocusedPane::Searches,
            layout: PaneLayout::default(),
            show_help: false,
            help_scroll: 0,
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
            chat_view: LogView::default(),

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
            popup_area: None,
        }
    }

    /// How far the focused list's long column is scrolled sideways.
    pub const fn name_offset_mut(&mut self, pane: FocusedPane) -> &mut usize {
        match pane {
            FocusedPane::Searches => &mut self.searches_query_offset,
            FocusedPane::Results => &mut self.results_name_offset,
            FocusedPane::Downloads => &mut self.downloads_name_offset,
        }
    }

    /// Put the focus on `pane`, bringing it back on screen if it was hidden:
    /// pressing a pane's number is also how it comes back.
    pub const fn focus_pane(&mut self, pane: FocusedPane) {
        self.layout.show(pane);
        self.focused_pane = pane;
    }

    /// Move the focus to the next (or previous) visible pane.
    pub fn cycle_focus(&mut self, forward: bool) {
        self.focused_pane = self.layout.neighbour(self.focused_pane, forward);
    }

    /// Hide the focused pane and move the focus to its neighbour. The last
    /// visible pane stays; there would be nowhere for the focus to go.
    pub fn hide_focused_pane(&mut self) {
        let pane = self.focused_pane;
        if self.layout.hide(pane) {
            self.focused_pane = self.layout.neighbour(pane, true);
        }
    }

    pub const fn toggle_zoom(&mut self) {
        self.layout.zoomed = !self.layout.zoomed;
    }

    /// Open the chat popup on a conversation with `peer`, ready to type.
    pub fn open_chat(&mut self, peer: String) {
        self.chat_peer = Some(peer);
        self.chat_input.clear();
        self.chat_view.to_newest();
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

    /// Conversation partners in first-contact order. Rescans every
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
        // A different conversation opens at its end; the same one stays put.
        if next != current {
            self.chat_view.to_newest();
        }
        self.chat_peer = Some(peer);
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

    fn visible(layout: &PaneLayout) -> Vec<FocusedPane> {
        layout.visible().collect()
    }

    #[test]
    fn every_pane_starts_visible_and_unzoomed() {
        let state = AppState::new();
        assert_eq!(visible(&state.layout), FocusedPane::ALL);
        assert!(!state.layout.zoomed);
        assert!(!state.show_help);
    }

    #[test]
    fn tab_cycles_the_focus_through_the_visible_panes_and_wraps() {
        let mut state = AppState::new();
        assert_eq!(state.focused_pane, FocusedPane::Searches);
        state.cycle_focus(true);
        assert_eq!(state.focused_pane, FocusedPane::Results);
        state.cycle_focus(true);
        assert_eq!(state.focused_pane, FocusedPane::Downloads);
        state.cycle_focus(true);
        assert_eq!(state.focused_pane, FocusedPane::Searches, "wraps");
        state.cycle_focus(false);
        assert_eq!(state.focused_pane, FocusedPane::Downloads, "and back");
    }

    #[test]
    fn hiding_the_focused_pane_moves_the_focus_past_it() {
        let mut state = AppState::new();
        state.focus_pane(FocusedPane::Results);
        state.hide_focused_pane();
        assert_eq!(
            visible(&state.layout),
            [FocusedPane::Searches, FocusedPane::Downloads]
        );
        assert_eq!(state.focused_pane, FocusedPane::Downloads);

        // Tab now skips the hidden pane in both directions.
        state.cycle_focus(true);
        assert_eq!(state.focused_pane, FocusedPane::Searches);
        state.cycle_focus(false);
        assert_eq!(state.focused_pane, FocusedPane::Downloads);
    }

    #[test]
    fn hiding_wraps_the_focus_to_the_first_pane() {
        let mut state = AppState::new();
        state.focus_pane(FocusedPane::Downloads);
        state.hide_focused_pane();
        assert_eq!(state.focused_pane, FocusedPane::Searches);
    }

    #[test]
    fn the_last_visible_pane_cannot_be_hidden() {
        let mut state = AppState::new();
        state.hide_focused_pane();
        state.hide_focused_pane();
        assert_eq!(visible(&state.layout), [FocusedPane::Downloads]);
        state.hide_focused_pane();
        assert_eq!(
            visible(&state.layout),
            [FocusedPane::Downloads],
            "one pane always stays on screen"
        );
        assert_eq!(state.focused_pane, FocusedPane::Downloads);
    }

    #[test]
    fn focusing_a_hidden_pane_by_number_brings_it_back() {
        let mut state = AppState::new();
        state.focus_pane(FocusedPane::Results);
        state.hide_focused_pane();
        assert!(!state.layout.is_visible(FocusedPane::Results));

        state.focus_pane(FocusedPane::Results);

        assert!(state.layout.is_visible(FocusedPane::Results));
        assert_eq!(state.focused_pane, FocusedPane::Results);
    }

    #[test]
    fn zoom_toggles_and_leaves_hidden_panes_alone() {
        let mut state = AppState::new();
        state.focus_pane(FocusedPane::Searches);
        state.hide_focused_pane();
        state.toggle_zoom();
        assert!(state.layout.zoomed);
        assert!(!state.layout.is_visible(FocusedPane::Searches));
        state.toggle_zoom();
        assert!(!state.layout.zoomed);
        assert!(!state.layout.is_visible(FocusedPane::Searches));
    }

    #[test]
    fn a_log_view_follows_the_tail_until_held_and_holds_against_new_rows() {
        let mut view = LogView::default();
        assert_eq!(view.window(100, 10), 90..100);
        assert!(view.following());

        view.scroll(-10);
        assert_eq!(view.window(100, 10), 80..90);
        assert!(!view.following());

        // Five more rows arrive: the held window does not move.
        assert_eq!(view.window(105, 10), 80..90);

        // Back down to the tail resumes following, including new rows.
        view.scroll(10);
        view.scroll(10);
        assert_eq!(view.window(105, 10), 95..105);
        assert!(view.following());

        view.to_oldest();
        assert_eq!(view.window(105, 10), 0..10);
        view.scroll(-10);
        assert_eq!(view.window(105, 10), 0..10, "clamped at the top");
        view.to_newest();
        assert_eq!(view.window(105, 10), 95..105);
    }

    #[test]
    fn a_log_shorter_than_its_window_is_never_held() {
        let mut view = LogView::default();
        assert_eq!(view.window(3, 10), 0..3);
        view.to_oldest();
        assert_eq!(view.window(3, 10), 0..3);
        assert!(view.following(), "nothing to hold");
    }

    #[test]
    fn cycling_to_the_same_chat_keeps_the_place() {
        let mut state = AppState::new();
        say(&mut state, "alice");
        state.chat_view.window(50, 10);
        state.chat_view.scroll(-10);
        state.cycle_chat_peer(true);
        assert!(!state.chat_view.following(), "only one chat: nothing moved");
        say(&mut state, "bob");
        state.cycle_chat_peer(true);
        assert!(state.chat_view.following(), "a new chat opens at its end");
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
