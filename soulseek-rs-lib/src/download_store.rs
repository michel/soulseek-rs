use crate::peer::part_path_of;
use crate::types::{Download, DownloadStatus};

/// How many finished downloads stay listed. Finished entries are history;
/// kept without limit they are also work, since everything that walks the
/// list — a status poll, the daemon's periodic save — walks all of it.
pub const MAX_FINISHED_DOWNLOADS: usize = 1000;

#[derive(Default)]
pub struct DownloadStore {
    downloads: Vec<Download>,
}

/// What names a download's partial: two downloads that agree on all of it
/// write to, and resume from, the same file.
fn partial_of(download: &Download) -> (&str, &str, u64, &str) {
    (
        &download.username,
        &download.filename,
        download.size,
        &download.download_directory,
    )
}

impl DownloadStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, download: Download) {
        let replaced: Vec<Download> = self
            .downloads
            .extract_if(.., |d| {
                d.is_finished()
                    && d.username == download.username
                    && d.filename == download.filename
            })
            .collect();
        self.downloads.push(download);
        self.forget_partials(&replaced);
        self.prune_finished(MAX_FINISHED_DOWNLOADS);
    }

    /// Forget the oldest finished downloads beyond `keep`. The list is in
    /// the order downloads were added, so the first are the oldest; anything
    /// still queued, running or paused is never touched.
    fn prune_finished(&mut self, keep: usize) {
        let finished =
            self.downloads.iter().filter(|d| d.is_finished()).count();
        let mut excess = finished.saturating_sub(keep);
        if excess == 0 {
            return;
        }
        let evicted: Vec<Download> = self
            .downloads
            .extract_if(.., |d| {
                let evict = excess > 0 && d.is_finished();
                excess -= usize::from(evict);
                evict
            })
            .collect();
        self.forget_partials(&evicted);
    }

    /// Delete the partials of failed downloads that have just left the list.
    /// A failed transfer keeps its partial so a retry can resume it; once no
    /// listed download would, nothing accounts for the file any more and a
    /// daemon would collect them for as long as it runs.
    fn forget_partials(&self, forgotten: &[Download]) {
        let mut failed = forgotten
            .iter()
            .filter(|d| matches!(d.status, DownloadStatus::Failed(_)))
            .peekable();
        if failed.peek().is_none() {
            return;
        }
        let listed: std::collections::HashSet<_> =
            self.downloads.iter().map(partial_of).collect();
        for download in failed {
            if !listed.contains(&partial_of(download))
                && let Some(partial) = part_path_of(download)
            {
                let _ = std::fs::remove_file(partial);
            }
        }
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

    /// The live entry for a file wins over a finished one, so queueing the
    /// same file twice and cancelling once leaves the second request reachable.
    #[must_use]
    pub fn get_by_file(
        &self,
        username: &str,
        filename: &str,
    ) -> Option<&Download> {
        self.downloads
            .iter()
            .filter(|d| d.username == username && d.filename == filename)
            .min_by_key(|d| d.is_finished())
    }

    pub fn get_by_file_mut(
        &mut self,
        username: &str,
        filename: &str,
    ) -> Option<&mut Download> {
        self.downloads
            .iter_mut()
            .filter(|d| d.username == username && d.filename == filename)
            .min_by_key(|d| d.is_finished())
    }

    /// The reason to turn down a peer's offer to send `filename`, or `None`
    /// when a download is waiting for it. Accepting any other offer invites
    /// a file connection no download can claim.
    #[must_use]
    pub fn refusal_of_offer(
        &self,
        username: &str,
        filename: &str,
    ) -> Option<&'static str> {
        match self.get_by_file(username, filename).map(|d| &d.status) {
            Some(DownloadStatus::Queued) => None,
            Some(DownloadStatus::Completed) => Some("Complete"),
            _ => Some("Cancelled"),
        }
    }

    #[must_use]
    pub fn tokens(&self) -> Vec<u32> {
        self.downloads.iter().map(|d| d.token).collect()
    }

    #[must_use]
    pub const fn list(&self) -> &Vec<Download> {
        &self.downloads
    }

    /// The row a peer means by `token`. A token is only unique to the peer
    /// that chose it, and peers count from the same small numbers, so a
    /// lookup by token alone finds whichever row was added first: another
    /// peer's, or one that finished long ago.
    #[must_use]
    pub fn get(&self, username: &str, token: u32) -> Option<&Download> {
        self.downloads
            .iter()
            .filter(|d| d.token == token && d.username == username)
            .min_by_key(|d| d.is_finished())
    }

    fn get_mut(&mut self, username: &str, token: u32) -> Option<&mut Download> {
        self.downloads
            .iter_mut()
            .filter(|d| d.token == token && d.username == username)
            .min_by_key(|d| d.is_finished())
    }

    /// Give the download of `filename` the token its peer offered it under.
    ///
    /// The peer's number may already be on another of its queued downloads,
    /// as the token we gave that one before it was offered. Ours never
    /// reaches the peer, so that download takes a new one from `fresh`
    /// rather than answer to a file connection meant for this one.
    pub fn rekey(
        &mut self,
        username: &str,
        filename: &str,
        token: u32,
        size: u64,
        mut fresh: impl FnMut() -> u32,
    ) {
        let holds = |download: &Download, number: u32| {
            download.username == username && download.token == number
        };
        while let Some(index) = self.downloads.iter().position(|download| {
            holds(download, token)
                && download.filename != filename
                && matches!(download.status, DownloadStatus::Queued)
        }) {
            let mut replacement = fresh();
            while self.downloads.iter().any(|d| holds(d, replacement)) {
                replacement = fresh();
            }
            self.downloads[index].token = replacement;
        }
        if let Some(download) = self.get_by_file_mut(username, filename) {
            download.token = token;
            download.size = size;
        }
    }

    pub fn update_status(
        &mut self,
        username: &str,
        token: u32,
        status: DownloadStatus,
    ) -> bool {
        let Some(download) = self.get_mut(username, token) else {
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
        if download.is_finished() {
            self.prune_finished(MAX_FINISHED_DOWNLOADS);
        }
        true
    }

    /// Fail every queued download of `username` and tell each one's waiter.
    ///
    /// One pass and one prune: a peer can go away with thousands queued, and
    /// applying the cap row by row is quadratic under the store's lock.
    pub fn fail_queued(&mut self, username: &str, reason: &str) {
        for download in &mut self.downloads {
            if download.username != username
                || !matches!(download.status, DownloadStatus::Queued)
            {
                continue;
            }
            let failed = DownloadStatus::Failed(Some(reason.to_string()));
            let _ = download.sender.send(failed.clone());
            download.status = failed;
        }
        self.prune_finished(MAX_FINISHED_DOWNLOADS);
    }

    /// Tell the waiters of `username`'s queued downloads (all of them, or
    /// the one for `filename`) that the upload failed for `reason`, and
    /// forget those downloads. One pass under one lock: nothing can claim a
    /// download between its being told and its being gone.
    pub fn fail_and_forget_queued(
        &mut self,
        username: &str,
        filename: Option<&str>,
        reason: &str,
    ) {
        self.downloads.retain(|download| {
            if download.username != username
                || filename.is_some_and(|name| download.filename != name)
                || !matches!(download.status, DownloadStatus::Queued)
            {
                return true;
            }
            let _ = download
                .sender
                .send(DownloadStatus::Failed(Some(reason.to_string())));
            false
        });
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
        let download = self.get_mut(username, token)?;
        if !matches!(download.status, DownloadStatus::Queued) {
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
        let cancelled = download.clone();
        self.prune_finished(MAX_FINISHED_DOWNLOADS);
        Some(cancelled)
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

        // Told before it goes: whoever follows this download, here or
        // through a daemon, would otherwise wait on it for good.
        let removed = self.downloads.remove(index);
        let _ = removed.sender.send(DownloadStatus::Cancelled);
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

    // Issue #119: finished downloads were kept until a client removed them.
    #[test]
    fn finished_downloads_are_kept_only_up_to_a_limit() {
        let mut store = DownloadStore::new();
        store.add(make_download(1, DownloadStatus::Queued));
        let extra = 25;
        for token in 0..(MAX_FINISHED_DOWNLOADS as u32 + extra) {
            store.add(make_download(1000 + token, DownloadStatus::Completed));
        }

        assert_eq!(store.list().len(), MAX_FINISHED_DOWNLOADS + 1);
        assert!(
            store.get_by_token(1).is_some(),
            "a download still waiting is never pruned"
        );
        assert!(store.get_by_token(1000).is_none(), "the oldest go first");
        assert!(
            store
                .get_by_token(1000 + MAX_FINISHED_DOWNLOADS as u32 + extra - 1)
                .is_some()
        );
    }

    #[test]
    fn only_a_waiting_download_takes_a_peers_offer() {
        let mut store = DownloadStore::new();
        store.add(make_download(1, DownloadStatus::Queued));
        store.add(make_download(2, DownloadStatus::Completed));
        store.add(make_download(3, DownloadStatus::Cancelled));

        assert_eq!(store.refusal_of_offer("peer", "file-1.mp3"), None);
        assert_eq!(
            store.refusal_of_offer("peer", "file-2.mp3"),
            Some("Complete")
        );
        assert_eq!(
            store.refusal_of_offer("peer", "file-3.mp3"),
            Some("Cancelled")
        );
        assert_eq!(
            store.refusal_of_offer("peer", "never-asked.mp3"),
            Some("Cancelled")
        );
        assert_eq!(
            store.refusal_of_offer("stranger", "file-1.mp3"),
            Some("Cancelled")
        );
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
    fn cancel_by_file_reaches_the_second_request_for_the_same_file() {
        let mut store = DownloadStore::new();
        let mut first = make_download(1, DownloadStatus::Queued);
        first.filename = "song.mp3".to_string();
        let mut second = make_download(2, DownloadStatus::Queued);
        second.filename = "song.mp3".to_string();
        store.add(first);
        store.add(second);

        assert_eq!(store.cancel_by_file("peer", "song.mp3").unwrap().token, 1);
        assert_eq!(store.cancel_by_file("peer", "song.mp3").unwrap().token, 2);
        assert!(store.cancel_by_file("peer", "song.mp3").is_none());
    }

    #[test]
    fn get_by_file_prefers_the_live_entry_over_a_cancelled_one() {
        let mut store = DownloadStore::new();
        let mut cancelled = make_download(1, DownloadStatus::Cancelled);
        cancelled.filename = "song.mp3".to_string();
        let mut queued = make_download(2, DownloadStatus::Queued);
        queued.filename = "song.mp3".to_string();
        store.add(cancelled);
        store.add(queued);

        assert_eq!(store.get_by_file("peer", "song.mp3").unwrap().token, 2);
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
            "peer",
            1,
            DownloadStatus::InProgress {
                bytes_downloaded: 0,
                total_bytes: 100,
                speed_bytes_per_sec: 0.0,
            },
        ));
        assert!(!store.update_status(
            "peer",
            1,
            DownloadStatus::Failed(Some("late".into()))
        ));
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
            "peer",
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
    fn removing_a_queued_download_tells_its_waiter_it_was_cancelled() {
        let mut store = DownloadStore::new();
        let (tx, rx) = mpsc::channel();
        let mut download = make_download(1, DownloadStatus::Queued);
        download.sender = tx;
        store.add(download);

        assert!(store.remove_queued_by_file("peer", "file-1.mp3"));

        assert!(store.get_by_token(1).is_none());
        assert!(matches!(
            rx.try_recv().expect("removal must notify the waiter"),
            DownloadStatus::Cancelled
        ));
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
    fn failing_a_peers_queue_leaves_started_and_finished_downloads() {
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

        store.fail_and_forget_queued("peer", None, "gone");

        assert_eq!(
            store.tokens(),
            vec![1, 2, 3],
            "only the queued download may fail"
        );
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
    fn failing_one_queued_file_tells_its_waiter_and_forgets_only_it() {
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

        store.fail_and_forget_queued("peer", Some("song.mp3"), "gone");

        assert_eq!(store.tokens(), vec![2, 3]);
        assert!(matches!(
            rx_match.try_recv().unwrap(),
            DownloadStatus::Failed(Some(reason)) if reason == "gone"
        ));
    }

    // Peers pick their own transfer tokens and count from the same small
    // numbers.
    #[test]
    fn two_peers_using_the_same_token_keep_their_own_downloads() {
        let mut store = DownloadStore::new();
        let (alice_tx, _alice_rx) = mpsc::channel();
        let (bob_tx, _bob_rx) = mpsc::channel();
        for (username, sender) in [("alice", alice_tx), ("bob", bob_tx)] {
            let mut download = make_download(7, DownloadStatus::Queued);
            download.username = username.to_string();
            download.sender = sender;
            store.add(download);
        }

        assert!(store.update_status("alice", 7, DownloadStatus::Completed));
        assert_eq!(
            store.claim_for_peer(7, "bob").map(|d| d.username),
            Some("bob".to_string()),
            "a finished row with the same token must not stand in the way"
        );
    }

    #[test]
    fn an_offered_token_already_on_the_peers_other_download_moves_aside() {
        let mut store = DownloadStore::new();
        for (token, filename) in [(5, "x.mp3"), (6, "y.mp3")] {
            let mut download = make_download(token, DownloadStatus::Queued);
            download.filename = filename.to_string();
            store.add(download);
        }

        // The peer offers y under 5, the number x was queued with.
        let mut next = 5;
        store.rekey("peer", "y.mp3", 5, 200, || {
            next += 1;
            next
        });

        assert_eq!(
            store.claim_for_peer(5, "peer").map(|d| d.filename),
            Some("y.mp3".to_string()),
            "the file connection for y must not be handed x"
        );
        let x = store.get_by_file("peer", "x.mp3").expect("x is kept").token;
        assert_ne!(x, 5);
        assert_eq!(
            store.claim_for_peer(x, "peer").map(|d| d.filename),
            Some("x.mp3".to_string()),
            "x still waits for its own offer"
        );
    }

    /// A store whose downloads land in a directory of this test's own, and
    /// a failed download in it with a partial on disk.
    fn failed_with_a_partial(
        test: &str,
        token: u32,
    ) -> (Download, std::path::PathBuf) {
        let dir = std::env::temp_dir()
            .join(format!("soulseek-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut download = make_download(token, DownloadStatus::Failed(None));
        download.download_directory = dir.display().to_string();
        let partial = part_path_of(&download).expect("a part path");
        std::fs::write(&partial, b"half").unwrap();
        (download, std::path::PathBuf::from(partial))
    }

    fn finish(store: &mut DownloadStore, downloads: usize) {
        for token in 10..(10 + downloads as u32) {
            store.add(make_download(token, DownloadStatus::Completed));
        }
    }

    fn clean_up(partial: &std::path::Path) {
        let _ = std::fs::remove_dir_all(partial.parent().expect("a folder"));
    }

    #[test]
    fn a_failed_download_takes_its_partial_with_it_when_it_is_forgotten() {
        let (oldest, forgotten) = failed_with_a_partial("evicted", 1);
        let (newest, kept) = failed_with_a_partial("evicted", 2);

        let mut store = DownloadStore::new();
        store.add(oldest);
        finish(&mut store, MAX_FINISHED_DOWNLOADS - 1);
        store.add(newest);

        assert!(store.get_by_token(1).is_none(), "the oldest is evicted");
        assert!(!forgotten.exists(), "and its partial goes with it");
        assert!(kept.exists(), "a failed download still listed can resume");
        clean_up(&kept);
    }

    #[test]
    fn a_partial_another_listed_download_resumes_outlives_the_forgotten_one() {
        let (failed, partial) = failed_with_a_partial("shared", 1);
        let mut resuming = make_download(2, DownloadStatus::Queued);
        resuming.filename.clone_from(&failed.filename);
        resuming
            .download_directory
            .clone_from(&failed.download_directory);

        let mut store = DownloadStore::new();
        store.add(failed);
        store.add(resuming);
        finish(&mut store, MAX_FINISHED_DOWNLOADS);

        assert!(store.get_by_token(1).is_none(), "the failed one is evicted");
        assert!(partial.exists(), "the queued one still resumes from it");
        clean_up(&partial);
    }

    #[test]
    fn a_retry_for_a_different_size_drops_the_partial_it_cannot_resume() {
        let (failed, stale) = failed_with_a_partial("resized", 1);
        let mut retry = make_download(2, DownloadStatus::Queued);
        retry.filename.clone_from(&failed.filename);
        retry
            .download_directory
            .clone_from(&failed.download_directory);
        retry.size = failed.size + 1;

        let mut store = DownloadStore::new();
        store.add(failed);
        store.add(retry);

        assert!(!stale.exists(), "no download would ever pick it up again");
        clean_up(&stale);
    }
}
