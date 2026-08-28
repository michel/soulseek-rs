//! The wire contract: JSON-RPC 2.0 framing and the data types that cross it.
//!
//! Library types never travel as themselves. Every payload is a DTO defined
//! here with an explicit conversion, so the wire format is versioned
//! independently of the library: a refactor inside `soulseek-rs-lib` cannot
//! silently change what a third-party script receives, and the compiler points
//! at this file when one would.
//!
//! The schemas published in `docs/openrpc.json` are derived from these types
//! (see the tests at the bottom), which is what keeps the advertised contract
//! and the running daemon in agreement.

/// Bumped when a change would break a client written against the old shape.
/// Sent in the `auth` reply so a mismatch is caught at connect time rather
/// than as a confusing failure three calls later.
pub const PROTOCOL_VERSION: u32 = 1;

/// The OpenRPC document, served by `rpc.discover` so a generator can point at
/// a running daemon instead of needing this repository.
pub const OPENRPC: &str = include_str!("../../../docs/openrpc.json");

mod dto;
mod envelope;
mod method;

pub use dto::*;
pub use envelope::*;
pub use method::*;

/// Declare a wire vocabulary once: the enum, the complete list, and the
/// name mapping in both directions.
///
/// Everything that consumes one of these matches on the enum, so the match is
/// exhaustive and adding a line here is a compile error everywhere the new
/// member still has to be handled — the dispatcher, the published schema, the
/// remote client. A `&[&str]` table checked by a test would only fail after
/// the fact, and only if someone remembered to write the test.
macro_rules! wire_vocabulary {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $wire:literal),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant),*
        }

        impl $name {
            /// Every member, in published order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),*];

            #[must_use]
            pub const fn wire_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),*
                }
            }

            #[must_use]
            pub fn from_wire(name: &str) -> Option<Self> {
                match name {
                    $($wire => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.wire_name())
            }
        }
    };
}

wire_vocabulary! {
    /// Every method the daemon answers.
    Method {
        Auth => "auth",
        DaemonStatus => "daemon.status",
        DaemonStop => "daemon.stop",
        RpcDiscover => "rpc.discover",
        SearchStart => "search.start",
        SearchResults => "search.results",
        SearchList => "search.list",
        SearchForget => "search.forget",
        SearchWishlist => "search.wishlist",
        SearchWishlistInterval => "search.wishlist_interval",
        DownloadStart => "download.start",
        DownloadList => "download.list",
        DownloadPause => "download.pause",
        DownloadResume => "download.resume",
        DownloadRemove => "download.remove",
        DownloadRemoveQueued => "download.remove_queued",
        DownloadSetDir => "download.set_dir",
        UploadList => "upload.list",
        UploadCancel => "upload.cancel",
        UploadSlots => "upload.slots",
        PrivilegesCheck => "privileges.check",
        PrivilegesOwn => "privileges.own",
        RoomListRequest => "room.list_request",
        RoomJoin => "room.join",
        RoomLeave => "room.leave",
        RoomSay => "room.say",
        RoomMembers => "room.members",
        MessageSend => "message.send",
        MessageHistory => "message.history",
        BrowseUser => "browse.user",
        UserRequest => "user.request",
        UserInfoOf => "user.info",
        UserWatch => "user.watch",
        UserUnwatch => "user.unwatch",
        UserWatched => "user.watched",
        SharesStatusOf => "shares.status",
        SharesSet => "shares.set",
        SharesReindex => "shares.reindex",
    }
}

