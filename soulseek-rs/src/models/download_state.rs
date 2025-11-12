use soulseek_rs::DownloadStatus;

#[derive(Debug, Clone)]
pub struct FileDownloadState {
    pub filename: String,
    pub username: String,
    pub total_bytes: u64,
    pub bytes_downloaded: u64,
    pub speed_bytes_per_sec: f64,
    pub status: DownloadStatus,
}

impl FileDownloadState {
    pub fn new(filename: String, username: String, total_bytes: u64) -> Self {
        Self {
            filename,
            username,
            total_bytes,
            bytes_downloaded: 0,
            speed_bytes_per_sec: 0.0,
            status: DownloadStatus::Queued,
        }
    }

    pub fn update_status(&mut self, status: DownloadStatus) {
        match &status {
            DownloadStatus::InProgress {
                bytes_downloaded,
                total_bytes,
                speed_bytes_per_sec,
            } => {
                self.bytes_downloaded = *bytes_downloaded;
                self.total_bytes = *total_bytes;
                self.speed_bytes_per_sec = *speed_bytes_per_sec;
            }
            DownloadStatus::Completed => {
                self.bytes_downloaded = self.total_bytes;
            }
            _ => {}
        }
        self.status = status;
    }

    pub fn is_finished(&self) -> bool {
        matches!(
            self.status,
            DownloadStatus::Completed
                | DownloadStatus::Failed
                | DownloadStatus::TimedOut
        )
    }
}
