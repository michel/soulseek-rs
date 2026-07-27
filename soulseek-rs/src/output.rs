//! Output discipline for the one-shot commands.
//!
//! stdout carries records and nothing else, stderr carries progress, and the
//! process exit code carries the verdict. Records render either as
//! newline-delimited JSON (`--json`) or as tab-separated text, so both `jq`
//! and `cut` are first-class consumers.

use serde::Serialize;
use std::io::{IsTerminal, Write};

/// Documented exit statuses. Anything a wrapper script needs to branch on has
/// its own code; clap already uses 2 for argument errors, so config problems
/// share it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Exit {
    Ok = 0,
    /// Something unexpected went wrong (also the panic/TUI bucket).
    Failure = 1,
    /// Bad arguments, missing credentials, unusable configuration.
    Usage = 2,
    /// Could not reach the server, or the server rejected the login.
    Connection = 3,
    /// The operation succeeded but produced nothing.
    NoResults = 4,
    /// Gave up waiting for a response or a transfer.
    Timeout = 5,
    /// A transfer started but did not complete.
    Transfer = 6,
    /// The server session ended mid-command, so nothing this run saw (or did
    /// not see) means anything. Retry rather than believe it.
    SessionLost = 7,
}

impl Exit {
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }
}

impl From<Exit> for std::process::ExitCode {
    fn from(exit: Exit) -> Self {
        Self::from(exit.code())
    }
}

/// A failure carrying the exit code it should produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub exit: Exit,
    pub message: String,
}

impl CliError {
    pub fn new(exit: Exit, message: impl Into<String>) -> Self {
        Self {
            exit,
            message: message.into(),
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(Exit::Usage, message)
    }

    pub fn connection(message: impl Into<String>) -> Self {
        Self::new(Exit::Connection, message)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(Exit::Timeout, message)
    }

    pub fn transfer(message: impl Into<String>) -> Self {
        Self::new(Exit::Transfer, message)
    }

    pub fn no_results(message: impl Into<String>) -> Self {
        Self::new(Exit::NoResults, message)
    }

    pub fn session_lost(message: impl Into<String>) -> Self {
        Self::new(Exit::SessionLost, message)
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

pub type CliResult<T = ()> = Result<T, CliError>;

/// One line of machine-readable output.
pub trait Record: Serialize {
    /// Tab-separated rendering. Fields must already be free of tabs and
    /// newlines — use [`sanitize`].
    fn text(&self) -> String;
}

/// Strip the field and record separators out of peer-supplied text so one
/// record always occupies exactly one line with stable columns.
#[must_use]
pub fn sanitize(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

/// Where records go and whether progress is shown.
#[derive(Clone)]
pub struct Out {
    json: bool,
    quiet: bool,
    color: bool,
}

impl Out {
    #[must_use]
    pub fn new(json: bool, quiet: bool) -> Self {
        Self {
            json,
            quiet,
            color: std::env::var_os("NO_COLOR").is_none()
                && std::io::stderr().is_terminal(),
        }
    }

    /// Write one record to stdout.
    pub fn emit<R: Record>(&self, record: &R) {
        let line = if self.json {
            serde_json::to_string(record)
                .expect("a record is plain data and always serializes")
        } else {
            record.text()
        };
        Self::write_line(&line);
    }

    /// Progress and narration. Suppressed by `--quiet`, never on stdout.
    pub fn status(&self, message: &str) {
        if !self.quiet {
            let _ = writeln!(std::io::stderr(), "{message}");
        }
    }

    /// A problem worth reporting that is not fatal. Shown even when quiet,
    /// because silently doing less than asked is worse than a stray line.
    pub fn warn(&self, message: &str) {
        let _ = if self.color {
            writeln!(std::io::stderr(), "\x1b[33mwarning:\x1b[0m {message}")
        } else {
            writeln!(std::io::stderr(), "warning: {message}")
        };
    }

    pub fn error(&self, message: &str) {
        let _ = if self.color {
            writeln!(std::io::stderr(), "\x1b[31merror:\x1b[0m {message}")
        } else {
            writeln!(std::io::stderr(), "error: {message}")
        };
    }

    // a closed downstream (`| head -1`) is a normal end for a
    // pipeline, so exit quietly instead of letting println! panic on EPIPE.
    fn write_line(line: &str) {
        if writeln!(std::io::stdout(), "{line}").is_err() {
            std::process::exit(Exit::Ok.code().into());
        }
    }
}

/// One file found by `search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchRecord {
    pub user: String,
    pub path: String,
    pub size: u64,
    pub bitrate: Option<u32>,
    pub duration: Option<u32>,
    pub free_slot: bool,
    /// The peer's free upload slots as it reported them, for a picker that
    /// wants to rank rather than just filter.
    pub slots: u8,
    pub speed: u32,
}

impl Record for SearchRecord {
    fn text(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}",
            sanitize(&self.user),
            self.size,
            self.bitrate
                .map_or_else(|| "-".to_string(), |b| b.to_string()),
            sanitize(&self.path)
        )
    }
}

/// One file in a peer's share listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrowseRecord {
    pub user: String,
    pub directory: String,
    pub path: String,
    pub size: u64,
}

