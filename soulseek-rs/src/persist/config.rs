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
    /// A daemon to control instead of opening a session of our own —
    /// `host:port`, or a Unix socket path. Unset means "use the local one if
    /// there is one", which is what a daemon on this machine wants.
    pub daemon: Option<String>,
    /// The token for a remote daemon. Not a secret about *this* account, and
    /// useless without reaching the host, so unlike a password it is
    /// reasonable to keep here rather than behind a command.
    pub daemon_token: Option<String>,
    /// Standing searches the server lets us repeat once per wishlist interval.
    /// A TOML array rather than a `config set` key, because a query may itself
    /// contain a comma.
    pub wishlist: Option<Vec<String>>,
}

impl FileConfig {
    /// Every setting `config get`/`set` accepts, in the order `config list`
    /// prints them. Keys match the TOML field names exactly, so what a script
    /// reads back is what it would have written by hand.
    pub const KEYS: &'static [&'static str] = &[
        "username",
        "server",
        "listener_port",
        "disable_listener",
        "download_dir",
        "shared_dirs",
        "max_concurrent_downloads",
        "search_timeout",
        "password_cmd",
        "daemon",
        "daemon_token",
    ];

    /// The value of `key`, or `None` when it is unset or unknown. Lists are
    /// rendered comma-separated, matching what [`Self::set`] accepts.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "username" => self.username.clone(),
            "server" => self.server.clone(),
            "listener_port" => self.listener_port.map(|v| v.to_string()),
            "disable_listener" => self.disable_listener.map(|v| v.to_string()),
            "download_dir" => self.download_dir.clone(),
            "shared_dirs" => {
                let dirs = self.shared_dirs.clone().unwrap_or_default();
                (!dirs.is_empty()).then(|| dirs.join(","))
            }
            "max_concurrent_downloads" => {
                self.max_concurrent_downloads.map(|v| v.to_string())
            }
            "search_timeout" => self.search_timeout.map(|v| v.to_string()),
            "password_cmd" => self.password_cmd.clone(),
            "daemon" => self.daemon.clone(),
            "daemon_token" => self.daemon_token.clone(),
            _ => None,
        }
    }

    /// Set `key` to `value`, or clear it when `value` is empty.
    ///
    /// # Errors
    /// Returns a message naming the problem when the key is unknown or the
    /// value does not parse as that setting's type.
    pub fn set(
        &mut self,
        key: &str,
        value: &str,
    ) -> std::result::Result<(), String> {
        let value = value.trim();
        let clear = value.is_empty();
        let text = |target: &mut Option<String>| {
            *target = (!clear).then(|| value.to_string());
        };
        match key {
            "username" => text(&mut self.username),
            "server" => text(&mut self.server),
            "download_dir" => text(&mut self.download_dir),
            "password_cmd" => text(&mut self.password_cmd),
            "daemon" => text(&mut self.daemon),
            "daemon_token" => text(&mut self.daemon_token),
            "listener_port" => {
                self.listener_port = parse_opt(value, clear, key)?;
            }
            "max_concurrent_downloads" => {
                self.max_concurrent_downloads = parse_opt(value, clear, key)?;
            }
            "search_timeout" => {
                self.search_timeout = parse_opt(value, clear, key)?;
            }
            "disable_listener" => {
                self.disable_listener = if clear {
                    None
                } else {
                    Some(match value {
                        "true" | "1" | "yes" => true,
                        "false" | "0" | "no" => false,
                        other => {
                            return Err(format!(
                                "disable_listener wants true or false, got \
                                 '{other}'"
                            ));
                        }
                    })
                };
            }
            "shared_dirs" => {
                self.shared_dirs = (!clear).then(|| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|dir| !dir.is_empty())
                        .map(String::from)
                        .collect()
                });
                // The single-folder spelling would otherwise shadow the list.
                self.shared_dir = None;
            }
            other => {
                return Err(format!(
                    "unknown setting '{other}' — try one of: {}",
                    Self::KEYS.join(", ")
                ));
            }
        }
        Ok(())
    }

    /// The standing searches, in the order they were added.
    #[must_use]
    pub fn wishes(&self) -> Vec<String> {
        self.wishlist.clone().unwrap_or_default()
    }

    /// Add `query` to the wishlist. Returns whether it changed anything: a
    /// blank query and a case-insensitive repeat are both refused, so adding
    /// the same wish twice cannot fill the file with duplicates the server
    /// would then be asked about twice per interval.
    pub fn add_wish(&mut self, query: &str) -> bool {
        let query = query.trim();
        if query.is_empty() || self.find_wish(query).is_some() {
            return false;
        }
        self.wishlist
            .get_or_insert_with(Vec::new)
            .push(query.to_string());
        true
    }

    /// Remove `query`, matched the same way [`Self::add_wish`] compares.
    /// Returns whether it was there.
    pub fn remove_wish(&mut self, query: &str) -> bool {
        let Some(index) = self.find_wish(query.trim()) else {
            return false;
        };
        self.wishlist.get_or_insert_with(Vec::new).remove(index);
        true
    }

    /// The stored wish matching `query`, compared the way [`Self::add_wish`]
    /// compares. Callers that need to act on a named wish go through this so
    /// "the same wish" means one thing everywhere.
    #[must_use]
    pub fn wish(&self, query: &str) -> Option<&String> {
        let index = self.find_wish(query.trim())?;
        self.wishlist.as_ref()?.get(index)
    }

    fn find_wish(&self, query: &str) -> Option<usize> {
        let wanted = query.to_lowercase();
        self.wishlist
            .as_ref()?
            .iter()
            .position(|wish| wish.to_lowercase() == wanted)
    }

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

