//! Owner-draw plumbing shared by all form windows (Go nativeform
//! interaction.go parity): subclasses standard controls only to observe
//! hover / press / focus, and exposes WM_DRAWITEM dispatch helpers. Native
//! controls keep their input, keyboard, and accessibility behavior.

use std::collections::HashMap;
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, WM_MOUSELEAVE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TRACKMOUSEEVENT_FLAGS, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::paint::ControlState;

#[derive(Clone, Copy, Default)]
pub struct InteractionState {
    hovered: bool,
    focused: bool,
}

struct Tracked {
    old_proc: isize,
    state: InteractionState,
}

static CONTROLS: Mutex<Option<HashMap<isize, Tracked>>> = Mutex::new(None);

// Keyboard-focus visibility (Go tracker focusVisible): the owner-drawn focus
// ring renders only when focus arrived via the keyboard, so clicking a
// control never shows the frame.
static FOCUS_VISIBLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn with_map<R>(f: impl FnOnce(&mut HashMap<isize, Tracked>) -> R) -> R {
    let mut guard = CONTROLS.lock().unwrap();
    f(guard.get_or_insert_with(HashMap::new))
}

/// Subclasses `control` to observe hover/press/focus for owner drawing.
pub fn track(control: HWND) {
    if control.is_invalid() {
        return;
    }
    let key = control.0 as isize;
    if with_map(|m| m.contains_key(&key)) {
        return;
    }
    unsafe {
        let proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT = tracked_proc;
        // WNDPROC roundtrip through GWLP_WNDPROC is the documented contract;
        // 32-bit maps the LONG_PTR family to i32 in the windows crate.
        #[allow(clippy::fn_to_numeric_cast)]
        let raw = proc as isize;
        #[cfg(target_arch = "x86")]
        let old = SetWindowLongPtrW(control, GWLP_WNDPROC, raw as i32) as isize;
        #[cfg(not(target_arch = "x86"))]
        let old = SetWindowLongPtrW(control, GWLP_WNDPROC, raw);
        if old == 0 {
            return;
        }
        with_map(|m| {
            m.insert(
                key,
                Tracked {
                    old_proc: old,
                    state: InteractionState::default(),
                },
            );
        });
    }
}

/// Current hover/focus for an owner-drawn control.
pub fn interaction(control: HWND) -> InteractionState {
    let key = control.0 as isize;
    with_map(|m| m.get(&key).map(|t| t.state).unwrap_or_default())
}

const ODS_SELECTED: u32 = 0x0001;
const ODS_DISABLED: u32 = 0x0004;
const ODS_FOCUS: u32 = 0x0010;

/// Merges tracked interaction into a paint ControlState; pressed/disabled
/// come from the WM_DRAWITEM itemState flags (native-sourced like Go).
pub fn control_state(control: HWND, item_state: u32) -> ControlState {
    let it = interaction(control);
    ControlState {
        hovered: it.hovered,
        pressed: item_state & ODS_SELECTED != 0,
        focused: item_state & ODS_FOCUS != 0
            && FOCUS_VISIBLE.load(std::sync::atomic::Ordering::SeqCst),
        disabled: item_state & ODS_DISABLED != 0,
        active: false,
        open: false,
    }
}

/// Buffered WM_DRAWITEM painting (Go DrawBuffered): render into a compatible
/// memory bitmap and blit once, so repeated hover/focus repaints never flash
/// the control background and text stays ClearType-crisp on an opaque surface.
pub unsafe fn draw_buffered(hdc: HDC, bounds: &RECT, paint: impl FnOnce(HDC, &RECT)) -> bool {
    // Opt-out escape hatch for diagnosing paint issues.
    if std::env::var("IDLETRIGGER_NO_BUFFER").is_ok() {
        return false;
    }
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, SRCCOPY,
        SelectObject,
    };
    unsafe {
        let width = bounds.right - bounds.left;
        let height = bounds.bottom - bounds.top;
        if width <= 0 || height <= 0 {
            return false;
        }
        let mem = CreateCompatibleDC(Some(hdc));
        if mem.is_invalid() {
            return false;
        }
        let bitmap = CreateCompatibleBitmap(hdc, width, height);
        if bitmap.is_invalid() {
            let _ = DeleteDC(mem);
            return false;
        }
        let old = SelectObject(mem, windows::Win32::Graphics::Gdi::HGDIOBJ(bitmap.0));
        let local = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        paint(mem, &local);
        let ok = BitBlt(
            hdc,
            bounds.left,
            bounds.top,
            width,
            height,
            Some(mem),
            0,
            0,
            SRCCOPY,
        )
        .is_ok();
        SelectObject(mem, old);
        let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem);
        ok
    }
}

