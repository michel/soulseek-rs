use super::MainTui;
use crate::models::{BrowseStatus, files_under, find_node};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::{thread, time::Duration};

/// How long to wait for a browse response before showing a timeout notice.
const BROWSE_TIMEOUT: Duration = Duration::from_secs(20);

impl MainTui {
    pub(super) fn handle_browse_input(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            self.state.show_browse = false;
            return;
        }

        // Tab management: switch between browsed users or close the active tab.
        match key.code {
            KeyCode::Tab => {
                self.state.browse.next_tab();
                self.sync_browse_selection();
                return;
            }
            KeyCode::BackTab => {
                self.state.browse.prev_tab();
                self.sync_browse_selection();
                return;
            }
            KeyCode::Char('w') => {
                if !self.state.browse.close_active() {
                    self.state.show_browse = false;
                }
                self.sync_browse_selection();
                return;
            }
            KeyCode::Char('r') => {
                // Retry a timed-out browse.
                if let Some(username) = self.state.browse.retry_active() {
                    let _ = self.client.browse_user(&username);
                }
                return;
            }
            _ => {}
        }

        // Snapshot the flattened rows + current selection, then drop the borrow.
        let (rows, sel, row) = {
            let Some(browse) = self.state.browse.active_tab() else {
                self.state.show_browse = false;
                return;
            };
            if browse.status != BrowseStatus::Loaded {
                return;
            }
            let rows = browse.rows();
            if rows.is_empty() {
                return;
            }
            let sel = browse.selected_row.min(rows.len() - 1);
            let row = rows[sel].clone();
            (rows, sel, row)
        };

        // Downloads need `&self.client` free of the browse borrow.
        match key.code {
            KeyCode::Enter if !row.is_folder => {
                self.queue_browse_files(vec![(
                    row.path.clone(),
                    row.size.unwrap_or(0),
                )]);
                return;
            }
            KeyCode::Char('d') => {
                let files = if row.is_folder {
                    self.browse_folder_files(&row.path)
                } else {
                    vec![(row.path.clone(), row.size.unwrap_or(0))]
                };
                self.queue_browse_files(files);
                return;
            }
            _ => {}
        }

        // Navigation and expand/collapse mutate the browse state.
        if let Some(browse) = self.state.browse.active_tab_mut() {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    browse.selected_row = sel.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    browse.selected_row = (sel + 1).min(rows.len() - 1);
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    if row.is_folder && !row.expanded {
                        browse.expanded.insert(row.path.clone());
                    } else if row.is_folder {
                        browse.selected_row = (sel + 1).min(rows.len() - 1);
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if row.is_folder && row.expanded {
                        browse.expanded.remove(&row.path);
                    } else if let Some(parent) =
                        (0..sel).rev().find(|&i| rows[i].depth < row.depth)
                    {
                        browse.selected_row = parent;
                    }
                }
                KeyCode::Enter => {
                    // Folder toggle (files handled above).
                    if row.expanded {
                        browse.expanded.remove(&row.path);
                    } else {
                        browse.expanded.insert(row.path.clone());
                    }
                }
                _ => {}
            }
            // Re-clamp against the new flattened length.
            let new_len = browse.rows().len();
            browse.selected_row =
                browse.selected_row.min(new_len.saturating_sub(1));
        }

        self.sync_browse_selection();
    }

    /// Point the browse table cursor at the active tab's selected row.
    fn sync_browse_selection(&mut self) {
        let selected = self.state.browse.active_tab().map(|b| b.selected_row);
        self.state.browse_table_state.select(selected);
    }

    /// Files (`path`, `size`) under the active browse tab's folder at `path`.
    fn browse_folder_files(&self, path: &str) -> Vec<(String, u64)> {
        self.state
            .browse
            .active_tab()
            .and_then(|b| find_node(&b.tree, path))
            .map(files_under)
            .unwrap_or_default()
    }

    /// Queue downloads of `files` (path, size) from the active browse tab's user.
    fn queue_browse_files(&mut self, files: Vec<(String, u64)>) {
        let Some(username) =
            self.state.browse.active_tab().map(|b| b.username.clone())
        else {
            return;
        };
        if files.is_empty() {
            return;
        }
        let sender = self.downloads_sender();
        let client = self.client.clone();
        let download_dir = self.download_dir.clone();
        thread::spawn(move || {
            for (path, size) in files {
                match client.download(
                    path.clone(),
                    username.clone(),
                    size,
                    download_dir.clone(),
                ) {
                    Ok((download, rx)) => {
                        let _ = sender.send((download, rx));
                    }
                    Err(e) => {
                        soulseek_rs::warn!(
                            "Failed to queue download {path}: {e}"
                        );
                    }
                }
            }
        });
    }

    /// Request a user's shared files and open (or focus) their browse tab.
    pub(super) fn start_browse(&mut self, username: String) {
        let username = username.trim().to_string();
        if username.is_empty() {
            return;
        }
        // Open/focus the tab; only (re)issue the request when it's new or a
        // previous attempt timed out.
        if self.state.browse.open(&username) {
            let _ = self.client.browse_user(&username);
        }
        self.state.show_browse = true;
        self.sync_browse_selection();
    }

    /// The username of the highlighted search result (filter-aware).
    pub(super) fn highlighted_result_owner(&self) -> Option<String> {
        let selected = self.state.results_table_state.selected()?;
        let items = if self.state.results_filter_query.is_empty() {
            &self.state.results_items
        } else {
            &self.state.results_filtered_items
        };
        items.get(selected).map(|f| f.username.clone())
    }

    /// Drain browse responses into any loading tabs, or time them out.
    pub(super) fn poll_browse_result(&mut self) {
        // Which loading tabs are waiting, and for whom.
        let loading: Vec<(usize, String, std::time::Instant)> = self
            .state
            .browse
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, b)| b.status == BrowseStatus::Loading)
            .map(|(i, b)| (i, b.username.clone(), b.requested_at))
            .collect();

        for (idx, username, requested_at) in loading {
            if let Some(directories) = self.client.take_browse_result(&username)
            {
                if let Some(browse) = self.state.browse.tabs.get_mut(idx) {
                    browse.load(&directories);
                }
                if idx == self.state.browse.active {
                    self.state.browse_table_state.select(Some(0));
                }
            } else if requested_at.elapsed() > BROWSE_TIMEOUT
                && let Some(browse) = self.state.browse.tabs.get_mut(idx)
            {
                browse.status = BrowseStatus::TimedOut;
            }
        }
    }
}
