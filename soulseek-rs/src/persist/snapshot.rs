//! Capture the persistable slice of [`AppState`] (and restore the pure
//! parts of it). Live handles (channels, cancel flags) never leave the
//! process; only plain data goes to disk.

use super::state::PersistedDownload;
use crate::models::{AppState, SearchEntry, SearchStatus};
use soulseek_rs::DownloadStatus;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub downloads: Vec<PersistedDownload>,
    pub queries: Vec<String>,
    pub rooms: Vec<String>,
}

impl Snapshot {
    /// Extract what should survive a restart. Downloads that are not yet
    /// `Completed` (including failed/timed-out ones) are marked incomplete
    /// so the next start can re-enqueue them.
    pub fn capture(state: &AppState) -> Self {
        let downloads = state
            .downloads
            .iter()
            .map(|entry| PersistedDownload {
                username: entry.download.username.clone(),
                filename: entry.download.filename.clone(),
                size: entry.download.size,
                download_directory: entry
                    .download
                    .download_directory
                    .clone(),
                completed: matches!(
                    entry.download.status,
                    DownloadStatus::Completed
                ),
            })
            .collect();

        let mut queries: Vec<String> = Vec::new();
        for entry in &state.searches {
            if !queries.contains(&entry.query) {
                queries.push(entry.query.clone());
            }
        }

        let rooms =
            state.rooms.open.iter().map(|room| room.name.clone()).collect();

        Self {
            downloads,
            queries,
            rooms,
        }
    }
}

/// Rebuild search history entries from persisted query strings. Restored
/// searches are inert (completed, no results) — selecting one shows an
/// empty result set until it is re-run.
pub fn restore_searches(state: &mut AppState, queries: &[String]) {
    for query in queries {
        state.searches.push(SearchEntry {
            query: query.clone(),
            status: SearchStatus::Completed,
            results: Vec::new(),
            start_time: std::time::Instant::now(),
            cancel_flag: std::sync::Arc::new(
                std::sync::atomic::AtomicBool::new(false),
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DownloadEntry;
    use soulseek_rs::types::{Download, DownloadMetadata};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, mpsc};
    use std::time::Instant;

    fn download(
        filename: &str,
        status: DownloadStatus,
    ) -> DownloadEntry {
        let (sender, _receiver) = mpsc::channel();
        DownloadEntry {
            download: Download {
                username: "peer".into(),
                filename: filename.into(),
                token: 7,
                size: 42,
                download_directory: "/music".into(),
                status,
                sender,
                queue_position: None,
                metadata: DownloadMetadata::default(),
            },
            receiver: None,
        }
    }

    fn search(query: &str) -> SearchEntry {
        SearchEntry {
            query: query.into(),
            status: SearchStatus::Active,
            results: Vec::new(),
            start_time: Instant::now(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn capture_maps_download_status_to_completed_flag() {
        let mut state = AppState::new();
        state.downloads.push(download("done.mp3", DownloadStatus::Completed));
        state.downloads.push(download("queued.mp3", DownloadStatus::Queued));
        state.downloads.push(download(
            "failed.mp3",
            DownloadStatus::Failed(Some("nope".into())),
        ));

        let snapshot = Snapshot::capture(&state);
        assert_eq!(
            snapshot
                .downloads
                .iter()
                .map(|d| (d.filename.as_str(), d.completed))
                .collect::<Vec<_>>(),
            vec![
                ("done.mp3", true),
                ("queued.mp3", false),
                ("failed.mp3", false)
            ]
        );
        assert_eq!(snapshot.downloads[0].username, "peer");
        assert_eq!(snapshot.downloads[0].size, 42);
        assert_eq!(snapshot.downloads[0].download_directory, "/music");
    }

    #[test]
    fn capture_takes_queries_in_order_without_duplicates() {
        let mut state = AppState::new();
        state.searches.push(search("beatles"));
        state.searches.push(search("miles davis"));
        state.searches.push(search("beatles"));
        let snapshot = Snapshot::capture(&state);
        assert_eq!(
            snapshot.queries,
            vec!["beatles".to_string(), "miles davis".to_string()]
        );
    }

    #[test]
    fn capture_takes_open_room_names() {
        let mut state = AppState::new();
        state.rooms.focus_or_open("indie");
        state.rooms.focus_or_open("jazz");
        let snapshot = Snapshot::capture(&state);
        assert_eq!(
            snapshot.rooms,
            vec!["indie".to_string(), "jazz".to_string()]
        );
    }

    #[test]
    fn restore_searches_creates_inert_completed_entries() {
        let mut state = AppState::new();
        restore_searches(
            &mut state,
            &["beatles".to_string(), "miles davis".to_string()],
        );
        assert_eq!(state.searches.len(), 2);
        assert_eq!(state.searches[0].query, "beatles");
        assert_eq!(state.searches[0].status, SearchStatus::Completed);
        assert!(state.searches[0].results.is_empty());
    }

    #[test]
    fn capture_of_restored_searches_round_trips() {
        let mut state = AppState::new();
        restore_searches(&mut state, &["beatles".to_string()]);
        let snapshot = Snapshot::capture(&state);
        assert_eq!(snapshot.queries, vec!["beatles".to_string()]);
    }
}
