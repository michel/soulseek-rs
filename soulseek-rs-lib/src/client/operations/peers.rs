//! Dispatch arms about a peer connection appearing or going away.
//!
//! Lifted out of the dispatch loop whole, so the bodies read exactly as they
//! did inside it — including `own_username`, which is re-owned on the way in
//! rather than have every line that clones it rewritten.

use super::{
    Arc, CHILD_WRITE_TIMEOUT, Client, ClientContext, ClientOperation,
    ConnectionType, RwLock, RwLockExt, ServerMessage, debug, error,
    next_connect_token, warn,
};

impl Client {
    pub(super) fn on_peers(
        operation: ClientOperation,
        client_context: &Arc<RwLock<ClientContext>>,
        own_username: &str,
    ) {
        let own_username = own_username.to_string();
        match operation {
            ClientOperation::PeerDisconnected(id, username, error) => {
                // Scope the read guard: process_failed_uploads
                // below acquires a write lock on the same
                // RwLock, which would self-deadlock the entire
                // client ops loop if this read guard were still
                // held on this thread. Evict only if this exact
                // actor still occupies the slot, so a replaced
                // actor's shutdown can't remove its successor.
                {
                    let context = match client_context.read_safe() {
                        Ok(c) => c,
                        Err(e) => {
                            error!("[client] PeerDisconnected read: {}", e);
                            return;
                        }
                    };
                    if let Some(ref registry) = context.peer_registry
                        && let Some(handle) =
                            registry.remove_peer_if(&username, id)
                    {
                        let _ = handle.stop();
                    }
                }
                // Only an error is evidence the peer is gone. A clean
                // close — our idle reaper, or a remote client tidying
                // an idle socket while it waits in our queue — must
                // not throw away everything that peer has queued: the
                // connection comes back, the queue cannot. A departed
                // peer wedging uploads shut is still covered: its
                // erroring transfer fails within the socket deadlines
                // and the error branch here frees its slots.
                if let Some(error) = error {
                    warn!(
                        "[client] Peer {} disconnected with error: {:?}",
                        username, error
                    );
                    Self::process_failed_uploads(
                        client_context.clone(),
                        &username,
                        None,
                    );
                    Self::release_upload_slots(client_context, &username);
                }
            }
            ClientOperation::PeerConnectFailed(
                id,
                username,
                brokered_token,
            ) => {
                // Direct connect failed. A dial the server asked
                // us to make is answered with CantConnectToPeer
                // quoting the peer's own token: that peer is
                // already waiting on the server, and asking it to
                // broker the same connection back only trades one
                // unreachable direction for the other. Every other
                // dial falls back to the broker below.
                if let Some(peer_token) = brokered_token {
                    let server_sender = match client_context.write_safe() {
                        Ok(ctx) => {
                            if let Some(handle) = ctx
                                .peer_registry
                                .as_ref()
                                .and_then(|r| r.remove_peer_if(&username, id))
                            {
                                let _ = handle.stop();
                            }
                            ctx.server_sender.clone()
                        }
                        Err(e) => {
                            error!("[client] PeerConnectFailed write: {}", e);
                            return;
                        }
                    };
                    if let Some(sender) = server_sender {
                        let msg = crate::message::server::MessageFactory::build_cant_connect_to_peer(
                            peer_token,
                            &username,
                        );
                        let _ = sender.send(ServerMessage::SendMessage(msg));
                    }
                    // Nothing else is coming from this peer, so
                    // anything queued for it fails now rather than
                    // waiting out its timeout.
                    Self::fail_queued_downloads(client_context, &username);
                    return;
                }

                // Direct connect failed: ask the server to
                // broker it. Register a correlation token, then
                // send ConnectToPeer so the (firewalled) peer
                // connects back to our listener quoting it.
                let token = next_connect_token();
                let server_sender = match client_context.write_safe() {
                    Ok(mut ctx) => {
                        // Reap the dead outbound actor so it
                        // stops pinning a pool worker and no
                        // longer shadows the brokered reconnect
                        // (a stale registry entry would make
                        // later downloads queue into a dead,
                        // streamless actor and hang). Identity-
                        // aware so a newer namesake is untouched.
                        if let Some(handle) = ctx
                            .peer_registry
                            .as_ref()
                            .and_then(|r| r.remove_peer_if(&username, id))
                        {
                            let _ = handle.stop();
                        }
                        ctx.add_pending_connect(token, username.clone());
                        ctx.server_sender.clone()
                    }
                    Err(e) => {
                        error!("[client] PeerConnectFailed write: {}", e);
                        return;
                    }
                };
                let Some(sender) = server_sender else {
                    return;
                };
                let msg = crate::message::server::MessageFactory::build_connect_to_peer(
                    token,
                    &username,
                    ConnectionType::P,
                );
                let _ = sender.send(ServerMessage::SendMessage(msg));
            }
            ClientOperation::ChildConnected { username, stream } => {
                // A peer wants to hang from us. Take it on if this
                // client serves children and has room, then tell it
                // where our branch sits so it can report its own
                // place; otherwise the socket is dropped here and the
                // peer looks for another parent.
                let Ok(mut ctx) = client_context.write_safe() else {
                    return;
                };
                if !ctx.children.has_room() {
                    return;
                }
                // A child that stops reading must not wedge us: the
                // relay writes under the context lock, so the write
                // is bounded and a child that hits it loses its slot.
                if stream.set_nodelay(true).is_err()
                    || stream
                        .set_write_timeout(Some(CHILD_WRITE_TIMEOUT))
                        .is_err()
                {
                    return;
                }
                if username == own_username {
                    return; // we cannot hang from ourselves
                }
                if ctx.children.accept(&username, stream) {
                    let branch = ctx.leaf.branch();
                    ctx.children.send_stance_to(
                        &username,
                        &branch.root,
                        branch.level,
                    );
                    debug!(
                        "[distributed] carrying {} children",
                        ctx.children.len()
                    );
                    // The server stops offering us to peers once we
                    // are full, and starts again when one leaves.
                    let full = !ctx.children.has_room();
                    drop(ctx);
                    if full {
                        Self::announce_child_capacity(client_context);
                    }
                }
            }
            // The dispatch loop routes only the variants above to here.
            _ => {}
        }
    }
}
