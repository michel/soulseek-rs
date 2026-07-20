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
    project_dirs().map(|d| d.config_dir().join("config.toml"))
}

#[must_use]
pub fn state_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SOULSEEK_STATE_DIR") {
        return Some(PathBuf::from(dir));
    }
    project_dirs().map(|d| d.data_dir().join("state"))
}

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "soulseek-rs")
}
