//! Generates the published OpenRPC contract from the wire types themselves.
//!
//! `docs/openrpc.json` is what third parties generate clients from, and
//! `rpc.discover` serves it straight out of the binary. Hand-maintaining it
//! would drift the moment a DTO gained a field, so it is derived here from the
//! same structs the daemon actually serializes, and [`document_is_current`]
//! fails the build when the checked-in copy no longer matches.
//!
//! Regenerate with `UPDATE_OPENRPC=1 cargo test -p soulseek-rs openrpc`.

use super::proto::{
    Ack, AuthParams, AuthResult, BrowseEvent, DaemonStatus, DownloadStartParams,
    DownloadStatusEvent, Downloads, DownloadStarted, Event, IntervalSeconds,
    MessageParams, Members, Messages, Method, QueryParams, RoomEventDto,
    RoomRef, SayParams, SearchResults, Seconds, SessionLossEvent, SharesStatus,
    SlotsParams, TransferRef, UploadInfoDto, Uploads, UserMessageDto, UserRef,
    UserResult, DirectoriesParams,
};
use schemars::{JsonSchema, SchemaGenerator, generate::SchemaSettings};
use serde_json::{Value, json};

/// Where the generated `$ref`s point. OpenRPC keeps shared schemas under
/// `components/schemas`, the same place OpenAPI does.
const DEFINITIONS_PATH: &str = "#/components/schemas/";

/// A method that takes no parameters.
fn nothing() -> Value {
    json!({ "type": "null" })
}

fn schema_of<T: JsonSchema>(generator: &mut SchemaGenerator) -> Value {
    generator.subschema_for::<T>().to_value()
}

/// The parameter schema for one method.
///
/// Exhaustive on purpose: a new [`Method`] variant does not compile until it is
/// described here, so the published contract cannot fall behind the daemon.
fn params_schema(method: Method, generator: &mut SchemaGenerator) -> Value {
    match method {
        Method::Auth => schema_of::<AuthParams>(generator),
        Method::SearchStart
        | Method::SearchResults
        | Method::SearchWishlist => schema_of::<QueryParams>(generator),
        Method::DownloadStart => schema_of::<DownloadStartParams>(generator),
        Method::DownloadPause
        | Method::DownloadResume
        | Method::DownloadRemove
        | Method::DownloadRemoveQueued
        | Method::UploadCancel => schema_of::<TransferRef>(generator),
        Method::UploadSlots => schema_of::<SlotsParams>(generator),
        Method::RoomJoin | Method::RoomLeave | Method::RoomMembers => {
            schema_of::<RoomRef>(generator)
        }
        Method::RoomSay => schema_of::<SayParams>(generator),
        Method::MessageSend => schema_of::<MessageParams>(generator),
        Method::BrowseUser | Method::UserRequest | Method::UserInfoOf => {
            schema_of::<UserRef>(generator)
        }
        Method::SharesSet => schema_of::<DirectoriesParams>(generator),
        Method::DaemonStatus
        | Method::DaemonStop
        | Method::RpcDiscover
        | Method::SearchWishlistInterval
        | Method::DownloadList
        | Method::UploadList
        | Method::PrivilegesCheck
        | Method::PrivilegesOwn
        | Method::RoomListRequest
        | Method::MessageHistory
        | Method::SharesStatusOf
        | Method::SharesReindex => nothing(),
    }
}

