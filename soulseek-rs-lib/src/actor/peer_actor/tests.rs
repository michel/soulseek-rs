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

    actor.handle_message(PeerMessage::UploadFailed {
        filename: "song.mp3".to_string(),
        reason: Some("Too many files".to_string()),
    });

    match rx.try_recv() {
        Ok(ClientOperation::UploadFailed {
            username,
            filename,
            reason,
        }) => {
            assert_eq!(username, "bob");
            assert_eq!(filename, "song.mp3");
            assert_eq!(reason.as_deref(), Some("Too many files"));
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

// Issue #115: the declared length was trusted, so a peer announcing a huge
// message had everything it then sent buffered for as long as it kept
// sending.
#[test]
fn a_peer_declaring_an_enormous_message_is_disconnected() {
    use std::io::Write;

    let (mut actor, rx, mut far_end) = connected_actor();
    far_end.write_all(&u32::MAX.to_le_bytes()).unwrap();
    far_end.write_all(&[0u8; 4096]).unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        actor.tick();
        if let Ok(ClientOperation::PeerDisconnected(_, username, error)) =
            rx.try_recv()
        {
            assert_eq!(username, "bob");
            assert!(error.is_some(), "this is not a clean close");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the actor kept waiting for four gigabytes"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(actor.reader.buffer_len(), 0, "and holds none of it");
}

/// The token of the next upload offer (`TransferRequest`, code 40) the far
/// end receives, skipping whatever else arrives first.
fn next_offer_token(far_end: &mut TcpStream) -> u32 {
    use std::io::Read;
    far_end
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    loop {
        let mut length = [0u8; 4];
        far_end.read_exact(&mut length).expect("an offer arrives");
        let mut body = vec![0u8; u32::from_le_bytes(length) as usize];
        far_end.read_exact(&mut body).unwrap();
        let word = |at: usize| {
            u32::from_le_bytes(body[at..at + 4].try_into().unwrap())
        };
        if word(0) == 40 {
            return word(8);
        }
    }
}

// A peer that keeps its control connection open and never answers an offer
// must not leave the actor listening for that answer, or a token per offer
// piles up for as long as the connection lasts.
#[test]
fn an_offer_the_client_gave_up_on_cannot_be_accepted_but_a_live_one_can() {
    use crate::actor::ActorSystem;
    use crate::actor::peer_registry::PeerRegistry;
    use crate::client::{Client, ClientContext};
    use std::io::Write;

    let (tx, rx) = std::sync::mpsc::channel();
    let registry =
        PeerRegistry::new(Arc::new(ActorSystem::new()), tx, "me".to_string());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    stream.set_nonblocking(true).unwrap();
    let mut far_end = listener.accept().unwrap().0;
    let bob = Peer::new(
        "bob".to_string(),
        ConnectionType::P,
        "127.0.0.1".to_string(),
        0,
        None,
        0,
        0,
        0,
    );
    registry.register_peer(bob, Some(stream), None).unwrap();

    let mut ctx = ClientContext::new();
    ctx.peer_registry = Some(registry);
    let ctx = Arc::new(RwLock::new(ctx));
    let offer = |file: &str| {
        ctx.write().unwrap().enqueue_upload(
            "bob",
            file,
            std::path::PathBuf::from("/tmp/x.mp3"),
            1,
        );
        Client::pump_upload_queue(&ctx);
    };

    offer("expired.mp3");
    let expired = next_offer_token(&mut far_end);
    let long_after = Instant::now() + Duration::from_hours(1);
    assert!(ctx.write().unwrap().expire_stale_offers(long_after));
    Client::pump_upload_queue(&ctx);

    offer("cancelled.mp3");
    let cancelled = next_offer_token(&mut far_end);
    assert!(Client::cancel_upload_in(&ctx, "bob", "cancelled.mp3"));

    // The actor reads its mailbox in order, so this offer reaching the wire
    // means it has heard about the two before it.
    offer("live.mp3");
    let live = next_offer_token(&mut far_end);

    for token in [expired, cancelled, live] {
        let accept = Message::new()
            .write_int32(41)
            .write_int32(token)
            .write_bool(true)
            .clone();
        far_end.write_all(&accept.get_buffer()).unwrap();
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut started = Vec::new();
    while !started.contains(&live) {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(ClientOperation::StartUpload { token }) => started.push(token),
            Ok(_) => {}
            Err(e) => panic!("the live offer never started: {e}"),
        }
    }
    assert_eq!(
        started,
        [live],
        "an acceptance of an offer the client gave up on starts nothing"
    );
}
