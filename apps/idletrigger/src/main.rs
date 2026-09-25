//! IdleTrigger — native Windows power automation tray utility.
//!
//! Single-instance tray app with a control panel for Stay
//! Awake and Idle Monitoring (lock/sleep/hibernate/shutdown/restart actions
//! with a cancellable non-activating countdown), config persisted to
//! `IdleTrigger.toml` next to the EXE using stable configuration fields.

#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicIsize, AtomicU32, Ordering};
use std::time::Duration;

use idletrigger_core::config;
use idletrigger_core::i18n::I18n;

use runtime::*;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::Graphics::Gdi::{HDC, HFONT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, EXECUTION_STATE, GetSystemPowerStatus,
    SYSTEM_POWER_STATUS, SetSuspendState, SetThreadExecutionState,
};
use windows::Win32::System::Shutdown::{
    EWX_FORCEIFHUNG, EWX_POWEROFF, EWX_REBOOT, ExitWindowsEx, LockWorkStation,
};
use windows::Win32::System::StationsAndDesktops::{
    BSF_POSTMESSAGE, BSM_APPLICATIONS, BroadcastSystemMessageW,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Controls::{
    ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BS_OWNERDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
    DispatchMessageW, FindWindowW, GetDlgItem, GetMessageW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, HMENU, IDC_ARROW, IsWindowVisible, KillTimer, LoadCursorW, LoadIconW,
    MoveWindow, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SW_HIDE,
    SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SendMessageW, SetTimer, SetWindowPos,
    SetWindowTextW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE,
    WM_COMMAND, WM_DESTROY, WM_DRAWITEM, WM_TIMER, WNDCLASSW, WS_CHILD, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPED, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

const APP_VERSION: &str = match option_env!("IDLETRIGGER_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

// Control identifiers.
const IDC_NOSLEEP: usize = 110;
const IDC_IDLE: usize = 112;
// 211-213 deliberately land inside the subtitle static range below so the
// panel's summary rows draw through the subtitle painter; adding ordinary
// controls at 214-219 would silently render them as subtitles too.
const IDC_POWER_SUMMARY: usize = 211;
const IDC_AUTOMATION_SUMMARY: usize = 212;
const IDC_THEME_SCHEDULE: usize = 213;
const IDC_AUTOMATION: usize = 141;
const IDC_SYSTEM_BUTTON: usize = 146;
const IDC_MANAGE_BUTTON: usize = 147;
const IDC_SETTINGS_BUTTON: usize = 148;
const IDC_THEME_ENABLE: usize = 149;
const IDC_THEME_SWITCH: usize = 150;
const IDC_THEME_REPAIR: usize = 151;
const IDC_NOSLEEP_TIMED_30M: usize = 152;
const IDC_NOSLEEP_TIMED_1H: usize = 153;
const IDC_NOSLEEP_TIMED_2H: usize = 154;
const IDC_NOSLEEP_TIMED_CANCEL: usize = 155;
const IDC_EXIT_BUTTON: usize = 140;
const IDC_WARN_TEXT: usize = 130;
const IDC_WARN_CANCEL: usize = 131;

// Owner-drawn static ranges: 201-209 section headers, 211-219 subtitles.
const STATIC_SECTION_BASE: usize = 201;
const STATIC_SUBTITLE_BASE: usize = 211;

// Fonts retained for WM_DRAWITEM painting (Go keeps them on the panel).

/// Go panel.create: "IdleTrigger" or "IdleTrigger v{version}".
fn panel_title_text() -> String {
    if APP_VERSION.is_empty() || APP_VERSION == "dev" {
        "IdleTrigger".to_string()
    } else {
        format!("IdleTrigger v{APP_VERSION}")
    }
}

fn panel_font_body() -> HFONT {
    make_font(14, 400)
}

fn panel_font_section() -> HFONT {
    make_font(14, 700)
}

fn panel_font_subtitle() -> HFONT {
    make_font(12, 600)
}

// Layout tokens, identical to the Go control panel (96-DPI logical pixels):
// visual style may differ between toolkits, geometry must not.
const PANEL_CLIENT_WIDTH: i32 = 486;
// pad18 + (22+2 + 36+2 + 18 + 14) ×3 sections … exact Go flow ends at 344
// including the theme schedule subtitle.
const PANEL_CLIENT_HEIGHT: i32 = 344;
const PAD: i32 = 18;
const GAP: i32 = 8;
const SECTION_GAP: i32 = 14;
const LABEL_GAP: i32 = 2;
const SECTION_H: i32 = 22;
const SUBTITLE_H: i32 = 18;
const BUTTON_H: i32 = 36;

// Internal window messages (WM_APP range).
const WM_IDLE_WARN: u32 = 0x8002;
const WM_IDLE_CANCEL: u32 = 0x8003;
const WM_REFRESH_UI: u32 = 0x8004;
const WM_ACTION_SHOW: u32 = 0x8005;
const WM_LOCK_NOTIFY: u32 = 0x8006;
const WM_EXTERNAL_RELOAD: u32 = 0x8007;
const WM_IPC_REQUEST: u32 = 0x8008;
const WM_REFRESH_THEME: u32 = 0x8009;
static THEME_REFRESH: theme_refresh::Requests = theme_refresh::Requests::new();

pub(crate) fn request_theme_repair_refresh() {
    enqueue_theme_refresh(true);
}

pub(crate) fn request_theme_refresh() {
    enqueue_theme_refresh(false);
}

fn enqueue_theme_refresh(force: bool) {
    if hwnd(&HIDDEN).is_invalid() {
        return;
    }
    if THEME_REFRESH.request(force) {
        post_theme_refresh();
    }
}

fn post_theme_refresh() {
    if unsafe { PostMessageW(Some(hwnd(&HIDDEN)), WM_REFRESH_THEME, WPARAM(0), LPARAM(0)) }.is_err()
    {
        THEME_REFRESH.post_failed();
    }
}

mod accessibility;
mod automation;
mod automation_ui;
#[cfg(feature = "devtools")]
mod capture;
mod choice;
#[cfg(feature = "devtools")]
mod devtools;
mod display;
mod dpi;
mod gpu_activity;
mod idle_monitor;
mod ipc;
mod iplocate;
mod list_style;
mod nativeform;
mod paint;
mod pipe;
mod popups;
#[cfg(test)]
mod render_tests;
mod runtime;
mod settings_ui;
mod single_instance;
mod system;
mod theme;
mod theme_com;
mod theme_contrast;
mod theme_engine;
mod theme_recovery;
mod theme_refresh;
mod theme_repair;
mod tooltips;
mod viewport;

const PANEL_TIMER: usize = 1;
const WARN_TIMER: usize = 2;
const POWER_STATUS_TIMER: usize = 3;
const LOCK_POLL_TIMER: usize = 4;
const NOSLEEP_TIMED_TIMER: usize = 5;
const BATTERY_POLL_TICKS: u32 = 120; // 250ms * 120 = 30s

// ---- Shared runtime state ------------------------------------------------
// CONFIG state and logging live in runtime.rs, re-exported here so every
// sibling keeps resolving crate::CONFIG and friends unchanged.

static I18N: std::sync::RwLock<Option<I18n>> = std::sync::RwLock::new(None);

static REGISTERED_SHOW_PANEL: AtomicU32 = AtomicU32::new(0);
static DPI_SCALE: AtomicI32 = AtomicI32::new(96);

static HIDDEN: AtomicIsize = AtomicIsize::new(0);
static PANEL: AtomicIsize = AtomicIsize::new(0);
static WARNING: AtomicIsize = AtomicIsize::new(0);
pub static ACTION_WARN_HWND: AtomicIsize = AtomicIsize::new(0);
pub static LOCK_NOTIFY_HWND: AtomicIsize = AtomicIsize::new(0);
static TRAY_PTR: AtomicIsize = AtomicIsize::new(0);
// Last theme the tray icon was rendered for (Go s.trayThemeDark).
static TRAY_THEME_DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CHK_NOSLEEP: AtomicIsize = AtomicIsize::new(0);
static CHK_IDLE: AtomicIsize = AtomicIsize::new(0);
static CHK_AUTOMATION: AtomicIsize = AtomicIsize::new(0);
static LBL_POWER_SUMMARY: AtomicIsize = AtomicIsize::new(0);
static LBL_AUTOMATION_SUMMARY: AtomicIsize = AtomicIsize::new(0);
static LBL_THEME_SCHEDULE: AtomicIsize = AtomicIsize::new(0);
static WARN_TEXT: AtomicIsize = AtomicIsize::new(0);

static TRAY_ALIVE: AtomicBool = AtomicBool::new(false);
static MENU_OPEN_ID: Mutex<Option<tray_icon::menu::MenuId>> = Mutex::new(None);
static MENU_EXIT_ID: Mutex<Option<tray_icon::menu::MenuId>> = Mutex::new(None);

static IDLE_MS: AtomicI64 = AtomicI64::new(0);
static WARNING_ACTIVE: AtomicBool = AtomicBool::new(false);
static WARN_SECONDS_LEFT: AtomicI32 = AtomicI32::new(0);
// Recovering lock: a torn mid-update clock reads as stale, and the next
// 250ms sample self-heals — the worst case is a cancelled warning, never a
// spurious one (a deadline is always set after the flag that guards it).
static IDLE_CLOCK: std::sync::LazyLock<Mutex<idle_monitor::Clock>> =
    std::sync::LazyLock::new(|| Mutex::new(idle_monitor::Clock::default()));
static WARNING_PREVIEW_SESSION: AtomicBool = AtomicBool::new(false);
static WARN_ACTION: Mutex<String> = Mutex::new(String::new());
static ON_AC: AtomicBool = AtomicBool::new(true);
static BATTERY_PERCENT: AtomicI32 = AtomicI32::new(100);
static BATTERY_BLOCKED: AtomicBool = AtomicBool::new(false);
static NOSLEEP_EXECUTION_ON: AtomicBool = AtomicBool::new(false);
static EXITING: AtomicBool = AtomicBool::new(false);
static BTN_NOSLEEP_TIMED_CANCEL: AtomicIsize = AtomicIsize::new(0);

/// Timed stay-awake override: a runtime-only overlay source above the saved
/// switch. Expiry rides a one-shot timer on the hidden window; restarts drop
/// it because nothing is persisted (task overrides behave the same way).
struct TimedNosleep {
    until: std::time::Instant,
    keep_screen: bool,
}
static NOSLEEP_TIMED: Mutex<Option<TimedNosleep>> = Mutex::new(None);
pub(crate) const NOSLEEP_TIMED_MAX_SECS: u64 = 24 * 60 * 60;

/// Resolved display language is Chinese (Go ResolveLanguage short unit).
pub(crate) fn i18n_is_chinese() -> bool {
    match cfg_map(|c| c.language.clone()).as_str() {
        "en" => false,
        "zh-CN" => true,
        _ => unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() == 0x0804 },
    }
}

fn t(key: &str) -> String {
    I18N.read()
        .ok()
        .and_then(|guard| guard.as_ref().map(|i| i.t(key).to_string()))
        .unwrap_or_else(|| key.to_string())
}

/// Resolves and hot-swaps the active locale; all future `t()` calls use it.
fn apply_language(lang: &str) {
    let previous = t("settings_title");
    let resolved = match lang {
        "en" => "en".to_string(),
        "zh-CN" => "zh-CN".to_string(),
        _ => {
            let native = unsafe { GetUserDefaultUILanguage() };
            if native == 0x0804 {
                "zh-CN".into()
            } else {
                "en".into()
            }
        }
    };
    if let Ok(mut guard) = I18N.write() {
        *guard = Some(I18n::load(&resolved));
    }
    if previous != t("settings_title") && !hwnd(&PANEL).is_invalid() {
        refresh_language();
    }
}

fn refresh_language() {
    let panel = hwnd(&PANEL);
    let _dpi = dpi::Scope::window(panel);
    set_control_text(panel, &panel_title_text());
    for (id, key) in [
        (STATIC_SECTION_BASE, "menu_power_management"),
        (STATIC_SECTION_BASE + 1, "menu_automation_section"),
        (STATIC_SECTION_BASE + 2, "menu_theme_switch"),
        (IDC_NOSLEEP, "menu_nosleep_enable"),
        (IDC_IDLE, "menu_idle_enable"),
        (IDC_AUTOMATION, "automation_master"),
        (IDC_MANAGE_BUTTON, "menu_automation_manage"),
        (IDC_THEME_ENABLE, "menu_theme_enable"),
        (IDC_THEME_SWITCH, "menu_theme_switch_now"),
        (IDC_THEME_REPAIR, "menu_theme_repair"),
        (IDC_SYSTEM_BUTTON, "menu_system_controls"),
        (IDC_SETTINGS_BUTTON, "settings_open"),
        (IDC_EXIT_BUTTON, "menu_exit_panel"),
    ] {
        let control = unsafe { GetDlgItem(Some(panel), id as i32).unwrap_or_default() };
        set_control_text(control, &t(key));
        if matches!(id, IDC_NOSLEEP | IDC_IDLE | IDC_AUTOMATION) {
            unsafe {
                let mut rect = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(control, &mut rect);
                let font = HFONT(
                    SendMessageW(
                        control,
                        windows::Win32::UI::WindowsAndMessaging::WM_GETFONT,
                        None,
                        None,
                    )
                    .0 as *mut _,
                );
                let width = checkbox_hit_width(panel, font, &t(key))
                    .min(split_row(PANEL_CLIENT_WIDTH - 2 * PAD, 2));
                let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                    control,
                    None,
                    0,
                    0,
                    scale(width),
                    rect.bottom,
                    windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
                );
            }
        }
    }
    let tray = TRAY_PTR.load(Ordering::SeqCst);
    if tray != 0 {
        unsafe {
            (&*(tray as *const tray_icon::TrayIcon)).set_menu(Some(Box::new(tray_menu())));
        }
    }
    settings_ui::refresh_language();
    automation_ui::refresh_language();
    tooltips::refresh_all(panel);
    refresh_status();
}

fn hwnd(slot: &AtomicIsize) -> HWND {
    HWND(slot.load(Ordering::SeqCst) as *mut core::ffi::c_void)
}

fn scale(v: i32) -> i32 {
    dpi::scale(v)
}

fn main() {
    // GUI-process panics are otherwise invisible; keep them diagnosable by
    // writing next to the EXE (a CWD-relative path can point at system32).
    std::panic::set_hook(Box::new(|info| {
        let path = std::env::current_exe()
            .ok()
            .and_then(|p| {
                p.parent()
                    .map(|d| d.join("IdleTrigger-panic.txt").to_path_buf())
            })
            .unwrap_or_else(|| std::path::PathBuf::from("IdleTrigger-panic.txt"));
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            use std::io::Write;
            let _ = writeln!(f, "{info:?}");
            let _ = f.flush();
        }
    }));
    let mut start_minimized = false;
    let mut startup_delay = 0u64;
    let mut cli_args: Vec<String> = Vec::new();
    for arg in std::env::args().skip(1) {
        if arg == "--minimized" {
            start_minimized = true;
        } else if let Some(v) = arg.strip_prefix("--delay=") {
            startup_delay = v.parse().unwrap_or(0).clamp(0, 60);
        } else {
            cli_args.push(arg);
        }
    }
    if !cli_args.is_empty() {
        let language = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.parent()
                    .map(|dir| config::load(&dir.join("IdleTrigger.toml")).config.language)
            })
            .unwrap_or_else(|| "auto".into());
        apply_language(&language);
        #[cfg(feature = "devtools")]
        let _ = devtools::load();
        // CLI mode: attach to the parent console and exit.
        ipc::attach_console();
        std::process::exit(ipc::run_cli(&cli_args));
    }

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let config_path = exe_dir.join("IdleTrigger.toml");
    let loaded = config::load(&config_path);

    let effective_config = loaded.config.clone();
    #[cfg(feature = "devtools")]
    let _ = devtools::load();

    *lock(&CONFIG) = Some(effective_config.clone());
    *lock(&CONFIG_DOC) = Some(loaded.document);
    *lock(&CONFIG_SOURCE) = loaded.source_text;
    CONFIG_LOAD_FAILED.store(loaded.load_error.is_some(), Ordering::SeqCst);
    *lock(&CONFIG_PATH) = Some(config_path.clone());
    apply_language(&effective_config.language);

    let _instance_guard = match single_instance::acquire() {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            request_show_from_second_instance();
            log_line("another instance is running; requested panel show and exiting");
            return;
        }
        Err(err) => {
            warn_dialog("", &t_pub("error_single_instance").replace("%s", &err));
            return;
        }
    };

    // Runtime diagnostic reads never mutate the persisted configuration.
    if cfg_map(|c| c.logging_enabled) {
        init_log(&exe_dir);
    }
    log_line("IdleTrigger starting");
    #[cfg(feature = "devtools")]
    let diagnostics = devtools::ENABLED.load(Ordering::SeqCst);
    #[cfg(not(feature = "devtools"))]
    let diagnostics = false;
    if !diagnostics && let Err(error) = system::autostart_ensure_current() {
        log_line(&format!("autostart registration repair failed: {error}"));
    }
    if let Some(err) = &loaded.load_error {
        log_line(&format!("config load failed, defaults used: {err}"));
        let body = format!(
            "{}\n{}\n\n{}",
            t_pub("warning_config_defaults"),
            t_pub("warning_config_recovery"),
            err
        );
        warn_dialog("", &body);
    } else if let Some(fields) = &loaded.field_errors {
        // Field-level type mistakes keep the document parseable; one save
        // rewrites the offenders, so this must not lock committing.
        log_line(&format!("config fields reset to defaults: {fields}"));
        warn_dialog(
            "",
            &t_pub("warning_config_fields").replacen("%s", fields, 1),
        );
    }

    if startup_delay > 0 {
        std::thread::sleep(Duration::from_secs(startup_delay));
    }

    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES,
        };
        let _ = InitCommonControlsEx(&icc);
        DPI_SCALE.store(GetDpiForSystem() as i32, Ordering::SeqCst);
    }
    theme::refresh_from_registry();
    #[cfg(feature = "devtools")]
    devtools::apply_theme_override();
    // Register the second-instance broadcast message on THIS (primary)
    // instance so its hidden window actually handles it.
    unsafe {
        let name: Vec<u16> = "IdleTriggerShowPanel".encode_utf16().chain([0]).collect();
        let message = RegisterWindowMessageW(PCWSTR(name.as_ptr()));
        REGISTERED_SHOW_PANEL.store(message, Ordering::SeqCst);
    }

    create_windows();
    // Register power-source and battery-percentage notifications on the
    // hidden window so AC/battery changes arrive instantly (Go power_windows).
    unsafe {
        use windows::Win32::System::Power::RegisterPowerSettingNotification;
        use windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE;
        let recipient = windows::Win32::Foundation::HANDLE(hwnd(&HIDDEN).0);
        for guid in [
            windows::Win32::System::SystemServices::GUID_ACDC_POWER_SOURCE,
            windows::Win32::System::SystemServices::GUID_BATTERY_PERCENTAGE_REMAINING,
        ] {
            let _ = RegisterPowerSettingNotification(recipient, &guid, DEVICE_NOTIFY_WINDOW_HANDLE);
        }
    }
    // The tray handle is not `Send`; keep it alive on the main thread for the
    // whole run (dropped after the message loop ends).
    // Menu theming must precede native menu creation (muda HMENU).
    theme::set_process_menu_theme(theme::is_dark());
    let _tray_guard = tray_init();
    TRAY_ALIVE.store(_tray_guard.is_some(), Ordering::SeqCst);
    if _tray_guard.is_none() {
        log_line("tray creation failed; keeping the control panel visible");
    }

    if crate::cfg_map(|c| c.hotkeys_enabled) {
        let failed = system::register_all();
        if failed.is_empty() {
            log_line("global hotkeys registered");
        } else {
            log_line(&format!("hotkeys failed: {}", failed.join(", ")));
            let body: String = failed.iter().map(|f| format!("\u{2022} {f}\n")).collect();
            warn_dialog(&t_pub("msg_hotkey_conflict"), &body);
        }
    }

    automation::reload_rules();
    automation::spawn(exe_dir.clone());
    theme::apply_to_all();
    refresh_battery();
    theme_engine::spawn();
    spawn_config_watcher();
    #[cfg(feature = "devtools")]
    devtools::maybe_show_warning_preview();
    #[cfg(feature = "devtools")]
    devtools::start_capture_sequence();
    ipc::spawn_server();

    refresh_battery();
    apply_stay_awake();
    refresh_checkboxes();
    refresh_status();
    spawn_idle_thread();

    if !start_minimized || _tray_guard.is_none() {
        // Atomic first presentation: no light flash in dark mode.
        FirstFrameGate::begin(hwnd(&PANEL)).reveal();
    }

    let mut msg = msg_default();
    'pump: loop {
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if result.0 <= 0 {
            break;
        }
        unsafe {
            if !pre_translate_dialog(&msg) {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let mut exit = false;
        while let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
            if let tray_icon::TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                button_state: tray_icon::MouseButtonState::Up,
                ..
            } = event
            {
                toggle_panel();
            }
        }
        while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            let is_open = MENU_OPEN_ID
                .lock()
                .map(|g| g.as_ref() == Some(&event.id))
                .unwrap_or(false);
            let is_exit = MENU_EXIT_ID
                .lock()
                .map(|g| g.as_ref() == Some(&event.id))
                .unwrap_or(false);
            if is_open {
                toggle_panel();
            } else if is_exit {
                exit = true;
            }
        }
        if exit {
            unsafe { PostQuitMessage(0) };
            break 'pump;
        }
    }

    EXITING.store(true, Ordering::SeqCst);
    system::unregister_all();
    TRAY_PTR.store(0, Ordering::SeqCst);
    drop(_tray_guard);
    TRAY_ALIVE.store(false, Ordering::SeqCst);
    unsafe {
        let _ = SetThreadExecutionState(ES_CONTINUOUS);
    }
    log_line("IdleTrigger exited");
}

