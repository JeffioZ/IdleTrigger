//! Display helpers: active-monitor placement for overlays and fullscreen
//! detection for the two skip-fullscreen behaviors.

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
};

/// Work area of the monitor that `hwnd` is on (nearest), falling back to the
/// primary screen metrics.
pub fn work_area_for(hwnd: HWND) -> RECT {
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !monitor.is_invalid() && GetMonitorInfoW(monitor, &mut info).as_bool() {
            return RECT {
                left: info.rcWork.left,
                top: info.rcWork.top,
                right: info.rcWork.right,
                bottom: info.rcWork.bottom,
            };
        }
        RECT {
            left: 0,
            top: 0,
            right: GetSystemMetrics(SM_CXSCREEN),
            bottom: GetSystemMetrics(SM_CYSCREEN),
        }
    }
}

/// Centers `w×h` in the work area of the monitor `hwnd` lives on.
pub fn centered_on(hwnd: HWND, w: i32, h: i32) -> (i32, i32) {
    let work = work_area_for(hwnd);
    (
        (work.left + (work.right - work.left - w) / 2).max(work.left),
        (work.top + (work.bottom - work.top - h) / 2).max(work.top),
    )
}

/// Bottom-center of the monitor `hwnd` lives on, with a logical margin.
#[allow(dead_code)]
pub fn bottom_center_on(hwnd: HWND, w: i32, h: i32, margin: i32) -> (i32, i32) {
    let work = work_area_for(hwnd);
    let m = crate::scale_pub(margin);
    (
        (work.left + (work.right - work.left - w) / 2).max(work.left),
        (work.bottom - h - m).max(work.top),
    )
}

/// True when the foreground window covers the entire monitor it is on
/// (heuristic for fullscreen apps / presentations), matching the Go
/// environment check closely enough for the two skip-fullscreen flags.
pub fn foreground_is_fullscreen() -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let work = work_area_for(fg);
        let mut wr = RECT::default();
        if windows::Win32::UI::WindowsAndMessaging::GetWindowRect(fg, &mut wr).is_err() {
            return false;
        }
        // Covers the full monitor (not just the work area — taskbar hidden).
        let full = RECT {
            left: 0,
            top: 0,
            right: GetSystemMetrics(SM_CXSCREEN),
            bottom: GetSystemMetrics(SM_CYSCREEN),
        };
        wr.left <= full.left
            && wr.top <= full.top
            && wr.right >= full.right
            && wr.bottom >= full.bottom
            && (wr.right - wr.left) >= (work.right - work.left)
    }
}
