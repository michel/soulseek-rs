use crate::types::{Download, DownloadStatus};

#[derive(Default)]
pub struct DownloadStore {
    downloads: Vec<Download>,
}

impl DownloadStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, download: Download) {
        self.downloads.push(download);
    }

    pub fn remove(&mut self, token: u32) {
        self.downloads.retain(|d| d.token != token);
    }

    pub fn get_by_token(&self, token: u32) -> Option<&Download> {
        self.downloads.iter().find(|d| d.token == token)
    }

    pub fn get_by_token_mut(&mut self, token: u32) -> Option<&mut Download> {
        self.downloads.iter_mut().find(|d| d.token == token)
    }

    pub fn get_by_file_mut(
        &mut self,
        username: &str,
        filename: &str,
    ) -> Option<&mut Download> {
        self.downloads
            .iter_mut()
            .find(|d| d.username == username && d.filename == filename)
    }

    pub fn tokens(&self) -> Vec<u32> {
        self.downloads.iter().map(|d| d.token).collect()
    }

    pub fn list(&self) -> &Vec<Download> {
        &self.downloads
    }

    pub fn update_status(&mut self, token: u32, status: DownloadStatus) {
        if let Some(download) = self.get_by_token_mut(token) {
            download.status = status;
        }
    }

    pub fn remove_queued_by_file(
        &mut self,
        username: &str,
        filename: &str,
    ) -> bool {
        let Some(index) = self.downloads.iter().position(|download| {
            download.username == username
                && download.filename == filename
                && matches!(download.status, DownloadStatus::Queued)
        }) else {
            return false;
        };

        self.downloads.remove(index);
        true
    }

    pub fn pause_by_file(&mut self, username: &str, filename: &str) -> bool {
        let Some(download) = self.get_by_file_mut(username, filename) else {
            return false;
        };

        let paused_status = match &download.status {
            DownloadStatus::InProgress {
                bytes_downloaded,
                total_bytes,
                ..
            } => DownloadStatus::Paused {
                bytes_downloaded: *bytes_downloaded,
                total_bytes: *total_bytes,
            },
            DownloadStatus::Paused { .. } => return true,
            _ => return false,
        };

        download.status = paused_status.clone();
        let _ = download.sender.send(paused_status);
        true
    }

    pub fn resume_by_file(&mut self, username: &str, filename: &str) -> bool {
        let Some(download) = self.get_by_file_mut(username, filename) else {
            return false;
        };

        let resumed_status = match &download.status {
            DownloadStatus::Paused {
                bytes_downloaded,
                total_bytes,
            } => DownloadStatus::InProgress {
                bytes_downloaded: *bytes_downloaded,
                total_bytes: *total_bytes,
                speed_bytes_per_sec: 0.0,
            },
            DownloadStatus::InProgress { .. } => return true,
            _ => return false,
        };

        download.status = resumed_status.clone();
        let _ = download.sender.send(resumed_status);
        true
    }
}

/// Returns the tokens of downloads matching `username` (and optionally a
/// `filename`) after notifying their senders of `Failed`. Caller is
/// responsible for then calling `update_status` and `remove` for each token,
/// typically under a write lock.
pub fn collect_failed_tokens(
    store: &DownloadStore,
    username: &str,
    filename: Option<&str>,
) -> Vec<u32> {
    store
        .list()
        .iter()
        .filter(|d| {
            d.username == username && filename.is_none_or(|f| d.filename == *f)
        })
        .map(|d| {
            let _ = d.sender.send(DownloadStatus::Failed);
            d.token
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn make_download(token: u32, status: DownloadStatus) -> Download {
        Download {
            username: "peer".to_string(),
            filename: format!("file-{}.mp3", token),
            token,
            size: 100,
            download_directory: "test".to_string(),
            status,
            sender: mpsc::channel().0,
        }
    }

    #[test]
    fn add_get_remove_roundtrip() {
        let mut store = DownloadStore::new();
        store.add(make_download(123, DownloadStatus::Queued));

        assert!(store.get_by_token(123).is_some());
        assert_eq!(store.tokens(), vec![123]);
        assert_eq!(store.list().len(), 1);

        store.remove(123);
        assert!(store.get_by_token(123).is_none());
        assert!(store.list().is_empty());
    }

    #[test]
    fn pause_then_resume_in_progress_download() {
        let mut store = DownloadStore::new();
        let (tx, rx) = mpsc::channel();
        let mut download = make_download(
            1,
            DownloadStatus::InProgress {
                bytes_downloaded: 25,
                total_bytes: 100,
                speed_bytes_per_sec: 10.0,
            },
        );
        download.sender = tx;
        store.add(download);

        assert!(store.pause_by_file("peer", "file-1.mp3"));
        assert!(matches!(
            store.get_by_token(1).unwrap().status,
            DownloadStatus::Paused {
                bytes_downloaded: 25,
                total_bytes: 100
            }
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            DownloadStatus::Paused {
                bytes_downloaded: 25,
                total_bytes: 100
            }
        ));

        assert!(store.resume_by_file("peer", "file-1.mp3"));
        assert!(matches!(
            store.get_by_token(1).unwrap().status,
            DownloadStatus::InProgress {
                bytes_downloaded: 25,
                total_bytes: 100,
                speed_bytes_per_sec: 0.0
            }
        ));
    }

    #[test]
    fn remove_queued_skips_active_downloads() {
        let mut store = DownloadStore::new();
        store.add(make_download(123, DownloadStatus::Queued));
        store.add(make_download(
            456,
            DownloadStatus::InProgress {
                bytes_downloaded: 25,
                total_bytes: 100,
                speed_bytes_per_sec: 10.0,
            },
        ));
        // Override second download's filename so they don't collide
        store.get_by_token_mut(456).unwrap().filename =
            "active.mp3".to_string();
        store.get_by_token_mut(123).unwrap().filename =
            "queued.mp3".to_string();

        assert!(store.remove_queued_by_file("peer", "queued.mp3"));
        assert!(!store.remove_queued_by_file("peer", "active.mp3"));
        assert!(store.get_by_token(123).is_none());
        assert!(store.get_by_token(456).is_some());
    }

    #[test]
    fn collect_failed_tokens_notifies_and_lists_matching() {
        let mut store = DownloadStore::new();
        let (tx_match, rx_match) = mpsc::channel();
        let (tx_other_user, _rx_other_user) = mpsc::channel();
        let (tx_other_file, _rx_other_file) = mpsc::channel();

        let mut a = make_download(1, DownloadStatus::Queued);
        a.sender = tx_match;
        a.username = "peer".to_string();
        a.filename = "song.mp3".to_string();

        let mut b = make_download(2, DownloadStatus::Queued);
        b.sender = tx_other_user;
        b.username = "other".to_string();
        b.filename = "song.mp3".to_string();

        let mut c = make_download(3, DownloadStatus::Queued);
        c.sender = tx_other_file;
        c.username = "peer".to_string();
        c.filename = "different.mp3".to_string();

        store.add(a);
        store.add(b);
        store.add(c);

        let tokens = collect_failed_tokens(&store, "peer", Some("song.mp3"));

        assert_eq!(tokens, vec![1]);
        assert!(matches!(
            rx_match.try_recv().unwrap(),
            DownloadStatus::Failed
        ));
    }
}
