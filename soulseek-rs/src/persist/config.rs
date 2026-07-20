use color_eyre::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Optional settings read from `config.toml`. Every field is optional so a
/// partial file (or none at all) is valid; unknown keys are ignored so newer
/// configs still load in older builds.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileConfig {
    pub username: Option<String>,
    pub server: Option<String>,
    pub listener_port: Option<u16>,
    pub disable_listener: Option<bool>,
    pub download_dir: Option<String>,
    /// Single shared folder (also what `--shared-dir` sets). Prefer
    /// `shared_dirs` for multiple; both may be combined.
    pub shared_dir: Option<String>,
    /// Multiple shared folders. An explicitly empty list disables sharing.
    pub shared_dirs: Option<Vec<String>>,
    pub max_concurrent_downloads: Option<usize>,
    pub search_timeout: Option<u64>,
    /// Command whose stdout is the password (headless fallback, like mutt's
    /// `password_cmd`). Never store the password itself in the file.
    pub password_cmd: Option<String>,
}

impl FileConfig {
    /// Load from `path`; a missing file is an empty config, a malformed file
    /// is an error (silently ignoring a typo'd config would be worse).
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(e) => {
                return Err(color_eyre::eyre::eyre!(
                    "Cannot read {}: {e}",
                    path.display()
                ));
            }
        };
        toml::from_str(&text).map_err(|e| {
            color_eyre::eyre::eyre!("Malformed {}: {e}", path.display())
        })
    }

    /// Save to `path`, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// Fully-resolved settings after layering CLI (which already includes env via
/// clap) over the config file over built-in defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub username: Option<String>,
    pub server: String,
    pub listener_port: u16,
    pub disable_listener: bool,
    pub download_dir: String,
    pub shared_dirs: Vec<String>,
    pub max_concurrent_downloads: usize,
    pub search_timeout: u64,
    pub password_cmd: Option<String>,
}

pub const DEFAULT_SERVER: &str = "server.slsknet.org:2416";
pub const DEFAULT_LISTENER_PORT: u16 = 2234;
pub const DEFAULT_MAX_CONCURRENT_DOWNLOADS: usize = 5;
pub const DEFAULT_SEARCH_TIMEOUT: u64 = 10;

/// Layer CLI/env values over the config file over defaults.
///
/// The `--disable-listener` flag can only enable the disable (a bare flag
/// has no "explicitly off" form), so file `disable_listener = true` wins
/// unless the flag is passed.
#[must_use]
pub fn resolve(cli: &crate::cli::Cli, file: &FileConfig) -> Resolved {
    let download_dir = cli
        .download_dir
        .clone()
        .or_else(|| file.download_dir.clone())
        .unwrap_or_else(super::paths::default_download_dir);
    Resolved {
        username: cli.username.clone().or_else(|| file.username.clone()),
        server: cli
            .server
            .clone()
            .or_else(|| file.server.clone())
            .unwrap_or_else(|| DEFAULT_SERVER.to_string()),
        listener_port: cli
            .listener_port
            .or(file.listener_port)
            .unwrap_or(DEFAULT_LISTENER_PORT),
        disable_listener: cli.disable_listener
            || file.disable_listener.unwrap_or(false),
        download_dir: download_dir.clone(),
        shared_dirs: resolve_shared_dirs(cli, file, &download_dir),
        max_concurrent_downloads: cli
            .max_concurrent_downloads
            .or(file.max_concurrent_downloads)
            .unwrap_or(DEFAULT_MAX_CONCURRENT_DOWNLOADS),
        search_timeout: cli
            .search_timeout
            .or(file.search_timeout)
            .unwrap_or(DEFAULT_SEARCH_TIMEOUT),
        password_cmd: file.password_cmd.clone(),
    }
}

