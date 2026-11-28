//! The queue peers wait in for one of our upload slots.
//!
//! Soulseek's rules ask an alternative client for "respect / recognition of
//! privileges": a user who has donated is meant to be served before one who
//! has not. That only means anything if there is a queue to jump — serving
//! every request the instant it arrives treats a privileged peer exactly like
//! everybody else — so the slot cap and the ordering are one feature.
//!
//! Ordering is privileged-first, then first-come. Nothing else: a fancier
//! policy (per-user fairness, small-file-first) would be a different promise
//! than the one the rules ask for.

use super::{
    ActiveUpload, ClientContext, Duration, Instant, PeerRegistry, UploadJob,
};
use crate::types::{UploadInfo, UploadStatus};
use std::collections::HashMap;
use std::path::PathBuf;

/// Cap on remembered queued-upload states, so a caller that never drains
/// them cannot grow the list without bound.
const MAX_UPLOAD_EVENTS: usize = 256;

/// How long an offered slot waits for the peer's TransferResponse. A peer that
/// never answers, having gone away or hung, would otherwise hold the slot for
/// the life of the process; with two slots, two of them shut uploads down.
const OFFER_TIMEOUT: Duration = Duration::from_secs(30);

/// How long an accepted offer may wait for the transfer to begin. Between the
/// peer's TransferResponse and the first byte sits a `GetPeerAddress` round
/// trip through the server, and an answer lost to a dropped session never
/// comes: without a limit that upload holds its slot until the process exits.
/// Generous next to a healthy lookup, which takes well under a second.
const ACCEPTED_TIMEOUT: Duration = Duration::from_mins(2);

/// How many finished uploads stay listed. The live ones are bounded by the
/// slot count; the finished ones are history, and history kept for the life
/// of a daemon grows by a row per file served, forever.
pub const FINISHED_UPLOADS_KEPT: usize = 200;

/// A queued upload that has just been given a slot, ready to be offered to the
/// peer that asked for it.
#[derive(Debug, Clone)]
pub struct UploadOffer {
    pub requester_key: String,
    pub token: u32,
    pub virtual_path: String,
    pub size: u64,
}

/// A file a peer has asked us for and is waiting on.
#[derive(Debug, Clone)]
pub struct QueuedUpload {
    /// The asking peer's username — also the registry key of its actor.
    pub downloader: String,
    pub virtual_path: String,
    pub real_path: PathBuf,
    pub size: u64,
    /// Whether the server listed this user as privileged when they asked.
    pub privileged: bool,
    /// Arrival order, so the tie-break among equals is first-come.
    pub seq: u64,
    /// Whether this wait has been recorded in the upload events yet. Each is
    /// recorded once: re-recording the whole queue on every pump is what made
    /// a long queue quadratic to maintain.
    pub recorded: bool,
}

/// What decides the order: donors first, then whoever asked first.
const fn rank(job: &QueuedUpload) -> (bool, u64) {
    (!job.privileged, job.seq)
}

/// Indices of `queue` in the order they will be served.
#[must_use]
pub fn serve_order(queue: &[QueuedUpload]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..queue.len()).collect();
    order.sort_by_key(|&i| rank(&queue[i]));
    order
}

/// Which entry to serve next, or `None` when nobody is waiting.
#[must_use]
pub fn next_to_serve(queue: &[QueuedUpload]) -> Option<usize> {
    queue
        .iter()
        .enumerate()
        .min_by_key(|(_, job)| rank(job))
        .map(|(index, _)| index)
}

/// A queue position as the protocol reports it: counting from 1.
fn as_place(zero_based: usize) -> u32 {
    u32::try_from(zero_based + 1).unwrap_or(u32::MAX)
}

/// Where `filename` sits for `downloader`, counting from 1 as the protocol's
/// `PlaceInQueueResponse` does. `None` when that peer is not waiting for it,
/// which is also the answer once the file has been handed over.
#[must_use]
pub fn place_in_queue(
    queue: &[QueuedUpload],
    downloader: &str,
    filename: &str,
) -> Option<u32> {
    serve_order(queue)
        .into_iter()
        .position(|i| {
            queue[i].downloader == downloader
                && queue[i].virtual_path == filename
        })
        .map(as_place)
}

