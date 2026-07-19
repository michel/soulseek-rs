//! An in-memory index of the files we share with other peers.
//!
//! Scanning a directory produces a read-only [`Shares`] snapshot keyed by the
//! peer-facing *virtual path* (the shared directory's own name followed by the
//! backslash-separated relative path, matching the Soulseek wire convention).

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

/// One shared file: its peer-facing virtual path and where it lives on disk.
#[derive(Debug, Clone)]
pub struct SharedFile {
    /// Backslash-separated path exposed to peers, e.g. `music\album\song.mp3`.
    pub virtual_path: String,
    /// The real filesystem path used to serve the bytes.
    pub real_path: PathBuf,
    pub size: u64,
    /// Optional `(code, value)` audio attributes (empty for now).
    pub attributes: Vec<(u32, u32)>,
}

/// A read-only snapshot of the shared files, cheap to clone behind an `Arc`.
#[derive(Debug, Default)]
pub struct Shares {
    files: Vec<SharedFile>,
    by_virtual: HashMap<String, usize>,
    folder_count: usize,
}

impl Shares {
    /// An empty share set (nothing shared).
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Scan `root` recursively into a share index. Symlinks and unreadable
    /// entries are skipped so a single bad entry never aborts the scan.
    ///
    /// # Errors
    /// Returns an error only if `root` itself cannot be read.
    pub fn scan(root: &Path) -> io::Result<Self> {
        // Fail fast if the root is unreadable; deeper failures are skipped.
        let _ = std::fs::read_dir(root)?;

        let root_name = root.file_name().map_or_else(
            || "shared".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );

        let mut files = Vec::new();
        let mut folders_with_files: HashSet<PathBuf> = HashSet::new();
        let mut stack = vec![root.to_path_buf()];

        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                // `entry.metadata()` does not traverse the symlink itself.
                if meta.file_type().is_symlink() {
                    continue;
                }
                let path = entry.path();
                if meta.is_dir() {
                    stack.push(path);
                } else if meta.is_file() {
                    let virtual_path =
                        virtual_path_for(&root_name, root, &path);
                    files.push(SharedFile {
                        virtual_path,
                        real_path: path,
                        size: meta.len(),
                        attributes: Vec::new(),
                    });
                    folders_with_files.insert(dir.clone());
                }
            }
        }

        let by_virtual = files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.virtual_path.clone(), i))
            .collect();

        Ok(Self {
            files,
            by_virtual,
            folder_count: folders_with_files.len(),
        })
    }

    /// Files whose virtual path contains *every* whitespace-separated term of
    /// `query` (case-insensitive). An empty query matches nothing.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&SharedFile> {
        let terms: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(String::from)
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }
        self.files
            .iter()
            .filter(|f| {
                let haystack = f.virtual_path.to_lowercase();
                terms.iter().all(|t| haystack.contains(t.as_str()))
            })
            .collect()
    }

    /// Look up a shared file by its exact virtual path.
    #[must_use]
    pub fn get(&self, virtual_path: &str) -> Option<&SharedFile> {
        self.by_virtual.get(virtual_path).map(|&i| &self.files[i])
    }

    /// All shared files, in scan order.
    #[must_use]
    pub fn files(&self) -> &[SharedFile] {
        &self.files
    }

    /// Files grouped by their virtual directory (everything before the final
    /// backslash), as `(directory, [(basename, size)])`. Used to build a
    /// SharedFileListResponse.
    #[must_use]
    pub fn directories(&self) -> Vec<(String, Vec<(String, u64)>)> {
        let mut by_dir: std::collections::BTreeMap<String, Vec<(String, u64)>> =
            std::collections::BTreeMap::new();
        for file in &self.files {
            let (dir, base) = file
                .virtual_path
                .rsplit_once('\\')
                .unwrap_or(("", file.virtual_path.as_str()));
            by_dir
                .entry(dir.to_string())
                .or_default()
                .push((base.to_string(), file.size));
        }
        by_dir.into_iter().collect()
    }

    #[must_use]
    pub const fn file_count(&self) -> u32 {
        self.files.len() as u32
    }

    #[must_use]
    pub const fn folder_count(&self) -> u32 {
        self.folder_count as u32
    }
}

/// Build the peer-facing virtual path for `path` under `root`: the root's own
/// name followed by the backslash-separated components relative to it.
fn virtual_path_for(root_name: &str, root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let mut parts = vec![root_name.to_string()];
    for component in rel.components() {
        parts.push(component.as_os_str().to_string_lossy().into_owned());
    }
    parts.join("\\")
}

#[cfg(test)]
mod tests {
    use super::Shares;

    fn temp_tree() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir()
            .join(format!("soulseek-shares-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("album")).unwrap();
        std::fs::write(root.join("top.mp3"), b"aaaa").unwrap();
        std::fs::write(root.join("album").join("song one.flac"), b"bbbbbb")
            .unwrap();
        std::fs::write(root.join("album").join("song two.flac"), b"cc")
            .unwrap();
        root
    }

    #[test]
    fn scan_counts_files_and_folders() {
        let root = temp_tree();
        let shares = Shares::scan(&root).unwrap();
        assert_eq!(shares.file_count(), 3);
        // Two folders contain files: the root and `album`.
        assert_eq!(shares.folder_count(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn nested_files_use_backslash_virtual_paths() {
        let root = temp_tree();
        let base = root.file_name().unwrap().to_string_lossy().into_owned();
        let shares = Shares::scan(&root).unwrap();

        let vpath = format!("{base}\\album\\song one.flac");
        let file = shares.get(&vpath).expect("nested file indexed");
        assert_eq!(file.size, 6);
        assert!(shares.get("does\\not\\exist").is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn search_is_case_insensitive_and_multi_term_and() {
        let root = temp_tree();
        let shares = Shares::scan(&root).unwrap();

        // Both album files match "SONG"; "song one" narrows to one.
        assert_eq!(shares.search("SONG").len(), 2);
        assert_eq!(shares.search("song ONE").len(), 1);
        assert!(shares.search("nonexistent").is_empty());
        assert!(shares.search("").is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn empty_shares_have_no_files_or_folders() {
        let shares = Shares::empty();
        assert_eq!(shares.file_count(), 0);
        assert_eq!(shares.folder_count(), 0);
        assert!(shares.search("anything").is_empty());
    }
}
