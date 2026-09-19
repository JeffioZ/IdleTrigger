//! Cancellable countdown window shown before automatic system actions.
//! The countdown lasts at least 10 seconds; cancel
//! stops the occurrence, timeout executes the action.
#![allow(clippy::manual_dangling_ptr)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, Ordering};

use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowRect, HMENU, KillTimer, LoadCursorW, LoadIconW,
    MoveWindow, RegisterClassW, SW_HIDE, SetTimer, SetWindowTextW, ShowWindow, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_TIMER, WNDCLASSW, WS_CHILD, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_SYSMENU, WS_VISIBLE,
};
use windows::core::PCWSTR;

use crate::automation::{self, ACTION_BUSY, PENDING_ACTION};

const IDC_TEXT: usize = 230;
const IDC_CANCEL: usize = 231;
const TIMER: usize = 7;

static WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static TEXT: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static SECONDS_LEFT: AtomicI32 = AtomicI32::new(0);
#[derive(Clone)]
struct Countdown {
    pending: automation::PendingAction,
    deadline: std::time::Instant,
}
static CURRENT: Mutex<Option<Countdown>> = Mutex::new(None);

const IDC_EXECUTE: usize = 232;

pub fn create() {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module handle");
        let font = crate::make_font_pub(14, 400);
        let class_name = windows::core::w!("IdleTriggerActionWarning");
        let icon =
            LoadIconW(Some(instance.into()), PCWSTR(1usize as *const u16)).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(action_wnd_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            hCursor: LoadCursorW(None, windows::Win32::UI::WindowsAndMessaging::IDC_ARROW)
                .unwrap_or_default(),
            hIcon: icon,
            hbrBackground: windows::Win32::Graphics::Gdi::GetSysColorBrush(
                windows::Win32::Graphics::Gdi::COLOR_WINDOW,
            ),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        // Tolerate ERROR_CLASS_ALREADY_EXISTS: DPI rebuilds re-enter this fn.
        if atom == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            panic!("RegisterClassW action warning failed");
        }
        let s = crate::scale_pub;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0),
            class_name,
            PCWSTR(wide(&crate::t_pub("app_title")).as_ptr()),
            WINDOW_STYLE(WS_POPUP.0 | WS_SYSMENU.0),
            s(600),
            s(300),
            s(430),
            s(180),
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("action warning window");
        WINDOW.store(hwnd.0 as isize, Ordering::SeqCst);
        crate::ACTION_WARN_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        crate::dpi::install(hwnd);
        crate::theme::apply_to_window(hwnd);

        let text = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("STATIC"),
            windows::core::w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            s(20),
            s(18),
            s(380),
            s(84),
            Some(hwnd),
            Some(HMENU(IDC_TEXT as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("action warning text");
        crate::set_control_font_pub(text, font);
        TEXT.store(text.0 as isize, Ordering::SeqCst);

        let cancel = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("BUTTON"),
            PCWSTR(wide(&crate::t_pub("automation_cancel_once")).as_ptr()),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | windows::Win32::UI::WindowsAndMessaging::WS_TABSTOP.0
                    | windows::Win32::UI::WindowsAndMessaging::BS_OWNERDRAW as u32,
            ),
            s(20),
            s(116),
            s(120),
            s(36),
            Some(hwnd),
            Some(HMENU(IDC_CANCEL as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("action warning cancel");
        crate::set_control_font_pub(cancel, font);
        crate::nativeform::track(cancel);

        // Run-now button: executes the pending action immediately (clears
        // the remaining queue like the Go OnExecute path).
        let execute = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("BUTTON"),
            PCWSTR(wide(&crate::t_pub("automation_execute_now")).as_ptr()),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | windows::Win32::UI::WindowsAndMessaging::WS_TABSTOP.0
                    | windows::Win32::UI::WindowsAndMessaging::BS_OWNERDRAW as u32,
            ),
            s(148),
            s(116),
            s(120),
            s(36),
            Some(hwnd),
            Some(HMENU(IDC_EXECUTE as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("action warning execute");
        crate::set_control_font_pub(execute, font);
        crate::nativeform::track(execute);
    }
}

use crate::wide;

/// Shows the countdown for the pending action, if any.
pub fn show_pending() {
    let Some(pending) = crate::runtime::lock(&PENDING_ACTION).take() else {
        return;
    };
    if !pending.is_current() {
        ACTION_BUSY.store(false, Ordering::SeqCst);
        return;
    }
    *crate::runtime::lock(&CURRENT) = Some(Countdown {
        pending: pending.clone(),
        deadline: std::time::Instant::now()
            + std::time::Duration::from_secs(pending.seconds as u64),
    });
    SECONDS_LEFT.store(pending.seconds, Ordering::SeqCst);
    ACTION_BUSY.store(true, Ordering::SeqCst);
    update_text(&pending, pending.seconds);
    unsafe {
        let hwnd = HWND(WINDOW.load(Ordering::SeqCst) as *mut _);
        for (id, key) in [
            (IDC_CANCEL, "automation_cancel_once"),
            (IDC_EXECUTE, "automation_execute_now"),
        ] {
            if let Ok(button) =
                windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(hwnd), id as i32)
            {
                let _ = SetWindowTextW(button, PCWSTR(wide(&crate::t_pub(key)).as_ptr()));
            }
        }
        layout_warning(
            hwnd,
            HWND(TEXT.load(Ordering::SeqCst) as *mut _),
            &[IDC_CANCEL, IDC_EXECUTE],
        );
        center(hwnd);
        crate::viewport::fit(hwnd);
        crate::FirstFrameGate::begin(hwnd).reveal_no_activate();
        let _ = SetTimer(Some(hwnd), TIMER, 1000, None);
    }
    crate::log_line(&format!(
        "action countdown started: {} in {}s (rule {})",
        pending.action, pending.seconds, pending.rule_id
    ));
}

pub fn default_button(hwnd: HWND) -> Option<HWND> {
    if hwnd.0 as isize != WINDOW.load(Ordering::SeqCst) {
        return None;
    }
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(hwnd), IDC_CANCEL as i32).ok()
    }
}

