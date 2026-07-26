//! Being a peer on the network: `serve`, `whoami`, and `user`.

use super::{Ctx, Session};
use crate::cli::{ServeArgs, UserArgs};
use crate::output::{
    CliError, CliResult, UploadRecord, UserRecord, WhoamiRecord,
};
use soulseek_rs::types::{UploadInfo, UploadStatus, UserInfo};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How often the serve loop samples the upload table.
const POLL: Duration = Duration::from_millis(500);

/// How long `whoami` waits for the server's privilege answer. It is one small
/// message on an already-open connection, so a server that has not replied by
/// now is not going to.
const PRIVILEGE_TIMEOUT: Duration = Duration::from_secs(5);

pub fn whoami(ctx: &Ctx) -> CliResult {
    let session = Session::open(ctx)?;
    let (folders, files) = session.client.shared_counts();
    ctx.out.emit(&WhoamiRecord {
        user: ctx.settings.username.clone(),
        server: ctx.settings.server_address.to_string(),
        listening: ctx.settings.enable_listen,
        listen_port: ctx.settings.listen_port,
        shared_folders: folders,
        shared_files: files,
        download_dir: ctx.download_dir.clone(),
        privilege_seconds: own_privileges(&session.client),
    });
    Ok(())
}

/// Ask the server how much privilege time this account has left.
///
/// A server that does not answer leaves the field null rather than reporting a
/// zero, because "no privileges" and "did not say" are different facts.
fn own_privileges(client: &soulseek_rs::Client) -> Option<u32> {
    if client.check_privileges().is_err() {
        return None;
    }
    let deadline = Instant::now() + PRIVILEGE_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(seconds) = client.own_privilege_seconds() {
            return Some(seconds);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

pub fn user(ctx: &Ctx, args: &UserArgs) -> CliResult {
    let session = Session::open(ctx)?;
    session.client.request_user_info(&args.user).map_err(|e| {
        CliError::connection(format!("cannot ask about {}: {e}", args.user))
    })?;
    ctx.out
        .status(&format!("asking the server about {}…", args.user));

    // Presence and statistics are two replies; wait for both, but report
    // whichever arrived rather than defaulting the other to something that
    // reads like an answer.
    let deadline = Instant::now() + Duration::from_secs(args.timeout);
    let mut latest = None;
    while Instant::now() < deadline {
        latest = session.client.user_info(&args.user);
        if latest.as_ref().is_some_and(UserInfo::is_complete) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let Some(info) = latest else {
        return Err(CliError::timeout(format!(
            "the server said nothing about {} within {}s",
            args.user, args.timeout
        )));
    };
    if !info.is_complete() {
        ctx.out
            .warn("the server answered only partly; unknown fields are null");
    }
    ctx.out.emit(&UserRecord {
        user: info.username,
        status: info.presence.map(|p| p.status.to_string()),
        privileged: info.presence.map(|p| p.privileged),
        average_speed: info.stats.map(|s| s.average_speed),
        shared_files: info.stats.map(|s| s.shared_files),
        shared_folders: info.stats.map(|s| s.shared_folders),
    });
    Ok(())
}

/// Stay online sharing files, and — with `--wishlist` — re-run the standing
/// searches on the interval the server sets while doing so.
pub fn serve(
    ctx: &Ctx,
    args: &ServeArgs,
    mut sweeper: Option<crate::commands::wish::Sweeper>,
) -> CliResult {
    if ctx.settings.shared_directories.is_empty() {
        return Err(CliError::usage(
            "nothing to share: pass --shared-dir, or add one with `shares add`",
        ));
    }
    if !ctx.settings.enable_listen {
        return Err(CliError::usage(
            "serving needs the listener: drop --no-listener",
        ));
    }

    let session = Session::open(ctx)?;
    if let Some(slots) = args.upload_slots {
        session.client.set_upload_slots(usize::from(slots));
        ctx.out.status(&format!("{slots} upload slots"));
    }
    let (folders, files) = session.client.shared_counts();
    ctx.out.status(&format!(
        "serving {files} files in {folders} folders on port {}",
        ctx.settings.listen_port
    ));
    if files == 0 {
        ctx.out
            .warn("the share index is empty — peers will find nothing");
    }

    let span = (!args.follow).then(|| Duration::from_secs(args.duration));
    ctx.out.status(&match span {
        Some(span) => format!("online for {}s…", span.as_secs()),
        None => "online until interrupted…".to_string(),
    });

    let deadline = span.map(|span| Instant::now() + span);
    let mut reported: HashMap<(String, String), UploadRecord> = HashMap::new();
    while deadline.is_none_or(|deadline| Instant::now() < deadline) {
        for upload in session.client.uploads() {
            let record = upload_record(&upload);
            let key = (record.user.clone(), record.path.clone());
            if reported.get(&key) != Some(&record) {
                ctx.out.emit(&record);
                reported.insert(key, record);
            }
        }
        if let Some(sweeper) = sweeper.as_mut() {
            sweeper.tick(ctx, &session.client);
        }
        std::thread::sleep(POLL);
    }
    Ok(())
}

/// Flatten a live upload into the record `serve` reports.
fn upload_record(upload: &UploadInfo) -> UploadRecord {
    let (status, reason) = match &upload.status {
        UploadStatus::Queued(place) => {
            ("queued", Some(format!("place {place}")))
        }
        UploadStatus::InProgress => ("uploading", None),
        UploadStatus::Completed => ("completed", None),
        UploadStatus::Cancelled => ("cancelled", None),
        UploadStatus::Failed(why) => ("failed", Some(why.clone())),
    };
    UploadRecord {
        user: upload.username.clone(),
        path: upload.filename.clone(),
        size: upload.size,
        bytes_sent: upload.bytes_sent,
        speed: upload.speed_bytes_per_sec,
        status: status.to_string(),
        reason,
    }
}

/// Emit `record` only when it differs from the last one reported for that
/// (user, file). Exposed for tests; the serve loop inlines the same rule.
#[cfg(test)]
fn emit_changed(
    reported: &mut HashMap<(String, String), UploadRecord>,
    record: UploadRecord,
    out: &crate::output::Out,
) -> bool {
    let key = (record.user.clone(), record.path.clone());
    if reported.get(&key) == Some(&record) {
        return false;
    }
    out.emit(&record);
    reported.insert(key, record);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::{Out, Record};

    fn upload(status: UploadStatus, bytes_sent: u64) -> UploadInfo {
        UploadInfo {
            username: "bob".to_string(),
            filename: "@@x\\track.mp3".to_string(),
            size: 100,
            bytes_sent,
            status,
            speed_bytes_per_sec: 1.0,
        }
    }

    #[test]
    fn an_upload_maps_to_its_wire_status() {
        assert_eq!(
            upload_record(&upload(UploadStatus::InProgress, 5)).status,
            "uploading"
        );
        assert_eq!(
            upload_record(&upload(UploadStatus::Completed, 100)).status,
            "completed"
        );
        assert_eq!(
            upload_record(&upload(UploadStatus::Cancelled, 5)).status,
            "cancelled"
        );

        let failed =
            upload_record(&upload(UploadStatus::Failed("gone".into()), 5));
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.reason.as_deref(), Some("gone"));
    }

    #[test]
    fn the_upload_line_leads_with_status_and_progress() {
        let record = upload_record(&upload(UploadStatus::InProgress, 42));
        assert_eq!(record.text(), "uploading\tbob\t42/100\t@@x\\track.mp3");
    }

    #[test]
    fn an_unchanged_upload_is_reported_once() {
        let out = Out::new(false, true);
        let mut reported = HashMap::new();
        let record = upload_record(&upload(UploadStatus::InProgress, 42));

        assert!(emit_changed(&mut reported, record.clone(), &out));
        assert!(
            !emit_changed(&mut reported, record, &out),
            "polling must not re-report an upload that has not moved"
        );
    }

    #[test]
    fn progress_and_status_changes_are_both_reported() {
        let out = Out::new(false, true);
        let mut reported = HashMap::new();
        emit_changed(
            &mut reported,
            upload_record(&upload(UploadStatus::InProgress, 10)),
            &out,
        );
        assert!(
            emit_changed(
                &mut reported,
                upload_record(&upload(UploadStatus::InProgress, 60)),
                &out
            ),
            "more bytes sent is news"
        );
        assert!(
            emit_changed(
                &mut reported,
                upload_record(&upload(UploadStatus::Completed, 100)),
                &out
            ),
            "finishing is news"
        );
    }
}
