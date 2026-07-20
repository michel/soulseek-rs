//! End-to-end test of the persistence pipeline across a simulated restart:
//! capture live-ish app state, write it through the real state files, load
//! it back in a fresh store (a "second session"), and restore it.

use soulseek_rs::DownloadStatus;
use soulseek_rs::types::{Download, DownloadMetadata};
use soulseek_rs_tui::models::{AppState, DownloadEntry};
use soulseek_rs_tui::persist::config::FileConfig;
use soulseek_rs_tui::persist::snapshot::{Snapshot, restore_searches};
use soulseek_rs_tui::persist::state::StateStore;

fn download_entry(filename: &str, status: DownloadStatus) -> DownloadEntry {
    DownloadEntry {
        download: Download {
            username: "peer".into(),
            filename: filename.into(),
            token: 1,
            size: 1000,
            download_directory: "/music".into(),
            status,
            sender: std::sync::mpsc::channel().0,
            queue_position: None,
            metadata: DownloadMetadata::default(),
        },
        receiver: None,
    }
}

#[test]
fn state_survives_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let state_dir = tmp.path().join("state");

    // Session 1: user searched, joined rooms, and has one finished and one
    // in-flight download when the app exits.
    {
        let mut state = AppState::new();
        state
            .downloads
            .push(download_entry("done.flac", DownloadStatus::Completed));
        state.downloads.push(download_entry(
            "half.flac",
            DownloadStatus::InProgress {
                bytes_downloaded: 500,
                total_bytes: 1000,
                speed_bytes_per_sec: 1.0,
            },
        ));
        restore_searches(&mut state, &["beatles".to_string()]);
        state.rooms.focus_or_open("indie");

        let store = StateStore::new(state_dir.clone());
        let snapshot = Snapshot::capture(&state);
        store.save_downloads(&snapshot.downloads).unwrap();
        store.save_search_queries(&snapshot.queries).unwrap();
        store.save_rooms(&snapshot.rooms).unwrap();
    }

    // The files on disk are versioned envelopes.
    let raw =
        std::fs::read_to_string(state_dir.join("downloads.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(parsed.get("version").is_some(), "missing version envelope");

    // Session 2: a fresh store (new process) sees the same state; the
    // incomplete download is flagged for auto-resume.
    let store = StateStore::new(state_dir);
    let downloads = store.load_downloads();
    assert_eq!(downloads.len(), 2);
    assert!(
        downloads
            .iter()
            .any(|d| d.filename == "done.flac" && d.completed)
    );
    assert!(
        downloads
            .iter()
            .any(|d| d.filename == "half.flac" && !d.completed)
    );
    assert_eq!(store.load_search_queries(), vec!["beatles".to_string()]);
    assert_eq!(store.load_rooms(), vec!["indie".to_string()]);

    let mut state = AppState::new();
    restore_searches(&mut state, &store.load_search_queries());
    assert_eq!(state.searches[0].query, "beatles");
}

#[test]
fn config_survives_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config").join("config.toml");

    let config = FileConfig {
        username: Some("alice".into()),
        server: Some("localhost:2242".into()),
        ..FileConfig::default()
    };
    config.save(&path).unwrap();

    // "Second session": reload from disk.
    let loaded = FileConfig::load(&path).unwrap();
    assert_eq!(loaded, config);
}