/// Measure wrapped content before showing; the shared viewport handles a small work area.
pub fn layout_warning(hwnd: HWND, label: HWND, buttons: &[usize]) {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    let _dpi = crate::dpi::Scope::window(hwnd);
    let s = crate::scale_pub;
    unsafe {
        crate::viewport::begin_layout(hwnd);
        let len = GetWindowTextLengthW(label).max(0) as usize;
        let mut text = vec![0u16; len + 1];
        let count = GetWindowTextW(label, &mut text).max(0) as usize;
        let dc = GetDC(Some(label));
        if dc.is_invalid() {
            return;
        }
        let font = SendMessageW(label, WM_GETFONT, None, None);
        let old = SelectObject(dc, HGDIOBJ(font.0 as *mut _));
        let mut rect = windows::Win32::Foundation::RECT {
            right: s(390),
            ..Default::default()
        };
        DrawTextW(
            dc,
            &mut text[..count],
            &mut rect,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(dc, old);
        ReleaseDC(Some(label), dc);
        let body = rect.bottom.max(s(64));
        let _ = MoveWindow(label, s(20), s(18), s(390), body, true);
        let width =
            (s(390) - s(8) * buttons.len().saturating_sub(1) as i32) / buttons.len().max(1) as i32;
        for (index, id) in buttons.iter().enumerate() {
            if let Ok(button) = GetDlgItem(Some(hwnd), *id as i32) {
                let _ = MoveWindow(
                    button,
                    s(20) + index as i32 * (width + s(8)),
                    s(34) + body,
                    width,
                    s(36),
                    true,
                );
            }
        }
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            s(430),
            body + s(88),
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn update_text(pending: &automation::PendingAction, seconds: i32) {
    let action_label = crate::t_pub(&format!("menu_action_{}", pending.action));
    let name = pending
        .rule
        .as_ref()
        .map_or(pending.rule_id.as_str(), |rule| rule.name.as_str());
    let text = crate::t_args(
        "automation_warning_body",
        &[name, &action_label, &seconds.to_string()],
    );
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = SetWindowTextW(
            HWND(TEXT.load(Ordering::SeqCst) as *mut _),
            PCWSTR(wide.as_ptr()),
        );
    }
}

unsafe fn center(hwnd: HWND) {
    unsafe {
        let mut wr = windows::Win32::Foundation::RECT::default();
        if GetWindowRect(hwnd, &mut wr).is_err() {
            return;
        }
        let w = wr.right - wr.left;
        let h = wr.bottom - wr.top;
        // Center on the monitor where the user currently works (the main
        // panel), not blindly on the primary screen.
        let anchor = HWND(crate::PANEL.load(Ordering::SeqCst) as *mut _);
        let (x, y) = if anchor.is_invalid() {
            crate::display::centered_on(hwnd, w, h)
        } else {
            crate::display::centered_on(anchor, w, h)
        };
        let _ = MoveWindow(hwnd, x, y, w, h, false);
    }
}

fn tick() {
    let current = crate::runtime::lock(&CURRENT).clone();
    let Some(current) = current else {
        return;
    };
    if !current.pending.is_current() {
        close(false);
        return;
    }
    let left = current
        .deadline
        .saturating_duration_since(std::time::Instant::now());
    let seconds = left.as_secs() + u64::from(left.subsec_nanos() != 0);
    if seconds == 0 {
        execute_now();
    } else {
        SECONDS_LEFT.store(seconds as i32, Ordering::SeqCst);
        update_text(&current.pending, seconds as i32);
    }
}

fn close(cancelled: bool) {
    let current = crate::runtime::lock(&CURRENT).take();
    if current.is_none() {
        return;
    }
    unsafe {
        let hwnd = HWND(WINDOW.load(Ordering::SeqCst) as *mut _);
        let _ = KillTimer(Some(hwnd), TIMER);
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    if cancelled {
        if let Some(current) = &current
            && current.pending.is_current()
            && current.pending.once_date.is_some()
        {
            automation::disable_rule_after_cancel(&current.pending.rule_id);
        }
        crate::log_line("action countdown cancelled");
    }
    ACTION_BUSY.store(false, Ordering::SeqCst);
}

fn execute_now() {
    let current = crate::runtime::lock(&CURRENT).clone();
    let Some(current) = current else {
        return;
    };
    let valid = current.pending.is_current();
    close(false);
    if valid {
        if current.pending.once_date.is_some() {
            automation::disable_rule_after_cancel(&current.pending.rule_id);
        }
        crate::log_line(&format!("action executing: {}", current.pending.action));
        crate::execute_system_action(&current.pending.action);
    }
}
unsafe extern "system" fn action_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND if (wparam.0 & 0xFFFF) == IDC_CANCEL => {
                close(true);
                LRESULT(0)
            }
            WM_COMMAND if (wparam.0 & 0xFFFF) == IDC_EXECUTE => {
                execute_now();
                LRESULT(0)
            }
            WM_CLOSE => {
                close(true);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER => {
                tick();
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_DRAWITEM => draw_warning_button(lparam),
            windows::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND => {
                erase_theme_bg(hwnd, wparam);
                LRESULT(1)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_CTLCOLORSTATIC => {
                paint_static_theme(wparam, lparam);
                LRESULT(crate::theme::bg_brush().0 as isize)
            }
            WM_DESTROY => {
                // A queued action can own the slot before this window shows
                // it. Destroying an idle window must not release that slot.
                if crate::runtime::lock(&CURRENT).take().is_some() {
                    ACTION_BUSY.store(false, Ordering::SeqCst);
                }
                WINDOW.store(0, Ordering::SeqCst);
                TEXT.store(0, Ordering::SeqCst);
                crate::ACTION_WARN_HWND.store(0, Ordering::SeqCst);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

pub fn draw_warning_button(lparam: LPARAM) -> LRESULT {
    let Some(item) = crate::nativeform::draw_item(lparam) else {
        return LRESULT(0);
    };
    let paint = |dc, rect: &windows::Win32::Foundation::RECT| unsafe {
        let mut text = [0u16; 128];
        let len = windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(item.control, &mut text);
        crate::paint::draw_button(
            dc,
            rect,
            crate::make_font_pub(14, 400),
            &String::from_utf16_lossy(&text[..len.max(0) as usize]),
            crate::theme::palette(),
            crate::theme::bg_color(),
            crate::nativeform::control_state(item.control, item.state),
            crate::paint::control_radius(),
        );
    };
    unsafe {
        if !crate::nativeform::draw_buffered(item.dc, &item.bounds, paint) {
            paint(item.dc, &item.bounds);
        }
    }
    LRESULT(1)
}

/// Themed background fill shared by the popup modules.
pub unsafe fn erase_theme_bg(hwnd: HWND, wparam: WPARAM) {
    use windows::Win32::Foundation::RECT;
    unsafe {
        let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
        let mut client = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
        let _ = windows::Win32::Graphics::Gdi::FillRect(hdc, &client, crate::theme::bg_brush());
    }
}

/// Themed static text: primary color on the themed background.
pub unsafe fn paint_static_theme(wparam: WPARAM, _lparam: LPARAM) {
    use windows::Win32::Graphics::Gdi::{SetBkColor, SetTextColor};
    unsafe {
        let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
        SetTextColor(
            hdc,
            windows::Win32::Foundation::COLORREF(crate::theme::text_color()),
        );
        SetBkColor(
            hdc,
            windows::Win32::Foundation::COLORREF(crate::theme::bg_color()),
        );
    }
}

// ---- Lock-key notices (Go locknotify parity) -----------------------------
// A brief non-activating layered card near the bottom of the foreground
// monitor: rounded corners, hairline border, soft shadow, fade + rise
// animation that follows the system "animate controls" setting.

use windows::Win32::Foundation::POINT;
use windows::Win32::Foundation::{COLORREF, SIZE};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, CreateFontIndirectW, DIB_RGB_COLORS, DT_CALCRECT, DT_CENTER, DT_SINGLELINE,
    DeleteDC, DeleteObject, DrawTextW, GdiFlush, GetMonitorInfoW, HBITMAP, HDC, HGDIOBJ, LOGFONTW,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows::Win32::UI::WindowsAndMessaging::UpdateLayeredWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW, WS_EX_TRANSPARENT,
};

const LN_TIMER: usize = 2;
const SHADOW_INSET: i32 = 10;
const CARD_W: i32 = 140;
const CARD_H: i32 = 88;
// Animation curve (Go): enter 120ms, hold 1500ms, exit 90ms.
const ENTER_MS: u128 = 120;
const HOLD_MS: u128 = 1500;
const EXIT_MS: u128 = 90;
/// Hold-phase opacity: a hint of translucency so the card sits softer on
/// desktop content (≈4%, visually subtle at any background). The entry and
/// exit curves use this as their endpoint so opacity is continuous across
/// phase boundaries.
const HOLD_OPACITY: u8 = 245;

pub const VK_CAPITAL: i32 = 0x14;
pub const VK_NUMLOCK: i32 = 0x90;
pub const VK_SCROLL: i32 = 0x91;

static LN_WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// A rendered premultiplied BGRA card (Go surface).
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    w: i32,
    h: i32,
    inset: i32,
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

unsafe impl Send for Surface {}

struct NoticeState {
    surface: Option<Surface>,
    shown: Option<std::time::Instant>,
    alpha: u8,
    initial_alpha: u8,
    rise: i32,
    animated: bool,
    position: (i32, i32),
}

static NOTICE: Mutex<Option<NoticeState>> = Mutex::new(None);

fn notice() -> std::sync::MutexGuard<'static, Option<NoticeState>> {
    crate::runtime::lock(&NOTICE)
}

pub fn lock_create() {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module handle");
        let class_name = windows::core::w!("IdleTriggerLockNotify");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(lock_wnd_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            hCursor: LoadCursorW(None, windows::Win32::UI::WindowsAndMessaging::IDC_ARROW)
                .unwrap_or_default(),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        // Tolerate ERROR_CLASS_ALREADY_EXISTS: DPI rebuilds re-enter this fn.
        if atom == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            panic!("RegisterClassW lock notify failed");
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(
                WS_EX_LAYERED.0
                    | WS_EX_TRANSPARENT.0
                    | WS_EX_NOACTIVATE.0
                    | WS_EX_TOPMOST.0
                    | WS_EX_TOOLWINDOW.0,
            ),
            class_name,
            windows::core::w!("IdleTrigger"),
            WINDOW_STYLE(WS_POPUP.0),
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("lock notify window");
        LN_WINDOW.store(hwnd.0 as isize, Ordering::SeqCst);
        crate::LOCK_NOTIFY_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
    }
}

/// Grayscale-AA message-font derivative (Go notificationFont): subpixel
/// fringes do not survive alpha composition over arbitrary desktop content.
fn notification_font(size_px: i32, weight: i32) -> windows::Win32::Graphics::Gdi::HFONT {
    unsafe {
        let mut metrics = windows::Win32::UI::WindowsAndMessaging::NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<windows::Win32::UI::WindowsAndMessaging::NONCLIENTMETRICSW>(
            ) as u32,
            ..Default::default()
        };
        if SystemParametersInfoW(
            windows::Win32::UI::WindowsAndMessaging::SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            Some(&mut metrics as *mut _ as *mut core::ffi::c_void),
            Default::default(),
        )
        .is_err()
        {
            return windows::Win32::Graphics::Gdi::HFONT::default();
        }
        let mut lf: LOGFONTW = metrics.lfMessageFont;
        lf.lfHeight = -size_px.max(1);
        lf.lfWeight = weight;
        lf.lfQuality = windows::Win32::Graphics::Gdi::ANTIALIASED_QUALITY;
        CreateFontIndirectW(&lf)
    }
}

struct Measured {
    width: i32,
    height: i32,
}

fn text_bounds(dc: HDC, font: windows::Win32::Graphics::Gdi::HFONT, text: &str) -> Measured {
    unsafe {
        let old = SelectObject(dc, HGDIOBJ(font.0));
        let mut value: Vec<u16> = text.encode_utf16().collect();
        let mut bounds = windows::Win32::Foundation::RECT::default();
        let _ = DrawTextW(dc, &mut value, &mut bounds, DT_CALCRECT | DT_SINGLELINE);
        SelectObject(dc, old);
        Measured {
            width: bounds.right - bounds.left,
            height: bounds.bottom - bounds.top,
        }
    }
}

/// Renders the rounded card with border rim and soft shadow (Go
/// renderSurface + composeSurface).
#[allow(clippy::chunks_exact_to_as_chunks)]
fn render_surface(dpi: u32, dark: bool, on: bool, symbol: &str, text: &str) -> Option<Surface> {
    let scale = |v: i32| -> i32 { ((v as i64 * dpi as i64 + 48) / 96) as i32 };
    let p = crate::theme::palette();
    let title_font = notification_font(scale(28), 500);
    let label_font = notification_font(scale(15), 400);
    struct Fonts(
        windows::Win32::Graphics::Gdi::HFONT,
        windows::Win32::Graphics::Gdi::HFONT,
    );
    impl Drop for Fonts {
        fn drop(&mut self) {
            unsafe {
                if !self.0.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.0.0));
                }
                if !self.1.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.1.0));
                }
            }
        }
    }
    let _fonts = Fonts(title_font, label_font);
    if title_font.is_invalid() || label_font.is_invalid() {
        return None;
    }
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let title = text_bounds(dc, title_font, symbol);
        let label = text_bounds(dc, label_font, text);
        let (gap, pad) = (scale(4), scale(SHADOW_INSET));
        // Default footprint stays CARD_W logical px; larger text grows the
        // card (Go parity). Do not scale CARD_W through CARD_H: the rounded
        // division below inflates fractional DPI (e.g. 150% -> scale(1)=2).
        let width = (title.width.max(label.width) + scale(28)).max(scale(CARD_W));
        let content_h = title.height + gap + label.height;
        let height = scale(CARD_H).max(content_h + scale(22));
        let (sw, sh) = (width + 2 * pad, height + 2 * pad);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: 40,
                biWidth: sw,
                biHeight: -sh, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => bitmap,
            Err(_) => {
                let _ = DeleteDC(dc);
                return None;
            }
        };
        if bitmap.is_invalid() || bits.is_null() {
            if !bitmap.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
            let _ = DeleteDC(dc);
            return None;
        }
        let old = SelectObject(dc, HGDIOBJ(bitmap.0));
        let pixels = bits as *mut u8;
        let count = (sw * sh * 4) as usize;
        // Opaque face fill; alpha is composed after the GDI text pass.
        let face = p.window_bg;
        let (fb, fg, fr) = ((face >> 16) as u8, (face >> 8) as u8, face as u8);
        let slice = std::slice::from_raw_parts_mut(pixels, count);
        for px in slice.chunks_exact_mut(4) {
            px[0] = fb;
            px[1] = fg;
            px[2] = fr;
            px[3] = 255;
        }
        SetBkMode(dc, TRANSPARENT);
        let mut ink = p.text2;
        if on {
            ink = if dark { p.focus } else { p.accent };
        }
        let mut top = pad + (height - content_h) / 2 - scale(1);
        for (i, (font, line, color)) in [(title_font, symbol, ink), (label_font, text, p.text)]
            .into_iter()
            .enumerate()
        {
            let measured = if i == 0 { &title } else { &label };
            let mut box_ = windows::Win32::Foundation::RECT {
                left: pad + scale(12),
                top,
                right: pad + width - scale(12),
                bottom: top + measured.height,
            };
            let old_font = SelectObject(dc, HGDIOBJ(font.0));
            SetTextColor(dc, COLORREF(color));
            let mut value: Vec<u16> = line.encode_utf16().collect();
            let _ = DrawTextW(dc, &mut value, &mut box_, DT_CENTER | DT_SINGLELINE);
            SelectObject(dc, old_font);
            top = box_.bottom + gap;
        }
        let _ = GdiFlush();

        // composeSurface: rounded-rect SDF coverage, border rim, soft shadow.
        let scale_f = dpi as f64 / 96.0;
        let (half_w, half_h) = (sw as f64 / 2.0, sh as f64 / 2.0);
        let (card_hw, card_hh) = (half_w - pad as f64, half_h - pad as f64);
        let (radius, sigma) = (9.0 * scale_f, 3.0 * scale_f);
        let strength = if dark { 0.12 } else { 0.06 };
        let border = p.subtle_border;
        let (bb, bg_, br_) = ((border >> 16) as u8, (border >> 8) as u8, border as u8);
        for y in 0..sh {
            for x in 0..sw {
                let (px, py) = (x as f64 + 0.5 - half_w, y as f64 + 0.5 - half_h);
                let qx = (px.abs() - card_hw + radius).max(0.0);
                let qy = (py.abs() - card_hh + radius).max(0.0);
                let d = (qx.hypot(qy) + qx.max(qy).min(0.0)) - radius;
                let coverage = (0.5 - d).clamp(0.0, 1.0);
                let inner = (0.5 - d - 0.75 * scale_f).clamp(0.0, 1.0);
                let rim = coverage - inner;
                let shadow_d = d.max(0.0);
                let shadow = strength * (-shadow_d * shadow_d / (2.0 * sigma * sigma)).exp();
                let alpha = coverage + shadow * (1.0 - coverage);
                let index = ((y * sw + x) * 4) as usize;
                for (c, border_channel) in [(0usize, bb), (1, bg_), (2, br_)] {
                    let v = slice[index + c] as f64 * inner + border_channel as f64 * rim;
                    slice[index + c] = v.round() as u8;
                }
                slice[index + 3] = (alpha * 255.0).round() as u8;
            }
        }
        Some(Surface {
            dc,
            bitmap,
            old,
            w: sw,
            h: sh,
            inset: pad,
        })
    }
}

