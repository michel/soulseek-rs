//! State machine for the settings popup (account, download folder, shares).
//! Pure — no client or terminal — so every transition is unit-testable.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

/// What the popup is doing with the keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsMode {
    /// Moving between rows.
    Navigate,
    /// Typing a new download directory.
    EditingDownloadDir,
    /// Typing a new share path to add.
    AddingShare,
    /// Typing a new password; the keystrokes are masked.
    NewPassword,
    /// Typing it again, with the first entry held here. A password reaches the
    /// server unconfirmed, so a typo entered once would change the account to
    /// something nobody knows.
    RepeatPassword(String),
    /// Waiting for "log out?" to be answered.
    ConfirmingLogout,
}

impl SettingsMode {
    /// Whether what is being typed must not be shown.
    const fn is_secret(&self) -> bool {
        matches!(self, Self::NewPassword | Self::RepeatPassword(_))
    }
}

/// One line the selection can rest on.
///
/// The rows are derived from the state rather than counted, because the logout
/// row is absent when the session belongs to a daemon and index arithmetic
/// would silently shift with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsRow {
    ChangePassword,
    Logout,
    DownloadDir,
    /// Share path at this index.
    Share(usize),
}

/// What the TUI should do after a key was handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    None,
    /// Settings changed: persist config and apply the new share set live.
    Apply,
    /// Re-scan the current share paths (files changed on disk).
    Reindex,
    /// Close the popup.
    Close,
    /// Make this the account's password.
    ChangePassword(String),
    /// End the session and forget the stored password.
    Logout,
}

/// Who the session is logged in as and what it offers the network.
///
/// Captured when the popup opens rather than read while drawing: attached to a
/// daemon every one of these is a round trip, and the popup redraws each tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountInfo {
    pub username: String,
    /// Already formatted, e.g. "1234 files in 56 folders".
    pub shares: String,
    /// The port peers can reach us on, when the listener is up.
    pub listen_port: Option<u16>,
    /// The daemon this session is borrowed from. `Some` means the account is
    /// the daemon's, so this window cannot log it out.
    pub daemon: Option<String>,
}

pub struct SettingsState {
    pub account: AccountInfo,
    pub download_dir: String,
    pub share_dirs: Vec<String>,
    /// Index into [`Self::rows`].
    pub selected: usize,
    pub mode: SettingsMode,
    /// Edit buffer while typing.
    pub input: String,
    /// One-line feedback ("Re-indexed: 1234 files in 56 folders").
    pub status: Option<String>,
}

impl SettingsState {
    #[must_use]
    pub const fn new(
        account: AccountInfo,
        download_dir: String,
        share_dirs: Vec<String>,
    ) -> Self {
        Self {
            account,
            download_dir,
            share_dirs,
            selected: 0,
            mode: SettingsMode::Navigate,
            input: String::new(),
            status: None,
        }
    }

    /// The daemon owns its own login; a window borrowing it can only detach.
    #[must_use]
    pub const fn can_log_out(&self) -> bool {
        self.account.daemon.is_none()
    }

    /// Every row, in the order they are drawn.
    #[must_use]
    pub fn rows(&self) -> Vec<SettingsRow> {
        let mut rows = vec![SettingsRow::ChangePassword];
        if self.can_log_out() {
            rows.push(SettingsRow::Logout);
        }
        rows.push(SettingsRow::DownloadDir);
        rows.extend((0..self.share_dirs.len()).map(SettingsRow::Share));
        rows
    }

    fn last_row(&self) -> usize {
        self.rows().len() - 1
    }

