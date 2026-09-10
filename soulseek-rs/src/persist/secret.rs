use crate::api::SessionApi;
use color_eyre::Result;

/// Where passwords live. The real implementation is the OS keychain
/// ([`KeyringStore`]); tests use an in-memory fake. Passwords are never
/// written to config.toml.
///
/// `Send + Sync` because the daemon holds one behind an `Arc` and answers
/// requests on a thread per connection.
pub trait SecretStore: Send + Sync {
    fn get(&self, username: &str) -> Result<Option<String>>;
    fn set(&self, username: &str, password: &str) -> Result<()>;
    /// Forget this account's password. Succeeds when there was nothing
    /// stored: the caller wants it gone, and it is.
    fn delete(&self, username: &str) -> Result<()>;
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

    fn delete(&self, username: &str) -> Result<()> {
        match keyring::Entry::new(SERVICE, username)
            .and_then(|entry| entry.delete_credential())
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(color_eyre::eyre::eyre!("keyring: {e}")),
        }
    }
}

/// Change the account's password and keep the local copy in step, so the next
/// start still logs in.
///
/// One rule in one place: whoever owns the login owns the stored copy. A
/// window or a command borrowing a daemon's session owns neither, and the
/// daemon writes its own host's store when it serves the request.
///
/// `Ok` means the request reached the server, not that the server took it —
/// Soulseek answers a password change with nothing at all — and carries the
/// store's complaint when the change went out but the local copy did not.
pub fn change_password(
    session: &dyn SessionApi,
    secrets: &dyn SecretStore,
    password: &str,
) -> soulseek_rs::Result<Option<String>> {
    session.change_password(password)?;
    if session.daemon_endpoint().is_some() {
        return Ok(None);
    }
    Ok(secrets
        .set(&session.username(), password)
        .err()
        .map(|e| e.to_string()))
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

/// An in-memory store for tests, so no test touches the machine's keychain.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryStore {
    secrets: std::sync::Mutex<std::collections::HashMap<String, String>>,
    /// Answer every read as if there were no secret service at all.
    pub fail: bool,
}

#[cfg(test)]
impl SecretStore for MemoryStore {
    fn get(&self, username: &str) -> Result<Option<String>> {
        if self.fail {
            return Err(color_eyre::eyre::eyre!("no secret service"));
        }
        Ok(self
            .secrets
            .lock()
            .expect("not poisoned")
            .get(username)
            .cloned())
    }

    fn set(&self, username: &str, password: &str) -> Result<()> {
        self.secrets
            .lock()
            .expect("not poisoned")
            .insert(username.into(), password.into());
        Ok(())
    }

    fn delete(&self, username: &str) -> Result<()> {
        self.secrets.lock().expect("not poisoned").remove(username);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type FakeStore = MemoryStore;

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
    fn a_deleted_password_is_no_longer_resolved() {
        let store = FakeStore::default();
        store.set("alice", "from-keyring").unwrap();
        store.delete("alice").unwrap();
        assert_eq!(resolve_password(None, Some("alice"), None, &store), None);
        // Deleting again is not an error: the password is gone either way.
        store.delete("alice").unwrap();
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