fn msg_default() -> windows::Win32::UI::WindowsAndMessaging::MSG {
    Default::default()
}

// ---- Single instance -----------------------------------------------------

fn request_show_from_second_instance() {
    if ipc::send("open").is_some_and(|reply| !reply.starts_with("err:")) {
        return;
    }
    unsafe {
        let name: Vec<u16> = "IdleTriggerShowPanel".encode_utf16().chain([0]).collect();
        let message = RegisterWindowMessageW(PCWSTR(name.as_ptr()));
        if message == 0 {
            // Fall back to finding the panel window class directly.
            if let Ok(panel) = FindWindowW(w!("IdleTriggerPanel"), None) {
                let _ = ShowWindow(panel, SW_SHOW);
            }
            return;
        }
        REGISTERED_SHOW_PANEL.store(message, Ordering::SeqCst);
        let mut info = BSM_APPLICATIONS;
        let _ = BroadcastSystemMessageW(
            BSF_POSTMESSAGE,
            Some(&mut info as *mut _),
            message,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

// ---- Windows creation ----------------------------------------------------

unsafe extern "system" fn hidden_proc(
    hwnd_: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    guarded_proc("hidden", hwnd_, msg, wparam, lparam, move || unsafe {
        let registered = REGISTERED_SHOW_PANEL.load(Ordering::SeqCst);
        if registered != 0 && msg == registered {
            show_panel();
            return LRESULT(0);
        }
        match msg {
            WM_REFRESH_THEME => {
                let force = THEME_REFRESH.begin();
                if theme::refresh_from_registry() {
                    log_line("system theme changed");
                }
                if force {
                    theme::apply_to_all_after_repair();
                } else {
                    theme::apply_to_all();
                }
                refresh_status();
                if THEME_REFRESH.finish() {
                    post_theme_refresh();
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == POWER_STATUS_TIMER => {
                // Coalesced power-status re-read (drivers broadcast before
                // the status settles).
                let _ = KillTimer(Some(hwnd_), POWER_STATUS_TIMER);
                refresh_battery();
                apply_stay_awake();
                refresh_status();
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == NOSLEEP_TIMED_TIMER => {
                // Timed stay-awake expiry: drop the overlay and re-merge.
                let _ = KillTimer(Some(hwnd_), NOSLEEP_TIMED_TIMER);
                if crate::runtime::lock(&NOSLEEP_TIMED).take().is_some() {
                    apply_stay_awake();
                    refresh_status();
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == LOCK_POLL_TIMER => {
                // Lock-key sampling must stay on this message-pumping thread.
                popups::poll();
                LRESULT(0)
            }
            WM_IDLE_WARN => {
                show_warning();
                LRESULT(0)
            }
            WM_IDLE_CANCEL => {
                let cancelled = crate::runtime::lock(&IDLE_CLOCK)
                    .countdown(std::time::Instant::now())
                    .is_none();
                if cancelled && !WARNING_PREVIEW_SESSION.load(Ordering::SeqCst) {
                    hide_warning("session changed");
                }
                LRESULT(0)
            }
            WM_IPC_REQUEST => {
                ipc::process_requests();
                LRESULT(0)
            }
            WM_REFRESH_UI => {
                settings_ui::refresh_location_status();
                theme_engine::finish_manual_switch();
                theme_engine::finish_repair();
                automation::show_save_errors();
                apply_stay_awake();
                refresh_checkboxes();
                refresh_status();
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_POWERBROADCAST if wparam.0 == 0x8013 => {
                // PBT_POWERSETTINGCHANGE from our registered GUIDs:
                // AC/DC source flipped or battery percentage changed. The
                // payload's GUID tells which; both refresh the same state.
                let _ = SetTimer(Some(hwnd_), POWER_STATUS_TIMER, 1000, None);
                LRESULT(1)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_POWERBROADCAST => {
                // PBT_APMRESUMEAUTOMATIC = 18 (always sent on wake);
                // PBT_APMRESUMESUSPEND = 7 (follows 18 when the user input
                // triggered the wake). Re-assert Stay Awake —
                // SetThreadExecutionState flags do not survive sleep — and
                // refresh battery + theme state.
                if wparam.0 == 18 || wparam.0 == 7 {
                    log_line("power resume detected");
                    refresh_battery();
                    apply_stay_awake();
                    theme_recovery::environment_changed(false, true);
                    crate::theme::refresh_from_registry();
                    crate::theme::apply_to_all();
                }
                LRESULT(1)
            }
            WM_ACTION_SHOW => {
                popups::show_pending();
                LRESULT(0)
            }
            WM_LOCK_NOTIFY => {
                popups::show(wparam.0 as i32, lparam.0 != 0);
                LRESULT(0)
            }
            WM_EXTERNAL_RELOAD => {
                let _ = hot_reload_config();
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_DISPLAYCHANGE => {
                theme_recovery::environment_changed(true, false);
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_TIMECHANGE => {
                theme_engine::wake();
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_SETTINGCHANGE => {
                dpi::refresh_text_scale();
                if theme_contrast::refresh() || theme::is_color_set_change(lparam.0) {
                    request_theme_refresh();
                }
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_SYSCOLORCHANGE
            | windows::Win32::UI::WindowsAndMessaging::WM_THEMECHANGED => {
                request_theme_refresh();
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_HOTKEY => {
                let id = wparam.0 as u32;
                match id {
                    10 => {
                        log_line("hotkey: sleep");
                        execute_system_action("sleep");
                    }
                    11 => {
                        log_line("hotkey: lock");
                        execute_system_action("lock");
                    }
                    12 => {
                        log_line("hotkey: toggle stay awake");
                        on_toggle(IDC_NOSLEEP);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd_, msg, wparam, lparam),
        }
    })
}

/// Reads a control's window text (owner-drawn controls carry their label in
/// the native text slot, like Go p.labels).
fn window_text(control: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(control);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(control, &mut buf);
        String::from_utf16_lossy(&buf[..copied.max(0) as usize])
    }
}

/// Wide string with a NUL terminator for PCWSTR call sites.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Runs a window-proc body with a panic guard. A panic crossing the
/// "system" ABI aborts the process before anything else could react, so
/// each proc catches its own body here, logs, and degrades to
/// DefWindowProcW instead of taking the tray down silently.
fn guarded_proc(
    name: &str,
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    body: impl FnOnce() -> LRESULT,
) -> LRESULT {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(result) => result,
        Err(payload) => {
            log_line(&format!(
                "panic in {name} proc (msg 0x{msg:04X}): {}",
                runtime::panic_message(payload)
            ));
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

fn invalidate_control(id: usize) {
    unsafe {
        let control = GetDlgItem(Some(hwnd(&PANEL)), id as i32).unwrap_or_default();
        if !control.is_invalid() {
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, false);
        }
    }
}

/// Semantic on/off state for the owner-drawn toggles (Go p.toggles).
fn toggle_value(id: usize) -> bool {
    match id {
        IDC_NOSLEEP => cfg_map(|c| c.nosleep_enabled),
        IDC_IDLE => cfg_map(|c| c.idle_enabled),
        IDC_AUTOMATION => cfg_map(|c| c.automation_enabled),
        IDC_THEME_ENABLE => cfg_map(|c| c.theme_switch_enabled),
        _ => false,
    }
}

/// WM_DRAWITEM dispatcher for the panel (Go drawItemBuffered).
fn draw_panel_item(item: &nativeform::DrawItem) {
    // Buffered blit first (Go drawItemBuffered); fall back to direct paint.
    unsafe {
        if crate::nativeform::draw_buffered(item.dc, &item.bounds, |dc, bounds| {
            draw_panel_item_impl(item, dc, bounds);
        }) {
            return;
        }
        draw_panel_item_impl(item, item.dc, &item.bounds);
    }
}

fn draw_panel_item_impl(item: &nativeform::DrawItem, dc: HDC, bounds: &RECT) {
    let p = theme::palette();
    let scale = scale(96);
    let id = item.control_id as usize;
    let label = window_text(item.control);
    unsafe {
        if (STATIC_SECTION_BASE..STATIC_SECTION_BASE + 9).contains(&id) {
            draw_section_static(dc, bounds, &label, p, scale);
        } else if (STATIC_SUBTITLE_BASE..STATIC_SUBTITLE_BASE + 9).contains(&id) {
            draw_subtitle_static(dc, bounds, &label, p);
        } else if matches!(
            id,
            IDC_NOSLEEP | IDC_IDLE | IDC_AUTOMATION | IDC_THEME_ENABLE
        ) {
            let mut state = nativeform::control_state(item.control, item.state);
            state.active = toggle_value(id);
            accessibility::check(item.control, state.active);
            paint::draw_checkbox(
                dc,
                bounds,
                panel_font_body(),
                &label,
                p,
                p.window_bg,
                state,
                scale,
                16,
            );
        } else if id == IDC_EXIT_BUTTON {
            draw_exit_button(item, dc, bounds, &label, p, scale);
        } else {
            let state = nativeform::control_state(item.control, item.state);
            paint::draw_button(
                dc,
                bounds,
                panel_font_body(),
                &label,
                p,
                p.window_bg,
                state,
                paint::control_radius(),
            );
        }
    }
}

/// Section header: accent bar + bold title + divider (Go drawStatic).
unsafe fn draw_section_static(dc: HDC, bounds: &RECT, label: &str, p: &theme::Palette, scale: i32) {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Gdi as gdi;
    paint::fill_rect(dc, bounds, p.window_bg);
    let accent_height = paint::sp(16, scale);
    let accent = RECT {
        left: bounds.left,
        top: bounds.top + (bounds.bottom - bounds.top - accent_height) / 2,
        right: bounds.left + paint::sp(3, scale),
        bottom: bounds.top + (bounds.bottom - bounds.top + accent_height) / 2,
    };
    paint::fill_rect(dc, &accent, p.accent);

    let mut text_bounds = RECT {
        left: bounds.left + paint::sp(10, scale),
        ..*bounds
    };
    let mut text: Vec<u16> = label.encode_utf16().collect();
    let mut measured = text_bounds;
    unsafe {
        gdi::SetTextColor(dc, COLORREF(p.text));
        gdi::SetBkMode(dc, gdi::TRANSPARENT);
        let old = gdi::SelectObject(dc, gdi::HGDIOBJ(panel_font_section().0));
        let _ = gdi::DrawTextW(
            dc,
            &mut text,
            &mut measured,
            gdi::DT_LEFT | gdi::DT_VCENTER | gdi::DT_SINGLELINE | gdi::DT_CALCRECT,
        );
        let _ = gdi::DrawTextW(
            dc,
            &mut text,
            &mut text_bounds,
            gdi::DT_LEFT | gdi::DT_VCENTER | gdi::DT_SINGLELINE,
        );
        gdi::SelectObject(dc, old);
    }
    // Divider follows the measured title width.
    let divider = RECT {
        left: (measured.right + paint::sp(14, scale)).min(bounds.right),
        top: bounds.top + paint::sp(10, scale),
        right: bounds.right,
        bottom: bounds.top + paint::sp(10, scale) + 1,
    };
    if divider.left < divider.right {
        paint::fill_rect(dc, &divider, p.subtle_border);
    }
}

/// Muted status line under a row (Go drawStatic subtitle).
unsafe fn draw_subtitle_static(dc: HDC, bounds: &RECT, label: &str, p: &theme::Palette) {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Gdi as gdi;
    paint::fill_rect(dc, bounds, p.window_bg);
    if label.is_empty() {
        return;
    }
    unsafe {
        let mut text: Vec<u16> = label.encode_utf16().collect();
        let mut text_bounds = *bounds;
        gdi::SetTextColor(dc, COLORREF(p.muted));
        gdi::SetBkMode(dc, gdi::TRANSPARENT);
        let old = gdi::SelectObject(dc, gdi::HGDIOBJ(panel_font_subtitle().0));
        let _ = gdi::DrawTextW(
            dc,
            &mut text,
            &mut text_bounds,
            gdi::DT_LEFT | gdi::DT_VCENTER | gdi::DT_WORDBREAK,
        );
        gdi::SelectObject(dc, old);
    }
}

/// The exit button keeps a quiet danger look that only commits on hover or
/// press (Go drawing.go idExit branch).
unsafe fn draw_exit_button(
    item: &nativeform::DrawItem,
    dc: HDC,
    bounds: &RECT,
    label: &str,
    p: &theme::Palette,
    scale: i32,
) {
    let state = nativeform::control_state(item.control, item.state);
    let (mut fill, mut border, mut text) = (p.surface, p.border, p.danger_surface_text);
    if state.hovered {
        fill = p.danger_hover;
        border = p.danger_hover_border;
        text = p.danger_text;
    }
    if state.pressed {
        fill = p.danger_pressed;
        border = p.danger_pressed_border;
        text = p.danger_text;
    }
    if state.disabled {
        fill = p.disabled_surface;
        border = p.subtle_border;
        text = p.disabled_text;
    }
    paint::draw_surface(
        dc,
        bounds,
        p.window_bg,
        fill,
        border,
        paint::control_radius(),
    );
    paint::draw_button_label(dc, bounds, panel_font_body(), label, text, false, 8, 8);
    if state.focused && !state.disabled {
        let inset = paint::sp(2, scale);
        paint::frame_rect(
            dc,
            &RECT {
                left: bounds.left + inset,
                top: bounds.top + inset,
                right: bounds.right - inset,
                bottom: bounds.bottom - inset,
            },
            if state.hovered || state.pressed {
                p.danger_focus
            } else {
                p.focus
            },
        );
    }
}

unsafe extern "system" fn panel_proc(
    hwnd_: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    guarded_proc("panel", hwnd_, msg, wparam, lparam, move || unsafe {
        match msg {
            WM_CLOSE => {
                let _ = ShowWindow(hwnd_, SW_HIDE);
                LRESULT(0)
            }
            WM_COMMAND => {
                let code = wparam.0 & 0xFFFF;
                if matches!(code, IDC_NOSLEEP | IDC_IDLE | IDC_AUTOMATION) {
                    on_toggle(code);
                    // Owner-drawn toggles need a repaint after the state flip.
                    invalidate_control(code);
                } else if matches!(
                    code,
                    IDC_NOSLEEP_TIMED_30M | IDC_NOSLEEP_TIMED_1H | IDC_NOSLEEP_TIMED_2H
                ) {
                    let seconds = match code {
                        IDC_NOSLEEP_TIMED_30M => 30 * 60,
                        IDC_NOSLEEP_TIMED_1H => 60 * 60,
                        _ => 2 * 60 * 60,
                    };
                    set_timed_nosleep(seconds, false);
                } else if code == IDC_NOSLEEP_TIMED_CANCEL {
                    clear_timed_nosleep();
                } else if code == IDC_MANAGE_BUTTON {
                    automation_ui::show();
                } else if code == IDC_SETTINGS_BUTTON {
                    settings_ui::show();
                } else if code == IDC_THEME_ENABLE {
                    theme_engine::toggle_enabled();
                    refresh_checkboxes();
                    invalidate_control(IDC_THEME_ENABLE);
                } else if code == IDC_THEME_SWITCH {
                    theme_engine::manual_switch();
                } else if code == IDC_THEME_REPAIR {
                    theme_engine::repair();
                } else if code == IDC_EXIT_BUTTON {
                    log_line("exit via panel button");
                    PostQuitMessage(0);
                } else if code == IDC_SYSTEM_BUTTON {
                    if (wparam.0 >> 16) == 1 {
                        // CBN_SELCHANGE
                        let button =
                            GetDlgItem(Some(hwnd_), IDC_SYSTEM_BUTTON as i32).unwrap_or_default();
                        if let Ok(id) = choice::value(button).parse::<usize>()
                            && (900..=904).contains(&id)
                        {
                            execute_system_action(
                                ["lock", "sleep", "hibernate", "shutdown", "restart"][id - 900],
                            );
                        }
                    } else {
                        show_system_controls_menu(hwnd_);
                    }
                } else if (900..=904).contains(&code) {
                    // Quick-action rows committed from the choice popup.
                    let action = ["lock", "sleep", "hibernate", "shutdown", "restart"][code - 900];
                    log_line(&format!("quick action: {action}"));
                    execute_system_action(action);
                }
                LRESULT(0)
            }
            WM_DRAWITEM => {
                if let Some(item) = nativeform::draw_item(lparam) {
                    draw_panel_item(&item);
                    LRESULT(1)
                } else {
                    DefWindowProcW(hwnd_, msg, wparam, lparam)
                }
            }
            WM_TIMER if wparam.0 == PANEL_TIMER => {
                refresh_status();
                LRESULT(0)
            }
            #[cfg(feature = "devtools")]
            WM_TIMER if wparam.0 == devtools::CAPTURE_TIMER => {
                devtools::handle_capture_timer();
                LRESULT(0)
            }
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
                paint_static(wparam, lparam);
                LRESULT(theme::bg_brush().0 as isize)
            }

            WM_DESTROY => {
                // UI reconstruction is not application shutdown.
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd_, msg, wparam, lparam),
        }
    })
}

/// Uniform static backgrounds: primary text for section headers and the
/// warning body, subtle color for summary lines.
unsafe fn paint_static(wparam: WPARAM, lparam: LPARAM) {
    use windows::Win32::Graphics::Gdi::{SetBkColor, SetTextColor};
    unsafe {
        let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
        let control = HWND(lparam.0 as *mut _);
        let summary = control.0 as isize == LBL_POWER_SUMMARY.load(Ordering::SeqCst)
            || control.0 as isize == LBL_AUTOMATION_SUMMARY.load(Ordering::SeqCst);
        let color = if summary {
            theme::subtle_color()
        } else {
            theme::text_color()
        };
        SetTextColor(hdc, windows::Win32::Foundation::COLORREF(color));
        SetBkColor(hdc, windows::Win32::Foundation::COLORREF(theme::bg_color()));
    }
}

unsafe extern "system" fn warning_proc(
    hwnd_: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    guarded_proc("warning", hwnd_, msg, wparam, lparam, move || unsafe {
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
                paint_static(wparam, lparam);
                LRESULT(theme::bg_brush().0 as isize)
            }
            _ => DefWindowProcW(hwnd_, msg, wparam, lparam),
        }
    })
}

fn pre_translate_dialog(msg: &windows::Win32::UI::WindowsAndMessaging::MSG) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled};
    use windows::Win32::UI::WindowsAndMessaging::*;
    if msg.message != WM_KEYDOWN || !choice::open_popup().is_invalid() {
        return false;
    }
    unsafe {
        let root = GetAncestor(msg.hwnd, GA_ROOT);
        if root.is_invalid() {
            return false;
        }
        let default = automation_ui::default_button(root)
            .or_else(|| (root == settings_ui::theme_hwnd()).then(settings_ui::default_button))
            .or_else(|| {
                (root == hwnd(&WARNING))
                    .then(|| GetDlgItem(Some(root), IDC_WARN_CANCEL as i32).ok())
                    .flatten()
            })
            .or_else(|| popups::default_button(root));
        if default.is_none() && root != hwnd(&PANEL) {
            return false;
        }
        match msg.wParam.0 {
            0x09 => {
                nativeform::keyboard_navigation();
                let handled = IsDialogMessageW(root, msg).as_bool();
                viewport::reveal_control(root, GetFocus());
                handled
            }
            0x1b => {
                SendMessageW(root, WM_CLOSE, None, None);
                true
            }
            0x0d => {
                let focus = GetFocus();
                let mut class = [0u16; 32];
                let len = GetClassNameW(focus, &mut class);
                let button = if String::from_utf16_lossy(&class[..len.max(0) as usize])
                    .eq_ignore_ascii_case("BUTTON")
                {
                    Some(focus)
                } else {
                    default
                };
                if let Some(button) = button
                    && IsWindowEnabled(button).as_bool()
                {
                    SendMessageW(button, BM_CLICK, None, None);
                }
                true
            }
            _ => automation_ui::weekday_key(root, msg.wParam.0),
        }
    }
}

static CLASSES_REGISTERED: AtomicBool = AtomicBool::new(false);

/// Creates the application windows; DPI changes preserve these handles.
fn create_windows() {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module handle");
        if !CLASSES_REGISTERED.swap(true, Ordering::SeqCst) {
            register_class(instance, w!("IdleTriggerHiddenWindow"), hidden_proc);
            register_class(instance, w!("IdleTriggerPanel"), panel_proc);
            register_class(instance, w!("IdleTriggerIdleWarning"), warning_proc);
        }

        if hwnd(&HIDDEN).is_invalid() {
            let hidden = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("IdleTriggerHiddenWindow"),
                w!("IdleTriggerHidden"),
                WINDOW_STYLE(WS_OVERLAPPED.0),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .expect("hidden window");
            HIDDEN.store(hidden.0 as isize, Ordering::SeqCst);
            // Lock-key polling lives here on the UI thread; Go used a 50ms
            // timer on the notification window itself.
            let _ = SetTimer(Some(hidden), LOCK_POLL_TIMER, 50, None);
        }

        // Floating-panel shell copied from Go: a topmost popup with a slim
        // caption owned by the tray's hidden window, created on the cursor's
        // monitor so per-monitor DPI is correct before the first layout.
        let panel_style = WINDOW_STYLE(
            (WS_POPUP.0 | windows::Win32::UI::WindowsAndMessaging::WS_CAPTION.0 | WS_SYSMENU.0)
                | windows::Win32::UI::WindowsAndMessaging::WS_CLIPCHILDREN.0,
        );
        let panel_ex_style = WINDOW_EX_STYLE(WS_EX_TOPMOST.0);
        let (create_x, create_y) = cursor_work_area()
            .map(|work| (work.right - 1, work.bottom - 1))
            .unwrap_or((CW_USEDEFAULT, CW_USEDEFAULT));
        let title: Vec<u16> = panel_title_text().encode_utf16().chain([0]).collect();
        let panel = CreateWindowExW(
            panel_ex_style,
            w!("IdleTriggerPanel"),
            PCWSTR(title.as_ptr()),
            panel_style,
            create_x,
            create_y,
            1,
            1,
            Some(hwnd(&HIDDEN)),
            None,
            Some(instance.into()),
            None,
        )
        .expect("panel window");
        PANEL.store(panel.0 as isize, Ordering::SeqCst);
        dpi::install(panel);
        // Per-monitor DPI beats the system-wide guess now that the window
        // exists on its destination monitor.
        let window_dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(panel);
        if window_dpi > 0 {
            DPI_SCALE.store(window_dpi as i32, Ordering::SeqCst);
        }
        let _dpi = dpi::Scope::window(panel);
        let font = make_font(14, 400);
        let section_font = make_font(14, 700);
        let subtitle_font = make_font(12, 600);
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: scale(PANEL_CLIENT_WIDTH),
            bottom: scale(PANEL_CLIENT_HEIGHT),
        };
        let _ = AdjustWindowRectEx(&mut frame, panel_style, false, panel_ex_style);
        let panel_w = frame.right - frame.left;
        let panel_h = frame.bottom - frame.top;
        // Two-step commit: position first, show later (Go behavior).
        let (x, y) = cursor_work_area()
            .map(|work| panel_origin(work, panel_w, panel_h))
            .unwrap_or((CW_USEDEFAULT, CW_USEDEFAULT));
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
            panel,
            Some(windows::Win32::Foundation::HWND::default()),
            x,
            y,
            panel_w,
            panel_h,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
        );
        let _ = SetTimer(Some(panel), PANEL_TIMER, 1000, None);
        theme::apply_to_window(panel);
        set_window_icons(panel, instance);

        // Vertical flow copied from the Go compact_controls builder:
        // sections, rows, and subtitles advance y by the shared tokens.
        let row_width = PANEL_CLIENT_WIDTH - 2 * PAD; // 450
        let two_w = split_row(row_width, 2); // 221
        let three_w = split_row(row_width, 3); // 144
        let mut y = PAD;

        section_header(
            panel,
            &t("menu_power_management"),
            y,
            STATIC_SECTION_BASE,
            instance,
            section_font,
        );
        y += SECTION_H + LABEL_GAP;
        // Toggle controls shrink to their drawn checkbox footprint (Go row()).
        let nosleep_w = checkbox_hit_width(panel, font, &t("menu_nosleep_enable"))
            .min(two_w)
            .max(1);
        let chk_nosleep = checkbox(&ControlSpec {
            parent: panel,
            label: t("menu_nosleep_enable"),
            x: scale(PAD),
            y: scale(y),
            width: scale(nosleep_w),
            id: IDC_NOSLEEP,
            instance,
            font,
        });
        CHK_NOSLEEP.store(chk_nosleep.0 as isize, Ordering::SeqCst);
        let idle_w = checkbox_hit_width(panel, font, &t("menu_idle_enable"))
            .min(two_w)
            .max(1);
        let chk_idle = checkbox(&ControlSpec {
            parent: panel,
            label: t("menu_idle_enable"),
            // Slot position stays on the split grid; only the control width
            // shrinks to the drawn checkbox footprint (Go row()).
            x: scale(PAD + two_w + GAP),
            y: scale(y),
            width: scale(idle_w),
            id: IDC_IDLE,
            instance,
            font,
        });
        CHK_IDLE.store(chk_idle.0 as isize, Ordering::SeqCst);
        y += BUTTON_H + LABEL_GAP;
        // Timed stay-awake preset chips: one click arms the runtime overlay
        // (set_timed_nosleep) without touching the saved switches.
        let timed_buttons = [
            (IDC_NOSLEEP_TIMED_30M, "menu_nosleep_timed_30m"),
            (IDC_NOSLEEP_TIMED_1H, "menu_nosleep_timed_1h"),
            (IDC_NOSLEEP_TIMED_2H, "menu_nosleep_timed_2h"),
            (IDC_NOSLEEP_TIMED_CANCEL, "menu_nosleep_timed_cancel"),
        ];
        let timed_w = (row_width - 3 * GAP) / 4;
        for (index, (id, key)) in timed_buttons.iter().enumerate() {
            let button = owner_button(
                &ControlSpec {
                    parent: panel,
                    label: t(key),
                    x: scale(PAD + index as i32 * (timed_w + GAP)),
                    y: scale(y),
                    width: scale(timed_w),
                    id: *id,
                    instance,
                    font,
                },
                scale(BUTTON_H),
            );
            if *id == IDC_NOSLEEP_TIMED_CANCEL {
                BTN_NOSLEEP_TIMED_CANCEL.store(button.0 as isize, Ordering::SeqCst);
            }
        }
        y += BUTTON_H + LABEL_GAP;
        let power_summary = owner_static(
            &ControlSpec {
                parent: panel,
                label: String::new(),
                x: scale(PAD),
                y: scale(y),
                width: scale(row_width),
                id: IDC_POWER_SUMMARY,
                instance,
                font: subtitle_font,
            },
            scale(SUBTITLE_H),
        );
        LBL_POWER_SUMMARY.store(power_summary.0 as isize, Ordering::SeqCst);
        y += SUBTITLE_H + SECTION_GAP;

        section_header(
            panel,
            &t("menu_automation_section"),
            y,
            STATIC_SECTION_BASE + 1,
            instance,
            section_font,
        );
        y += SECTION_H + LABEL_GAP;
        let automation_w = checkbox_hit_width(panel, font, &t("automation_master"))
            .min(two_w)
            .max(1);
        let chk_automation = checkbox(&ControlSpec {
            parent: panel,
            label: t("automation_master"),
            x: scale(PAD),
            y: scale(y),
            width: scale(automation_w),
            id: IDC_AUTOMATION,
            instance,
            font,
        });
        CHK_AUTOMATION.store(chk_automation.0 as isize, Ordering::SeqCst);
        let manage_btn = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            scale(PAD + two_w + GAP),
            scale(y),
            scale(two_w),
            scale(BUTTON_H),
            Some(panel),
            Some(HMENU(IDC_MANAGE_BUTTON as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("manage button");
        let _ = set_control_font(manage_btn, font);
        nativeform::track(manage_btn);
        let manage_label = t("menu_automation_manage");
        set_control_text(manage_btn, &manage_label);
        y += BUTTON_H + LABEL_GAP;
        let automation_summary = owner_static(
            &ControlSpec {
                parent: panel,
                label: String::new(),
                x: scale(PAD),
                y: scale(y),
                width: scale(row_width),
                id: IDC_AUTOMATION_SUMMARY,
                instance,
                font: subtitle_font,
            },
            scale(SUBTITLE_H),
        );
        LBL_AUTOMATION_SUMMARY.store(automation_summary.0 as isize, Ordering::SeqCst);
        y += SUBTITLE_H + SECTION_GAP;

        section_header(
            panel,
            &t("menu_theme_switch"),
            y,
            STATIC_SECTION_BASE + 2,
            instance,
            section_font,
        );
        y += SECTION_H + LABEL_GAP;
        // Go themeRow: the toggle reserves at least 154px so its full label
        // fits; the two direct actions split the remainder equally.
        let theme_toggle_w = checkbox_hit_width(panel, font, &t("menu_theme_enable"))
            .max(154)
            .min(row_width - 2 * GAP);
        let theme_action_w = (row_width - theme_toggle_w - 2 * GAP) / 2;
        let theme_toggle = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            scale(PAD),
            scale(y),
            scale(theme_toggle_w),
            scale(BUTTON_H),
            Some(panel),
            Some(HMENU(IDC_THEME_ENABLE as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("theme enable");
        let _ = set_control_font(theme_toggle, font);
        nativeform::track(theme_toggle);
        let theme_enable_label = t("menu_theme_enable");
        set_control_text(theme_toggle, &theme_enable_label);
        let theme_switch_btn = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            scale(PAD + theme_toggle_w + GAP),
            scale(y),
            scale(theme_action_w),
            scale(BUTTON_H),
            Some(panel),
            Some(HMENU(IDC_THEME_SWITCH as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("theme switch");
        let _ = set_control_font(theme_switch_btn, font);
        nativeform::track(theme_switch_btn);
        let theme_switch_label = t("menu_theme_switch_now");
        set_control_text(theme_switch_btn, &theme_switch_label);
        let theme_repair_btn = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            scale(PAD + theme_toggle_w + GAP + theme_action_w + GAP),
            scale(y),
            scale(theme_action_w),
            scale(BUTTON_H),
            Some(panel),
            Some(HMENU(IDC_THEME_REPAIR as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("theme repair");
        let _ = set_control_font(theme_repair_btn, font);
        nativeform::track(theme_repair_btn);
        let theme_repair_label = t("menu_theme_repair");
        set_control_text(theme_repair_btn, &theme_repair_label);
        y += BUTTON_H + LABEL_GAP;
        // Schedule subtitle under the theme row (Go p.themeSchedule).
        let theme_schedule = owner_static(
            &ControlSpec {
                parent: panel,
                label: String::new(),
                x: scale(PAD),
                y: scale(y),
                width: scale(row_width),
                id: IDC_THEME_SCHEDULE,
                instance,
                font: subtitle_font,
            },
            scale(SUBTITLE_H),
        );
        LBL_THEME_SCHEDULE.store(theme_schedule.0 as isize, Ordering::SeqCst);
        y += SUBTITLE_H + SECTION_GAP;

        let system_btn = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            scale(PAD),
            scale(y),
            scale(three_w),
            scale(BUTTON_H),
            Some(panel),
            Some(HMENU(IDC_SYSTEM_BUTTON as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("system button");
        let _ = set_control_font(system_btn, font);
        nativeform::track(system_btn);
        let system_label = t("menu_system_controls");
        set_control_text(system_btn, &system_label);

        let settings_btn = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            scale(PAD + three_w + GAP),
            scale(y),
            scale(three_w),
            scale(BUTTON_H),
            Some(panel),
            Some(HMENU(IDC_SETTINGS_BUTTON as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("settings button");
        let _ = set_control_font(settings_btn, font);
        nativeform::track(settings_btn);
        let settings_label = t("settings_open");
        set_control_text(settings_btn, &settings_label);
        let exit_btn = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            scale(PAD + 2 * (three_w + GAP)),
            scale(y),
            scale(three_w),
            scale(BUTTON_H),
            Some(panel),
            Some(HMENU(IDC_EXIT_BUTTON as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("exit button");
        let _ = set_control_font(exit_btn, font);
        nativeform::track(exit_btn);
        let exit_label = t("menu_exit_panel");
        set_control_text(exit_btn, &exit_label);

        // Tooltips for every panel control (Go tooltips.go parity).
        tooltips::create_for_panel(panel);
        viewport::fit(panel);
        // Control visual styles follow the active light/dark mode from the
        // start (Go ApplyControl at creation).
        theme::retheme_children(panel);
        // Paint the full frame once before the window is ever shown so dark
        // mode never flashes a blank frame (Go first-frame presenter). Each
        // child gets its own synchronous paint too — owner-draw children
        // otherwise blit the default (light) class surface on first show.
        unsafe extern "system" fn paint_child(hwnd: HWND, _: LPARAM) -> windows::core::BOOL {
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
            }
            windows::core::BOOL(1)
        }
        let _ = windows::Win32::UI::WindowsAndMessaging::EnumChildWindows(
            Some(panel),
            Some(paint_child),
            LPARAM(0),
        );
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(panel);

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
            Some(instance.into()),
            None,
        )
        .expect("warning window");
        WARNING.store(warning.0 as isize, Ordering::SeqCst);
        dpi::install(warning);

        let warn_text = static_text(&ControlSpec {
            parent: warning,
            label: String::new(),
            x: scale(20),
            y: scale(18),
            width: scale(380),
            id: IDC_WARN_TEXT,
            instance,
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

        let warn_cancel = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(
                t("common_cancel")
                    .encode_utf16()
                    .chain([0])
                    .collect::<Vec<_>>()
                    .as_ptr(),
            ),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            scale(160),
            scale(116),
            scale(120),
            scale(36),
            Some(warning),
            Some(HMENU(IDC_WARN_CANCEL as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("warning cancel button");
        let _ = set_control_font(warn_cancel, font);
        nativeform::track(warn_cancel);

        popups::create();
        popups::lock_create();
    }
}

fn register_class(
    instance: windows::Win32::Foundation::HMODULE,
    name: PCWSTR,
    proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
) {
    unsafe {
        // winresource embeds the app icon as the first icon group; title
        // bars stay blank without a class icon.
        #[allow(clippy::manual_dangling_ptr)]
        let icon =
            LoadIconW(Some(instance.into()), PCWSTR(1usize as *const u16)).unwrap_or_else(|_| {
                LoadIconW(
                    None,
                    windows::Win32::UI::WindowsAndMessaging::IDI_APPLICATION,
                )
                .unwrap_or_default()
            });
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance.into(),
            lpszClassName: name,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: icon,
            // Background is painted per-theme in WM_ERASEBKGND; the class
            // brush covers the brief pre-first-paint flash — it must be the
            // themed brush or dark mode shows one white frame at startup.
            hbrBackground: theme::bg_brush(),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        assert!(atom != 0, "RegisterClassW failed");
    }
}

/// Shared resource icons are cached by Windows for each theme and size.
pub fn set_window_icons(hwnd_: HWND, _instance: windows::Win32::Foundation::HMODULE) {
    use windows::Win32::UI::WindowsAndMessaging::{LR_DEFAULTCOLOR, LoadImageW};
    // Go WindowIcons parity: the title bar uses the theme-specific tray icon
    // mark (resource 3 dark strokes on light captions, 4 light on dark).
    let resource: isize = if theme::is_dark() { 4 } else { 3 };
    unsafe {
        for (kind, size) in [(0usize, 16), (1, 32)] {
            if let Ok(icon) = LoadImageW(
                Some(_instance.into()),
                PCWSTR(resource as *const u16),
                windows::Win32::UI::WindowsAndMessaging::IMAGE_ICON,
                scale(size),
                scale(size),
                LR_DEFAULTCOLOR | windows::Win32::UI::WindowsAndMessaging::LR_SHARED,
            ) {
                let _ = SendMessageW(
                    hwnd_,
                    0x0080, // WM_SETICON
                    Some(WPARAM(kind)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
    }
}

fn cursor_work_area() -> Option<RECT> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    };
    unsafe {
        let mut point = windows::Win32::Foundation::POINT::default();
        windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point).ok()?;
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        if monitor.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        Some(RECT {
            left: info.rcWork.left,
            top: info.rcWork.top,
            right: info.rcWork.right,
            bottom: info.rcWork.bottom,
        })
    }
}

/// Panel origin: bottom-right of the work area with a 16-logical-px margin,
/// clamped inside, matching the Go `panelOrigin`.
fn panel_origin(work: RECT, width: i32, height: i32) -> (i32, i32) {
    let margin = scale(16);
    let mut x = work.right - width - margin;
    let mut y = work.bottom - height - margin;
    if x < work.left {
        x = work.left;
    }
    if y < work.top {
        y = work.top;
    }
    (x, y)
}

/// Re-anchor a retained, hidden panel using the current monitor topology.
fn position_panel_for_show(panel: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOACTIVATE, SWP_NOSIZE};
    let Some(work) = cursor_work_area() else {
        return;
    };
    unsafe {
        // Move onto the destination monitor first so WM_DPICHANGED can resize
        // the panel and its controls before we measure its final outer bounds.
        if SetWindowPos(
            panel,
            None,
            work.left,
            work.top,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .is_err()
        {
            return;
        }
        viewport::fit_work_area(panel, work);
        let mut bounds = RECT::default();
        if GetWindowRect(panel, &mut bounds).is_err() {
            return;
        }
        let _dpi = dpi::Scope::window(panel);
        let (x, y) = panel_origin(work, bounds.right - bounds.left, bounds.bottom - bounds.top);
        let _ = SetWindowPos(
            panel,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Splits a row into `count` equal widths separated by GAP, like the Go
/// `splitRow` helper.
fn split_row(total: i32, count: i32) -> i32 {
    if count <= 0 {
        return 0;
    }
    (total - (count - 1) * GAP) / count
}

fn section_header(
    parent: HWND,
    label: &str,
    y: i32,
    section_id: usize,
    instance: windows::Win32::Foundation::HMODULE,
    font: HFONT,
) -> HWND {
    owner_static(
        &ControlSpec {
            parent,
            label: label.to_string(),
            x: scale(PAD),
            y: scale(y),
            width: scale(PANEL_CLIENT_WIDTH - 2 * PAD),
            id: section_id,
            instance,
            font,
        },
        scale(SECTION_H),
    )
}

fn set_control_text(control: HWND, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = SetWindowTextW(control, PCWSTR(wide.as_ptr()));
    }
}

/// Shared creation parameters for standard child controls.
struct ControlSpec {
    parent: HWND,
    label: String,
    x: i32,
    y: i32,
    width: i32,
    id: usize,
    instance: windows::Win32::Foundation::HMODULE,
    font: HFONT,
}

unsafe fn create_control(spec: &ControlSpec, class: PCWSTR, extra_style: u32, height: i32) -> HWND {
    let text: Vec<u16> = spec.label.encode_utf16().chain([0]).collect();
    unsafe {
        let hwnd_ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(text.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | extra_style),
            spec.x,
            spec.y,
            spec.width,
            height,
            Some(spec.parent),
            Some(HMENU(spec.id as *mut _)),
            Some(spec.instance.into()),
            None,
        )
        .expect("child control");
        let _ = set_control_font(hwnd_, spec.font);
        hwnd_
    }
}

/// Owner-drawn toggle/command button (Go: bsOwnerDraw BUTTON). Visuals come
/// from paint::draw_button / draw_checkbox in the parent's WM_DRAWITEM.
fn owner_button(spec: &ControlSpec, height: i32) -> HWND {
    unsafe {
        let hwnd_ = create_control(
            spec,
            w!("BUTTON"),
            BS_OWNERDRAW as u32 | WS_TABSTOP.0,
            height,
        );
        nativeform::track(hwnd_);
        hwnd_
    }
}

/// Owner-drawn static (Go: ssOwnerDraw STATIC) painted by the parent.
fn owner_static(spec: &ControlSpec, height: i32) -> HWND {
    // SS_OWNERDRAW = 13 (Go raw style value).
    unsafe { create_control(spec, w!("STATIC"), 13, height) }
}

fn checkbox(spec: &ControlSpec) -> HWND {
    owner_button(spec, scale(BUTTON_H))
}

/// Logical width that tightly contains the checkbox glyph + label + focus
/// inset (Go CheckboxHitWidth), so toggle hit areas match what is drawn.
fn checkbox_hit_width(parent: HWND, font: HFONT, label: &str) -> i32 {
    unsafe {
        let hdc = windows::Win32::Graphics::Gdi::GetDC(Some(parent));
        if hdc.is_invalid() {
            return 0;
        }
        let old = windows::Win32::Graphics::Gdi::SelectObject(
            hdc,
            windows::Win32::Graphics::Gdi::HGDIOBJ(font.0),
        );
        let mut text: Vec<u16> = label.encode_utf16().collect();
        let mut bounds = RECT::default();
        let height = windows::Win32::Graphics::Gdi::DrawTextW(
            hdc,
            &mut text,
            &mut bounds,
            windows::Win32::Graphics::Gdi::DT_LEFT
                | windows::Win32::Graphics::Gdi::DT_SINGLELINE
                | windows::Win32::Graphics::Gdi::DT_CALCRECT,
        );
        windows::Win32::Graphics::Gdi::SelectObject(hdc, old);
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(Some(parent), hdc);
        if height == 0 || bounds.right <= bounds.left {
            return 0;
        }
        let dpi = scale(96).max(96);
        let logical = (bounds.right - bounds.left) * 96 / dpi;
        // 2 before the glyph, 16 glyph, 8 before label, 2 focus inset.
        2 + 16 + 8 + logical + 2
    }
}

fn static_text(spec: &ControlSpec) -> HWND {
    unsafe { create_control(spec, w!("STATIC"), 0, scale(20)) }
}

fn set_control_font(control: HWND, font: HFONT) -> bool {
    unsafe {
        SendMessageW(
            control,
            0x0030,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        )
        .0 != 0
    }
}

/// Builds a scaled UI font from the message font family (Go makeFont).
fn make_font(size_px: i32, weight: i32) -> HFONT {
    dpi::font(size_px, weight, false)
}

// ---- Tray ----------------------------------------------------------------

fn tray_init() -> Option<Box<tray_icon::TrayIcon>> {
    use tray_icon::TrayIconBuilder;

    // Extract from our own EXE so it works on any machine, not just the
    // build host with its source tree; dark/light variant follows the theme.
    let icon = tray_icon_for_theme()?;

    let menu = tray_menu();

    let tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_tooltip("IdleTrigger")
        .with_menu_on_left_click(false) // Go: left toggles panel, right opens menu
        .build()
        .ok()?;
    // The main-thread guard owns the stable allocation. Clear the borrowed
    // pointer before dropping it after the message loop.
    let tray = Box::new(tray);
    TRAY_PTR.store(
        (&*tray as *const tray_icon::TrayIcon) as isize,
        Ordering::SeqCst,
    );
    Some(tray)
}

fn tray_menu() -> tray_icon::menu::Menu {
    use tray_icon::menu::{ContextMenu, Menu, MenuItem, PredefinedMenuItem};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMenuInfo, HMENU, MENUINFO, MIM_STYLE, MNS_NOCHECK, SetMenuInfo,
    };
    let menu = Menu::new();
    let open = MenuItem::new(t("menu_open_panel"), true, None);
    let exit = MenuItem::new(t("menu_exit"), true, None);
    let _ = menu.append(&open);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&exit);
    // These text-only actions need no checkmark gutter. Keep native text
    // measurement so each language and DPI gets its own compact menu width.
    unsafe {
        let handle = HMENU(menu.hpopupmenu() as *mut _);
        let mut info = MENUINFO {
            cbSize: std::mem::size_of::<MENUINFO>() as u32,
            fMask: MIM_STYLE,
            ..Default::default()
        };
        if GetMenuInfo(handle, &mut info).is_ok() {
            info.dwStyle |= MNS_NOCHECK;
            let _ = SetMenuInfo(handle, &info);
        }
    }
    *crate::runtime::lock(&MENU_OPEN_ID) = Some(open.id().clone());
    *crate::runtime::lock(&MENU_EXIT_ID) = Some(exit.id().clone());
    menu
}

/// Loads an embedded icon from the running module, independent of path length.
fn exe_embedded_icon_by(resource_id: Option<i32>) -> Option<tray_icon::Icon> {
    use windows::Win32::UI::WindowsAndMessaging::{HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW};
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let icon = HICON(
            LoadImageW(
                Some(module.into()),
                PCWSTR(resource_id.unwrap_or(1) as usize as *const u16),
                IMAGE_ICON,
                32,
                32,
                LR_DEFAULTCOLOR,
            )
            .ok()?
            .0,
        );
        // Convert HICON to RGBA for tray-icon's Icon type.
        let rgba = hicon_to_rgba(icon);
        let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(icon);
        tray_icon::Icon::from_rgba(rgba?, 32, 32).ok()
    }
}

// Go resourceid: tray-dark (3) shows on light mode, tray-light (4) on dark.
const TRAY_ICON_DARK_STROKES: i32 = 3;
const TRAY_ICON_LIGHT_STROKES: i32 = 4;

/// The tray icon matching the active theme (Go updateIcon mapping).
fn tray_icon_for_theme() -> Option<tray_icon::Icon> {
    let id = if theme::is_dark() {
        TRAY_ICON_LIGHT_STROKES
    } else {
        TRAY_ICON_DARK_STROKES
    };
    exe_embedded_icon_by(Some(id)).or_else(|| exe_embedded_icon_by(None))
}

/// Swaps the tray icon when the theme flipped (Go refreshTrayThemeIcon on
/// the tray tick). Only call on the UI thread.
fn tray_refresh_theme_icon() {
    let dark = theme::is_dark();
    if dark == TRAY_THEME_DARK.swap(dark, Ordering::SeqCst) {
        return;
    }
    let ptr = TRAY_PTR.load(Ordering::SeqCst);
    if ptr == 0 {
        return;
    }
    unsafe {
        let tray = &*(ptr as *const tray_icon::TrayIcon);
        if let Some(icon) = tray_icon_for_theme() {
            let _ = tray.set_icon(Some(icon));
        }
        // Rebuild the tray menu so the native HMENU picks up the current
        // immersive theme (muda caches the rendering mode at creation).
        theme::set_process_menu_theme(dark);
        tray.set_menu(Some(Box::new(tray_menu())));
    }
}

/// Converts an HICON to an RGBA byte vector.
unsafe fn hicon_to_rgba(icon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, HGDIOBJ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    unsafe {
        let mut info = ICONINFO::default();
        if GetIconInfo(icon, &mut info).is_err() {
            return None;
        }
        let hdc = windows::Win32::Graphics::Gdi::CreateCompatibleDC(None);
        let mut bitmap = windows::Win32::Graphics::Gdi::BITMAP::default();
        let _ = windows::Win32::Graphics::Gdi::GetObjectW(
            HGDIOBJ(info.hbmColor.0),
            std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAP>() as i32,
            Some(&mut bitmap as *mut _ as *mut _),
        );
        let (w, h) = (bitmap.bmWidth, bitmap.bmHeight);
        let mut pixels = vec![0u8; (w * h * 4) as usize];

        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let ok = GetDIBits(
            hdc,
            info.hbmColor,
            0,
            h as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut bi,
            DIB_RGB_COLORS,
        );
        let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
        let _ = DeleteDC(hdc);

        if ok == 0 {
            return None;
        }
        // BGRA → RGBA
        for chunk in pixels.as_chunks_mut::<4>().0 {
            chunk.swap(0, 2);
        }
        Some(pixels)
    }
}

/// Go buildTooltip parity: title + 保持唤醒/空闲监测/主题/自动任务 lines,
/// "%s：%s" per line, joined by newlines, capped at 120 UTF-16 units.
fn build_tray_tooltip(
    effective_nosleep: bool,
    idle_running: bool,
    power: &EffectivePowerState,
) -> String {
    let line = |key: &str, value: String| t_args("status_line", &[&t(key), &value]);

    let mut lines = vec![if APP_VERSION.is_empty() || APP_VERSION == "dev" {
        "IdleTrigger".to_string()
    } else {
        format!("IdleTrigger v{APP_VERSION}")
    }];

    // Stay awake: effective state, paused wording under battery block.
    let stay_awake = if effective_nosleep {
        t("status_short_on")
    } else if power.requested && !power.awake {
        t("status_paused")
    } else {
        t("status_short_off")
    };
    lines.push(line("tooltip_nosleep", stay_awake));

    // Timed override: a distinct line while it is the active overlay source.
    if let Some((remaining, _)) = timed_nosleep_state() {
        lines.push(line("tooltip_nosleep_timed", format_remaining(remaining)));
    }

    // Idle monitor: paused by stay-awake, or "Nm 动作" when running.
    if idle_running {
        let unit = if crate::i18n_is_chinese() { "分" } else { "m" };
        let action_label = t(&format!(
            "menu_action_{}",
            cfg_map(|c| c.idle_action.clone())
        ));
        lines.push(line(
            "tooltip_idle",
            format!("{}{unit} {action_label}", power.idle_minutes),
        ));
    } else if power.idle_requested && (effective_nosleep || power.idle_paused) {
        lines.push(line("tooltip_idle", t("status_paused")));
    } else {
        lines.push(line("tooltip_idle", t("status_short_off")));
    }

    lines.push(line("tooltip_theme", theme_tooltip_value_short()));

    let enabled_count = crate::runtime::lock(&automation::RULES)
        .iter()
        .filter(|r| r.enabled)
        .count();
    if enabled_count > 0 {
        lines.push(line(
            "tooltip_automation",
            t("status_automation_count").replace("%d", &enabled_count.to_string()),
        ));
    }
    lines.join("\n")
}

/// Go themeTooltipValueShort: 开 + short schedule, 关, or 不支持.
fn theme_tooltip_value_short() -> String {
    let (enabled, _mode) = cfg_map(|c| (c.theme_switch_enabled, c.theme_mode.clone()));
    if !enabled {
        return t("status_short_off");
    }
    let schedule = theme_schedule_text_short();
    if schedule.is_empty() {
        return t("status_short_on");
    }
    format!("{} {}", t("status_short_on"), schedule)
}

/// Short theme schedule for the tooltip (Go formatThemeSchedule short=true):
/// 浅7:00/深19:00, 日出6:33/日落18:34, with the fixed-fallback suffix.
fn theme_schedule_text_short() -> String {
    theme_schedule_summary(true)
}

/// Updates the tray tooltip with a status line capped like Go's 120 UTF-16
/// units. Only call on the UI thread. Skips the `Shell_NotifyIconW` round
/// trip when the truncated text is unchanged: this runs every second from
/// `refresh_status`, and tray-icon always forwards the modify.
fn tray_update_tooltip(line: &str) {
    static LAST: Mutex<Option<String>> = Mutex::new(None);
    let ptr = TRAY_PTR.load(Ordering::SeqCst);
    if ptr == 0 {
        return;
    }
    let mut wide: Vec<u16> = line.encode_utf16().collect();
    if wide.len() > 118 {
        wide.truncate(118);
        wide.extend_from_slice("…".encode_utf16().collect::<Vec<u16>>().as_slice());
    }
    let text = String::from_utf16_lossy(&wide);
    let mut last = runtime::lock(&LAST);
    if last.as_deref() == Some(text.as_str()) {
        return;
    }
    unsafe {
        let tray = &*(ptr as *const tray_icon::TrayIcon);
        // Cache only after the modify succeeds, so a failed round trip
        // retries on the next refresh.
        if tray.set_tooltip(Some(&text)).is_ok() {
            *last = Some(text);
        }
    }
}

// ---- Panel behavior ------------------------------------------------------

/// Go nativeform/first_frame.go parity: keeps a newly created top-level
/// window out of the DWM composition stream (cloaked, transitions disabled)
/// until its layout and first synchronous paint are complete, so a dark
/// window never flashes the default light surface on first show.
pub struct FirstFrameGate {
    window: HWND,
    cloaked: bool,
    transitions_disabled: bool,
}

const DWMWA_TRANSITIONS_FORCED_DISABLED: u32 = 3;
const DWMWA_CLOAK: u32 = 13;
const FRAME_RECOVERY_TIMER: usize = 0x49544652;
const FRAME_RECOVERY_PROP: PCWSTR = w!("IdleTrigger.FrameRecovery");

#[cfg(test)]
thread_local! {
    static FAIL_UNCLOAK: std::cell::Cell<(isize, u32)> = const { std::cell::Cell::new((0, 0)) };
}

fn set_dwm_boolean(window: HWND, attribute: u32, enabled: bool) -> bool {
    #[cfg(test)]
    if attribute == DWMWA_CLOAK && !enabled {
        let (target, remaining) = FAIL_UNCLOAK.get();
        if target == window.0 as isize && remaining > 0 {
            FAIL_UNCLOAK.set((target, remaining - 1));
            return false;
        }
    }
    let value: u32 = if enabled { 1 } else { 0 };
    unsafe {
        windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            window,
            windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(attribute as i32),
            &value as *const u32 as *const core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
        .is_ok()
    }
}

fn clear_frame_recovery(window: HWND) {
    unsafe {
        let _ = KillTimer(Some(window), FRAME_RECOVERY_TIMER);
        let _ = windows::Win32::UI::WindowsAndMessaging::RemovePropW(window, FRAME_RECOVERY_PROP);
    }
}

fn schedule_frame_recovery(window: HWND) {
    unsafe {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::UI::WindowsAndMessaging::SetPropW;
        let _ = SetPropW(window, FRAME_RECOVERY_PROP, Some(HANDLE(window.0)));
        if SetTimer(
            Some(window),
            FRAME_RECOVERY_TIMER,
            500,
            Some(frame_recovery_proc),
        ) == 0
        {
            log_line("frame recovery timer failed; retrying on next window show");
        }
    }
}

fn recover_pending_frame(window: HWND) {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{GetPropW, IsWindowVisible};
        if GetPropW(window, FRAME_RECOVERY_PROP).is_invalid() {
            return; // Ignore a timer message already queued before cancellation.
        }
        if set_dwm_boolean(window, DWMWA_CLOAK, false) {
            clear_frame_recovery(window);
            if IsWindowVisible(window).as_bool() {
                present_layout(window);
            }
        }
    }
}

unsafe extern "system" fn frame_recovery_proc(window: HWND, _: u32, _: usize, _: u32) {
    // USER32 removes this timer and its window properties at destruction.
    recover_pending_frame(window);
}

/// Synchronous full-subtree paint (Go PresentFrame): invalidate the window
/// and every child, then RedrawWindow with UPDATENOW.
fn present_frame(window: HWND) {
    use windows::Win32::Graphics::Gdi::{
        InvalidateRect, RDW_ALLCHILDREN, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW,
        REDRAW_WINDOW_FLAGS,
    };
    unsafe {
        let _ = InvalidateRect(Some(window), None, false);
        let _ = windows::Win32::Graphics::Gdi::RedrawWindow(
            Some(window),
            None,
            None,
            REDRAW_WINDOW_FLAGS(
                RDW_INVALIDATE.0 | RDW_ALLCHILDREN.0 | RDW_UPDATENOW.0 | RDW_FRAME.0,
            ),
        );
    }
}

/// Layout moves suppress intermediate erases, so clear the vacated area when
/// committing the final frame (child paints are included by present_frame).
fn present_layout(window: HWND) {
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(window), None, true);
    }
    present_frame(window);
}

impl FirstFrameGate {
    /// Prepares a still-hidden top-level window for atomic presentation.
    /// Unsupported DWM attributes safely keep the normal hidden path.
    pub fn begin(window: HWND) -> Self {
        let transitions_disabled = set_dwm_boolean(window, DWMWA_TRANSITIONS_FORCED_DISABLED, true);
        let cloaked = set_dwm_boolean(window, DWMWA_CLOAK, true);
        Self {
            window,
            cloaked,
            transitions_disabled,
        }
    }

    /// Marks the window visible while cloaked, commits one complete frame,
    /// then uncloaks — and presents once more, because DWM can retain the
    /// pre-cloak surface for one composition cycle.
    pub fn reveal(self) {
        self.reveal_with(SW_SHOW);
    }

    /// Present a warning without taking activation from the user's window.
    pub fn reveal_no_activate(self) {
        self.reveal_with(SW_SHOWNOACTIVATE);
    }

    fn reveal_with(mut self, command: windows::Win32::UI::WindowsAndMessaging::SHOW_WINDOW_CMD) {
        unsafe {
            let _ = ShowWindow(self.window, command);
            present_frame(self.window);
            if self.cloaked {
                if set_dwm_boolean(self.window, DWMWA_CLOAK, false) {
                    clear_frame_recovery(self.window);
                    present_frame(self.window);
                } else {
                    schedule_frame_recovery(self.window);
                }
            }
            if self.transitions_disabled {
                set_dwm_boolean(self.window, DWMWA_TRANSITIONS_FORCED_DISABLED, false);
                self.transitions_disabled = false;
            }
        }
    }
}

/// Go BeginFrameTransition: repaint a visible form while DWM keeps the
/// incomplete palette out of the presentation stream, without changing focus.
pub struct FrameTransition(FirstFrameGate);

impl FrameTransition {
    pub fn begin(window: HWND) -> Option<Self> {
        // A nested DPI/theme callback must not uncloak its caller's frame.
        let mut cloaked = 0u32;
        unsafe {
            let _ = windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                window,
                windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
                (&mut cloaked as *mut u32).cast(),
                size_of::<u32>() as u32,
            );
        }
        if cloaked & 1 != 0 {
            return None;
        }
        unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(window) }
            .as_bool()
            .then(|| Self(FirstFrameGate::begin(window)))
    }
}

impl Drop for FrameTransition {
    fn drop(&mut self) {
        let gate = &mut self.0;
        if !unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(gate.window)) }
            .as_bool()
        {
            return;
        }
        present_layout(gate.window);
        if gate.cloaked {
            for _ in 0..3 {
                if set_dwm_boolean(gate.window, DWMWA_CLOAK, false) {
                    gate.cloaked = false;
                    clear_frame_recovery(gate.window);
                    present_layout(gate.window);
                    break;
                }
            }
            if gate.cloaked {
                log_line("theme frame uncloak failed");
                schedule_frame_recovery(gate.window);
            }
        }
        if gate.transitions_disabled {
            set_dwm_boolean(gate.window, DWMWA_TRANSITIONS_FORCED_DISABLED, false);
        }
    }
}

fn show_panel() {
    unsafe {
        let target = active_modal_window().unwrap_or_else(|| hwnd(&PANEL));
        recover_pending_frame(target);
        if windows::Win32::UI::WindowsAndMessaging::IsIconic(target).as_bool() {
            let _ = ShowWindow(target, windows::Win32::UI::WindowsAndMessaging::SW_RESTORE);
        } else if !IsWindowVisible(target).as_bool() {
            // Reopening a retained panel needs the same complete first frame
            // as startup; ShowWindow alone can expose an unfinished surface.
            let frame = FirstFrameGate::begin(target);
            if target == hwnd(&PANEL) {
                position_panel_for_show(target);
            }
            frame.reveal();
        } else {
            let _ = ShowWindow(target, SW_SHOW);
        }
        let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(target);
    }
}

fn active_modal_window() -> Option<HWND> {
    automation_ui::theme_hwnds()
        .into_iter()
        .rev()
        .chain([settings_ui::theme_hwnd()])
        .find(|window| !window.is_invalid() && unsafe { IsWindowVisible(*window).as_bool() })
}

fn toggle_panel() {
    if active_modal_window().is_some() {
        show_panel();
        return;
    }
    unsafe {
        if IsWindowVisible(hwnd(&PANEL)).as_bool() {
            let _ = ShowWindow(hwnd(&PANEL), SW_HIDE);
        } else {
            show_panel();
        }
    }
}

fn on_toggle(code: usize) {
    if !matches!(code, IDC_NOSLEEP | IDC_IDLE | IDC_AUTOMATION) {
        return;
    }
    if let Err(err) = edit_config(|c| match code {
        IDC_NOSLEEP => {
            c.nosleep_enabled = !c.nosleep_enabled;
            if c.nosleep_enabled {
                c.idle_enabled = false;
            }
        }
        IDC_IDLE => {
            c.idle_enabled = !c.idle_enabled;
            if c.idle_enabled {
                c.nosleep_enabled = false;
            }
        }
        IDC_AUTOMATION => c.automation_enabled = !c.automation_enabled,
        _ => unreachable!(),
    }) {
        warn_dialog("", &err);
        return;
    }
    sync_timed_with_manual();
    apply_stay_awake();
    refresh_checkboxes();
    refresh_status();
}
/// Go dialog.Warn: simple warning box owned by the desktop (NULL parent),
/// titled with the app name.
fn warn_dialog(heading: &str, body: &str) {
    let text = match (heading.is_empty(), body.is_empty()) {
        (true, _) => body.to_string(),
        (_, true) => heading.to_string(),
        _ => format!("{heading}\r\n\r\n{body}"),
    };
    let text_wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let title_wide: Vec<u16> = t_pub("app_title").encode_utf16().chain([0]).collect();
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
            None,
            PCWSTR(text_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            windows::Win32::UI::WindowsAndMessaging::MB_OK
                | windows::Win32::UI::WindowsAndMessaging::MB_ICONWARNING,
        );
    }
}

/// Serialize application writers; publish config and rules only after saving.
/// Publish a valid external configuration and apply its UI/platform settings.
fn hot_reload_config() -> Result<(), String> {
    let writer = lock(&CONFIG_WRITER);
    let config_path = crate::runtime::lock(&CONFIG_PATH)
        .clone()
        .ok_or("configuration path unavailable")?;
    let loaded = config::load(&config_path);
    if loaded.created_from_template {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "configuration file is missing",
        )
        .to_string());
    }
    if let Some(err) = loaded.load_error.as_ref() {
        log_line(&format!(
            "config reload rejected; retaining last valid configuration: {err}"
        ));
        return Err(err.clone());
    }
    *lock(&CONFIG) = Some(loaded.config.clone());
    *lock(&CONFIG_DOC) = Some(loaded.document);
    *lock(&CONFIG_SOURCE) = loaded.source_text;
    CONFIG_LOAD_FAILED.store(false, Ordering::SeqCst);
    automation::reload_rules();
    drop(writer);
    apply_language(&loaded.config.language);
    theme_engine::wake();
    sync_logging();
    system::unregister_all();
    if loaded.config.hotkeys_enabled {
        let failed = system::register_all();
        if !failed.is_empty() {
            log_line(&format!("hotkeys failed: {}", failed.join(", ")));
        }
    }
    refresh_battery();
    apply_stay_awake();
    refresh_checkboxes();
    refresh_status();
    theme::apply_to_all();
    log_line("config reloaded from external change");
    Ok(())
}

/// Poll exact file contents so atomic replacements and edits with preserved
/// timestamps are detected; our own saved document is skipped automatically.
fn spawn_config_watcher() {
    let Some(path) = lock(&CONFIG_PATH).clone() else {
        return;
    };
    std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            let mut observed = lock(&CONFIG_SOURCE).clone();
            while !EXITING.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(3));
                runtime::catch_and_log("config-watch", || {
                    let Ok(text) = std::fs::read_to_string(&path) else {
                        return;
                    };
                    if observed.as_ref() == Some(&text) {
                        return;
                    }
                    if lock(&CONFIG_SOURCE).as_ref() != Some(&text) {
                        let posted = unsafe {
                            PostMessageW(
                                Some(hwnd(&HIDDEN)),
                                WM_EXTERNAL_RELOAD,
                                WPARAM(0),
                                LPARAM(0),
                            )
                        };
                        if posted.is_err() {
                            return;
                        }
                    }
                    observed = Some(text);
                });
            }
        })
        .expect("spawn config watcher");
}

fn set_checkbox(slot: &AtomicIsize, checked: bool) {
    accessibility::check(hwnd(slot), checked);
    // Owner-draw buttons repaint from config via WM_DRAWITEM.
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd(slot)), None, false);
    }
}

fn refresh_checkboxes() {
    set_checkbox(&CHK_NOSLEEP, cfg_map(|c| c.nosleep_enabled));
    set_checkbox(&CHK_IDLE, cfg_map(|c| c.idle_enabled));
    set_checkbox(&CHK_AUTOMATION, cfg_map(|c| c.automation_enabled));
    accessibility::check(
        unsafe { GetDlgItem(Some(hwnd(&PANEL)), IDC_THEME_ENABLE as i32).unwrap_or_default() },
        cfg_map(|c| c.theme_switch_enabled),
    );
    invalidate_control(IDC_THEME_ENABLE);
}

/// Single derivation of the effective power-management state, consumed by
/// the panel status text, the tray tooltip, the execution-state flags, and
/// the idle monitor settings. Low battery is a runtime pause layered over
/// the manual toggles (Go battery.go contract); automation overrides layer
/// on top without rewriting them.
pub(crate) struct EffectivePowerState {
    /// Manual toggle or automation override requested Stay Awake.
    pub requested: bool,
    /// Automation asked to pause Stay Awake.
    pub paused: bool,
    /// Battery allows Stay Awake right now.
    pub battery_allowed: bool,
    /// Stay Awake is actually in effect.
    pub awake: bool,
    /// Screen keep-awake accompanies Stay Awake.
    pub keep_screen: bool,
    /// Idle monitoring is requested (manual or override).
    pub idle_requested: bool,
    /// Automation paused the idle monitor.
    pub idle_paused: bool,
    /// Idle monitor minutes in effect (an override wins over the config).
    pub idle_minutes: i32,
}

impl EffectivePowerState {
    /// Idle monitor actually runs: requested, not paused, and Stay Awake —
    /// which outranks it — is off.
    pub fn idle_running(&self) -> bool {
        self.idle_requested && !self.idle_paused && !self.awake
    }
}

pub(crate) fn effective_power_state() -> EffectivePowerState {
    let overrides = automation::overrides();
    let config = cfg_map(Clone::clone);
    let timed = timed_nosleep_state();
    let requested = config.nosleep_enabled || overrides.stay_awake || timed.is_some();
    let battery_allowed = ON_AC.load(Ordering::SeqCst)
        || (config.nosleep_on_battery
            && BATTERY_PERCENT.load(Ordering::SeqCst) >= config.nosleep_battery_threshold);
    EffectivePowerState {
        requested,
        paused: overrides.pause_stay_awake,
        battery_allowed,
        awake: requested && !overrides.pause_stay_awake && battery_allowed,
        keep_screen: (config.nosleep_enabled && config.keep_screen_on)
            || (overrides.stay_awake && overrides.keep_screen_on)
            || timed.is_some_and(|(_, keep_screen)| keep_screen),
        idle_requested: config.idle_enabled || overrides.enable_idle,
        idle_paused: overrides.pause_idle,
        idle_minutes: if overrides.enable_idle {
            overrides.idle_minutes
        } else {
            config.idle_timeout_minutes
        },
    }
}

fn power_status() -> (String, String) {
    let power = effective_power_state();
    let idle_action = cfg_map(|c| c.idle_action.clone());
    // Reason attribution: which task and/or the timed overlay is keeping the
    // machine awake. Shown only while actually awake.
    let mut reasons: Vec<String> = Vec::new();
    let sources = automation::overrides().stay_awake_sources;
    if !sources.is_empty() {
        let separator = if i18n_is_chinese() { "、" } else { ", " };
        reasons.push(t_args("status_reason_task", &[&sources.join(separator)]));
    }
    if let Some((remaining, _)) = timed_nosleep_state() {
        reasons.push(t_args(
            "status_reason_timed",
            &[&format_remaining(remaining)],
        ));
    }
    let reason_suffix = if power.awake && !reasons.is_empty() {
        t_args("status_reason_suffix", &[&reasons.join("+")])
    } else {
        String::new()
    };
    let mut awake_status = t(if !power.requested {
        "status_disabled"
    } else if power.paused {
        "status_paused_by_automation"
    } else if !power.battery_allowed {
        "status_paused_by_battery"
    } else if power.keep_screen {
        "status_enabled_keep_screen"
    } else {
        "status_enabled"
    });
    awake_status.push_str(&reason_suffix);
    let idle_status = if !power.idle_requested {
        t("status_disabled")
    } else if power.awake {
        t("status_paused_by_nosleep")
    } else if power.idle_paused {
        t("status_paused_by_automation")
    } else {
        t_args(
            "status_monitor_active",
            &[
                &power.idle_minutes.to_string(),
                &t(&format!("menu_action_{idle_action}")),
            ],
        )
    };
    (awake_status, idle_status)
}

fn refresh_status() {
    // The cancel chip is only meaningful while a timed overlay is armed.
    unsafe {
        let _ = EnableWindow(
            hwnd(&BTN_NOSLEEP_TIMED_CANCEL),
            timed_nosleep_state().is_some(),
        );
    }
    let (nosleep_status, idle_status) = power_status();
    let overview = format!(
        "{}{}{}{}",
        t("power_overview_prefix"),
        nosleep_status,
        t("power_overview_separator"),
        idle_status
    );
    set_text(&LBL_POWER_SUMMARY, &overview);

    let automation_on = cfg_map(|c| c.automation_enabled);
    // One lock for both counts: two separate samples could pair a total
    // from before a rule save with an enabled count from after it.
    let (rule_count, enabled_count) = {
        let rules = crate::runtime::lock(&automation::RULES);
        (rules.len(), rules.iter().filter(|r| r.enabled).count())
    };
    // Go automationOverviewText four states.
    let automation = if rule_count == 0 {
        t("automation_overview_empty")
    } else if !automation_on {
        t("automation_overview_paused").replace("%d", &rule_count.to_string())
    } else {
        match automation::next_scheduled() {
            Some(next) => t("automation_overview_next")
                .replace("%d", &enabled_count.to_string())
                .replace("%s", &next),
            None => t("automation_overview_enabled").replace("%d", &enabled_count.to_string()),
        }
    };

    set_text(&LBL_AUTOMATION_SUMMARY, &automation);
    set_text(&LBL_THEME_SCHEDULE, &theme_schedule_text());

    // Tray tooltip mirrors the effective power-management state. The tray
    // line reads the latched execution flag (apply_stay_awake owns it), the
    // paused wording reads the recomputed derivation.
    let power = effective_power_state();
    let effective_nosleep = NOSLEEP_EXECUTION_ON.load(Ordering::SeqCst);
    let idle_running = power.idle_requested && !power.idle_paused && !effective_nosleep;
    tray_update_tooltip(&build_tray_tooltip(effective_nosleep, idle_running, &power));
    tray_refresh_theme_icon();
    tooltips::refresh_all(hwnd(&PANEL));
}

fn set_text(slot: &AtomicIsize, text: &str) {
    if window_text(hwnd(slot)) == text {
        return;
    }
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = SetWindowTextW(hwnd(slot), PCWSTR(wide.as_ptr()));
    }
}

/// Theme schedule subtitle under the theme row (Go formatThemeSchedule,
/// showSource = true). Fixed mode lists both times; sunrise mode lists the
/// solved solar times plus the location source.
fn theme_schedule_text() -> String {
    theme_schedule_summary(false)
}

/// Go formatThemeSchedule for both consumers: the panel schedule row (long,
/// with source attribution and fixed-time fallback) and the compact tray
/// tooltip line.
fn theme_schedule_summary(short: bool) -> String {
    let (mode, light, dark, ip_enabled) = cfg_map(|c| {
        (
            c.theme_mode.clone(),
            c.theme_light_time.clone(),
            c.theme_dark_time.clone(),
            c.theme_ip_location_enabled,
        )
    });
    let hhmm = |v: &str| -> String {
        if v.len() == 5 {
            v.to_string()
        } else {
            "--:--".to_string()
        }
    };
    if mode == "sunrise" {
        match theme_engine::solar_window(ip_enabled) {
            Some((rise, set)) => {
                let times = [
                    format!("{:02}:{:02}", rise / 60, rise % 60),
                    format!("{:02}:{:02}", set / 60, set % 60),
                ];
                if short {
                    t("theme_schedule_sunrise_short_format")
                        .replacen("%s", &times[0], 1)
                        .replacen("%s", &times[1], 1)
                } else {
                    let schedule = t("theme_schedule_sunrise_format")
                        .replacen("%s", &times[0], 1)
                        .replacen("%s", &times[1], 1);
                    let source = t(theme_engine::location(ip_enabled).2);
                    t("theme_schedule_source_format")
                        .replacen("%s", &schedule, 1)
                        .replacen("%s", &source, 1)
                }
            }
            None if short => t("theme_schedule_unavailable"),
            None => {
                let fixed = t("theme_schedule_format")
                    .replacen("%s", &hhmm(&light), 1)
                    .replacen("%s", &hhmm(&dark), 1);
                t_args("theme_schedule_fallback_format", &[&fixed])
            }
        }
    } else {
        let key = if short {
            "theme_schedule_short_format"
        } else {
            "theme_schedule_format"
        };
        t(key)
            .replacen("%s", &hhmm(&light), 1)
            .replacen("%s", &hhmm(&dark), 1)
    }
}

// ---- Stay awake ----------------------------------------------------------

/// Remaining duration and screen flag of the timed override; an expired
/// entry is cleared on read so no code path can observe a stale value.
pub(crate) fn timed_nosleep_state() -> Option<(std::time::Duration, bool)> {
    let mut guard = crate::runtime::lock(&NOSLEEP_TIMED);
    let timed = guard.as_ref()?;
    let now = std::time::Instant::now();
    if now < timed.until {
        Some((timed.until - now, timed.keep_screen))
    } else {
        *guard = None;
        None
    }
}

/// Arms the timed override (seconds are clamped) and schedules the one-shot
/// expiry. The saved switch and idle monitoring stay untouched: the runtime
/// merge handles both, so expiry simply restores the previous state.
pub(crate) fn set_timed_nosleep(seconds: u64, keep_screen: bool) {
    let seconds = seconds.clamp(1, NOSLEEP_TIMED_MAX_SECS);
    *runtime::lock(&NOSLEEP_TIMED) = Some(TimedNosleep {
        until: std::time::Instant::now() + std::time::Duration::from_secs(seconds),
        keep_screen,
    });
    unsafe {
        let _ = KillTimer(Some(hwnd(&HIDDEN)), NOSLEEP_TIMED_TIMER);
        let _ = SetTimer(
            Some(hwnd(&HIDDEN)),
            NOSLEEP_TIMED_TIMER,
            u32::try_from(seconds * 1000).unwrap_or(u32::MAX),
            None,
        );
    }
    apply_stay_awake();
    refresh_status();
}

/// Drops the timed override; no-op when nothing is armed.
pub(crate) fn clear_timed_nosleep() {
    if crate::runtime::lock(&NOSLEEP_TIMED).take().is_some() {
        unsafe {
            let _ = KillTimer(Some(hwnd(&HIDDEN)), NOSLEEP_TIMED_TIMER);
        }
        apply_stay_awake();
        refresh_status();
    }
}

/// Any request that leaves the saved switch off also drops the timed
/// override, so "off" always means off regardless of the source.
pub(crate) fn sync_timed_with_manual() {
    if !cfg_map(|c| c.nosleep_enabled) {
        clear_timed_nosleep();
    }
}

/// Short countdown for tooltip/status: whole minutes, or seconds below one.
fn format_remaining(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs >= 60 {
        format!(
            "{} {}",
            secs / 60,
            if i18n_is_chinese() { "分钟" } else { "min" }
        )
    } else {
        format!("{} {}", secs, if i18n_is_chinese() { "秒" } else { "s" })
    }
}

fn apply_stay_awake() {
    let power = effective_power_state();
    let effective = power.awake;
    let keep_screen = power.keep_screen;

    unsafe {
        let mut flags = ES_CONTINUOUS.0;
        if effective {
            flags |= ES_SYSTEM_REQUIRED.0;
            if keep_screen {
                flags |= ES_DISPLAY_REQUIRED.0;
            }
        }
        if SetThreadExecutionState(EXECUTION_STATE(flags)).0 == 0 {
            log_line("SetThreadExecutionState failed; retaining previous execution status");
            return;
        }
    }
    let previous = NOSLEEP_EXECUTION_ON.swap(effective, Ordering::SeqCst);
    if previous != effective {
        log_line(&format!("stay awake effective={effective}"));
    }
}

// ---- Idle monitor --------------------------------------------------------

fn idle_settings() -> idle_monitor::Settings {
    let power = effective_power_state();
    let (warning, action, enhanced) = cfg_map(|c| {
        (
            c.idle_warning_seconds,
            c.idle_action.clone(),
            c.idle_enhanced_monitor,
        )
    });
    let threshold = Duration::from_secs(power.idle_minutes as u64 * 60);
    #[cfg(feature = "devtools")]
    let threshold = if devtools::IDLE_MONITOR_TEST.load(Ordering::SeqCst) {
        Duration::from_secs(devtools::IDLE_TEST_SECONDS.load(Ordering::SeqCst) as u64)
    } else {
        threshold
    };
    idle_monitor::Settings {
        enabled: power.idle_running(),
        threshold,
        warning: Duration::from_secs(warning as u64),
        action,
        enhanced,
    }
}

fn input_sample() -> Option<(u32, Duration)> {
    unsafe {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if !GetLastInputInfo(&mut info).as_bool() {
            return None;
        }
        let delta = GetTickCount().wrapping_sub(info.dwTime);
        let delta = if delta > i32::MAX as u32 { 0 } else { delta };
        Some((info.dwTime, Duration::from_millis(u64::from(delta))))
    }
}

fn sample_idle_clock() -> idle_monitor::Update {
    let settings = idle_settings();
    let input = input_sample();
    #[cfg(feature = "devtools")]
    devtools::trace_idle_sample(input.map(|(tick, _)| tick));
    IDLE_MS.store(
        input.map_or(0, |(_, idle)| idle.as_millis() as i64),
        Ordering::SeqCst,
    );
    crate::runtime::lock(&IDLE_CLOCK).sample(std::time::Instant::now(), input, settings)
}

fn spawn_idle_thread() {
    std::thread::Builder::new()
        .name("idle-monitor".into())
        .spawn(|| {
            let mut tick: u32 = 0;
            while !EXITING.load(Ordering::SeqCst) {
                runtime::catch_and_log("idle-monitor", || {
                    let update = sample_idle_clock();
                    unsafe {
                        if update.cancel {
                            let _ = PostMessageW(
                                Some(hwnd(&HIDDEN)),
                                WM_IDLE_CANCEL,
                                WPARAM(0),
                                LPARAM(0),
                            );
                        }
                        if update.show
                            && PostMessageW(Some(hwnd(&HIDDEN)), WM_IDLE_WARN, WPARAM(0), LPARAM(0))
                                .is_err()
                        {
                            crate::runtime::lock(&IDLE_CLOCK).restart(std::time::Instant::now());
                        }
                    }
                    tick = tick.wrapping_add(1);
                    if tick.is_multiple_of(BATTERY_POLL_TICKS) {
                        refresh_battery();
                    }
                });
                std::thread::sleep(Duration::from_millis(250));
            }
        })
        .expect("spawn idle monitor");
}
fn refresh_battery() {
    unsafe {
        let mut status = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut status).is_ok() {
            let old_ac = ON_AC.swap(status.ACLineStatus != 0, Ordering::SeqCst);
            if old_ac != (status.ACLineStatus != 0) {
                theme_engine::wake();
            }
            let percent = if status.BatteryLifePercent <= 100 {
                status.BatteryLifePercent as i32
            } else {
                100
            };
            let old_percent = BATTERY_PERCENT.swap(percent, Ordering::SeqCst);
            // Go contract: low battery *pauses* Stay Awake at runtime and
            // restores it when AC returns — the user's manual toggle is
            // never rewritten.
            let was_blocked = BATTERY_BLOCKED.load(Ordering::SeqCst);
            let blocked = status.ACLineStatus == 0
                && cfg_map(|c| c.nosleep_enabled)
                && percent < cfg_map(|c| c.nosleep_battery_threshold);
            if blocked != was_blocked {
                BATTERY_BLOCKED.store(blocked, Ordering::SeqCst);
                log_line(&format!(
                    "stay awake {} by low battery (runtime pause)",
                    if blocked { "paused" } else { "resumed" }
                ));
            }
            if blocked != was_blocked
                || old_ac != (status.ACLineStatus != 0)
                || old_percent != percent
            {
                let _ = PostMessageW(Some(hwnd(&HIDDEN)), WM_REFRESH_UI, WPARAM(0), LPARAM(0));
            }
        }
    }
}

// ---- Idle warning overlay -------------------------------------------------

fn show_warning() {
    if !WARNING_PREVIEW_SESSION.load(Ordering::SeqCst) {
        sample_idle_clock();
        let countdown = crate::runtime::lock(&IDLE_CLOCK).countdown(std::time::Instant::now());
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
    log_line("idle warning shown");
}

fn update_warning_text(seconds: i32) {
    let action = t(&format!(
        "menu_action_{}",
        crate::runtime::lock(&WARN_ACTION).clone()
    ));
    let text = t("msg_idle_warning")
        .replace("%s", &action)
        .replace("%d", &seconds.to_string());
    set_text(&WARN_TEXT, &text);
}

fn center_on_screen(hwnd_: HWND) {
    unsafe {
        let mut wr = RECT::default();
        if GetWindowRect(hwnd_, &mut wr).is_err() {
            return;
        }
        let width = wr.right - wr.left;
        let height = wr.bottom - wr.top;
        let anchor = windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
        let (x, y) = display::centered_on(anchor, width, height);
        let _ = MoveWindow(hwnd_, x, y, width, height, false);
    }
}

fn tick_warning() {
    if !WARNING_ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    let (left, action) = if WARNING_PREVIEW_SESSION.load(Ordering::SeqCst) {
        (
            WARN_SECONDS_LEFT.fetch_sub(1, Ordering::SeqCst) - 1,
            crate::runtime::lock(&WARN_ACTION).clone(),
        )
    } else {
        sample_idle_clock();
        let countdown = crate::runtime::lock(&IDLE_CLOCK).countdown(std::time::Instant::now());
        let Some((seconds, action)) = countdown else {
            hide_warning("session no longer active");
            return;
        };
        (seconds as i32, action)
    };
    if left <= 0 {
        cancel_warning("timeout");
        log_line("idle action executing");
        execute_system_action(&action);
    } else {
        WARN_SECONDS_LEFT.store(left, Ordering::SeqCst);
        *crate::runtime::lock(&WARN_ACTION) = action;
        update_warning_text(left);
    }
}

fn cancel_warning(reason: &str) {
    crate::runtime::lock(&IDLE_CLOCK).restart(std::time::Instant::now());
    WARNING_PREVIEW_SESSION.store(false, Ordering::SeqCst);
    hide_warning(reason);
}

fn hide_warning(reason: &str) {
    if !WARNING_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    unsafe {
        let warning = hwnd(&WARNING);
        let _ = KillTimer(Some(warning), WARN_TIMER);
        let _ = ShowWindow(warning, SW_HIDE);
    }
    log_line(&format!("idle warning closed ({reason})"));
}
/// Executes one built-in system action by its config name. Shared by the
/// idle monitor, automatic tasks, the system-controls menu, and hotkeys.
pub fn execute_system_action(action: &str) {
    if let Err(error) = try_system_action(action) {
        log_line(&format!("{action} failed: {error}"));
        warn_dialog(
            "",
            &t_args(
                "msg_action_failed",
                &[&t(&format!("menu_action_{action}")), &error],
            ),
        );
    }
}

fn try_system_action(action: &str) -> Result<(), String> {
    if !matches!(
        action,
        "lock" | "sleep" | "hibernate" | "shutdown" | "restart"
    ) {
        return Err(format!("unsupported system action: {action}"));
    }
    #[cfg(feature = "devtools")]
    if devtools::preview_only() {
        log_line(&format!("preview: suppressed system action {action}"));
        return Ok(());
    }
    unsafe {
        match action {
            "lock" => LockWorkStation().map_err(|error| error.to_string()),
            "sleep" | "hibernate" => {
                if !ipc::suspend_available(action == "hibernate") {
                    return Err(t_pub(if action == "hibernate" {
                        "cli_error_hibernate_unavailable"
                    } else {
                        "cli_error_sleep_unavailable"
                    }));
                }
                if SetSuspendState(action == "hibernate", false, false) {
                    Ok(())
                } else {
                    let error = windows::core::Error::from_thread();
                    // A FALSE return without a last-error code would render
                    // as "operation completed successfully".
                    Err(if error.code().0 == 0 {
                        "suspend request was rejected".to_string()
                    } else {
                        error.to_string()
                    })
                }
            }
            _ => {
                enable_shutdown_privilege()?;
                let flags = if action == "shutdown" {
                    EWX_POWEROFF | EWX_FORCEIFHUNG
                } else {
                    EWX_REBOOT | EWX_FORCEIFHUNG
                };
                ExitWindowsEx(flags, Default::default()).map_err(|error| error.to_string())
            }
        }
    }
}
/// Enables SeShutdownPrivilege in the current process token so
/// `ExitWindowsEx` can actually power off or reboot. Mirrors Go's
/// `systemaction.go` LookupPrivilegeValue + AdjustTokenPrivileges flow.
unsafe fn enable_shutdown_privilege() -> Result<(), String> {
    use windows::Win32::Foundation::LUID;
    use windows::Win32::Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = windows::Win32::Foundation::HANDLE::default();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .is_err()
        {
            return Err(windows::core::Error::from_thread().to_string());
        }
        let name: Vec<u16> = "SeShutdownPrivilege".encode_utf16().chain([0]).collect();
        let mut luid = LUID::default();
        if LookupPrivilegeValueW(None, PCWSTR(name.as_ptr()), &mut luid).is_err() {
            let error = windows::core::Error::from_thread().to_string();
            let _ = windows::Win32::Foundation::CloseHandle(token);
            return Err(error);
        }
        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let result = AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None);
        let last_error = windows::Win32::Foundation::GetLastError();
        let _ = windows::Win32::Foundation::CloseHandle(token);
        result.map_err(|error| error.to_string())?;
        if last_error == windows::Win32::Foundation::ERROR_NOT_ALL_ASSIGNED {
            return Err(last_error.to_hresult().message().to_string());
        }
        Ok(())
    }
}

/// Native popup menu anchored to the system-controls button; selections run
/// immediately like the Go quick-actions popup.
unsafe fn show_system_controls_menu(owner: HWND) {
    // Go quick-actions: one owner-drawn choice popup above the button with
    // danger styling on shutdown/restart (menus.go openQuickMenu).
    const QUICK_ACTIONS: [(&str, bool); 5] = [
        ("menu_action_lock", false),
        ("menu_action_sleep", false),
        ("menu_action_hibernate", false),
        ("menu_action_shutdown", true),
        ("menu_action_restart", true),
    ];
    let button = unsafe { GetDlgItem(Some(owner), IDC_SYSTEM_BUTTON as i32) }.unwrap_or_default();
    if button.is_invalid() {
        return;
    }
    let items: Vec<(String, String, bool)> = QUICK_ACTIONS
        .iter()
        .enumerate()
        .map(|(index, (key, danger))| ((900 + index).to_string(), t(key), *danger))
        .collect();
    crate::choice::set_items_danger(button, &items);
    crate::choice::set_prefer_above(button, true);
    crate::choice::select_index(button, -1);
    crate::choice::toggle(button, owner, IDC_SYSTEM_BUTTON as i32);
}

// ---- Module-facing helpers -----------------------------------------------

pub fn t_pub(key: &str) -> String {
    t(key)
}

pub fn t_args(key: &str, arguments: &[&str]) -> String {
    idletrigger_core::i18n::format(&t(key), arguments)
}

/// Devtools warning preview entry.
#[cfg(feature = "devtools")]
pub fn popups_show_warning_preview() {
    WARNING_PREVIEW_SESSION.store(true, Ordering::SeqCst);
    *crate::runtime::lock(&WARN_ACTION) = cfg_map(|c| c.idle_action.clone());
    WARNING_ACTIVE.store(true, Ordering::SeqCst);
    WARN_SECONDS_LEFT.store(10, Ordering::SeqCst);
    show_warning();
}

/// Window capture for devtools.
#[cfg(feature = "devtools")]
pub fn capture_client_bmp_pub(hwnd_: HWND, path: &std::path::Path) -> std::io::Result<()> {
    capture::capture_client_bmp(hwnd_, path)
}

/// Title-bar icons for secondary windows (file-based extraction).
pub fn set_window_icons_pub(hwnd_: HWND) {
    set_window_icons(hwnd_, unsafe {
        windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default()
    });
}

pub fn scale_pub(v: i32) -> i32 {
    scale(v)
}

pub fn make_font_pub(size_px: i32, weight: i32) -> windows::Win32::Graphics::Gdi::HFONT {
    make_font(size_px, weight)
}

pub fn set_control_font_pub(control: HWND, font: windows::Win32::Graphics::Gdi::HFONT) -> bool {
    set_control_font(control, font)
}

#[cfg(test)]
#[test]
fn guarded_proc_catches_panics_and_degrades_to_default() {
    let result = guarded_proc(
        "test",
        HWND(std::ptr::null_mut()),
        0x0010,
        WPARAM(1),
        LPARAM(2),
        || panic!("boom"),
    );
    // DefWindowProcW on a null HWND returns 0; reaching here at all proves
    // the panic was caught instead of aborting the process.
    assert_eq!(result.0, 0);
}

#[cfg(test)]
#[test]
fn theme_changes_survive_native_messages() {
    use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, PM_REMOVE, PeekMessageW, WM_THEMECHANGED,
    };
    static NATIVE_REFRESHES: AtomicU32 = AtomicU32::new(0);
    static UNGUARDED_SHOWS: AtomicU32 = AtomicU32::new(0);
    unsafe extern "system" fn observe_show(
        window: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        _: usize,
        _: usize,
    ) -> LRESULT {
        if msg == windows::Win32::UI::WindowsAndMessaging::WM_SHOWWINDOW && wp.0 != 0 {
            let mut cloaked = 0u32;
            unsafe {
                let _ = windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                    window,
                    windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
                    (&mut cloaked as *mut u32).cast(),
                    size_of::<u32>() as u32,
                );
            }
            if cloaked & 1 == 0 {
                UNGUARDED_SHOWS.fetch_add(1, Ordering::SeqCst);
            }
        }
        unsafe { DefSubclassProc(window, msg, wp, lp) }
    }
    unsafe extern "system" fn observe_theme(
        window: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        _: usize,
        _: usize,
    ) -> LRESULT {
        if msg == WM_THEMECHANGED {
            NATIVE_REFRESHES.fetch_add(1, Ordering::SeqCst);
            // Reentrant native notifications must not retheme the subtree again.
            theme::apply_to_all();
            request_theme_refresh();
        }
        unsafe { DefSubclassProc(window, msg, wp, lp) }
    }
    const CHILD: &str = "IDLETRIGGER_TEST_THEME_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // Separate processes still share USER32's foreground window. Hold
        // the UI test lock until the child exits, just like in-process tests.
        let _ui_test = crate::runtime::lock(&CONFIG_TEST_LOCK);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "theme_changes_survive_native_messages",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .spawn()
            .unwrap();
        // A wedged child would hold the UI test lock forever and stall every
        // other native UI test in this binary.
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("theme child timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(status.success(), "theme child failed: {status}");
        return;
    }
    *crate::runtime::lock(&CONFIG) = Some(Default::default());
    *I18N.write().unwrap() = Some(I18n::load("en"));
    create_windows();
    let tray = tray_init().expect("test tray");
    FirstFrameGate::begin(hwnd(&PANEL)).reveal();
    unsafe {
        assert!(SetWindowSubclass(hwnd(&PANEL), Some(observe_show), 2, 0).as_bool());
        for warning in [hwnd(&WARNING), hwnd(&ACTION_WARN_HWND)] {
            assert!(SetWindowSubclass(warning, Some(observe_show), 2, 0).as_bool());
        }
    }
    {
        let frame = FrameTransition::begin(hwnd(&PANEL)).expect("visible panel");
        assert!(
            FrameTransition::begin(hwnd(&PANEL)).is_none(),
            "nested update must not own the outer cloak"
        );
        drop(frame);
    }
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::GetPropW;
        let panel = hwnd(&PANEL);
        let frame = FrameTransition::begin(panel).unwrap();
        FAIL_UNCLOAK.set((panel.0 as isize, 4));
        drop(frame); // All three immediate attempts fail.
        assert!(!GetPropW(panel, FRAME_RECOVERY_PROP).is_invalid());
        frame_recovery_proc(panel, WM_TIMER, FRAME_RECOVERY_TIMER, 0);
        assert!(!GetPropW(panel, FRAME_RECOVERY_PROP).is_invalid());
        frame_recovery_proc(panel, WM_TIMER, FRAME_RECOVERY_TIMER, 0);
        assert!(GetPropW(panel, FRAME_RECOVERY_PROP).is_invalid());
        let mut cloaked = 1u32;
        windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
            panel,
            windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
        .unwrap();
        assert_eq!(cloaked, 0, "delayed recovery must restore visibility");
        let frame = FrameTransition::begin(panel).unwrap();
        frame_recovery_proc(panel, WM_TIMER, FRAME_RECOVERY_TIMER, 0);
        windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
            panel,
            windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
        .unwrap();
        assert_eq!(
            cloaked & 1,
            1,
            "stale timers must not reveal a new transaction"
        );
        drop(frame);
        let frame = FrameTransition::begin(panel).unwrap();
        FAIL_UNCLOAK.set((panel.0 as isize, 3));
        drop(frame);
        show_panel(); // User-initiated recovery does not wait for the timer.
        assert!(GetPropW(panel, FRAME_RECOVERY_PROP).is_invalid());
    }
    unsafe {
        let button = GetDlgItem(Some(hwnd(&PANEL)), IDC_THEME_SWITCH as i32).unwrap();
        assert!(SetWindowSubclass(button, Some(observe_theme), 1, 0).as_bool());
    }
    for dark in [true, false, true, false] {
        theme::force_dark(dark);
        theme::apply_to_all();
        toggle_panel();
        assert!(!unsafe { IsWindowVisible(hwnd(&PANEL)) }.as_bool());
        toggle_panel();
        assert!(unsafe { IsWindowVisible(hwnd(&PANEL)) }.as_bool());
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            for warning in [hwnd(&WARNING), hwnd(&ACTION_WARN_HWND)] {
                let foreground = GetForegroundWindow();
                let focus = windows::Win32::UI::Input::KeyboardAndMouse::GetFocus();
                FirstFrameGate::begin(warning).reveal_no_activate();
                assert!(IsWindowVisible(warning).as_bool());
                assert_eq!(GetForegroundWindow(), foreground);
                assert_eq!(
                    windows::Win32::UI::Input::KeyboardAndMouse::GetFocus(),
                    focus
                );
                let mut cloaked = 1u32;
                windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                    warning,
                    windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
                    (&mut cloaked as *mut u32).cast(),
                    size_of::<u32>() as u32,
                )
                .unwrap();
                assert_eq!(cloaked, 0, "warning must be visible after its first frame");
                let _ = ShowWindow(warning, SW_HIDE);
            }
        }
        assert_eq!(UNGUARDED_SHOWS.load(Ordering::SeqCst), 0);
        let refreshes = NATIVE_REFRESHES.load(Ordering::SeqCst);
        assert!(refreshes > 0);
        theme::apply_to_all();
        assert_eq!(NATIVE_REFRESHES.load(Ordering::SeqCst), refreshes);
        theme::apply_to_all_after_repair();
        assert!(
            NATIVE_REFRESHES.load(Ordering::SeqCst) > refreshes,
            "system repair must refresh native styles even with the same palette"
        );
        refresh_status();
        present_layout(hwnd(&PANEL));
        unsafe {
            let setting: Vec<u16> = "ImmersiveColorSet".encode_utf16().chain([0]).collect();
            SendMessageW(
                hwnd(&HIDDEN),
                windows::Win32::UI::WindowsAndMessaging::WM_SETTINGCHANGE,
                None,
                Some(LPARAM(setting.as_ptr() as isize)),
            );
            SendMessageW(
                hwnd(&HIDDEN),
                windows::Win32::UI::WindowsAndMessaging::WM_THEMECHANGED,
                None,
                None,
            );
            let mut msg = msg_default();
            while PeekMessageW(
                &mut msg,
                None,
                WM_REFRESH_THEME,
                WM_REFRESH_THEME,
                PM_REMOVE,
            )
            .as_bool()
            {
                DispatchMessageW(&msg);
            }
            assert!(THEME_REFRESH.is_idle());
            let mut cloaked = 1u32;
            windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                hwnd(&PANEL),
                windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
                (&mut cloaked as *mut u32).cast(),
                size_of::<u32>() as u32,
            )
            .unwrap();
            assert_eq!(cloaked, 0, "completed frame must be visible to DWM");
        }
    }
    TRAY_PTR.store(0, Ordering::SeqCst);
    drop(tray);
    unsafe {
        DestroyWindow(hwnd(&PANEL)).unwrap();
        DestroyWindow(hwnd(&HIDDEN)).unwrap();
    }
}

