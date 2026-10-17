//! Registry of live form windows: the lookup source for cross-window UI
//! mechanics (tray focus routing, theme refresh, text-scale cascades).
//! Entries self-heal: lookups prune HWNDs that have been destroyed, so no
//! destroy hooks are required. Top-level form windows only — child panes
//! (the automation editor) and the panel/popups stay with their owning
//! modules.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::IsWindow;

/// Identity of one form window kind, mirroring the window classes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Settings,
    Automation,
    ContextRules,
    ProcessPicker,
}

struct Entry {
    kind: Kind,
    hwnd: isize,
}

static REGISTRY: std::sync::Mutex<Vec<Entry>> = std::sync::Mutex::new(Vec::new());

fn lock() -> std::sync::MutexGuard<'static, Vec<Entry>> {
    crate::runtime::lock(&REGISTRY)
}

/// Records a live form window (called by FormWindow::finish).
pub fn register(kind: Kind, hwnd: HWND) {
    let mut registry = lock();
    registry.retain(|entry| entry.hwnd != hwnd.0 as isize);
    registry.push(Entry {
        kind,
        hwnd: hwnd.0 as isize,
    });
}

/// Drops entries whose HWNDs no longer exist; returns the live list.
fn live_entries() -> Vec<(Kind, HWND)> {
    let mut registry = lock();
    registry.retain(|entry| unsafe { IsWindow(Some(HWND(entry.hwnd as *mut _))).as_bool() });
    registry
        .iter()
        .map(|entry| (entry.kind, HWND(entry.hwnd as *mut _)))
        .collect()
}

/// The HWND for one window kind, if it is still alive.
pub fn hwnd_of(kind: Kind) -> Option<HWND> {
    live_entries()
        .into_iter()
        .find(|(entry_kind, _)| *entry_kind == kind)
        .map(|(_, hwnd)| hwnd)
}