    /// The row the selection rests on.
    #[must_use]
    pub fn selected_row(&self) -> SettingsRow {
        self.rows()[self.selected.min(self.last_row())]
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SettingsAction {
        match self.mode {
            SettingsMode::Navigate => self.handle_navigate(key),
            SettingsMode::ConfirmingLogout => self.handle_logout_confirm(key),
            SettingsMode::EditingDownloadDir
            | SettingsMode::AddingShare
            | SettingsMode::NewPassword
            | SettingsMode::RepeatPassword(_) => self.handle_typing(key),
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
                self.selected = (self.selected + 1).min(self.last_row());
                SettingsAction::None
            }
            KeyCode::Enter => self.activate(),
            // 'e' has always meant "edit this folder"; it must not reach a row
            // that logs the account out.
            KeyCode::Char('e')
                if self.selected_row() == SettingsRow::DownloadDir =>
            {
                self.activate()
            }
            KeyCode::Char('a') => {
                self.mode = SettingsMode::AddingShare;
                self.input.clear();
                SettingsAction::None
            }
            KeyCode::Char('d') => {
                let SettingsRow::Share(index) = self.selected_row() else {
                    return SettingsAction::None;
                };
                self.share_dirs.remove(index);
                self.selected = self.selected.min(self.last_row());
                SettingsAction::Apply
            }
            KeyCode::Char('r') => SettingsAction::Reindex,
            _ => SettingsAction::None,
        }
    }

    fn activate(&mut self) -> SettingsAction {
        match self.selected_row() {
            SettingsRow::ChangePassword => {
                self.mode = SettingsMode::NewPassword;
                // Defensive: a password buffer is never left holding anything.
                self.input.clear();
            }
            SettingsRow::Logout => self.mode = SettingsMode::ConfirmingLogout,
            SettingsRow::DownloadDir => {
                self.mode = SettingsMode::EditingDownloadDir;
                self.input.clone_from(&self.download_dir);
            }
            SettingsRow::Share(_) => {}
        }
        SettingsAction::None
    }

    fn handle_logout_confirm(&mut self, key: KeyEvent) -> SettingsAction {
        self.mode = SettingsMode::Navigate;
        if matches!(key.code, KeyCode::Char('y' | 'Y') | KeyCode::Enter) {
            return SettingsAction::Logout;
        }
        SettingsAction::None
    }

    fn handle_typing(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = SettingsMode::Navigate;
                self.input.clear();
                SettingsAction::None
            }
            KeyCode::Enter => self.submit(),
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

