//! Coalesce notifications without discarding changes received during a paint.
use std::sync::atomic::{AtomicU32, Ordering};

const QUEUED: u32 = 1;
const DIRTY: u32 = 2;
const FORCE: u32 = 4;

pub struct Requests(AtomicU32);

impl Requests {
    pub const fn new() -> Self {
        Self(AtomicU32::new(0))
    }

    /// Returns whether the caller owns posting the next window message.
    pub fn request(&self, force: bool) -> bool {
        self.0.fetch_or(
            QUEUED | DIRTY | if force { FORCE } else { 0 },
            Ordering::SeqCst,
        ) & QUEUED
            == 0
    }

    /// Snapshot this batch, keeping ownership until its painting finishes.
    pub fn begin(&self) -> bool {
        self.0.fetch_and(QUEUED, Ordering::SeqCst) & FORCE != 0
    }

    pub fn finish(&self) -> bool {
        // A racing request either leaves DIRTY for us or observes an idle
        // queue and posts its own message. It cannot fall between both paths.
        loop {
            let state = self.0.load(Ordering::SeqCst);
            if state & DIRTY != 0 {
                return true;
            }
            if self
                .0
                .compare_exchange(state, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return false;
            }
        }
    }

    pub fn post_failed(&self) {
        self.0.fetch_and(!QUEUED, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub fn is_idle(&self) -> bool {
        self.0.load(Ordering::SeqCst) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_during_paint_and_after_completion_both_get_a_message() {
        let queue = Requests::new();
        assert!(queue.request(false));
        assert!(!queue.request(false));
        assert!(!queue.begin());
        assert!(!queue.request(false)); // An external change during painting.
        assert!(queue.finish());
        assert!(!queue.begin());
        assert!(!queue.finish());
        assert!(queue.is_idle());
        assert!(queue.request(false)); // Or after completion, before returning.
        assert!(!queue.begin());
        assert!(!queue.finish());
    }

    #[test]
    fn repair_and_failed_posts_preserve_unprocessed_work() {
        let queue = Requests::new();
        assert!(queue.request(false));
        assert!(!queue.begin());
        assert!(!queue.request(true));
        assert!(queue.finish());
        queue.post_failed();
        assert!(queue.request(false));
        assert!(queue.begin());
        assert!(!queue.finish());
        assert!(queue.is_idle());
    }
}
