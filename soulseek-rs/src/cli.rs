//! Command-line surface.
//!
//! Two modes share one binary: no subcommand launches the interactive TUI,
//! any subcommand runs a one-shot operation designed for scripts and agents.
//! The one-shot half follows three rules — stdout carries data only, stderr
//! carries progress, and the exit code carries the verdict (see
//! [`crate::output::Exit`]).

use clap::builder::FalseyValueParser;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Upper bound for every wait expressed in seconds. A one-shot command that
/// runs longer than a day is a mistyped flag, and rejecting it here keeps the
/// deadline arithmetic in the command layer from overflowing.
const MAX_WAIT_SECS: u64 = 24 * 60 * 60;

const EXAMPLES: &str = "\
Examples:
  soulseek-rs                                   launch the interactive TUI
  soulseek-rs search 'aphex twin xtal'          print matching files
  soulseek-rs get 'aphex twin xtal'             search, pick the best, download
  soulseek-rs search 'rjd2' --json | jq -c 'select(.bitrate >= 320)' \\
    | soulseek-rs download --stdin              filter, then fetch
  soulseek-rs browse someuser --json            list a peer's shares
  soulseek-rs room listen lobby --follow        stream room chat
  soulseek-rs serve --follow --json             share files, stream upload events
  soulseek-rs user someuser                     is this peer online and worth it
  soulseek-rs skills install                    teach your coding agent this CLI
  soulseek-rs completions install               tab completion for your shell

Exit codes:
  0 success   2 usage/config   3 connect or login   4 no results
  5 timed out   6 transfer failed   7 session lost   1 unexpected error";

#[derive(Parser, Debug, Default)]
#[command(
    name = "soulseek-rs",
    author,
    version,
    about = "Soulseek client in Rust 🦀",
    long_about = "Soulseek client in Rust.\n\nRun without a subcommand for the \
                  interactive TUI, or use a subcommand for a scriptable \
                  one-shot operation: stdout is data, stderr is progress, and \
                  the exit code says what happened.",
    after_help = EXAMPLES
)]
// A flag is a bool; a command line with several switches is not a struct that
// wants splitting up.
#[allow(clippy::struct_excessive_bools)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Soulseek username
    #[arg(short, long, global = true, env = "SOULSEEK_USERNAME")]
    pub username: Option<String>,

    /// Password (visible in `ps`; prefer --password-stdin or the OS keychain)
    #[arg(
        short,
        long,
        global = true,
        env = "SOULSEEK_PASSWORD",
        hide_env_values = true
    )]
    pub password: Option<String>,

    /// Read the password from the first line of stdin (takes precedence
    /// over --password)
    #[arg(long, global = true)]
    pub password_stdin: bool,

    /// Shell command whose stdout is the password, used when nothing else
    /// supplies one
    #[arg(
        long,
        global = true,
        env = "SOULSEEK_PASSWORD_CMD",
        hide_env_values = true
    )]
    pub password_cmd: Option<String>,

    /// Server as host:port
    #[arg(long, global = true, env = "SOULSEEK_SERVER")]
    pub server: Option<String>,

    /// Do not accept incoming peer connections
    #[arg(
        long,
        global = true,
        env = "SOULSEEK_NO_LISTENER",
        action = ArgAction::SetTrue,
        value_parser = FalseyValueParser::new()
    )]
    pub no_listener: bool,

    /// Accept incoming peer connections, overriding the config file
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub listener: bool,

    /// Port to accept peer connections on
    #[arg(long, global = true, env = "SOULSEEK_LISTENER_PORT")]
    pub listener_port: Option<u16>,

    /// Directory downloads are saved to
    #[arg(short, long, global = true, env = "SOULSEEK_DOWNLOAD_DIR")]
    pub download_dir: Option<String>,

    /// Directory shared with other peers; repeat for several
    #[arg(long, global = true, env = "SOULSEEK_SHARED_DIR", action = ArgAction::Append)]
    pub shared_dir: Vec<String>,

    /// Maximum simultaneous downloads
    #[arg(
        short = 'c',
        long,
        global = true,
        env = "SOULSEEK_MAX_CONCURRENT_DOWNLOADS"
    )]
    pub max_concurrent_downloads: Option<usize>,

    /// Seconds a search collects results before reporting
    #[arg(
        long,
        global = true,
        env = "SOULSEEK_SEARCH_TIMEOUT",
        value_parser = clap::value_parser!(u64).range(1..=MAX_WAIT_SECS)
    )]
    pub search_timeout: Option<u64>,

    /// Config file to read instead of the default location
    #[arg(long, global = true, env = "SOULSEEK_CONFIG", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Ignore the config file entirely
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub no_config: bool,

    /// Emit newline-delimited JSON instead of tab-separated text
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub json: bool,

    /// Suppress progress and status output on stderr
    #[arg(short, long, global = true, action = ArgAction::SetTrue)]
    pub quiet: bool,

    /// Increase log verbosity (repeat up to -vvvv)
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,

    /// Write logs to a file instead of stderr
    #[arg(long, global = true, env = "SOULSEEK_LOG_FILE")]
    pub log_file: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Search the network and print matching files
    Search(SearchArgs),

    /// Download files by user and remote path
    Download(DownloadArgs),

    /// Search, pick automatically, and download in one step
    Get(GetArgs),

    /// List the files a user shares
    Browse(BrowseArgs),

    /// Standing searches the server re-runs on its own schedule
    #[command(subcommand)]
    Wish(WishCommand),

    /// Chat rooms
    #[command(subcommand)]
    Room(RoomCommand),

    /// Private messages
    #[command(subcommand)]
    Message(MessageCommand),

    /// Stay online sharing files, reporting each upload as it happens
    Serve(ServeArgs),

    /// What the server knows about another user
    User(UserArgs),

    /// Confirm the credentials and connection work
    Whoami,

    /// The folders this client shares
    #[command(subcommand)]
    Shares(SharesCommand),

    /// Read and write the config file
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Check whether the router opens the listen port (UPnP/NAT-PMP)
    Portmap,

    /// Teach a local coding agent this command surface
    #[command(subcommand)]
    Skills(SkillsCommand),

    /// Tab completion for bash, zsh, and fish
    #[command(subcommand)]
    Completions(CompletionsCommand),
}

