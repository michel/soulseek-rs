//! Resolution and validation of the configured download / shared directories.

use soulseek_rs::utils::path::expand_tilde;
use std::path::PathBuf;

/// Validate and normalize an optional shared-directory setting.
///
/// Returns `Ok(None)` when nothing is shared (unset or blank), `Ok(Some(path))`
/// for a `~`-expanded, existing directory, or `Err` with a human-readable
/// message when the configured path is not a usable directory.
pub fn resolve_shared_directory(
    raw: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let path = expand_tilde(trimmed);
    if !path.exists() {
        return Err(format!(
            "shared directory does not exist: {}",
            path.display()
        ));
    }
    if !path.is_dir() {
        return Err(format!(
            "shared path is not a directory: {}",
            path.display()
        ));
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::resolve_shared_directory;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn unique_temp_dir() -> std::path::PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("soulseek-rs-dirtest-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn unset_or_blank_shares_nothing() {
        assert_eq!(resolve_shared_directory(None), Ok(None));
        assert_eq!(resolve_shared_directory(Some("")), Ok(None));
        assert_eq!(resolve_shared_directory(Some("   ")), Ok(None));
    }

    #[test]
    fn existing_directory_is_accepted() {
        let dir = unique_temp_dir();
        let resolved =
            resolve_shared_directory(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(resolved, Some(dir.clone()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_directory_is_rejected() {
        let missing = std::env::temp_dir().join("soulseek-rs-does-not-exist-x");
        let _ = std::fs::remove_dir_all(&missing);
        let result = resolve_shared_directory(Some(missing.to_str().unwrap()));
        assert!(result.is_err());
    }

    #[test]
    fn a_file_is_not_a_valid_shared_directory() {
        let dir = unique_temp_dir();
        let file = dir.join("not-a-dir.txt");
        std::fs::write(&file, b"x").unwrap();
        let result = resolve_shared_directory(Some(file.to_str().unwrap()));
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
