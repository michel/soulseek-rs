use super::MainTui;
use crate::models::{DownloadEntry, FileDisplayData};
use soulseek_rs::{DownloadStatus, types::Download};
use std::{sync::mpsc, thread};

/// Done, given up, or ran out of time: the row is all that is left.
const fn is_finished(download: &Download) -> bool {
    matches!(
        download.status,
        DownloadStatus::Completed
            | DownloadStatus::Failed(_)
            | DownloadStatus::TimedOut
    )
}

impl MainTui {
    /// Channel for queued downloads, created on first use.
    pub(super) fn downloads_sender(
        &mut self,
    ) -> mpsc::Sender<(Download, mpsc::Receiver<DownloadStatus>)> {
        if let Some(sender) = &self.state.downloads_sender_channel {
            return sender.clone();
        }
        let (sender, receiver) = mpsc::channel();
        self.state.downloads_receiver_channel = Some(receiver);
        self.state.downloads_sender_channel = Some(sender.clone());
        sender
    }

    /// Re-queue the selected download if it failed or timed out.
    pub(super) fn retry_selected_download(&mut self) {
        let Some(index) = self.state.downloads_table_state.selected() else {
            return;
        };
        let Some(entry) = self.state.downloads.get(index) else {
            return;
        };
        if !matches!(
            entry.download.status,
            DownloadStatus::Failed(_) | DownloadStatus::TimedOut
        ) {
            return;
        }
        let filename = entry.download.filename.clone();
        let username = entry.download.username.clone();
        let size = entry.download.size;
        let directory = entry.download.download_directory.clone();

        let sender = self.downloads_sender();
        let client = self.client.clone();

        // Drop the old failed entry from both the UI list and the client's
        // download store; otherwise the stale entry (same md5-derived token and
        // filename) shadows the retry and its completion is misrouted, leaving
        // the retried download stuck showing "Queued".
        self.state.downloads.remove(index);
        self.select_download_after_removal(index);
        let _ = self.client.remove_download(&username, &filename);

        thread::spawn(move || {
            match client.download(filename.clone(), username, size, directory) {
                Ok((download, rx)) => {
                    let _ = sender.send((download, rx));
                }
                Err(e) => soulseek_rs::warn!("Failed to retry {filename}: {e}"),
            }
        });
    }

    /// Remove all completed / failed / timed-out downloads from the list.
    pub(super) fn clear_finished_downloads(&mut self) {
        // Forget them in the session too. The list is a view of what the
        // session holds, so dropping them only here would last until the next
        // poll put them straight back.
        for entry in &self.state.downloads {
            if is_finished(&entry.download) {
                self.client.remove_download(
                    &entry.download.username,
                    &entry.download.filename,
                );
            }
        }
        self.state
            .downloads
            .retain(|entry| !is_finished(&entry.download));
        let len = self.state.downloads.len();
        if len == 0 {
            self.state.downloads_table_state.select(None);
        } else {
            let selected = self
                .state
                .downloads_table_state
                .selected()
                .unwrap_or(0)
                .min(len - 1);
            self.state.downloads_table_state.select(Some(selected));
        }
    }

    pub(super) fn toggle_selected_download_pause(&self) {
        let Some(index) = self.state.downloads_table_state.selected() else {
            return;
        };
        let Some(download_entry) = self.state.downloads.get(index) else {
            return;
        };

        let download = &download_entry.download;
        match download.status {
            DownloadStatus::InProgress { .. } => {
                let _ = self
                    .client
                    .pause_download(&download.username, &download.filename);
            }
            DownloadStatus::Paused { .. } => {
                let _ = self
                    .client
                    .resume_download(&download.username, &download.filename);
            }
            _ => {}
        }
    }

    /// Drop the selected transfer: a queued one leaves the queue, a finished
    /// one leaves the list.
    pub(super) fn remove_selected_download(&mut self) {
        let Some(index) = self.state.downloads_table_state.selected() else {
            return;
        };
        let Some(download_entry) = self.state.downloads.get(index) else {
            return;
        };

        let download = &download_entry.download;
        let dismissed = match download.status {
            DownloadStatus::Queued => self
                .client
                .remove_queued_download(&download.username, &download.filename),
            // A finished transfer may exist only in this window (restored from
            // an earlier run), so the session's "nothing to forget" is not a
            // reason to keep the row.
            DownloadStatus::Completed => {
                self.client
                    .remove_download(&download.username, &download.filename);
                true
            }
            _ => false,
        };
        if !dismissed {
            return;
        }

        self.state.downloads.remove(index);
        self.select_download_after_removal(index);
    }

    fn select_download_after_removal(&mut self, removed_index: usize) {
        if self.state.downloads.is_empty() {
            self.state.downloads_table_state.select(None);
            return;
        }

        let next_index = removed_index.min(self.state.downloads.len() - 1);
        self.state.downloads_table_state.select(Some(next_index));
    }

