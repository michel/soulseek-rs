use super::*;

fn code_of(message: &Message) -> u32 {
    u32::from_le_bytes(message.get_data()[0..4].try_into().unwrap())
}

#[test]
fn a_timed_out_connect_parks_the_actor_in_disconnected() {
    let mut actor = parked_actor(1);
    actor.connection_state = ConnectionState::Connecting {
        since: Instant::now().checked_sub(Duration::from_secs(21)).unwrap(),
    };

    actor.tick();

    assert!(
        matches!(actor.connection_state, ConnectionState::Disconnected),
        "a timed-out connect must leave Connecting"
    );
}

#[cfg(unix)]
#[test]
fn the_server_socket_keeps_itself_alive() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut actor = parked_actor(listener.local_addr().unwrap().port());

    assert!(actor.initiate_connection());

    assert!(
        crate::utils::keepalive::keepalive_enabled(
            actor.stream.as_ref().unwrap()
        ),
        "an idle NAT mapping must be kept alive and a dead path noticed"
    );
}

fn parked_actor(port: u16) -> ServerActor {
    parked_actor_with_client(port).0
}

fn parked_actor_with_client(
    port: u16,
) -> (ServerActor, std::sync::mpsc::Receiver<ClientOperation>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let actor = ServerActor::new(
        PeerAddress::new("127.0.0.1".to_string(), port),
        tx,
        0,
        false,
        0,
        0,
    );
    (actor, rx)
}

#[test]
fn dropping_the_connection_forgets_the_dispatcher_and_the_verdict() {
    let mut actor = parked_actor(1);
    actor.initialize_dispatcher();
    actor.connection_state = ConnectionState::Connected;
    actor.handle_login_status(true);

    actor.disconnect_with_error();

    assert_eq!(actor.session.loss(), Some(SessionLoss::Disconnected));
    assert!(
        actor.dispatcher_sender.is_none(),
        "a login queued now must wait in the buffer, not in a channel \
         the next connection replaces"
    );
    assert_eq!(
        actor.context.read().unwrap().logged_in,
        None,
        "the old verdict must not answer the next login"
    );
}

#[test]
fn disconnect_clears_a_partial_frame_so_the_next_session_reframes_clean() {
    use std::io::Write;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).unwrap();
    let (mut server_side, _) = listener.accept().unwrap();
    let partial_frame = [10u8, 0, 0, 0, 1, 2, 3];
    server_side.write_all(&partial_frame).unwrap();
    server_side.flush().unwrap();
    client.set_nonblocking(true).unwrap();

    let mut actor = parked_actor(addr.port());
    actor.stream = Some(client);
    actor.connection_state = ConnectionState::Connected;
    for _ in 0..50 {
        actor.process_read();
        if actor.reader.buffer_len() > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        actor.reader.buffer_len() > 0,
        "an incomplete frame should be waiting for its rest"
    );

    actor.disconnect();

    assert_eq!(
        actor.reader.buffer_len(),
        0,
        "a stale partial frame carried across a reconnect reads the next \
         session's bytes as a length prefix and wedges recovery"
    );
}

#[test]
fn a_login_on_a_parked_actor_dials_again_and_queues_the_login() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut actor = parked_actor(listener.local_addr().unwrap().port());
    actor.session.record(SessionLoss::Disconnected);
    let (response, _verdict) = std::sync::mpsc::channel();

    actor.handle_login(
        "u".into(),
        "p".into(),
        ClientVersion::default(),
        response,
    );

    assert!(
        matches!(actor.connection_state, ConnectionState::Connecting { .. }),
        "a parked actor must dial again on login"
    );
    assert_eq!(
        actor.queued_messages.len(),
        1,
        "the login waits for the connection to come up"
    );
}

#[test]
fn a_parked_actor_handles_login_rather_than_queueing_it() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut actor = parked_actor(listener.local_addr().unwrap().port());
    actor.session.record(SessionLoss::Disconnected);
    let (response, _verdict) = std::sync::mpsc::channel();

    actor.handle_message(ServerMessage::Login {
        username: "u".into(),
        password: "p".into(),
        version: ClientVersion::default(),
        response,
    });

    assert!(
        matches!(actor.connection_state, ConnectionState::Connecting { .. }),
        "a parked actor must act on a login, not park it in the queue"
    );
    assert!(
        actor
            .queued_messages
            .iter()
            .all(|m| !matches!(m, ServerMessage::Login { .. })),
        "the login must not be sitting unhandled in the queue"
    );
}

#[test]
fn a_login_with_nobody_listening_is_refused_at_once() {
    let mut actor = parked_actor(1);
    let (response, verdict) = std::sync::mpsc::channel();

    actor.handle_login(
        "u".into(),
        "p".into(),
        ClientVersion::default(),
        response,
    );

    assert!(
        matches!(
            verdict.recv_timeout(Duration::from_secs(5)),
            Ok(Err(SoulseekRs::NotConnected))
        ),
        "a refused dial must not make the caller wait out the verdict \
         timeout"
    );
}

#[test]
fn a_successful_login_marks_the_session_live_again() {
    let mut actor = parked_actor(1);
    actor.session.record(SessionLoss::Disconnected);

    actor.handle_login_status(true);

    assert_eq!(actor.session.loss(), None);
}

// A new session starts without a parent: the leaf drops whatever tree it
// hung from and announces its parentless stance from there.
#[test]
fn a_successful_login_resets_the_distributed_leaf() {
    let (mut actor, client) = parked_actor_with_client(1);

    actor.handle_login_status(true);

    assert!(matches!(
        client.try_recv(),
        Ok(ClientOperation::ResetDistributed)
    ));
    // A new session also has to be told what only the session held: our
    // interests, which the server forgets when the old one ended.
    assert!(matches!(
        client.try_recv(),
        Ok(ClientOperation::SessionEstablished)
    ));
    actor.handle_login_status(false);
    assert!(client.try_recv().is_err(), "a failed login resets nothing");
}

#[test]
fn post_login_messages_carry_counts_and_conditional_wait_port() {
    let messages = post_login_messages(true, 4321, 3, 7);
    let codes: Vec<u32> = messages.iter().map(code_of).collect();
    // SharedFolders; SetStatus; SetWaitPort.
    assert_eq!(codes, vec![35, 28, 2]);

    // The SharedFolders message (code 35) carries the real counts.
    let shared = messages[0].get_data();
    assert_eq!(u32::from_le_bytes(shared[4..8].try_into().unwrap()), 3);
    assert_eq!(u32::from_le_bytes(shared[8..12].try_into().unwrap()), 7);

    // Not listening omits SetWaitPort (code 2).
    let no_listen = post_login_messages(false, 4321, 3, 7);
    let codes: Vec<u32> = no_listen.iter().map(code_of).collect();
    assert_eq!(codes, vec![35, 28]);
}
