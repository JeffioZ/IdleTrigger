//! Owner-draw plumbing shared by all form windows. Subclasses standard
//! controls to observe hover, press, and focus, and provides WM_DRAWITEM
//! dispatch helpers. Native controls retain input and accessibility behavior.

use std::collections::HashMap;
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, WM_MOUSELEAVE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TRACKMOUSEEVENT_FLAGS, TrackMouseEvent, VK_ESCAPE,
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
    pub(crate) hovered: bool,
    pub(crate) focused: bool,
}

struct Tracked {
    old_proc: isize,
    state: InteractionState,
    /// Right-edge hover column in physical px (0 = the whole control
    /// hovers). Switch rows set this to their pill column so hover lives
    /// on the pill alone - panel parity, where the label is a separate
    /// static that never hovers.
    hover_column: i32,
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

/// Restricts a tracked control's hover state to its right-edge column of
/// `physical_width` px (switch rows: the pill). Call after `track`.
pub fn set_hover_column(control: HWND, physical_width: i32) {
    with_map(|m| {
        if let Some(t) = m.get_mut(&(control.0 as isize)) {
            t.hover_column = physical_width;
        }
    });
}

/// Shared blank-drag hit-test for the panel-family top-level windows: a
/// HTCLIENT point that resolves to the window itself becomes HTCAPTION,
/// so true blanks drag with the system's move (and snap) while controls
/// keep native hit-testing - including for direct WM_NCHITTEST queries
/// from automation and tests. NOTE: tracked controls must never answer
/// HTTRANSPARENT to make this probe fall through to them - that
/// re-enters the caller's in-flight hit-test and recurses infinitely
/// (the switch-row blank crash); column-limited clicks are swallowed in
/// WM_LBUTTONDOWN instead.
pub fn blank_drag_hit(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let hit = DefWindowProcW(hwnd, msg, wparam, lparam);
        if hit.0 as u32 != windows::Win32::UI::WindowsAndMessaging::HTCLIENT {
            return hit;
        }
        let screen = windows::Win32::Foundation::POINT {
            x: (lparam.0 & 0xFFFF) as i16 as i32,
            y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
        };
        if windows::Win32::UI::WindowsAndMessaging::WindowFromPoint(screen) == hwnd {
            LRESULT(windows::Win32::UI::WindowsAndMessaging::HTCAPTION as isize)
        } else {
            hit
        }
    }
}

