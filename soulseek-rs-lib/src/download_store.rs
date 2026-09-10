use crate::types::{Download, DownloadStatus};

#[derive(Default)]
pub struct DownloadStore {
    downloads: Vec<Download>,
}

impl DownloadStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, download: Download) {
        self.downloads.retain(|d| {
            !(d.is_finished()
                && d.username == download.username
                && d.filename == download.filename)
        });
        self.downloads.push(download);
    }

    pub fn remove(&mut self, token: u32) {
        self.downloads.retain(|d| d.token != token);
    }

    #[must_use]
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

    #[must_use]
    pub fn tokens(&self) -> Vec<u32> {
        self.downloads.iter().map(|d| d.token).collect()
    }

    #[must_use]
    pub const fn list(&self) -> &Vec<Download> {
        &self.downloads
    }

    pub fn update_status(
        &mut self,
        token: u32,
        status: DownloadStatus,
    ) -> bool {
        let Some(download) = self.get_by_token_mut(token) else {
            return false;
        };
        if download.status.is_terminal()
            || matches!(
                (&download.status, &status),
                (
                    DownloadStatus::Paused { .. },
                    DownloadStatus::InProgress { .. }
                )
            )
        {
            return false;
        }
        download.status = status;
        true
    }

    /// Atomically reserve a queued download for the peer that owns it.
    ///
    /// Every file-connection direction uses this before opening a partial, so
    /// two sockets cannot both observe `Queued` and write the same target.
    pub fn claim_for_peer(
        &mut self,
        token: u32,
        username: &str,
    ) -> Option<Download> {
        let download = self.get_by_token_mut(token)?;
        if download.username != username
            || !matches!(download.status, DownloadStatus::Queued)
        {
            return None;
        }

        download.status = DownloadStatus::InProgress {
            bytes_downloaded: 0,
            total_bytes: download.size,
            speed_bytes_per_sec: 0.0,
        };
        Some(download.clone())
    }

    pub fn cancel_by_file(
        &mut self,
        username: &str,
        filename: &str,
    ) -> Option<Download> {
        let download = self.get_by_file_mut(username, filename)?;
        if download.is_finished() {
            return None;
        }
        download.status = DownloadStatus::Cancelled;
        let _ = download.sender.send(DownloadStatus::Cancelled);
        Some(download.clone())
    }

    pub fn update_queue_position(
        &mut self,
        username: &str,
        filename: &str,
        position: u32,
    ) -> bool {
        let Some(download) = self.get_by_file_mut(username, filename) else {
            return false;
        };
        download.queue_position = Some(position);
        true
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

    /// Remove every download matching `username`/`filename` regardless of
    /// status. Used before retrying a failed download so the stale entry (whose
    /// md5-derived token collides with the retry's) can't shadow the fresh one.
    /// Returns whether anything was removed.
    pub fn remove_by_file(&mut self, username: &str, filename: &str) -> bool {
        let before = self.downloads.len();
        self.downloads
            .retain(|d| !(d.username == username && d.filename == filename));
        self.downloads.len() != before
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

/// Returns the tokens of queued downloads matching `username` (and optionally
/// a `filename`) after notifying their senders of `Failed`.
///
/// Caller is responsible for then calling `update_status` and `remove` for
/// each token, typically under a write lock.
#[must_use]
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
        .filter(|d| matches!(d.status, DownloadStatus::Queued))
        .map(|d| {
            let _ = d.sender.send(DownloadStatus::Failed(Some(
                "The upload failed on the other side".to_string(),
            )));
            d.token
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DownloadMetadata;
    use std::sync::mpsc;

    fn make_download(token: u32, status: DownloadStatus) -> Download {
        Download {
            username: "peer".to_string(),
            filename: format!("file-{token}.mp3"),
            token,
            size: 100,
            download_directory: "test".to_string(),
            status,
            sender: mpsc::channel().0,
            queue_position: None,
            metadata: DownloadMetadata::default(),
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
    fn update_queue_position_sets_field_when_match() {
        let mut store = DownloadStore::new();
        let mut download = make_download(1, DownloadStatus::Queued);
        download.username = "peer".to_string();
        download.filename = "song.mp3".to_string();
        store.add(download);

        assert!(store.update_queue_position("peer", "song.mp3", 42));
        assert_eq!(store.get_by_token(1).unwrap().queue_position, Some(42));

        assert!(!store.update_queue_position("peer", "missing.mp3", 1));
        assert!(!store.update_queue_position("other", "song.mp3", 1));
    }

    #[test]
    fn cancel_by_file_flips_an_unfinished_download_and_tells_its_channel() {
        let mut store = DownloadStore::new();
        let (tx, rx) = mpsc::channel();
        let mut queued = make_download(1, DownloadStatus::Queued);
        queued.sender = tx;
        store.add(queued);
        store.add(make_download(
            2,
            DownloadStatus::InProgress {
                bytes_downloaded: 5,
                total_bytes: 100,
                speed_bytes_per_sec: 1.0,
            },
        ));
        store.add(make_download(
            3,
            DownloadStatus::Paused {
                bytes_downloaded: 5,
                total_bytes: 100,
            },
        ));
        store.add(make_download(4, DownloadStatus::Completed));
        store.add(make_download(5, DownloadStatus::Failed(None)));

        assert!(store.cancel_by_file("peer", "file-1.mp3").is_some());
        assert!(matches!(rx.try_recv(), Ok(DownloadStatus::Cancelled)));
        assert!(store.cancel_by_file("peer", "file-2.mp3").is_some());
        assert!(store.cancel_by_file("peer", "file-3.mp3").is_some());
        assert!(store.cancel_by_file("peer", "file-4.mp3").is_none());
        assert!(store.cancel_by_file("peer", "file-5.mp3").is_none());
        assert!(store.cancel_by_file("peer", "file-9.mp3").is_none());
        for token in 1..=3 {
            let download = store.get_by_token(token).unwrap();
            assert!(matches!(download.status, DownloadStatus::Cancelled));
            assert!(download.is_finished());
        }
    }

    #[test]
    fn a_cancelled_download_stays_cancelled() {
        let mut store = DownloadStore::new();
        store.add(make_download(1, DownloadStatus::Queued));
        assert!(store.cancel_by_file("peer", "file-1.mp3").is_some());
        assert!(!store.update_status(
            1,
            DownloadStatus::InProgress {
                bytes_downloaded: 0,
                total_bytes: 100,
                speed_bytes_per_sec: 0.0,
            },
        ));
        assert!(
            !store
                .update_status(1, DownloadStatus::Failed(Some("late".into())))
        );
        assert!(matches!(
            store.get_by_token(1).unwrap().status,
            DownloadStatus::Cancelled
        ));
        assert!(store.cancel_by_file("peer", "file-1.mp3").is_none());
        assert!(!store.resume_by_file("peer", "file-1.mp3"));
        assert!(!store.pause_by_file("peer", "file-1.mp3"));
    }

    #[test]
    fn add_evicts_a_finished_entry_for_the_same_file() {
        let mut store = DownloadStore::new();
        store.add(make_download(1, DownloadStatus::Cancelled));
        store.add(make_download(2, DownloadStatus::Queued));
        let mut fresh = make_download(3, DownloadStatus::Queued);
        fresh.filename = "file-1.mp3".to_string();
        store.add(fresh);
        let mut again = make_download(4, DownloadStatus::Queued);
        again.filename = "file-2.mp3".to_string();
        store.add(again);

        assert_eq!(store.tokens(), vec![2, 3, 4], "only the dead row goes");
        assert!(store.cancel_by_file("peer", "file-1.mp3").is_some());
        assert!(matches!(
            store.get_by_token(3).unwrap().status,
            DownloadStatus::Cancelled
        ));
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
    fn progress_cannot_implicitly_resume_a_paused_download() {
        let mut store = DownloadStore::new();
        store.add(make_download(
            1,
            DownloadStatus::Paused {
                bytes_downloaded: 25,
                total_bytes: 100,
            },
        ));

        assert!(!store.update_status(
            1,
            DownloadStatus::InProgress {
                bytes_downloaded: 50,
                total_bytes: 100,
                speed_bytes_per_sec: 10.0,
            },
        ));
        assert!(matches!(
            store.get_by_token(1).unwrap().status,
            DownloadStatus::Paused {
                bytes_downloaded: 25,
                total_bytes: 100
            }
        ));
    }

    #[test]
    fn a_download_can_only_be_claimed_once_and_by_its_peer() {
        let mut store = DownloadStore::new();
        store.add(make_download(1, DownloadStatus::Queued));

        assert!(store.claim_for_peer(1, "other").is_none());
        let claimed = store.claim_for_peer(1, "peer").unwrap();
        assert!(matches!(
            claimed.status,
            DownloadStatus::InProgress {
                bytes_downloaded: 0,
                total_bytes: 100,
                speed_bytes_per_sec: 0.0
            }
        ));
        assert!(store.claim_for_peer(1, "peer").is_none());
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
    fn remove_by_file_removes_regardless_of_status() {
        let mut store = DownloadStore::new();
        // A failed download (the retry case) plus a same-name duplicate that a
        // token-migration could have left behind — both must go.
        let mut failed = make_download(1, DownloadStatus::Failed(None));
        failed.filename = "song.mp3".to_string();
        store.add(failed);
        let mut dup = make_download(2, DownloadStatus::Queued);
        dup.filename = "song.mp3".to_string();
        store.add(dup);
        let mut other = make_download(3, DownloadStatus::Queued);
        other.filename = "other.mp3".to_string();
        store.add(other);

        assert!(store.remove_by_file("peer", "song.mp3"));
        assert!(store.get_by_token(1).is_none());
        assert!(store.get_by_token(2).is_none());
        assert!(store.get_by_token(3).is_some(), "other file untouched");
        assert!(!store.remove_by_file("peer", "song.mp3"), "idempotent");
    }

    #[test]
    fn collect_failed_tokens_collects_only_queued_downloads() {
        let mut store = DownloadStore::new();
        let (tx_active, rx_active) = mpsc::channel();
        let (tx_paused, rx_paused) = mpsc::channel();
        let (tx_completed, rx_completed) = mpsc::channel();
        let (tx_queued, _rx_queued) = mpsc::channel();

        let mut active = make_download(
            1,
            DownloadStatus::InProgress {
                bytes_downloaded: 25,
                total_bytes: 100,
                speed_bytes_per_sec: 10.0,
            },
        );
        active.sender = tx_active;

        let mut paused = make_download(
            2,
            DownloadStatus::Paused {
                bytes_downloaded: 25,
                total_bytes: 100,
            },
        );
        paused.sender = tx_paused;

        let mut completed = make_download(3, DownloadStatus::Completed);
        completed.sender = tx_completed;

        let mut queued = make_download(4, DownloadStatus::Queued);
        queued.sender = tx_queued;

        store.add(active);
        store.add(paused);
        store.add(completed);
        store.add(queued);

        let tokens = collect_failed_tokens(&store, "peer", None);

        assert_eq!(tokens, vec![4], "only the queued download may fail");
        assert!(
            rx_active.try_recv().is_err(),
            "an in-progress transfer must not be notified of failure"
        );
        assert!(
            rx_paused.try_recv().is_err(),
            "a paused transfer must not be notified of failure"
        );
        assert!(
            rx_completed.try_recv().is_err(),
            "a completed download must not be notified of failure"
        );
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
            DownloadStatus::Failed(_)
        ));
    }
}
