//! Dispatch arms that move bytes: what a peer allows, what it asks of us,
//! and the tokens tying the two together.
//!
//! Lifted out of the dispatch loop whole, so the bodies read exactly as they
//! did inside it — including `own_username`, which is re-owned on the way in
//! rather than have every line that clones it rewritten.

use super::{
    Arc, Client, ClientContext, ClientOperation, DownloadError, DownloadPeer,
    MessageFactory, PeerMessage, RwLock, RwLockExt, ServerMessage, debug,
    error, info, next_download_token, thread, trace,
};

impl Client {
    pub(super) fn on_transfers(
        operation: ClientOperation,
        client_context: &Arc<RwLock<ClientContext>>,
        own_username: &str,
    ) {
        let own_username = own_username.to_string();
        match operation {
            ClientOperation::DownloadFromPeer(token, peer, allowed) => {
                let maybe_download = match client_context.write_safe() {
                    Ok(mut ctx) => {
                        ctx.downloads.claim_for_peer(token, &peer.username)
                    }
                    Err(e) => {
                        error!("[client] DownloadFromPeer write: {}", e);
                        return;
                    }
                };
                trace!(
                    "[client] DownloadFromPeer token: {} peer: {:?}",
                    token, peer
                );
                let Some(download) = maybe_download else {
                    debug!(
                        "[client] transfer token {} is missing, belongs \
                         to another peer, or is already claimed; ignoring",
                        token,
                    );
                    return;
                };

                let client_context = client_context.clone();

                thread::spawn(move || {
                    let download_peer = DownloadPeer::new(
                        download.username.clone(),
                        peer.host.clone(),
                        peer.port,
                        token,
                        allowed,
                        own_username,
                    );
                    match download_peer.download_file(
                        client_context,
                        Some(download.clone()),
                        None,
                    ) {
                        Ok((download, filename)) => {
                            info!(
                                "Successfully downloaded {} bytes to {}",
                                download.size, filename
                            );
                        }
                        Err(DownloadError::Cancelled) => {}
                        Err(e) => {
                            error!(
                                "Failed to download file '{}' from {}:{} (token: {}) - Error: {}",
                                download.filename,
                                peer.host,
                                peer.port,
                                download.token,
                                e
                            );
                        }
                    }
                });
            }
            ClientOperation::UpdateDownloadTokens(transfer, username) => {
                let mut context = match client_context.write_safe() {
                    Ok(c) => c,
                    Err(e) => {
                        error!("[client] UpdateDownloadTokens write: {}", e);
                        return;
                    }
                };

                let refusal = context
                    .downloads
                    .refusal_of_offer(&username, &transfer.filename);
                if refusal.is_none() {
                    context.downloads.rekey(
                        &username,
                        &transfer.filename,
                        transfer.token,
                        transfer.size,
                        next_download_token,
                    );
                }

                // Only now invite the file connection: it is
                // matched by this token, which is recorded as
                // of the line above. Answering any earlier
                // races the peer's connection against our own
                // bookkeeping.
                let registry = context.peer_registry.clone();
                drop(context);
                if let Some(registry) = registry {
                    let response = match refusal {
                        Some(reason) => {
                            MessageFactory::build_transfer_denial_message(
                                transfer.token,
                                reason,
                            )
                        }
                        None => {
                            MessageFactory::build_transfer_response_message(
                                transfer,
                            )
                        }
                    };
                    let _ = registry.send_to_peer(
                        &username,
                        PeerMessage::SendMessage(response),
                    );
                }
            }
            ClientOperation::StartUpload { token } => {
                // The peer accepted our offer: resolve their
                // address (from the code-9 GetPeerAddress) and
                // stream the file, or queue until it resolves.
                let (job_addr, downloader) = match client_context.write_safe() {
                    Ok(mut ctx) => {
                        ctx.mark_offer_answered(token);
                        let Some(job) = ctx.uploads.get(&token) else {
                            return;
                        };
                        (
                            ctx.peer_address(&job.downloader),
                            job.downloader.clone(),
                        )
                    }
                    Err(_) => return,
                };
                if let Some((host, port)) = job_addr {
                    Self::spawn_serve(
                        client_context,
                        &own_username,
                        token,
                        host,
                        port,
                    );
                } else {
                    if let Ok(mut ctx) = client_context.write_safe() {
                        ctx.pending_serves
                            .entry(downloader.clone())
                            .or_default()
                            .push(token);
                    }
                    if let Ok(ctx) = client_context.read_safe()
                        && let Some(sender) = ctx.server_sender.clone()
                    {
                        let _ = sender
                            .send(ServerMessage::GetPeerAddress(downloader));
                    }
                }
            }
            ClientOperation::QueueUpload {
                requester_key,
                filename,
                transfer_token,
            } => {
                let shared = match client_context.write_safe() {
                    Ok(mut ctx) => ctx
                        .shares
                        .get(&filename)
                        .map(|file| (file.real_path.clone(), file.size))
                        .map(|(real_path, size)| {
                            ctx.enqueue_upload(
                                &requester_key,
                                &filename,
                                real_path,
                                size,
                            )
                        })
                        .is_some(),
                    Err(e) => {
                        error!("[client] QueueUpload write: {}", e);
                        return;
                    }
                };
                let refusal = (!shared).then_some("File not shared.");
                let answer = match transfer_token {
                    Some(token) => {
                        Some(MessageFactory::build_transfer_denial_message(
                            token,
                            refusal.unwrap_or("Queued"),
                        ))
                    }
                    None => refusal.map(|reason| {
                        MessageFactory::build_upload_denied_message(
                            &filename, reason,
                        )
                    }),
                };
                if let Some(answer) = answer {
                    Self::reply_to_peer(client_context, &requester_key, |_| {
                        answer
                    });
                }
                if shared {
                    // The peer served next may not be this one.
                    Self::pump_upload_queue(client_context);
                }
            }
            // The dispatch loop routes only the variants above to here.
            _ => {}
        }
    }
}
