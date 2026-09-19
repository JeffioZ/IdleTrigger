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

/// True when the foreground window covers the entire monitor it is on
/// (heuristic for fullscreen apps / presentations), matching the Go
/// environment check closely enough for the two skip-fullscreen flags.
pub fn foreground_is_fullscreen() -> bool {
    unsafe {
        if windows::Win32::UI::Shell::SHQueryUserNotificationState()
            .is_ok_and(|state| matches!(state.0, 2..=4))
        {
            return true;
        }
        let fg = GetForegroundWindow();
        if fg.is_invalid()
            || !windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(fg).as_bool()
            || windows::Win32::UI::WindowsAndMessaging::IsIconic(fg).as_bool()
        {
            return false;
        }
        let mut cloaked = 0u32;
        if windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
            fg,
            windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return false;
        }
        let mut class = [0u16; 64];
        let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameW(fg, &mut class);
        if matches!(
            String::from_utf16_lossy(&class[..len.max(0) as usize])
                .to_ascii_lowercase()
                .as_str(),
            "progman" | "workerw" | "shell_traywnd" | "shell_secondarytraywnd"
        ) {
            return false;
        }
        let monitor = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return false;
        }
        let mut wr = RECT::default();
        if windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
            fg,
            windows::Win32::Graphics::Dwm::DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut wr as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
        .is_err()
            && windows::Win32::UI::WindowsAndMessaging::GetWindowRect(fg, &mut wr).is_err()
        {
            return false;
        }
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(fg).max(96);
        covers_monitor(wr, info.rcMonitor, ((dpi * 2 + 48) / 96) as i32)
    }
}

fn covers_monitor(window: RECT, monitor: RECT, tolerance: i32) -> bool {
    window.right > window.left
        && window.bottom > window.top
        && monitor.right > monitor.left
        && monitor.bottom > monitor.top
        && window.left as i64 <= monitor.left as i64 + tolerance as i64
        && window.top as i64 <= monitor.top as i64 + tolerance as i64
        && window.right as i64 >= monitor.right as i64 - tolerance as i64
        && window.bottom as i64 >= monitor.bottom as i64 - tolerance as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fullscreen_accepts_dpi_rounding_but_not_taskbar_work_area() {
        let monitor = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        assert!(covers_monitor(
            RECT {
                left: -1918,
                top: 2,
                right: -2,
                bottom: 1078
            },
            monitor,
            2
        ));
        assert!(!covers_monitor(
            RECT {
                bottom: 1040,
                ..monitor
            },
            monitor,
            2
        ));
        assert!(!covers_monitor(RECT::default(), monitor, 2));
    }
}
