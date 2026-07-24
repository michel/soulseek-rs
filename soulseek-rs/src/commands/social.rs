//! Talking to people and looking at what they share: `browse`, `room`, and
//! `message`.

use super::{Ctx, Session, confirm_flush};
use crate::cli::BrowseArgs;
use crate::output::{
    BrowseRecord, CliError, CliResult, Out, PrivateMessageRecord,
    RoomEventRecord, RoomMemberRecord, RoomRecord,
};
use soulseek_rs::types::RoomEvent;
use soulseek_rs::{Client, SharedDirectory};
use std::time::{Duration, Instant};

/// How long to wait for the server to acknowledge something we sent.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(10);
/// Polling cadence for the request/poll parts of the client API.
const POLL: Duration = Duration::from_millis(100);

/// Join a listing's directory and file name into the remote path that
/// `download` expects — the protocol reports the two separately.
#[must_use]
pub fn full_path(directory: &str, basename: &str) -> String {
    let directory = directory.trim_end_matches('\\');
    if directory.is_empty() {
        basename.to_string()
    } else {
        format!("{directory}\\{basename}")
    }
}

/// Ask a peer for its share listing and wait for the answer.
pub fn fetch_listing(
    client: &Client,
    user: &str,
    timeout: Duration,
) -> CliResult<Vec<SharedDirectory>> {
    client.browse_user(user).map_err(|e| {
        CliError::connection(format!("cannot ask {user} for a listing: {e}"))
    })?;

    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(directories) = client.take_browse_result(user) {
            return Ok(directories);
        }
        std::thread::sleep(POLL);
    }
    Err(CliError::timeout(format!(
        "{user} did not send a file list within {}s",
        timeout.as_secs()
    )))
}

pub fn browse(ctx: &Ctx, args: &BrowseArgs) -> CliResult {
    let session = Session::open(ctx)?;
    ctx.out
        .status(&format!("requesting {}'s shared files…", args.user));

    let directories = fetch_listing(
        &session.client,
        &args.user,
        Duration::from_secs(args.timeout),
    )?;

    let mut files = 0usize;
    for directory in &directories {
        for (basename, size) in &directory.files {
            files += 1;
            ctx.out.emit(&BrowseRecord {
                user: args.user.clone(),
                directory: directory.name.clone(),
                path: full_path(&directory.name, basename),
                size: *size,
            });
        }
    }

    if files == 0 {
        return Err(CliError::no_results(format!(
            "{} shares no files",
            args.user
        )));
    }
    Ok(())
}

pub fn room_list(ctx: &Ctx, timeout: Duration) -> CliResult {
    let session = Session::open(ctx)?;
    ctx.out.status("fetching the room list…");

    let Some(mut rooms) =
        super::await_room_list(&session.client, timeout, POLL)
    else {
        return Err(CliError::timeout(format!(
            "the server sent no room list within {}s",
            timeout.as_secs()
        )));
    };
    if rooms.is_empty() {
        return Err(CliError::no_results("the server lists no public rooms"));
    }

    rooms.sort_by(|a, b| {
        b.user_count
            .cmp(&a.user_count)
            .then_with(|| a.name.cmp(&b.name))
    });
    for room in rooms {
        ctx.out.emit(&RoomRecord {
            room: room.name,
            users: room.user_count,
        });
    }
    Ok(())
}

pub fn room_say(ctx: &Ctx, room: &str, message: &str) -> CliResult {
    let session = Session::open(ctx)?;
    session.client.join_room(room).map_err(|e| {
        CliError::connection(format!("cannot join {room}: {e}"))
    })?;
    session.client.say_in_room(room, message).map_err(|e| {
        CliError::connection(format!("cannot post to {room}: {e}"))
    })?;

    if confirm_flush(&session.client, FLUSH_TIMEOUT) {
        ctx.out.status(&format!("posted to {room}"));
        return Ok(());
    }
    Err(CliError::timeout(format!(
        "the server did not acknowledge the message to {room}"
    )))
}

/// List a room's membership. Joining is what makes the server send it, so we
/// join, wait for the roster, and leave again.
pub fn room_users(ctx: &Ctx, room: &str, timeout: Duration) -> CliResult {
    let session = Session::open(ctx)?;
    session.client.join_room(room).map_err(|e| {
        CliError::connection(format!("cannot join {room}: {e}"))
    })?;
    ctx.out
        .status(&format!("waiting for {room}'s member list…"));

    let deadline = Instant::now() + timeout;
    let mut members = Vec::new();
    while Instant::now() < deadline && members.is_empty() {
        members = session.client.room_members(room);
        if members.is_empty() {
            std::thread::sleep(POLL);
        }
    }
    let _ = session.client.leave_room(room);

    if members.is_empty() {
        return Err(CliError::timeout(format!(
            "{room} sent no member list within {}s",
            timeout.as_secs()
        )));
    }
    for user in members {
        ctx.out.emit(&RoomMemberRecord {
            room: room.to_string(),
            user,
        });
    }
    Ok(())
}

