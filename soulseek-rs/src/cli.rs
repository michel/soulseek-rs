use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "soulseek-rs",
    author,
    version,
    about = "Soulseek client in Rust 🦀",
    long_about = None
)]
pub struct Cli {
    #[arg(short, long, env = "SOULSEEK_USERNAME")]
    pub username: Option<String>,

    #[arg(short, long, env = "SOULSEEK_PASSWORD")]
    pub password: Option<String>,

    #[arg(
        short,
        long,
        env = "SOULSEEK_SERVER",
        default_value = "server.slsknet.org:2416"
    )]
    pub server: String,

    #[arg(long, env = "DISABLE_LISTENER")]
    pub disable_listener: bool,

    #[arg(short, long, env = "LISTENER_PORT", default_value = "2234")]
    pub listener_port: u16,

    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[arg(
        long,
        env = "LOG_FILE",
        help = "Write logs to file instead of stderr"
    )]
    pub log_file: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(
        short,
        long,
        env = "SOULSEEK_DOWNLOAD_DIR",
        default_value = "~/Downloads"
    )]
    pub download_dir: String,

    #[arg(
        long,
        env = "SOULSEEK_SHARED_DIR",
        help = "Directory whose files are shared with (uploaded to) other peers"
    )]
    pub shared_dir: Option<String>,

    #[arg(
        short = 'c',
        long,
        env = "MAX_CONCURRENT_DOWNLOADS",
        default_value = "5"
    )]
    pub max_concurrent_downloads: usize,

    #[arg(long, default_value = "10")]
    pub search_timeout: u64,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Search {
        query: String,

        #[arg(short, long, default_value = "10")]
        timeout: u64,

        #[arg(short, long, default_value = "~/Downloads")]
        download_dir: String,

        #[arg(
            short = 'c',
            long,
            env = "MAX_CONCURRENT_DOWNLOADS",
            default_value = "5"
        )]
        max_concurrent_downloads: usize,
    },

    /// Send a private message to another user
    Message {
        /// Username of the recipient
        username: String,

        /// Message text to send
        message: String,
    },

    /// Browse another user's shared files
    Browse {
        /// Username whose shares to list
        username: String,
    },

    /// List the public chat rooms and their user counts
    Rooms,

    /// Join a chat room: print messages, or send one and exit
    Chat {
        /// Name of the room to join
        room: String,

        /// Optional message to say in the room (omit to just listen)
        message: Option<String>,

        /// Seconds to stay and print incoming messages when only listening
        #[arg(short, long, default_value = "30")]
        listen_secs: u64,
    },
}

pub fn parse_server_address(server: &str) -> color_eyre::Result<(String, u16)> {
    let parts: Vec<&str> = server.split(':').collect();
    if parts.len() != 2 {
        return Err(color_eyre::eyre::eyre!(
            "Invalid server format. Expected 'host:port', got '{}'",
            server
        ));
    }
    let host = parts[0].to_string();
    let port = parts[1].parse::<u16>().map_err(|_| {
        color_eyre::eyre::eyre!("Invalid port number: {}", parts[1])
    })?;
    Ok((host, port))
}
