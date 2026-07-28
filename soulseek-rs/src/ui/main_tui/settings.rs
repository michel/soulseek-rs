//! Settings popup behavior: open, key handling, and applying changes —
//! persisted to config.toml, applied live to the session (pushed over the
//! socket when the session is a daemon's).

use super::MainTui;
use crate::models::{SettingsAction, SettingsState};
use ratatui::crossterm::event::KeyEvent;

impl MainTui {
    pub(super) fn open_settings(&mut self) {
        self.state.settings = Some(SettingsState::new(
            self.download_dir.clone(),
            self.client.shared_directories(),
        ));
    }

    pub(super) fn handle_settings_input(&mut self, key: KeyEvent) {
        let Some(settings) = self.state.settings.as_mut() else {
            return;
        };
        match settings.handle_key(key) {
            SettingsAction::None => {}
            SettingsAction::Close => self.state.settings = None,
            SettingsAction::Apply => self.apply_settings(),
            SettingsAction::Reindex => self.reindex_shares(),
        }
    }

    /// Persist the edited settings and apply them live.
    ///
    /// Attached to a daemon, both folders name paths on the *daemon's*
    /// machine: they are pushed over the socket and validated there, and no
    /// directory is created on this one. Locally they are this machine's and
    /// are validated here.
    pub(super) fn apply_settings(&mut self) {
        let Some(settings) = self.state.settings.as_ref() else {
            return;
        };
        let download_dir = settings.download_dir.clone();
        let share_dirs = settings.share_dirs.clone();

        let shares = if self.client.daemon_endpoint().is_some() {
            share_dirs.clone()
        } else {
            if let Err(e) = std::fs::create_dir_all(&download_dir) {
                self.set_settings_status(format!(
                    "Cannot create {download_dir}: {e}"
                ));
                return;
            }
            // Validate the share paths (tilde-expand, must exist).
            crate::directories::resolve_shared_directories(&share_dirs)
        };
        let dropped = share_dirs.len() - shares.len();

        // Two independent applies: a failure of one must not misreport or
        // silently discard the other, so both run and both are told.
        let mut refused = Vec::new();
        let dir_applied =
            match self.client.set_download_directory(download_dir.clone()) {
                Ok(()) => true,
                Err(e) => {
                    refused.push(format!("download folder: {e}"));
                    false
                }
            };
        if let Err(e) = self.client.set_shared_directories(shares) {
            refused.push(format!("shares: {e}"));
        }

        let mut status = if !refused.is_empty() {
            format!("Could not apply {}", refused.join("; "))
        } else if dropped > 0 {
            format!("Applied ({dropped} invalid path(s) ignored)")
        } else {
            format!("Applied · sharing {}", self.share_counts())
        };
        if dir_applied {
            self.download_dir.clone_from(&download_dir);
        }

        // Persist to config.toml so the change survives a restart. What was
        // refused live is not written either: the file must not promise a
        // folder the session never accepted.
        if let Some(path) = self.config_path.clone() {
            let result = crate::persist::config::FileConfig::load(&path)
                .and_then(|mut config| {
                    if dir_applied {
                        config.download_dir = Some(download_dir);
                    }
                    // The singular `shared_dir` is merged with `shared_dirs` on
                    // load, so leaving it would resurrect a removed folder.
                    config.shared_dir = None;
                    config.shared_dirs = Some(share_dirs);
                    config.save(&path)
                });
            if let Err(e) = result {
                status = format!("Could not save config: {e}");
            }
        }
        self.set_settings_status(status);
    }

    fn set_settings_status(&mut self, status: String) {
        if let Some(settings) = self.state.settings.as_mut() {
            settings.status = Some(status);
        }
    }

    /// Re-scan the current share paths (picks up files changed on disk).
    fn reindex_shares(&mut self) {
        let dirs = self.client.shared_directories();
        let result = self.client.set_shared_directories(dirs);
        let counts = self.share_counts();
        if let Some(settings) = self.state.settings.as_mut() {
            settings.status = Some(match result {
                Ok(()) => format!("Re-indexed · sharing {counts}"),
                Err(e) => format!("Re-index failed: {e}"),
            });
        }
    }

    fn share_counts(&self) -> String {
        let (folders, files) = self.client.shared_counts();
        format!("{files} files in {folders} folders")
    }
}
