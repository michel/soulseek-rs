//! First-run login/registration screen.
//!
//! Soulseek auto-registers unknown usernames on login, so a single form
//! covers both flows. The form itself ([`LoginForm`]) is a pure state
//! machine so it can be tested without a terminal; the IO loop
//! ([`run_login_flow`]) drives it against a real terminal and client.

use color_eyre::Result;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, poll},
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use soulseek_rs::{Client, ClientSettings};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

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
}

/// What the caller should do after feeding a key to the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginAction {
    None,
    /// Both fields are filled and the user pressed Enter.
    Submit,
    /// User pressed Esc — abort the whole program.
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
        match self.phase {
            LoginPhase::Connecting => return LoginAction::None,
            LoginPhase::Failed(_) => {
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
            KeyCode::Enter => {
                if !self.username.is_empty() && !self.password.is_empty() {
                    self.phase = LoginPhase::Connecting;
                    LoginAction::Submit
                } else {
                    // Move to the first empty field instead of submitting.
                    self.focused = if self.username.is_empty() {
                        LoginField::Username
                    } else {
                        LoginField::Password
                    };
                    LoginAction::None
                }
            }
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
/// cancels with Esc (`None`). When both credentials are already known an
/// attempt starts immediately and the form is only shown on failure.
pub fn run_login_flow(
    terminal: &mut DefaultTerminal,
    make_settings: &dyn Fn(String, String) -> ClientSettings,
    initial_username: Option<String>,
    initial_password: Option<String>,
) -> Result<Option<LoginOutcome>> {
    let mut form = LoginForm::new(initial_username);
    let mut entered_via_form = false;
    let mut attempt: Option<Receiver<Result<Client, String>>> =
        match initial_password {
            Some(password) if !form.username.is_empty() => {
                form.password = password;
                form.phase = LoginPhase::Connecting;
                Some(spawn_attempt(make_settings(
                    form.username.clone(),
                    form.password.clone(),
                )))
            }
            _ => None,
        };

    loop {
        terminal.draw(|frame| render(frame, &form))?;

        if let Some(rx) = &attempt {
            match rx.try_recv() {
                Ok(Ok(client)) => {
                    return Ok(Some(LoginOutcome {
                        client,
                        username: form.username,
                        password: form.password,
                        entered_via_form,
                    }));
                }
                Ok(Err(message)) => {
                    form.phase = LoginPhase::Failed(message);
                    attempt = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    form.phase =
                        LoginPhase::Failed("Connection thread died".into());
                    attempt = None;
                }
            }
        }

        if poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match form.handle_key(key) {
                LoginAction::Cancel => return Ok(None),
                LoginAction::Submit => {
                    entered_via_form = true;
                    attempt = Some(spawn_attempt(make_settings(
                        form.username.clone(),
                        form.password.clone(),
                    )));
                }
                LoginAction::None => {}
            }
        }
    }
}

/// Connect and log in on a background thread so the UI stays responsive.
fn spawn_attempt(settings: ClientSettings) -> Receiver<Result<Client, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut client = Client::with_settings(settings);
        let result = client
            .connect()
            .map_err(|e| format!("Failed to connect: {e}"))
            .and_then(|()| match client.login() {
                Ok(true) => Ok(()),
                Ok(false) => Err(
                    "Login rejected: wrong password, or the username is taken"
                        .to_string(),
                ),
                Err(e) => Err(format!("Login failed: {e}")),
            });
        let _ = tx.send(result.map(|()| client));
    });
    rx
}

fn render(frame: &mut Frame, form: &LoginForm) {
    let area = centered(frame.area(), 52, 12);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Soulseek Login ")
        .title_style(Style::default().add_modifier(Modifier::BOLD));
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
        let marker = if focused { "> " } else { "  " };
        let style = if focused {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{marker}{label:<10}"), style),
            Span::raw(value),
            Span::raw(if focused { "▏" } else { "" }),
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
            "•".repeat(form.password.chars().count()),
            editing && form.focused == LoginField::Password,
        ),
        rows[1],
    );

    let status = match &form.phase {
        LoginPhase::Editing => Paragraph::new(""),
        LoginPhase::Connecting => Paragraph::new(Line::from(Span::styled(
            "Connecting…",
            Style::default().fg(Color::Yellow),
        ))),
        LoginPhase::Failed(message) => Paragraph::new(Line::from(
            Span::styled(message.clone(), Style::default().fg(Color::Red)),
        ))
        .wrap(ratatui::widgets::Wrap { trim: true }),
    };
    frame.render_widget(status, rows[3]);

    frame.render_widget(
        Paragraph::new(
            "New usernames are registered automatically.\n\
             Tab: switch · Enter: log in · Esc: quit",
        )
        .style(Style::default().fg(Color::DarkGray)),
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
    use ratatui::crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_str(form: &mut LoginForm, text: &str) {
        for c in text.chars() {
            form.handle_key(key(KeyCode::Char(c)));
        }
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
    fn key_after_failure_returns_to_editing_and_clears_password() {
        let mut form = LoginForm::new(Some("alice".into()));
        type_str(&mut form, "wrong");
        form.handle_key(key(KeyCode::Enter));
        form.phase = LoginPhase::Failed("INVALIDPASS".into());
        let action = form.handle_key(key(KeyCode::Char('x')));
        assert_eq!(action, LoginAction::None);
        assert_eq!(form.phase, LoginPhase::Editing);
        assert_eq!(form.password, "");
    }

    #[test]
    fn keys_ignored_while_connecting() {
        let mut form = LoginForm::new(Some("alice".into()));
        type_str(&mut form, "pw");
        form.handle_key(key(KeyCode::Enter));
        assert_eq!(form.phase, LoginPhase::Connecting);
        let action = form.handle_key(key(KeyCode::Char('x')));
        assert_eq!(action, LoginAction::None);
        assert_eq!(form.password, "pw");
    }
}