#[cfg(test)]
mod power_state_tests {
    use super::*;

    /// Locks the truth table of the single power derivation shared by the
    /// panel status, tray tooltip, execution flags, and idle settings.
    #[test]
    fn effective_power_state_truth_table() {
        let _test = crate::runtime::lock(&CONFIG_TEST_LOCK);
        let previous_config = crate::runtime::lock(&CONFIG).replace(Default::default());
        let previous_overrides = automation::overrides();
        let previous_ac = ON_AC.swap(true, Ordering::SeqCst);
        let previous_battery = BATTERY_PERCENT.swap(100, Ordering::SeqCst);

        let set_config = |nosleep: bool, on_battery: bool, keep_screen: bool, idle: bool| {
            *crate::runtime::lock(&CONFIG) = Some(idletrigger_core::config::Config {
                nosleep_enabled: nosleep,
                nosleep_on_battery: on_battery,
                keep_screen_on: keep_screen,
                idle_enabled: idle,
                ..Default::default()
            });
        };
        let set_overrides = |stay_awake: bool,
                             pause_stay_awake: bool,
                             enable_idle: bool,
                             keep_screen_on: bool,
                             idle_minutes: i32| {
            automation::set_test_overrides(idletrigger_core::automation::EffectiveState {
                stay_awake,
                pause_stay_awake,
                enable_idle,
                keep_screen_on,
                idle_minutes,
                ..Default::default()
            });
        };

        // Manual off, idle on: idle runs, nothing requested.
        set_config(false, true, false, true);
        set_overrides(false, false, false, false, 0);
        let state = effective_power_state();
        assert!(!state.requested && !state.awake && state.idle_running());

        // Manual on: awake outranks and pauses the idle monitor.
        set_config(true, true, false, true);
        let state = effective_power_state();
        assert!(state.requested && state.awake && !state.keep_screen && !state.idle_running());

        // Battery below the threshold pauses Stay Awake at runtime only.
        ON_AC.store(false, Ordering::SeqCst);
        BATTERY_PERCENT.store(19, Ordering::SeqCst);
        let state = effective_power_state();
        assert!(state.requested && !state.awake && state.idle_running());
        // The pause is a runtime state; the config toggle stays untouched.
        assert!(
            crate::runtime::lock(&CONFIG)
                .as_ref()
                .is_some_and(|c| c.nosleep_enabled)
        );

        // Automation override keeps the screen awake alongside.
        ON_AC.store(true, Ordering::SeqCst);
        set_overrides(true, false, false, true, 0);
        let state = effective_power_state();
        assert!(state.awake && state.keep_screen);

        // Automation pause beats the override request.
        set_overrides(true, true, false, false, 0);
        let state = effective_power_state();
        assert!(state.requested && state.paused && !state.awake);

        // Idle override supplies its own minutes.
        set_config(false, true, false, false);
        set_overrides(false, false, true, false, 42);
        let state = effective_power_state();
        assert!(state.idle_requested && state.idle_running());
        // The override wins over the config's idle timeout minutes.
        assert_eq!(state.idle_minutes, 42);

        ON_AC.store(previous_ac, Ordering::SeqCst);
        BATTERY_PERCENT.store(previous_battery, Ordering::SeqCst);
        automation::set_test_overrides(previous_overrides);
        *crate::runtime::lock(&CONFIG) = previous_config;
    }