pub fn room_listen(ctx: &Ctx, room: &str, span: Option<Duration>) -> CliResult {
    let session = Session::open(ctx)?;
    session.client.join_room(room).map_err(|e| {
        CliError::connection(format!("cannot join {room}: {e}"))
    })?;

    stream(&ctx.out, &format!("listening to {room}"), span, || {
        for event in session.client.take_room_events() {
            if let Some(record) = room_event_record(room, &event) {
                ctx.out.emit(&record);
            }
        }
    });
    let _ = session.client.leave_room(room);
    Ok(())
}

/// Drain `next` until `span` elapses, or forever when it is `None`
/// (`--follow`, ended by the operator).
fn stream(
    out: &Out,
    what: &str,
    span: Option<Duration>,
    mut next: impl FnMut(),
) {
    out.status(&match span {
        Some(span) => format!("{what} for {}s…", span.as_secs()),
        None => format!("{what} until interrupted…"),
    });
    let deadline = span.map(|span| Instant::now() + span);
    while deadline.is_none_or(|deadline| Instant::now() < deadline) {
        next();
        std::thread::sleep(POLL);
    }
}

/// Turn a room event into a record, ignoring events for other rooms and the
/// bookkeeping ones a listener has no use for.
fn room_event_record(room: &str, event: &RoomEvent) -> Option<RoomEventRecord> {
    match event {
        RoomEvent::Message {
            room: name,
            username,
            message,
        } if name == room => Some(RoomEventRecord {
            kind: "message",
            room: name.clone(),
            user: username.clone(),
            message: message.clone(),
        }),
        RoomEvent::UserJoined {
            room: name,
            username,
        } if name == room => Some(RoomEventRecord {
            kind: "join",
            room: name.clone(),
            user: username.clone(),
            message: String::new(),
        }),
        RoomEvent::UserLeft {
            room: name,
            username,
        } if name == room => Some(RoomEventRecord {
            kind: "leave",
            room: name.clone(),
            user: username.clone(),
            message: String::new(),
        }),
        _ => None,
    }
}

pub fn message_send(ctx: &Ctx, user: &str, message: &str) -> CliResult {
    let session = Session::open(ctx)?;
    session
        .client
        .send_private_message(user, message)
        .map_err(|e| {
            CliError::connection(format!("cannot send to {user}: {e}"))
        })?;

    if confirm_flush(&session.client, FLUSH_TIMEOUT) {
        ctx.out.status(&format!("sent to {user}"));
        return Ok(());
    }
    Err(CliError::timeout(format!(
        "the server did not acknowledge the message to {user}"
    )))
}

pub fn message_read(ctx: &Ctx, span: Option<Duration>) -> CliResult {
    let session = Session::open(ctx)?;

    stream(&ctx.out, "reading private messages", span, || {
        for message in session.client.take_private_messages() {
            ctx.out.emit(&PrivateMessageRecord {
                timestamp: message.timestamp(),
                user: message.username().to_string(),
                message: message.message().to_string(),
            });
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_path_joins_a_directory_and_basename_with_a_backslash() {
        assert_eq!(full_path("@@abc\\Music", "a.mp3"), "@@abc\\Music\\a.mp3");
    }

    #[test]
    fn full_path_does_not_double_a_trailing_separator() {
        assert_eq!(full_path("@@abc\\Music\\", "a.mp3"), "@@abc\\Music\\a.mp3");
    }

    #[test]
    fn full_path_of_a_rootless_listing_is_the_basename() {
        assert_eq!(full_path("", "a.mp3"), "a.mp3");
    }

    #[test]
    fn only_events_for_the_listened_room_become_records() {
        let mine = RoomEvent::Message {
            room: "lobby".into(),
            username: "bob".into(),
            message: "hi".into(),
        };
        let elsewhere = RoomEvent::Message {
            room: "other".into(),
            username: "bob".into(),
            message: "hi".into(),
        };
        assert!(room_event_record("lobby", &mine).is_some());
        assert!(room_event_record("lobby", &elsewhere).is_none());
    }

    #[test]
    fn joins_and_leaves_are_labelled() {
        let joined = RoomEvent::UserJoined {
            room: "lobby".into(),
            username: "sue".into(),
        };
        let left = RoomEvent::UserLeft {
            room: "lobby".into(),
            username: "sue".into(),
        };
        assert_eq!(room_event_record("lobby", &joined).unwrap().kind, "join");
        assert_eq!(room_event_record("lobby", &left).unwrap().kind, "leave");
    }

    #[test]
    fn our_own_join_confirmation_is_not_a_record() {
        let joined = RoomEvent::Joined {
            room: "lobby".into(),
            users: vec!["bob".into()],
        };
        assert!(room_event_record("lobby", &joined).is_none());
    }
}
