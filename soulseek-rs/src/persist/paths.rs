#[cfg(not(target_os = "macos"))]
use directories::ProjectDirs;
use std::path::PathBuf;

/// Platform-appropriate locations for the config file and state directory.
/// Overridable via `SOULSEEK_CONFIG_DIR` / `SOULSEEK_STATE_DIR` (used by the
/// e2e tests, and handy for portable installs).
#[must_use]
pub fn config_file() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SOULSEEK_CONFIG_DIR") {
        return Some(PathBuf::from(dir).join("config.toml"));
    }
    config_dir().map(|d| d.join("config.toml"))
}

#[must_use]
pub fn state_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SOULSEEK_STATE_DIR") {
        return Some(PathBuf::from(dir));
    }
    data_dir().map(|d| d.join("state"))
}

// ponytail: macOS follows the XDG layout (~/.config, ~/.local/share) instead of
// Application Support, by request. Other platforms keep the crate defaults.
#[cfg(target_os = "macos")]
fn config_dir() -> Option<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

#[cfg(target_os = "macos")]
fn data_dir() -> Option<PathBuf> {
    xdg_dir("XDG_DATA_HOME", ".local/share")
}

#[cfg(target_os = "macos")]
fn xdg_dir(env: &str, fallback: &str) -> Option<PathBuf> {
    let base = match std::env::var_os(env) {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => directories::UserDirs::new()?.home_dir().join(fallback),
    };
    Some(base.join("soulseek-rs"))
}

#[cfg(not(target_os = "macos"))]
fn config_dir() -> Option<PathBuf> {
    project_dirs().map(|d| d.config_dir().to_path_buf())
}

#[cfg(not(target_os = "macos"))]
fn data_dir() -> Option<PathBuf> {
    project_dirs().map(|d| d.data_dir().to_path_buf())
}

#[cfg(not(target_os = "macos"))]
fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "soulseek-rs")
}

/// Default download (and shared) folder: a `Soulseek` subfolder of the
/// platform Downloads directory (XDG on Linux, known-folder on Windows),
/// so Soulseek files don't clutter the Downloads root.
#[must_use]
pub fn default_download_dir() -> String {
    let user_dirs = directories::UserDirs::new();
    let downloads = user_dirs
        .as_ref()
        .and_then(|dirs| dirs.download_dir().map(std::path::Path::to_path_buf))
        .or_else(|| {
            user_dirs
                .as_ref()
                .map(|dirs| dirs.home_dir().join("Downloads"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("Downloads"));
    downloads.join("Soulseek").display().to_string()
}