    #[test]
    fn timed_override_actives_stay_awake_without_touching_the_switch() {
        let _test = crate::runtime::lock(&CONFIG_TEST_LOCK);
        let previous_config = crate::runtime::lock(&CONFIG).replace(Default::default());
        let previous_overrides = automation::overrides();
        let previous_ac = ON_AC.swap(true, Ordering::SeqCst);
        let previous_battery = BATTERY_PERCENT.swap(100, Ordering::SeqCst);
        let previous_timed = crate::runtime::lock(&NOSLEEP_TIMED).take();

        // The overlay alone keeps the machine awake; the saved switch stays
        // off and the screen flag comes from the timed entry itself.
        *crate::runtime::lock(&NOSLEEP_TIMED) = Some(TimedNosleep {
            until: std::time::Instant::now() + std::time::Duration::from_secs(60),
            keep_screen: true,
        });
        let state = effective_power_state();
        assert!(state.requested && state.awake && state.keep_screen);
        assert!(!cfg_map(|c| c.nosleep_enabled));

        *crate::runtime::lock(&NOSLEEP_TIMED) = None;
        let state = effective_power_state();
        assert!(!state.requested && !state.awake);

        *crate::runtime::lock(&NOSLEEP_TIMED) = previous_timed;
        ON_AC.store(previous_ac, Ordering::SeqCst);
        BATTERY_PERCENT.store(previous_battery, Ordering::SeqCst);
        automation::set_test_overrides(previous_overrides);
        *crate::runtime::lock(&CONFIG) = previous_config;
    }
}