#[derive(Subcommand, Debug)]
pub enum CompletionsCommand {
    /// Write the completion script where the shell looks for it
    Install(CompletionsArgs),

    /// Remove the completion script again
    Uninstall(CompletionsArgs),
}

#[derive(Args, Debug)]
pub struct CompletionsArgs {
    /// Shell to act on instead of the ones found on this machine; repeat for
    /// several
    #[arg(long, value_enum, action = ArgAction::Append)]
    pub shell: Vec<Shell>,
}

/// The shells with a conventional per-user completions location.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Subcommand, Debug)]
pub enum SkillsCommand {
    /// Write the skill into every agent found on this machine, replacing an
    /// older copy
    Install(SkillsArgs),

    /// Remove the skill again
    Uninstall(SkillsArgs),

    /// Show where the skill goes and what is there now
    List(SkillsArgs),
}

#[derive(Args, Debug)]
pub struct SkillsArgs {
    /// Skills directory to use instead of the agents found on this machine;
    /// repeat for several. Use `--dir .claude/skills` to commit the skill
    /// alongside a project
    #[arg(long, value_name = "PATH", action = ArgAction::Append)]
    pub dir: Vec<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Seconds to stay online before exiting
    #[arg(
        long,
        default_value_t = 3600,
        value_parser = clap::value_parser!(u64).range(1..=MAX_WAIT_SECS)
    )]
    pub duration: u64,

    /// Stay online until interrupted, ignoring --duration
    #[arg(long, action = ArgAction::SetTrue)]
    pub follow: bool,

    /// Also re-run the wishlist on the interval the server sets, reporting
    /// matches as they arrive
    #[arg(long, action = ArgAction::SetTrue)]
    pub wishlist: bool,

    /// How many uploads to run at once [default: 10]; the rest queue,
    /// privileged users first
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=64))]
    pub upload_slots: Option<u8>,
}

#[derive(Args, Debug)]
pub struct UserArgs {
    /// The user to ask the server about
    pub user: String,

