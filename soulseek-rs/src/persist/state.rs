//! Versioned JSON state files (downloads, search queries, open rooms,
//! private-message history).
//!
//! Each file is an envelope `{ "version": N, "data": ... }`. On load the
//! data passes through the migration chain from its stored version up to
//! current. State is disposable: a missing, corrupt, or newer-than-known
//! file loads as empty rather than failing startup. Writes are atomic
//! (tmp file + rename) so a crash never leaves a torn file.

use color_eyre::Result;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A migration takes the `data` value at version `i` and returns it at
/// version `i + 1`.
type Migration = fn(Value) -> Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct PersistedDownload {
    pub username: String,
    pub filename: String,
    pub size: u64,
    pub download_directory: String,
    pub completed: bool,
}

/// One line of private-message history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct PersistedMessage {
    pub peer: String,
    /// True when we sent it, false when the peer did.
    pub outgoing: bool,
    pub text: String,
    pub at: chrono::DateTime<chrono::Local>,
}

pub struct StateStore {
    dir: PathBuf,
}

impl StateStore {
    #[must_use]
    pub const fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn load_downloads(&self) -> Vec<PersistedDownload> {
        load(&self.dir.join("downloads.json"), DOWNLOADS_MIGRATIONS)
    }

    pub fn save_downloads(
        &self,
        downloads: &[PersistedDownload],
    ) -> Result<()> {
        save(
            &self.dir.join("downloads.json"),
            DOWNLOADS_MIGRATIONS.len() as u32,
            &downloads,
        )
    }

    pub fn load_search_queries(&self) -> Vec<String> {
        load(&self.dir.join("searches.json"), SEARCHES_MIGRATIONS)
    }

    pub fn save_search_queries(&self, queries: &[String]) -> Result<()> {
        save(
            &self.dir.join("searches.json"),
            SEARCHES_MIGRATIONS.len() as u32,
            &queries,
        )
    }

    pub fn load_rooms(&self) -> Vec<String> {
        load(&self.dir.join("rooms.json"), ROOMS_MIGRATIONS)
    }

    pub fn save_rooms(&self, rooms: &[String]) -> Result<()> {
        save(
            &self.dir.join("rooms.json"),
            ROOMS_MIGRATIONS.len() as u32,
            &rooms,
        )
    }

    pub fn load_messages(&self) -> Vec<PersistedMessage> {
        load(&self.dir.join("messages.json"), MESSAGES_MIGRATIONS)
    }

    pub fn save_messages(&self, messages: &[PersistedMessage]) -> Result<()> {
        save(
            &self.dir.join("messages.json"),
            MESSAGES_MIGRATIONS.len() as u32,
            &messages,
        )
    }

    /// The unread private-message count, so the badge survives a restart.
    pub fn load_unread(&self) -> usize {
        load(&self.dir.join("unread.json"), UNREAD_MIGRATIONS)
    }

    pub fn save_unread(&self, unread: usize) -> Result<()> {
        save(
            &self.dir.join("unread.json"),
            UNREAD_MIGRATIONS.len() as u32,
            &unread,
        )
    }
}

/// Per-file migration chains. `data` at version `i` is upgraded by
/// `MIGRATIONS[i]`; the current version is the chain length. All formats
/// are at version 0 today — add a fn here when the schema changes.
const DOWNLOADS_MIGRATIONS: &[Migration] = &[];
const SEARCHES_MIGRATIONS: &[Migration] = &[];
const ROOMS_MIGRATIONS: &[Migration] = &[];
const MESSAGES_MIGRATIONS: &[Migration] = &[];
const UNREAD_MIGRATIONS: &[Migration] = &[];

/// Load `data` from an envelope file, migrating old versions forward.
/// Missing, corrupt, or newer-than-known files all yield `T::default()`.
fn load<T: DeserializeOwned + Default>(
    path: &Path,
    migrations: &[Migration],
) -> T {
    let Ok(text) = std::fs::read_to_string(path) else {
        return T::default();
    };
    let Ok(envelope) = serde_json::from_str::<Value>(&text) else {
        set_aside(path, "corrupt");
        return T::default();
    };
    let version = envelope
        .get("version")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX) as usize;
    if version > migrations.len() {
        set_aside(path, "from a newer version");
        return T::default();
    }
    let mut data = envelope.get("data").cloned().unwrap_or(Value::Null);
    for migration in &migrations[version..] {
        data = migration(data);
    }
    serde_json::from_value(data).unwrap_or_else(|_| {
        set_aside(path, "unreadable");
        T::default()
    })
}

/// Rename an unusable state file to `.bak` so a later save doesn't
/// overwrite it and the user can still recover or inspect it.
fn set_aside(path: &Path, why: &str) {
    let bak = path.with_extension("json.bak");
    // Windows rename fails when the target exists; drop the old backup.
    let _ = std::fs::remove_file(&bak);
    match std::fs::rename(path, &bak) {
        Ok(()) => soulseek_rs::warn!(
            "State file {} is {why}; moved to {}",
            path.display(),
            bak.display()
        ),
        Err(e) => soulseek_rs::warn!(
            "State file {} is {why} (backup failed: {e})",
            path.display()
        ),
    }
}

