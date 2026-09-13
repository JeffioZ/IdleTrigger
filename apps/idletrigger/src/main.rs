//! IdleTrigger — native Windows power automation tray utility (Rust rewrite).
//!
//! Vertical slice: single-instance tray app with a control panel for Stay
//! Awake and Idle Monitoring (lock/sleep/hibernate/shutdown/restart actions
//! with a cancellable non-activating countdown), config persisted to
//! `IdleTrigger.toml` next to the EXE using the Go-compatible schema.

#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicIsize, AtomicU32, Ordering};
use std::time::Duration;

use idletrigger_core::config;
use idletrigger_core::i18n::I18n;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::Graphics::Gdi::{CreateFontIndirectW, HDC, HFONT};
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
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BS_OWNERDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
    DestroyWindow, DispatchMessageW, FindWindowW, GetDlgItem, GetMessageW, GetSystemMetrics,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, HMENU, IDC_ARROW, IsWindowVisible,
    KillTimer, LoadCursorW, LoadIconW, MoveWindow, PostMessageW, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SM_CXSCREEN, SM_CYSCREEN, SPI_GETNONCLIENTMETRICS, SW_HIDE, SW_SHOW,
    SW_SHOWNOACTIVATE, SendMessageW, SetTimer, SetWindowTextW, ShowWindow, SystemParametersInfoW,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_DRAWITEM,
    WM_TIMER, WNDCLASSW, WS_CHILD, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_OVERLAPPED, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

const APP_VERSION: &str = match option_env!("IDLETRIGGER_VERSION") {
    Some(v) => v,
    None => concat!("rust-", env!("CARGO_PKG_VERSION"), "+build.unknown"),
};

// Control identifiers.
const IDC_NOSLEEP: usize = 110;
const IDC_IDLE: usize = 112;
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
const IDC_EXIT_BUTTON: usize = 140;
const IDC_WARN_TEXT: usize = 130;
const IDC_WARN_CANCEL: usize = 131;

// Owner-drawn static ranges: 201-209 section headers, 211-219 subtitles.
const STATIC_SECTION_BASE: usize = 201;
const STATIC_SUBTITLE_BASE: usize = 211;

// Fonts retained for WM_DRAWITEM painting (Go keeps them on the panel).
static PANEL_FONT_BODY: AtomicIsize = AtomicIsize::new(0);
static PANEL_FONT_SECTION: AtomicIsize = AtomicIsize::new(0);
static PANEL_FONT_SUBTITLE: AtomicIsize = AtomicIsize::new(0);

/// Go panel.create: "IdleTrigger" or "IdleTrigger v{version}".
fn panel_title_text() -> String {
    if APP_VERSION.is_empty() || APP_VERSION == "dev" {
        "IdleTrigger".to_string()
    } else {
        format!("IdleTrigger v{APP_VERSION}")
    }
}

fn panel_font_body() -> HFONT {
    HFONT(PANEL_FONT_BODY.load(Ordering::SeqCst) as *mut _)
}

fn panel_font_section() -> HFONT {
    HFONT(PANEL_FONT_SECTION.load(Ordering::SeqCst) as *mut _)
}

fn panel_font_subtitle() -> HFONT {
    HFONT(PANEL_FONT_SUBTITLE.load(Ordering::SeqCst) as *mut _)
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

mod automation;
mod automation_ui;
mod capture;
mod choice;
#[cfg(feature = "devtools")]
mod devtools;
mod display;
mod gpu_activity;
mod ipc;
mod iplocate;
mod nativeform;
mod paint;
mod popups;
mod settings_ui;
mod system;
mod theme;
mod theme_engine;
mod theme_repair;
mod tooltips;

const PANEL_TIMER: usize = 1;
const WARN_TIMER: usize = 2;
const POWER_STATUS_TIMER: usize = 3;
const BATTERY_POLL_TICKS: u32 = 120; // 250ms * 120 = 30s

// ---- Shared runtime state ------------------------------------------------

static I18N: std::sync::RwLock<Option<I18n>> = std::sync::RwLock::new(None);
static CONFIG: Mutex<Option<config::Config>> = Mutex::new(None);
static CONFIG_DOC: Mutex<Option<toml_edit::DocumentMut>> = Mutex::new(None);
static CONFIG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

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
static WAITING_INPUT_RESET: AtomicBool = AtomicBool::new(false);
static ON_AC: AtomicBool = AtomicBool::new(true);
static BATTERY_PERCENT: AtomicI32 = AtomicI32::new(100);
static BATTERY_BLOCKED: AtomicBool = AtomicBool::new(false);
static NOSLEEP_EXECUTION_ON: AtomicBool = AtomicBool::new(false);
static EXITING: AtomicBool = AtomicBool::new(false);

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
}

fn hwnd(slot: &AtomicIsize) -> HWND {
    HWND(slot.load(Ordering::SeqCst) as *mut core::ffi::c_void)
}

fn scale(v: i32) -> i32 {
    v * DPI_SCALE.load(Ordering::SeqCst) / 96
}

fn cfg_map<T>(f: impl FnOnce(&config::Config) -> T) -> T {
    let guard = CONFIG.lock().unwrap();
    f(guard.as_ref().expect("config initialized"))
}

fn cfg_edit<T>(f: impl FnOnce(&mut config::Config) -> T) -> T {
    let mut guard = CONFIG.lock().unwrap();
    f(guard.as_mut().expect("config initialized"))
}

