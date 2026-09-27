//! Native tray icon and context menu on Shell_NotifyIconW, hosted on the
//! hidden window (Go tray parity; replaced the tray-icon/muda crates).
//! Left click toggles the control panel; right click opens the popup menu
//! built fresh at popup time so language and theme are always current.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, GetMenuInfo,
    GetSystemMetrics, HICON, HMENU, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, MENUINFO,
    MF_SEPARATOR, MF_STRING, MIM_STYLE, MNS_NOCHECK, PostMessageW, PostQuitMessage,
    RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, SetMenuInfo, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_LBUTTONUP,
    WM_NULL, WM_RBUTTONUP,
};
use windows::core::PCWSTR;

use crate::runtime;

/// Old-style callback contract (the previous tray-icon crate used the same
/// pre-version-4 form): lParam carries the mouse message, wParam the icon id.
/// Reserved in main.rs's app-message block.
pub const CALLBACK_MSG: u32 = 0x8001;

const TRAY_ID: u32 = 1;
// Popup commands, returned by TrackPopupMenu with TPM_RETURNCMD.
const CMD_OPEN_PANEL: usize = 1;
const CMD_EXIT: usize = 2;
// Go resourceid: tray-dark (3) shows on light mode, tray-light (4) on dark.
const RES_DARK_STROKES: i32 = 3;
const RES_LIGHT_STROKES: i32 = 4;
const TIP_DEFAULT: &str = "IdleTrigger";

static HOST: AtomicIsize = AtomicIsize::new(0);
static ICON: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
// Last theme the tray icon was rendered for (Go s.trayThemeDark).
static THEME_DARK: AtomicBool = AtomicBool::new(false);
// Last accepted tooltip; re-applied when the taskbar is recreated.
static TIP: Mutex<String> = Mutex::new(String::new());

/// Adds the tray icon for the active theme. Only call on the UI thread.
pub fn add(host: HWND) -> bool {
    register_taskbar_created();
    let Some(icon) = load_theme_hicon() else {
        return false;
    };
    let mut nid = nid_base(host, NIF_MESSAGE | NIF_ICON | NIF_TIP);
    nid.hIcon = icon;
    nid.szTip = make_tip(TIP_DEFAULT);
    if !notify(NIM_ADD, &nid) {
        unsafe {
            let _ = DestroyIcon(icon);
        }
        return false;
    }
    HOST.store(host.0 as isize, Ordering::SeqCst);
    ICON.store(icon.0 as isize, Ordering::SeqCst);
    THEME_DARK.store(crate::theme::is_dark(), Ordering::SeqCst);
    *runtime::lock(&TIP) = TIP_DEFAULT.to_string();
    true
}

/// Removes the tray icon and frees its HICON. Only call on the UI thread.
pub fn remove() {
    let host = HWND(HOST.swap(0, Ordering::SeqCst) as *mut core::ffi::c_void);
    let icon = HICON(ICON.swap(0, Ordering::SeqCst) as *mut core::ffi::c_void);
    if host.is_invalid() {
        return;
    }
    let mut nid = nid_base(host, NIF_ICON);
    nid.hIcon = icon;
    let _ = notify(NIM_DELETE, &nid);
    if !icon.is_invalid() {
        unsafe {
            let _ = DestroyIcon(icon);
        }
    }
}

/// True when `msg` is the registered TaskbarCreated broadcast.
pub fn is_taskbar_created(msg: u32) -> bool {
    let registered = TASKBAR_CREATED.load(Ordering::SeqCst);
    registered != 0 && msg == registered
}

/// Explorer restarted: re-add the icon with the current icon and tooltip.
pub fn on_taskbar_created() {
    let host = HWND(HOST.load(Ordering::SeqCst) as *mut core::ffi::c_void);
    let icon = HICON(ICON.load(Ordering::SeqCst) as *mut core::ffi::c_void);
    if host.is_invalid() || icon.is_invalid() {
        return;
    }
    let mut nid = nid_base(host, NIF_MESSAGE | NIF_ICON | NIF_TIP);
    nid.hIcon = icon;
    nid.szTip = make_tip(&runtime::lock(&TIP).clone());
    let _ = notify(NIM_ADD, &nid);
}

/// Dispatches a tray callback (old-style lParam mouse message). Only call on
/// the UI thread: left click toggles the panel, right click opens the menu.
pub fn handle_callback(wparam: WPARAM, lparam: LPARAM) {
    let _ = wparam; // Always TRAY_ID; nothing distinguishes multiple icons.
    match lparam.0 as u32 {
        WM_LBUTTONUP => crate::toggle_panel(),
        WM_RBUTTONUP => show_context_menu(host()),
        _ => {}
    }
}

/// Updates the tooltip; returns true when the shell accepted the modify so
/// callers can cache the text and retry after a failed round trip.
pub fn set_tooltip(text: &str) -> bool {
    let host = host();
    if host.is_invalid() {
        return false;
    }
    let mut nid = nid_base(host, NIF_TIP);
    nid.szTip = make_tip(text);
    if notify(NIM_MODIFY, &nid) {
        *runtime::lock(&TIP) = text.to_string();
        true
    } else {
        false
    }
}

