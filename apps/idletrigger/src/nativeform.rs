//! Owner-draw plumbing shared by all form windows. Subclasses standard
//! controls to observe hover, press, and focus, and provides WM_DRAWITEM
//! dispatch helpers. Native controls retain input and accessibility behavior.

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

/// Change child visibility without presenting an intermediate layout.
/// The caller commits the complete parent frame after all changes.
pub fn set_visible_deferred(control: HWND, visible: bool) {
    unsafe {
        if control.is_invalid()
            || (GetWindowLongW(control, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0) == visible
        {
            return;
        }
        let _ = SetWindowPos(
            control,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE
                | SWP_NOSIZE
                | SWP_NOZORDER
                | SWP_NOACTIVATE
                | SWP_NOREDRAW
                | if visible {
                    SWP_SHOWWINDOW
                } else {
                    SWP_HIDEWINDOW
                },
        );
    }
}

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
// Deliberately process-global: keyboard focus rings turn on from any key
// press and turn off from any mouse activity across all tracked windows —
// "there was recent mouse input" is a session-wide fact, not per-window.
static FOCUS_VISIBLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn keyboard_navigation() {
    FOCUS_VISIBLE.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Keep the native edit (including its cue accessibility text), but paint the
/// empty-field hint with the same readable palette as the other form controls.
pub fn cue_banner(control: HWND, key: &'static str) {
    unsafe {
        let text: Vec<u16> = crate::t_pub(key).encode_utf16().chain([0]).collect();
        SendMessageW(
            control,
            0x1501,
            Some(WPARAM(1)),
            Some(LPARAM(text.as_ptr() as isize)),
        );
        let mut existing = 0;
        if windows::Win32::UI::Shell::GetWindowSubclass(
            control,
            Some(cue_proc),
            0x49544355,
            Some(&mut existing),
        )
        .as_bool()
        {
            *(existing as *mut &'static str) = key;
            return;
        }
        let data = Box::into_raw(Box::new(key));
        if !windows::Win32::UI::Shell::SetWindowSubclass(
            control,
            Some(cue_proc),
            0x49544355,
            data as usize,
        )
        .as_bool()
        {
            drop(Box::from_raw(data));
        }
    }
}

unsafe extern "system" fn cue_proc(
    control: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    unsafe {
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(control, Some(cue_proc), id);
            drop(Box::from_raw(data as *mut &'static str));
            return DefSubclassProc(control, msg, wp, lp);
        }
        let result = DefSubclassProc(control, msg, wp, lp);
        if matches!(msg, WM_PAINT | WM_PRINTCLIENT) && GetWindowTextLengthW(control) == 0 {
            let dc = if msg == WM_PRINTCLIENT {
                HDC(wp.0 as *mut _)
            } else {
                GetDC(Some(control))
            };
            if !dc.is_invalid() {
                let saved = SaveDC(dc);
                let mut rect = RECT::default();
                SendMessageW(
                    control,
                    0x00B2,
                    None,
                    Some(LPARAM(&mut rect as *mut _ as isize)),
                ); // EM_GETRECT
                let p = crate::theme::palette();
                crate::paint::fill_rect(dc, &rect, p.surface);
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, windows::Win32::Foundation::COLORREF(p.muted));
                let font = SendMessageW(control, WM_GETFONT, None, None);
                SelectObject(dc, HGDIOBJ(font.0 as *mut _));
                let key = *(data as *const &'static str);
                let mut text: Vec<u16> = crate::t_pub(key).encode_utf16().collect();
                DrawTextW(
                    dc,
                    &mut text,
                    &mut rect,
                    DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS | DT_VCENTER,
                );
                let _ = RestoreDC(dc, saved);
                if msg != WM_PRINTCLIENT {
                    ReleaseDC(Some(control), dc);
                }
            }
        }
        result
    }
}

fn with_map<R>(f: impl FnOnce(&mut HashMap<isize, Tracked>) -> R) -> R {
    let mut guard = crate::runtime::lock(&CONTROLS);
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
                crate::accessibility::clear(hwnd);
                with_map(|m| {
                    m.remove(&key);
                });
                // Native controls still need their original destruction chain.
                return CallWindowProcW(
                    std::mem::transmute::<isize, WNDPROC>(old),
                    hwnd,
                    msg,
                    wparam,
                    lparam,
                );
            }
            _ => {}
        }
        let result = CallWindowProcW(
            std::mem::transmute::<isize, WNDPROC>(old),
            hwnd,
            msg,
            wparam,
            lparam,
        );
        if matches!(msg, WM_SETFOCUS | WM_KILLFOCUS | WM_ENABLE) {
            crate::accessibility::refresh(hwnd);
        }
        result
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

/// Owned tooltip window; reinstallation replaces translated text atomically.
pub fn form_tooltips(parent: HWND, bindings: &[(usize, &str)]) {
    use windows::Win32::UI::Controls::*;
    use windows::core::{PCWSTR, PWSTR, w};
    unsafe {
        let old = GetPropW(parent, w!("IdleTriggerFormTooltip"));
        if !old.is_invalid() {
            let _ = DestroyWindow(HWND(old.0));
        }
        let _ = RemovePropW(parent, w!("IdleTriggerFormTooltip"));
        let Ok(tip) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            TOOLTIPS_CLASSW,
            w!(""),
            WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP),
            0,
            0,
            0,
            0,
            Some(parent),
            None,
            None,
            None,
        ) else {
            return;
        };
        if SetPropW(
            parent,
            w!("IdleTriggerFormTooltip"),
            Some(windows::Win32::Foundation::HANDLE(tip.0)),
        )
        .is_err()
        {
            let _ = DestroyWindow(tip);
            return;
        }
        SendMessageW(
            tip,
            TTM_SETMAXTIPWIDTH,
            None,
            Some(LPARAM(crate::scale_pub(480) as isize)),
        );
        for (id, key) in bindings {
            let Ok(control) = GetDlgItem(Some(parent), *id as i32) else {
                continue;
            };
            let mut text = crate::t_pub(key)
                .encode_utf16()
                .chain([0])
                .collect::<Vec<_>>();
            let tool = TTTOOLINFOW {
                cbSize: size_of::<TTTOOLINFOW>() as u32,
                uFlags: TTF_IDISHWND | TTF_SUBCLASS,
                hwnd: parent,
                uId: control.0 as usize,
                lpszText: PWSTR(text.as_mut_ptr()),
                ..Default::default()
            };
            SendMessageW(
                tip,
                TTM_ADDTOOLW,
                None,
                Some(LPARAM(&tool as *const _ as isize)),
            );
        }
        let empty = [0u16];
        let _ = SetWindowTheme(tip, PCWSTR(empty.as_ptr()), PCWSTR(empty.as_ptr()));
        let p = crate::theme::palette();
        SendMessageW(
            tip,
            TTM_SETTIPBKCOLOR,
            Some(WPARAM(p.tooltip_bg as usize)),
            None,
        );
        SendMessageW(
            tip,
            TTM_SETTIPTEXTCOLOR,
            Some(WPARAM(p.tooltip_text as usize)),
            None,
        );
    }
}
