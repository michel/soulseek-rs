use super::MainTui;
use super::input::{jumped, list_jump};
use crate::models::BrowseStatus;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{thread, time::Duration};

/// How long to wait for a browse response before showing a timeout notice.
const BROWSE_TIMEOUT: Duration = Duration::from_secs(20);

impl MainTui {
    pub(super) fn handle_browse_input(&mut self, key: KeyEvent) {
        // Typing goes to the filter; the other keys still move around.
        if self.state.browse.active_tab().is_some_and(|b| b.filtering)
            && self.handle_browse_filter_input(key)
        {
            return;
        }
        // Esc peels back one level: a filter first, then the popup.
        if key.code == KeyCode::Esc
            && let Some(browse) = self.state.browse.active_tab_mut()
            && !browse.filter().is_empty()
        {
            browse.set_filter(String::new());
            return;
        }
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            self.state.show_browse = false;
            return;
        }
        let jump = list_jump(key, self.popup_page());
        // Control combinations are only ever jumps: ctrl-d pages, d downloads.
        if jump.is_none() && key.modifiers.contains(KeyModifiers::CONTROL) {
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

        // The highlighted row and where it sits, then drop the borrow.
        let (len, sel, row) = {
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
            (rows.len(), sel, rows[sel].clone())
        };

        if let Some(jump) = jump {
            if let Some(browse) = self.state.browse.active_tab_mut() {
                browse.selected_row = jumped(sel, len, jump);
            }
            self.sync_browse_selection();
            return;
        }

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
                    self.state
                        .browse
                        .active_tab()
                        .map(|b| b.folder_files(&row.path))
                        .unwrap_or_default()
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
                    browse.selected_row = (sel + 1).min(len - 1);
                }
                // The folder-sized steps: the next or previous folder row.
                KeyCode::Char('J') => {
                    if let Some(next) =
                        (sel + 1..len).find(|&i| browse.rows()[i].is_folder)
                    {
                        browse.selected_row = next;
                    }
                }
                KeyCode::Char('K') => {
                    if let Some(previous) =
                        (0..sel).rev().find(|&i| browse.rows()[i].is_folder)
                    {
                        browse.selected_row = previous;
                    }
                }
                KeyCode::Char('H') => browse.collapse_all(),
                KeyCode::Char('L') => browse.expand_all(),
                KeyCode::Char('/') => browse.filtering = true,
                KeyCode::Right | KeyCode::Char('l') => {
                    if row.is_folder && !row.expanded {
                        browse.set_expanded(&row.path, true);
                    } else if row.is_folder {
                        browse.selected_row = (sel + 1).min(len - 1);
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if row.is_folder && row.expanded {
                        browse.set_expanded(&row.path, false);
                    } else if let Some(parent) = (0..sel)
                        .rev()
                        .find(|&i| browse.rows()[i].depth < row.depth)
                    {
                        browse.selected_row = parent;
                    }
                }
                KeyCode::Enter => {
                    // Folder toggle (files handled above).
                    browse.set_expanded(&row.path, !row.expanded);
                }
                _ => {}
            }
        }

        self.sync_browse_selection();
    }

    /// The keys that edit the filter while it is being typed. Says whether
    /// `key` was one of them; the rest move around the narrowed rows.
    fn handle_browse_filter_input(&mut self, key: KeyEvent) -> bool {
        let Some(browse) = self.state.browse.active_tab_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                browse.filtering = false;
                browse.set_filter(String::new());
            }
            KeyCode::Enter => browse.filtering = false,
            KeyCode::Char(c) => {
                let mut filter = browse.filter().to_string();
                filter.push(c);
                browse.set_filter(filter);
            }
            KeyCode::Backspace => {
                let mut filter = browse.filter().to_string();
                filter.pop();
                browse.set_filter(filter);
            }
            _ => return false,
        }
        self.sync_browse_selection();
        true
    }

    /// Point the browse table cursor at the active tab's selected row.
    fn sync_browse_selection(&mut self) {
        let selected = self.state.browse.active_tab().map(|b| b.selected_row);
        self.state.browse_table_state.select(selected);
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
        self.highlighted_result().map(|f| f.username.clone())
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
