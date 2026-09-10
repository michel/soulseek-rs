//! The settings popup: the account it names, the folders it applies,
//! and the password and logout it can reach.

use super::*;

/// Attached, both folders in the settings form name paths on the
/// *daemon's* machine: they go over the socket exactly as typed, and
/// nothing is created or validated against this machine's filesystem.
#[test]
fn applying_settings_attached_pushes_both_folders_to_the_daemon() {
    let session = Arc::new(TalkativeSession {
        shared: true,
        ..TalkativeSession::default()
    });
    let scratch = tempfile::tempdir().expect("a temp dir");
    let config_path = scratch.path().join("config.toml");
    let mut tui = MainTui::new(
        session.clone(),
        "/daemon/old".to_string(),
        Duration::from_secs(1),
        None,
        Some(config_path.clone()),
    );

    let daemon_only = scratch.path().join("only-on-the-daemon");
    let daemon_dir = daemon_only.to_str().expect("utf-8").to_string();
    tui.open_settings();
    {
        let settings = tui.state.settings.as_mut().expect("open");
        settings.download_dir.clone_from(&daemon_dir);
        settings.share_dirs = vec!["/not-here-either".to_string()];
    }
    tui.apply_settings();

    assert_eq!(
        *session.download_dir_set.lock().expect("not poisoned"),
        Some(daemon_dir.clone()),
        "the folder is pushed to the daemon"
    );
    assert_eq!(
        *session.shares_set.lock().expect("not poisoned"),
        Some(vec!["/not-here-either".to_string()]),
        "shares go over as typed — the daemon's filesystem judges them"
    );
    assert!(
        !daemon_only.exists(),
        "no directory is created on this machine for a daemon's path"
    );
    assert_eq!(tui.download_dir, daemon_dir);

    let saved = crate::persist::config::FileConfig::load(&config_path)
        .expect("config readable");
    assert_eq!(
        saved.download_dir.as_deref(),
        Some(daemon_dir.as_str()),
        "the change still lands in config.toml for the next restart"
    );
}

/// Locally the same form works on this machine: the folder is created
/// here and share paths that do not exist here are dropped before the
/// session hears about them.
#[test]
fn applying_settings_locally_stays_on_this_machine() {
    let session = Arc::new(TalkativeSession {
        shared: false,
        ..TalkativeSession::default()
    });
    let scratch = tempfile::tempdir().expect("a temp dir");
    let config_path = scratch.path().join("config.toml");
    let share = scratch.path().join("music");
    std::fs::create_dir_all(&share).expect("share dir");
    let mut tui = MainTui::new(
        session.clone(),
        "/tmp".to_string(),
        Duration::from_secs(1),
        None,
        Some(config_path),
    );

    let downloads = scratch.path().join("downloads");
    let downloads_dir = downloads.to_str().expect("utf-8").to_string();
    tui.open_settings();
    {
        let settings = tui.state.settings.as_mut().expect("open");
        settings.download_dir.clone_from(&downloads_dir);
        settings.share_dirs = vec![
            share.to_str().expect("utf-8").to_string(),
            "/definitely-not-a-real-share".to_string(),
        ];
    }
    tui.apply_settings();

    // The seam is told in both modes; a local session accepts and takes
    // the directory per download instead of storing it.
    assert_eq!(
        *session.download_dir_set.lock().expect("not poisoned"),
        Some(downloads_dir.clone())
    );
    assert_eq!(
        *session.shares_set.lock().expect("not poisoned"),
        Some(vec![share.to_str().expect("utf-8").to_string()]),
        "paths missing on this machine are dropped before applying"
    );
    assert!(downloads.is_dir(), "the folder is created here");
    assert_eq!(tui.download_dir, downloads_dir);
}

/// The account block answers "who is this session, and what is it
/// offering" without a trip to another screen.
#[test]
fn the_settings_popup_names_the_account_and_its_shares() {
    let mut tui = with_session(TalkativeSession::default());
    press(&mut tui, KeyCode::Char('o'));

    let screen = screen_sized(&mut tui, 100, 30);
    assert!(screen.contains("tester"), "{screen}");
    assert!(screen.contains("12 files in 3 folders"), "{screen}");
}

/// Typed twice the same way, the new password reaches the session — and
/// attached to a daemon it is the daemon that stores it.
#[test]
fn a_password_typed_twice_reaches_the_session() {
    let session = Arc::new(TalkativeSession {
        shared: true,
        ..TalkativeSession::default()
    });
    let mut tui = attach(session.clone());
    press(&mut tui, KeyCode::Char('o'));
    press(&mut tui, KeyCode::Enter); // the change-password row is first
    for _ in 0..2 {
        for c in "hunter2".chars() {
            press(&mut tui, KeyCode::Char(c));
        }
        press(&mut tui, KeyCode::Enter);
    }

    assert_eq!(
        *session.password_set.lock().expect("not poisoned"),
        Some("hunter2".to_string())
    );
    let settings = tui.state.settings.as_ref().expect("still open");
    assert_eq!(settings.status.as_deref(), Some("Password changed"));
}

/// A mistyped repeat is caught here rather than on the server, where the
/// account would be left with a password nobody knows.
#[test]
fn a_password_typed_two_different_ways_is_not_sent() {
    let session = Arc::new(TalkativeSession {
        shared: true,
        ..TalkativeSession::default()
    });
    let mut tui = attach(session.clone());
    press(&mut tui, KeyCode::Char('o'));
    press(&mut tui, KeyCode::Enter);
    for typed in ["hunter2", "hunter3"] {
        for c in typed.chars() {
            press(&mut tui, KeyCode::Char(c));
        }
        press(&mut tui, KeyCode::Enter);
    }

    assert_eq!(*session.password_set.lock().expect("not poisoned"), None);
}

/// Logging out ends the window, but as a log-out: the caller brings the
/// login screen back rather than returning to the shell.
#[test]
fn logging_out_asks_first_and_then_closes_the_window() {
    let mut tui = with_session(TalkativeSession::default());
    press(&mut tui, KeyCode::Char('o'));
    press(&mut tui, KeyCode::Down); // change password -> log out
    press(&mut tui, KeyCode::Enter);
    assert!(tui.state.exit.is_none(), "the confirm comes first");

    press(&mut tui, KeyCode::Char('y'));
    assert_eq!(tui.state.exit, Some(TuiExit::Logout));
}

/// Attached, the login is the daemon's: there is no logout row to land
/// on, so the row below the password is the download folder.
#[test]
fn an_attached_window_offers_no_logout() {
    let session = Arc::new(TalkativeSession {
        shared: true,
        ..TalkativeSession::default()
    });
    let mut tui = attach(session);
    press(&mut tui, KeyCode::Char('o'));
    press(&mut tui, KeyCode::Down);
    press(&mut tui, KeyCode::Enter);

    let settings = tui.state.settings.as_ref().expect("still open");
    assert_eq!(
        settings.mode,
        crate::models::SettingsMode::EditingDownloadDir
    );
    assert!(tui.state.exit.is_none());
}