impl Record for BrowseRecord {
    fn text(&self) -> String {
        format!(
            "{}\t{}\t{}",
            sanitize(&self.user),
            self.size,
            sanitize(&self.path)
        )
    }
}

/// The outcome of one transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DownloadRecord {
    pub user: String,
    pub path: String,
    pub size: u64,
    pub file: String,
}

impl Record for DownloadRecord {
    fn text(&self) -> String {
        sanitize(&self.file)
    }
}

/// One public chat room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoomRecord {
    pub room: String,
    pub users: u32,
}

impl Record for RoomRecord {
    fn text(&self) -> String {
        format!("{}\t{}", self.users, sanitize(&self.room))
    }
}

/// Something that happened in a room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoomEventRecord {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub room: String,
    pub user: String,
    pub message: String,
}

impl Record for RoomEventRecord {
    fn text(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}",
            self.kind,
            sanitize(&self.room),
            sanitize(&self.user),
            sanitize(&self.message)
        )
    }
}

/// One incoming private message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrivateMessageRecord {
    pub timestamp: u32,
    pub user: String,
    pub message: String,
}

impl Record for PrivateMessageRecord {
    fn text(&self) -> String {
        format!(
            "{}\t{}\t{}",
            self.timestamp,
            sanitize(&self.user),
            sanitize(&self.message)
        )
    }
}

/// One upload being served to a peer, reported by `serve` as it changes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UploadRecord {
    pub user: String,
    pub path: String,
    pub size: u64,
    pub bytes_sent: u64,
    pub speed: f64,
    /// `queued`, `uploading`, `completed`, `cancelled`, or `failed`.
    pub status: String,
    /// Why it failed, when it did.
    pub reason: Option<String>,
}

impl Record for UploadRecord {
    fn text(&self) -> String {
        format!(
            "{}\t{}\t{}/{}\t{}",
            self.status,
            sanitize(&self.user),
            self.bytes_sent,
            self.size,
            sanitize(&self.path)
        )
    }
}

/// Who we are and what this session offers the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WhoamiRecord {
    pub user: String,
    pub server: String,
    pub listening: bool,
    pub listen_port: u16,
    pub shared_folders: u32,
    pub shared_files: u32,
    pub download_dir: String,
    /// Seconds of privileges left on this account, or `null` when the server
    /// did not answer in time. Zero means an ordinary account.
    pub privilege_seconds: Option<u32>,
}

impl Record for WhoamiRecord {
    fn text(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}",
            sanitize(&self.user),
            sanitize(&self.server),
            self.shared_folders,
            self.shared_files
        )
    }
}

/// What the server knows about another user.
/// Each field is `None` when the server has not answered that half yet, so a
/// consumer never reads "offline" or "shares nothing" from a missing reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserRecord {
    pub user: String,
    /// `online`, `away`, or `offline`.
    pub status: Option<String>,
    pub privileged: Option<bool>,
    pub average_speed: Option<u32>,
    pub shared_files: Option<u32>,
    pub shared_folders: Option<u32>,
}

