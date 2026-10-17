//! Registry of live form windows: the single source for "which window is
//! open / which modal is on top" queries (tray focus routing, devtools
//! capture enumeration). Entries self-heal: queries prune HWNDs that have
//! been destroyed, so no destroy hooks are required. The query surface is
//! staged (compiled, unused) until those paths adopt it; registration
//! itself is live via FormWindow::finish.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::IsWindow;

/// Identity of one form window kind, mirroring the window classes.
/// One variant per form window; all are registered via FormWindow::finish.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Settings,
    Automation,
    ContextRules,
    ProcessPicker,
}

#[allow(dead_code)]
struct Entry {
    kind: Kind,
    hwnd: isize,
    anchor: isize,
}

static REGISTRY: std::sync::Mutex<Vec<Entry>> = std::sync::Mutex::new(Vec::new());

fn lock() -> std::sync::MutexGuard<'static, Vec<Entry>> {
    crate::runtime::lock(&REGISTRY)
}

/// Records a live form window (called by FormWindow::finish).
pub fn register(kind: Kind, hwnd: HWND, anchor: HWND) {
    let mut registry = lock();
    registry.retain(|entry| entry.hwnd != hwnd.0 as isize);
    registry.push(Entry {
        kind,
        hwnd: hwnd.0 as isize,
        anchor: anchor.0 as isize,
    });
}

/// Drops entries whose HWNDs no longer exist; returns the live list in
/// registration order.
#[allow(dead_code)]
fn live_entries() -> Vec<(Kind, HWND, HWND)> {
    let mut registry = lock();
    registry.retain(|entry| unsafe { IsWindow(Some(HWND(entry.hwnd as *mut _))).as_bool() });
    registry
        .iter()
        .map(|entry| {
            (
                entry.kind,
                HWND(entry.hwnd as *mut _),
                HWND(entry.anchor as *mut _),
            )
        })
        .collect()
}

/// The HWND for one window kind, if it is still alive.
#[allow(dead_code)]
pub fn hwnd_of(kind: Kind) -> Option<HWND> {
    live_entries()
        .into_iter()
        .find(|(entry_kind, _, _)| *entry_kind == kind)
        .map(|(_, hwnd, _)| hwnd)
}

/// The top-most live form window: the most recently registered entry.
/// Tray left-click routes here while any form window is open.
#[allow(dead_code)]
pub fn top_most() -> Option<HWND> {
    live_entries().pop().map(|(_, hwnd, _)| hwnd)
}