fn log_line(msg: &str) {
    use std::io::Write;
    let Ok(mut guard) = LOG_FILE.lock() else {
        return;
    };
    if let Some(file) = guard.as_mut() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "[{now}] [{APP_VERSION}] {msg}");
        let _ = file.flush();
    }
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

    let mut effective_config = loaded.config.clone();
    #[cfg(feature = "devtools")]
    let devtools_active = devtools::load();
    #[cfg(feature = "devtools")]
    {
        if devtools_active {
            devtools::apply_idle_test(&mut effective_config);
            devtools::apply_config_overrides(&mut effective_config);
        }
    }
    #[cfg(not(feature = "devtools"))]
    let _ = &mut effective_config;

    *CONFIG.lock().unwrap() = Some(effective_config.clone());
    *CONFIG_DOC.lock().unwrap() = Some(loaded.document);
    *CONFIG_PATH.lock().unwrap() = Some(config_path.clone());
    apply_language(&effective_config.language);

    // effective_config carries the devtools FORCE_LOG override too.
    if effective_config.logging_enabled {
        init_log(&exe_dir);
    }
    log_line("IdleTrigger starting");
    if let Some(err) = &loaded.load_error {
        log_line(&format!("config load failed, defaults used: {err}"));
        let body = format!(
            "{}\n{}\n\n{}",
            t_pub("warning_config_defaults"),
            t_pub("warning_config_recovery"),
            err
        );
        warn_dialog("", &body);
    }

    if !acquire_single_instance() {
        request_show_from_second_instance();
        log_line("another instance is running; requested panel show and exiting");
        return;
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
    TRAY_ALIVE.store(true, Ordering::SeqCst);

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

    if startup_delay > 0 {
        std::thread::sleep(Duration::from_secs(startup_delay));
    }
    if !start_minimized {
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
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
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

fn acquire_single_instance() -> bool {
    use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
    use windows::Win32::System::Threading::CreateEventW;

    let name: Vec<u16> = "Local\\IdleTriggerSingleton"
        .encode_utf16()
        .chain([0])
        .collect();
    unsafe {
        match CreateEventW(None, true, false, PCWSTR(name.as_ptr())) {
            Ok(_handle) => windows::Win32::Foundation::GetLastError() != ERROR_ALREADY_EXISTS,
            Err(_) => true, // fail open: better a second instance than no app
        }
    }
}

fn request_show_from_second_instance() {
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

// ---- Logging -------------------------------------------------------------

fn init_log(exe_dir: &std::path::Path) {
    use std::io::Write;
    let path = exe_dir.join("IdleTrigger.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "---- session {} ----", std::process::id());
        *LOG_FILE.lock().unwrap() = Some(file);
    }
}

// ---- Windows creation ----------------------------------------------------

unsafe extern "system" fn hidden_proc(
    hwnd_: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let registered = REGISTERED_SHOW_PANEL.load(Ordering::SeqCst);
        if registered != 0 && msg == registered {
            show_panel();
            return LRESULT(0);
        }
        match msg {
            WM_TIMER if wparam.0 == POWER_STATUS_TIMER => {
                // Coalesced power-status re-read (drivers broadcast before
                // the status settles).
                let _ = KillTimer(Some(hwnd_), POWER_STATUS_TIMER);
                refresh_battery();
                apply_stay_awake();
                refresh_status();
                LRESULT(0)
            }
            WM_IDLE_WARN => {
                show_warning();
                LRESULT(0)
            }
            WM_IDLE_CANCEL => {
                cancel_warning("input");
                LRESULT(0)
            }
            WM_REFRESH_UI => {
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
                hot_reload_config();
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_SETTINGCHANGE => {
                if theme::is_color_set_change(lparam.0) && theme::refresh_from_registry() {
                    theme::apply_to_all();
                    // Title-bar marks follow the theme (Go WindowIcons.Apply
                    // on every form window).
                    let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                        .unwrap_or_default();
                    set_window_icons(hwnd_, instance);
                    // Also refresh secondary window title icons.
                    for w in [crate::hwnd(&crate::PANEL), crate::settings_ui::theme_hwnd()]
                        .into_iter()
                        .chain(crate::automation_ui::theme_hwnds())
                    {
                        if !w.is_invalid() {
                            set_window_icons(w, instance);
                        }
                    }
                    refresh_status();
                    tooltips::retheme();
                    log_line("system theme changed");
                }
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_SYSCOLORCHANGE
            | windows::Win32::UI::WindowsAndMessaging::WM_THEMECHANGED => {
                theme::refresh_from_registry();
                theme::apply_to_all();
                tooltips::retheme();
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
                        cfg_edit(|c| {
                            c.nosleep_enabled = !c.nosleep_enabled;
                            if c.nosleep_enabled {
                                c.idle_enabled = false;
                            }
                        });
                        persist_config();
                        apply_stay_awake();
                        refresh_checkboxes();
                        refresh_status();
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd_, msg, wparam, lparam),
        }
    }
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
    let scale = DPI_SCALE.load(Ordering::SeqCst);
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
                6,
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
    let (mut fill, mut border, mut text) =
        (p.surface, p.danger_surface_text, p.danger_surface_text);
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
    paint::draw_surface(dc, bounds, p.window_bg, fill, border, 6);
    paint::draw_button_label(dc, bounds, panel_font_body(), label, text, false, 8, 8);
    if state.focused {
        let inset = paint::sp(2, scale);
        paint::frame_rect(
            dc,
            &RECT {
                left: bounds.left + inset,
                top: bounds.top + inset,
                right: bounds.right - inset,
                bottom: bounds.bottom - inset,
            },
            p.danger_focus,
        );
    }
}

unsafe extern "system" fn panel_proc(
    hwnd_: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
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
                    show_system_controls_menu(hwnd_);
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
            windows::Win32::UI::WindowsAndMessaging::WM_DPICHANGED => {
                let dpi = ((wparam.0 >> 16) & 0xFFFF) as u32;
                recreate_ui_for_dpi(dpi, hwnd_);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd_, msg, wparam, lparam),
        }
    }
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

/// Rebuilds every visible window after a per-monitor DPI change. The hidden
/// window and registered classes survive.
unsafe fn recreate_ui_for_dpi(dpi: u32, old_panel: HWND) {
    unsafe {
        let was_visible = IsWindowVisible(old_panel).as_bool();
        let mut rect = RECT::default();
        let _ = GetWindowRect(old_panel, &mut rect);
        DPI_SCALE.store(dpi as i32, Ordering::SeqCst);
        let _ = DestroyWindow(old_panel);
        let _ = DestroyWindow(hwnd(&WARNING));
        let _ = DestroyWindow(hwnd(&ACTION_WARN_HWND));
        let _ = DestroyWindow(hwnd(&LOCK_NOTIFY_HWND));
        // A destroyed action-warning window never sends its close callback;
        // clear the busy flag so future countdowns are not silently skipped.
        let _ = automation::ACTION_BUSY.swap(false, Ordering::SeqCst);
        PANEL.store(0, Ordering::SeqCst);
        WARNING.store(0, Ordering::SeqCst);
        ACTION_WARN_HWND.store(0, Ordering::SeqCst);
        LOCK_NOTIFY_HWND.store(0, Ordering::SeqCst);
        create_windows();
        refresh_checkboxes();
        refresh_status();
        if was_visible {
            let _ = ShowWindow(hwnd(&PANEL), SW_SHOW);
        }
        log_line(&format!("ui rebuilt for dpi={dpi}"));
    }
}

unsafe extern "system" fn warning_proc(
    hwnd_: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
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
    }
}

static CLASSES_REGISTERED: AtomicBool = AtomicBool::new(false);

/// Creates the hidden window (once) plus every visible window; repeatable
/// after a DPI change destroys them.
fn create_windows() {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module handle");
        // Go font tokens: body 14/400, section 14/700, subtitle 12/600.
        let font = make_font(14, 400);
        let section_font = make_font(14, 700);
        let subtitle_font = make_font(12, 600);

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
        let panel = CreateWindowExW(
            panel_ex_style,
            w!("IdleTriggerPanel"),
            PCWSTR({
                let t: Vec<u16> = panel_title_text().encode_utf16().chain([0]).collect();
                t.leak().as_ptr()
            }),
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
        // Per-monitor DPI beats the system-wide guess now that the window
        // exists on its destination monitor.
        let window_dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(panel);
        if window_dpi > 0 {
            DPI_SCALE.store(window_dpi as i32, Ordering::SeqCst);
        }
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
        let (x, y) = panel_origin(panel_w, panel_h);
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

        PANEL_FONT_BODY.store(font.0 as isize, Ordering::SeqCst);
        PANEL_FONT_SECTION.store(section_font.0 as isize, Ordering::SeqCst);
        PANEL_FONT_SUBTITLE.store(subtitle_font.0 as isize, Ordering::SeqCst);

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

        let warn_cancel = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!("取消 / Cancel"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
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

/// Explicit small+big icons via WM_SETICON, extracted from our own EXE file
/// with LR_LOADFROMFILE — immune to how the resource compiler names entries.
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
                LR_DEFAULTCOLOR,
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
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point);
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
fn panel_origin(width: i32, height: i32) -> (i32, i32) {
    let Some(work) = cursor_work_area() else {
        return (CW_USEDEFAULT, CW_USEDEFAULT);
    };
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
        let dpi = DPI_SCALE.load(Ordering::SeqCst).max(96);
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
    // Go font.NewForLayout: YaHei UI has Regular/Light/Bold, not Semibold.
    // Weight 600 renders as synthetic semibold; convert to true Bold(700).
    let weight = if weight == 600 && i18n_is_chinese() {
        700
    } else {
        weight
    };
    unsafe {
        let mut metrics = windows::Win32::UI::WindowsAndMessaging::NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<windows::Win32::UI::WindowsAndMessaging::NONCLIENTMETRICSW>(
            ) as u32,
            ..Default::default()
        };
        if SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            Some(&mut metrics as *mut _ as *mut core::ffi::c_void),
            Default::default(),
        )
        .is_ok()
        {
            metrics.lfMessageFont.lfHeight = -scale(size_px);
            metrics.lfMessageFont.lfWeight = weight;
            return CreateFontIndirectW(&metrics.lfMessageFont);
        }
        HFONT::default()
    }
}

// ---- Tray ----------------------------------------------------------------

fn tray_init() -> Option<tray_icon::TrayIcon> {
    use tray_icon::TrayIconBuilder;
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};

    // Extract from our own EXE so it works on any machine, not just the
    // build host with its source tree; dark/light variant follows the theme.
    let icon = tray_icon_for_theme()?;

    let menu = Menu::new();
    let open = MenuItem::new(t("menu_open_panel"), true, None);
    let _ = menu.append(&open);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let exit = MenuItem::new(t("menu_exit"), true, None);
    let _ = menu.append(&exit);
    *MENU_OPEN_ID.lock().unwrap() = Some(open.id().clone());
    *MENU_EXIT_ID.lock().unwrap() = Some(exit.id().clone());

    let tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_tooltip("IdleTrigger")
        .with_menu_on_left_click(false) // Go: left toggles panel, right opens menu
        .build()
        .ok()?;
    // TrayIcon is not Send; keep a main-thread-only pointer so the status
    // refresh can update the tooltip. The leaked box owns the same object the
    // main loop drops at exit, so ownership stays single.
    let leaked = Box::into_raw(Box::new(tray));
    TRAY_PTR.store(leaked as isize, Ordering::SeqCst);
    // SAFETY: reconstruct the owned handle from the leaked pointer; the
    // leaked allocation is intentionally never freed before process exit.
    let tray = unsafe { std::ptr::read(leaked) };
    Some(tray)
}

/// Extracts an icon from this EXE's embedded resources via
/// `PrivateExtractIconsW` — the only reliable extraction API that works
/// from a running module's own file on all Windows builds tested.
/// `resource_id` selects a numeric icon-group resource (Go resourceid);
/// None extracts the first icon group.
fn exe_embedded_icon_by(resource_id: Option<i32>) -> Option<tray_icon::Icon> {
    use windows::Win32::UI::WindowsAndMessaging::PrivateExtractIconsW;
    let exe = std::env::current_exe().ok()?;
    // PrivateExtractIconsW wants a fixed 260-wchar path buffer.
    let mut path_buf = [0u16; 260];
    let path_str = exe.as_os_str().to_string_lossy();
    let path_chars: Vec<u16> = path_str.encode_utf16().collect();
    if path_chars.len() >= 260 {
        return None;
    }
    path_buf[..path_chars.len()].copy_from_slice(&path_chars);
    // Negative index = ordinal resource identifier (documented contract).
    let index = resource_id.map_or(0, |id| -id);
    unsafe {
        let mut icons: [windows::Win32::UI::WindowsAndMessaging::HICON; 1] = Default::default();
        let n = PrivateExtractIconsW(&path_buf, index, 32, 32, Some(&mut icons), None, 0);
        let icon = icons[0];
        if n == 0 || icon.is_invalid() {
            log_line("tray icon extraction from EXE failed");
            return None;
        }
        // Convert HICON to RGBA for tray-icon's Icon type.
        let rgba = hicon_to_rgba(icon)?;
        let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(icon);
        tray_icon::Icon::from_rgba(rgba, 32, 32).ok()
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
        let menu = tray_icon::menu::Menu::new();
        let open = tray_icon::menu::MenuItem::new(t("menu_open_panel"), true, None);
        let _ = menu.append(&open);
        let _ = menu.append(&tray_icon::menu::PredefinedMenuItem::separator());
        let exit = tray_icon::menu::MenuItem::new(t("menu_exit"), true, None);
        let _ = menu.append(&exit);
        *MENU_OPEN_ID.lock().unwrap() = Some(open.id().clone());
        *MENU_EXIT_ID.lock().unwrap() = Some(exit.id().clone());
        tray.set_menu(Some(Box::new(menu)));
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
fn build_tray_tooltip(effective_nosleep: bool, idle_running: bool) -> String {
    let line = |key: &str, value: String| {
        t("status_line")
            .replace("%s", &t(key))
            .replace("%s", &value)
    };

    let mut lines = vec![if APP_VERSION.is_empty() || APP_VERSION == "dev" {
        "IdleTrigger".to_string()
    } else {
        format!("IdleTrigger v{APP_VERSION}")
    }];

    // Stay awake: effective state, paused wording under battery block.
    let stay_awake = if effective_nosleep {
        t("status_short_on")
    } else if cfg_map(|c| c.nosleep_enabled) && BATTERY_BLOCKED.load(Ordering::SeqCst) {
        t("status_paused")
    } else {
        t("status_short_off")
    };
    lines.push(line("tooltip_nosleep", stay_awake));

    // Idle monitor: paused by stay-awake, or "Nm 动作" when running.
    if idle_running {
        let (minutes, action) = cfg_map(|c| (c.idle_timeout_minutes, c.idle_action.clone()));
        let unit = if crate::i18n_is_chinese() { "分" } else { "m" };
        let action_label = t(&format!("menu_action_{action}"));
        lines.push(line(
            "tooltip_idle",
            format!("{minutes}{unit} {action_label}"),
        ));
    } else if effective_nosleep {
        lines.push(line("tooltip_idle", t("status_paused")));
    } else {
        lines.push(line("tooltip_idle", t("status_short_off")));
    }

    lines.push(line("tooltip_theme", theme_tooltip_value_short()));

    let enabled_count = automation::RULES
        .lock()
        .unwrap()
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
            v[..5].to_string()
        } else {
            "--:--".to_string()
        }
    };
    if mode == "sunrise" {
        let (lat, lon) = if ip_enabled {
            crate::iplocate::cached().unwrap_or((39.9042, 116.4074))
        } else {
            (39.9042, 116.4074)
        };
        match theme_engine::solar_times(lat, lon, theme_engine::day_of_year_today()) {
            Some((rise, set)) => t("theme_schedule_sunrise_short_format")
                .replacen("%s", &format!("{:02}:{:02}", rise / 60, rise % 60), 1)
                .replacen("%s", &format!("{:02}:{:02}", set / 60, set % 60), 1),
            None => t("theme_schedule_unavailable"),
        }
    } else {
        t("theme_schedule_short_format")
            .replacen("%s", &hhmm(&light), 1)
            .replacen("%s", &hhmm(&dark), 1)
    }
}

/// Updates the tray tooltip with a status line capped like Go's 120 UTF-16
/// units. Only call on the UI thread.
fn tray_update_tooltip(line: &str) {
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
    unsafe {
        let tray = &*(ptr as *const tray_icon::TrayIcon);
        let _ = tray.set_tooltip(Some(&text));
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

fn set_dwm_boolean(window: HWND, attribute: u32, enabled: bool) -> bool {
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
    pub fn reveal(mut self) {
        unsafe {
            let _ = ShowWindow(self.window, SW_SHOW);
            present_frame(self.window);
            if self.cloaked && set_dwm_boolean(self.window, DWMWA_CLOAK, false) {
                present_frame(self.window);
            }
            if self.transitions_disabled {
                set_dwm_boolean(self.window, DWMWA_TRANSITIONS_FORCED_DISABLED, false);
                self.transitions_disabled = false;
            }
        }
    }
}

fn show_panel() {
    unsafe {
        let _ = ShowWindow(hwnd(&PANEL), SW_SHOW);
    }
}

fn toggle_panel() {
    unsafe {
        if IsWindowVisible(hwnd(&PANEL)).as_bool() {
            let _ = ShowWindow(hwnd(&PANEL), SW_HIDE);
        } else {
            let _ = ShowWindow(hwnd(&PANEL), SW_SHOW);
        }
    }
}

fn on_toggle(code: usize) {
    // Owner-draw toggles carry no native check state: a click means "invert
    // the current config value".
    let changed = match code {
        IDC_NOSLEEP => Some(cfg_edit(|c| {
            c.nosleep_enabled = !c.nosleep_enabled;
            if c.nosleep_enabled {
                c.idle_enabled = false;
            }
            "nosleep_enabled"
        })),
        IDC_IDLE => Some(cfg_edit(|c| {
            c.idle_enabled = !c.idle_enabled;
            if c.idle_enabled {
                c.nosleep_enabled = false;
            }
            "idle_enabled"
        })),
        IDC_AUTOMATION => Some(cfg_edit(|c| {
            c.automation_enabled = !c.automation_enabled;
            "automation_enabled"
        })),
        _ => None,
    };
    if let Some(key) = changed {
        log_line(&format!("setting changed: {key}"));
        persist_config();
        apply_stay_awake();
        refresh_checkboxes();
        refresh_status();
    }
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

fn persist_config() {
    let Some(path) = CONFIG_PATH.lock().unwrap().clone() else {
        return;
    };
    let cfgv = CONFIG.lock().unwrap().clone().expect("config");
    let mut doc_guard = CONFIG_DOC.lock().unwrap();
    let Some(doc) = doc_guard.as_mut() else {
        return;
    };
    if let Err(err) = config::save(&path, doc, &cfgv) {
        log_line(&format!("config save failed: {err}"));
        warn_dialog(
            "",
            &t_pub("msg_config_save_failed").replace("%s", &err.to_string()),
        );
    } else {
        log_line("config saved");
        // Remember our own write's mtime so the config watcher doesn't
        // treat it as an external edit (Go selfConfigWrite guard).
        if let Ok(meta) = std::fs::metadata(&path)
            && let Ok(modified) = meta.modified()
            && let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            *SELF_CONFIG_MTIME.lock().unwrap() = Some(dur.as_secs_f64());
        }
    }
    drop(doc_guard);
    automation::reload_rules();
}

/// Last mtime (seconds) written by this process; external watcher skips it.
static SELF_CONFIG_MTIME: Mutex<Option<f64>> = Mutex::new(None);
/// Last mtime seen by the config watcher.
static LAST_SEEN_MTIME: Mutex<Option<f64>> = Mutex::new(None);

/// Restarts all feature threads and hot-applies every setting after an
/// external config change (Go reloadConfig).
fn hot_reload_config() {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let config_path = exe_dir.join("IdleTrigger.toml");
    let loaded = config::load(&config_path);
    *CONFIG.lock().unwrap() = Some(loaded.config.clone());
    *CONFIG_DOC.lock().unwrap() = Some(loaded.document);
    apply_language(&loaded.config.language);
    if loaded.config.logging_enabled && LOG_FILE.lock().unwrap().is_none() {
        init_log(&exe_dir);
    }
    automation::reload_rules();
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
}

/// Spawns the config watcher: 3-second mtime poll, skipping our own writes
/// (Go config_watch.go decision logic).
fn spawn_config_watcher() {
    let config_path = CONFIG_PATH.lock().unwrap().clone();
    let Some(config_path) = config_path else {
        return;
    };
    // Seed the last-seen mtime from the current file.
    if let Ok(meta) = std::fs::metadata(&config_path)
        && let Ok(modified) = meta.modified()
        && let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH)
    {
        *LAST_SEEN_MTIME.lock().unwrap() = Some(dur.as_secs_f64());
    }
    std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            loop {
                if EXITING.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_secs(3));
                let Ok(meta) = std::fs::metadata(&config_path) else {
                    continue;
                };
                let Ok(modified) = meta.modified() else {
                    continue;
                };
                let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH) else {
                    continue;
                };
                let mtime = dur.as_secs_f64();
                let mut last_seen = LAST_SEEN_MTIME.lock().unwrap();
                if mtime <= last_seen.unwrap_or(0.0) {
                    continue;
                }
                let self_write = SELF_CONFIG_MTIME
                    .lock()
                    .unwrap()
                    .is_some_and(|own| (mtime - own).abs() < 0.05);
                *last_seen = Some(mtime);
                if self_write {
                    continue;
                }
                drop(last_seen);
                unsafe {
                    let _ = PostMessageW(
                        Some(hwnd(&HIDDEN)),
                        WM_EXTERNAL_RELOAD,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        })
        .expect("spawn config watcher");
}

fn set_checkbox(slot: &AtomicIsize, _checked: bool) {
    // Owner-draw buttons repaint from config via WM_DRAWITEM.
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd(slot)), None, false);
    }
}