/// The queue as the client drives it: taking a request, filling slots, and
/// answering where a peer sits.
impl ClientContext {
    /// Take `filename` from a peer's request and put them in the queue,
    /// returning whether that added anything. The caller pumps afterwards;
    /// queueing and serving are separate so a request for a file we do not
    /// have never reaches the queue.
    ///
    /// A request for a file the peer is already waiting for, has been offered
    /// or is being sent changes nothing. Clients repeat `QueueUpload` as a
    /// matter of course — on reconnect, on a timer, on every retry — and each
    /// repeat used to become a row of its own: a daemon left running for a few
    /// hours held several rows per file and offered the same transfer again
    /// and again. The first request keeps its place in line.
    pub fn enqueue_upload(
        &mut self,
        downloader: &str,
        filename: &str,
        real_path: std::path::PathBuf,
        size: u64,
    ) -> bool {
        if self.is_upload_pending(downloader, filename) {
            return false;
        }
        // The new request is this peer's row for the file from here on; a
        // finished one left beside it would list the same upload twice. How
        // that one ended goes to the events, for a caller that samples the
        // list and has not seen it yet.
        let finished = self.active_uploads.values().filter(|upload| {
            upload.username == downloader
                && upload.filename == filename
                && upload.status.is_terminal()
        });
        self.upload_events.extend(finished.map(|upload| UploadInfo {
            username: upload.username.clone(),
            filename: upload.filename.clone(),
            size: upload.size,
            bytes_sent:
                upload.bytes_sent.load(std::sync::atomic::Ordering::Relaxed),
            speed_bytes_per_sec: 0.0,
            status: upload.status.clone(),
        }));
        self.cap_upload_events();
        self.remove_upload(downloader, filename);
        self.upload_seq += 1;
        self.upload_queue.push(QueuedUpload {
            downloader: downloader.to_string(),
            virtual_path: filename.to_string(),
            real_path,
            size,
            privileged: self.privileged_users.contains(downloader),
            seq: self.upload_seq,
            recorded: false,
        });
        true
    }

    /// Whether `downloader` already has `filename` waiting, offered or in
    /// flight. A finished transfer does not count: asking again after one is
    /// a new download.
    fn is_upload_pending(&self, downloader: &str, filename: &str) -> bool {
        self.upload_queue.iter().any(|job| {
            job.downloader == downloader && job.virtual_path == filename
        }) || self.uploads.values().any(|job| {
            job.downloader == downloader && job.virtual_path == filename
        }) || self.active_uploads.values().any(|upload| {
            upload.username == downloader
                && upload.filename == filename
                && matches!(upload.status, UploadStatus::InProgress)
        })
    }

    /// Move as many queued uploads into free slots as will fit, and return the
    /// offers to send. Ordering is privileged-first, then first-come.
    ///
    /// The registry comes back with them so the caller can send after dropping
    /// the context lock: a peer actor's mailbox is not something to hold it
    /// across.
    pub fn pump_uploads(
        &mut self,
        mut next_token: impl FnMut() -> u32,
    ) -> (Option<PeerRegistry>, Vec<UploadOffer>) {
        let mut offers = Vec::new();
        while self.has_free_upload_slot() {
            let Some(index) = next_to_serve(&self.upload_queue) else {
                break;
            };
            let job = self.upload_queue.remove(index);
            let token = next_token();
            offers.push(UploadOffer {
                requester_key: job.downloader.clone(),
                token,
                virtual_path: job.virtual_path.clone(),
                size: job.size,
            });
            self.uploads.insert(
                token,
                UploadJob {
                    downloader: job.downloader,
                    real_path: job.real_path,
                    virtual_path: job.virtual_path,
                    size: job.size,
                    deadline: Instant::now() + OFFER_TIMEOUT,
                    answered: false,
                },
            );
        }

        // Whatever is still waiting had to wait. Record it, because a caller
        // polling `uploads()` on an interval cannot see a peer that queued and
        // was served between two polls — and with a small slot count that is
        // the common case, not the rare one.
        if self.upload_queue.iter().any(|job| !job.recorded) {
            for (place, index) in
                serve_order(&self.upload_queue).into_iter().enumerate()
            {
                let job = &mut self.upload_queue[index];
                if job.recorded {
                    continue;
                }
                job.recorded = true;
                let info = queued_info(job, place);
                self.upload_events.push(info);
            }
        }
        self.cap_upload_events();

        (self.peer_registry.clone(), offers)
    }

    /// A caller that never drains must not grow the events without bound.
    fn cap_upload_events(&mut self) {
        if self.upload_events.len() > MAX_UPLOAD_EVENTS {
            let excess = self.upload_events.len() - MAX_UPLOAD_EVENTS;
            self.upload_events.drain(..excess);
        }
    }

    /// Take the queued-upload states recorded since the last call.
    pub fn take_upload_events(&mut self) -> Vec<UploadInfo> {
        std::mem::take(&mut self.upload_events)
    }

    /// Uploads occupying a slot: offered but not yet accepted, plus those
    /// actually streaming.
    ///
    /// `active_uploads` keeps finished transfers so `uploads()` can report them,
    /// so only the in-progress ones count — counting the whole table would let
    /// a few completed transfers wedge the queue shut forever.
    fn uploads_in_flight(&self) -> usize {
        self.uploads.len()
            + self
                .active_uploads
                .values()
                .filter(|upload| {
                    matches!(upload.status, UploadStatus::InProgress)
                })
                .count()
    }

    pub(super) fn has_free_upload_slot(&self) -> bool {
        self.uploads_in_flight() < self.upload_slots
    }