    fn submit(&mut self) -> SettingsAction {
        if self.mode.is_secret() {
            return self.submit_password();
        }
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
            self.selected = self.last_row();
        } else {
            self.download_dir = value;
        }
        SettingsAction::Apply
    }

    /// The first entry is only remembered; the second is what commits.
    fn submit_password(&mut self) -> SettingsAction {
        let typed = std::mem::take(&mut self.input);
        let SettingsMode::RepeatPassword(first) =
            std::mem::replace(&mut self.mode, SettingsMode::Navigate)
        else {
            if !typed.is_empty() {
                self.mode = SettingsMode::RepeatPassword(typed);
            }
            return SettingsAction::None;
        };
        if typed != first {
            self.status =
                Some("The two passwords differ — nothing was changed".into());
            return SettingsAction::None;
        }
        SettingsAction::ChangePassword(typed)
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

    fn account() -> AccountInfo {
        AccountInfo {
            username: "alice".into(),
            shares: "12 files in 3 folders".into(),
            listen_port: Some(2234),
            daemon: None,
        }
    }

    fn state() -> SettingsState {
        SettingsState::new(
            account(),
            "/dl".into(),
            vec!["/dl".into(), "/music".into()],
        )
    }

    /// Put the selection on a row by name, whatever index it has.
    fn select(state: &mut SettingsState, row: SettingsRow) {
        state.selected = state
            .rows()
            .iter()
            .position(|candidate| *candidate == row)
            .expect("the row is present");
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
        assert_eq!(s.selected_row(), SettingsRow::Share(2));
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
        select(&mut s, SettingsRow::Share(1));
        let action = s.handle_key(key(KeyCode::Char('d')));
        assert_eq!(action, SettingsAction::Apply);
        assert_eq!(s.share_dirs, vec!["/dl"]);
    }

    #[test]
    fn delete_on_a_row_that_is_not_a_share_does_nothing() {
        let mut s = state();
        select(&mut s, SettingsRow::DownloadDir);
        assert_eq!(s.handle_key(key(KeyCode::Char('d'))), SettingsAction::None);
        assert_eq!(s.share_dirs.len(), 2);
    }

    #[test]
    fn editing_the_download_dir_applies() {
        let mut s = state();
        select(&mut s, SettingsRow::DownloadDir);
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
        select(&mut s, SettingsRow::DownloadDir);
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
        assert_eq!(s.selected_row(), SettingsRow::Share(1));
        for _ in 0..10 {
            s.handle_key(key(KeyCode::Up));
        }
        assert_eq!(s.selected_row(), SettingsRow::ChangePassword);
    }

    #[test]
    fn a_password_typed_twice_the_same_way_is_changed() {
        let mut s = state();
        select(&mut s, SettingsRow::ChangePassword);
        s.handle_key(key(KeyCode::Enter));
        assert_eq!(s.mode, SettingsMode::NewPassword);
        type_str(&mut s, "hunter2");
        assert_eq!(s.handle_key(key(KeyCode::Enter)), SettingsAction::None);
        assert!(matches!(s.mode, SettingsMode::RepeatPassword(_)));
        // The first entry is not left lying in the buffer the popup draws.
        assert!(s.input.is_empty());
        type_str(&mut s, "hunter2");
        assert_eq!(
            s.handle_key(key(KeyCode::Enter)),
            SettingsAction::ChangePassword("hunter2".into())
        );
        assert_eq!(s.mode, SettingsMode::Navigate);
    }

    #[test]
    fn a_mistyped_repeat_changes_nothing_and_says_so() {
        let mut s = state();
        select(&mut s, SettingsRow::ChangePassword);
        s.handle_key(key(KeyCode::Enter));
        type_str(&mut s, "hunter2");
        s.handle_key(key(KeyCode::Enter));
        type_str(&mut s, "hunter3");
        assert_eq!(s.handle_key(key(KeyCode::Enter)), SettingsAction::None);
        assert_eq!(s.mode, SettingsMode::Navigate);
        assert!(s.status.is_some());
    }

    #[test]
    fn an_empty_password_is_not_asked_for_twice() {
        let mut s = state();
        select(&mut s, SettingsRow::ChangePassword);
        s.handle_key(key(KeyCode::Enter));
        assert_eq!(s.handle_key(key(KeyCode::Enter)), SettingsAction::None);
        assert_eq!(s.mode, SettingsMode::Navigate);
    }

    #[test]
    fn escape_during_the_repeat_forgets_the_first_entry() {
        let mut s = state();
        select(&mut s, SettingsRow::ChangePassword);
        s.handle_key(key(KeyCode::Enter));
        type_str(&mut s, "hunter2");
        s.handle_key(key(KeyCode::Enter));
        s.handle_key(key(KeyCode::Esc));
        // Starting over asks for both entries again rather than matching the
        // abandoned one.
        s.handle_key(key(KeyCode::Enter));
        type_str(&mut s, "hunter2");
        assert_eq!(s.handle_key(key(KeyCode::Enter)), SettingsAction::None);
        assert!(matches!(s.mode, SettingsMode::RepeatPassword(_)));
    }

    #[test]
    fn logging_out_is_confirmed_first() {
        let mut s = state();
        select(&mut s, SettingsRow::Logout);
        assert_eq!(s.handle_key(key(KeyCode::Enter)), SettingsAction::None);
        assert_eq!(s.mode, SettingsMode::ConfirmingLogout);
        assert_eq!(
            s.handle_key(key(KeyCode::Char('y'))),
            SettingsAction::Logout
        );
    }

    #[test]
    fn anything_but_yes_cancels_the_logout() {
        let mut s = state();
        select(&mut s, SettingsRow::Logout);
        s.handle_key(key(KeyCode::Enter));
        assert_eq!(s.handle_key(key(KeyCode::Char('n'))), SettingsAction::None);
        assert_eq!(s.mode, SettingsMode::Navigate);
    }

    #[test]
    fn a_daemons_session_offers_no_logout_row() {
        let mut s = SettingsState::new(
            AccountInfo {
                daemon: Some("127.0.0.1:2245".into()),
                ..account()
            },
            "/dl".into(),
            vec!["/music".into()],
        );
        assert_eq!(
            s.rows(),
            vec![
                SettingsRow::ChangePassword,
                SettingsRow::DownloadDir,
                SettingsRow::Share(0),
            ]
        );
        // The row below "change password" is the download folder, not a
        // logout the window cannot perform.
        s.handle_key(key(KeyCode::Down));
        assert_eq!(s.selected_row(), SettingsRow::DownloadDir);
    }
}