/// The result schema for one method. Exhaustive for the same reason.
fn result_schema(method: Method, generator: &mut SchemaGenerator) -> Value {
    match method {
        Method::Auth => schema_of::<AuthResult>(generator),
        Method::DaemonStatus => schema_of::<DaemonStatus>(generator),
        Method::RpcDiscover => json!({ "type": "object" }),
        Method::SearchResults => schema_of::<SearchResults>(generator),
        Method::SearchWishlistInterval => {
            schema_of::<IntervalSeconds>(generator)
        }
        Method::DownloadStart => schema_of::<DownloadStarted>(generator),
        Method::DownloadList => schema_of::<Downloads>(generator),
        Method::UploadList => schema_of::<Uploads>(generator),
        Method::PrivilegesOwn => schema_of::<Seconds>(generator),
        Method::RoomMembers => schema_of::<Members>(generator),
        Method::MessageHistory => schema_of::<Messages>(generator),
        Method::UserInfoOf => schema_of::<UserResult>(generator),
        Method::SharesStatusOf | Method::SharesSet | Method::SharesReindex => {
            schema_of::<SharesStatus>(generator)
        }
        // Everything else acknowledges and says nothing more.
        Method::DaemonStop
        | Method::SearchStart
        | Method::SearchWishlist
        | Method::DownloadPause
        | Method::DownloadResume
        | Method::DownloadRemove
        | Method::DownloadRemoveQueued
        | Method::UploadCancel
        | Method::UploadSlots
        | Method::PrivilegesCheck
        | Method::RoomListRequest
        | Method::RoomJoin
        | Method::RoomLeave
        | Method::RoomSay
        | Method::MessageSend
        | Method::BrowseUser
        | Method::UserRequest => schema_of::<Ack>(generator),
    }
}

/// What a method does, for the generated documentation.
fn summary(method: Method) -> &'static str {
    match method {
        Method::Auth => "Authenticate. Must be the first request on a connection.",
        Method::DaemonStatus => "Who the daemon is logged in as and what its session is doing.",
        Method::DaemonStop => "Ask the daemon to shut down.",
        Method::RpcDiscover => "This document.",
        Method::SearchStart => "Put a search on the wire and return at once; results accumulate.",
        Method::SearchResults => "Everything that has answered a search so far.",
        Method::SearchWishlist => "Send a query as a wishlist search (server code 103).",
        Method::SearchWishlistInterval => "How long the server wants between wishlist searches.",
        Method::DownloadStart => "Queue a transfer. Progress arrives as event.download_status.",
        Method::DownloadList => "Every transfer the daemon knows about.",
        Method::DownloadPause => "Pause a transfer in progress.",
        Method::DownloadResume => "Resume a paused transfer.",
        Method::DownloadRemove => "Forget a transfer entirely.",
        Method::DownloadRemoveQueued => "Drop a transfer that has not started.",
        Method::UploadList => "Uploads being served, active first.",
        Method::UploadCancel => "Stop serving an upload in progress.",
        Method::UploadSlots => "How many uploads may run at once.",
        Method::PrivilegesCheck => "Ask the server for this account's privilege time.",
        Method::PrivilegesOwn => "Privilege seconds left, or null if the server has not answered.",
        Method::RoomListRequest => "Ask the server for the public room list; it arrives as event.room.",
        Method::RoomJoin => "Join a chat room.",
        Method::RoomLeave => "Leave a chat room.",
        Method::RoomSay => "Post a message to a room.",
        Method::RoomMembers => "Who is currently in a room.",
        Method::MessageSend => "Send a private message.",
        Method::MessageHistory => "Private messages the daemon collected, including while nobody was attached.",
        Method::BrowseUser => "Ask a peer for its share listing; it arrives as event.browse.",
        Method::UserRequest => "Ask the server about a user; poll user.info for the answer.",
        Method::UserInfoOf => "What the server has said about a user so far.",
        Method::SharesStatusOf => "What the share index currently holds.",
        Method::SharesSet => "Replace the shared folders and re-index.",
        Method::SharesReindex => "Re-scan the configured folders.",
    }
}

/// The payload schema for one pushed event. Exhaustive, like the methods.
fn event_schema(event: Event, generator: &mut SchemaGenerator) -> Value {
    match event {
        Event::Room => schema_of::<RoomEventDto>(generator),
        Event::Message => schema_of::<UserMessageDto>(generator),
        Event::Upload => schema_of::<UploadInfoDto>(generator),
        Event::DownloadStatus => schema_of::<DownloadStatusEvent>(generator),
        Event::Browse => schema_of::<BrowseEvent>(generator),
        Event::SessionLoss => schema_of::<SessionLossEvent>(generator),
    }
}

fn event_summary(event: Event) -> &'static str {
    match event {
        Event::Room => "Something happened in a room this session has joined.",
        Event::Message => "A private message arrived.",
        Event::Upload => "An upload changed state.",
        Event::DownloadStatus => "A transfer moved.",
        Event::Browse => "A peer answered a browse request.",
        Event::SessionLoss => "The server session ended; nothing seen after this means anything.",
    }
}