impl Surface {
    /// Inspect the actual layered-card renderer without reading the desktop.
    #[cfg(all(test, feature = "devtools"))]
    fn save_preview(&self, path: &std::path::Path) -> std::io::Result<()> {
        unsafe {
            let mut pixels = vec![0u8; (self.w * self.h * 4) as usize];
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: 40,
                    biWidth: self.w,
                    biHeight: -self.h,
                    biPlanes: 1,
                    biBitCount: 32,
                    ..Default::default()
                },
                ..Default::default()
            };
            SelectObject(self.dc, self.old);
            let lines = windows::Win32::Graphics::Gdi::GetDIBits(
                self.dc,
                self.bitmap,
                0,
                self.h as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            );
            SelectObject(self.dc, HGDIOBJ(self.bitmap.0));
            if lines != self.h {
                return Err(std::io::Error::other("card bitmap readback failed"));
            }
            let mut bytes = vec![0u8; 54];
            bytes[..2].copy_from_slice(b"BM");
            bytes[2..6].copy_from_slice(&(54 + pixels.len() as u32).to_le_bytes());
            bytes[10..14].copy_from_slice(&54u32.to_le_bytes());
            bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
            bytes[18..22].copy_from_slice(&self.w.to_le_bytes());
            bytes[22..26].copy_from_slice(&(-self.h).to_le_bytes());
            bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
            bytes[28..30].copy_from_slice(&32u16.to_le_bytes());
            bytes.extend(pixels);
            std::fs::write(path, bytes)
        }
    }
    /// UpdateLayeredWindow with per-pixel premultiplied alpha and a global
    /// opacity factor (Go present).
    unsafe fn present(&self, hwnd: HWND, position: (i32, i32), alpha: u8) -> bool {
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                None,
                Some(&POINT {
                    x: position.0,
                    y: position.1,
                }),
                Some(&SIZE {
                    cx: self.w,
                    cy: self.h,
                }),
                Some(self.dc),
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: alpha,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                }),
                windows::Win32::UI::WindowsAndMessaging::ULW_ALPHA,
            )
            .is_ok()
        }
    }
}

