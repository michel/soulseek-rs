//! State machine for the settings popup (download folder + share paths).
//! Pure — no client or terminal — so every transition is unit-testable.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

/// Which row is highlighted: 0 = download dir, 1.. = share paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsMode {
    /// Moving between rows.
    Navigate,
    /// Typing a new download directory.
    EditingDownloadDir,
    /// Typing a new share path to add.
    AddingShare,
}

/// What the TUI should do after a key was handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    None,
    /// Settings changed: persist config and apply the new share set live.
    Apply,
    /// Re-scan the current share paths (files changed on disk).
    Reindex,
    /// Close the popup.
    Close,
}

pub struct SettingsState {
    pub download_dir: String,
    pub share_dirs: Vec<String>,
    /// 0 = download dir row; 1 + i = share path i.
    pub selected: usize,
    pub mode: SettingsMode,
    /// Edit buffer while typing.
    pub input: String,
    /// One-line feedback ("Re-indexed: 1234 files in 56 folders").
    pub status: Option<String>,
}

impl SettingsState {
    #[must_use]
    pub const fn new(download_dir: String, share_dirs: Vec<String>) -> Self {
        Self {
            download_dir,
            share_dirs,
            selected: 0,
            mode: SettingsMode::Navigate,
            input: String::new(),
            status: None,
        }
    }

    const fn rows(&self) -> usize {
        1 + self.share_dirs.len()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SettingsAction {
        match self.mode {
            SettingsMode::Navigate => self.handle_navigate(key),
            SettingsMode::EditingDownloadDir | SettingsMode::AddingShare => {
                self.handle_typing(key)
            }
        }
    }

    fn handle_navigate(&mut self, key: KeyEvent) -> SettingsAction {
        self.status = None;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'o') => SettingsAction::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                SettingsAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.rows() - 1);
                SettingsAction::None
            }
            KeyCode::Enter | KeyCode::Char('e') if self.selected == 0 => {
                self.mode = SettingsMode::EditingDownloadDir;
                self.input = self.download_dir.clone();
                SettingsAction::None
            }
            KeyCode::Char('a') => {
                self.mode = SettingsMode::AddingShare;
                self.input.clear();
                SettingsAction::None
            }
            KeyCode::Char('d') if self.selected > 0 => {
                self.share_dirs.remove(self.selected - 1);
                self.selected = self.selected.min(self.rows() - 1);
                SettingsAction::Apply
            }
            KeyCode::Char('r') => SettingsAction::Reindex,
            _ => SettingsAction::None,
        }
    }

    fn handle_typing(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = SettingsMode::Navigate;
                self.input.clear();
                SettingsAction::None
            }
            KeyCode::Enter => {
                let value = self.input.trim().to_string();
                let adding = self.mode == SettingsMode::AddingShare;
                self.mode = SettingsMode::Navigate;
                self.input.clear();
                if value.is_empty() {
                    return SettingsAction::None;
                }
                if adding {
                    if self.share_dirs.contains(&value) {
                        return SettingsAction::None;
                    }
                    self.share_dirs.push(value);
                    self.selected = self.rows() - 1;
                } else {
                    self.download_dir = value;
                }
                SettingsAction::Apply
            }
            KeyCode::Backspace => {
                self.input.pop();
                SettingsAction::None
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                SettingsAction::None
            }
            _ => SettingsAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_str(state: &mut SettingsState, text: &str) {
        for c in text.chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
    }

    fn state() -> SettingsState {
        SettingsState::new("/dl".into(), vec!["/dl".into(), "/music".into()])
    }

    #[test]
    fn escape_closes_from_navigation() {
        assert_eq!(
            state().handle_key(key(KeyCode::Esc)),
            SettingsAction::Close
        );
    }

    #[test]
    fn adding_a_share_path_applies() {
        let mut s = state();
        s.handle_key(key(KeyCode::Char('a')));
        assert_eq!(s.mode, SettingsMode::AddingShare);
        type_str(&mut s, "/flacs");
        let action = s.handle_key(key(KeyCode::Enter));
        assert_eq!(action, SettingsAction::Apply);
        assert_eq!(s.share_dirs, vec!["/dl", "/music", "/flacs"]);
        assert_eq!(s.mode, SettingsMode::Navigate);
    }

    #[test]
    fn adding_a_duplicate_share_is_a_no_op() {
        let mut s = state();
        s.handle_key(key(KeyCode::Char('a')));
        type_str(&mut s, "/music");
        assert_eq!(s.handle_key(key(KeyCode::Enter)), SettingsAction::None);
        assert_eq!(s.share_dirs.len(), 2);
    }

    #[test]
    fn deleting_the_selected_share_applies() {
        let mut s = state();
        s.handle_key(key(KeyCode::Down)); // select share 0
        s.handle_key(key(KeyCode::Down)); // select share 1 (/music)
        let action = s.handle_key(key(KeyCode::Char('d')));
        assert_eq!(action, SettingsAction::Apply);
        assert_eq!(s.share_dirs, vec!["/dl"]);
    }

    #[test]
    fn delete_on_the_download_dir_row_does_nothing() {
        let mut s = state();
        assert_eq!(s.handle_key(key(KeyCode::Char('d'))), SettingsAction::None);
        assert_eq!(s.share_dirs.len(), 2);
    }

    #[test]
    fn editing_the_download_dir_applies() {
        let mut s = state();
        s.handle_key(key(KeyCode::Enter));
        assert_eq!(s.mode, SettingsMode::EditingDownloadDir);
        assert_eq!(s.input, "/dl");
        s.handle_key(key(KeyCode::Backspace));
        s.handle_key(key(KeyCode::Backspace));
        s.handle_key(key(KeyCode::Backspace));
        type_str(&mut s, "/new");
        assert_eq!(s.handle_key(key(KeyCode::Enter)), SettingsAction::Apply);
        assert_eq!(s.download_dir, "/new");
    }

    #[test]
    fn escape_cancels_an_edit_without_applying() {
        let mut s = state();
        s.handle_key(key(KeyCode::Enter));
        type_str(&mut s, "junk");
        assert_eq!(s.handle_key(key(KeyCode::Esc)), SettingsAction::None);
        assert_eq!(s.download_dir, "/dl");
        assert_eq!(s.mode, SettingsMode::Navigate);
    }

    #[test]
    fn r_requests_a_reindex() {
        assert_eq!(
            state().handle_key(key(KeyCode::Char('r'))),
            SettingsAction::Reindex
        );
    }

    #[test]
    fn selection_is_clamped_to_the_row_count() {
        let mut s = state();
        for _ in 0..10 {
            s.handle_key(key(KeyCode::Down));
        }
        assert_eq!(s.selected, 2);
        for _ in 0..10 {
            s.handle_key(key(KeyCode::Up));
        }
        assert_eq!(s.selected, 0);
    }
}