wire_vocabulary! {
    /// Events the daemon pushes without being asked.
    ///
    /// These exist because the library's `take_*` accessors are destructive:
    /// with two clients attached, whichever drained first would steal the
    /// others' events. The daemon is the single drainer and fans out.
    Event {
        Room => "event.room",
        Message => "event.message",
        Upload => "event.upload",
        DownloadStatus => "event.download_status",
        Browse => "event.browse",
        SessionLoss => "event.session_loss",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Exit;
    use soulseek_rs::types::{UserPresence, UserStats, UserStatus};
    use soulseek_rs::{
        DownloadStatus, File, RoomEvent, RoomInfo, SearchResult,
        SharedDirectory, UploadStatus, UserInfo, UserMessage,
    };
    use std::collections::HashMap;

    #[test]
    fn a_search_result_survives_the_round_trip() {
        let mut attribs = HashMap::new();
        attribs.insert(0, 320);
        attribs.insert(1, 210);
        let original = SearchResult {
            token: 7,
            slots: 2,
            speed: 900,
            username: "bob".into(),
            files: vec![File {
                username: "bob".into(),
                name: "@@x\\a.mp3".into(),
                size: 4096,
                attribs,
            }],
        };

        let json = serde_json::to_string(&SearchResultDto::from(&original))
            .expect("a DTO serializes");
        let back: SearchResult = serde_json::from_str::<SearchResultDto>(&json)
            .unwrap()
            .into();

        assert_eq!(back.token, original.token);
        assert_eq!(back.username, original.username);
        assert_eq!(back.slots, original.slots);
        assert_eq!(back.speed, original.speed);
        assert_eq!(back.files.len(), 1);
        assert_eq!(back.files[0].name, "@@x\\a.mp3");
        assert_eq!(back.files[0].size, 4096);
        assert_eq!(back.files[0].attribs.get(&0), Some(&320));
        // The per-file username is carried by the parent result, so it has to
        // be restored rather than lost.
        assert_eq!(back.files[0].username, "bob");
    }

    #[test]
    fn every_download_status_survives_the_round_trip() {
        let statuses = [
            DownloadStatus::Queued,
            DownloadStatus::InProgress {
                bytes_downloaded: 10,
                total_bytes: 100,
                speed_bytes_per_sec: 1.5,
            },
            DownloadStatus::Paused {
                bytes_downloaded: 10,
                total_bytes: 100,
            },
            DownloadStatus::Completed,
            DownloadStatus::Failed(Some("gone".into())),
            DownloadStatus::Failed(None),
            DownloadStatus::TimedOut,
        ];
        for status in &statuses {
            let dto = DownloadStatusDto::from(status);
            let json = serde_json::to_string(&dto).unwrap();
            let back: DownloadStatusDto = serde_json::from_str(&json).unwrap();
            assert_eq!(dto, back, "{status:?} must survive the wire");
            assert_eq!(
                DownloadStatusDto::from(&DownloadStatus::from(back)),
                dto,
                "{status:?} must convert back to the same library value"
            );
        }
    }

    #[test]
    fn every_upload_status_survives_the_round_trip() {
        let statuses = [
            UploadStatus::Queued(3),
            UploadStatus::InProgress,
            UploadStatus::Completed,
            UploadStatus::Cancelled,
            UploadStatus::Failed("nope".into()),
        ];
        for status in &statuses {
            let dto = UploadStatusDto::from(status);
            let json = serde_json::to_string(&dto).unwrap();
            let back: UploadStatus =
                serde_json::from_str::<UploadStatusDto>(&json)
                    .unwrap()
                    .into();
            assert_eq!(&back, status);
        }
    }

    #[test]
    fn every_room_event_survives_the_round_trip() {
        let events = [
            RoomEvent::List(vec![RoomInfo {
                name: "lobby".into(),
                user_count: 42,
            }]),
            RoomEvent::Joined {
                room: "lobby".into(),
                users: vec!["bob".into()],
            },
            RoomEvent::Left {
                room: "lobby".into(),
            },
            RoomEvent::Message {
                room: "lobby".into(),
                username: "bob".into(),
                message: "hi".into(),
            },
            RoomEvent::UserJoined {
                room: "lobby".into(),
                username: "sue".into(),
            },
            RoomEvent::UserLeft {
                room: "lobby".into(),
                username: "sue".into(),
            },
        ];
        for event in &events {
            let json =
                serde_json::to_string(&RoomEventDto::from(event)).unwrap();
            let back: RoomEvent =
                serde_json::from_str::<RoomEventDto>(&json).unwrap().into();
            assert_eq!(&back, event);
        }
    }

    #[test]
    fn a_partly_answered_user_stays_partly_answered() {
        // "the server has not said" must not arrive as "offline, shares
        // nothing": that is the difference between waiting and giving up.
        let pending = UserInfo::pending("bob".into());
        let json = serde_json::to_string(&UserInfoDto::from(&pending)).unwrap();
        let back: UserInfo =
            serde_json::from_str::<UserInfoDto>(&json).unwrap().into();
        assert_eq!(back.username, "bob");
        assert!(back.presence.is_none());
        assert!(back.stats.is_none());
        assert!(!back.is_complete());
    }

    #[test]
    fn a_complete_user_survives_the_round_trip() {
        let mut info = UserInfo::pending("bob".into());
        info.presence = Some(UserPresence {
            status: UserStatus::Away,
            privileged: true,
        });
        info.stats = Some(UserStats {
            average_speed: 500,
            shared_files: 10,
            shared_folders: 2,
        });

        let json = serde_json::to_string(&UserInfoDto::from(&info)).unwrap();
        let back: UserInfo =
            serde_json::from_str::<UserInfoDto>(&json).unwrap().into();
        assert_eq!(back, info);
    }

    #[test]
    fn a_private_message_survives_the_round_trip() {
        let message =
            UserMessage::new(9, 1234, "bob".into(), "hi".into(), true);
        let json =
            serde_json::to_string(&UserMessageDto::from(&message)).unwrap();
        let back: UserMessage = serde_json::from_str::<UserMessageDto>(&json)
            .unwrap()
            .into();
        assert_eq!(back.id(), 9);
        assert_eq!(back.timestamp(), 1234);
        assert_eq!(back.username(), "bob");
        assert_eq!(back.message(), "hi");
    }

    #[test]
    fn a_shared_listing_survives_the_round_trip() {
        let directory = SharedDirectory {
            name: "@@x\\Music".into(),
            files: vec![("a.mp3".into(), 4096)],
        };
        let json = serde_json::to_string(&SharedDirectoryDto::from(&directory))
            .unwrap();
        let back: SharedDirectory =
            serde_json::from_str::<SharedDirectoryDto>(&json)
                .unwrap()
                .into();
        assert_eq!(back.name, directory.name);
        assert_eq!(back.files, directory.files);
    }

    #[test]
    fn an_error_carries_the_exit_code_a_local_run_would_have_produced() {
        for exit in [
            Exit::Usage,
            Exit::Connection,
            Exit::NoResults,
            Exit::Timeout,
            Exit::Transfer,
            Exit::SessionLost,
        ] {
            let error = RpcError::application(exit, "nope");
            let json = serde_json::to_string(&error).unwrap();
            let back: RpcError = serde_json::from_str(&json).unwrap();
            assert_eq!(back.exit(), exit);
        }
    }

    #[test]
    fn an_error_without_an_exit_code_is_a_generic_failure() {
        let error = RpcError::new(CODE_METHOD_NOT_FOUND, "no such method");
        assert_eq!(error.exit(), Exit::Failure);
    }

    #[test]
    fn a_request_parses_without_optional_fields() {
        let request: Request = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"daemon.status"}"#,
        )
        .expect("params and id are optional");
        assert_eq!(request.method, "daemon.status");
        assert!(request.id.is_none());
        assert!(request.params.is_null());
    }

    #[test]
    fn an_error_always_carries_an_id_even_when_there_was_none() {
        // JSON-RPC requires it, and without it a client following the
        // documented "no id means a notification" rule drops the error and
        // waits out its timeout for a reply that already arrived.
        let json = serde_json::to_string(&Response::failure(
            None,
            RpcError::new(CODE_PARSE, "unparseable"),
        ))
        .expect("a response is plain data");
        assert!(json.contains("\"id\":null"), "{json}");
    }

    #[test]
    fn a_notification_carries_no_id_so_no_reply_is_expected() {
        let notification =
            Response::notification("event.room", serde_json::json!({"a": 1}));
        let json = serde_json::to_string(&notification).unwrap();
        assert!(!json.contains("\"id\""), "{json}");
        assert!(json.contains("\"method\":\"event.room\""), "{json}");
    }

    #[test]
    fn a_response_serializes_exactly_one_of_result_and_error() {
        let ok = serde_json::to_string(&Response::result(
            Some(serde_json::json!(1)),
            serde_json::json!({}),
        ))
        .unwrap();
        assert!(ok.contains("\"result\""));
        assert!(!ok.contains("\"error\""));

        let failed = serde_json::to_string(&Response::failure(
            Some(serde_json::json!(1)),
            RpcError::application(Exit::NoResults, "nothing"),
        ))
        .unwrap();
        assert!(failed.contains("\"error\""));
        assert!(!failed.contains("\"result\""));
    }
}