fn refresh_checkboxes() {
    set_checkbox(&CHK_NOSLEEP, cfg_map(|c| c.nosleep_enabled));
    set_checkbox(&CHK_IDLE, cfg_map(|c| c.idle_enabled));
    set_checkbox(&CHK_AUTOMATION, cfg_map(|c| c.automation_enabled));
    invalidate_control(IDC_THEME_ENABLE);
}

fn refresh_status() {
    // Read both flags in one closure pass: cfg_map nests would deadlock on
    // the non-reentrant CONFIG mutex.
    let (nosleep_on, _idle_on, keep_screen_on) =
        cfg_map(|c| (c.nosleep_enabled, c.idle_enabled, c.keep_screen_on));
    // Go noSleepStatusText: automation pause > battery pause > keep-screen >
    // enabled > disabled.
    let nosleep_status = if automation::OVR.nosleep_paused.load(Ordering::SeqCst) {
        t("status_paused_by_automation")
    } else if nosleep_on && BATTERY_BLOCKED.load(Ordering::SeqCst) {
        t("status_paused_by_battery")
    } else if !nosleep_on {
        t("status_disabled")
    } else if keep_screen_on {
        t("status_enabled_keep_screen")
    } else {
        t("status_enabled")
    };
    // Go monitorStatusText: active "N 分钟 → 动作"; suspended by stay-awake
    // or automation; otherwise disabled.
    let idle_summary_running = cfg_map(|c| c.idle_enabled)
        && !automation::OVR.idle_paused.load(Ordering::SeqCst)
        && !nosleep_on;
    let idle_status = if idle_summary_running {
        let (minutes, action) = cfg_map(|c| (c.idle_timeout_minutes, c.idle_action.clone()));
        let action_label = t(&format!("menu_action_{action}"));
        t("status_monitor_active")
            .replace("%d", &minutes.to_string())
            .replace("%s", &action_label)
    } else if cfg_map(|c| c.idle_enabled) && nosleep_on {
        t("status_paused_by_nosleep")
    } else if cfg_map(|c| c.idle_enabled) && automation::OVR.idle_paused.load(Ordering::SeqCst) {
        t("status_paused_by_automation")
    } else {
        t("status_disabled")
    };
    let overview = format!(
        "{}{}{}{}",
        t("power_overview_prefix"),
        nosleep_status,
        t("power_overview_separator"),
        idle_status
    );
    set_text(&LBL_POWER_SUMMARY, &overview);

    let (automation_on, rule_count) = (
        cfg_map(|c| c.automation_enabled),
        automation::RULES.lock().unwrap().len(),
    );
    let enabled_count = automation::RULES
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.enabled)
        .count();
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

    // Tray tooltip mirrors the effective power-management state.
    let effective_nosleep = NOSLEEP_EXECUTION_ON.load(Ordering::SeqCst);
    let idle_running = (cfg_map(|c| c.idle_enabled)
        || automation::OVR.idle_on.load(Ordering::SeqCst))
        && !automation::OVR.idle_paused.load(Ordering::SeqCst)
        && !effective_nosleep;
    tray_update_tooltip(&build_tray_tooltip(effective_nosleep, idle_running));
    tray_refresh_theme_icon();
    tooltips::refresh_all(hwnd(&PANEL));
}