    /// The peers still waiting, in the order they will be served, each carrying
    /// its own 1-based place.
    pub(super) fn queued_uploads(&self) -> Vec<UploadInfo> {
        serve_order(&self.upload_queue)
            .into_iter()
            .enumerate()
            .map(|(place, index)| queued_info(&self.upload_queue[index], place))
            .collect()
    }

    /// Forget the finished uploads of `filename` to `downloader`. One still
    /// streaming stays, because it holds a slot until its thread ends.
    pub fn remove_upload(&mut self, downloader: &str, filename: &str) -> bool {
        let before = self.active_uploads.len();
        self.active_uploads.retain(|_, upload| {
            upload.username != downloader
                || upload.filename != filename
                || !upload.status.is_terminal()
        });
        self.active_uploads.len() != before
    }

    /// Withdraw everything `downloader` is waiting for or has been offered
    /// under `filename`, recording each as cancelled, and ask any transfer of
    /// it already streaming to stop. Returns whether a waiting or offered
    /// upload was withdrawn, which is the caller's cue to tell the peer, and
    /// whether anything matched at all.
    ///
    /// The streaming transfer frees its own slot when its thread notices the
    /// flag; the caller pumps for the rest.
    pub fn cancel_upload(
        &mut self,
        downloader: &str,
        filename: &str,
    ) -> (bool, bool) {
        let mut found = false;
        for upload in self.active_uploads.values() {
            if upload.username == downloader
                && upload.filename == filename
                && upload.status == UploadStatus::InProgress
            {
                upload
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                found = true;
            }
        }

        let mut size = None;
        self.upload_queue.retain(|job| {
            let matches =
                job.downloader == downloader && job.virtual_path == filename;
            if matches {
                size = Some(job.size);
            }
            !matches
        });
        let offered: Vec<u32> = self
            .uploads
            .iter()
            .filter(|(_, job)| {
                job.downloader == downloader && job.virtual_path == filename
            })
            .map(|(&token, _)| token)
            .collect();
        for token in offered {
            if let Some(job) = self.retire_offer(token) {
                size = Some(job.size);
            }
        }

        let withdrawn = size.is_some();
        if let Some(size) = size {
            found = true;
            self.upload_events.push(UploadInfo {
                username: downloader.to_string(),
                filename: filename.to_string(),
                size,
                bytes_sent: 0,
                speed_bytes_per_sec: 0.0,
                status: UploadStatus::Cancelled,
            });
            self.cap_upload_events();
        }
        (withdrawn, found)
    }

    /// Stop waiting on `downloader`'s address on behalf of upload `token`.
    fn forget_pending_serve(&mut self, downloader: &str, token: u32) {
        if let Some(tokens) = self.pending_serves.get_mut(downloader) {
            tokens.retain(|waiting| *waiting != token);
            if tokens.is_empty() {
                self.pending_serves.remove(downloader);
            }
        }
    }

    /// Start tracking a transfer that is about to stream, replacing whatever
    /// finished row the same peer and file left behind and letting the oldest
    /// finished rows fall off the end.
    pub(super) fn track_active_upload(
        &mut self,
        token: u32,
        upload: ActiveUpload,
    ) {
        self.active_uploads.retain(|_, other| {
            matches!(other.status, UploadStatus::InProgress)
                || other.username != upload.username
                || other.filename != upload.filename
        });
        self.active_uploads.insert(token, upload);
        prune_finished(&mut self.active_uploads, FINISHED_UPLOADS_KEPT);
    }

    /// Where `downloader`'s queued `filename` sits, counting from 1. `None`
    /// once it has left the queue for a slot.
    #[must_use]
    pub fn place_in_queue(
        &self,
        downloader: &str,
        filename: &str,
    ) -> Option<u32> {
        place_in_queue(&self.upload_queue, downloader, filename)
    }

    /// Replace the privileged set, re-ranking anyone already waiting: the list
    /// arrives at login, which can be after a peer has queued something.
    pub fn set_privileged_users(&mut self, users: Vec<String>) {
        self.privileged_users = users.into_iter().collect();
        for job in &mut self.upload_queue {
            job.privileged = self.privileged_users.contains(&job.downloader);
        }
    }

    #[must_use]
    pub fn is_privileged(&self, username: &str) -> bool {
        self.privileged_users.contains(username)
    }

    /// Give up on an offer: forget it here and note it for the peer's actor,
    /// which keeps its own record of the tokens it may be answered on.
    fn retire_offer(&mut self, token: u32) -> Option<UploadJob> {
        let job = self.uploads.remove(&token)?;
        self.forget_pending_serve(&job.downloader, token);
        self.retired_offers.push((job.downloader.clone(), token));
        Some(job)
    }

