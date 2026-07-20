//! Settings popup behavior: open, key handling, and applying changes
//! (persist to config.toml + live share rescan on the client).

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

    /// Persist the edited settings and apply the share set live.
    fn apply_settings(&mut self) {
        let Some(settings) = self.state.settings.as_ref() else {
            return;
        };
        let download_dir = settings.download_dir.clone();
        let share_dirs = settings.share_dirs.clone();

        if let Err(e) = std::fs::create_dir_all(&download_dir) {
            self.set_settings_status(format!(
                "Cannot create {download_dir}: {e}"
            ));
            return;
        }
        self.download_dir.clone_from(&download_dir);

        // Validate the share paths (tilde-expand, must exist) and apply.
        let valid = crate::directories::resolve_shared_directories(&share_dirs);
        let dropped = share_dirs.len() - valid.len();
        let mut status = match self.client.set_shared_directories(valid) {
            Ok(()) if dropped > 0 => {
                format!("Applied ({dropped} invalid path(s) ignored)")
            }
            Ok(()) => format!("Applied · sharing {}", self.share_counts()),
            Err(e) => format!("Could not apply shares: {e}"),
        };

        // Persist to config.toml so the change survives a restart.
        if let Some(path) = crate::persist::paths::config_file() {
            let result = crate::persist::config::FileConfig::load(&path)
                .and_then(|mut config| {
                    config.download_dir = Some(download_dir);
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