/// Swaps the tray icon when the theme flipped (Go refreshTrayThemeIcon on
/// the tray tick). Only call on the UI thread.
pub fn refresh_theme_icon() {
    let dark = crate::theme::is_dark();
    if dark == THEME_DARK.swap(dark, Ordering::SeqCst) {
        return;
    }
    let host = host();
    if host.is_invalid() {
        return;
    }
    if let Some(icon) = load_theme_hicon() {
        let mut nid = nid_base(host, NIF_ICON);
        nid.hIcon = icon;
        if notify(NIM_MODIFY, &nid) {
            let old = HICON(ICON.swap(icon.0 as isize, Ordering::SeqCst) as *mut core::ffi::c_void);
            if !old.is_invalid() {
                unsafe {
                    let _ = DestroyIcon(old);
                }
            }
        } else {
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
    }
}

/// Shows the tray menu anchored at the cursor. Also the devtools capture
/// entry point, where `owner` is the panel instead of the hidden window.
pub fn show_context_menu(owner: HWND) {
    unsafe {
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let Some(menu) = build_menu() else {
            return;
        };
        theme_prep_and_track(menu, owner, point.x, point.y);
    }
}

fn host() -> HWND {
    HWND(HOST.load(Ordering::SeqCst) as *mut core::ffi::c_void)
}

/// Builds the tray menu: Open / separator / Exit. Created fresh at popup
/// time so labels track the active language and DPI.
fn build_menu() -> Option<HMENU> {
    let menu = unsafe { CreatePopupMenu() }.unwrap_or_default();
    if menu.is_invalid() {
        return None;
    }
    let open: Vec<u16> = crate::t("menu_open_panel")
        .encode_utf16()
        .chain([0])
        .collect();
    let exit: Vec<u16> = crate::t("menu_exit").encode_utf16().chain([0]).collect();
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN_PANEL, PCWSTR(open.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, CMD_EXIT, PCWSTR(exit.as_ptr()));
        // These text-only actions need no checkmark gutter. Keep native text
        // measurement so each language and DPI gets its own compact menu width.
        let mut info = MENUINFO {
            cbSize: std::mem::size_of::<MENUINFO>() as u32,
            fMask: MIM_STYLE,
            ..Default::default()
        };
        if GetMenuInfo(menu, &mut info).is_ok() {
            info.dwStyle |= MNS_NOCHECK;
            let _ = SetMenuInfo(menu, &info);
        }
    }
    Some(menu)
}

/// Prepares the immersive theme, then tracks the menu above-right of the
/// anchor like a notification-area menu. TPM_RETURNCMD carries the choice.
fn theme_prep_and_track(menu: HMENU, owner: HWND, x: i32, y: i32) {
    unsafe {
        crate::theme::prepare_popup_menu(owner, crate::theme::is_dark());
        // Foreground first so the menu dismisses on any click outside.
        let _ = SetForegroundWindow(owner);
        let choice = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY | TPM_BOTTOMALIGN | TPM_LEFTALIGN,
            x,
            y,
            None,
            owner,
            None,
        )
        .0;
        // The shell docs recommend a benign post-menu message so the task
        // switch is finalized correctly.
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match choice as usize {
            CMD_OPEN_PANEL => crate::toggle_panel(),
            CMD_EXIT => PostQuitMessage(0),
            _ => {}
        }
    }
}

/// The tray icon matching the active theme (Go updateIcon mapping): tray-dark
/// (3) on light mode, tray-light (4) on dark, app icon as the fallback.
fn load_theme_hicon() -> Option<HICON> {
    let id = if crate::theme::is_dark() {
        RES_LIGHT_STROKES
    } else {
        RES_DARK_STROKES
    };
    load_resource_hicon(id).or_else(|| load_resource_hicon(1))
}

/// Loads an embedded icon from the running module, independent of path
/// length. Sized to the notification-area metric so each DPI picks its own
/// crisp ICO entry (16/20/24/32/40/48/64) instead of a scaled 32; clamped
/// because a broken metric must never request a size the ICOs don't carry.
fn load_resource_hicon(resource_id: i32) -> Option<HICON> {
    unsafe {
        let module = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok()?;
        let size = GetSystemMetrics(SM_CXSMICON).clamp(16, 64);
        let handle = LoadImageW(
            Some(module.into()),
            PCWSTR(resource_id as usize as *const u16),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        )
        .ok()?;
        Some(HICON(handle.0))
    }
}

fn nid_base(
    host: HWND,
    flags: windows::Win32::UI::Shell::NOTIFY_ICON_DATA_FLAGS,
) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: host,
        uID: TRAY_ID,
        uFlags: flags,
        uCallbackMessage: CALLBACK_MSG,
        ..Default::default()
    }
}

fn notify(cmd: windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE, nid: &NOTIFYICONDATAW) -> bool {
    unsafe { Shell_NotifyIconW(cmd, nid).as_bool() }
}

/// Builds a NOTIFYICONDATAW tooltip buffer (127 UTF-16 + NUL). Value
/// semantics: i686 packs NOTIFYICONDATAW, so field references must not form.
fn make_tip(text: &str) -> [u16; 128] {
    let mut tip = [0u16; 128];
    for (slot, unit) in tip.iter_mut().zip(text.encode_utf16().take(127)) {
        *slot = unit;
    }
    tip
}

fn register_taskbar_created() {
    if TASKBAR_CREATED.load(Ordering::SeqCst) != 0 {
        return;
    }
    let name: Vec<u16> = "TaskbarCreated".encode_utf16().chain([0]).collect();
    let message = unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) };
    TASKBAR_CREATED.store(message, Ordering::SeqCst);
}