#[cfg(all(test, feature = "devtools"))]
pub(crate) fn capture_lock_preview(
    dpi: u32,
    vk: i32,
    on: bool,
    path: &std::path::Path,
) -> std::io::Result<()> {
    let (symbol, name) = match vk {
        VK_CAPITAL => (if on { "AA" } else { "aa" }, "Caps Lock"),
        VK_NUMLOCK => ("123", "Num Lock"),
        _ => ("\u{2195}", "Scroll Lock"),
    };
    let text = format!(
        "{} {}",
        name,
        crate::t_pub(if on { "lock_keys_on" } else { "lock_keys_off" })
    );
    render_surface(dpi, crate::theme::is_dark(), on, symbol, &text)
        .ok_or_else(|| std::io::Error::other("card render failed"))?
        .save_preview(path)
}

fn client_area_animations() -> bool {
    unsafe {
        let mut value: i32 = 0;
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut value as *mut i32 as *mut core::ffi::c_void),
            Default::default(),
        )
        .ok();
        value != 0
    }
}

fn entry_progress(elapsed_ms: u128) -> f64 {
    let t = (elapsed_ms.min(ENTER_MS) as f64) / ENTER_MS as f64;
    1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t)
}

fn opacity_at(elapsed_ms: u128, initial: u8, animated: bool) -> u8 {
    if elapsed_ms >= HOLD_MS + EXIT_MS {
        return 0;
    }
    if !animated {
        return 255;
    }
    if elapsed_ms < ENTER_MS {
        return initial
            + (HOLD_OPACITY.saturating_sub(initial) as f64 * entry_progress(elapsed_ms)) as u8;
    }
    if elapsed_ms > HOLD_MS {
        let t = (elapsed_ms - HOLD_MS) as f64 / EXIT_MS as f64;
        return (HOLD_OPACITY as f64 * (1.0 - t * t)) as u8;
    }
    HOLD_OPACITY
}

