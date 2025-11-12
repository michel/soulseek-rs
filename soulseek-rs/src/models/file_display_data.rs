#[derive(Clone)]
pub struct FileDisplayData {
    pub filename: String,
    pub size: u64,
    pub username: String,
    pub speed: u32,
    pub slots: u8,
    pub bitrate: Option<u32>,
}
