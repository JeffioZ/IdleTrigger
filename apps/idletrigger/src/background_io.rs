//! Bounded, cancellable preparation for potentially slow system/file reads.
//! A timed-out native read may finish later, so callbacks must be read-only:
//! no config publication, UI, or system mutations are allowed in them.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

// A stuck network provider must not accumulate a new thread on every retry.
static READING: AtomicBool = AtomicBool::new(false);
struct ReadGuard;
impl Drop for ReadGuard {
    fn drop(&mut self) {
        READING.store(false, Ordering::SeqCst);
    }
}

pub fn ensure_running() -> Result<(), String> {
    if crate::EXITING.load(Ordering::SeqCst) {
        Err("application is exiting".into())
    } else {
        Ok(())
    }
}

pub fn read<T: Send + 'static>(
    label: &'static str,
    task: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    ensure_running()?;
    if READING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(format!(
            "{label}: a previous system read is still finishing"
        ));
    }
    let guard = ReadGuard;
    let (send, receive) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("system-read".into())
        .spawn(move || {
            let _guard = guard;
            let result = task();
            // Clear the gate before delivery: the caller may immediately read
            // another item. Unwinding also releases it through ReadGuard.
            drop(_guard);
            let _ = send.send(result);
        })
        .map_err(|error| format!("{label}: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        ensure_running()?;
        if Instant::now() >= deadline {
            return Err(format!("{label}: read timed out"));
        }
        match receive.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(format!("{label}: read interrupted"));
            }
        }
    }
}

pub fn require_file(path: &std::path::Path) -> Result<(), String> {
    let path = path.to_owned();
    read("appearance file", move || match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(format!("not a file: {}", path.display())),
        Err(error) => Err(format!("{}: {error}", path.display())),
    })
}
