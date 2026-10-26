use super::{
    ActiveUpload, Arc, BrokeredUpload, Client, ClientContext, DownloadStatus,
    PeerMessage, RwLock, RwLockExt, ServerMessage, collect_failed_tokens,
    debug, error, next_connect_token, next_upload_token, thread,
};
use crate::message::server::MessageFactory;
use crate::peer::{ConnectionType, upload_peer};
use crate::types::UploadStatus;
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

impl Client {
    /// Whether the server listed `username` as privileged. Privileged peers are
    /// served before everyone else.
    #[must_use]
    pub fn is_privileged(&self, username: &str) -> bool {
        self.context
            .read_safe()
            .is_ok_and(|ctx| ctx.is_privileged(username))
    }

    /// Ask the server how much of our own privilege time is left (code 92). The
    /// answer arrives asynchronously; read it with
    /// [`Client::own_privilege_seconds`].
    ///
    /// # Errors
    /// [`crate::error::SoulseekRs::NotConnected`] without a server connection.
    pub fn check_privileges(&self) -> crate::error::Result<()> {
        let Some(handle) = &self.server_handle else {
            return Err(crate::error::SoulseekRs::NotConnected);
        };
        let _ = handle.send(super::ServerMessage::CheckPrivileges);
        Ok(())
    }

