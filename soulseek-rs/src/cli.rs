use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "soulseek-rs",
    author,
    version,
    about = "Soulseek client in Rust 🦀",
    long_about = None,
    arg_required_else_help = true
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
    pub listener_port: u32,

    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Commands,
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
