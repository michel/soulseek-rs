//! Room rosters, ticker boards, and the private rooms the server lets us
//! see.

use super::*;

#[test]
fn the_room_roster_follows_the_joins_and_leaves_the_server_reports() {
    let mut context = ClientContext::new();

    // Joining hands us the current membership in one event.
    context.apply_room_event(RoomEvent::Joined {
        room: "lobby".to_string(),
        users: vec!["bob".to_string(), "alice".to_string()],
    });
    assert_eq!(context.room_members("lobby"), ["alice", "bob"]);

    context.apply_room_event(RoomEvent::UserJoined {
        room: "lobby".to_string(),
        username: "carol".to_string(),
    });
    context.apply_room_event(RoomEvent::UserLeft {
        room: "lobby".to_string(),
        username: "bob".to_string(),
    });
    assert_eq!(context.room_members("lobby"), ["alice", "carol"]);

    // Events for other rooms do not leak into this one.
    context.apply_room_event(RoomEvent::UserJoined {
        room: "elsewhere".to_string(),
        username: "dave".to_string(),
    });
    assert_eq!(context.room_members("lobby"), ["alice", "carol"]);

    // A room we never joined has no roster, and leaving forgets it.
    assert!(context.room_members("unknown").is_empty());
    context.apply_room_event(RoomEvent::Left {
        room: "lobby".to_string(),
    });
    assert!(context.room_members("lobby").is_empty());
}

#[test]
fn a_user_joining_twice_is_listed_once() {
    let mut context = ClientContext::new();
    context.apply_room_event(RoomEvent::Joined {
        room: "lobby".to_string(),
        users: vec!["alice".to_string()],
    });
    context.apply_room_event(RoomEvent::UserJoined {
        room: "lobby".to_string(),
        username: "alice".to_string(),
    });
    assert_eq!(context.room_members("lobby"), ["alice"]);
}

#[test]
fn room_member_stats_are_returned_sorted_and_scoped_to_their_room() {
    use crate::types::{RoomUserStats, UserStatus};
    let stat = |username: &str| RoomUserStats {
        username: username.to_string(),
        status: UserStatus::Online,
        average_speed: 1,
        shared_files: 2,
        shared_folders: 3,
        slots_full: true,
        country: None,
    };

    let mut context = ClientContext::new();
    context.apply_room_member_stats(
        "jazz".to_string(),
        vec![stat("carol"), stat("alice")],
    );

    let names: Vec<String> = context
        .room_member_stats("jazz")
        .into_iter()
        .map(|s| s.username)
        .collect();
    assert_eq!(names, vec!["alice", "carol"]);
    assert!(context.room_member_stats("unjoined").is_empty());
}

#[test]
fn rejoining_replaces_the_previous_member_stats() {
    use crate::types::{RoomUserStats, UserStatus};
    let stat = |username: &str| RoomUserStats {
        username: username.to_string(),
        status: UserStatus::Online,
        average_speed: 1,
        shared_files: 2,
        shared_folders: 3,
        slots_full: false,
        country: None,
    };

    let mut context = ClientContext::new();
    context.apply_room_member_stats("jazz".to_string(), vec![stat("alice")]);
    context.apply_room_member_stats("jazz".to_string(), vec![stat("bob")]);

    let stats = context.room_member_stats("jazz");
    assert_eq!(stats.len(), 1, "a rejoin supersedes the old snapshot");
    assert_eq!(stats[0].username, "bob");
}

#[test]
fn a_new_ticker_replaces_that_users_previous_one() {
    // The server treats a user's ticker as singular: a second one from the
    // same user supersedes the first rather than stacking beside it.
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::Tickers {
        room: "jazz".into(),
        tickers: vec![
            RoomTicker {
                username: "alice".into(),
                ticker: "first".into(),
            },
            RoomTicker {
                username: "bob".into(),
                ticker: "bobs".into(),
            },
        ],
    });
    ctx.apply_room_event(RoomEvent::TickerAdded {
        room: "jazz".into(),
        username: "alice".into(),
        ticker: "second".into(),
    });

    let board = ctx.room_tickers("jazz");
    assert_eq!(board.len(), 2);
    assert_eq!(
        board.iter().find(|t| t.username == "alice").unwrap().ticker,
        "second"
    );
}