impl Record for UserRecord {
    fn text(&self) -> String {
        let unknown = || "-".to_string();
        format!(
            "{}\t{}\t{}\t{}",
            sanitize(&self.user),
            self.status.clone().unwrap_or_else(unknown),
            self.average_speed.map_or_else(unknown, |v| v.to_string()),
            self.shared_files.map_or_else(unknown, |v| v.to_string())
        )
    }
}

/// One member of a chat room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoomMemberRecord {
    pub room: String,
    pub user: String,
}

impl Record for RoomMemberRecord {
    fn text(&self) -> String {
        sanitize(&self.user)
    }
}

/// One configured share folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShareRecord {
    pub directory: String,
    /// False when the folder is configured but missing on disk.
    pub usable: bool,
}

impl Record for ShareRecord {
    fn text(&self) -> String {
        format!(
            "{}\t{}",
            if self.usable { "ok" } else { "missing" },
            sanitize(&self.directory)
        )
    }
}

/// What the share index currently holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShareStatusRecord {
    pub folders: u32,
    pub files: u32,
    pub directories: Vec<String>,
}

impl Record for ShareStatusRecord {
    fn text(&self) -> String {
        format!("{}\t{}", self.folders, self.files)
    }
}

/// One standing search on the wishlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WishRecord {
    pub query: String,
}

impl Record for WishRecord {
    fn text(&self) -> String {
        sanitize(&self.query)
    }
}

/// One configuration setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigRecord {
    pub key: String,
    pub value: Option<String>,
}

impl Record for ConfigRecord {
    fn text(&self) -> String {
        match &self.value {
            Some(value) => {
                format!("{}\t{}", sanitize(&self.key), sanitize(value))
            }
            None => format!("{}\t", sanitize(&self.key)),
        }
    }
}

/// What happened to one agent's copy of the skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillRecord {
    /// The agent the directory belongs to, or `custom` for a `--dir`.
    pub agent: String,
    pub path: String,
    /// `installed` (written), `unchanged` (already identical), `outdated`
    /// (present but different, reported by `list`), `removed`, or `absent`.
    pub action: String,
}

impl Record for SkillRecord {
    fn text(&self) -> String {
        format!(
            "{}\t{}\t{}",
            sanitize(&self.action),
            sanitize(&self.agent),
            sanitize(&self.path)
        )
    }
}

/// What happened to one shell's completion script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompletionRecord {
    /// `bash`, `zsh`, or `fish`.
    pub shell: String,
    pub path: String,
    /// `installed` (written), `unchanged` (already identical), `removed`, or
    /// `absent`.
    pub action: String,
}

impl Record for CompletionRecord {
    fn text(&self) -> String {
        format!(
            "{}\t{}\t{}",
            sanitize(&self.action),
            sanitize(&self.shell),
            sanitize(&self.path)
        )
    }
}

/// The token a remote client authenticates with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenRecord {
    pub token: String,
}

impl Record for TokenRecord {
    fn text(&self) -> String {
        sanitize(&self.token)
    }
}

/// What a running daemon is and what its session is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DaemonStatusRecord {
    pub user: String,
    pub server: String,
    pub version: String,
    pub protocol: u32,
    pub listening: bool,
    pub listen_port: u16,
    pub shared_folders: u32,
    pub shared_files: u32,
    pub download_dir: String,
    /// `displaced` or `disconnected` when the server session has ended.
    pub session_loss: Option<String>,
    /// Control connections currently attached.
    pub clients: usize,
    pub uptime_secs: u64,
}

impl Record for DaemonStatusRecord {
    fn text(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}",
            sanitize(&self.user),
            sanitize(&self.server),
            self.clients,
            self.uptime_secs
        )
    }
}

/// The result of probing automatic port mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortmapRecord {
    pub ok: bool,
    pub backend: String,
    pub external: Option<String>,
    pub port: u16,
}