fn set_text(slot: &AtomicIsize, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = SetWindowTextW(hwnd(slot), PCWSTR(wide.as_ptr()));
    }
}

/// Theme schedule subtitle under the theme row (Go formatThemeSchedule,
/// showSource = true). Fixed mode lists both times; sunrise mode lists the
/// solved solar times plus the location source.
fn theme_schedule_text() -> String {
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
        let (lat, lon) = if ip_enabled {
            crate::iplocate::cached().unwrap_or((39.9042, 116.4074))
        } else {
            (39.9042, 116.4074)
        };
        let (rise, set) = theme_engine::solar_times(lat, lon, theme_engine::day_of_year_today())
            .unwrap_or((0, 0));
        let schedule = t("theme_schedule_sunrise_format")
            .replacen("%s", &format!("{:02}:{:02}", rise / 60, rise % 60), 1)
            .replacen("%s", &format!("{:02}:{:02}", set / 60, set % 60), 1);
        let source = if ip_enabled && crate::iplocate::cached().is_some() {
            t("theme_location_ip")
        } else {
            t("theme_location_default")
        };
        t("theme_schedule_source_format")
            .replacen("%s", &schedule, 1)
            .replacen("%s", &source, 1)
    } else {
        t("theme_schedule_format")
            .replacen("%s", &hhmm(&light), 1)
            .replacen("%s", &hhmm(&dark), 1)
    }
}