/// Sharing follows the Soulseek convention of sharing what you download:
/// with nothing configured, the download folder is shared. Configuring any
/// of `--shared-dir` / `shared_dir` / `shared_dirs` replaces that default
/// (their non-empty values are combined), and an explicitly empty value
/// (`shared_dir = ""` or `shared_dirs = []`) disables sharing entirely.
fn resolve_shared_dirs(
    cli: &crate::cli::Cli,
    file: &FileConfig,
    download_dir: &str,
) -> Vec<String> {
    if cli.shared_dir.is_none()
        && file.shared_dir.is_none()
        && file.shared_dirs.is_none()
    {
        return vec![download_dir.to_string()];
    }
    let mut dirs: Vec<String> = Vec::new();
    let singles = cli.shared_dir.iter().chain(file.shared_dir.iter());
    for dir in singles.chain(file.shared_dirs.iter().flatten()) {
        let dir = dir.trim();
        if !dir.is_empty() && !dirs.iter().any(|d| d == dir) {
            dirs.push(dir.to_string());
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;

    fn bare_cli() -> Cli {
        Cli {
            username: None,
            password: None,
            server: None,
            disable_listener: false,
            listener_port: None,
            verbose: 0,
            log_file: None,
            command: None,
            download_dir: None,
            shared_dir: None,
            max_concurrent_downloads: None,
            search_timeout: None,
        }
    }

    #[test]
    fn defaults_apply_when_cli_and_file_are_empty() {
        let resolved = resolve(&bare_cli(), &FileConfig::default());
        assert_eq!(resolved.server, DEFAULT_SERVER);
        assert_eq!(resolved.listener_port, DEFAULT_LISTENER_PORT);
        assert_eq!(
            resolved.max_concurrent_downloads,
            DEFAULT_MAX_CONCURRENT_DOWNLOADS
        );
        assert_eq!(resolved.search_timeout, DEFAULT_SEARCH_TIMEOUT);
        assert!(!resolved.disable_listener);
        assert_eq!(resolved.username, None);
    }

    #[test]
    fn default_download_dir_is_a_soulseek_folder_under_downloads() {
        let resolved = resolve(&bare_cli(), &FileConfig::default());
        let path = std::path::Path::new(&resolved.download_dir);
        assert!(path.is_absolute(), "must not rely on ~ expansion");
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("Soulseek"));
        assert_eq!(
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str()),
            Some("Downloads")
        );
    }

    #[test]
    fn sharing_defaults_to_the_download_dir() {
        let resolved = resolve(&bare_cli(), &FileConfig::default());
        assert_eq!(resolved.shared_dirs, vec![resolved.download_dir.clone()]);
    }

    #[test]
    fn sharing_follows_a_customized_download_dir() {
        let file = FileConfig {
            download_dir: Some("/music".into()),
            ..FileConfig::default()
        };
        let resolved = resolve(&bare_cli(), &file);
        assert_eq!(resolved.shared_dirs, vec!["/music".to_string()]);
    }

    #[test]
    fn empty_shared_dir_disables_sharing() {
        let file = FileConfig {
            shared_dir: Some(String::new()),
            ..FileConfig::default()
        };
        let resolved = resolve(&bare_cli(), &file);
        assert!(resolved.shared_dirs.is_empty());
    }

    #[test]
    fn empty_shared_dirs_list_disables_sharing() {
        let file = FileConfig {
            shared_dirs: Some(Vec::new()),
            ..FileConfig::default()
        };
        let resolved = resolve(&bare_cli(), &file);
        assert!(resolved.shared_dirs.is_empty());
    }

    #[test]
    fn shared_dirs_list_replaces_the_default_share() {
        let file = FileConfig {
            shared_dirs: Some(vec!["/a".into(), "/b".into()]),
            ..FileConfig::default()
        };
        let resolved = resolve(&bare_cli(), &file);
        assert_eq!(
            resolved.shared_dirs,
            vec!["/a".to_string(), "/b".to_string()]
        );
    }

    #[test]
    fn cli_shared_dir_combines_with_file_list_and_dedupes() {
        let mut cli = bare_cli();
        cli.shared_dir = Some("/a".into());
        let file = FileConfig {
            shared_dirs: Some(vec!["/a".into(), "/b".into()]),
            ..FileConfig::default()
        };
        let resolved = resolve(&cli, &file);
        assert_eq!(
            resolved.shared_dirs,
            vec!["/a".to_string(), "/b".to_string()]
        );
    }

    #[test]
    fn file_values_override_defaults() {
        let file = FileConfig {
            username: Some("alice".into()),
            server: Some("localhost:2242".into()),
            listener_port: Some(4321),
            disable_listener: Some(true),
            download_dir: Some("/music".into()),
            shared_dir: Some("/shared".into()),
            shared_dirs: None,
            max_concurrent_downloads: Some(2),
            search_timeout: Some(30),
            password_cmd: Some("pass show slsk".into()),
        };
        let resolved = resolve(&bare_cli(), &file);
        assert_eq!(resolved.username.as_deref(), Some("alice"));
        assert_eq!(resolved.server, "localhost:2242");
        assert_eq!(resolved.listener_port, 4321);
        assert!(resolved.disable_listener);
        assert_eq!(resolved.download_dir, "/music");
        assert_eq!(resolved.shared_dirs, vec!["/shared".to_string()]);
        assert_eq!(resolved.max_concurrent_downloads, 2);
        assert_eq!(resolved.search_timeout, 30);
        assert_eq!(resolved.password_cmd.as_deref(), Some("pass show slsk"));
    }

    #[test]
    fn cli_values_override_file_values() {
        let mut cli = bare_cli();
        cli.username = Some("cli-user".into());
        cli.server = Some("cli-server:1".into());
        cli.listener_port = Some(1111);
        cli.download_dir = Some("/cli-dl".into());
        let file = FileConfig {
            username: Some("file-user".into()),
            server: Some("file-server:2".into()),
            listener_port: Some(2222),
            download_dir: Some("/file-dl".into()),
            ..FileConfig::default()
        };
        let resolved = resolve(&cli, &file);
        assert_eq!(resolved.username.as_deref(), Some("cli-user"));
        assert_eq!(resolved.server, "cli-server:1");
        assert_eq!(resolved.listener_port, 1111);
        assert_eq!(resolved.download_dir, "/cli-dl");
    }

    #[test]
    fn missing_file_loads_as_empty_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = FileConfig::load(&dir.path().join("config.toml")).unwrap();
        assert_eq!(config, FileConfig::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let config = FileConfig {
            username: Some("alice".into()),
            server: Some("localhost:2242".into()),
            listener_port: Some(2234),
            max_concurrent_downloads: Some(3),
            ..FileConfig::default()
        };
        config.save(&path).unwrap();
        assert_eq!(FileConfig::load(&path).unwrap(), config);
    }

    #[test]
    fn unknown_keys_and_partial_files_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "username = \"bob\"\nfuture_option = true\n")
            .unwrap();
        let config = FileConfig::load(&path).unwrap();
        assert_eq!(config.username.as_deref(), Some("bob"));
        assert_eq!(config.server, None);
    }

    #[test]
    fn malformed_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "username = [unclosed").unwrap();
        assert!(FileConfig::load(&path).is_err());
    }
}
