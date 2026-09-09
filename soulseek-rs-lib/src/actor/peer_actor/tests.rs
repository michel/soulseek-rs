use super::*;
use crate::message::peer::build_shared_file_list;
use crate::peer::{ConnectionType, Peer};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::Receiver;

/// A connected inbound actor over a real loopback socket, plus the far
/// end (kept alive so reads see silence, not EOF).
fn connected_actor() -> (PeerActor, Receiver<ClientOperation>, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_nonblocking(true).unwrap();
    let (far_end, _) = listener.accept().unwrap();

    let (mut actor, rx) = make_actor(Some(stream));
    actor.on_start();
    (actor, rx, far_end)
}

fn make_actor(
    stream: Option<TcpStream>,
) -> (PeerActor, Receiver<ClientOperation>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let peer = Peer::new(
        "bob".to_string(),
        ConnectionType::P,
        "127.0.0.1".to_string(),
        0,
        None,
        0,
        0,
        0,
    );
    let actor = PeerActor::new(peer, stream, None, tx, "me".to_string(), 7);
    (actor, rx)
}

#[test]
fn a_timed_out_connect_parks_the_actor_in_disconnected() {
    let (mut actor, rx) = make_actor(None);
    actor.connection_state = ConnectionState::Connecting {
        since: Instant::now().checked_sub(Duration::from_secs(21)).unwrap(),
    };

    actor.tick();

    assert!(
        matches!(actor.connection_state, ConnectionState::Disconnected),
        "a timed-out connect must leave Connecting"
    );
    match rx.try_recv() {
        Ok(ClientOperation::PeerConnectFailed(7, username, _)) => {
            assert_eq!(username, "bob");
        }
        other => panic!("expected PeerConnectFailed, got {other:?}"),
    }
}

#[test]
fn an_upload_failed_message_reaches_the_client_as_this_peer() {
    let (mut actor, rx, _far_end) = connected_actor();

    actor.handle_message(PeerMessage::UploadFailed(
        String::new(),
        "song.mp3".to_string(),
    ));

    match rx.try_recv() {
        Ok(ClientOperation::UploadFailed(username, filename)) => {
            assert_eq!(username, "bob");
            assert_eq!(filename, "song.mp3");
        }
        other => panic!("expected UploadFailed, got {other:?}"),
    }
}

#[test]
fn tick_reaps_a_peer_idle_past_the_deadline() {
    let (mut actor, rx, _far_end) = connected_actor();

    actor.tick();
    assert!(actor.stream.is_some(), "a fresh connection stays open");

    actor.last_activity = Instant::now()
        .checked_sub(IDLE_DISCONNECT + Duration::from_secs(1))
        .unwrap();
    actor.tick();

    assert!(actor.stream.is_none(), "an idle stream must be closed");
    match rx.try_recv() {
        Ok(ClientOperation::PeerDisconnected(7, username, None)) => {
            assert_eq!(username, "bob");
        }
        other => panic!("expected a clean PeerDisconnected, got {other:?}"),
    }
}

/// A listing well past one socket buffer, so delivery within a couple of
/// ticks proves the actor drains the socket rather than sipping 1 KiB a tick.
fn big_listing() -> Vec<u8> {
    let files = (0..400u64)
        .map(|i| crate::message::peer::SharedFileEntry {
            name: format!("track-{i:03}-{}.flac", "x".repeat(i as usize % 17)),
            size: i,
            attributes: Vec::new(),
        })
        .collect();
    let dir = crate::message::peer::SharedDirectory {
        name: "album".to_string(),
        files,
    };
    let bytes = build_shared_file_list(std::slice::from_ref(&dir)).get_buffer();
    assert!(bytes.len() > 4096, "the listing must span several reads");
    bytes
}

fn self_handle(
    actor: &mut PeerActor,
) -> Receiver<crate::actor::ActorMessage<PeerMessage>> {
    let (tx, rx) = std::sync::mpsc::channel();
    actor.set_self_handle(ActorHandle { sender: tx });
    rx
}