impl Record for PortmapRecord {
    fn text(&self) -> String {
        format!(
            "{}\t{}\t{}",
            if self.ok { "ok" } else { "failed" },
            sanitize(&self.backend),
            self.external.as_deref().unwrap_or("-")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search_record() -> SearchRecord {
        SearchRecord {
            user: "bob".into(),
            path: "@@dir\\01 - Track.mp3".into(),
            size: 4096,
            bitrate: Some(320),
            duration: Some(210),
            free_slot: true,
            slots: 1,
            speed: 1000,
        }
    }

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(Exit::Ok.code(), 0);
        assert_eq!(Exit::Failure.code(), 1);
        assert_eq!(Exit::Usage.code(), 2);
        assert_eq!(Exit::Connection.code(), 3);
        assert_eq!(Exit::NoResults.code(), 4);
        assert_eq!(Exit::Timeout.code(), 5);
        assert_eq!(Exit::Transfer.code(), 6);
        assert_eq!(Exit::SessionLost.code(), 7);
    }

    #[test]
    fn sanitize_keeps_records_on_one_line_with_stable_columns() {
        assert_eq!(sanitize("a\tb\nc\r\nd"), "a b c  d");
        assert_eq!(
            sanitize("plain (name) [2024].mp3"),
            "plain (name) [2024].mp3"
        );
    }

    #[test]
    fn search_text_is_user_size_bitrate_path() {
        let text = search_record().text();
        let fields: Vec<&str> = text.split('\t').collect();
        assert_eq!(fields, ["bob", "4096", "320", "@@dir\\01 - Track.mp3"]);
    }

    #[test]
    fn unknown_bitrate_renders_as_a_dash() {
        let record = SearchRecord {
            bitrate: None,
            ..search_record()
        };
        assert_eq!(record.text().split('\t').nth(2), Some("-"));
    }

    #[test]
    fn search_json_carries_every_field() {
        let json = serde_json::to_value(search_record()).unwrap();
        assert_eq!(json["user"], "bob");
        assert_eq!(json["size"], 4096);
        assert_eq!(json["bitrate"], 320);
        assert_eq!(json["duration"], 210);
        assert_eq!(json["free_slot"], true);
        assert_eq!(json["slots"], 1);
        assert_eq!(json["speed"], 1000);
        assert_eq!(json["path"], "@@dir\\01 - Track.mp3");
    }

    #[test]
    fn a_tab_in_a_filename_cannot_forge_a_column() {
        let record = SearchRecord {
            path: "evil\tname.mp3".into(),
            ..search_record()
        };
        assert_eq!(record.text().split('\t').count(), 4);
    }

    #[test]
    fn browse_text_is_user_size_path() {
        let record = BrowseRecord {
            user: "bob".into(),
            directory: "@@music".into(),
            path: "@@music\\a.flac".into(),
            size: 12,
        };
        assert_eq!(record.text(), "bob\t12\t@@music\\a.flac");
    }

    #[test]
    fn download_text_is_just_the_local_path() {
        let record = DownloadRecord {
            user: "bob".into(),
            path: "@@music\\a.flac".into(),
            size: 12,
            file: "/music/a.flac".into(),
        };
        assert_eq!(record.text(), "/music/a.flac");
    }

    #[test]
    fn room_text_leads_with_the_user_count() {
        let record = RoomRecord {
            room: "indie rock".into(),
            users: 42,
        };
        assert_eq!(record.text(), "42\tindie rock");
    }

    #[test]
    fn room_event_type_is_first_and_serializes_as_type() {
        let record = RoomEventRecord {
            kind: "message",
            room: "lobby".into(),
            user: "bob".into(),
            message: "hi".into(),
        };
        assert_eq!(record.text(), "message\tlobby\tbob\thi");
        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["type"], "message");
    }

    #[test]
    fn portmap_text_reports_status_backend_and_address() {
        let ok = PortmapRecord {
            ok: true,
            backend: "upnp".into(),
            external: Some("1.2.3.4:2234".into()),
            port: 2234,
        };
        assert_eq!(ok.text(), "ok\tupnp\t1.2.3.4:2234");
        let failed = PortmapRecord {
            ok: false,
            backend: "none".into(),
            external: None,
            port: 2234,
        };
        assert_eq!(failed.text(), "failed\tnone\t-");
    }
}
