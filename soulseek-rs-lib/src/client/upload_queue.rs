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

use super::{ClientContext, PeerRegistry, UploadJob};
use crate::types::{UploadInfo, UploadStatus};
use std::path::PathBuf;

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
    /// Registry key of the asking peer's actor, which may carry a `:direct`
    /// suffix — not the same string as `downloader`.
    pub requester_key: String,
    pub downloader: String,
    pub virtual_path: String,
    pub real_path: PathBuf,
    pub size: u64,
    /// Whether the server listed this user as privileged when they asked.
    pub privileged: bool,
    /// Arrival order, so the tie-break among equals is first-come.
    pub seq: u64,
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
    /// Take `filename` from a peer's request and put them in the queue. The
    /// caller pumps afterwards; queueing and serving are separate so a request
    /// for a file we do not have never reaches the queue.
    pub fn enqueue_upload(
        &mut self,
        requester_key: &str,
        downloader: &str,
        filename: &str,
        real_path: std::path::PathBuf,
        size: u64,
    ) {
        self.upload_seq += 1;
        self.upload_queue.push(QueuedUpload {
            requester_key: requester_key.to_string(),
            downloader: downloader.to_string(),
            virtual_path: filename.to_string(),
            real_path,
            size,
            privileged: self.privileged_users.contains(downloader),
            seq: self.upload_seq,
        });
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
        while self.uploads_in_flight() < self.upload_slots {
            let Some(index) = next_to_serve(&self.upload_queue) else {
                break;
            };
            let job = self.upload_queue.remove(index);
            let token = next_token();
            offers.push(UploadOffer {
                requester_key: job.requester_key,
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
                },
            );
        }
        (self.peer_registry.clone(), offers)
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

    /// The peers still waiting, in the order they will be served, each carrying
    /// its own 1-based place.
    pub(super) fn queued_uploads(&self) -> Vec<UploadInfo> {
        serve_order(&self.upload_queue)
            .into_iter()
            .enumerate()
            .map(|(place, index)| {
                let job = &self.upload_queue[index];
                UploadInfo {
                    username: job.downloader.clone(),
                    filename: job.virtual_path.clone(),
                    size: job.size,
                    bytes_sent: 0,
                    speed_bytes_per_sec: 0.0,
                    status: UploadStatus::Queued(as_place(place)),
                }
            })
            .collect()
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
        before != self.upload_queue.len() + self.uploads.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(user: &str, privileged: bool, seq: u64) -> QueuedUpload {
        QueuedUpload {
            requester_key: user.to_string(),
            downloader: user.to_string(),
            virtual_path: format!("@@share\\{user}.mp3"),
            real_path: PathBuf::from("/tmp/x.mp3"),
            size: 1,
            privileged,
            seq,
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
    use crate::client::{ClientContext, DEFAULT_UPLOAD_SLOTS};
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
        ctx.enqueue_upload(user, user, file, PathBuf::from("/tmp/x"), 4096);
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
    fn completed_transfers_do_not_keep_occupying_slots() {
        // active_uploads keeps finished transfers so `uploads()` can report
        // them; counting those would wedge the queue shut for the session.
        let (mut ctx, mut token) = context(1);
        ask(&mut ctx, "first", "f.mp3");
        offered(&mut ctx, &mut token);

        let job = ctx.uploads.remove(&1).expect("the offer was recorded");
        ctx.active_uploads.insert(
            1,
            crate::client::ActiveUpload {
                username: job.downloader,
                filename: job.virtual_path,
                size: job.size,
                bytes_sent: std::sync::Arc::new(
                    std::sync::atomic::AtomicU64::new(job.size),
                ),
                cancel: std::sync::Arc::new(
                    std::sync::atomic::AtomicBool::new(false),
                ),
                status: UploadStatus::Completed,
                started: std::time::Instant::now(),
            },
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
}
