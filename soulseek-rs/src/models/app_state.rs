use crate::models::FileDisplayData;
use ratatui::{layout::Rect, widgets::TableState};
use soulseek_rs::{DownloadStatus, types::Download};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc::Receiver, mpsc::Sender};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FocusedPane {
    Searches,
    Results,
    Downloads,
}

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

    // Pane areas for mouse interaction
    pub searches_pane_area: Option<Rect>,
    pub results_pane_area: Option<Rect>,
    pub downloads_pane_area: Option<Rect>,
}

impl AppState {
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

            searches_pane_area: None,
            results_pane_area: None,
            downloads_pane_area: None,
        }
    }

    #[allow(dead_code)]
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