fn save<T: Serialize>(path: &Path, version: u32, data: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let envelope = serde_json::json!({ "version": version, "data": data });
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&envelope)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempfile::tempdir().expect("a temporary state directory");
        let store = StateStore::new(dir.path().join("state"));
        (dir, store)
    }

    fn sample_download() -> PersistedDownload {
        PersistedDownload {
            username: "peer".into(),
            filename: "@@abc\\music\\song.mp3".into(),
            size: 123,
            download_directory: "/music".into(),
            completed: false,
        }
    }

    #[test]
    fn missing_files_load_as_empty() {
        let (_tmp, store) = store();
        assert_eq!(store.load_downloads(), vec![]);
        assert_eq!(store.load_search_queries(), Vec::<String>::new());
        assert_eq!(store.load_rooms(), Vec::<String>::new());
    }

    #[test]
    fn downloads_round_trip() {
        let (_tmp, store) = store();
        let downloads = vec![sample_download()];
        store.save_downloads(&downloads).unwrap();
        assert_eq!(store.load_downloads(), downloads);
    }

    #[test]
    fn messages_round_trip_including_the_timestamp() {
        let (_tmp, store) = store();
        let messages = vec![PersistedMessage {
            peer: "alice".into(),
            outgoing: true,
            text: "hey".into(),
            at: chrono::Local::now(),
        }];
        store.save_messages(&messages).unwrap();
        assert_eq!(store.load_messages(), messages);
    }

    #[test]
    fn unread_count_round_trips_and_defaults_to_zero() {
        let (_tmp, store) = store();
        // Missing file → no unread, so a fresh install shows no badge.
        assert_eq!(store.load_unread(), 0);
        store.save_unread(4).unwrap();
        assert_eq!(store.load_unread(), 4);
    }

    #[test]
    fn searches_and_rooms_round_trip() {
        let (_tmp, store) = store();
        store.save_search_queries(&["beatles".into()]).unwrap();
        store.save_rooms(&["indie".into(), "jazz".into()]).unwrap();
        assert_eq!(store.load_search_queries(), vec!["beatles".to_string()]);
        assert_eq!(
            store.load_rooms(),
            vec!["indie".to_string(), "jazz".to_string()]
        );
    }

    #[test]
    fn corrupt_file_loads_as_empty_and_is_kept_as_bak() {
        let (tmp, store) = store();
        let path = tmp.path().join("state").join("downloads.json");
        std::fs::create_dir_all(path.parent().expect("a parent"))
            .expect("the state directory");
        std::fs::write(&path, "{ not json").expect("the corrupt file");
        assert_eq!(store.load_downloads(), vec![]);
        // The bad file is set aside, not left to be overwritten.
        assert!(!path.exists());
        let bak = path.with_extension("json.bak");
        assert_eq!(
            std::fs::read_to_string(bak).expect("the .bak"),
            "{ not json"
        );
    }

    #[test]
    fn a_second_corrupt_file_replaces_the_previous_bak() {
        let (tmp, store) = store();
        let path = tmp.path().join("state").join("downloads.json");
        std::fs::create_dir_all(path.parent().expect("a parent"))
            .expect("the state directory");
        std::fs::write(path.with_extension("json.bak"), "old bak")
            .expect("the previous .bak");
        std::fs::write(&path, "{ newer corruption").expect("the corrupt file");
        assert_eq!(store.load_downloads(), vec![]);
        assert_eq!(
            std::fs::read_to_string(path.with_extension("json.bak"))
                .expect("the .bak"),
            "{ newer corruption"
        );
    }

    #[test]
    fn file_from_a_newer_build_loads_as_empty_and_is_kept_as_bak() {
        let (tmp, store) = store();
        let path = tmp.path().join("state").join("rooms.json");
        std::fs::create_dir_all(path.parent().expect("a parent"))
            .expect("the state directory");
        std::fs::write(&path, r#"{"version": 99, "data": ["x"]}"#)
            .expect("the newer-version file");
        assert_eq!(store.load_rooms(), Vec::<String>::new());
        assert!(!path.exists());
        assert!(path.with_extension("json.bak").exists());
    }

    #[test]
    fn migrations_upgrade_old_data() {
        // Machinery test with a synthetic chain: v0 stored plain strings,
        // v1 wraps each in an object {"name": ...}.
        #[derive(Debug, PartialEq, Default, Serialize, serde::Deserialize)]
        struct Named {
            name: String,
        }
        let chain: &[Migration] = &[|data| {
            Value::Array(
                data.as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|s| serde_json::json!({ "name": s }))
                    .collect(),
            )
        }];
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("things.json");
        std::fs::write(&path, r#"{"version": 0, "data": ["a", "b"]}"#).unwrap();
        let loaded: Vec<Named> = load(&path, chain);
        assert_eq!(
            loaded,
            vec![Named { name: "a".into() }, Named { name: "b".into() }]
        );
    }

    #[test]
    fn save_is_atomic_no_tmp_file_left_behind() {
        let (tmp, store) = store();
        store.save_rooms(&["indie".into()]).unwrap();
        let entries: Vec<_> = std::fs::read_dir(tmp.path().join("state"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(entries, vec!["rooms.json".to_string()]);
    }
}