// ---- Stay awake ----------------------------------------------------------

fn apply_stay_awake() {
    // Low battery is a runtime pause, not a config rewrite: the manual
    // toggle survives and resumes when AC returns (Go battery.go contract).
    let manual = cfg_map(|c| {
        if !c.nosleep_enabled || BATTERY_BLOCKED.load(Ordering::SeqCst) {
            return false;
        }
        if !ON_AC.load(Ordering::SeqCst) && !c.nosleep_on_battery {
            return false;
        }
        true
    });
    // Runtime overrides from automatic tasks layer on top of the manual
    // toggles without rewriting them.
    let auto_on = automation::OVR.nosleep_on.load(Ordering::SeqCst);
    let auto_keep_screen = automation::OVR.keep_screen.load(Ordering::SeqCst);
    let paused = automation::OVR.nosleep_paused.load(Ordering::SeqCst);
    let effective = if auto_on { !paused } else { manual };
    let keep_screen = if auto_on {
        auto_keep_screen
    } else {
        cfg_map(|c| c.keep_screen_on)
    };

    let previous = NOSLEEP_EXECUTION_ON.swap(effective, Ordering::SeqCst);
    if previous != effective {
        log_line(&format!("stay awake effective={effective}"));
    }
    unsafe {
        let mut flags = ES_CONTINUOUS.0;
        if effective {
            flags |= ES_SYSTEM_REQUIRED.0;
            if keep_screen {
                flags |= ES_DISPLAY_REQUIRED.0;
            }
        }
        let _ = SetThreadExecutionState(EXECUTION_STATE(flags));
    }
}