fn rise_at(elapsed_ms: u128, rise: i32, animated: bool) -> i32 {
    if !animated {
        return 0;
    }
    (rise as f64 * (1.0 - entry_progress(elapsed_ms)) + 0.5) as i32
}

fn hide_notice() {
    unsafe {
        let hwnd = HWND(LN_WINDOW.load(Ordering::SeqCst) as *mut _);
        let _ = KillTimer(Some(hwnd), LN_TIMER);
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    if let Some(state) = notice().as_mut() {
        state.shown = None;
        state.alpha = 0;
    }
}

fn animate_notice(hwnd: HWND) {
    let (elapsed, animated, rise) = {
        let guard = notice();
        let Some(state) = guard.as_ref() else {
            return;
        };
        (
            state
                .shown
                .map(|t| t.elapsed().as_millis())
                .unwrap_or(u128::MAX),
            state.animated,
            state.rise,
        )
    };
    if elapsed >= HOLD_MS + EXIT_MS {
        hide_notice();
        return;
    }
    let (mut position, alpha) = {
        let mut guard = notice();
        let Some(state) = guard.as_mut() else {
            return;
        };
        state.alpha = opacity_at(elapsed, state.initial_alpha, animated);
        (state.position, state.alpha)
    };
    position.1 += rise_at(elapsed, rise, animated);
    let ok = notice()
        .as_ref()
        .and_then(|s| s.surface.as_ref())
        .map(|surface| unsafe { surface.present(hwnd, position, alpha) })
        .unwrap_or(false);
    if !ok {
        hide_notice();
    }
}

pub fn show(vk: i32, on: bool) {
    unsafe {
        let hwnd = HWND(LN_WINDOW.load(Ordering::SeqCst) as *mut _);
        if hwnd.is_invalid() {
            return;
        }
        // System-action countdowns take priority; drop stale key notices.
        let warning = crate::hwnd(&crate::ACTION_WARN_HWND);
        if !warning.is_invalid()
            && windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(warning).as_bool()
        {
            hide_notice();
            return;
        }
        let (symbol, name) = match vk {
            VK_CAPITAL => (if on { "AA" } else { "aa" }, "Caps Lock"),
            VK_NUMLOCK => ("123", "Num Lock"),
            _ => ("\u{2195}", "Scroll Lock"),
        };
        let text = format!(
            "{} {}",
            name,
            crate::t_pub(if on { "lock_keys_on" } else { "lock_keys_off" })
        );
        let foreground = GetForegroundWindow();
        if foreground.is_invalid() {
            return;
        }
        let monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        // Follow the target monitor so our DPI read matches the card scale.
        if MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) != monitor {
            hide_notice();
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                hwnd,
                None,
                info.rcWork.left,
                info.rcWork.top,
                0,
                0,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                    | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                    | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
            );
        }
        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = |v: i32| ((v as i64 * dpi as i64 + 48) / 96) as i32;
        let Some(surface) = render_surface(dpi, crate::theme::is_dark(), on, symbol, &text) else {
            hide_notice();
            return;
        };
        let (sw, sh, inset) = (surface.w, surface.h, surface.inset);
        let mut position = (
            info.rcWork.left + (info.rcWork.right - info.rcWork.left - sw) / 2,
            info.rcWork.bottom - scale(48) - sh + inset,
        );
        // A card larger than the work area (huge accessibility text on a
        // small remote-session desktop) must not invert the clamp bounds:
        // std clamps panic when min > max, so pin max at min first.
        let max_x = (info.rcWork.right - sw).max(info.rcWork.left);
        let max_y = (info.rcWork.bottom - sh - scale(4)).max(info.rcWork.top);
        position.0 = position.0.clamp(info.rcWork.left, max_x);
        position.1 = position.1.clamp(info.rcWork.top, max_y);
        let animated = client_area_animations();
        {
            let mut guard = notice();
            let state = guard.get_or_insert_with(|| NoticeState {
                surface: None,
                shown: None,
                alpha: 0,
                initial_alpha: 0,
                rise: 0,
                animated: false,
                position: (0, 0),
            });
            state.surface = Some(surface);
            state.initial_alpha = state.alpha;
            state.rise = scale(4);
            state.animated = animated;
            state.position = position;
            state.shown = Some(std::time::Instant::now());
        }
        animate_notice(hwnd);
        if reveal_notice(hwnd).is_err() {
            hide_notice();
            return;
        }
        if SetTimer(Some(hwnd), LN_TIMER, 15, None) == 0 {
            hide_notice();
        }
    }
}

