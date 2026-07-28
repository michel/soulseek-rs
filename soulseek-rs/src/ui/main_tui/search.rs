use super::MainTui;
use crate::models::{
    ChatMessage, FileDisplayData, FocusedPane, MessageDirection, SearchEntry,
    SearchStatus,
};
use chrono::Local;
use std::{
    sync::{Arc, atomic::AtomicBool},
    thread,
    time::Instant,
};

impl MainTui {
    pub(super) fn remove_search_at_index(&mut self, index: usize) {
        if index >= self.state.searches.len() {
            return;
        }

        // Cancel the search if it's active
        if let Some(search) = self.state.searches.get(index) {
            search
                .cancel_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        // Check if we're removing the currently active search
        let was_active_search = self.state.selected_search_index == Some(index);

        // Drop it from the session too. The list is a view of what the session
        // holds, so removing it only here would last until the next sync put
        // it straight back — and against a daemon it would come back for every
        // other window as well.
        if let Some(search) = self.state.searches.get(index) {
            self.client.forget_search(&search.query);
        }
        self.state.searches.remove(index);

        if let Some(current_idx) = self.state.selected_search_index {
            if current_idx == index {
                // Removed the active search - clear it
                self.state.selected_search_index = None;
            } else if current_idx > index {
                // Active search was after removed one - decrement index
                self.state.selected_search_index = Some(current_idx - 1);
            }
            // If current_idx < index, no change needed
        }

        // If we removed the active search, clear results pane
        if was_active_search {
            self.clear_results_pane();
        }

        if self.state.searches.is_empty() {
            self.state.searches_table_state.select(None);
        } else {
            let new_selection = if index >= self.state.searches.len() {
                self.state.searches.len() - 1
            } else {
                index
            };
            self.state.searches_table_state.select(Some(new_selection));
        }
    }

    pub(super) fn clear_all_searches(&mut self) {
        // Cancel all active searches
        for search in &self.state.searches {
            search
                .cancel_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        // And drop them from the session, or the next sync brings every one
        // of them straight back — the same reason removing a single search
        // has to say so.
        let queries: Vec<String> = self
            .state
            .searches
            .iter()
            .map(|search| search.query.clone())
            .collect();
        for query in queries {
            self.client.forget_search(&query);
        }

        self.state.searches.clear();
        self.state.searches_table_state.select(None);
        self.state.selected_search_index = None;

        // Clear results pane
        self.clear_results_pane();
    }

    fn clear_results_pane(&mut self) {
        self.state.results_items.clear();
        self.state.results_filtered_items.clear();
        self.state.results_filtered_indices.clear();
        self.state.results_selected_indices.clear();
        self.state.results_table_state.select(None);
        self.state.results_filter_query.clear();
        self.state.results_is_filtering = false;
    }

    pub(super) fn apply_filter(&mut self) {
        let (items, indices) = filter_results(
            &self.state.results_items,
            &self.state.results_filter_query,
        );
        self.state.results_filtered_items = items;
        self.state.results_filtered_indices = indices;
        if !self.state.results_filtered_items.is_empty() {
            self.state.results_table_state.select(Some(0));
        }
    }

    /// Drain any private messages received since the last tick into the inbox.
    pub(super) fn poll_private_messages(&mut self) {
        for msg in self.client.take_private_messages() {
            self.state.messages.push(ChatMessage {
                direction: MessageDirection::Incoming,
                peer: msg.username().to_string(),
                text: msg.message().to_string(),
                at: Local::now(),
            });
            // Badge the inbox when it isn't currently open.
            if !self.state.show_messages {
                self.state.unread_messages += 1;
            }
        }
    }

    /// Parse a `<recipient> <text>` compose line and send it.
    pub(super) fn send_message_from_input(&mut self, input: &str) {
        let Some((recipient, text)) = input.split_once(char::is_whitespace)
        else {
            return;
        };
        let recipient = recipient.trim();
        let text = text.trim();
        if recipient.is_empty() || text.is_empty() {
            return;
        }

        match self.client.send_private_message(recipient, text) {
            Ok(()) => {
                self.state.messages.push(ChatMessage {
                    direction: MessageDirection::Outgoing,
                    peer: recipient.to_string(),
                    text: text.to_string(),
                    at: Local::now(),
                });
                // Land in the chat box on the conversation just started.
                self.state.open_chat(recipient.to_string());
                self.state.chat_composing = false;
            }
            Err(e) => soulseek_rs::warn!("Failed to send message: {e}"),
        }
    }

    /// Send the chat popup's compose buffer to the active conversation,
    /// staying in compose mode so a reply can follow immediately.
    pub(super) fn send_chat_message(&mut self) {
        let Some(peer) = self.state.active_chat_peer().map(ToString::to_string)
        else {
            return;
        };
        let text = self.state.chat_input.trim().to_string();
        if text.is_empty() {
            return;
        }

        match self.client.send_private_message(&peer, &text) {
            Ok(()) => self.state.messages.push(ChatMessage {
                direction: MessageDirection::Outgoing,
                peer,
                text,
                at: Local::now(),
            }),
            Err(e) => soulseek_rs::warn!("Failed to send message: {e}"),
        }
        self.state.chat_input.clear();
    }

    pub(super) fn start_search(&mut self, query: String) {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let search_entry = SearchEntry {
            query: query.clone(),
            status: SearchStatus::Active,
            results: Vec::new(),
            known_files: 0,
            owned: true,
            start_time: Instant::now(),
            cancel_flag: cancel_flag.clone(),
        };

        self.state.searches.push(search_entry);
        let search_index = self.state.searches.len() - 1;
        self.state.searches_table_state.select(Some(search_index));

        // Make this search the active one
        self.state.selected_search_index = Some(search_index);

        // Initialize results display (empty at first)
        self.state.results_items.clear();
        self.state.results_filtered_items.clear();
        self.state.results_filtered_indices.clear();
        self.state.results_selected_indices.clear();
        self.state.results_table_state.select(Some(0));

        // Switch focus to Results pane
        self.state.focused_pane = FocusedPane::Results;

        let client = self.client.clone();
        let timeout = self.search_timeout;

        thread::spawn(move || {
            match client.search_with_cancel(
                &query,
                timeout,
                Some(cancel_flag.clone()),
            ) {
                Ok(_results) => {
                    // Results will be polled in update_search_results
                }
                Err(e) => {
                    soulseek_rs::warn!("Search failed: {e}");
                }
            }
        });
    }

    /// Show the searches the session knows about, not just this window's.
    ///
    /// Against a daemon the search cache is shared, so a window that opens
    /// later sees what the others have looked for, and a query run in one
    /// appears in the other. A search is still collecting if it went out less
    /// than one search window ago — the window belongs to whichever client
    /// started it, so the age is the only thing another window can judge by.
    fn sync_searches_with_session(&mut self) {
        // Only worth doing when the session is shared: a window with its own
        // session ran every search in the list and already knows their state,
        // so asking would cost a full copy of every result set per frame and
        // tell us nothing.
        if self.client.daemon_endpoint().is_none() {
            return;
        }

        let timeout = self.search_timeout.as_secs();
        for known in self.client.all_searches() {
            let running = known
                .started_secs_ago
                .is_some_and(|elapsed| elapsed < timeout);

            if let Some(existing) = self
                .state
                .searches
                .iter_mut()
                .find(|search| search.query == known.query)
            {
                // Ours keeps the status its own worker thread sets; the rest
                // follow the session, so a search finishing in one window
                // stops saying "Active" in the others.
                if known.started_secs_ago.is_some() && !existing.owned {
                    existing.status = if running {
                        SearchStatus::Active
                    } else {
                        SearchStatus::Completed
                    };
                    existing.known_files = known.files;
                }
                continue;
            }

            self.state.searches.push(SearchEntry {
                query: known.query,
                status: if running {
                    SearchStatus::Active
                } else {
                    SearchStatus::Completed
                },
                results: Vec::new(),
                known_files: known.files,
                owned: false,
                start_time: std::time::Instant::now(),
                cancel_flag: std::sync::Arc::new(
                    std::sync::atomic::AtomicBool::new(false),
                ),
            });
        }

        // Dismissed elsewhere means dismissed here. Ours stay: this window is
        // the authority on a search it started.
        let live: Vec<String> = self
            .client
            .all_searches()
            .into_iter()
            .map(|search| search.query)
            .collect();
        self.state
            .searches
            .retain(|search| search.owned || live.contains(&search.query));
    }

    pub(super) fn update_search_results(&mut self) {
        self.sync_searches_with_session();
        let timeout = self.search_timeout;
        let selected_search_index = self.state.selected_search_index;

        // Fetch all results in one go (single lock acquisition per query)
        // Use try_get_search_results to avoid blocking the UI thread
        // Only what the user can see or what is still arriving. A shared
        // cache can hold every search every window has ever run, and fetching
        // all of them each frame is a round trip apiece against a daemon.
        // The rest show the count the session reported.
        let all_results: Vec<(usize, Vec<_>)> = self
            .state
            .searches
            .iter()
            .enumerate()
            .filter(|(idx, search)| {
                search.status == SearchStatus::Active
                    || selected_search_index == Some(*idx)
            })
            .map(|(idx, s)| (idx, s.query.clone()))
            .filter_map(|(idx, query)| {
                self.client
                    .try_get_search_results(&query)
                    .map(|results| (idx, results))
            })
            .collect();

        // Now update state without holding any client locks
        for (idx, search_results) in all_results {
            if let Some(search) = self.state.searches.get_mut(idx) {
                // Results only accumulate, so an unchanged file count means
                // nothing new arrived: skip the rebuild, which clones the
                // full result list several times and dominates frame time.
                let total_files: usize =
                    search_results.iter().map(|r| r.files.len()).sum();
                if total_files != search.results.len() {
                    search.results.clear();
                    for result in search_results {
                        for file in result.files {
                            search.results.push(FileDisplayData {
                                filename: file.name.clone(),
                                size: file.size,
                                username: result.username.clone(),
                                speed: result.speed,
                                slots: result.slots,
                                bitrate: file.attribs.get(&0).copied(),
                                length_seconds: file.attribs.get(&1).copied(),
                            });
                        }
                    }

                    // Update selected search if this is the active one. Re-derive
                    // the filtered view from the current query so an active
                    // filter is preserved as new results stream in, rather than
                    // being clobbered by the full unfiltered list.
                    if let Some(selected_idx) = selected_search_index
                        && selected_idx == idx
                    {
                        self.state.results_items = search.results.clone();
                        let (items, indices) = filter_results(
                            &self.state.results_items,
                            &self.state.results_filter_query,
                        );
                        self.state.results_filtered_items = items;
                        self.state.results_filtered_indices = indices;
                    }
                }

                // Mark as completed after timeout
                if search.status == SearchStatus::Active
                    && search.start_time.elapsed() > timeout
                {
                    search.status = SearchStatus::Completed;
                }
            }
        }
    }
}

/// Filter `items` by a case-insensitive substring match on filename or
/// username. Returns the matching items alongside their indices in the original
/// list, so callers can translate a filtered display index back to the
/// unfiltered results. An empty query returns everything (identity mapping).
fn filter_results(
    items: &[FileDisplayData],
    query: &str,
) -> (Vec<FileDisplayData>, Vec<usize>) {
    let query = query.to_lowercase();
    if query.is_empty() {
        return (items.to_vec(), (0..items.len()).collect());
    }

    let mut filtered_items = Vec::new();
    let mut filtered_indices = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        if item.filename.to_lowercase().contains(&query)
            || item.username.to_lowercase().contains(&query)
        {
            filtered_items.push(item.clone());
            filtered_indices.push(idx);
        }
    }
    (filtered_items, filtered_indices)
}

#[cfg(test)]
mod tests {
    use super::filter_results;
    use crate::models::FileDisplayData;