    /// Seconds of our own privileges left, or `None` until the server has
    /// answered a [`Client::check_privileges`].
    #[must_use]
    pub fn own_privilege_seconds(&self) -> Option<u32> {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| ctx.own_privileges)
    }

    /// Queued-upload states recorded since the last call, for a caller that
    /// samples [`Client::uploads`] and would otherwise miss a peer that queued
    /// and was served between two samples.
    #[must_use]
    pub fn take_upload_events(&self) -> Vec<crate::types::UploadInfo> {
        self.context
            .write_safe()
            .map(|mut ctx| ctx.take_upload_events())
            .unwrap_or_default()
    }

    /// How many uploads run at once. Lowering it does not interrupt transfers
    /// already in flight; the queue simply refills more slowly.
    pub fn set_upload_slots(&self, slots: usize) {
        if let Ok(mut ctx) = self.context.write_safe() {
            ctx.upload_slots = slots.max(1);
        }
    }

    /// Drop everything `username` was queued or offered, then hand their slot to
    /// whoever is next.
    pub(crate) fn release_upload_slots(
        client_context: &Arc<RwLock<ClientContext>>,
        username: &str,
    ) {
        let freed = client_context
            .write_safe()
            .is_ok_and(|mut ctx| ctx.release_upload_slots(username));
        if freed {
            Self::pump_upload_queue(client_context);
        }
    }

    /// Fill every free upload slot from the queue and send the offers.
    ///
    /// Called whenever the queue or the slot count could have changed: a new
    /// request, a finished transfer, or a privileged list that re-ranked who is
    /// waiting.
    pub(crate) fn pump_upload_queue(
        client_context: &Arc<RwLock<ClientContext>>,
    ) {
        let (registry, offers) = match client_context.write_safe() {
            Ok(mut ctx) => ctx.pump_uploads(next_upload_token),
            Err(e) => {
                error!("[client] pump_upload_queue write: {}", e);
                return;
            }
        };
        let Some(registry) = registry else {
            return;
        };
        for offer in offers {
            let _ = registry.send_to_peer(
                &offer.requester_key,
                PeerMessage::ServeUpload {
                    token: offer.token,
                    filename: offer.virtual_path,
                    size: offer.size,
                },
            );
        }
    }

    /// Consume the upload job for `token` and stream the file to `host:port`
    /// on a background thread.
    pub(crate) fn spawn_serve(
        client_context: &Arc<RwLock<ClientContext>>,
        own_username: &str,
        token: u32,
        host: String,
        port: u32,
    ) {
        let Ok(mut ctx) = client_context.write_safe() else {
            return;
        };
        let Some(job) = ctx.uploads.remove(&token) else {
            return;
        };
        let bytes_sent = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        ctx.active_uploads.insert(
            token,
            ActiveUpload {
                username: job.downloader.clone(),
                filename: job.virtual_path.clone(),
                size: job.size,
                bytes_sent: bytes_sent.clone(),
                cancel: cancel.clone(),
                status: UploadStatus::InProgress,
                started: std::time::Instant::now(),
            },
        );
        drop(ctx);
        let own = own_username.to_string();
        let real_path = job.real_path;
        let downloader = job.downloader;
        let context = client_context.clone();
        thread::spawn(move || {
            // Most downloaders sit behind a NAT, so dialling them is a gamble
            // that costs a 20s timeout when it fails. Like the reference
            // clients, ask the server to have them connect back at the same
            // time; the first socket to arrive claims the transfer, and port 0
            // (the server not knowing where they listen) skips the dial.
            let Some((connect_token, claimed)) =
                Self::broker_upload(&context, token, &real_path, &downloader)
            else {
                return;
            };
            if port == 0 {
                return;
            }
            let stream = match upload_peer::dial(&host, port) {
                Ok(stream) => stream,
                Err(e) => {
                    debug!(
                        "[client] cannot dial {downloader} at {host}:{port} \
                         for upload {token} ({e}); waiting for them to connect"
                    );
                    return;
                }
            };
            if claimed
                .compare_exchange(
                    false,
                    true,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
            {
                return;
            }
            if let Ok(mut ctx) = context.write_safe() {
                ctx.take_brokered_upload(connect_token);
            }
            let result = upload_peer::serve_direct(
                stream,
                &own,
                token,
                &real_path,
                &bytes_sent,
                &cancel,
            );
            Self::finish_upload(&context, token, &real_path, result);
        });
    }

    /// Park the upload under a correlation token and ask the server to have
    /// `downloader` connect back to us with it. Returns the token and the claim
    /// flag the dialling route races against; `None` fails the upload.
    fn broker_upload(
        context: &Arc<RwLock<ClientContext>>,
        token: u32,
        real_path: &Path,
        downloader: &str,
    ) -> Option<(u32, Arc<AtomicBool>)> {
        let connect_token = next_connect_token();
        let (claimed, sender) = match context.write_safe() {
            Ok(mut ctx) => (
                ctx.add_brokered_upload(
                    connect_token,
                    token,
                    real_path.to_path_buf(),
                ),
                ctx.server_sender.clone(),
            ),
            Err(_) => return None,
        };
        let Some(sender) = sender else {
            Self::fail_upload(context, token, "not connected to the server");
            return None;
        };
        let _ = sender.send(ServerMessage::SendMessage(
            MessageFactory::build_connect_to_peer(
                connect_token,
                downloader,
                ConnectionType::F,
            ),
        ));
        Some((connect_token, claimed))
    }

    /// The downloader of a brokered upload connected back: stream the file
    /// over its socket on a background thread, unless the dial beat it.
    pub(crate) fn serve_pierced(
        client_context: &Arc<RwLock<ClientContext>>,
        upload: BrokeredUpload,
        stream: TcpStream,
    ) {
        if !upload.claim() {
            return;
        }
        let Some((bytes_sent, cancel)) =
            client_context.write_safe().ok().and_then(|mut ctx| {
                let active = ctx.active_uploads.get_mut(&upload.token)?;
                active.started = std::time::Instant::now();
                Some((active.bytes_sent.clone(), active.cancel.clone()))
            })
        else {
            return;
        };
        let context = client_context.clone();
        thread::spawn(move || {
            let result = upload_peer::serve_pierced(
                stream,
                upload.token,
                &upload.real_path,
                &bytes_sent,
                &cancel,
            );
            Self::finish_upload(
                &context,
                upload.token,
                &upload.real_path,
                result,
            );
        });
    }

    /// Mark upload `token` failed and hand its slot to the next in line.
    pub(crate) fn fail_upload(
        client_context: &Arc<RwLock<ClientContext>>,
        token: u32,
        reason: &str,
    ) {
        if let Ok(mut ctx) = client_context.write_safe()
            && let Some(upload) = ctx.active_uploads.get_mut(&token)
        {
            error!("[client] upload {token} to {}: {reason}", upload.username);
            upload.status = UploadStatus::Failed(reason.to_string());
        }
        Self::pump_upload_queue(client_context);
    }

    /// Record how a served upload ended, report its rate to the server and free
    /// its slot.
    fn finish_upload(
        context: &Arc<RwLock<ClientContext>>,
        token: u32,
        real_path: &Path,
        result: std::io::Result<u64>,
    ) {
        let streamed = result.as_ref().ok().copied();
        let status = match &result {
            Ok(_) => UploadStatus::Completed,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                UploadStatus::Cancelled
            }
            Err(e) => {
                error!("[client] serve {}: {}", real_path.display(), e);
                UploadStatus::Failed(e.to_string())
            }
        };
        let mut report = None;
        if let Ok(mut ctx) = context.write_safe()
            && let Some(upload) = ctx.active_uploads.get_mut(&token)
        {
            let secs = upload.started.elapsed().as_secs_f64();
            upload.status = status;
            if let Some(bytes) = streamed
                && secs > 0.0
            {
                let speed = (bytes as f64 / secs) as u32;
                ctx.last_upload_speed = speed;
                report = ctx.server_sender.clone().map(|s| (s, speed));
            }
        }
        // The server folds each finished upload's rate into the average
        // speed it shows other users for us.
        if let Some((server, speed)) = report {
            let _ = server.send(ServerMessage::SendMessage(
                MessageFactory::build_send_upload_speed(speed),
            ));
        }
        // The slot this transfer held is free now, so whoever is next in
        // line gets it without waiting for another request to arrive.
        Self::pump_upload_queue(context);
    }

    pub(crate) fn process_failed_uploads(
        client_context: Arc<RwLock<ClientContext>>,
        username: &str,
        filename: Option<&str>,
    ) {
        let failed_tokens = match client_context.read_safe() {
            Ok(context) => {
                collect_failed_tokens(&context.downloads, username, filename)
            }
            Err(e) => {
                error!("[client] process_failed_uploads read: {}", e);
                return;
            }
        };

        if failed_tokens.is_empty() {
            return;
        }

        match client_context.write_safe() {
            Ok(mut context) => {
                for token in failed_tokens {
                    context.downloads.update_status(
                        token,
                        DownloadStatus::Failed(Some(
                            "The upload failed on the other side".to_string(),
                        )),
                    );
                    context.downloads.remove(token);
                }
            }
            Err(e) => {
                error!("[client] process_failed_uploads write: {}", e);
            }
        }
    }
}
