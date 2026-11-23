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

/// The last component of a remote path, unsanitized.
///
/// The library appends the filename itself (its `resolve_download_path` joins
/// the configured directory with this), so this is the name that actually
/// lands on disk; sanitizing it here would report a file that never exists.
/// Splitting on both separators keeps it a single component, which is why the
/// raw name cannot widen the path the sanitized directory chose.
fn peer_filename(remote_path: &str) -> &str {
    remote_path
        .split(['/', '\\'])
        .next_back()
        .unwrap_or(remote_path)
}

/// The directory to hand the library for a remote file: [`download_path_for`]
/// without its last component, in the `String` form its download API takes.
#[must_use]
pub fn download_dir_for(
    download_dir: &str,
    username: &str,
    remote_path: &str,
    per_user: bool,
    preserve_remote_dirs: bool,
) -> String {
    download_path_for(
        download_dir,
        username,
        remote_path,
        per_user,
        preserve_remote_dirs,
    )
    .parent()
    .unwrap_or_else(|| std::path::Path::new("."))
    .display()
    .to_string()
}

/// Where a download lands, in the two halves the library needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// The `download_directory` passed to the library; peer names sanitized.
    pub directory: String,
    /// The file written there. Its folders are the sanitized ones above, its
    /// filename the peer's raw one, which the library picks independently.
    pub file: PathBuf,
}

/// Resolve both halves of a remote file's destination.
#[must_use]
pub fn destination_for(
    download_dir: &str,
    username: &str,
    remote_path: &str,
    per_user: bool,
    preserve_remote_dirs: bool,
) -> Destination {
    let mut file = download_path_for(
        download_dir,
        username,
        remote_path,
        per_user,
        preserve_remote_dirs,
    );
    let directory = file
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .display()
        .to_string();
    file.set_file_name(peer_filename(remote_path));
    Destination { directory, file }
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
    use super::{
        destination_for, download_dir_for, download_path_for,
        resolve_shared_directory,
    };
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

    #[test]
    fn the_directory_handed_to_the_library_is_the_sanitized_parent() {
        assert_eq!(
            download_dir_for(
                "/dl",
                "bob",
                "@@share\\Album: One\\track.flac",
                true,
                true,
            ),
            "/dl/bob/@@share/Album_ One"
        );
        // Without the folders there is nothing to keep, and the download root
        // is still what gets created and written into.
        assert_eq!(
            download_dir_for("/dl", "bob", "track.flac", false, false),
            "/dl"
        );
    }

    #[test]
    fn the_written_file_keeps_the_peers_filename() {
        // The library joins the directory with the basename itself, so the
        // name it writes is the peer's raw one — sanitizing it here would
        // report a file that never exists.
        let destination = destination_for(
            "/dl",
            "bob",
            "@@share\\Album: One\\greatest hits? vol:2.flac",
            false,
            true,
        );
        assert_eq!(destination.directory, "/dl/@@share/Album_ One");
        assert_eq!(
            destination.file,
            std::path::PathBuf::from(
                "/dl/@@share/Album_ One/greatest hits? vol:2.flac"
            )
        );
    }

    #[test]
    fn a_filename_carrying_separators_is_still_only_one_component() {
        let destination = destination_for(
            "/dl",
            "bob",
            "../../outside\\evil.exe",
            false,
            true,
        );
        assert_eq!(destination.directory, "/dl/.._/.._/outside");
        assert_eq!(
            destination.file,
            std::path::PathBuf::from("/dl/.._/.._/outside/evil.exe")
        );
    }
}