/// Parse a setting that is not a string, turning a bad value into a message
/// naming the key rather than a bare parse error.
fn parse_opt<T: std::str::FromStr>(
    value: &str,
    clear: bool,
    key: &str,
) -> std::result::Result<Option<T>, String> {
    if clear {
        return Ok(None);
    }
    value
        .parse::<T>()
        .map(Some)
        .map_err(|_| format!("{key} does not accept '{value}'"))
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
    pub daemon: Option<String>,
    pub daemon_token: Option<String>,
}

pub const DEFAULT_SERVER: &str = "server.slsknet.org:2416";
pub const DEFAULT_LISTENER_PORT: u16 = 2234;
/// How many downloads run at once by default.
///
/// Downstream is the abundant direction, and a Soulseek transfer is paced by
/// the sending peer — usually a few hundred KiB/s — so filling a modern link
/// takes many transfers at once, not faster ones. Twenty at a typical peer rate
/// is tens of MiB/s, and costs twenty threads and sockets, which is nothing.
pub const DEFAULT_MAX_CONCURRENT_DOWNLOADS: usize = 20;
pub const DEFAULT_SEARCH_TIMEOUT: u64 = 10;

/// Layer CLI/env values over the config file over defaults.
///
/// The listener has an explicit form in both directions: `--listener` beats
/// `--no-listener` (and `SOULSEEK_NO_LISTENER`), which beats the file's
/// `disable_listener`, so a config file can always be overridden per run.
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
        disable_listener: if cli.listener {
            false
        } else {
            cli.no_listener || file.disable_listener.unwrap_or(false)
        },
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
        password_cmd: cli
            .password_cmd
            .clone()
            .or_else(|| file.password_cmd.clone()),
        daemon: cli.daemon.clone().or_else(|| file.daemon.clone()),
        daemon_token: cli
            .daemon_token
            .clone()
            .or_else(|| file.daemon_token.clone()),
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
    if cli.shared_dir.is_empty()
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
        Cli::default()
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
        cli.shared_dir = vec!["/a".into()];
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
    fn repeated_cli_shared_dirs_are_all_kept() {
        let mut cli = bare_cli();
        cli.shared_dir = vec!["/a".into(), "/b".into()];
        let resolved = resolve(&cli, &FileConfig::default());
        assert_eq!(
            resolved.shared_dirs,
            vec!["/a".to_string(), "/b".to_string()]
        );
    }

    #[test]
    fn an_empty_cli_shared_dir_disables_sharing() {
        let mut cli = bare_cli();
        cli.shared_dir = vec![String::new()];
        let resolved = resolve(&cli, &FileConfig::default());
        assert!(resolved.shared_dirs.is_empty());
    }

    #[test]
    fn no_listener_flag_disables_the_listener() {
        let mut cli = bare_cli();
        cli.no_listener = true;
        assert!(resolve(&cli, &FileConfig::default()).disable_listener);
    }

    #[test]
    fn listener_flag_overrides_the_config_file() {
        let file = FileConfig {
            disable_listener: Some(true),
            ..FileConfig::default()
        };
        assert!(resolve(&bare_cli(), &file).disable_listener);

        let mut cli = bare_cli();
        cli.listener = true;
        assert!(!resolve(&cli, &file).disable_listener);
    }

    #[test]
    fn listener_flag_wins_over_no_listener() {
        let mut cli = bare_cli();
        cli.listener = true;
        cli.no_listener = true;
        assert!(!resolve(&cli, &FileConfig::default()).disable_listener);
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
            daemon: Some("nas.local:5030".into()),
            daemon_token: Some("deadbeef".into()),
            wishlist: None,
        };
        let resolved = resolve(&bare_cli(), &file);
        assert_eq!(resolved.username.as_deref(), Some("alice"));
        assert_eq!(resolved.server, "localhost:2242");
        assert_eq!(resolved.listener_port, 4321);
        assert!(resolved.disable_listener);
        assert_eq!(resolved.download_dir, "/music");
        assert_eq!(resolved.shared_dirs, vec!["/shared".to_string()]);
        assert_eq!(resolved.daemon.as_deref(), Some("nas.local:5030"));
        assert_eq!(resolved.daemon_token.as_deref(), Some("deadbeef"));
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
    fn every_advertised_key_round_trips_through_get_and_set() {
        let mut config = FileConfig::default();
        for key in FileConfig::KEYS {
            assert!(config.get(key).is_none(), "{key} should start unset");
        }

        for (key, value) in [
            ("username", "alice"),
            ("server", "localhost:2242"),
            ("listener_port", "4321"),
            ("disable_listener", "true"),
            ("download_dir", "/music"),
            ("shared_dirs", "/a,/b"),
            ("max_concurrent_downloads", "3"),
            ("search_timeout", "30"),
            ("password_cmd", "pass show slsk"),
        ] {
            config.set(key, value).expect("should accept");
            assert_eq!(
                config.get(key).as_deref(),
                Some(value),
                "{key} should read back what was written"
            );
        }
        assert_eq!(config.shared_dirs, Some(vec!["/a".into(), "/b".into()]));
    }

    #[test]
    fn an_empty_value_clears_a_setting() {
        let mut config = FileConfig {
            username: Some("alice".into()),
            listener_port: Some(2234),
            ..FileConfig::default()
        };
        config.set("username", "").unwrap();
        config.set("listener_port", "").unwrap();
        assert_eq!(config.username, None);
        assert_eq!(config.listener_port, None);
    }

    #[test]
    fn a_value_of_the_wrong_type_is_rejected_by_name() {
        let mut config = FileConfig::default();
        let error = config.set("listener_port", "not-a-port").unwrap_err();
        assert!(error.contains("listener_port"), "got {error}");
        assert!(config.get("listener_port").is_none(), "must not be set");

        let error = config.set("disable_listener", "maybe").unwrap_err();
        assert!(error.contains("true or false"), "got {error}");
    }

    #[test]
    fn an_unknown_key_lists_the_ones_that_exist() {
        let mut config = FileConfig::default();
        let error = config.set("colour", "blue").unwrap_err();
        assert!(error.contains("unknown setting"), "got {error}");
        assert!(error.contains("username"), "should suggest real keys");
        assert!(config.get("colour").is_none());
    }

    #[test]
    fn setting_the_share_list_drops_the_single_folder_spelling() {
        // Both spellings surviving would make the effective share set depend
        // on resolution order rather than on what was just written.
        let mut config = FileConfig {
            shared_dir: Some("/old".into()),
            ..FileConfig::default()
        };
        config.set("shared_dirs", "/new").unwrap();
        assert_eq!(config.shared_dir, None);
        assert_eq!(config.shared_dirs, Some(vec!["/new".to_string()]));
    }

    #[test]
    fn a_set_value_survives_a_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = FileConfig::default();
        config.set("download_dir", "/music").unwrap();
        config.set("max_concurrent_downloads", "2").unwrap();
        config.save(&path).unwrap();

        let loaded = FileConfig::load(&path).unwrap();
        assert_eq!(loaded.get("download_dir").as_deref(), Some("/music"));
        assert_eq!(
            loaded.get("max_concurrent_downloads").as_deref(),
            Some("2")
        );
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
    fn a_wish_is_added_once_however_it_is_spelled() {
        let mut config = FileConfig::default();
        assert!(config.add_wish("Aphex Twin"));
        assert!(!config.add_wish("aphex twin"), "a repeat is not a new wish");
        assert!(!config.add_wish("  Aphex Twin  "), "surrounding space too");
        assert_eq!(config.wishes(), ["Aphex Twin"]);
    }

    #[test]
    fn an_empty_wish_is_refused() {
        let mut config = FileConfig::default();
        assert!(!config.add_wish("   "));
        assert!(config.wishes().is_empty());
    }

    #[test]
    fn removing_a_wish_ignores_case_and_reports_whether_it_was_there() {
        let mut config = FileConfig::default();
        config.add_wish("Boards of Canada");
        assert!(config.remove_wish("BOARDS OF CANADA"));
        assert!(config.wishes().is_empty());
        assert!(!config.remove_wish("never added"));
    }

    #[test]
    fn wishes_keep_the_order_they_were_added_in() {
        let mut config = FileConfig::default();
        for wish in ["first", "second", "third"] {
            config.add_wish(wish);
        }
        config.remove_wish("second");
        assert_eq!(config.wishes(), ["first", "third"]);
    }

    #[test]
    fn a_wishlist_survives_a_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = FileConfig::default();
        config.add_wish("autechre, incunabula");
        config.add_wish("plaid");
        config.save(&path).unwrap();
        assert_eq!(
            FileConfig::load(&path).unwrap().wishes(),
            ["autechre, incunabula", "plaid"],
            "a comma inside a wish must survive the round trip"
        );
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