fn reveal_notice(hwnd: HWND) -> windows::core::Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
    };
    unsafe {
        // Reassert the notification's Z-order on every reveal, including
        // repeated key changes while the card is already visible. The window
        // never takes activation, and animation ticks do not reorder windows.
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    }
}

unsafe extern "system" fn lock_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            // Never activate, including from active-window tracking.
            windows::Win32::UI::WindowsAndMessaging::WM_MOUSEACTIVATE => LRESULT(3),
            WM_TIMER if wparam.0 == LN_TIMER => {
                animate_notice(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                *notice() = None;
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Polls the three lock keys; posts `WM_LOCK_NOTIFY` on transitions.
///
/// Must run on the message-pumping UI thread (hidden-window WM_TIMER):
/// `GetKeyState`'s toggle bit only updates for a thread that reads keyboard
/// messages from its own queue, so a worker thread would see a frozen
/// snapshot. Go sampled from the notification window's own timer.
pub fn poll() {
    static LAST_STATES: std::sync::Mutex<[(i32, i16); 3]> =
        std::sync::Mutex::new([(VK_CAPITAL, 0), (VK_NUMLOCK, 0), (VK_SCROLL, 0)]);
    static SEEDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let mut last_states = crate::runtime::lock(&LAST_STATES);
    if !SEEDED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        // Seed the history once so keys already toggled at startup do not
        // fire a notice (Go baseline=false).
        for (vk, last) in last_states.iter_mut() {
            *last = key_toggled(*vk);
        }
        return;
    }
    let (enabled, caps, num, scroll) = crate::cfg_map(|c| {
        (
            c.lock_keys_enabled,
            c.lock_keys_caps_enabled,
            c.lock_keys_num_enabled,
            c.lock_keys_scroll_enabled,
        )
    });
    if !enabled {
        for (vk, last) in last_states.iter_mut() {
            *last = key_toggled(*vk);
        }
        return;
    }
    // Fullscreen / presentation suppression (Go contract: hide during
    // fullscreen apps, do not replay skipped notices).
    let suppressed = crate::cfg_map(|c| c.lock_keys_skip_fullscreen)
        && crate::display::foreground_is_fullscreen();
    if suppressed {
        // Keep last_states fresh so returning from fullscreen doesn't
        // replay the missed transitions.
        for (vk, last) in last_states.iter_mut() {
            *last = key_toggled(*vk);
        }
        return;
    }
    let allowed = |vk: i32| match vk {
        VK_CAPITAL => caps,
        VK_NUMLOCK => num,
        _ => scroll,
    };
    for (vk, last) in last_states.iter_mut() {
        let state = key_toggled(*vk);
        if state != *last && allowed(*vk) {
            #[cfg(feature = "devtools")]
            crate::devtools::trace_input(&format!("lock key {vk}: {state}"));
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    Some(crate::hwnd(&crate::HIDDEN)),
                    crate::WM_LOCK_NOTIFY,
                    WPARAM(*vk as usize),
                    LPARAM(state as isize),
                );
            }
        }
        *last = state;
    }
}