    fn file(filename: &str, username: &str) -> FileDisplayData {
        FileDisplayData {
            filename: filename.to_string(),
            username: username.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_query_returns_identity_mapping() {
        let items = vec![file("a.mp3", "bob"), file("b.flac", "amy")];
        let (filtered, indices) = filter_results(&items, "");
        assert_eq!(filtered.len(), 2);
        assert_eq!(indices, vec![0, 1]);
    }

    #[test]
    fn query_matches_filename_and_username_and_maps_indices() {
        let items = vec![
            file("track.mp3", "bob"),
            file("song.flac", "alice"),
            file("alice_demo.mp3", "carol"),
        ];
        // "alice" matches item 1 (username) and item 2 (filename).
        let (filtered, indices) = filter_results(&items, "alice");
        assert_eq!(filtered.len(), 2);
        assert_eq!(indices, vec![1, 2]);
        assert_eq!(filtered[0].filename, "song.flac");
        assert_eq!(filtered[1].filename, "alice_demo.mp3");
    }

    #[test]
    fn query_is_case_insensitive() {
        let items = vec![file("The Weeknd.mp3", "dj")];
        let (filtered, indices) = filter_results(&items, "WEEKND");
        assert_eq!(filtered.len(), 1);
        assert_eq!(indices, vec![0]);
    }
}
