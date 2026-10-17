//! Idle-action countdown warning: the topmost no-activate popup that
//! counts down to the configured idle action with a cancel escape.
//! Extracted from main; the idle monitor and IPC drive it through
//! show/tick/hide, and the panel's keyboard routing resolves its cancel
//! button via hwnd()/IDC_WARN_CANCEL.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use crate::panel_controls::{ControlSpec, set_control_text, static_text};
use crate::popups;
use crate::{FirstFrameGate, theme, viewport};

pub(crate) const IDC_WARN_TEXT: usize = 130;
pub(crate) const IDC_WARN_CANCEL: usize = 131;
const WARN_TIMER: usize = 2;

pub(crate) static WARNING: AtomicIsize = AtomicIsize::new(0);
static WARN_TEXT: AtomicIsize = AtomicIsize::new(0);
static WARNING_ACTIVE: AtomicBool = AtomicBool::new(false);
static WARN_SECONDS_LEFT: AtomicI32 = AtomicI32::new(0);
pub(crate) static WARNING_PREVIEW_SESSION: AtomicBool = AtomicBool::new(false);
static WARN_ACTION: Mutex<String> = Mutex::new(String::new());

fn hwnd(slot: &AtomicIsize) -> HWND {
    HWND(slot.load(Ordering::SeqCst) as *mut _)
}

pub(crate) fn hwnd_of_warning() -> HWND {
    hwnd(&WARNING)
}

fn scale(v: i32) -> i32 {
    crate::dpi::scale(v)
}

fn t(key: &str) -> String {
    crate::t_pub(key)
}

/// Creates the retained warning window and its two children (called once
/// from the startup window pass).
pub(crate) unsafe fn ensure_created(font: windows::Win32::Graphics::Gdi::HFONT) {
    unsafe {
        let warning = CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0),
            w!("IdleTriggerIdleWarning"),
            w!("IdleTrigger"),
            WINDOW_STYLE(WS_POPUP.0 | WS_SYSMENU.0),
            scale(600),
            scale(300),
            scale(430),
            scale(180),
            None,
            None,
            Some(GetModuleHandleW(None).expect("module handle").into()),
            None,
        )
        .expect("warning window");
        WARNING.store(warning.0 as isize, Ordering::SeqCst);
        crate::dpi::install(warning);

        let warn_text = static_text(&ControlSpec {
            parent: warning,
            label: String::new(),
            x: scale(20),
            y: scale(18),
            width: scale(380),
            id: IDC_WARN_TEXT,
            font,
        });
        WARN_TEXT.store(warn_text.0 as isize, Ordering::SeqCst);
        let _ = SetWindowPos(
            warn_text,
            None,
            0,
            0,
            scale(390),
            scale(84),
            SWP_NOMOVE | SWP_NOZORDER,
        );

        let warn_cancel = crate::kit::controls::raw_physical(
            warning,
            w!("BUTTON"),
            &t("common_cancel"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            IDC_WARN_CANCEL,
            font,
            (scale(160), scale(116), scale(120), scale(36)),
        );
        crate::nativeform::track(warn_cancel);
    }
}

pub(crate) unsafe extern "system" fn warning_proc(
    hwnd_: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    crate::guarded_proc("warning", hwnd_, msg, wparam, lparam, move || unsafe {
        match msg {
            WM_COMMAND if (wparam.0 & 0xFFFF) == IDC_WARN_CANCEL => {
                cancel_warning("user");
                LRESULT(0)
            }
            WM_CLOSE => {
                cancel_warning("user");
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == WARN_TIMER => {
                tick_warning();
                LRESULT(0)
            }
            WM_DRAWITEM => popups::draw_warning_button(lparam),
            windows::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                let mut client = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd_, &mut client);
                let _ = windows::Win32::Graphics::Gdi::FillRect(hdc, &client, theme::bg_brush());
                LRESULT(1)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_CTLCOLORSTATIC
            | windows::Win32::UI::WindowsAndMessaging::WM_CTLCOLORBTN
            | windows::Win32::UI::WindowsAndMessaging::WM_CTLCOLOREDIT
            | windows::Win32::UI::WindowsAndMessaging::WM_CTLCOLORLISTBOX => {
                crate::paint_static(wparam, lparam);
                LRESULT(theme::bg_brush().0 as isize)
            }
            _ => DefWindowProcW(hwnd_, msg, wparam, lparam),
        }
    })
}

