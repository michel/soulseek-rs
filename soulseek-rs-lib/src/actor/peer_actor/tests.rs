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
        Ok(ClientOperation::PeerConnectFailed(7, username)) => {
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

// A connection replaced by a newer one to the same peer may still carry
// a reply already on the wire: it is retired, not cut.
#[test]
fn a_retired_actor_delivers_what_is_on_the_wire_then_closes() {
    use std::io::Write;
    let (mut actor, rx, mut far_end) = connected_actor();

    actor.handle_message(PeerMessage::Retire);
    far_end
        .write_all(&build_shared_file_list(&[]).get_buffer())
        .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    actor.tick();

    assert!(
        matches!(rx.try_recv(), Ok(ClientOperation::BrowseResult { .. })),
        "the listing on the wire still reaches the client"
    );
    assert!(actor.stream.is_some(), "retiring is not yet closing");

    actor.retire_deadline =
        Some(Instant::now().checked_sub(Duration::from_secs(1)).unwrap());
    actor.tick();

    assert!(actor.stream.is_none(), "past the grace the stream closes");
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientOperation::PeerDisconnected(7, _, None))
    ));
}

#[test]
fn a_retired_actor_closed_by_the_peer_reports_a_clean_close() {
    let (mut actor, rx, far_end) = connected_actor();

    actor.handle_message(PeerMessage::Retire);
    drop(far_end);
    std::thread::sleep(Duration::from_millis(50));
    actor.tick();

    assert!(actor.stream.is_none());
    assert!(
        matches!(
            rx.try_recv(),
            Ok(ClientOperation::PeerDisconnected(7, _, None))
        ),
        "the peer dropping a retired connection is not an error"
    );
}