unsafe extern "system" fn tracked_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let key = hwnd.0 as isize;
        let old = with_map(|m| m.get(&key).map(|t| t.old_proc));
        let Some(old) = old else {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        };
        match msg {
            // Owner-drawn controls cover their whole client area; skipping the
            // class-brush erase is half of the no-flicker contract (the other
            // half is buffered WM_DRAWITEM painting).
            windows::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND => return LRESULT(1),
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                // The focus ring renders only for keyboard navigation.
                if !FOCUS_VISIBLE.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    invalidate(hwnd);
                }
            }
            WM_MOUSEMOVE => {
                if FOCUS_VISIBLE.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    invalidate(hwnd);
                }
                let changed = with_map(|m| {
                    if let Some(t) = m.get_mut(&key)
                        && !t.state.hovered
                    {
                        t.state.hovered = true;
                        return true;
                    }
                    false
                });
                if changed {
                    begin_leave_tracking(hwnd);
                    invalidate(hwnd);
                }
            }
            WM_LBUTTONDOWN => {
                if FOCUS_VISIBLE.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    invalidate(hwnd);
                }
            }
            WM_MOUSELEAVE => {
                let changed = with_map(|m| {
                    if let Some(t) = m.get_mut(&key) {
                        t.state.hovered = false;
                        return true;
                    }
                    false
                });
                if changed {
                    invalidate(hwnd);
                }
            }
            WM_SETFOCUS | WM_KILLFOCUS => {
                let focused = msg == WM_SETFOCUS;
                with_map(|m| {
                    if let Some(t) = m.get_mut(&key) {
                        t.state.focused = focused;
                    }
                });
                invalidate(hwnd);
            }
            WM_ENABLE | WM_CAPTURECHANGED => {
                invalidate(hwnd);
            }
            WM_NCDESTROY => {
                with_map(|m| {
                    m.remove(&key);
                });
                // The subclass dies with the window; nothing to restore.
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            _ => {}
        }
        CallWindowProcW(
            std::mem::transmute::<isize, WNDPROC>(old),
            hwnd,
            msg,
            wparam,
            lparam,
        )
    }
}

unsafe fn begin_leave_tracking(hwnd: HWND) {
    unsafe {
        let mut event = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TRACKMOUSEEVENT_FLAGS(TME_LEAVE.0),
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        let _ = TrackMouseEvent(&mut event);
    }
}

unsafe fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
    }
}

// ---- WM_DRAWITEM helpers ---------------------------------------------------

/// Parsed WM_DRAWITEM payload (ODT_BUTTON / ODT_STATIC).
pub struct DrawItem {
    pub control_id: i32,
    pub control: HWND,
    pub dc: HDC,
    pub bounds: RECT,
    pub state: u32,
}

/// Parses WM_DRAWITEM; returns None for other control types.
pub fn draw_item(lparam: LPARAM) -> Option<DrawItem> {
    if lparam.0 == 0 {
        return None;
    }
    unsafe {
        let item = &*(lparam.0 as *const DRAWITEMSTRUCT);
        if item.CtlType.0 != ODT_BUTTON && item.CtlType.0 != ODT_STATIC {
            return None;
        }
        Some(DrawItem {
            control_id: item.CtlID as i32,
            control: item.hwndItem,
            dc: item.hDC,
            bounds: item.rcItem,
            state: item.itemState.0,
        })
    }
}

const ODT_BUTTON: u32 = 4;
const ODT_STATIC: u32 = 5;
