//! Coalesced display recovery. No display timings enter the fingerprint, so
//! variable refresh rate does not look like a monitor being reconnected.
use std::sync::{
    Mutex,
    atomic::{AtomicU32, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, LUID};

static GENERATION: AtomicU64 = AtomicU64::new(0);
static EVENTS: AtomicU32 = AtomicU32::new(0);
static HISTORY: Mutex<(Option<Instant>, Option<Instant>)> = Mutex::new((None, None));

pub fn changed() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}
pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}
pub fn environment_changed(display: bool, resume: bool) {
    EVENTS.fetch_or(display as u32 | ((resume as u32) << 1), Ordering::SeqCst);
    crate::theme_engine::wake();
}
pub fn pending() -> bool {
    EVENTS.load(Ordering::SeqCst) != 0
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Path {
    target_adapter: i64,
    target: u32,
    source_adapter: i64,
    source: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    rotation: i32,
}
fn luid(value: LUID) -> i64 {
    ((value.HighPart as i64) << 32) | value.LowPart as i64
}

fn snapshot() -> Result<Vec<Path>, String> {
    unsafe {
        for _ in 0..5 {
            let (mut np, mut nm) = (0, 0);
            let result = GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm);
            if result != ERROR_SUCCESS {
                return Err(format!("display sizes: {}", result.0));
            }
            if np == 0 {
                return Ok(Vec::new());
            }
            if np > 4096 || nm > 16384 {
                return Err("display counts exceed bounds".into());
            }
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
            let result = QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut np,
                paths.as_mut_ptr(),
                &mut nm,
                modes.as_mut_ptr(),
                None,
            );
            if result == ERROR_INSUFFICIENT_BUFFER {
                continue;
            }
            if result != ERROR_SUCCESS {
                return Err(format!("display query: {}", result.0));
            }
            paths.truncate(np as usize);
            modes.truncate(nm as usize);
            let mut result = Vec::with_capacity(paths.len());
            for path in paths {
                let source = path.sourceInfo;
                let mode = modes
                    .get(source.Anonymous.modeInfoIdx as usize)
                    .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE)
                    .or_else(|| {
                        modes.iter().find(|m| {
                            m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE
                                && m.id == source.id
                                && m.adapterId == source.adapterId
                        })
                    });
                let mode = mode.map(|m| m.Anonymous.sourceMode).unwrap_or_default();
                result.push(Path {
                    target_adapter: luid(path.targetInfo.adapterId),
                    target: path.targetInfo.id,
                    source_adapter: luid(source.adapterId),
                    source: source.id,
                    x: mode.position.x,
                    y: mode.position.y,
                    width: mode.width,
                    height: mode.height,
                    rotation: path.targetInfo.rotation.0,
                });
            }
            result.sort();
            return Ok(result);
        }
        Err("display configuration kept changing".into())
    }
}

pub struct Prepared {
    generation: u64,
    events: u32,
    displays: usize,
    stable: bool,
}
impl Prepared {
    pub fn current(&self) -> bool {
        self.generation == GENERATION.load(Ordering::SeqCst)
            && !crate::EXITING.load(Ordering::SeqCst)
    }
}

/// Background-thread wait only. Three equal nonempty samples after the grace
/// period; a newer request cancels the entire preparation.
pub fn prepare(generation: u64) -> Option<Prepared> {
    let mut prepared = Prepared {
        generation,
        events: EVENTS.load(Ordering::SeqCst),
        displays: 0,
        stable: false,
    };
    let start = Instant::now();
    let mut previous = None;
    let mut identical = 0;
    while start.elapsed() < Duration::from_secs(8) {
        if !prepared.current() {
            return None;
        }
        if start.elapsed() >= Duration::from_millis(2500) {
            match snapshot() {
                Ok(paths) if !paths.is_empty() => {
                    identical = if previous.as_ref() == Some(&paths) {
                        identical + 1
                    } else {
                        1
                    };
                    prepared.displays = paths.len();
                    previous = Some(paths);
                    if identical >= 3 {
                        prepared.stable = true;
                        break;
                    }
                }
                _ => {
                    identical = 0;
                    previous = None;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(450));
    }
    Some(prepared)
}

fn needs_full_repair(switched: bool, events: u32, displays: usize, recent: bool) -> bool {
    (switched && (displays >= 2 || events != 0))
        || (!switched && ((events & 1 != 0 && recent) || (events & 6 != 0 && displays >= 2)))
}

pub fn transition_applied() {
    HISTORY.lock().unwrap().0 = Some(Instant::now());
    // Retain the post-transition check if preparation is cancelled or unstable.
    EVENTS.fetch_or(4, Ordering::SeqCst);
}

/// Called while the shared theme-operation lock is held. The caller broadcasts
/// afterwards, outside the lock, to avoid cross-thread message deadlocks.
pub fn finish(prepared: Prepared, switched: bool) {
    if !prepared.current() {
        return;
    }
    let mut history = HISTORY.lock().unwrap();
    let recent = history
        .0
        .is_some_and(|time| time.elapsed() <= Duration::from_secs(15));
    if prepared.stable
        && needs_full_repair(switched, prepared.events, prepared.displays, recent)
        && history
            .1
            .is_none_or(|time| time.elapsed() >= Duration::from_secs(30))
        && crate::theme_repair::full_dwm_refresh_available()
    {
        history.1 = Some(Instant::now());
        if let Err(error) = crate::theme_repair::refresh_dwm_colorization() {
            crate::log_line(&format!("automatic theme recovery failed: {error}"));
            return;
        }
    }
    if prepared.stable {
        let consumed = prepared.events | if switched { 4 } else { 0 };
        EVENTS.fetch_and(!consumed, Ordering::SeqCst);
        if !prepared.current() {
            EVENTS.fetch_or(consumed, Ordering::SeqCst);
        }
    } else {
        crate::log_line("display topology did not stabilize; deferring full theme repair");
    }
}

pub fn manual_repair_completed() {
    HISTORY.lock().unwrap().1 = Some(Instant::now());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repairs_only_risky_transitions_and_recent_display_changes() {
        assert!(!needs_full_repair(true, 0, 1, false));
        assert!(needs_full_repair(true, 0, 2, false));
        assert!(needs_full_repair(false, 2, 2, false));
        assert!(!needs_full_repair(false, 2, 1, true));
        assert!(needs_full_repair(false, 1, 1, true));
        assert!(!needs_full_repair(false, 1, 2, false));
    }
}
