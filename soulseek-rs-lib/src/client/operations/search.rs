//! The dispatch arm that answers a search reaching us from the network.
//!
//! Lifted out of the dispatch loop whole, so the bodies read exactly as they
//! did inside it — including `own_username`, which is re-owned on the way in
//! rather than have every line that clones it rewritten.

use super::{
    Arc, Client, ClientContext, ClientOperation, Instant, PeerMessage, RwLock,
    RwLockExt, ServerMessage, build_search_response, error,
};

impl Client {
    pub(super) fn on_search(
        operation: ClientOperation,
        client_context: &Arc<RwLock<ClientContext>>,
        own_username: &str,
    ) {
        let own_username = own_username.to_string();
        let ClientOperation::IncomingSearch {
            username,
            token,
            query,
            from_parent,
        } = operation
        else {
            return;
        };
        {
            // Every search we receive comes from the tree: from
            // our parent, or from the server acting as one. That
            // is what qualifies us to carry children at all —
            // Nicotine+ refuses them until something feeds it —
            // and passing the search down is the whole duty.
            let (capacity_changed, answer) = match client_context.write_safe() {
                Ok(mut ctx) => {
                    let was_fed = ctx.children.is_fed();
                    let before = ctx.children.len();
                    ctx.set_fed_by(true);
                    if !from_parent {
                        // A parent's search was passed down where
                        // it arrived; this is the server's.
                        ctx.children.broadcast_search(&username, token, &query);
                    }
                    // Answering costs a scan of everything we
                    // share, so the budget applies to searches
                    // from the tree. The server's own are few.
                    let answer =
                        !from_parent || ctx.leaf.admit_search(Instant::now());
                    // A child whose socket has gone is dropped by
                    // that write, which frees a slot the server
                    // should hear about.
                    (!was_fed || ctx.children.len() != before, answer)
                }
                Err(_) => (false, false),
            };
            if capacity_changed {
                Self::announce_child_capacity(client_context);
            }
            if !answer {
                return;
            }

            // Don't answer our own distributed search.
            if username == own_username {
                return;
            }
            let response = match client_context.read_safe() {
                Ok(ctx) => build_search_response(
                    &ctx.shares,
                    &own_username,
                    token,
                    &query,
                    ctx.has_free_upload_slot(),
                    ctx.last_upload_speed,
                    ctx.upload_queue.len() as u32,
                    &ctx.excluded_search_phrases,
                ),
                Err(e) => {
                    error!("[client] IncomingSearch read: {}", e);
                    return;
                }
            };
            let Some(message) = response else {
                return; // no matching shares
            };

            // Deliver to the searcher: send now if we have a
            // control connection, else open one and queue.
            let (connected, registry, server_sender) =
                match client_context.read_safe() {
                    Ok(ctx) => (
                        ctx.peer_registry
                            .as_ref()
                            .is_some_and(|r| r.contains(&username)),
                        ctx.peer_registry.clone(),
                        ctx.server_sender.clone(),
                    ),
                    Err(_) => return,
                };
            if connected {
                if let Some(registry) = registry {
                    let _ = registry.send_to_peer(
                        &username,
                        PeerMessage::SendMessage(message),
                    );
                }
            } else {
                if let Ok(mut ctx) = client_context.write_safe() {
                    ctx.queue_peer_message(&username, message);
                }
                if let Some(sender) = server_sender {
                    let _ =
                        sender.send(ServerMessage::GetPeerAddress(username));
                }
            }
        }
    }
}
