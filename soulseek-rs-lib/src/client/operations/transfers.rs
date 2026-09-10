//! Dispatch arms that move bytes: what a peer allows, what it asks of us,
//! and the tokens tying the two together.
//!
//! Lifted out of the dispatch loop whole, so the bodies read exactly as they
//! did inside it — including `own_username`, which is re-owned on the way in
//! rather than have every line that clones it rewritten.

use super::{
    Arc, Client, ClientContext, ClientOperation, Download, DownloadError,
    DownloadPeer, DownloadStatus, MessageFactory, PeerMessage, RwLock,
    RwLockExt, ServerMessage, debug, error, info, thread, trace,
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

                let download_to_update =
                    context.get_downloads().iter().find_map(|d| {
                        if d.username == username
                            && d.filename == transfer.filename
                        {
                            Some((d.token, d.clone()))
                        } else {
                            None
                        }
                    });

                let cancelled =
                    download_to_update.as_ref().is_some_and(|(_, d)| {
                        matches!(d.status, DownloadStatus::Cancelled)
                    });
                if !cancelled
                    && let Some((old_token, download)) = download_to_update
                {
                    trace!(
                        "[client] UpdateDownloadTokens found {old_token}, transfer: {:?}",
                        transfer
                    );

                    context.add_download(Download {
                        token: transfer.token,
                        size: transfer.size,
                        ..download
                    });
                    context.remove_download(old_token);
                }

                // Only now invite the file connection: it is
                // matched by this token, which is recorded as
                // of the line above. Answering any earlier
                // races the peer's connection against our own
                // bookkeeping.
                let registry = context.peer_registry.clone();
                drop(context);
                if let Some(registry) = registry {
                    let response = if cancelled {
                        MessageFactory::build_transfer_denial_message(
                            transfer.token,
                            "Cancelled",
                        )
                    } else {
                        MessageFactory::build_transfer_response_message(
                            transfer,
                        )
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
            } => {
                // The peer served next may not be this one.
                match client_context.write_safe() {
                    Ok(mut ctx) => {
                        let Some(file) = ctx.shares.get(&filename) else {
                            debug!(
                                "[client] QueueUpload for unknown file {}",
                                filename
                            );
                            return;
                        };
                        let size = file.size;
                        let real_path = file.real_path.clone();
                        ctx.enqueue_upload(
                            &requester_key,
                            &filename,
                            real_path,
                            size,
                        );
                    }
                    Err(e) => {
                        error!("[client] QueueUpload write: {}", e);
                        return;
                    }
                }
                Self::pump_upload_queue(client_context);
            }
            // The dispatch loop routes only the variants above to here.
            _ => {}
        }
    }
}