/// Overlay-pane variant (the rule editor inside the manager): the pane's
/// own surface is always hit-transparent, so blank clicks fall through to
/// the manager's blank drag while the pane's controls keep claiming their
/// areas (the system asks them first). Unconditional is safe: the pane
/// hosts no interactive surface of its own, and no tracked control on it
/// ever routes through here.
pub fn pane_blank_transparent(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let _ = (hwnd, msg, wparam, lparam);
    LRESULT(windows::Win32::UI::WindowsAndMessaging::HTTRANSPARENT as isize)
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
                    hover_column: 0,
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
                // Esc on a tracked child closes the control panel. Draft-
                // owning windows (settings, task editor) are deliberately
                // excluded: their children's parent is not the panel.
                if msg == WM_KEYDOWN
                    && wparam.0 as u16 == VK_ESCAPE.0
                    && GetParent(hwnd).is_ok_and(|parent| parent == crate::hwnd(&crate::PANEL))
                {
                    let _ = PostMessageW(
                        Some(crate::hwnd(&crate::PANEL)),
                        WM_CLOSE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
            WM_MOUSEMOVE => {
                // Keyboard focus tips yield to mouse input (keyboard-cue
                // convention); the tip's own control keeps it as its hover tip.
                crate::tooltips::yield_focus_tip(hwnd);
                if FOCUS_VISIBLE.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    invalidate(hwnd);
                }
                // Column-tracked controls (switch rows) hover only inside
                // their right-edge pill column; crossing out of it clears
                // hover, so the pill never keeps a stale hover ring.
                let in_column = with_map(|m| {
                    match m.get(&key) {
                        Some(t) if t.hover_column > 0 => {
                            // WM_MOUSEMOVE carries client coordinates.
                            let x = (lparam.0 & 0xFFFF) as i16 as i32;
                            let mut rect = RECT::default();
                            GetClientRect(hwnd, &mut rect).is_ok()
                                && x >= rect.right - t.hover_column
                        }
                        _ => true,
                    }
                });
                let changed = with_map(|m| {
                    if let Some(t) = m.get_mut(&key)
                        && t.state.hovered != in_column
                    {
                        t.state.hovered = in_column;
                        return true;
                    }
                    false
                });
                if changed {
                    begin_leave_tracking(hwnd);
                    invalidate(hwnd);
                }
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                // Switch rows click as their pill column alone (hover
                // parity). The label/blank half behaves like every other
                // blank in the app: the press is forwarded to the parent
                // as a caption drag, so the row-wide control geometry
                // creates no dead zone - behaviorally the control is only
                // as wide as its pill. No HTTRANSPARENT routing here (that
                // re-entered the parent hit-test chain and crashed).
                let in_column = with_map(|m| match m.get(&key) {
                    Some(t) if t.hover_column > 0 => {
                        let x = (lparam.0 & 0xFFFF) as i16 as i32;
                        let mut rect = RECT::default();
                        GetClientRect(hwnd, &mut rect).is_ok() && x >= rect.right - t.hover_column
                    }
                    _ => true,
                });
                if !in_column {
                    let mut pt = windows::Win32::Foundation::POINT {
                        x: (lparam.0 & 0xFFFF) as i16 as i32,
                        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                    };
                    if windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut pt).as_bool() {
                        // Forward to the ROOT window, not the direct parent:
                        // switch rows inside the editor pane are children of
                        // a child, and a caption drag on the pane would slide
                        // the pane (with all controls) inside the manager
                        // instead of moving the window.
                        let root = windows::Win32::UI::WindowsAndMessaging::GetAncestor(
                            hwnd,
                            windows::Win32::UI::WindowsAndMessaging::GA_ROOT,
                        );
                        if !root.is_invalid() {
                            let packed =
                                (((pt.y as isize) & 0xFFFF) << 16) | (pt.x as isize & 0xFFFF);
                            let _ = SendMessageW(
                                root,
                                WM_NCLBUTTONDOWN,
                                Some(WPARAM(
                                    windows::Win32::UI::WindowsAndMessaging::HTCAPTION as usize,
                                )),
                                Some(LPARAM(packed)),
                            );
                        }
                    }
                    return LRESULT(0);
                }
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
                // Keyboard users deserve the tooltip too: show it while a
                // focusable control carries focus, hide when focus leaves.
                // Focus arriving from a mouse click skips activation — the
                // hover tip is already on screen, and track-activating the
                // same tool again makes the visible tip blink. Dismissals
                // are posted: this can fire inside DestroyWindow dispatch,
                // where a synchronous one poisons the tooltip control.
                if focused {
                    if !cursor_on_control(hwnd) {
                        crate::tooltips::set_focus_tip(hwnd, true);
                    }
                } else if crate::tooltips::focus_tip_active(hwnd) {
                    crate::tooltips::dismiss_focus_tip(hwnd);
                }
            }
            WM_ENABLE | WM_CAPTURECHANGED => {
                invalidate(hwnd);
            }
            WM_NCDESTROY => {
                // A focus tip outliving its control would wedge the tooltip
                // control forever — tracked tips never time out. Posted,
                // because this runs inside DestroyWindow dispatch.
                if crate::tooltips::focus_tip_active(hwnd) {
                    crate::tooltips::dismiss_focus_tip(hwnd);
                }
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

/// Whether the mouse cursor currently rests inside the control's client
/// area — i.e. focus arrived from a click on that control, not the keyboard.
unsafe fn cursor_on_control(hwnd: HWND) -> bool {
    unsafe {
        let mut point = POINT::default();
        if GetCursorPos(&mut point).is_err()
            || !windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point).as_bool()
        {
            return false;
        }
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect).is_ok()
            && point.x >= rect.left
            && point.x < rect.right
            && point.y >= rect.top
            && point.y < rect.bottom
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
            // Switch rows hover as their pill column alone (draw + click
            // parity): clip the tooltip tool to that column too, so the
            // blank half between label and pill never pops a tip over
            // nothing. Rect is in the tool window's own client coords.
            let column = with_map(|m| m.get(&(control.0 as isize)).map_or(0, |t| t.hover_column));
            if column > 0 {
                let mut client = RECT::default();
                if GetClientRect(control, &mut client).is_ok() {
                    let rect = RECT {
                        left: (client.right - column).max(0),
                        top: 0,
                        right: client.right,
                        bottom: client.bottom,
                    };
                    let clipped = TTTOOLINFOW {
                        cbSize: size_of::<TTTOOLINFOW>() as u32,
                        uFlags: TTF_IDISHWND,
                        hwnd: parent,
                        uId: control.0 as usize,
                        rect,
                        ..Default::default()
                    };
                    SendMessageW(
                        tip,
                        TTM_NEWTOOLRECT,
                        None,
                        Some(LPARAM(&clipped as *const _ as isize)),
                    );
                }
            }
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
