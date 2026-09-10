//! The account itself: its password on the server, and the credentials this
//! machine keeps for it.

use super::{Ctx, Session};
use crate::cli::PasswordArgs;
use crate::output::{AccountRecord, CliError, CliResult, Out};
use crate::persist::secret::{KeyringStore, SecretStore};

/// Change the account's password and store the new one, so the next start
/// still logs in.
pub fn password(ctx: &Ctx, args: &PasswordArgs) -> CliResult {
    let new = new_password(args)?;
    let session = Session::open(ctx)?;
    let user = session.client.username();
    let stored = crate::persist::secret::change_password(
        session.client.as_ref(),
        &KeyringStore,
        &new,
    )
    .map_err(|e| {
        CliError::connection(format!("cannot change the password: {e}"))
    })?;
    if let Some(e) = stored {
        ctx.out
            .warn(&format!("password changed, but not stored here: {e}"));
    }
    ctx.out.emit(&AccountRecord {
        user,
        action: "password-changed".to_string(),
    });
    Ok(())
}

/// Forget the stored password. The account is untouched on the server; what
/// goes is this machine's copy of the way in.
pub fn logout(out: &Out, username: Option<&str>) -> CliResult {
    let user = username.ok_or_else(crate::run::missing_username)?;
    KeyringStore.delete(user).map_err(|e| {
        CliError::usage(format!("cannot clear the stored password: {e}"))
    })?;
    out.emit(&AccountRecord {
        user: user.to_string(),
        action: "logged-out".to_string(),
    });
    Ok(())
}

/// Stdin beats the flag, the way `--password-stdin` beats `--password`.
fn new_password(args: &PasswordArgs) -> CliResult<String> {
    if args.new_password_stdin {
        return crate::run::password_from_stdin();
    }
    match args.new_password.as_deref() {
        Some(password) if !password.is_empty() => Ok(password.to_string()),
        _ => Err(CliError::usage(
            "no new password: pass --new-password or --new-password-stdin",
        )),
    }
}