/// Build the whole document.
fn document() -> Value {
    let settings = SchemaSettings::draft2020_12().with(|settings| {
        settings.definitions_path = DEFINITIONS_PATH.into();
    });
    let mut generator = SchemaGenerator::new(settings);

    let methods: Vec<Value> = Method::ALL
        .iter()
        .map(|&method| {
            let params = params_schema(method, &mut generator);
            let result = result_schema(method, &mut generator);
            json!({
                "name": method.wire_name(),
                "summary": summary(method),
                "paramStructure": "by-name",
                "params": if params == json!({ "type": "null" }) {
                    json!([])
                } else {
                    json!([{
                        "name": "params",
                        "required": true,
                        "schema": params,
                    }])
                },
                "result": {
                    "name": "result",
                    "schema": result,
                },
            })
        })
        .collect();

    // OpenRPC has no first-class notion of a server-pushed notification, so
    // the events are published as methods flagged with an extension field.
    // A generator still emits their payload types; the delivery semantics are
    // prose in docs/daemon-protocol.md.
    let events: Vec<Value> = Event::ALL
        .iter()
        .map(|&event| {
            json!({
                "name": event.wire_name(),
                "summary": event_summary(event),
                "x-notification": true,
                "paramStructure": "by-name",
                "params": [{
                    "name": "params",
                    "required": true,
                    "schema": event_schema(event, &mut generator),
                }],
                "result": { "name": "result", "schema": { "type": "null" } },
            })
        })
        .collect();

    let definitions: serde_json::Map<String, Value> =
        generator.take_definitions(true).into_iter().collect();

    json!({
        "openrpc": "1.3.2",
        "info": {
            "title": "soulseek-rs daemon",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Control a running soulseek-rs daemon. Newline-delimited \
JSON-RPC 2.0 over a Unix socket (local, authenticated by socket permissions) or \
TCP (remote, authenticated by the token in <state-dir>/daemon.token). `auth` must \
be the first request on a connection. Errors carry the CLI exit status in \
`error.data.exit`. Entries marked `x-notification` are pushed by the daemon and \
are never requested.",
            "license": { "name": "MIT" },
        },
        "methods": methods.into_iter().chain(events).collect::<Vec<_>>(),
        "components": { "schemas": definitions },
    })
}

/// Where the published document lives, relative to this source file.
fn document_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("docs")
        .join("openrpc.json")
}

#[test]
fn document_is_current() {
    let generated = format!(
        "{}\n",
        serde_json::to_string_pretty(&document())
            .expect("the document is plain data")
    );

    let path = document_path();
    if std::env::var_os("UPDATE_OPENRPC").is_some() {
        std::fs::write(&path, &generated).expect("docs/ is writable");
        return;
    }

    let published = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        published, generated,
        "docs/openrpc.json is out of date with the wire types — regenerate it \
         with `UPDATE_OPENRPC=1 cargo test -p soulseek-rs openrpc`"
    );
}

#[test]
fn every_method_and_event_is_published() {
    let document = document();
    let published: Vec<&str> = document["methods"]
        .as_array()
        .expect("methods is an array")
        .iter()
        .map(|method| method["name"].as_str().expect("a name is a string"))
        .collect();

    for method in Method::ALL {
        assert!(
            published.contains(&method.wire_name()),
            "{method} is missing from the published contract"
        );
    }
    for event in Event::ALL {
        assert!(
            published.contains(&event.wire_name()),
            "{event} is missing from the published contract"
        );
    }
    assert_eq!(published.len(), Method::ALL.len() + Event::ALL.len());
}

#[test]
fn a_wire_name_round_trips_through_its_enum() {
    for method in Method::ALL {
        assert_eq!(Method::from_wire(method.wire_name()), Some(*method));
    }
    for event in Event::ALL {
        assert_eq!(Event::from_wire(event.wire_name()), Some(*event));
    }
    assert_eq!(Method::from_wire("search.nope"), None);
}
