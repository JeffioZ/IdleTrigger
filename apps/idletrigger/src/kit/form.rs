//! The secondary-window shell: one pair of calls owning the class
//! registration, frame sizing, anchoring, DPI subclass installation,
//! theme application, and icon setup that every form window used to
//! hand-roll. Split into `create` + `finish` because every window stores
//! its HWND static between the two steps — its wndproc already runs
//! during `finish` (WM_CREATE fires inside CreateWindowExW, and later
//! steps dispatch messages) and must find that static populated, exactly
//! as the hand-rolled code ordered things.

use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use crate::kit::registry::Kind;

/// Everything `FormWindow::create` needs to build one secondary window;
/// constructed as a plain struct literal at call sites.
pub struct FormSpec {
    /// Window class name (a `w!` literal, e.g. "IdleTriggerSettings").
    pub class: PCWSTR,
    /// Registry identity for queries (tray focus routing, capture).
    pub kind: Kind,
    /// Window title, localized by the caller.
    pub title: String,
    /// Window procedure for the class (an `Option<fn>` — pass `Some(proc)`).
    pub proc: windows::Win32::UI::WindowsAndMessaging::WNDPROC,
    /// Client size in logical pixels, scaled under the caller's DPI scope.
    pub client_w: i32,
    pub client_h: i32,
    /// Window style (typically `secondary_style()`).
    pub style: WINDOW_STYLE,
    /// Modal owner / centering anchor (usually the panel).
    pub anchor: HWND,
    /// Class background brush (theme::bg_brush()).
    pub bg_brush: HBRUSH,
}

/// Centers a window on its anchor's rect, clamped into the anchor's work
/// area; falls back to the primary screen metrics (settings-window
/// behavior).
pub fn center_on_anchor(anchor: HWND, win_w: i32, win_h: i32) -> (i32, i32) {
    unsafe {
        let mut center = RECT {
            left: 0,
            top: 0,
            right: GetSystemMetrics(SM_CXSCREEN),
            bottom: GetSystemMetrics(SM_CYSCREEN),
        };
        let mut wr = RECT::default();
        if GetWindowRect(anchor, &mut wr).is_ok() && wr.right > wr.left && wr.bottom > wr.top {
            center = wr;
        }
        let mut x = center.left + (center.right - center.left - win_w) / 2;
        let mut y = center.top + (center.bottom - center.top - win_h) / 2;
        let work = crate::display::work_area_for(anchor);
        let work_w = work.right - work.left;
        let work_h = work.bottom - work.top;
        if win_w >= work_w {
            x = work.left;
        } else {
            x = x.clamp(work.left, work.right - win_w);
        }
        if win_h >= work_h {
            y = work.top;
        } else {
            y = y.clamp(work.top, work.bottom - win_h);
        }
        (x, y)
    }
}

/// The standard fixed secondary-window style (settings / managers): a
/// non-resizable overlapped window that clips children so overlay
/// scrollbar siblings survive background erases.
pub fn secondary_style() -> WINDOW_STYLE {
    // WS_CLIPCHILDREN: these forms host overlay scrollbar siblings above
    // their lists - without the clip the form's background erase would
    // blank the bar between its repaints.
    WINDOW_STYLE(
        (WS_OVERLAPPEDWINDOW.0 & !WS_MAXIMIZEBOX.0 & !WS_THICKFRAME.0 & !WS_MINIMIZEBOX.0)
            | WS_CLIPCHILDREN.0,
    )
}

unsafe fn register_class(
    instance: HMODULE,
    class: PCWSTR,
    proc: windows::Win32::UI::WindowsAndMessaging::WNDPROC,
    bg_brush: HBRUSH,
) {
    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: proc,
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            // MAKEINTRESOURCE(1) = winresource's "1 ICON" entry.
            #[allow(clippy::manual_dangling_ptr)]
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1usize as *const u16))
                .unwrap_or_default(),
            hbrBackground: bg_brush,
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
}

/// A created secondary window handle. `create` builds the raw window;
/// `finish` installs the shared chrome (DPI subclass, theme, icons) and
/// records the registry entry.
pub struct FormWindow {
    pub hwnd: HWND,
    kind: Kind,
}

impl FormWindow {
    /// Registers the class (idempotently — RegisterClassW fails
    /// atomically on re-registration, matching the old per-window
    /// behavior), sizes the frame from the logical client size, centers
    /// on the anchor, and creates the window.
    pub unsafe fn create(spec: FormSpec) -> FormWindow {
        unsafe {
            let instance = GetModuleHandleW(None).expect("module handle");
            register_class(instance, spec.class, spec.proc, spec.bg_brush);
            let mut frame = RECT {
                left: 0,
                top: 0,
                right: crate::scale_pub(spec.client_w),
                bottom: crate::scale_pub(spec.client_h),
            };
            let _ = AdjustWindowRectEx(&mut frame, spec.style, false, WINDOW_EX_STYLE(0));
            let win_w = frame.right - frame.left;
            let win_h = frame.bottom - frame.top;
            let (x, y) = center_on_anchor(spec.anchor, win_w, win_h);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                spec.class,
                PCWSTR(crate::wide(&spec.title).as_ptr()),
                spec.style,
                x,
                y,
                win_w,
                win_h,
                Some(spec.anchor),
                None,
                Some(instance.into()),
                None,
            )
            .expect("form window");
            FormWindow {
                hwnd,
                kind: spec.kind,
            }
        }
    }

    /// Installs the DPI subclass, applies the theme, sets icons, stamps
    /// the theme fingerprint (a follow-up broadcast refresh then skips
    /// this window instead of flashing it), and records the registry
    /// entry. Call after storing the window HWND static, mirroring the
    /// hand-rolled ordering.
    pub fn finish(&self) {
        crate::dpi::install(self.hwnd);
        crate::theme::apply_to_window(self.hwnd);
        crate::set_window_icons_pub(self.hwnd);
        crate::theme::stamp_current_palette(self.hwnd);
        crate::kit::registry::register(self.kind, self.hwnd);
    }
}
