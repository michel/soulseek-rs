use super::{
    ActiveUpload, Arc, Client, ClientContext, DownloadStatus, RwLock,
    RwLockExt, collect_failed_tokens, error, thread,
};
use crate::types::UploadStatus;
use std::sync::atomic::{AtomicBool, AtomicU64};

impl Client {
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
            },
        );
        drop(ctx);
        let own = own_username.to_string();
        let real_path = job.real_path;
        let context = client_context.clone();
        thread::spawn(move || {
            let result = crate::peer::upload_peer::serve_file(
                &host,
                port,
                &own,
                token,
                &real_path,
                &bytes_sent,
                &cancel,
            );
            let status = match &result {
                Ok(()) => UploadStatus::Completed,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    UploadStatus::Cancelled
                }
                Err(e) => {
                    error!("[client] serve {}: {}", real_path.display(), e);
                    UploadStatus::Failed(e.to_string())
                }
            };
            if let Ok(mut ctx) = context.write_safe()
                && let Some(upload) = ctx.active_uploads.get_mut(&token)
            {
                upload.status = status;
            }
        });
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