    /// Seconds to wait for the server's answer
    #[arg(
        short,
        long,
        default_value_t = 10,
        value_parser = clap::value_parser!(u64).range(1..=MAX_WAIT_SECS)
    )]
    pub timeout: u64,
}

#[derive(Subcommand, Debug)]
pub enum SharesCommand {
    /// List the configured share folders
    List,

    /// Add a folder to the shares and save it to the config file
    Add {
        /// Folder to start sharing
        directory: String,
    },

    /// Stop sharing a folder and save that to the config file
    Remove {
        /// Folder to stop sharing
        directory: String,
    },

    /// Re-index the shares and report what the network will see
    Reindex,

    /// Report how many folders and files are indexed
    Status,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Print the path of the config file in use
    Path,

    /// Print every setting and its effective value
    List,

    /// Print one setting
    Get {
        /// Setting name, as shown by `config list`
        key: String,
    },

    /// Change one setting in the config file
    Set {
        /// Setting name, as shown by `config list`
        key: String,
        /// New value; an empty string clears it
        value: String,
    },
}

/// How to order search results.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum SortKey {
    /// Free slot first, then bitrate, peer speed, and size
    Best,
    /// Largest file first
    Size,
    /// Highest bitrate first
    Bitrate,
    /// Fastest peer first
    Speed,
}

/// Which of the ranked results `get` should download.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Pick {
    /// The single best-ranked result
    Best,
    /// The first result that arrives, without waiting out the search
    First,
    /// Every result, subject to --limit
    All,
}

/// The result restrictions every command that searches accepts, so `search`,
/// `get` and `wish run` all narrow a result set the same way.
#[derive(Args, Debug, Default, Clone)]
pub struct FilterArgs {
    /// Drop results below this bitrate in kbps
    #[arg(long, value_name = "KBPS")]
    pub min_bitrate: Option<u32>,

    /// Keep only peers advertising a free upload slot
    #[arg(long, action = ArgAction::SetTrue)]
    pub free_slots: bool,

    /// Drop results whose path contains this term; repeat for several
    #[arg(long, value_name = "TERM", action = ArgAction::Append)]
    pub exclude: Vec<String>,

    /// Keep only this file extension, with or without a dot; repeat for several
    #[arg(long, value_name = "EXT", action = ArgAction::Append)]
    pub extension: Vec<String>,

    /// Drop results smaller than this many bytes
    #[arg(long, value_name = "BYTES")]
    pub min_size: Option<u64>,

    /// Drop results larger than this many bytes
    #[arg(long, value_name = "BYTES")]
    pub max_size: Option<u64>,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// What to search for
    pub query: String,

    #[command(flatten)]
    pub filter: FilterArgs,

    /// Print at most this many results (0 means all)
    #[arg(short = 'n', long, default_value_t = 0)]
    pub limit: usize,

    /// Result order
    #[arg(long, value_enum, default_value_t = SortKey::Best)]
    pub sort: SortKey,
}

#[derive(Args, Debug)]
pub struct DownloadArgs {
    /// Peer holding the file
    #[arg(required_unless_present = "stdin")]
    pub user: Option<String>,

    /// Remote path exactly as printed by `search` or `browse`
    #[arg(required_unless_present = "stdin")]
    pub path: Option<String>,

    /// File size in bytes, as reported by `search` or `browse`
    #[arg(long)]
    pub size: Option<u64>,

    /// Read records from stdin: `search --json` output, or TAB-separated
    /// `search` text lines
    #[arg(long, action = ArgAction::SetTrue, conflicts_with_all = ["user", "path", "size"])]
    pub stdin: bool,

    /// Seconds to wait for each transfer before giving up
    #[arg(
        short,
        long,
        default_value_t = 300,
        value_parser = clap::value_parser!(u64).range(1..=MAX_WAIT_SECS)
    )]
    pub timeout: u64,
}

#[derive(Args, Debug)]
pub struct GetArgs {
    /// What to search for
    pub query: String,

    /// Which results to download
    #[arg(long, value_enum, default_value_t = Pick::Best)]
    pub pick: Pick,

    /// With `--pick all`, download at most this many (0 means all)
    #[arg(short = 'n', long, default_value_t = 0)]
    pub limit: usize,