    pub(super) fn queue_selected_downloads(&mut self) {
        let selected_files: Vec<FileDisplayData> = self
            .state
            .results_selected_indices
            .iter()
            .filter_map(|&idx| self.state.results_items.get(idx))
            .cloned()
            .collect();

        if selected_files.is_empty() {
            return;
        }

        let sender = self.downloads_sender();
        let client = self.client.clone();
        let download_dir = self.download_dir.clone();

        thread::spawn(move || {
            for file in selected_files {
                let metadata = soulseek_rs::types::DownloadMetadata {
                    bitrate: file.bitrate,
                    length_seconds: file.length_seconds,
                    peer_upload_speed: Some(file.speed),
                    peer_free_slots: Some(file.slots),
                };
                match client.download_with_metadata(
                    file.filename.clone(),
                    file.username.clone(),
                    file.size,
                    download_dir.clone(),
                    metadata,
                ) {
                    Ok((download, rx)) => {
                        let _ = sender.send((download, rx));
                    }
                    Err(e) => {
                        soulseek_rs::warn!(
                            "Failed to start download for {}: {}",
                            file.filename,
                            e
                        );
                    }
                }
            }
        });

        // Clear selection
        self.state.results_selected_indices.clear();
    }

    /// Cancel the selected transfer when it is an upload row (uploads are
    /// listed after the downloads in the shared pane).
    pub(super) fn cancel_selected_upload(&self) {
        let Some(index) = self.state.downloads_table_state.selected() else {
            return;
        };
        let Some(upload) = self
            .state
            .uploads
            .get(index.wrapping_sub(self.state.downloads.len()))
        else {
            return;
        };
        if upload.status == soulseek_rs::types::UploadStatus::InProgress
            && !self
                .client
                .cancel_upload(&upload.username, &upload.filename)
        {
            soulseek_rs::warn!(
                "No in-progress upload of {} to {} to cancel",
                upload.filename,
                upload.username
            );
        }
    }

    pub(super) fn update_downloads(&mut self) {
        if let Some(ref receiver) = self.state.downloads_receiver_channel {
            while let Ok((download, download_receiver)) = receiver.try_recv() {
                // A transfer of ours can already be listed: the session
                // registers it the moment it is queued, so a frame between
                // that and the worker handing it over here will have picked it
                // up from the session. Take over that row rather than adding a
                // second one for the same file.
                let existing = self.state.downloads.iter_mut().find(|entry| {
                    entry.download.username == download.username
                        && entry.download.filename == download.filename
                });
                match existing {
                    Some(entry) => {
                        entry.download = download;
                        entry.receiver = Some(download_receiver);
                    }
                    None => self.state.downloads.push(DownloadEntry {
                        download,
                        receiver: Some(download_receiver),
                    }),
                }
            }
        }

        self.sync_downloads_with_session();

        self.state.active_downloads_count = 0;
        for download_entry in &mut self.state.downloads {
            if let Some(ref receiver) = download_entry.receiver {
                while let Ok(status) = receiver.try_recv() {
                    download_entry.download.status = status;
                }
            }

            if matches!(
                download_entry.download.status,
                DownloadStatus::InProgress { .. }
            ) {
                self.state.active_downloads_count += 1;
            }
        }
    }

    /// Make the list show the session's transfers, not just this window's.
    ///
    /// The queue belongs to the session, so against a daemon it includes what
    /// another client queued and what was still running before this window
    /// opened. Transfers this window started keep their own status channel,
    /// which reports sooner than a poll; everything else is refreshed from the
    /// session each tick.
    fn sync_downloads_with_session(&mut self) {
        // Only worth doing when the session is shared. A window with a
        // session of its own started every transfer in it, so there is
        // nothing to learn — and the list also holds completed transfers
        // restored from disk that the session has never heard of, which this
        // reconciliation would take to mean "cancelled elsewhere" and delete.
        if self.client.daemon_endpoint().is_none() {
            return;
        }

        let known = self.client.get_all_downloads();

        for download in &known {
            let existing = self.state.downloads.iter_mut().find(|entry| {
                entry.download.username == download.username
                    && entry.download.filename == download.filename
            });
            match existing {
                // Ours: the channel is more current than this snapshot.
                Some(entry) if entry.receiver.is_some() => {}
                Some(entry) => entry.download.status = download.status.clone(),
                None => self.state.downloads.push(DownloadEntry {
                    download: download.clone(),
                    receiver: None,
                }),
            }
        }

        // Anything the session has forgotten is gone for every window.
        self.state.downloads.retain(|entry| {
            entry.receiver.is_some()
                || known.iter().any(|download| {
                    download.username == entry.download.username
                        && download.filename == entry.download.filename
                })
        });
    }
}