    /// Drop offers nobody answered within [`OFFER_TIMEOUT`], and accepted ones
    /// that never got as far as streaming within [`ACCEPTED_TIMEOUT`],
    /// recording each as a failed upload so an operator sees why the peer
    /// never got its file. Returns whether anything was dropped, which is the
    /// caller's cue to pump again.
    pub fn expire_stale_offers(&mut self, now: Instant) -> bool {
        let stale: Vec<u32> = self
            .uploads
            .iter()
            .filter(|(_, job)| now >= job.deadline)
            .map(|(&token, _)| token)
            .collect();
        for token in &stale {
            let Some(job) = self.retire_offer(*token) else {
                continue;
            };
            let reason = if job.answered {
                "the transfer never started"
            } else {
                "no answer to the transfer offer"
            };
            self.upload_events.push(UploadInfo {
                username: job.downloader,
                filename: job.virtual_path,
                size: job.size,
                bytes_sent: 0,
                speed_bytes_per_sec: 0.0,
                status: UploadStatus::Failed(reason.to_string()),
            });
        }
        self.cap_upload_events();
        !stale.is_empty()
    }

    /// The peer turned down the offer for `token`, which frees its slot.
    /// Returns whether the offer was still open.
    pub fn refuse_offer(&mut self, token: u32, reason: Option<&str>) -> bool {
        let Some(job) = self.uploads.remove(&token) else {
            return false;
        };
        self.forget_pending_serve(&job.downloader, token);
        let status = match reason {
            None | Some("Cancelled") => UploadStatus::Cancelled,
            Some(reason) => UploadStatus::Failed(reason.to_string()),
        };
        self.upload_events.push(UploadInfo {
            username: job.downloader,
            filename: job.virtual_path,
            size: job.size,
            bytes_sent: 0,
            speed_bytes_per_sec: 0.0,
            status,
        });
        self.cap_upload_events();
        true
    }

    /// The peer accepted the offer for `token`. Resolving their address and
    /// connecting can outlast the offer window, so the clock restarts with the
    /// longer allowance — restarts rather than stops, because an address
    /// lookup lost to a dropped session would otherwise hold the slot forever.
    pub fn mark_offer_answered(&mut self, token: u32) {
        if let Some(job) = self.uploads.get_mut(&token) {
            job.answered = true;
            job.deadline = Instant::now() + ACCEPTED_TIMEOUT;
        }
    }

    /// Forget everything `downloader` was waiting for or had been offered, and
    /// report whether that freed anything.
    ///
    /// An offer sits in `uploads` counting against the slot cap until the peer
    /// accepts it. A peer that never will — because it has gone — would hold
    /// that slot for the life of the process, so leaving is what releases it.
    pub fn release_upload_slots(&mut self, downloader: &str) -> bool {
        let before = self.upload_queue.len() + self.uploads.len();
        self.upload_queue.retain(|job| job.downloader != downloader);
        self.uploads.retain(|_, job| job.downloader != downloader);
        self.pending_serves.remove(downloader);
        before != self.upload_queue.len() + self.uploads.len()
    }
}

/// A waiting upload as it is reported, `place` counting from zero.
fn queued_info(job: &QueuedUpload, place: usize) -> UploadInfo {
    UploadInfo {
        username: job.downloader.clone(),
        filename: job.virtual_path.clone(),
        size: job.size,
        bytes_sent: 0,
        speed_bytes_per_sec: 0.0,
        status: UploadStatus::Queued(as_place(place)),
    }
}