    #[command(flatten)]
    pub filter: FilterArgs,

    /// Seconds to wait for each transfer before giving up
    #[arg(
        short,
        long,
        default_value_t = 300,
        value_parser = clap::value_parser!(u64).range(1..=MAX_WAIT_SECS)
    )]
    pub timeout: u64,
}

#[derive(Args, Debug)]
pub struct BrowseArgs {
    /// User whose shares to list
    pub user: String,

    /// Seconds to wait for the listing
    #[arg(
        short,
        long,
        default_value_t = 20,
        value_parser = clap::value_parser!(u64).range(1..=MAX_WAIT_SECS)
    )]
    pub timeout: u64,
}

#[derive(Subcommand, Debug)]
pub enum WishCommand {
    /// Store a query and search for it from now on
    Add {
        /// What to keep looking for
        query: String,
    },

    /// Stop looking for a query
    Remove {
        /// The wish to drop, as shown by `wish list`
        query: String,
    },

    /// Print the stored queries
    List,

    /// Search every stored query once and print what came back
    Run(WishRunArgs),
}

#[derive(Args, Debug)]
pub struct WishRunArgs {
    /// Only run this wish instead of all of them
    #[arg(long, value_name = "QUERY")]
    pub only: Option<String>,

    #[command(flatten)]
    pub filter: FilterArgs,

    /// Print at most this many results per wish (0 means all)
    #[arg(short = 'n', long, default_value_t = 0)]
    pub limit: usize,

    /// Result order
    #[arg(long, value_enum, default_value_t = SortKey::Best)]
    pub sort: SortKey,
}

#[derive(Subcommand, Debug)]
pub enum RoomCommand {
    /// List public rooms and their user counts
    List {
        /// Seconds to wait for the list
        #[arg(
            short,
            long,
            default_value_t = 5,
            value_parser = clap::value_parser!(u64).range(1..=MAX_WAIT_SECS)
        )]
        timeout: u64,
    },

    /// Post a message to a room
    Say {
        /// Room to post in
        room: String,
        /// Message text
        message: String,
    },

    /// List who is currently in a room
    Users {
        /// Room whose members to list
        room: String,
        /// Seconds to wait for the membership list
        #[arg(
            short,
            long,
            default_value_t = 10,
            value_parser = clap::value_parser!(u64).range(1..=MAX_WAIT_SECS)
        )]
        timeout: u64,
    },

    /// Stream messages and joins/leaves from a room
    Listen {
        /// Room to listen to
        room: String,
        /// Seconds to listen before exiting
        #[arg(
            long,
            default_value_t = 30,
            value_parser = clap::value_parser!(u64).range(1..=MAX_WAIT_SECS)
        )]
        duration: u64,
        /// Listen until interrupted, ignoring --duration
        #[arg(long, action = ArgAction::SetTrue)]
        follow: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum MessageCommand {
    /// Send a private message
    Send {
        /// Recipient username
        user: String,
        /// Message text
        message: String,
    },

    /// Stream incoming private messages
    Read {
        /// Seconds to listen before exiting
        #[arg(
            long,
            default_value_t = 30,
            value_parser = clap::value_parser!(u64).range(1..=MAX_WAIT_SECS)
        )]
        duration: u64,
        /// Listen until interrupted, ignoring --duration
        #[arg(long, action = ArgAction::SetTrue)]
        follow: bool,
    },
}

