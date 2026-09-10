//! What the server tells us about people: watches, interests, and the
//! two halves of a user snapshot.

use super::*;

#[test]
fn a_fresh_request_discards_the_previous_answer() {
    // Without this, a poll after a second request returns the old
    // snapshot immediately and the caller cannot tell stale from fresh.
    let mut context = ClientContext::new();
    context.apply_user_status("alice".to_string(), 2, false);
    context.apply_user_stats("alice".to_string(), 10, 20, 30);
    assert!(context.user_info("alice").is_some_and(|i| i.is_complete()));

    context.invalidate_user_info("alice");
    assert!(
        context.user_info("alice").is_none(),
        "a new request must not be answerable from the old reply"
    );
}

#[test]
fn each_reply_fills_only_its_own_half() {
    let mut context = ClientContext::new();
    context.apply_user_status("bob".to_string(), 1, true);

    let info = context.user_info("bob").expect("a snapshot");
    assert!(!info.is_complete(), "stats have not arrived");
    assert_eq!(
        info.presence.map(|p| p.status),
        Some(crate::types::UserStatus::Away)
    );
    assert!(info.stats.is_none(), "must not invent statistics");

    context.apply_user_stats("bob".to_string(), 5, 6, 7);
    let info = context.user_info("bob").expect("a snapshot");
    assert!(info.is_complete());
    assert_eq!(info.stats.map(|s| s.shared_files), Some(6));
    assert_eq!(
        info.presence.map(|p| p.privileged),
        Some(true),
        "the earlier half must survive the merge"
    );
}

#[test]
fn a_watch_reply_fills_both_halves_of_the_snapshot() {
    let mut context = ClientContext::new();
    context.add_watched_user("alice");
    context.apply_watched_user(
        "alice".to_string(),
        true,
        Some(2),
        Some(1024),
        Some(20),
        Some(3),
    );

    let info = context.user_info("alice").expect("a snapshot");
    assert!(info.is_complete(), "a watch reply carries status and stats");
    assert_eq!(info.stats.map(|s| s.average_speed), Some(1024));
    assert_eq!(context.watched_users(), vec!["alice".to_string()]);
}

#[test]
fn a_watch_reply_keeps_a_privileged_flag_it_cannot_carry() {
    // WatchUser has no privileged field, so it must not overwrite what
    // GetUserStatus already told us with a fabricated `false`.
    let mut context = ClientContext::new();
    context.apply_user_status("bob".to_string(), 1, true);
    context.apply_watched_user(
        "bob".to_string(),
        true,
        Some(2),
        Some(1),
        Some(2),
        Some(3),
    );

    let info = context.user_info("bob").expect("a snapshot");
    assert_eq!(info.presence.map(|p| p.privileged), Some(true));
}

#[test]
fn watching_an_unknown_user_drops_them_from_the_list() {
    // The server will never push status for a name it does not know, so
    // keeping it in the watch list would show a permanently blank row.
    let mut context = ClientContext::new();
    context.add_watched_user("ghost");
    context.apply_watched_user(
        "ghost".to_string(),
        false,
        None,
        None,
        None,
        None,
    );

    assert!(context.watched_users().is_empty());
    assert!(context.user_info("ghost").is_none());
}

#[test]
fn unwatching_forgets_the_users_snapshot() {
    let mut context = ClientContext::new();
    context.add_watched_user("alice");
    context.apply_user_status("alice".to_string(), 2, false);

    context.remove_watched_user("alice");
    assert!(context.watched_users().is_empty());
    assert!(
        context.user_info("alice").is_none(),
        "a later re-watch must report a fresh answer"
    );
}

#[test]
fn watched_users_are_listed_in_a_stable_order() {
    let mut context = ClientContext::new();
    context.add_watched_user("carol");
    context.add_watched_user("alice");
    context.add_watched_user("bob");
    assert_eq!(context.watched_users(), vec!["alice", "bob", "carol"]);
}

// A reply queued for a searcher the server cannot place waits a minute,
// not forever: every such search would otherwise pin a map entry and a
// protected slot in the peer registry.

#[test]
fn own_and_global_recommendations_are_kept_apart() {
    let mut ctx = ClientContext::new();
    ctx.apply_recommendations(
        false,
        vec![Recommendation {
            item: "jazz".into(),
            rating: 2,
        }],
        Vec::new(),
    );
    ctx.apply_recommendations(
        true,
        vec![Recommendation {
            item: "pop".into(),
            rating: 9,
        }],
        Vec::new(),
    );

    assert_eq!(ctx.recommendations(false).unwrap().0[0].item, "jazz");
    assert_eq!(ctx.recommendations(true).unwrap().0[0].item, "pop");
}

#[test]
fn asking_again_about_a_users_interests_drops_the_previous_answer() {
    let mut ctx = ClientContext::new();
    ctx.apply_user_interests(UserInterests {
        username: "alice".into(),
        likes: vec!["jazz".into()],
        hates: Vec::new(),
    });
    assert!(ctx.user_interests("alice").is_some());

    ctx.invalidate_user_interests("alice");
    assert!(
        ctx.user_interests("alice").is_none(),
        "a stale answer must not be mistaken for the next one"
    );
}

#[test]
fn our_own_interests_are_kept_lowercased_and_deduplicated() {
    // The server matches interests case-insensitively and forgets them when
    // the session ends, so they are held here in one spelling to be sent
    // again next login.
    let mut ctx = ClientContext::new();
    assert_eq!(
        ctx.add_own_interest("  Krautrock ", true).as_deref(),
        Some("krautrock")
    );
    assert_eq!(
        ctx.add_own_interest("   ", true),
        None,
        "an empty interest is not stored, and is not sent either"
    );
    ctx.add_own_interest("KRAUTROCK", true);
    ctx.add_own_interest("Muzak", false);

    let interests = ctx.own_interests();
    assert_eq!(interests.likes, ["krautrock"]);
    assert_eq!(interests.hates, ["muzak"]);

    ctx.remove_own_interest("KrautRock", true);
    assert!(ctx.own_interests().likes.is_empty());
    assert_eq!(
        ctx.own_interests().hates,
        ["muzak"],
        "the other list stands"
    );
}