// ---- Idle monitor --------------------------------------------------------

fn spawn_idle_thread() {
    std::thread::Builder::new()
        .name("idle-monitor".into())
        .spawn(|| {
            let mut last_idle = 0i64;
            let mut tick: u32 = 0;
            let started_at = std::time::Instant::now();
            // Startup clamp: if the machine was already idle before launch,
            // don't fire an instant warning/action (Go StartWindowClamped).
            let startup_clamp_until = std::time::Instant::now() + Duration::from_secs(5);
            // Enhanced-mode periodic-input filter state (Go classifyInputReset):
            // tracks consecutive near-equal input gaps inside the 20s–2min
            // window to ignore injected periodic ticks.
            let mut periodic_count: u32 = 0;
            let mut periodic_baseline: i64 = 0;
            let mut last_input_at: Option<std::time::Instant> = None;
            let mut lock_states: [(i32, i16); 3] = [
                (popups::VK_CAPITAL, 0),
                (popups::VK_NUMLOCK, 0),
                (popups::VK_SCROLL, 0),
            ];
            // Seed lock-key states so startup doesn't fire notices.
            for (vk, last) in lock_states.iter_mut() {
                *last = popups::poll_state(*vk);
            }
            loop {
                if EXITING.load(Ordering::SeqCst) {
                    return;
                }
                let idle_ms = last_input_idle_ms();
                IDLE_MS.store(idle_ms, Ordering::SeqCst);

                // A falling idle value means fresh keyboard/mouse input.
                if idle_ms < last_idle {
                    let enhanced = cfg_map(|c| c.idle_enhanced_monitor);
                    let mut accept = true;
                    if enhanced {
                        // Classify the gap between observed resets: gaps in
                        // the 20s–2min window that repeat 3× within ±5s of a
                        // rolling baseline are injected periodic ticks, not
                        // human input (Go ignored_as_periodic_input).
                        if let Some(prev) = last_input_at {
                            let gap = prev.elapsed().as_millis() as i64;
                            if (20_000..=120_000).contains(&gap) {
                                if periodic_count == 0 || (gap - periodic_baseline).abs() <= 5_000 {
                                    periodic_baseline = if periodic_count == 0 {
                                        gap
                                    } else {
                                        (periodic_baseline + gap) / 2
                                    };
                                    periodic_count += 1;
                                    if periodic_count >= 3 {
                                        accept = false; // ignored as periodic
                                    }
                                } else {
                                    periodic_count = 1;
                                    periodic_baseline = gap;
                                }
                            } else {
                                periodic_count = 0;
                                periodic_baseline = 0;
                            }
                        }
                    }
                    if accept {
                        periodic_count = 0;
                        WAITING_INPUT_RESET.store(false, Ordering::SeqCst);
                        if WARNING_ACTIVE.load(Ordering::SeqCst) {
                            unsafe {
                                let _ = PostMessageW(
                                    Some(hwnd(&HIDDEN)),
                                    WM_IDLE_CANCEL,
                                    WPARAM(0),
                                    LPARAM(0),
                                );
                            }
                        }
                    }
                    last_input_at = Some(std::time::Instant::now());
                }
                last_idle = idle_ms;

                popups::poll(&mut lock_states);

                tick += 1;
                if tick.is_multiple_of(BATTERY_POLL_TICKS) {
                    refresh_battery();
                }

                let (manual_idle, manual_minutes, warn_seconds) = cfg_map(|c| {
                    (
                        c.idle_enabled,
                        c.idle_timeout_minutes,
                        c.idle_warning_seconds,
                    )
                });
                let auto_idle = automation::OVR.idle_on.load(Ordering::SeqCst);
                let idle_paused = automation::OVR.idle_paused.load(Ordering::SeqCst);
                let threshold_ms = (if auto_idle {
                    automation::OVR.idle_minutes.load(Ordering::SeqCst) as i64
                } else {
                    manual_minutes as i64
                }) * 60_000;
                let warn_ms = warn_seconds as i64 * 1000;
                let armed = (manual_idle || auto_idle)
                    && !idle_paused
                    && !NOSLEEP_EXECUTION_ON.load(Ordering::SeqCst)
                    && !WAITING_INPUT_RESET.load(Ordering::SeqCst);

                if armed
                    && !WARNING_ACTIVE.load(Ordering::SeqCst)
                    && std::time::Instant::now() >= startup_clamp_until
                    && threshold_ms - warn_ms > 0
                    && idle_ms >= threshold_ms - warn_ms
                    // Second clamp leg: the *process* must have run a while
                    // AND the machine idle must predate the process start —
                    // i.e. don't fire within the first threshold of runtime.
                    && idle_ms + (started_at.elapsed().as_millis() as i64) >= threshold_ms
                {
                    WARNING_ACTIVE.store(true, Ordering::SeqCst);
                    WARN_SECONDS_LEFT.store(warn_seconds, Ordering::SeqCst);
                    unsafe {
                        let _ =
                            PostMessageW(Some(hwnd(&HIDDEN)), WM_IDLE_WARN, WPARAM(0), LPARAM(0));
                    }
                }

                std::thread::sleep(Duration::from_millis(250));
            }
        })
        .expect("spawn idle monitor");
}

