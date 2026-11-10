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

/// Validate several configured share paths; invalid ones are skipped with a
/// warning instead of aborting (the client just shares less).
#[must_use]
pub fn resolve_shared_directories(dirs: &[String]) -> Vec<String> {
    let mut resolved = Vec::new();
    for dir in dirs {
        match resolve_shared_directory(Some(dir)) {
            Ok(Some(path)) => resolved.push(path.display().to_string()),
            Ok(None) => {}
            Err(e) => eprintln!("⚠️  Ignoring shared directory: {e}"),
        }
    }
    resolved
}

/// A peer-chosen name made safe as one path component: separators and
/// characters Windows refuses become `_`, so `../..` cannot climb out.
fn sanitize_path_component(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    if out.is_empty() || out.ends_with(['.', ' ']) {
        out.push('_');
    }
    if is_reserved(&out) {
        out.insert(0, '_');
    }
    out
}

/// Resolve the destination for a remote file, optionally adding its peer and
/// path beneath the configured download root. Remote separators are normalized
/// on every platform, and every peer-controlled component is sanitized.
#[must_use]
pub fn download_path_for(
    download_dir: &str,
    username: &str,
    remote_path: &str,
    per_user: bool,
    preserve_remote_dirs: bool,
) -> PathBuf {
    let mut path = expand_tilde(download_dir);
    if per_user {
        path.push(sanitize_path_component(username));
    }

    let components: Vec<&str> = remote_path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    if preserve_remote_dirs {
        path.extend(
            components.iter().map(|part| sanitize_path_component(part)),
        );
    } else if let Some(name) = components.last() {
        path.push(sanitize_path_component(name));
    } else {
        path.push("_");
    }
    path
}

/// Windows device names (`CON`, `COM1`, …) that cannot be a folder name.
fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (matches!(stem.get(..3), Some("COM" | "LPT"))
            && matches!(stem.as_bytes().get(3..), Some([b'1'..=b'9'])))
}

#[cfg(test)]
mod tests {
    use super::{download_path_for, resolve_shared_directory};
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

    #[test]
    fn download_path_flags_compose_independently() {
        let remote = "@@share\\Music\\Artist\\Album\\track.flac";
        assert_eq!(
            download_path_for("/dl", "bob", remote, false, false),
            std::path::PathBuf::from("/dl/track.flac")
        );
        assert_eq!(
            download_path_for("/dl", "bob", remote, true, false),
            std::path::PathBuf::from("/dl/bob/track.flac")
        );
        assert_eq!(
            download_path_for("/dl", "bob", remote, false, true),
            std::path::PathBuf::from(
                "/dl/@@share/Music/Artist/Album/track.flac"
            )
        );
        assert_eq!(
            download_path_for("/dl", "bob", remote, true, true),
            std::path::PathBuf::from(
                "/dl/bob/@@share/Music/Artist/Album/track.flac"
            )
        );
    }

    #[test]
    fn remote_path_components_cannot_escape_download_root() {
        let path = download_path_for(
            "/dl",
            "bob",
            "../../outside\\\\CON\\\\track.mp3",
            false,
            true,
        );
        assert_eq!(
            path,
            std::path::PathBuf::from("/dl/.._/.._/outside/_CON/track.mp3")
        );
    }

    #[test]
    fn a_hostile_username_cannot_leave_the_download_dir() {
        assert_eq!(
            download_path_for("/dl", "../..", "track.mp3", true, false),
            std::path::PathBuf::from("/dl/.._.._/track.mp3")
        );
        assert_eq!(
            download_path_for("/dl", "/tmp/x", "track.mp3", true, false),
            std::path::PathBuf::from("/dl/_tmp_x/track.mp3")
        );
        assert_eq!(
            download_path_for("/dl", "..", "track.mp3", true, false),
            std::path::PathBuf::from("/dl/.._/track.mp3")
        );
        assert_eq!(
            download_path_for("/dl", "", "track.mp3", true, false),
            std::path::PathBuf::from("/dl/_/track.mp3")
        );
        assert_eq!(
            download_path_for("/dl", "com1", "track.mp3", true, false),
            std::path::PathBuf::from("/dl/_com1/track.mp3")
        );
    }
}
