use color_eyre::Result;

/// Where passwords live. The real implementation is the OS keychain
/// ([`KeyringStore`]); tests use an in-memory fake. Passwords are never
/// written to config.toml.
pub trait SecretStore {
    fn get(&self, username: &str) -> Result<Option<String>>;
    fn set(&self, username: &str, password: &str) -> Result<()>;
}

/// OS keychain (macOS Keychain / Windows Credential Manager / Linux Secret
/// Service) under service name `soulseek-rs`, account = Soulseek username.
pub struct KeyringStore;

const SERVICE: &str = "soulseek-rs";

impl SecretStore for KeyringStore {
    fn get(&self, username: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new(SERVICE, username)
            .map_err(|e| color_eyre::eyre::eyre!("keyring: {e}"))?;
        match entry.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(color_eyre::eyre::eyre!("keyring: {e}")),
        }
    }

    fn set(&self, username: &str, password: &str) -> Result<()> {
        keyring::Entry::new(SERVICE, username)
            .and_then(|entry| entry.set_password(password))
            .map_err(|e| color_eyre::eyre::eyre!("keyring: {e}"))
    }
}

/// Resolve the password with precedence: CLI/env > keychain > `password_cmd`.
///
/// `cli_password` already has env merged in by clap. Returns `None` when no
/// source has one — the caller shows the login screen.
///
/// Keychain errors are downgraded to "not found": on headless boxes without
/// a secret service the fallbacks must still work.
pub fn resolve_password(
    cli_password: Option<&str>,
    username: Option<&str>,
    password_cmd: Option<&str>,
    store: &dyn SecretStore,
) -> Option<String> {
    if let Some(password) = cli_password {
        return Some(password.to_string());
    }
    if let Some(username) = username
        && let Ok(Some(password)) = store.get(username)
    {
        return Some(password);
    }
    password_cmd.and_then(run_password_cmd)
}

/// Run `password_cmd` through the platform shell and return trimmed stdout.
fn run_password_cmd(cmd: &str) -> Option<String> {
    let output = if cfg!(windows) {
        std::process::Command::new("cmd").args(["/C", cmd]).output()
    } else {
        std::process::Command::new("sh").args(["-c", cmd]).output()
    }
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let password = String::from_utf8(output.stdout).ok()?;
    let password = password.trim_end_matches(['\r', '\n']);
    (!password.is_empty()).then(|| password.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeStore {
        secrets: RefCell<HashMap<String, String>>,
        fail: bool,
    }

    impl SecretStore for FakeStore {
        fn get(&self, username: &str) -> Result<Option<String>> {
            if self.fail {
                return Err(color_eyre::eyre::eyre!("no secret service"));
            }
            Ok(self.secrets.borrow().get(username).cloned())
        }

        fn set(&self, username: &str, password: &str) -> Result<()> {
            self.secrets
                .borrow_mut()
                .insert(username.into(), password.into());
            Ok(())
        }
    }

    #[test]
    fn cli_password_wins_over_everything() {
        let store = FakeStore::default();
        store.set("alice", "from-keyring").unwrap();
        let got = resolve_password(
            Some("from-cli"),
            Some("alice"),
            Some("echo from-cmd"),
            &store,
        );
        assert_eq!(got.as_deref(), Some("from-cli"));
    }

    #[test]
    fn keyring_used_when_no_cli_password() {
        let store = FakeStore::default();
        store.set("alice", "from-keyring").unwrap();
        let got = resolve_password(None, Some("alice"), None, &store);
        assert_eq!(got.as_deref(), Some("from-keyring"));
    }

    #[cfg(unix)]
    #[test]
    fn password_cmd_used_when_keyring_has_nothing() {
        let store = FakeStore::default();
        let got = resolve_password(
            None,
            Some("alice"),
            Some("echo from-cmd"),
            &store,
        );
        assert_eq!(got.as_deref(), Some("from-cmd"));
    }

    #[cfg(unix)]
    #[test]
    fn keyring_failure_falls_through_to_password_cmd() {
        let store = FakeStore {
            fail: true,
            ..FakeStore::default()
        };
        let got = resolve_password(
            None,
            Some("alice"),
            Some("echo from-cmd"),
            &store,
        );
        assert_eq!(got.as_deref(), Some("from-cmd"));
    }

    #[test]
    fn none_when_no_source_has_a_password() {
        let store = FakeStore::default();
        assert_eq!(resolve_password(None, Some("alice"), None, &store), None);
    }

    #[test]
    fn no_username_means_no_keyring_lookup() {
        let store = FakeStore {
            fail: true,
            ..FakeStore::default()
        };
        assert_eq!(resolve_password(None, None, None, &store), None);
    }

    #[cfg(unix)]
    #[test]
    fn failing_password_cmd_yields_none() {
        let store = FakeStore::default();
        assert_eq!(
            resolve_password(None, Some("alice"), Some("false"), &store),
            None
        );
    }
}
