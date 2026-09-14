//! First-run login/registration screen.
//!
//! Soulseek auto-registers unknown usernames on login, so a single form
//! covers both flows. The form itself ([`LoginForm`]) is a pure state
//! machine so it can be tested without a terminal; the IO loop
//! ([`run_login_flow`]) drives it against a real terminal and client.

use crate::ui::{
    GLYPH_CURSOR, accent_style, dimmed_style, error_style, get_spinner_char,
    info_style, mask, pane_block, primary_style, title_style, warning_style,
};
use color_eyre::Result;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, poll,
    },
    layout::{Constraint, Flex, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use soulseek_rs::{CancelHandle, Client, ClientSettings, SoulseekRs};
use std::sync::mpsc::{Receiver, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const RESTART_FLOOR: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginField {
    Username,
    Password,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginPhase {
    /// User is typing into the form.
    Editing,
    /// A connect+login attempt is in flight.
    Connecting,
    /// The last attempt failed; the message is shown until the next key.
    Failed(String),
    Rejected(String),
}

/// What the caller should do after feeding a key to the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginAction {
    None,
    /// Both fields are filled and the user pressed Enter or r.
    Submit,
    Stop,
    /// User pressed Esc, q or Ctrl-C — abort the whole program.
    Cancel,
}

pub struct LoginForm {
    pub username: String,
    pub password: String,
    pub focused: LoginField,
    pub phase: LoginPhase,
}

impl LoginForm {
    pub fn new(username: Option<String>) -> Self {
        let username = username.unwrap_or_default();
        let focused = if username.is_empty() {
            LoginField::Username
        } else {
            LoginField::Password
        };
        Self {
            username,
            password: String::new(),
            focused,
            phase: LoginPhase::Editing,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> LoginAction {
        if key.code == KeyCode::Char('c')
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            return LoginAction::Cancel;
        }

        match self.phase {
            LoginPhase::Connecting => {
                return match key.code {
                    KeyCode::Enter | KeyCode::Char('r') => LoginAction::Submit,
                    KeyCode::Esc => {
                        self.phase = LoginPhase::Editing;
                        LoginAction::Stop
                    }
                    KeyCode::Char('q') => LoginAction::Cancel,
                    _ => LoginAction::None,
                };
            }
            LoginPhase::Failed(_) => {
                self.phase = LoginPhase::Editing;
                return match key.code {
                    KeyCode::Enter | KeyCode::Char('r') => self.submit(),
                    KeyCode::Esc => LoginAction::Cancel,
                    _ => LoginAction::None,
                };
            }
            LoginPhase::Rejected(_) => {
                // Any key acknowledges the error; the password was wrong (or
                // the name is taken), so make the user retype it.
                self.phase = LoginPhase::Editing;
                self.password.clear();
                self.focused = LoginField::Password;
                if key.code == KeyCode::Esc {
                    return LoginAction::Cancel;
                }
                return LoginAction::None;
            }
            LoginPhase::Editing => {}
        }

        match key.code {
            KeyCode::Esc => LoginAction::Cancel,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                self.focused = match self.focused {
                    LoginField::Username => LoginField::Password,
                    LoginField::Password => LoginField::Username,
                };
                LoginAction::None
            }
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.focused_field_mut().pop();
                LoginAction::None
            }
            KeyCode::Char(c) => {
                self.focused_field_mut().push(c);
                LoginAction::None
            }
            _ => LoginAction::None,
        }
    }

    fn submit(&mut self) -> LoginAction {
        if !self.username.is_empty() && !self.password.is_empty() {
            self.phase = LoginPhase::Connecting;
            return LoginAction::Submit;
        }
        // Move to the first empty field instead of submitting.
        self.focused = if self.username.is_empty() {
            LoginField::Username
        } else {
            LoginField::Password
        };
        LoginAction::None
    }

    const fn focused_field_mut(&mut self) -> &mut String {
        match self.focused {
            LoginField::Username => &mut self.username,
            LoginField::Password => &mut self.password,
        }
    }
}

/// A successful login: the connected client plus the credentials that worked.
pub struct LoginOutcome {
    pub client: Client,
    pub username: String,
    pub password: String,
    /// True when the password was typed into the form (as opposed to coming
    /// from CLI/env/keychain) — only then do we offer to store it.
    pub entered_via_form: bool,
}

/// Drive the login screen until a login succeeds (`Some`) or the user
/// quits (`None`). When both credentials are already known an
/// attempt starts immediately and the form is only shown on failure.
pub fn run_login_flow(
    terminal: &mut DefaultTerminal,
    make_settings: &dyn Fn(String, String) -> ClientSettings,
    initial_username: Option<String>,
    initial_password: Option<String>,
) -> Result<Option<LoginOutcome>> {
    let start = |form: &LoginForm, previous: Option<Attempt<Client>>| {
        let mut client = Client::with_settings(make_settings(
            form.username.clone(),
            form.password.clone(),
        ));
        let cancel = client.cancel_handle();
        Attempt::spawn(previous, cancel, move || {
            let outcome = client
                .connect()
                .map_err(|e| {
                    LoginPhase::Failed(format!("Failed to connect: {e}"))
                })
                .and_then(|()| verdict(client.login()));
            outcome.map(|()| client)
        })
    };
    let mut form = LoginForm::new(initial_username.clone());
    let mut attempt = match &initial_password {
        Some(password) if !form.username.is_empty() => {
            form.password.clone_from(password);
            form.phase = LoginPhase::Connecting;
            Some(start(&form, None))
        }
        _ => None,
    };
    let mut started = Instant::now();

    loop {
        if let Some(current) = &attempt {
            let connecting = form.phase == LoginPhase::Connecting;
            match current.result.try_recv() {
                Ok(_) | Err(std::sync::mpsc::TryRecvError::Disconnected)
                    if !connecting => {}
                Ok(Ok(client)) => {
                    let entered_via_form = initial_username.as_deref()
                        != Some(form.username.as_str())
                        || initial_password.as_deref()
                            != Some(form.password.as_str());
                    return Ok(Some(LoginOutcome {
                        client,
                        username: form.username,
                        password: form.password,
                        entered_via_form,
                    }));
                }
                Ok(Err(failure)) => form.phase = failure,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    form.phase =
                        LoginPhase::Failed("Connection thread died".into());
                }
            }
        }

        let spinner =
            get_spinner_char(started.elapsed().subsec_millis() as usize / 100);
        terminal.draw(|frame| render(frame, &form, spinner))?;

        if poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            let restarting = form.phase == LoginPhase::Connecting;
            match form.handle_key(key) {
                LoginAction::Cancel => return Ok(None),
                LoginAction::Submit
                    if restarting && started.elapsed() < RESTART_FLOOR => {}
                LoginAction::Submit => {
                    attempt = Some(start(&form, attempt.take()));
                    started = Instant::now();
                }
                LoginAction::Stop => {
                    if let Some(current) = &attempt {
                        current.cancel.cancel();
                    }
                }
                LoginAction::None => {}
            }
        }
    }
}