fn key_toggled(vk: i32) -> i16 {
    unsafe { (GetKeyState(vk) as u16 & 1) as i16 }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    #[test]
    fn lock_notice_restores_topmost_without_activating() {
        let _guard = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
            use windows::Win32::UI::WindowsAndMessaging::*;
            // Only manipulate windows created by this test. The ordinary
            // window models an ordinary overlapping application; it does
            // not simulate the Windows Snap shell or Edge's compositor.
            let ordinary = CreateWindowExW(
                WS_EX_NOACTIVATE,
                windows::core::w!("STATIC"),
                windows::core::w!("IdleTrigger Z-order test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                600,
                600,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            lock_create();
            let card = HWND(LN_WINDOW.load(Ordering::SeqCst) as *mut _);
            SetWindowPos(
                card,
                None,
                520,
                420,
                160,
                108,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            let foreground = GetForegroundWindow();
            let active = GetActiveWindow();
            for hidden in [true, false, true] {
                if hidden {
                    let _ = ShowWindow(card, SW_HIDE);
                }
                // Model a stale/lost topmost state between notifications.
                SetWindowPos(
                    card,
                    Some(HWND_NOTOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                )
                .unwrap();
                let _ = ShowWindow(ordinary, SW_SHOWNOACTIVATE);
                let mut bounds_before = windows::Win32::Foundation::RECT::default();
                GetWindowRect(card, &mut bounds_before).unwrap();
                reveal_notice(card).unwrap();
                let mut bounds_after = windows::Win32::Foundation::RECT::default();
                GetWindowRect(card, &mut bounds_after).unwrap();
                assert_eq!(bounds_after, bounds_before, "reveal changed card geometry");
                assert!(IsWindowVisible(card).as_bool());
                assert_ne!(
                    GetWindowLongW(card, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0,
                    0
                );
                assert_eq!(GetForegroundWindow(), foreground, "notice stole foreground");
                assert_eq!(GetActiveWindow(), active, "notice activated its UI thread");
                let mut above = GetWindow(card, GW_HWNDPREV).unwrap_or_default();
                while !above.is_invalid() {
                    assert_ne!(above, ordinary, "ordinary window covers the notice");
                    above = GetWindow(above, GW_HWNDPREV).unwrap_or_default();
                }
            }
            DestroyWindow(card).unwrap();
            DestroyWindow(ordinary).unwrap();
            LN_WINDOW.store(0, Ordering::SeqCst);
            crate::LOCK_NOTIFY_HWND.store(0, Ordering::SeqCst);
        }
    }
    #[test]
    fn long_warning_keeps_buttons_below_the_complete_body() {
        let _guard = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::*;
            let form = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WS_POPUP,
                0,
                0,
                430,
                180,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let long = wide(&"A long task name with spaces and 中文名称. ".repeat(20));
            let label = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                PCWSTR(long.as_ptr()),
                WS_CHILD,
                0,
                0,
                100,
                20,
                Some(form),
                None,
                None,
                None,
            )
            .unwrap();
            let button = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("BUTTON"),
                windows::core::w!("Cancel"),
                WS_CHILD,
                0,
                0,
                100,
                20,
                Some(form),
                Some(HMENU(231usize as *mut _)),
                None,
                None,
            )
            .unwrap();
            layout_warning(form, label, &[231]);
            let mut body = windows::Win32::Foundation::RECT::default();
            let mut action = body;
            GetWindowRect(label, &mut body).unwrap();
            GetWindowRect(button, &mut action).unwrap();
            assert!(body.bottom - body.top > crate::scale_pub(84));
            assert!(action.top > body.bottom);
            crate::dpi::install(form);
            crate::viewport::fit(form);
            crate::viewport::reveal_control(form, button);
            GetWindowRect(button, &mut action).unwrap();
            let work = crate::display::work_area_for(form);
            assert!(action.top >= work.top && action.bottom <= work.bottom);
            DestroyWindow(form).unwrap();
        }
    }
}