pub(crate) fn show_warning() {
    if !WARNING_PREVIEW_SESSION.load(Ordering::SeqCst) {
        crate::sample_idle_clock();
        let countdown =
            crate::runtime::lock(&crate::IDLE_CLOCK).countdown(std::time::Instant::now());
        let Some((seconds, action)) = countdown else {
            return;
        };
        WARN_SECONDS_LEFT.store(seconds as i32, Ordering::SeqCst);
        *crate::runtime::lock(&WARN_ACTION) = action;
    }
    WARNING_ACTIVE.store(true, Ordering::SeqCst);
    if WARN_SECONDS_LEFT.load(Ordering::SeqCst) == 0 {
        tick_warning();
        return;
    }
    update_warning_text(WARN_SECONDS_LEFT.load(Ordering::SeqCst));
    unsafe {
        let warning = hwnd(&WARNING);
        if let Ok(cancel) = GetDlgItem(Some(warning), IDC_WARN_CANCEL as i32) {
            set_control_text(cancel, &t("common_cancel"));
        }
        popups::layout_warning(warning, hwnd(&WARN_TEXT), &[IDC_WARN_CANCEL]);
        center_on_screen(warning);
        viewport::fit(warning);
        FirstFrameGate::begin(warning).reveal_no_activate();
        let _ = SetTimer(Some(warning), WARN_TIMER, 1000, None);
    }
    crate::log_line("idle warning shown");
}

pub(crate) fn update_warning_text(seconds: i32) {
    let action = t(&format!(
        "menu_action_{}",
        crate::runtime::lock(&WARN_ACTION).clone()
    ));
    let text = crate::t_args("msg_idle_warning", &[&action, &seconds.to_string()]);
    crate::set_text(&WARN_TEXT, &text);
}

pub(crate) fn center_on_screen(hwnd_: HWND) {
    unsafe {
        let mut wr = RECT::default();
        if GetWindowRect(hwnd_, &mut wr).is_err() {
            return;
        }
        let width = wr.right - wr.left;
        let height = wr.bottom - wr.top;
        let anchor = windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
        let (x, y) = crate::display::centered_on(anchor, width, height);
        let _ = MoveWindow(hwnd_, x, y, width, height, false);
    }
}

pub(crate) fn tick_warning() {
    if !WARNING_ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    let (left, action) = if WARNING_PREVIEW_SESSION.load(Ordering::SeqCst) {
        (
            WARN_SECONDS_LEFT.fetch_sub(1, Ordering::SeqCst) - 1,
            crate::runtime::lock(&WARN_ACTION).clone(),
        )
    } else {
        crate::sample_idle_clock();
        let countdown =
            crate::runtime::lock(&crate::IDLE_CLOCK).countdown(std::time::Instant::now());
        let Some((seconds, action)) = countdown else {
            hide_warning("session no longer active");
            return;
        };
        (seconds as i32, action)
    };
    if left <= 0 {
        cancel_warning("timeout");
        crate::log_line("idle action executing");
        crate::execute_system_action(&action);
    } else {
        WARN_SECONDS_LEFT.store(left, Ordering::SeqCst);
        *crate::runtime::lock(&WARN_ACTION) = action;
        update_warning_text(left);
    }
}

pub(crate) fn cancel_warning(reason: &str) {
    crate::runtime::lock(&crate::IDLE_CLOCK).restart(std::time::Instant::now());
    WARNING_PREVIEW_SESSION.store(false, Ordering::SeqCst);
    hide_warning(reason);
}

pub(crate) fn hide_warning(reason: &str) {
    if !WARNING_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    unsafe {
        let warning = hwnd(&WARNING);
        let _ = KillTimer(Some(warning), WARN_TIMER);
        let _ = ShowWindow(warning, SW_HIDE);
    }
    crate::log_line(&format!("idle warning closed ({reason})"));
}

/// Devtools warning preview entry.
#[cfg(feature = "devtools")]
pub fn popups_show_warning_preview() {
    WARNING_PREVIEW_SESSION.store(true, Ordering::SeqCst);
    *crate::runtime::lock(&WARN_ACTION) = crate::cfg_map(|c| c.idle_action.clone());
    WARNING_ACTIVE.store(true, Ordering::SeqCst);
    WARN_SECONDS_LEFT.store(10, Ordering::SeqCst);
    show_warning();
}