/// Forget the oldest finished uploads beyond `keep`. Tokens are handed out in
/// order, so the lowest are the oldest; transfers still running are never
/// touched.
fn prune_finished(uploads: &mut HashMap<u32, ActiveUpload>, keep: usize) {
    let mut finished: Vec<u32> = uploads
        .iter()
        .filter(|(_, upload)| {
            !matches!(upload.status, UploadStatus::InProgress)
        })
        .map(|(&token, _)| token)
        .collect();
    if finished.len() <= keep {
        return;
    }
    finished.sort_unstable();
    for token in &finished[..finished.len() - keep] {
        uploads.remove(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(user: &str, privileged: bool, seq: u64) -> QueuedUpload {
        QueuedUpload {
            downloader: user.to_string(),
            virtual_path: format!("@@share\\{user}.mp3"),
            real_path: PathBuf::from("/tmp/x.mp3"),
            size: 1,
            privileged,
            seq,
            recorded: false,
        }
    }

    #[test]
    fn an_empty_queue_serves_nobody() {
        assert_eq!(next_to_serve(&[]), None);
    }

    #[test]
    fn equals_are_served_in_the_order_they_asked() {
        let queue = [queued("amy", false, 1), queued("bob", false, 2)];
        assert_eq!(next_to_serve(&queue), Some(0));
        assert_eq!(serve_order(&queue), [0, 1]);
    }

    #[test]
    fn a_privileged_peer_is_served_before_one_that_asked_first() {
        let queue = [
            queued("early_plain", false, 1),
            queued("late_donor", true, 2),
        ];
        assert_eq!(
            next_to_serve(&queue),
            Some(1),
            "the donor jumps the plain user who was already waiting"
        );
    }

    #[test]
    fn privileged_peers_keep_first_come_order_among_themselves() {
        let queue = [
            queued("donor_late", true, 9),
            queued("plain", false, 2),
            queued("donor_early", true, 5),
        ];
        let order: Vec<&str> = serve_order(&queue)
            .into_iter()
            .map(|i| queue[i].downloader.as_str())
            .collect();
        assert_eq!(order, ["donor_early", "donor_late", "plain"]);
    }

    #[test]
    fn the_reported_place_counts_from_one() {
        let queue = [queued("amy", false, 1), queued("bob", false, 2)];
        assert_eq!(place_in_queue(&queue, "amy", "@@share\\amy.mp3"), Some(1));
        assert_eq!(place_in_queue(&queue, "bob", "@@share\\bob.mp3"), Some(2));
    }

    #[test]
    fn the_reported_place_reflects_being_overtaken() {
        let mut queue = vec![queued("plain", false, 1)];
        assert_eq!(
            place_in_queue(&queue, "plain", "@@share\\plain.mp3"),
            Some(1)
        );
        queue.push(queued("donor", true, 2));
        assert_eq!(
            place_in_queue(&queue, "plain", "@@share\\plain.mp3"),
            Some(2),
            "a peer that got overtaken must be told so, not left on 1"
        );
    }

    #[test]
    fn a_file_nobody_queued_has_no_place() {
        let queue = [queued("amy", false, 1)];
        assert_eq!(place_in_queue(&queue, "amy", "@@share\\other.mp3"), None);
        assert_eq!(place_in_queue(&queue, "zoe", "@@share\\amy.mp3"), None);
    }
}

/// Tests of the queue as the client actually drives it: what a request, a
/// privileged-user list, and a finished transfer do to a real
/// [`super::ClientContext`].
///
/// These sit here rather than in the e2e suite because a Soulseek server only
/// ever names privileged users it knows about, and the soulfind used for e2e
/// declares nobody — so the one behaviour the rules are about would otherwise
/// go untested.
#[cfg(test)]
mod context_tests {
    use super::{ACCEPTED_TIMEOUT, FINISHED_UPLOADS_KEPT, OFFER_TIMEOUT};
    use crate::client::{ActiveUpload, ClientContext, DEFAULT_UPLOAD_SLOTS};
    use crate::types::UploadStatus;
    use std::path::PathBuf;

    /// A context with `slots` upload slots and a token counter that hands out
    /// 1, 2, 3… so an assertion can name a token.
    fn context(slots: usize) -> (ClientContext, impl FnMut() -> u32) {
        let mut ctx = ClientContext::new();
        ctx.upload_slots = slots;
        let mut next = 0;
        (ctx, move || {
            next += 1;
            next
        })
    }

    fn ask(ctx: &mut ClientContext, user: &str, file: &str) {
        ctx.enqueue_upload(user, file, PathBuf::from("/tmp/x"), 4096);
    }

    /// Who the pump offered to, by the order it offered them.
    fn offered(
        ctx: &mut ClientContext,
        token: &mut impl FnMut() -> u32,
    ) -> Vec<String> {
        ctx.pump_uploads(token)
            .1
            .into_iter()
            .map(|offer| offer.requester_key)
            .collect()
    }

    fn active(user: &str, file: &str, status: UploadStatus) -> ActiveUpload {
        ActiveUpload {
            username: user.to_string(),
            filename: file.to_string(),
            size: 4096,
            bytes_sent: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
                0,
            )),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                false,
            )),
            status,
            started: std::time::Instant::now(),
        }
    }

    /// Move the offer under `token` on to streaming, as `spawn_serve` does.
    fn start_streaming(ctx: &mut ClientContext, token: u32) {
        let job = ctx.uploads.remove(&token).expect("the offer exists");
        ctx.track_active_upload(
            token,
            active(
                &job.downloader,
                &job.virtual_path,
                UploadStatus::InProgress,
            ),
        );
    }

    // Issue #112: clients repeat QueueUpload routinely, and every repeat used
    // to become a row — 8,013 rows for 2,131 distinct requests after ten
    // hours, one of them listed 76 times.
    #[test]
    fn a_repeated_request_keeps_one_row_and_its_place() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "blocker", "f.mp3");
        offered(&mut ctx, &mut token);
        ask(&mut ctx, "amy", "a.mp3");
        ask(&mut ctx, "bob", "b.mp3");

        for _ in 0..75 {
            assert!(
                !ctx.enqueue_upload(
                    "amy",
                    "a.mp3",
                    PathBuf::from("/tmp/x"),
                    4096
                ),
                "asking again must not add a row"
            );
        }

        assert_eq!(ctx.queued_uploads().len(), 2);
        assert_eq!(
            ctx.place_in_queue("amy", "a.mp3"),
            Some(1),
            "asking again must not cost the peer its place either"
        );
        assert_eq!(ctx.place_in_queue("bob", "b.mp3"), Some(2));
    }

    #[test]
    fn the_same_file_for_another_peer_or_another_file_still_queues() {
        let (mut ctx, _) = context(1);
        ask(&mut ctx, "amy", "a.mp3");
        ask(&mut ctx, "bob", "a.mp3");
        ask(&mut ctx, "amy", "b.mp3");
        assert_eq!(ctx.upload_queue.len(), 3);
    }

    #[test]
    fn a_request_for_a_transfer_already_under_way_is_not_queued_again() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "amy", "a.mp3");
        offered(&mut ctx, &mut token);

        ask(&mut ctx, "amy", "a.mp3");
        assert!(
            ctx.upload_queue.is_empty(),
            "an offer is out for it: a second row would send the file twice"
        );

        start_streaming(&mut ctx, 1);
        ask(&mut ctx, "amy", "a.mp3");
        assert!(ctx.upload_queue.is_empty(), "it is being sent right now");
    }

    #[test]
    fn asking_again_after_the_transfer_finished_is_a_new_request() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "amy", "a.mp3");
        offered(&mut ctx, &mut token);
        start_streaming(&mut ctx, 1);
        ctx.active_uploads.get_mut(&1).expect("tracked").status =
            UploadStatus::Completed;

        ask(&mut ctx, "amy", "a.mp3");
        assert_eq!(offered(&mut ctx, &mut token), ["amy"]);
    }

    #[test]
    fn cancelling_removes_a_queued_row_and_says_so() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "blocker", "f.mp3");
        offered(&mut ctx, &mut token);
        ask(&mut ctx, "amy", "a.mp3");
        ask(&mut ctx, "bob", "b.mp3");
        let _ = ctx.pump_uploads(&mut token);
        let _ = ctx.take_upload_events();

        let (withdrawn, found) = ctx.cancel_upload("amy", "a.mp3");

        assert!(found, "a queued row is something to cancel");
        assert!(withdrawn, "the peer is owed a word about it");
        assert_eq!(ctx.place_in_queue("amy", "a.mp3"), None);
        assert_eq!(
            ctx.place_in_queue("bob", "b.mp3"),
            Some(1),
            "whoever was behind moves up"
        );
        assert!(ctx.take_upload_events().iter().any(|event| {
            event.username == "amy"
                && event.filename == "a.mp3"
                && event.status == UploadStatus::Cancelled
        }));
    }

    #[test]
    fn cancelling_an_offer_frees_its_slot() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "amy", "a.mp3");
        ask(&mut ctx, "bob", "b.mp3");
        offered(&mut ctx, &mut token);

        let (withdrawn, found) = ctx.cancel_upload("amy", "a.mp3");

        assert!(found);
        assert!(withdrawn);
        assert_eq!(offered(&mut ctx, &mut token), ["bob"]);
    }

    #[test]
    fn cancelling_a_streaming_upload_raises_its_flag() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "amy", "a.mp3");
        offered(&mut ctx, &mut token);
        start_streaming(&mut ctx, 1);

        assert_eq!(
            ctx.cancel_upload("amy", "a.mp3"),
            (false, true),
            "found, but nothing left the queue"
        );
        assert!(
            ctx.active_uploads[&1]
                .cancel
                .load(std::sync::atomic::Ordering::Relaxed)
        );
    }

    #[test]
    fn cancelling_something_unknown_finds_nothing() {
        let (mut ctx, _) = context(1);
        ask(&mut ctx, "amy", "a.mp3");
        assert!(!ctx.cancel_upload("amy", "other.mp3").1);
        assert!(!ctx.cancel_upload("zoe", "a.mp3").1);
        assert_eq!(ctx.upload_queue.len(), 1);
    }

    // The address lookup that follows an accepted offer goes through the
    // server; one lost to a dropped session never answers, and that upload
    // used to hold its slot for the life of the process.
    #[test]
    fn an_accepted_offer_that_never_starts_gives_its_slot_back() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "stuck", "f.mp3");
        ask(&mut ctx, "next", "f.mp3");
        offered(&mut ctx, &mut token);
        ctx.mark_offer_answered(1);
        ctx.pending_serves.insert("stuck".to_string(), vec![1]);

        let later = std::time::Instant::now() + ACCEPTED_TIMEOUT;
        assert!(ctx.expire_stale_offers(later));
        assert!(
            ctx.pending_serves.is_empty(),
            "nothing is left waiting on that peer's address"
        );
        assert_eq!(offered(&mut ctx, &mut token), ["next"]);
    }

    #[test]
    fn removing_a_finished_upload_preserves_a_matching_stream() {
        let (mut ctx, _) = context(2);
        ctx.active_uploads
            .insert(1, active("amy", "a.mp3", UploadStatus::Completed));
        ctx.active_uploads
            .insert(2, active("amy", "a.mp3", UploadStatus::InProgress));

        assert!(!ctx.remove_upload("bob", "a.mp3"));
        assert!(!ctx.remove_upload("amy", "b.mp3"));
        assert!(ctx.remove_upload("amy", "a.mp3"));
        assert!(!ctx.active_uploads.contains_key(&1));
        assert_eq!(ctx.active_uploads[&2].status, UploadStatus::InProgress);
        assert!(!ctx.remove_upload("amy", "a.mp3"));
    }

    #[test]
    fn finished_uploads_are_kept_only_up_to_a_limit() {
        let (mut ctx, _) = context(2);
        let served = FINISHED_UPLOADS_KEPT as u32 + 50;
        for token in 0..served {
            ctx.track_active_upload(
                token,
                active("amy", &format!("{token}.mp3"), UploadStatus::Completed),
            );
        }
        ctx.track_active_upload(
            served,
            active("amy", "live.mp3", UploadStatus::InProgress),
        );

        assert_eq!(ctx.active_uploads.len(), FINISHED_UPLOADS_KEPT + 1);
        assert!(
            !ctx.active_uploads.contains_key(&0),
            "the oldest history goes first"
        );
        assert!(ctx.active_uploads.contains_key(&(served - 1)));
        assert!(
            ctx.active_uploads.contains_key(&served),
            "a running transfer is never pruned"
        );
    }

    #[test]
    fn serving_a_file_again_replaces_its_finished_row() {
        let (mut ctx, _) = context(2);
        ctx.track_active_upload(
            1,
            active("amy", "a.mp3", UploadStatus::Completed),
        );
        ctx.track_active_upload(
            2,
            active("amy", "a.mp3", UploadStatus::InProgress),
        );
        assert_eq!(
            ctx.active_uploads.keys().copied().collect::<Vec<_>>(),
            [2],
            "one row per peer and file, the current one"
        );
    }

    // A peer retrying a file left its finished row beside the queued one, and
    // `serve` printed the two in turn on every poll until the peer was served.
    #[test]
    fn asking_again_for_a_finished_file_replaces_its_row() {
        let (mut ctx, _) = context(1);
        ctx.track_active_upload(
            1,
            active("amy", "a.mp3", UploadStatus::Failed("gone".to_string())),
        );

        ask(&mut ctx, "amy", "a.mp3");

        assert!(
            ctx.active_uploads.is_empty(),
            "one row per peer and file, the current one"
        );
        assert_eq!(ctx.queued_uploads().len(), 1);
        assert!(
            ctx.take_upload_events().iter().any(|event| {
                event.username == "amy"
                    && event.filename == "a.mp3"
                    && event.status == UploadStatus::Failed("gone".to_string())
            }),
            "how it ended still reaches a caller that had not sampled it"
        );
    }

    #[test]
    fn a_wait_is_recorded_once_however_often_the_queue_is_pumped() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "blocker", "f.mp3");
        ask(&mut ctx, "amy", "a.mp3");
        let _ = ctx.pump_uploads(&mut token);
        assert_eq!(ctx.take_upload_events().len(), 1);

        for _ in 0..5 {
            let _ = ctx.pump_uploads(&mut token);
        }
        assert!(
            ctx.take_upload_events().is_empty(),
            "an unchanged queue has nothing new to say"
        );
    }

    #[test]
    fn a_fresh_context_starts_with_the_default_cap() {
        assert_eq!(ClientContext::new().upload_slots, DEFAULT_UPLOAD_SLOTS);
    }

    #[test]
    fn the_pump_fills_the_slots_and_leaves_the_rest_queued() {
        let (mut ctx, mut token) = context(2);
        for user in ["a", "b", "c"] {
            ask(&mut ctx, user, "f.mp3");
        }

        assert_eq!(offered(&mut ctx, &mut token), ["a", "b"]);
        assert_eq!(
            ctx.place_in_queue("c", "f.mp3"),
            Some(1),
            "the third asker waits, now at the front of the queue"
        );
        assert!(
            offered(&mut ctx, &mut token).is_empty(),
            "pumping again must not oversubscribe the slots"
        );
    }

    #[test]
    fn a_privileged_list_arriving_late_re_ranks_who_is_waiting() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "blocker", "f.mp3");
        ask(&mut ctx, "plain", "f.mp3");
        ask(&mut ctx, "donor", "f.mp3");
        assert_eq!(offered(&mut ctx, &mut token), ["blocker"]);

        // Before the server has said anything, first-come holds.
        assert_eq!(ctx.place_in_queue("plain", "f.mp3"), Some(1));
        assert_eq!(ctx.place_in_queue("donor", "f.mp3"), Some(2));

        ctx.set_privileged_users(vec!["donor".to_string()]);

        assert!(ctx.is_privileged("donor"));
        assert!(!ctx.is_privileged("plain"));
        assert_eq!(
            ctx.place_in_queue("donor", "f.mp3"),
            Some(1),
            "the donor should overtake the peer that asked first"
        );
        assert_eq!(
            ctx.place_in_queue("plain", "f.mp3"),
            Some(2),
            "and the plain peer should be told they were overtaken"
        );
    }

    #[test]
    fn the_privileged_peer_is_the_one_served_next() {
        let (mut ctx, mut token) = context(1);
        ctx.set_privileged_users(vec!["donor".to_string()]);

        // Someone else already holds the only slot: a transfer in progress is
        // never preempted, so the donor's turn is the *next* one.
        ask(&mut ctx, "blocker", "f.mp3");
        assert_eq!(offered(&mut ctx, &mut token), ["blocker"]);

        ask(&mut ctx, "plain", "f.mp3");
        ask(&mut ctx, "donor", "f.mp3");

        ctx.uploads.clear(); // the blocker's transfer finished
        assert_eq!(
            offered(&mut ctx, &mut token),
            ["donor"],
            "the freed slot goes to the donor, not the longer-waiting peer"
        );
    }

    #[test]
    fn a_finished_transfer_frees_its_slot_for_the_next_in_line() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "first", "f.mp3");
        ask(&mut ctx, "second", "f.mp3");
        assert_eq!(offered(&mut ctx, &mut token), ["first"]);

        ctx.uploads.clear();
        assert_eq!(offered(&mut ctx, &mut token), ["second"]);
        assert_eq!(ctx.place_in_queue("second", "f.mp3"), None);
    }

    #[test]
    fn an_offer_nobody_answers_frees_its_slot_after_the_timeout() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "gone", "f.mp3");
        ask(&mut ctx, "next", "f.mp3");
        assert_eq!(offered(&mut ctx, &mut token), ["gone"]);

        let now = std::time::Instant::now();
        assert!(!ctx.expire_stale_offers(now), "a fresh offer stays");
        assert!(
            ctx.expire_stale_offers(now + OFFER_TIMEOUT),
            "an unanswered offer expires"
        );
        assert_eq!(offered(&mut ctx, &mut token), ["next"]);
        assert!(ctx.take_upload_events().iter().any(|event| {
            event.username == "gone"
                && matches!(event.status, UploadStatus::Failed(_))
        }));
    }

    #[test]
    fn an_accepted_offer_waiting_on_an_address_does_not_expire() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "slow", "f.mp3");
        offered(&mut ctx, &mut token);

        ctx.mark_offer_answered(1);
        let later = std::time::Instant::now() + OFFER_TIMEOUT * 2;
        assert!(!ctx.expire_stale_offers(later), "an answered offer stays");
        assert!(ctx.uploads.contains_key(&1));
    }

    #[test]
    fn completed_transfers_do_not_keep_occupying_slots() {
        // active_uploads keeps finished transfers so `uploads()` can report
        // them; counting those would wedge the queue shut for the session.
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "first", "f.mp3");
        offered(&mut ctx, &mut token);

        let job = ctx.uploads.remove(&1).expect("the offer was recorded");
        ctx.active_uploads.insert(
            1,
            active(&job.downloader, &job.virtual_path, UploadStatus::Completed),
        );

        ask(&mut ctx, "second", "f.mp3");
        assert_eq!(
            offered(&mut ctx, &mut token),
            ["second"],
            "a completed upload must not hold its slot"
        );
    }

    #[test]
    fn queued_uploads_are_reported_in_the_order_they_will_be_served() {
        let (mut ctx, mut token) = context(1);
        ctx.set_privileged_users(vec!["donor".to_string()]);
        ask(&mut ctx, "blocker", "f.mp3");
        offered(&mut ctx, &mut token);
        ask(&mut ctx, "plain", "f.mp3");
        ask(&mut ctx, "donor", "f.mp3");

        let reported: Vec<(String, UploadStatus)> = ctx
            .queued_uploads()
            .into_iter()
            .map(|upload| (upload.username, upload.status))
            .collect();
        assert_eq!(
            reported,
            [
                ("donor".to_string(), UploadStatus::Queued(1)),
                ("plain".to_string(), UploadStatus::Queued(2)),
            ]
        );
    }

    // A peer that waits and is then served inside one poll interval must still
    // be reportable as queued: `serve` samples on an interval, so a state that
    // only ever existed between two samples would otherwise vanish from the
    // transfer log entirely.
    #[test]
    fn a_queued_upload_is_recorded_even_after_it_leaves_the_queue() {
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "amy", "@@share\\amy.mp3");
        ask(&mut ctx, "bob", "@@share\\bob.mp3");

        // One slot, so bob waits behind amy.
        let _ = ctx.pump_uploads(&mut token);
        assert!(
            ctx.queued_uploads().iter().any(|u| u.username == "bob"),
            "bob should be waiting while amy holds the only slot"
        );

        // Amy finishes and bob is served, so nothing is waiting any more...
        ctx.uploads.clear();
        ctx.active_uploads.clear();
        let _ = ctx.pump_uploads(&mut token);
        assert!(
            ctx.queued_uploads().is_empty(),
            "the queue should have drained"
        );

        // ...but the fact that bob waited survives for whoever asks next.
        let events = ctx.take_upload_events();
        assert!(
            events.iter().any(|u| u.username == "bob"
                && matches!(u.status, UploadStatus::Queued(_))),
            "bob's wait should still be reportable, got {events:?}"
        );
        assert!(
            ctx.take_upload_events().is_empty(),
            "draining twice must not repeat the same events"
        );
    }
}