#[test]
fn a_removed_ticker_leaves_the_rest_of_the_board() {
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::Tickers {
        room: "jazz".into(),
        tickers: vec![
            RoomTicker {
                username: "alice".into(),
                ticker: "a".into(),
            },
            RoomTicker {
                username: "bob".into(),
                ticker: "b".into(),
            },
        ],
    });
    ctx.apply_room_event(RoomEvent::TickerRemoved {
        room: "jazz".into(),
        username: "alice".into(),
    });

    let board = ctx.room_tickers("jazz");
    assert_eq!(board.len(), 1);
    assert_eq!(board[0].username, "bob");
}

#[test]
fn a_global_message_is_queued_without_touching_room_membership() {
    // The global feed carries messages from rooms we have not joined; they
    // must not invent a roster for those rooms.
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::GlobalMessage {
        room: "elsewhere".into(),
        username: "alice".into(),
        message: "hi".into(),
    });
    assert!(ctx.room_members("elsewhere").is_empty());
    assert_eq!(ctx.take_room_events().len(), 1);
}

#[test]
fn a_private_room_roster_tracks_who_is_added_and_removed() {
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::PrivateMembers {
        room: "club".into(),
        users: vec!["bob".into(), "alice".into(), "bob".into()],
    });
    assert_eq!(
        ctx.private_room_members("club"),
        vec!["alice".to_string(), "bob".to_string()],
        "a roster is sorted and free of repeats"
    );

    ctx.apply_room_event(RoomEvent::PrivateRosterChanged {
        room: "club".into(),
        username: "carol".into(),
        members: true,
        added: true,
    });
    ctx.apply_room_event(RoomEvent::PrivateRosterChanged {
        room: "club".into(),
        username: "alice".into(),
        members: true,
        added: false,
    });
    assert_eq!(
        ctx.private_room_members("club"),
        vec!["bob".to_string(), "carol".to_string()]
    );

    // Operators are a separate roster in the same room.
    ctx.apply_room_event(RoomEvent::PrivateRosterChanged {
        room: "club".into(),
        username: "bob".into(),
        members: false,
        added: true,
    });
    assert_eq!(ctx.private_room_operators("club"), vec!["bob".to_string()]);
    assert_eq!(ctx.private_rooms(), vec!["club".to_string()]);
}

#[test]
fn revoked_membership_drops_the_room_we_can_no_longer_see() {
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::PrivateMembers {
        room: "club".into(),
        users: vec!["alice".into()],
    });
    ctx.apply_room_event(RoomEvent::PrivateOperators {
        room: "club".into(),
        users: vec!["alice".into()],
    });

    ctx.apply_room_event(RoomEvent::OwnStandingChanged {
        room: "club".into(),
        members: true,
        granted: false,
    });
    assert!(ctx.private_rooms().is_empty());
    assert!(ctx.private_room_operators("club").is_empty());
}

#[test]
fn losing_operatorship_keeps_the_roster_we_can_still_see() {
    // Demotion does not blind us: we are still a member, and the room's other
    // operators are still ours to show. The server narrates our own removal
    // from that roster separately (code 144).
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::PrivateMembers {
        room: "club".into(),
        users: vec!["alice".into()],
    });
    ctx.apply_room_event(RoomEvent::PrivateOperators {
        room: "club".into(),
        users: vec!["alice".into(), "bob".into()],
    });

    ctx.apply_room_event(RoomEvent::OwnStandingChanged {
        room: "club".into(),
        members: false,
        granted: false,
    });
    assert_eq!(ctx.private_room_members("club"), vec!["alice".to_string()]);
    assert_eq!(
        ctx.private_room_operators("club"),
        vec!["alice".to_string(), "bob".to_string()],
        "the other operators are still there to show"
    );

    ctx.apply_room_event(RoomEvent::PrivateRosterChanged {
        room: "club".into(),
        username: "alice".into(),
        members: false,
        added: false,
    });
    assert_eq!(
        ctx.private_room_operators("club"),
        vec!["bob".to_string()],
        "only the demotion the server narrates removes us"
    );
}

#[test]
fn leaving_a_room_drops_its_ticker_board() {
    let mut ctx = ClientContext::new();
    ctx.apply_room_event(RoomEvent::Tickers {
        room: "jazz".into(),
        tickers: vec![RoomTicker {
            username: "alice".into(),
            ticker: "hi".into(),
        }],
    });
    ctx.apply_room_event(RoomEvent::Left {
        room: "jazz".into(),
    });
    assert!(
        ctx.room_tickers("jazz").is_empty(),
        "a board for a room we left is stale"
    );
}
