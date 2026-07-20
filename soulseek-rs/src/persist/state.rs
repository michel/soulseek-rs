//! Versioned JSON state files (downloads, search queries, open rooms).
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
}

/// Per-file migration chains. `data` at version `i` is upgraded by
/// `MIGRATIONS[i]`; the current version is the chain length. All formats
/// are at version 0 today — add a fn here when the schema changes.
const DOWNLOADS_MIGRATIONS: &[Migration] = &[];
const SEARCHES_MIGRATIONS: &[Migration] = &[];
const ROOMS_MIGRATIONS: &[Migration] = &[];

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
        soulseek_rs::warn!("Ignoring corrupt state file {}", path.display());
        return T::default();
    };
    let version = envelope
        .get("version")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX) as usize;
    if version > migrations.len() {
        soulseek_rs::warn!(
            "Ignoring state file {} from a newer version",
            path.display()
        );
        return T::default();
    }
    let mut data = envelope.get("data").cloned().unwrap_or(Value::Null);
    for migration in &migrations[version..] {
        data = migration(data);
    }
    serde_json::from_value(data).unwrap_or_else(|e| {
        soulseek_rs::warn!(
            "Ignoring unreadable state file {}: {e}",
            path.display()
        );
        T::default()
    })
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
        let dir = tempfile::tempdir().unwrap();
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
    fn corrupt_file_loads_as_empty() {
        let (tmp, store) = store();
        let path = tmp.path().join("state").join("downloads.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(store.load_downloads(), vec![]);
    }

    #[test]
    fn file_from_a_newer_build_loads_as_empty() {
        let (tmp, store) = store();
        let path = tmp.path().join("state").join("rooms.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"version": 99, "data": ["x"]}"#).unwrap();
        assert_eq!(store.load_rooms(), Vec::<String>::new());
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