fn last_input_idle_ms() -> i64 {
    unsafe {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if GetLastInputInfo(&mut info).as_bool() {
            let delta = GetTickCount().wrapping_sub(info.dwTime) as i64;
            // Another process can inject input with a future timestamp,
            // making the delta wrap to ~4.29e9. Treat that as fresh activity
            // (Go elapsedSinceLastInput clamps negatives to 0).
            if delta < 0 || delta > i32::MAX as i64 {
                0
            } else {
                delta
            }
        } else {
            0
        }
    }
}

fn refresh_battery() {
    unsafe {
        let mut status = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut status).is_ok() {
            ON_AC.store(status.ACLineStatus == 1, Ordering::SeqCst);
            let percent = if status.BatteryLifePercent <= 100 {
                status.BatteryLifePercent as i32
            } else {
                100
            };
            BATTERY_PERCENT.store(percent, Ordering::SeqCst);
            // Go contract: low battery *pauses* Stay Awake at runtime and
            // restores it when AC returns — the user's manual toggle is
            // never rewritten.
            let was_blocked = BATTERY_BLOCKED.load(Ordering::SeqCst);
            let blocked = status.ACLineStatus != 1
                && cfg_map(|c| c.nosleep_enabled)
                && percent < cfg_map(|c| c.nosleep_battery_threshold);
            if blocked != was_blocked {
                BATTERY_BLOCKED.store(blocked, Ordering::SeqCst);
                log_line(&format!(
                    "stay awake {} by low battery (runtime pause)",
                    if blocked { "paused" } else { "resumed" }
                ));
                let _ = PostMessageW(Some(hwnd(&HIDDEN)), WM_REFRESH_UI, WPARAM(0), LPARAM(0));
            }
        }
    }
}