/// Split `host:port`, the only shape the server setting accepts.
pub fn parse_server_address(server: &str) -> Result<(String, u16), String> {
    let (host, port) = server.rsplit_once(':').ok_or_else(|| {
        format!("Invalid server '{server}': expected host:port")
    })?;
    if host.is_empty() {
        return Err(format!("Invalid server '{server}': missing host"));
    }
    let port = port
        .parse::<u16>()
        .map_err(|_| format!("Invalid server '{server}': bad port '{port}'"))?;
    Ok((host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("should parse")
    }

    #[test]
    fn clap_definitions_are_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_subcommand_means_tui() {
        assert!(parse(&["soulseek-rs"]).command.is_none());
    }

    #[test]
    fn global_flags_parse_after_the_subcommand() {
        let cli = parse(&[
            "soulseek-rs",
            "search",
            "query",
            "--username",
            "alice",
            "--json",
            "--search-timeout",
            "30",
        ]);
        assert_eq!(cli.username.as_deref(), Some("alice"));
        assert!(cli.json);
        assert_eq!(cli.search_timeout, Some(30));
    }

    #[test]
    fn global_flags_parse_before_the_subcommand() {
        let cli =
            parse(&["soulseek-rs", "--username", "alice", "browse", "bob"]);
        assert_eq!(cli.username.as_deref(), Some("alice"));
        assert!(matches!(cli.command, Some(Commands::Browse(_))));
    }

    #[test]
    fn shared_dir_repeats_into_a_list() {
        let cli =
            parse(&["soulseek-rs", "--shared-dir", "/a", "--shared-dir", "/b"]);
        assert_eq!(cli.shared_dir, vec!["/a".to_string(), "/b".to_string()]);
    }

    #[test]
    fn download_requires_user_and_path_without_stdin() {
        assert!(Cli::try_parse_from(["soulseek-rs", "download"]).is_err());
        assert!(
            Cli::try_parse_from(["soulseek-rs", "download", "bob"]).is_err()
        );
        assert!(
            Cli::try_parse_from(["soulseek-rs", "download", "bob", "a\\b.mp3"])
                .is_ok()
        );
    }

    #[test]
    fn download_stdin_needs_no_positionals() {
        let cli = parse(&["soulseek-rs", "download", "--stdin"]);
        let Some(Commands::Download(args)) = cli.command else {
            panic!("expected download");
        };
        assert!(args.stdin);
        assert!(args.user.is_none());
    }

    #[test]
    fn download_stdin_conflicts_with_positionals() {
        assert!(
            Cli::try_parse_from([
                "soulseek-rs",
                "download",
                "--stdin",
                "bob",
                "a.mp3"
            ])
            .is_err()
        );
    }

    #[test]
    fn room_and_message_groups_parse() {
        assert!(matches!(
            parse(&["soulseek-rs", "room", "list"]).command,
            Some(Commands::Room(RoomCommand::List { .. }))
        ));
        assert!(matches!(
            parse(&["soulseek-rs", "room", "say", "lobby", "hi"]).command,
            Some(Commands::Room(RoomCommand::Say { .. }))
        ));
        assert!(matches!(
            parse(&["soulseek-rs", "message", "read", "--follow"]).command,
            Some(Commands::Message(MessageCommand::Read { follow: true, .. }))
        ));
    }

    #[test]
    fn skills_dir_repeats_into_a_list() {
        let cli = parse(&[
            "soulseek-rs",
            "skills",
            "install",
            "--dir",
            "/a",
            "--dir",
            "/b",
        ]);
        let Some(Commands::Skills(SkillsCommand::Install(args))) = cli.command
        else {
            panic!("expected skills install");
        };
        assert_eq!(args.dir, [PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn get_defaults_to_picking_the_best_result() {
        let cli = parse(&["soulseek-rs", "get", "query"]);
        let Some(Commands::Get(args)) = cli.command else {
            panic!("expected get");
        };
        assert_eq!(args.pick, Pick::Best);
        assert_eq!(args.limit, 0);
        assert_eq!(args.timeout, 300);
    }

    #[test]
    fn an_absurd_wait_is_rejected_rather_than_overflowing_a_deadline() {
        assert!(
            Cli::try_parse_from([
                "soulseek-rs",
                "download",
                "bob",
                "a.mp3",
                "--timeout",
                "99999999999999"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "soulseek-rs",
                "--search-timeout",
                "0",
                "search",
                "q"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from(["soulseek-rs", "browse", "bob", "-t", "20"])
                .is_ok()
        );
    }

    #[test]
    fn server_address_splits_host_and_port() {
        assert_eq!(
            parse_server_address("server.slsknet.org:2416"),
            Ok(("server.slsknet.org".to_string(), 2416))
        );
        assert!(parse_server_address("noport").is_err());
        assert!(parse_server_address("host:70000").is_err());
        assert!(parse_server_address(":2416").is_err());
    }
}