/// One login attempt, run on a background thread so the UI stays responsive.
struct Attempt<T> {
    cancel: CancelHandle,
    thread: JoinHandle<()>,
    result: Receiver<Result<T, LoginPhase>>,
}

impl<T: Send + 'static> Attempt<T> {
    fn spawn(
        previous: Option<Self>,
        cancel: CancelHandle,
        work: impl FnOnce() -> Result<T, LoginPhase> + Send + 'static,
    ) -> Self {
        if let Some(previous) = &previous {
            previous.cancel.cancel();
        }
        let (tx, result) = channel();
        let own = cancel.clone();
        let thread = std::thread::spawn(move || {
            if let Some(previous) = previous {
                let _ = previous.thread.join();
            }
            if own.is_cancelled() {
                return;
            }
            let outcome = work();
            if !own.is_cancelled() {
                let _ = tx.send(outcome);
            }
        });
        Self {
            cancel,
            thread,
            result,
        }
    }
}

fn verdict(login: soulseek_rs::Result<bool>) -> Result<(), LoginPhase> {
    match login {
        Ok(true) => Ok(()),
        Ok(false) | Err(SoulseekRs::AuthenticationFailed) => {
            Err(LoginPhase::Rejected(
                "Login rejected: wrong password, or the username is taken"
                    .to_string(),
            ))
        }
        Err(e) => Err(LoginPhase::Failed(format!("Login failed: {e}"))),
    }
}

