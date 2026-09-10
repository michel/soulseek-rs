//! Rooms and conversations: their logs, their lists, and their filters.

use super::*;

#[test]
fn slash_and_u_filter_a_rooms_log_and_its_members() {
    let mut tui = attach(Arc::new(TalkativeSession::default()));
    in_a_room(&mut tui, 30);
    press(&mut tui, KeyCode::Char('u'));
    for c in "user2".chars() {
        press(&mut tui, KeyCode::Char(c));
    }
    press(&mut tui, KeyCode::Enter);
    press(&mut tui, KeyCode::Char('j'));
    assert_eq!(
        tui.state.rooms.selected_user().as_deref(),
        Some("user21"),
        "j moves within the matching members"
    );
    let screen = screen_of(&mut tui);
    assert!(screen.contains("Users 10/30"), "{screen}");

    press(&mut tui, KeyCode::Char('/'));
    for c in "line 01".chars() {
        press(&mut tui, KeyCode::Char(c));
    }
    let screen = screen_of(&mut tui);
    assert!(screen.contains("line 010"), "{screen}");
    assert!(!screen.contains("line 029"), "{screen}");
    assert!(screen.contains("filter: line 01_"), "{screen}");
    ctrl(&mut tui, 'u');
    assert_eq!(tui.state.rooms.log_filter, "line 01", "^u scrolls");
    press(&mut tui, KeyCode::Enter);
    press(&mut tui, KeyCode::Esc);
    assert!(tui.state.rooms.log_filter.is_empty());
    assert!(tui.state.rooms.user_filter.is_empty());
    assert_eq!(tui.state.rooms.view, RoomsView::Chat, "still in the room");
    press(&mut tui, KeyCode::Esc);
    assert_eq!(tui.state.rooms.view, RoomsView::List, "then the list");
}

#[test]
fn slash_filters_the_open_conversation() {
    let mut tui = attach(Arc::new(TalkativeSession::default()));
    for text in ["hello there", "see you", "hello again"] {
        tui.state.messages.push(crate::models::ChatMessage {
            direction: MessageDirection::Incoming,
            peer: "bob".to_string(),
            text: text.to_string(),
            at: chrono::Local::now(),
        });
    }
    tui.state.chat_peer = Some("bob".to_string());
    tui.state.show_messages = true;
    press(&mut tui, KeyCode::Char('/'));
    for c in "hello".chars() {
        press(&mut tui, KeyCode::Char(c));
    }
    let screen = screen_of(&mut tui);
    assert!(screen.contains("hello again"), "{screen}");
    assert!(!screen.contains("see you"), "{screen}");
    press(&mut tui, KeyCode::Enter);
    press(&mut tui, KeyCode::Esc);
    assert!(tui.state.chat_filter.is_empty() && tui.state.show_messages);
    press(&mut tui, KeyCode::Esc);
    assert!(!tui.state.show_messages);
}

#[test]
fn page_up_scrolls_a_room_log_back_and_end_returns_to_the_newest() {
    let mut tui = with_session(TalkativeSession::default());
    in_a_room(&mut tui, 120);

    let screen = screen_of(&mut tui);
    assert!(screen.contains("line 119"), "tails by default: {screen}");
    assert!(!screen.contains("line 000"), "{screen}");

    press(&mut tui, KeyCode::PageUp);
    let screen = screen_of(&mut tui);
    assert!(!screen.contains("line 119"), "scrolled back: {screen}");

    press(&mut tui, KeyCode::Home);
    let screen = screen_of(&mut tui);
    assert!(screen.contains("line 000"), "the oldest: {screen}");
    press(&mut tui, KeyCode::PageUp);
    let screen = screen_of(&mut tui);
    assert!(screen.contains("line 000"), "clamped at the top: {screen}");

    press(&mut tui, KeyCode::End);
    let screen = screen_of(&mut tui);
    assert!(screen.contains("line 119"), "{screen}");

    // The member list keeps its own keys, and ctrl-x is not x.
    press(&mut tui, KeyCode::Down);
    assert_eq!(tui.state.rooms.user_selected, 1);
    ctrl(&mut tui, 'x');
    assert_eq!(tui.state.rooms.open.len(), 1, "still in the room");
}

#[test]
fn page_keys_move_through_the_room_list() {
    let mut tui = with_session(TalkativeSession::default());
    tui.state.rooms.apply_event(
        soulseek_rs::RoomEvent::List(
            (0..60)
                .map(|i| soulseek_rs::types::RoomInfo {
                    name: format!("room{i:02}"),
                    user_count: 60 - i,
                })
                .collect(),
        ),
        None,
    );
    tui.state.show_rooms = true;
    let _ = screen_of(&mut tui);

    press(&mut tui, KeyCode::End);
    assert_eq!(tui.state.rooms.list_selected, 59);
    press(&mut tui, KeyCode::PageUp);
    assert!(tui.state.rooms.list_selected < 59);
    press(&mut tui, KeyCode::Char('g'));
    assert_eq!(tui.state.rooms.list_selected, 0);
    ctrl(&mut tui, 'f');
    assert!(tui.state.rooms.list_selected > 1);
}

#[test]
fn page_up_scrolls_a_conversation_and_switching_chats_resets_it() {
    let mut tui = tui((0..80)
        .map(|i| ChatMessageDto {
            peer: "bob".into(),
            outgoing: i % 2 == 0,
            text: format!("msg {i:03}"),
            at: i,
        })
        .chain(std::iter::once(ChatMessageDto {
            peer: "carol".into(),
            outgoing: false,
            text: "hello".into(),
            at: 100,
        }))
        .collect());
    tui.state.chat_peer = Some("bob".to_string());
    tui.state.show_messages = true;
    let screen = screen_of(&mut tui);
    assert!(screen.contains("msg 079"), "{screen}");

    press(&mut tui, KeyCode::PageUp);
    let screen = screen_of(&mut tui);
    assert!(!screen.contains("msg 079"), "scrolled back: {screen}");
    press(&mut tui, KeyCode::Char('g'));
    let screen = screen_of(&mut tui);
    assert!(screen.contains("msg 000"), "{screen}");

    press(&mut tui, KeyCode::Tab);
    assert_eq!(tui.state.active_chat_peer(), Some("carol"));
    assert!(
        tui.state.chat_view.following(),
        "a fresh chat starts at its end"
    );
    press(&mut tui, KeyCode::BackTab);
    let screen = screen_of(&mut tui);
    assert!(screen.contains("msg 079"), "{screen}");
}