// A connection replaced by a newer one to the same peer may still carry
// a reply already on the wire: it is retired, not cut, and once nothing
// more can come it is the one to stop itself.
#[test]
fn a_retired_actor_delivers_what_is_on_the_wire_then_stops_itself() {
    use std::io::Write;
    let (mut actor, rx, mut far_end) = connected_actor();
    let stop_rx = self_handle(&mut actor);

    actor.handle_message(PeerMessage::Retire);
    far_end.write_all(&big_listing()).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    actor.tick();
    actor.tick();

    match rx.try_recv() {
        Ok(ClientOperation::BrowseResult { username, .. }) => {
            assert_eq!(username, "bob");
        }
        other => {
            panic!("expected the listing to reach the client, got {other:?}")
        }
    }
    assert!(actor.stream.is_some(), "retiring is not yet closing");
    assert!(stop_rx.try_recv().is_err(), "not stopped while listening");

    actor.retire_deadline =
        Some(Instant::now().checked_sub(Duration::from_secs(1)).unwrap());
    actor.tick();
    assert!(actor.stream.is_none(), "past the grace the stream closes");
    match rx.try_recv() {
        Ok(ClientOperation::PeerDisconnected(7, _, None)) => {}
        other => panic!("expected a clean PeerDisconnected, got {other:?}"),
    }

    actor.tick();
    assert!(
        matches!(stop_rx.try_recv(), Ok(crate::actor::ActorMessage::Stop)),
        "a closed retired actor stops itself"
    );
}

// The peer that replaced the connection considers the old one done too: it
// writes its reply and hangs up in the same breath. The reply still counts.
#[test]
fn a_reply_sent_right_before_the_peer_hangs_up_is_still_delivered() {
    use std::io::Write;
    let (mut actor, rx, mut far_end) = connected_actor();

    actor.handle_message(PeerMessage::Retire);
    far_end.write_all(&big_listing()).unwrap();
    drop(far_end);
    std::thread::sleep(Duration::from_millis(50));
    actor.tick();
    actor.tick();

    match rx.try_recv() {
        Ok(ClientOperation::BrowseResult { username, .. }) => {
            assert_eq!(username, "bob");
        }
        other => panic!("expected the listing before the close, got {other:?}"),
    }
    match rx.try_recv() {
        Ok(ClientOperation::PeerDisconnected(7, _, None)) => {}
        other => panic!("expected a clean PeerDisconnected, got {other:?}"),
    }
}

#[test]
fn a_retired_actor_reporting_an_error_reports_a_clean_close() {
    let (mut actor, rx, _far_end) = connected_actor();

    actor.handle_message(PeerMessage::Retire);
    actor.disconnect_with_error(Error::from(io::ErrorKind::ConnectionReset));

    match rx.try_recv() {
        Ok(ClientOperation::PeerDisconnected(7, _, None)) => {}
        other => panic!(
            "a retired connection going away is not an error, got {other:?}"
        ),
    }
}

// Retiring an actor whose dial has not completed: nothing went out, so
// there is nothing to wait for, and it must not linger in the queue that
// only a successful connection drains.
#[test]
fn retiring_a_connecting_actor_closes_it_at_once() {
    let (mut actor, rx) = make_actor(None);
    let stop_rx = self_handle(&mut actor);
    actor.connection_state = ConnectionState::Connecting {
        since: Instant::now(),
    };

    actor.handle_message(PeerMessage::Retire);

    assert!(matches!(
        actor.connection_state,
        ConnectionState::Disconnected
    ));
    match rx.try_recv() {
        Ok(ClientOperation::PeerDisconnected(7, _, None)) => {}
        other => panic!("expected a clean PeerDisconnected, got {other:?}"),
    }
    actor.tick();
    assert!(matches!(
        stop_rx.try_recv(),
        Ok(crate::actor::ActorMessage::Stop)
    ));
}
