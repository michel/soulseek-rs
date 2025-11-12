pub struct SearchConfig {
    pub username: String,
    pub password: String,
    pub server_host: String,
    pub server_port: u16,
    pub enable_listener: bool,
    pub listener_port: u32,
    pub query: String,
    pub timeout: u64,
    pub download_dir: String,
    pub verbose: u8,
    pub max_concurrent_downloads: usize,
}