// ---- Idle warning overlay -------------------------------------------------

fn show_warning() {
    update_warning_text(WARN_SECONDS_LEFT.load(Ordering::SeqCst));
    unsafe {
        let warning = hwnd(&WARNING);
        center_on_screen(warning);
        let _ = ShowWindow(warning, SW_SHOWNOACTIVATE);
        let _ = SetTimer(Some(warning), WARN_TIMER, 1000, None);
    }
    log_line("idle warning shown");
}

fn update_warning_text(seconds: i32) {
    let action = t(&format!(
        "menu_action_{}",
        cfg_map(|c| c.idle_action.clone())
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
        let x = (GetSystemMetrics(SM_CXSCREEN) - width) / 2;
        let y = (GetSystemMetrics(SM_CYSCREEN) - height) / 2;
        let _ = MoveWindow(hwnd_, x, y, width, height, false);
    }
}

fn tick_warning() {
    let left = WARN_SECONDS_LEFT.fetch_sub(1, Ordering::SeqCst) - 1;
    if left <= 0 {
        cancel_warning("timeout");
        execute_idle_action();
    } else {
        update_warning_text(left);
    }
}

fn cancel_warning(reason: &str) {
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

fn execute_idle_action() {
    log_line("idle action executing");
    let action = cfg_map(|c| c.idle_action.clone());
    execute_system_action(&action);
    // The idle clock only re-arms after real input resets the idle time.
    WAITING_INPUT_RESET.store(true, Ordering::SeqCst);
}

/// Executes one built-in system action by its config name. Shared by the
/// idle monitor, automatic tasks, the system-controls menu, and hotkeys.
pub fn execute_system_action(action: &str) {
    unsafe {
        match action {
            "lock" => {
                let _ = LockWorkStation();
            }
            "sleep" => {
                let _ = SetSuspendState(false, false, false);
            }
            "hibernate" => {
                let _ = SetSuspendState(true, false, false);
            }
            "shutdown" | "restart" => {
                enable_shutdown_privilege();
                let flags = if action == "shutdown" {
                    EWX_POWEROFF | EWX_FORCEIFHUNG
                } else {
                    EWX_REBOOT | EWX_FORCEIFHUNG
                };
                let result = ExitWindowsEx(flags, Default::default());
                if let Err(err) = result {
                    log_line(&format!("{action} failed (privilege or policy denied)"));
                    let action_name = match action {
                        "shutdown" => t_pub("menu_shutdown"),
                        _ => t_pub("menu_restart"),
                    };
                    warn_dialog(
                        "",
                        &t_pub("msg_action_failed")
                            .replace("%s", &action_name)
                            .replace("%s", &err.to_string()),
                    );
                }
            }
            _ => {
                let _ = LockWorkStation();
            }
        }
    }
}

/// Enables SeShutdownPrivilege in the current process token so
/// `ExitWindowsEx` can actually power off or reboot. Mirrors Go's
/// `systemaction.go` LookupPrivilegeValue + AdjustTokenPrivileges flow.
unsafe fn enable_shutdown_privilege() {
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
            return;
        }
        let name: Vec<u16> = "SeShutdownPrivilege".encode_utf16().chain([0]).collect();
        let mut luid = LUID::default();
        if LookupPrivilegeValueW(None, PCWSTR(name.as_ptr()), &mut luid).is_err() {
            let _ = windows::Win32::Foundation::CloseHandle(token);
            return;
        }
        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let _ = AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None);
        let _ = windows::Win32::Foundation::CloseHandle(token);
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

/// Devtools warning preview entry (feature-gated call site).
pub fn popups_show_warning_preview() {
    show_warning();
}

/// Window capture for devtools (feature-gated call site).
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
