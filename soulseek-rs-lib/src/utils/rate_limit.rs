use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(100);
const TICKS_PER_SECOND: u64 = 1000 / TICK.as_millis() as u64;

#[derive(Default)]
pub struct RateLimit {
    bytes_per_second: AtomicU64,
    bucket: Mutex<Bucket>,
}

#[derive(Default)]
struct Bucket {
    tokens: u64,
    refilled: Option<Instant>,
}

impl RateLimit {
    pub fn set(&self, bytes_per_second: u64) {
        self.bytes_per_second
            .store(bytes_per_second, Ordering::Relaxed);
    }

    pub fn take(&self, want: usize) -> usize {
        loop {
            let rate = self.bytes_per_second.load(Ordering::Relaxed);
            if rate == 0 {
                return want;
            }
            let mut bucket =
                self.bucket.lock().unwrap_or_else(PoisonError::into_inner);
            let now = Instant::now();
            let since = bucket.refilled.map_or(TICK, |at| now - at);
            if since >= TICK {
                bucket.tokens = (rate / TICKS_PER_SECOND).max(1);
                bucket.refilled = Some(now);
            }
            if bucket.tokens > 0 {
                let grant = bucket.tokens.min(want as u64);
                bucket.tokens -= grant;
                return grant as usize;
            }
            drop(bucket);
            std::thread::sleep(TICK.saturating_sub(since));
        }
    }

    pub fn give_back(&self, unused: usize) {
        if unused == 0 || self.bytes_per_second.load(Ordering::Relaxed) == 0 {
            return;
        }
        self.bucket
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .tokens += unused as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_limit_grants_everything_at_once() {
        let limit = RateLimit::default();
        let started = Instant::now();
        assert_eq!(limit.take(1 << 20), 1 << 20);
        assert!(started.elapsed() < TICK);
    }

    #[test]
    fn a_limit_paces_what_is_taken() {
        let limit = RateLimit::default();
        limit.set(20_000);
        let started = Instant::now();
        let mut taken = 0;
        while taken < 6_000 {
            let grant = limit.take(4096);
            assert!((1..=4096).contains(&grant), "granted {grant}");
            taken += grant;
        }
        let elapsed = started.elapsed();
        assert!(
            (2 * TICK..10 * TICK).contains(&elapsed),
            "{taken} bytes at 20 kB/s took {elapsed:?}"
        );
    }

    #[test]
    fn concurrent_takers_share_one_budget() {
        let limit = RateLimit::default();
        limit.set(20_000);
        let started = Instant::now();
        std::thread::scope(|s| {
            for _ in 0..3 {
                s.spawn(|| {
                    let mut taken = 0;
                    while taken < 2_000 {
                        taken += limit.take(4096);
                    }
                });
            }
        });
        assert!(
            started.elapsed() >= 2 * TICK,
            "6 kB at 20 kB/s across three takers took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn unused_allowance_goes_back_to_the_budget() {
        let limit = RateLimit::default();
        limit.set(20_000);
        let started = Instant::now();
        let grant = limit.take(4096);
        limit.give_back(grant);
        assert_eq!(limit.take(4096), grant);
        assert!(started.elapsed() < TICK);
    }

    #[test]
    fn lifting_a_limit_frees_a_take_that_is_waiting() {
        let limit = RateLimit::default();
        limit.set(100);
        assert_eq!(limit.take(1000), 10);
        std::thread::scope(|s| {
            let waiting = s.spawn(|| limit.take(1000));
            std::thread::sleep(Duration::from_millis(10));
            limit.set(0);
            assert_eq!(waiting.join().unwrap(), 1000);
        });
    }
}