fn render(frame: &mut Frame, form: &LoginForm, spinner: &str) {
    let area = centered(frame.area(), 52, 12);
    frame.render_widget(Clear, area);

    let block = pane_block(true).title(Line::from(vec![
        Span::styled(" soulseek-rs", primary_style()),
        Span::styled(" login ", title_style(true)),
    ]));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1), // username
        Constraint::Length(1), // password
        Constraint::Length(1), // spacer
        Constraint::Length(2), // status / error
        Constraint::Length(1), // spacer
        Constraint::Length(2), // hints
    ])
    .split(inner);

    let field = |label: &str, value: String, focused: bool| {
        let marker = if focused { "› " } else { "  " };
        let style = if focused {
            info_style().add_modifier(Modifier::BOLD)
        } else {
            dimmed_style()
        };
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{marker}{label:<10}"), style),
            Span::styled(value, primary_style()),
            Span::styled(
                if focused { GLYPH_CURSOR } else { "" },
                accent_style(),
            ),
        ]))
    };

    let editing = form.phase == LoginPhase::Editing;
    frame.render_widget(
        field(
            "Username:",
            form.username.clone(),
            editing && form.focused == LoginField::Username,
        ),
        rows[0],
    );
    frame.render_widget(
        field(
            "Password:",
            mask(&form.password),
            editing && form.focused == LoginField::Password,
        ),
        rows[1],
    );

    let status = match &form.phase {
        LoginPhase::Editing => Paragraph::new(""),
        LoginPhase::Connecting => Paragraph::new(Line::from(Span::styled(
            format!("{spinner} Connecting…"),
            warning_style(),
        ))),
        LoginPhase::Failed(message) | LoginPhase::Rejected(message) => {
            Paragraph::new(Line::from(Span::styled(
                message.clone(),
                error_style(),
            )))
            .wrap(ratatui::widgets::Wrap { trim: true })
        }
    };
    frame.render_widget(status, rows[3]);

    let keys = match form.phase {
        LoginPhase::Connecting => {
            "Enter/r: restart · Esc: stop · q/Ctrl-C: quit"
        }
        LoginPhase::Failed(_) => "Enter/r: retry · Esc: quit",
        LoginPhase::Editing | LoginPhase::Rejected(_) => {
            "Tab: switch · Enter: log in · Esc: quit"
        }
    };
    frame.render_widget(
        Paragraph::new(format!(
            "New usernames are registered automatically.\n{keys}"
        ))
        .style(dimmed_style()),
        rows[5],
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    area
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_str(form: &mut LoginForm, text: &str) {
        for c in text.chars() {
            form.handle_key(key(KeyCode::Char(c)));
        }
    }

    fn connecting() -> LoginForm {
        let mut form = LoginForm::new(Some("alice".into()));
        type_str(&mut form, "pw");
        assert_eq!(form.handle_key(key(KeyCode::Enter)), LoginAction::Submit);
        form
    }

    #[test]
    fn prefilled_username_focuses_password() {
        let form = LoginForm::new(Some("alice".into()));
        assert_eq!(form.username, "alice");
        assert_eq!(form.focused, LoginField::Password);
    }

    #[test]
    fn empty_form_focuses_username() {
        let form = LoginForm::new(None);
        assert_eq!(form.focused, LoginField::Username);
    }

    #[test]
    fn typing_fills_the_focused_field_and_tab_switches() {
        let mut form = LoginForm::new(None);
        type_str(&mut form, "bob");
        assert_eq!(form.username, "bob");
        form.handle_key(key(KeyCode::Tab));
        assert_eq!(form.focused, LoginField::Password);
        type_str(&mut form, "hunter2");
        assert_eq!(form.password, "hunter2");
        form.handle_key(key(KeyCode::Tab));
        assert_eq!(form.focused, LoginField::Username);
    }

    #[test]
    fn backspace_deletes_from_focused_field() {
        let mut form = LoginForm::new(None);
        type_str(&mut form, "bob");
        form.handle_key(key(KeyCode::Backspace));
        assert_eq!(form.username, "bo");
    }

    #[test]
    fn enter_on_username_moves_to_password() {
        let mut form = LoginForm::new(None);
        type_str(&mut form, "bob");
        let action = form.handle_key(key(KeyCode::Enter));
        assert_eq!(action, LoginAction::None);
        assert_eq!(form.focused, LoginField::Password);
    }

    #[test]
    fn enter_submits_when_both_fields_filled() {
        let mut form = LoginForm::new(Some("alice".into()));
        type_str(&mut form, "hunter2");
        let action = form.handle_key(key(KeyCode::Enter));
        assert_eq!(action, LoginAction::Submit);
        assert_eq!(form.phase, LoginPhase::Connecting);
    }

    #[test]
    fn enter_with_empty_password_does_not_submit() {
        let mut form = LoginForm::new(Some("alice".into()));
        let action = form.handle_key(key(KeyCode::Enter));
        assert_eq!(action, LoginAction::None);
        assert_eq!(form.phase, LoginPhase::Editing);
    }

    #[test]
    fn escape_cancels() {
        let mut form = LoginForm::new(None);
        assert_eq!(form.handle_key(key(KeyCode::Esc)), LoginAction::Cancel);
    }

    #[test]
    fn ctrl_c_quits_from_every_phase_without_typing() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        for phase in [
            LoginPhase::Editing,
            LoginPhase::Connecting,
            LoginPhase::Failed("unreachable".into()),
            LoginPhase::Rejected("INVALIDPASS".into()),
        ] {
            let mut form = LoginForm::new(None);
            form.phase = phase.clone();
            assert_eq!(
                form.handle_key(ctrl_c),
                LoginAction::Cancel,
                "{phase:?}"
            );
            assert_eq!(form.username, "", "{phase:?}");
        }
    }

    #[test]
    fn typing_is_ignored_while_connecting() {
        let mut form = connecting();
        let action = form.handle_key(key(KeyCode::Char('x')));
        assert_eq!(action, LoginAction::None);
        assert_eq!(form.password, "pw");
        assert_eq!(form.phase, LoginPhase::Connecting);
    }

    #[test]
    fn escape_while_connecting_stops_and_keeps_the_credentials() {
        let mut form = connecting();
        assert_eq!(form.handle_key(key(KeyCode::Esc)), LoginAction::Stop);
        assert_eq!(form.phase, LoginPhase::Editing);
        assert_eq!(form.password, "pw");
        assert_eq!(form.handle_key(key(KeyCode::Enter)), LoginAction::Submit);
    }

    #[test]
    fn enter_or_r_while_connecting_restarts() {
        for code in [KeyCode::Enter, KeyCode::Char('r')] {
            let mut form = connecting();
            assert_eq!(form.handle_key(key(code)), LoginAction::Submit);
            assert_eq!(form.phase, LoginPhase::Connecting);
            assert_eq!(form.password, "pw");
        }
    }

    #[test]
    fn q_while_connecting_quits() {
        let mut form = connecting();
        assert_eq!(
            form.handle_key(key(KeyCode::Char('q'))),
            LoginAction::Cancel
        );
    }

    #[test]
    fn enter_or_r_after_a_failed_connection_retries_with_the_same_password() {
        for code in [KeyCode::Enter, KeyCode::Char('r')] {
            let mut form = connecting();
            form.phase = LoginPhase::Failed("Login failed: timed out".into());
            assert_eq!(form.handle_key(key(code)), LoginAction::Submit);
            assert_eq!(form.phase, LoginPhase::Connecting);
            assert_eq!(form.password, "pw");
        }
    }

    #[test]
    fn other_key_after_a_failed_connection_edits_with_the_password_kept() {
        let mut form = connecting();
        form.phase = LoginPhase::Failed("Login failed: timed out".into());
        assert_eq!(form.handle_key(key(KeyCode::Char('x'))), LoginAction::None);
        assert_eq!(form.phase, LoginPhase::Editing);
        assert_eq!(form.password, "pw");
    }

    #[test]
    fn escape_after_a_failed_connection_quits() {
        let mut form = connecting();
        form.phase = LoginPhase::Failed("Login failed: timed out".into());
        assert_eq!(form.handle_key(key(KeyCode::Esc)), LoginAction::Cancel);
    }

    #[test]
    fn key_after_rejection_returns_to_editing_and_clears_password() {
        let mut form = connecting();
        form.phase = LoginPhase::Rejected("INVALIDPASS".into());
        let action = form.handle_key(key(KeyCode::Char('x')));
        assert_eq!(action, LoginAction::None);
        assert_eq!(form.phase, LoginPhase::Editing);
        assert_eq!(form.password, "");
    }

    #[test]
    fn a_wrong_password_is_rejected_and_an_unanswered_login_failed() {
        assert_eq!(verdict(Ok(true)), Ok(()));
        for rejected in [Ok(false), Err(SoulseekRs::AuthenticationFailed)] {
            assert!(matches!(verdict(rejected), Err(LoginPhase::Rejected(_))));
        }
        for failed in [Err(SoulseekRs::Timeout), Err(SoulseekRs::NotConnected)]
        {
            assert!(matches!(verdict(failed), Err(LoginPhase::Failed(_))));
        }
    }

    struct Held(&'static str, std::sync::mpsc::Sender<String>);

    impl Drop for Held {
        fn drop(&mut self) {
            let _ = self.1.send(format!("released {}", self.0));
        }
    }

    fn cancel_handle() -> CancelHandle {
        Client::new("attempt", "pw").cancel_handle()
    }

    fn next(seen: &Receiver<String>) -> String {
        seen.recv_timeout(Duration::from_secs(5))
            .expect("an attempt event")
    }

    #[test]
    fn a_restart_builds_nothing_until_the_attempt_it_replaces_has_let_go() {
        let (events, seen) = channel();
        let first_events = events.clone();
        let first = Attempt::spawn(None, cancel_handle(), move || {
            let _ = first_events.send("started first".to_string());
            std::thread::sleep(Duration::from_millis(100));
            Ok(Held("first", first_events))
        });
        assert_eq!(next(&seen), "started first");

        let _second = Attempt::spawn(Some(first), cancel_handle(), move || {
            let _ = events.send("started second".to_string());
            Err(LoginPhase::Failed("second".into()))
        });

        assert_eq!(next(&seen), "released first");
        assert_eq!(next(&seen), "started second");
    }

    #[test]
    fn a_finished_attempt_is_let_go_before_its_replacement_starts() {
        let (events, seen) = channel();
        let first_events = events.clone();
        let first = Attempt::spawn(None, cancel_handle(), move || {
            Ok(Held("first", first_events))
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !first.thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(first.thread.is_finished());

        let _second = Attempt::spawn(Some(first), cancel_handle(), move || {
            let _ = events.send("started second".to_string());
            Err(LoginPhase::Failed("second".into()))
        });

        assert_eq!(next(&seen), "released first");
        assert_eq!(next(&seen), "started second");
    }

    #[test]
    fn an_attempt_superseded_before_it_began_never_runs() {
        let (events, seen) = channel();
        let (release, gate) = channel::<()>();
        let first_events = events.clone();
        let first = Attempt::spawn(None, cancel_handle(), move || {
            let _ = first_events.send("started first".to_string());
            let _ = gate.recv();
            Err::<Held, _>(LoginPhase::Failed("first".into()))
        });
        assert_eq!(next(&seen), "started first");

        let second_events = events.clone();
        let second = Attempt::spawn(Some(first), cancel_handle(), move || {
            let _ = second_events.send("started second".to_string());
            Err(LoginPhase::Failed("second".into()))
        });
        let _third = Attempt::spawn(Some(second), cancel_handle(), move || {
            let _ = events.send("started third".to_string());
            Err(LoginPhase::Failed("third".into()))
        });
        release.send(()).expect("release the first attempt");

        assert_eq!(next(&seen), "started third");
    }

    #[test]
    fn a_result_that_lands_after_a_stop_is_let_go_on_the_attempts_thread() {
        let (events, seen) = channel();
        let (release, gate) = channel::<()>();
        let attempt = Attempt::spawn(None, cancel_handle(), move || {
            let _ = events.send("started late".to_string());
            let _ = gate.recv();
            Ok(Held("late", events))
        });
        assert_eq!(next(&seen), "started late");

        attempt.cancel.cancel();
        release.send(()).expect("release the attempt");

        assert_eq!(next(&seen), "released late");
    }
}