#[cfg(test)]
mod config_transaction_tests {
    use super::*;

    #[test]
    fn failed_or_stale_saves_leave_runtime_and_document_unchanged() {
        let _test = crate::runtime::lock(&CONFIG_TEST_LOCK);
        let dir = std::env::temp_dir().join(format!(
            "idletrigger-transaction-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("config.toml");
        let old_config = crate::runtime::lock(&CONFIG).replace(config::Config::default());
        let old_doc = crate::runtime::lock(&CONFIG_DOC).replace("custom = 42\n".parse().unwrap());
        let old_path = crate::runtime::lock(&CONFIG_PATH).replace(path.clone());
        let old_source = crate::runtime::lock(&CONFIG_SOURCE).take();
        let old_failed = CONFIG_LOAD_FAILED.swap(false, Ordering::SeqCst);
        edit_config(|c| c.nosleep_enabled = true).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(cfg_map(|c| c.nosleep_enabled));
        assert_eq!(
            crate::runtime::lock(&CONFIG_DOC).as_ref().unwrap()["custom"].as_integer(),
            Some(42)
        );

        std::fs::write(&path, format!("{saved}# external edit\n")).unwrap();
        assert!(edit_config(|c| c.nosleep_enabled = false).is_err());
        assert!(cfg_map(|c| c.nosleep_enabled));
        assert_eq!(
            crate::runtime::lock(&CONFIG_DOC)
                .as_ref()
                .unwrap()
                .to_string(),
            saved
        );
        std::fs::write(&path, &saved).unwrap();

        // A read-only destination makes the atomic replacement fail, after
        // candidate construction. Neither in-memory view may change.
        let original_permissions = std::fs::metadata(&path).unwrap().permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        let result = edit_config(|c| c.nosleep_enabled = false);
        std::fs::set_permissions(&path, original_permissions).unwrap();
        assert!(result.is_err());
        assert!(cfg_map(|c| c.nosleep_enabled));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
        assert_eq!(
            crate::runtime::lock(&CONFIG_DOC)
                .as_ref()
                .unwrap()
                .to_string(),
            saved
        );

        CONFIG_LOAD_FAILED.store(true, Ordering::SeqCst);
        assert!(edit_config(|c| c.nosleep_enabled = false).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
        std::fs::remove_file(&path).unwrap();
        assert!(hot_reload_config().is_err());
        assert!(cfg_map(|c| c.nosleep_enabled));
        *crate::runtime::lock(&CONFIG) = old_config;
        *crate::runtime::lock(&CONFIG_DOC) = old_doc;
        *crate::runtime::lock(&CONFIG_PATH) = old_path;
        *crate::runtime::lock(&CONFIG_SOURCE) = old_source;
        CONFIG_LOAD_FAILED.store(old_failed, Ordering::SeqCst);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
