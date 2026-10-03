//! IdleTrigger — native Windows power automation tray utility.
//!
//! Single-instance tray app with a control panel for Stay
//! Awake and Idle Monitoring (lock/sleep/hibernate/shutdown/restart actions
//! with a cancellable non-activating countdown), config persisted to
//! `IdleTrigger.toml` next to the EXE using stable configuration fields.

#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicI64, AtomicIsize, AtomicU32, AtomicUsize, Ordering,
};
use std::time::Duration;

use idletrigger_core::config;
use idletrigger_core::i18n::I18n;

use layout::{
    BUTTON_H, CARD_GAP, CARD_PAD_X, CARD_PAD_Y, CHIP_H, GAP, LABEL_GAP, LINK_BOX_H, PAD, SECTION_H,
    SUBTITLE_H, SWITCH_HIT_W, TITLE_GAP, row_slot,
};
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
    EWX_FORCEIFHUNG, EWX_LOGOFF, EWX_POWEROFF, EWX_REBOOT, ExitWindowsEx, LockWorkStation,
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
    DispatchMessageW, FindWindowW, GetDlgCtrlID, GetDlgItem, GetMessageW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, HMENU, HWND_BROADCAST, IDC_ARROW, IDC_HAND,
    IsWindowVisible, KillTimer, LoadCursorW, LoadIconW, MoveWindow, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SC_MONITORPOWER, SMTO_ABORTIFHUNG, SW_HIDE, SW_SHOW,
    SW_SHOWNOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SendMessageTimeoutW, SendMessageW, SetCursor,
    SetTimer, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_DRAWITEM, WM_SYSCOMMAND, WM_TIMER,
    WNDCLASSW, WS_CHILD, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPED,
    WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
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
const IDC_THEME_SNOOZE_30M: usize = 156;
const IDC_THEME_SNOOZE_1H: usize = 157;
const IDC_THEME_SNOOZE_MORNING: usize = 158;
const IDC_THEME_SNOOZE_CANCEL: usize = 159;
const IDC_EXIT_BUTTON: usize = 140;
const IDC_WARN_TEXT: usize = 130;
const IDC_WARN_CANCEL: usize = 131;

// Owner-drawn static ranges: 201-209 section headers, 211-219 subtitles,
// 221-229 plain row labels, 230-239 section card backgrounds.
const STATIC_SECTION_BASE: usize = 201;
const STATIC_SUBTITLE_BASE: usize = 211;
const IDC_ROW_LABEL_BASE: usize = 221;
const IDC_CARD_BASE: usize = 230;

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

/// Status lines use regular weight so they read a step quieter than the
/// semibold action links sharing their rows.
fn panel_font_status() -> HFONT {
    make_font(12, 400)
}

// Layout tokens (96-DPI logical pixels). One rounded card per feature
// section: group label, single-control rows, and a status line on the card
// face; section actions sit at the right end of the header row. A bare
// button row closes the panel — the cards separate it, no hairline needed.
const PANEL_CLIENT_WIDTH: i32 = 486;
// Exact flow: pad12 + (title 18 + 4 + card(8 + 36+6 + 28+6 + 20 + 8)) + 10
// + (18 + 4 + card(8 + 36+6 + 20 + 8)) + 10
// + (18 + 4 + card(8 + 36+6 + 28+6 + 20 + 8)) + 10 + 36 + pad12 = 458.
const PANEL_CLIENT_HEIGHT: i32 = 458;
// Internal window messages (WM_APP range). 0x8001 is tray::CALLBACK_MSG.
const WM_IDLE_WARN: u32 = 0x8002;
const WM_IDLE_CANCEL: u32 = 0x8003;
const WM_REFRESH_UI: u32 = 0x8004;
const WM_ACTION_SHOW: u32 = 0x8005;
const WM_LOCK_NOTIFY: u32 = 0x8006;
const WM_EXTERNAL_RELOAD: u32 = 0x8007;
const WM_IPC_REQUEST: u32 = 0x8008;
const WM_REFRESH_THEME: u32 = 0x8009;
/// Queued focus-tip dismissal (see tooltips::dismiss_focus_tip).
const WM_FOCUSTIP_DISMISS: u32 = 0x800A;
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
mod layout;
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
mod tray;
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
static CHK_NOSLEEP: AtomicIsize = AtomicIsize::new(0);
static CHK_IDLE: AtomicIsize = AtomicIsize::new(0);
static CHK_AUTOMATION: AtomicIsize = AtomicIsize::new(0);
static LBL_POWER_SUMMARY: AtomicIsize = AtomicIsize::new(0);
static LBL_AUTOMATION_SUMMARY: AtomicIsize = AtomicIsize::new(0);
static LBL_THEME_SCHEDULE: AtomicIsize = AtomicIsize::new(0);
static WARN_TEXT: AtomicIsize = AtomicIsize::new(0);

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
static BTN_THEME_SNOOZE_CANCEL: AtomicIsize = AtomicIsize::new(0);
/// Workstation lock state (this session), kept current by WTS events.
static SESSION_LOCKED: AtomicBool = AtomicBool::new(false);
/// Which panel chip armed each preset row (control id, 0 = none or armed
/// from the CLI). The two rows are independent features, so each tracks its
/// own armed chip; within a row the latest click replaces the previous one.
static TIMED_ARMED_CHIP: AtomicUsize = AtomicUsize::new(0);
static SNOOZE_ARMED_CHIP: AtomicUsize = AtomicUsize::new(0);

/// Switch thumb animation: per-control fractional position and travel
/// direction, driven by the panel timer while a run is in flight.
struct SwitchFlight {
    /// Whether the flight heads toward the switch's on position.
    to_now: bool,
    started: std::time::Instant,
    /// The control being flown: flights drive any window's switch rows
    /// (panel, settings, editor), invalidating the control directly. HWNDs
    /// are only ever touched on the UI thread; the wrapper keeps the
    /// static Mutex happy.
    control: SendHwnd,
}

/// HWND that asserts UI-thread confinement for cross-window flight driving.
#[derive(Clone, Copy, PartialEq, Eq)]
struct SendHwnd(usize);

impl SendHwnd {
    fn get(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

unsafe impl Send for SendHwnd {}

static SWITCH_FLIGHTS: Mutex<Vec<SwitchFlight>> = Mutex::new(Vec::new());
/// Switch thumb flight duration (ms). The paint eases the progress, so
/// USER32's coarse timer granularity still reads as a smooth glide.
const SWITCH_ANIM_MS: u64 = 180;
const SWITCH_ANIM_TICK: u64 = 15;

fn is_chip_id(id: usize) -> bool {
    matches!(
        id,
        IDC_NOSLEEP_TIMED_30M
            | IDC_NOSLEEP_TIMED_1H
            | IDC_NOSLEEP_TIMED_2H
            | IDC_THEME_SNOOZE_30M
            | IDC_THEME_SNOOZE_1H
            | IDC_THEME_SNOOZE_MORNING
    )
}

/// Action links on section header rows (owner-draw buttons drawn as
/// hyperlinks so they keep native input, tab order, and accessibility).
fn is_link_id(id: usize) -> bool {
    matches!(
        id,
        IDC_NOSLEEP_TIMED_CANCEL | IDC_THEME_SNOOZE_CANCEL | IDC_MANAGE_BUTTON | IDC_THEME_REPAIR
    )
}

/// The instant-action theme chip names the side it will switch to.
fn theme_switch_chip_label() -> String {
    if theme::is_dark() {
        t("theme_switch_to_light")
    } else {
        t("theme_switch_to_dark")
    }
}

/// Whether this chip's preset is the armed one in its row.
fn chip_armed(id: usize) -> bool {
    match id {
        IDC_NOSLEEP_TIMED_30M | IDC_NOSLEEP_TIMED_1H | IDC_NOSLEEP_TIMED_2H => {
            TIMED_ARMED_CHIP.load(Ordering::SeqCst) == id
        }
        IDC_THEME_SNOOZE_30M | IDC_THEME_SNOOZE_1H | IDC_THEME_SNOOZE_MORNING => {
            SNOOZE_ARMED_CHIP.load(Ordering::SeqCst) == id
        }
        _ => false,
    }
}

/// Timed stay-awake override: a runtime-only overlay source above the saved
/// switch. Expiry rides a one-shot timer on the hidden window; restarts drop
/// it because nothing is persisted (task overrides behave the same way).
struct TimedNosleep {
    until: std::time::Instant,
    keep_screen: bool,
}
static NOSLEEP_TIMED: Mutex<Option<TimedNosleep>> = Mutex::new(None);
pub(crate) const NOSLEEP_TIMED_MAX_SECS: u64 = 24 * 60 * 60;

/// Panel timer id for switch thumb animation ticks. IDs 1-5 are taken by
/// the panel/warning/power/lock/timed timers on the same window.
const SWITCH_ANIM_TIMER: usize = 6;

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
        (IDC_ROW_LABEL_BASE + 2, "power_nosleep"),
        (IDC_ROW_LABEL_BASE + 3, "power_idle"),
        // Switch controls carry the same text as their row labels for
        // screen readers (their painter draws geometry only).
        (IDC_NOSLEEP, "power_nosleep"),
        (IDC_IDLE, "power_idle"),
        (IDC_NOSLEEP_TIMED_30M, "chip_timed_30m"),
        (IDC_NOSLEEP_TIMED_1H, "chip_timed_1h"),
        (IDC_NOSLEEP_TIMED_2H, "chip_timed_2h"),
        (IDC_ROW_LABEL_BASE, "automation_master"),
        (IDC_ROW_LABEL_BASE + 1, "menu_theme_enable"),
        // Switch controls carry the same text as their row labels for
        // screen readers (their painter draws geometry only).
        (IDC_AUTOMATION, "automation_master"),
        (IDC_THEME_ENABLE, "menu_theme_enable"),
        (IDC_THEME_SNOOZE_30M, "chip_snooze_30m"),
        (IDC_THEME_SNOOZE_1H, "chip_snooze_1h"),
        (IDC_THEME_SNOOZE_MORNING, "chip_snooze_morning"),
        (IDC_NOSLEEP_TIMED_CANCEL, "menu_nosleep_timed_cancel"),
        (IDC_THEME_SNOOZE_CANCEL, "menu_theme_snooze_cancel"),
        (IDC_MANAGE_BUTTON, "panel_manage_link"),
        (IDC_THEME_REPAIR, "panel_repair_link"),
        (IDC_SYSTEM_BUTTON, "menu_system_controls"),
        (IDC_SETTINGS_BUTTON, "settings_open"),
        (IDC_EXIT_BUTTON, "menu_exit_panel"),
    ] {
        let control = unsafe { GetDlgItem(Some(panel), id as i32).unwrap_or_default() };
        set_control_text(control, &t(key));
    }
    let theme_chip =
        unsafe { GetDlgItem(Some(panel), IDC_THEME_SWITCH as i32) }.unwrap_or_default();
    set_control_text(theme_chip, &theme_switch_chip_label());
    layout_header_links(panel);
    // The tray menu is rebuilt at popup time, so language changes apply on
    // the next right click without any explicit rebuild here.
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
    // The native tray icon lives on the hidden window for the whole run;
    // tray::remove takes it down after the message loop ends.
    let tray_alive = tray::add(hwnd(&HIDDEN));
    if !tray_alive {
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

    if !start_minimized || !tray_alive {
        // Atomic first presentation: no light flash in dark mode.
        FirstFrameGate::begin(hwnd(&PANEL)).reveal();
    }

    let mut msg = msg_default();
    loop {
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
        // Tray clicks and menu choices arrive as hidden-window messages now
        // and are handled inside hidden_proc; the pump only transports.
    }

    EXITING.store(true, Ordering::SeqCst);
    system::unregister_all();
    tray::remove();
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
        // Explorer restarted: re-register the tray icon.
        if tray::is_taskbar_created(msg) {
            tray::on_taskbar_created();
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
            windows::Win32::UI::WindowsAndMessaging::WM_WTSSESSION_CHANGE => {
                // WTS_SESSION_LOCK (0x7) / WTS_SESSION_UNLOCK (0x8) for this
                // session. The lock pause re-merges the effective state; the
                // lock/unlock triggers fire their rules directly.
                let locked = match wparam.0 {
                    0x7 => Some(true),
                    0x8 => Some(false),
                    _ => None,
                };
                if let Some(locked) = locked {
                    SESSION_LOCKED.store(locked, Ordering::SeqCst);
                    log_line(&format!(
                        "session {}",
                        if locked { "locked" } else { "unlocked" }
                    ));
                    automation::on_session_event(locked);
                    apply_stay_awake();
                    refresh_status();
                }
                LRESULT(0)
            }
            tray::CALLBACK_MSG => {
                tray::handle_callback(wparam, lparam);
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
                    automation::on_resume();
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
                    13 => {
                        log_line("hotkey: manual theme switch");
                        theme_engine::manual_switch();
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

/// Full (un-ellipsized) text of a panel status line, for its tooltip. The
/// drawn line truncates with an end ellipsis, so the tooltip is where the
/// complete attribution lives.
pub(crate) fn status_line_text(id: usize) -> String {
    let panel = hwnd(&PANEL);
    if panel.is_invalid() {
        return String::new();
    }
    window_text(unsafe { GetDlgItem(Some(panel), id as i32) }.unwrap_or_default())
}

/// Verbose power overview for the summary line's tooltip: the panel line
/// itself uses the compact wording to stay on one row, so the tooltip is
/// the place for the detailed statuses.
pub(crate) fn power_overview_verbose() -> String {
    let (awake, idle) = power_status();
    format!(
        "{}{}{}{}",
        t("power_overview_prefix"),
        awake,
        t("power_overview_separator"),
        idle
    )
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

/// Progress (0..=1) of a running switch animation, if any.
pub(crate) fn switch_animation_progress(control: HWND) -> Option<f32> {
    let flights = crate::runtime::lock(&SWITCH_FLIGHTS);
    let flight = flights.iter().find(|f| f.control.get() == control)?;
    let elapsed = flight.started.elapsed().as_millis() as f32;
    let t = (elapsed / SWITCH_ANIM_MS as f32).clamp(0.0, 1.0);
    // Ease-out: fast start, soft landing. Few frames with a coarse system
    // timer still read as a glide instead of steps.
    let eased = 1.0 - (1.0 - t) * (1.0 - t);
    // The paint state (state.active) is already the DESTINATION. The thumb
    // position passed to draw_switch is measured from the OFF end, so a
    // flight toward on sweeps 0->1 while a flight toward off sweeps 1->0.
    // Without the direction term a to-off flight would draw at the
    // destination and visibly snap back mid-run.
    Some(if flight.to_now { eased } else { 1.0 - eased })
}

/// Arms a thumb flight for the control (no-op when animations are off or
/// the state did not change). The panel timer advances and retires runs
/// for every window's switches.
pub(crate) fn start_switch_animation(control: HWND, from: bool, to: bool) {
    if from == to || !popups::client_area_animations() {
        return;
    }
    let mut flights = crate::runtime::lock(&SWITCH_FLIGHTS);
    flights.retain(|f| f.control.get() != control);
    flights.push(SwitchFlight {
        to_now: to,
        started: std::time::Instant::now(),
        control: SendHwnd(control.0 as usize),
    });
    unsafe {
        let _ = SetTimer(
            Some(hwnd(&PANEL)),
            SWITCH_ANIM_TIMER,
            SWITCH_ANIM_TICK as u32,
            None,
        );
    }
}

/// Arms the flights for a toggle and releases the click's refresh gate when
/// none runs (animations disabled, or the toggle never changed state), so
/// suppression cannot outlive the click handler.
fn arm_switch_flights(flights: &[(usize, bool)]) {
    for &(id, from) in flights {
        let control = unsafe { GetDlgItem(Some(hwnd(&PANEL)), id as i32) }.unwrap_or_default();
        if !control.is_invalid() {
            start_switch_animation(control, from, toggle_value(id));
        }
    }
    if crate::runtime::lock(&SWITCH_FLIGHTS).is_empty() {
        crate::tooltips::set_suppressed(false);
    }
}

/// Advances in-flight switch animations; returns true while any remain.
fn tick_switch_animations() -> bool {
    let running = {
        let flights = crate::runtime::lock(&SWITCH_FLIGHTS);
        !flights.is_empty()
    };
    if running {
        crate::tooltips::set_suppressed(true);
    }
    let mut flights = crate::runtime::lock(&SWITCH_FLIGHTS);
    flights.retain(|flight| {
        let done = flight.started.elapsed().as_millis() >= SWITCH_ANIM_MS as u128;
        if !done {
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(
                    Some(flight.control.get()),
                    None,
                    false,
                );
            }
        }
        !done
    });
    let still = !flights.is_empty();
    if !still {
        crate::tooltips::set_suppressed(false);
    }
    still
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
        } else if (IDC_CARD_BASE..IDC_CARD_BASE + 9).contains(&id) {
            // Section card face: the rounded surface that groups its rows.
            paint::draw_surface(
                dc,
                bounds,
                p.window_bg,
                p.surface,
                p.subtle_border,
                paint::control_radius(),
            );
        } else if (IDC_ROW_LABEL_BASE..IDC_ROW_LABEL_BASE + 9).contains(&id) {
            paint::draw_row_label(dc, bounds, panel_font_body(), &label, p, p.surface);
        } else if matches!(
            id,
            IDC_NOSLEEP | IDC_IDLE | IDC_AUTOMATION | IDC_THEME_ENABLE
        ) {
            // Right-aligned pill switches for label-left rows.
            // Right-aligned pill switches for label-left rows.
            let mut state = nativeform::control_state(item.control, item.state);
            state.active = toggle_value(id);
            accessibility::check(item.control, state.active);
            let progress = switch_animation_progress(item.control);
            paint::draw_switch(dc, bounds, p, p.surface, state, scale, progress);
        } else if id == IDC_EXIT_BUTTON {
            draw_exit_button(item, dc, bounds, &label, p, scale);
        } else if is_chip_id(id) {
            let mut state = nativeform::control_state(item.control, item.state);
            // The armed preset keeps an accent outline until it expires.
            state.active = chip_armed(id);
            accessibility::clear(item.control);
            paint::draw_chip(
                dc,
                bounds,
                panel_font_body(),
                &label,
                p,
                p.surface,
                state,
                paint::control_radius(),
                false,
            );
        } else if id == IDC_THEME_SWITCH {
            // Instant action sharing the snooze strip: link-colored ink.
            let state = nativeform::control_state(item.control, item.state);
            accessibility::clear(item.control);
            paint::draw_chip(
                dc,
                bounds,
                panel_font_body(),
                &label,
                p,
                p.surface,
                state,
                paint::control_radius(),
                true,
            );
        } else if is_link_id(id) {
            let state = nativeform::control_state(item.control, item.state);
            paint::draw_text_link(
                dc,
                bounds,
                panel_font_subtitle(),
                &label,
                p,
                p.window_bg,
                state,
                scale,
            );
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

/// Section group label: small secondary text, separated from neighbors by
/// whitespace alone. High contrast keeps the old accent bar and divider —
/// with no color gradation available, the extra structure is the only
/// grouping cue that survives.
unsafe fn draw_section_static(dc: HDC, bounds: &RECT, label: &str, p: &theme::Palette, scale: i32) {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Gdi as gdi;
    paint::fill_rect(dc, bounds, p.window_bg);
    let high_contrast = crate::theme_contrast::palette().is_some();
    let font = if high_contrast {
        panel_font_section()
    } else {
        panel_font_subtitle()
    };
    let color = if high_contrast { p.text } else { p.text2 };
    let mut text: Vec<u16> = label.encode_utf16().collect();
    let mut measured = *bounds;
    unsafe {
        gdi::SetTextColor(dc, COLORREF(color));
        gdi::SetBkMode(dc, gdi::TRANSPARENT);
        let old = gdi::SelectObject(dc, gdi::HGDIOBJ(font.0));
        let _ = gdi::DrawTextW(
            dc,
            &mut text,
            &mut measured,
            gdi::DT_LEFT | gdi::DT_VCENTER | gdi::DT_SINGLELINE | gdi::DT_CALCRECT,
        );
        let mut text_bounds = *bounds;
        if high_contrast {
            // The measured title leaves room for the divider that follows.
            text_bounds.left += paint::sp(10, scale);
        }
        let _ = gdi::DrawTextW(
            dc,
            &mut text,
            &mut text_bounds,
            gdi::DT_LEFT | gdi::DT_VCENTER | gdi::DT_SINGLELINE,
        );
        gdi::SelectObject(dc, old);
    }
    if high_contrast {
        let accent_height = paint::sp(16, scale);
        let accent = RECT {
            left: bounds.left,
            top: bounds.top + (bounds.bottom - bounds.top - accent_height) / 2,
            right: bounds.left + paint::sp(3, scale),
            bottom: bounds.top + (bounds.bottom - bounds.top + accent_height) / 2,
        };
        paint::fill_rect(dc, &accent, p.accent);
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
}

/// Muted status line under a row (Go drawStatic subtitle). One line with an
/// end ellipsis — long attributions never wrap into a half-clipped second
/// line; the full text rides the line's tooltip.
unsafe fn draw_subtitle_static(dc: HDC, bounds: &RECT, label: &str, p: &theme::Palette) {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Gdi as gdi;
    paint::fill_rect(dc, bounds, p.surface);
    if label.is_empty() {
        return;
    }
    unsafe {
        let mut text: Vec<u16> = label.encode_utf16().collect();
        let mut text_bounds = *bounds;
        gdi::SetTextColor(dc, COLORREF(p.muted));
        gdi::SetBkMode(dc, gdi::TRANSPARENT);
        let old = gdi::SelectObject(dc, gdi::HGDIOBJ(panel_font_status().0));
        let _ = gdi::DrawTextW(
            dc,
            &mut text,
            &mut text_bounds,
            gdi::DT_LEFT
                | gdi::DT_VCENTER
                | gdi::DT_SINGLELINE
                | gdi::DT_NOPREFIX
                | gdi::DT_END_ELLIPSIS,
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
        paint::draw_focus_frame(
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
                let _ = KillTimer(Some(hwnd_), SWITCH_ANIM_TIMER);
                crate::runtime::lock(&SWITCH_FLIGHTS).clear();
                // A flight cut by the close must not leave text refreshes
                // suppressed for the rest of the panel's life.
                crate::tooltips::set_suppressed(false);
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_SHOWWINDOW if wparam.0 == 0 => {
                // Every hide path (close, Esc, tray toggle) lands here; a
                // tracked focus tip would otherwise stay afloat on the
                // desktop above a hidden panel.
                crate::tooltips::hide_focus_tip();
                DefWindowProcW(hwnd_, msg, wparam, lparam)
            }
            WM_FOCUSTIP_DISMISS => {
                // Focus-tip dismissals must run in a plain dispatch: sent
                // from inside ShowWindow/DestroyWindow handling they poison
                // the tooltip control and no tip ever hides again.
                let control = HWND(lparam.0 as *mut _);
                if crate::tooltips::focus_tip_active(control) {
                    crate::tooltips::set_focus_tip(control, false);
                }
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_NCDESTROY => {
                crate::tooltips::on_panel_destroyed();
                DefWindowProcW(hwnd_, msg, wparam, lparam)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE => {
                // Blank-area moves are mouse input too: keyboard focus tips
                // yield to the pointer wherever it lands.
                crate::tooltips::yield_focus_tip(hwnd_);
                DefWindowProcW(hwnd_, msg, wparam, lparam)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_KEYDOWN
                if wparam.0 as u16 == windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0 =>
            {
                let _ = ShowWindow(hwnd_, SW_HIDE);
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_SETCURSOR => {
                // Link-styled controls advertise clickability with the hand
                // cursor; children forward WM_SETCURSOR here.
                let child = HWND(wparam.0 as *mut core::ffi::c_void);
                let id = GetDlgCtrlID(child);
                if id != 0
                    && is_link_id(id.unsigned_abs() as usize)
                    && let Ok(hand) = LoadCursorW(None, IDC_HAND)
                {
                    let _ = SetCursor(Some(hand));
                    LRESULT(1)
                } else {
                    DefWindowProcW(hwnd_, msg, wparam, lparam)
                }
            }
            WM_COMMAND => {
                let code = wparam.0 & 0xFFFF;
                if matches!(code, IDC_NOSLEEP | IDC_IDLE | IDC_AUTOMATION) {
                    let before = toggle_value(code);
                    let twin = match code {
                        IDC_NOSLEEP => Some(IDC_IDLE),
                        IDC_IDLE => Some(IDC_NOSLEEP),
                        _ => None,
                    };
                    let twin_before = twin.map(toggle_value);
                    // Gate before the toggle: on_toggle refreshes
                    // synchronously and would push the new state line into
                    // a tooltip that is still on screen under the cursor.
                    crate::tooltips::set_suppressed(true);
                    on_toggle(code);
                    let mut flights = vec![(code, before)];
                    if let (Some(twin_id), Some(before)) = (twin, twin_before) {
                        flights.push((twin_id, before));
                    }
                    arm_switch_flights(&flights);
                    // Owner-drawn toggles need a repaint after the state
                    // flip; the mutual-exclusion twin flips too.
                    invalidate_control(code);
                    if let Some(twin_id) = twin {
                        invalidate_control(twin_id);
                    }
                } else if matches!(
                    code,
                    IDC_NOSLEEP_TIMED_30M | IDC_NOSLEEP_TIMED_1H | IDC_NOSLEEP_TIMED_2H
                ) {
                    // Clicking the armed preset again cancels the overlay —
                    // the same affordance the header cancel link offers.
                    if TIMED_ARMED_CHIP.load(Ordering::SeqCst) == code {
                        clear_timed_nosleep();
                    } else {
                        let seconds = match code {
                            IDC_NOSLEEP_TIMED_30M => 30 * 60,
                            IDC_NOSLEEP_TIMED_1H => 60 * 60,
                            _ => 2 * 60 * 60,
                        };
                        set_timed_nosleep(seconds, false);
                        // Repaint both ends of the swap: the newly armed chip
                        // gains the outline, the previous one must lose it.
                        let previous = TIMED_ARMED_CHIP.swap(code, Ordering::SeqCst);
                        if previous != code {
                            invalidate_control(previous);
                        }
                        invalidate_control(code);
                    }
                } else if code == IDC_NOSLEEP_TIMED_CANCEL {
                    clear_timed_nosleep();
                } else if code == IDC_MANAGE_BUTTON {
                    automation_ui::show();
                } else if code == IDC_SETTINGS_BUTTON {
                    settings_ui::show();
                } else if code == IDC_THEME_ENABLE {
                    let before = toggle_value(code);
                    crate::tooltips::set_suppressed(true);
                    theme_engine::toggle_enabled();
                    arm_switch_flights(&[(code, before)]);
                    refresh_checkboxes();
                    invalidate_control(IDC_THEME_ENABLE);
                } else if code == IDC_THEME_SWITCH {
                    theme_engine::manual_switch();
                } else if code == IDC_THEME_REPAIR {
                    theme_engine::repair();
                    // Surface the "repairing…" state right away: the DWM
                    // round plus its broadcasts take seconds to finish.
                    refresh_status();
                } else if matches!(
                    code,
                    IDC_THEME_SNOOZE_30M | IDC_THEME_SNOOZE_1H | IDC_THEME_SNOOZE_MORNING
                ) {
                    // Clicking the armed snooze again cancels it, mirroring
                    // the header cancel link.
                    if SNOOZE_ARMED_CHIP.load(Ordering::SeqCst) == code {
                        theme_engine::snooze_cancel();
                    } else {
                        match code {
                            IDC_THEME_SNOOZE_30M => theme_engine::snooze(30),
                            IDC_THEME_SNOOZE_1H => theme_engine::snooze(60),
                            _ => theme_engine::snooze_until_morning(),
                        }
                        let previous = SNOOZE_ARMED_CHIP.swap(code, Ordering::SeqCst);
                        if previous != code {
                            invalidate_control(previous);
                        }
                        invalidate_control(code);
                    }
                    refresh_status();
                } else if code == IDC_THEME_SNOOZE_CANCEL {
                    theme_engine::snooze_cancel();
                    refresh_status();
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
            windows::Win32::UI::WindowsAndMessaging::WM_NCHITTEST => {
                // Blank client areas drag the window (HTCAPTION gives the
                // system's move + snap for free). Latched probe (see
                // nativeform::blank_drag_hit): statics and switch-row
                // blanks fall through and drag, interactive controls keep
                // native hit-testing, and the internal WindowFromPoint
                // probe cannot recurse into this handler.
                crate::nativeform::blank_drag_hit(hwnd_, msg, wparam, lparam)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_NCRBUTTONUP => {
                // The panel carries WS_SYSMENU, so a right-click on a blank
                // drag area would open the system menu — blanks never had a
                // right-click action before and must not grow one now.
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == PANEL_TIMER => {
                tooltips::reconcile_focus_tip();
                refresh_status();
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == SWITCH_ANIM_TIMER => {
                // A flight paints each tick through WM_DRAWITEM; when the
                // last one lands, kill the timer.
                if !tick_switch_animations() {
                    let _ = KillTimer(Some(hwnd_), SWITCH_ANIM_TIMER);
                }
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
    // Test/devtools callers build the UI without main()'s startup: give
    // config commits a real temp path so save paths succeed silently
    // instead of popping a blocking "configuration path unavailable"
    // dialog (which stalled test children until someone clicked OK).
    if crate::runtime::lock(&CONFIG_PATH).as_ref().is_none() {
        let path = std::env::temp_dir().join(format!("idletrigger-ui-{}.toml", std::process::id()));
        *crate::runtime::lock(&CONFIG_PATH) = Some(path);
    }
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
            // Session notifications feed the lock pause and the lock/unlock
            // triggers. The session starts unlocked (auto-start runs at
            // logon); events keep the flag current from here on.
            if windows::Win32::System::RemoteDesktop::WTSRegisterSessionNotification(
                hidden,
                windows::Win32::System::RemoteDesktop::NOTIFY_FOR_THIS_SESSION,
            )
            .is_err()
            {
                // Without the subscription the lock pause and the lock/unlock
                // triggers silently never fire; leave a trace for support.
                log_line("WTSRegisterSessionNotification failed; lock triggers disabled");
            }
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

        // Vertical flow: one rounded card per feature section (Win11
        // settings grouping) with its title OUTSIDE the card — the title's
        // left edge and the links' right edge ride the card's border axes.
        // The card static is created before its rows so the rows stay above
        // it; statics are click-transparent either way.
        let card_w = PANEL_CLIENT_WIDTH - 2 * PAD; // 462
        let row_x = PAD + CARD_PAD_X;
        let row_width = card_w - 2 * CARD_PAD_X; // 438
        let mut y = PAD;
        let card_height = |rows: i32| 2 * CARD_PAD_Y + rows;
        let make_card = |id: usize, top: i32, height: i32| {
            owner_static(
                &ControlSpec {
                    parent: panel,
                    label: String::new(),
                    x: scale(PAD),
                    y: scale(top),
                    width: scale(card_w),
                    id,
                    instance,
                    font,
                },
                scale(height),
            )
        };

        // ---- 电源管理 ----
        section_header(
            panel,
            &t("menu_power_management"),
            y,
            STATIC_SECTION_BASE,
            instance,
            subtitle_font,
        );
        HEADER_LINK_ROWS[0].store(scale(y) as isize, Ordering::SeqCst);
        // Section actions share the outside title line, right-aligned to
        // the card's right border (the standard "header + action" pattern);
        // the timed cancel link occupies a fixed slot that stays hidden
        // until something is armed.
        let timed_cancel = owner_button(
            &ControlSpec {
                parent: panel,
                label: t("menu_nosleep_timed_cancel"),
                x: 0,
                y: 0,
                width: scale(4),
                id: IDC_NOSLEEP_TIMED_CANCEL,
                instance,
                font: subtitle_font,
            },
            scale(LINK_BOX_H),
        );
        BTN_NOSLEEP_TIMED_CANCEL.store(timed_cancel.0 as isize, Ordering::SeqCst);
        if timed_nosleep_state().is_none() {
            let _ = ShowWindow(timed_cancel, SW_HIDE);
        }
        y += SECTION_H + TITLE_GAP;
        make_card(
            IDC_CARD_BASE,
            y,
            card_height(BUTTON_H + LABEL_GAP + CHIP_H + LABEL_GAP + SUBTITLE_H),
        );
        let mut row = y + CARD_PAD_Y;
        // Two label+switch groups share the row: Stay Awake and Idle
        // Monitoring are mutually exclusive in the saved config (on_toggle
        // forces the other off), so enabling one visibly flips the other
        // switch off — the same live-feedback pattern the status line
        // explains.
        let half_w = (row_width - GAP) / 2; // 215
        let label_w = half_w - SWITCH_HIT_W - GAP; // 147
        for (index, (label_id, switch_id, key)) in [
            (IDC_ROW_LABEL_BASE + 2, IDC_NOSLEEP, "power_nosleep"),
            (IDC_ROW_LABEL_BASE + 3, IDC_IDLE, "power_idle"),
        ]
        .into_iter()
        .enumerate()
        {
            let group_x = row_x + index as i32 * (half_w + GAP);
            owner_static(
                &ControlSpec {
                    parent: panel,
                    label: t(key),
                    x: scale(group_x),
                    y: scale(row),
                    width: scale(label_w),
                    id: label_id,
                    instance,
                    font,
                },
                scale(BUTTON_H),
            );
            // The switch's window text is its accessible name.
            let switch = owner_button(
                &ControlSpec {
                    parent: panel,
                    label: t(key),
                    x: scale(group_x + label_w + GAP),
                    y: scale(row),
                    width: scale(SWITCH_HIT_W),
                    id: switch_id,
                    instance,
                    font,
                },
                scale(BUTTON_H),
            );
            match switch_id {
                IDC_NOSLEEP => CHK_NOSLEEP.store(switch.0 as isize, Ordering::SeqCst),
                _ => CHK_IDLE.store(switch.0 as isize, Ordering::SeqCst),
            }
        }
        row += BUTTON_H + LABEL_GAP;
        // Timed stay-awake preset chips: one click arms the runtime overlay
        // (set_timed_nosleep) without touching the saved switches. The strip
        // splits the full row so its right edge aligns with the segments
        // above.
        for (index, (id, key)) in [
            (IDC_NOSLEEP_TIMED_30M, "chip_timed_30m"),
            (IDC_NOSLEEP_TIMED_1H, "chip_timed_1h"),
            (IDC_NOSLEEP_TIMED_2H, "chip_timed_2h"),
        ]
        .into_iter()
        .enumerate()
        {
            let (slot_x, slot_w) = row_slot(row_width, 3, index as i32);
            owner_button(
                &ControlSpec {
                    parent: panel,
                    label: t(key),
                    x: scale(row_x + slot_x),
                    y: scale(row),
                    width: scale(slot_w),
                    id,
                    instance,
                    font,
                },
                scale(CHIP_H),
            );
        }
        row += CHIP_H + LABEL_GAP;
        let power_summary = owner_static(
            &ControlSpec {
                parent: panel,
                label: String::new(),
                x: scale(row_x),
                y: scale(row),
                width: scale(row_width),
                id: IDC_POWER_SUMMARY,
                instance,
                font: panel_font_status(),
            },
            scale(SUBTITLE_H),
        );
        LBL_POWER_SUMMARY.store(power_summary.0 as isize, Ordering::SeqCst);
        y += card_height(BUTTON_H + LABEL_GAP + CHIP_H + LABEL_GAP + SUBTITLE_H) + CARD_GAP;

        // ---- 自动任务 ----
        section_header(
            panel,
            &t("menu_automation_section"),
            y,
            STATIC_SECTION_BASE + 1,
            instance,
            subtitle_font,
        );
        HEADER_LINK_ROWS[1].store(scale(y) as isize, Ordering::SeqCst);
        owner_button(
            &ControlSpec {
                parent: panel,
                label: t("panel_manage_link"),
                x: 0,
                y: 0,
                width: scale(4),
                id: IDC_MANAGE_BUTTON,
                instance,
                font: subtitle_font,
            },
            scale(LINK_BOX_H),
        );
        y += SECTION_H + TITLE_GAP;
        make_card(
            IDC_CARD_BASE + 1,
            y,
            card_height(BUTTON_H + LABEL_GAP + SUBTITLE_H),
        );
        let mut row = y + CARD_PAD_Y;
        // Label-left row with a right-aligned pill switch.
        owner_static(
            &ControlSpec {
                parent: panel,
                label: t("automation_master"),
                x: scale(row_x),
                y: scale(row),
                width: scale(row_width - SWITCH_HIT_W - GAP),
                id: IDC_ROW_LABEL_BASE,
                instance,
                font,
            },
            scale(BUTTON_H),
        );
        // The switch's window text is its accessible name; draw_switch
        // paints geometry only, so the label costs nothing visually.
        let chk_automation = owner_button(
            &ControlSpec {
                parent: panel,
                label: t("automation_master"),
                x: scale(row_x + row_width - SWITCH_HIT_W),
                y: scale(row),
                width: scale(SWITCH_HIT_W),
                id: IDC_AUTOMATION,
                instance,
                font,
            },
            scale(BUTTON_H),
        );
        CHK_AUTOMATION.store(chk_automation.0 as isize, Ordering::SeqCst);
        row += BUTTON_H + LABEL_GAP;
        let automation_summary = owner_static(
            &ControlSpec {
                parent: panel,
                label: String::new(),
                x: scale(row_x),
                y: scale(row),
                width: scale(row_width),
                id: IDC_AUTOMATION_SUMMARY,
                instance,
                font: panel_font_status(),
            },
            scale(SUBTITLE_H),
        );
        LBL_AUTOMATION_SUMMARY.store(automation_summary.0 as isize, Ordering::SeqCst);
        y += card_height(BUTTON_H + LABEL_GAP + SUBTITLE_H) + CARD_GAP;

        // ---- 昼夜模式 ----
        section_header(
            panel,
            &t("menu_theme_switch"),
            y,
            STATIC_SECTION_BASE + 2,
            instance,
            subtitle_font,
        );
        HEADER_LINK_ROWS[2].store(scale(y) as isize, Ordering::SeqCst);
        let snooze_cancel = owner_button(
            &ControlSpec {
                parent: panel,
                label: t("menu_theme_snooze_cancel"),
                x: 0,
                y: 0,
                width: scale(4),
                id: IDC_THEME_SNOOZE_CANCEL,
                instance,
                font: subtitle_font,
            },
            scale(LINK_BOX_H),
        );
        BTN_THEME_SNOOZE_CANCEL.store(snooze_cancel.0 as isize, Ordering::SeqCst);
        if theme_engine::snooze_deadline().is_none() {
            let _ = ShowWindow(snooze_cancel, SW_HIDE);
        }
        owner_button(
            &ControlSpec {
                parent: panel,
                label: t("panel_repair_link"),
                x: 0,
                y: 0,
                width: scale(4),
                id: IDC_THEME_REPAIR,
                instance,
                font: subtitle_font,
            },
            scale(LINK_BOX_H),
        );
        y += SECTION_H + TITLE_GAP;
        make_card(
            IDC_CARD_BASE + 2,
            y,
            card_height(BUTTON_H + LABEL_GAP + CHIP_H + LABEL_GAP + SUBTITLE_H),
        );
        let mut row = y + CARD_PAD_Y;
        // Label-left row with a right-aligned pill switch.
        owner_static(
            &ControlSpec {
                parent: panel,
                label: t("menu_theme_enable"),
                x: scale(row_x),
                y: scale(row),
                width: scale(row_width - SWITCH_HIT_W - GAP),
                id: IDC_ROW_LABEL_BASE + 1,
                instance,
                font,
            },
            scale(BUTTON_H),
        );
        owner_button(
            &ControlSpec {
                parent: panel,
                label: t("menu_theme_enable"),
                x: scale(row_x + row_width - SWITCH_HIT_W),
                y: scale(row),
                width: scale(SWITCH_HIT_W),
                id: IDC_THEME_ENABLE,
                instance,
                font,
            },
            scale(BUTTON_H),
        );
        row += BUTTON_H + LABEL_GAP;
        // The instant switch-now action shares the snooze strip; its
        // link-colored ink sets "acts immediately" apart from the deferred
        // snoozes beside it.
        for (index, (id, label)) in [
            (IDC_THEME_SWITCH, theme_switch_chip_label()),
            (IDC_THEME_SNOOZE_30M, t("chip_snooze_30m")),
            (IDC_THEME_SNOOZE_1H, t("chip_snooze_1h")),
            (IDC_THEME_SNOOZE_MORNING, t("chip_snooze_morning")),
        ]
        .into_iter()
        .enumerate()
        {
            let (slot_x, slot_w) = row_slot(row_width, 4, index as i32);
            owner_button(
                &ControlSpec {
                    parent: panel,
                    label,
                    x: scale(row_x + slot_x),
                    y: scale(row),
                    width: scale(slot_w),
                    id,
                    instance,
                    font,
                },
                scale(CHIP_H),
            );
        }
        row += CHIP_H + LABEL_GAP;
        let theme_schedule = owner_static(
            &ControlSpec {
                parent: panel,
                label: String::new(),
                x: scale(row_x),
                y: scale(row),
                width: scale(row_width),
                id: IDC_THEME_SCHEDULE,
                instance,
                font: panel_font_status(),
            },
            scale(SUBTITLE_H),
        );
        LBL_THEME_SCHEDULE.store(theme_schedule.0 as isize, Ordering::SeqCst);
        y += card_height(BUTTON_H + LABEL_GAP + CHIP_H + LABEL_GAP + SUBTITLE_H) + CARD_GAP;

        // ---- Footer navigation (outside the cards) ----

        let (footer_x, footer_w) = row_slot(card_w, 3, 0);
        let system_btn = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            scale(PAD + footer_x),
            scale(y),
            scale(footer_w),
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

        let (footer_x, footer_w) = row_slot(card_w, 3, 1);
        let settings_btn = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            scale(PAD + footer_x),
            scale(y),
            scale(footer_w),
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
        let (footer_x, footer_w) = row_slot(card_w, 3, 2);
        let exit_btn = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            scale(PAD + footer_x),
            scale(y),
            scale(footer_w),
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

        // Header links get their exact-fit geometry from the labels.
        layout_header_links(panel);

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

/// Logical pixel width of a single-line label at the given font (Go
/// GetTextExtent-based measuring).
fn measured_text_width(parent: HWND, font: HFONT, label: &str) -> i32 {
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
        (bounds.right - bounds.left) * 96 / dpi
    }
}

/// Physical y of each section's outside title row, captured at creation so
/// the header links can be re-fitted after language swaps move their text.
static HEADER_LINK_ROWS: [AtomicIsize; 3] = [
    AtomicIsize::new(0),
    AtomicIsize::new(0),
    AtomicIsize::new(0),
];

/// Header action links wrap their text exactly (hit box == display box) and
/// center on the switch column of the card below; the theme's snooze cancel
/// parks in a fixed slot left of the repair link.
fn layout_header_links(panel: HWND) {
    let card_w = PANEL_CLIENT_WIDTH - 2 * PAD;
    let center = PAD + card_w - CARD_PAD_X - SWITCH_HIT_W / 2;
    let place = |id: usize, section: usize, left_of: Option<usize>| {
        let Ok(link) = (unsafe { GetDlgItem(Some(panel), id as i32) }) else {
            return;
        };
        let text = window_text(link);
        let width = scale(measured_text_width(panel, panel_font_subtitle(), &text).max(16));
        let x = match left_of {
            Some(other) => {
                let Ok(anchor) = (unsafe { GetDlgItem(Some(panel), other as i32) }) else {
                    return;
                };
                let mut rect = RECT::default();
                let _ = unsafe { GetWindowRect(anchor, &mut rect) };
                let mut pt = windows::Win32::Foundation::POINT {
                    x: rect.left,
                    y: rect.top,
                };
                let _ = unsafe { windows::Win32::Graphics::Gdi::ScreenToClient(panel, &mut pt) };
                pt.x - scale(GAP) - width
            }
            None => scale(center) - width / 2,
        };
        let row = HEADER_LINK_ROWS[section].load(Ordering::SeqCst) as i32;
        let y = row + (scale(SECTION_H) - scale(LINK_BOX_H)) / 2;
        let _ = unsafe {
            SetWindowPos(
                link,
                None,
                x,
                y,
                width,
                scale(LINK_BOX_H),
                SWP_NOZORDER | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
            )
        };
    };
    place(IDC_NOSLEEP_TIMED_CANCEL, 0, None);
    place(IDC_MANAGE_BUTTON, 1, None);
    place(IDC_THEME_REPAIR, 2, None);
    place(IDC_THEME_SNOOZE_CANCEL, 2, Some(IDC_THEME_REPAIR));
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
/// from paint::draw_button / paint::draw_switch_row in the parent's WM_DRAWITEM.
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

/// Updates the tray tooltip with a status line capped like Go's 120 UTF-16
/// units. Only call on the UI thread. Skips the `Shell_NotifyIconW` round
/// trip when the truncated text is unchanged: this runs every second from
/// `refresh_status`.
fn tray_update_tooltip(line: &str) {
    static LAST: Mutex<Option<String>> = Mutex::new(None);
    // Graceful degradation before the hard cut: dropping the version suffix
    // from the first line usually frees room for the last status line.
    let mut text = line.to_string();
    if text.encode_utf16().count() > 118
        && let Some((first, rest)) = text.split_once('\n')
        && let Some((_, bare_name)) = first.split_once(" v")
    {
        let candidate = format!("{bare_name}{rest}");
        if candidate.encode_utf16().count() <= 118 {
            text = candidate;
        }
    }
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    if wide.len() > 118 {
        wide.truncate(118);
        wide.extend_from_slice("…".encode_utf16().collect::<Vec<u16>>().as_slice());
    }
    let text = String::from_utf16_lossy(&wide);
    let mut last = runtime::lock(&LAST);
    if last.as_deref() == Some(text.as_str()) {
        return;
    }
    // Cache only after the modify succeeds, so a failed round trip
    // retries on the next refresh.
    if tray::set_tooltip(&text) {
        *last = Some(text);
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

    // Stay awake: effective state, paused wording under battery block. When
    // the saved switch is off and only the timed overlay is awake, the line
    // says so directly.
    let manual_on = cfg_map(|c| c.nosleep_enabled);
    let task_awake = !automation::overrides().stay_awake_sources.is_empty();
    let timed = timed_nosleep_state();
    let stay_awake = if effective_nosleep {
        if !manual_on
            && !task_awake
            && let Some((remaining, _)) = timed
        {
            t_args("status_short_timed", &[&format_remaining(remaining)])
        } else {
            t("status_short_on")
        }
    } else if power.requested && !power.awake {
        t("status_paused")
    } else {
        t("status_short_off")
    };
    lines.push(line("tooltip_nosleep", stay_awake));

    // Timed override: a distinct line only when the first line doesn't
    // already carry it (manual switch or a task is the primary source).
    if effective_nosleep
        && (manual_on || task_awake)
        && let Some((remaining, _)) = timed
    {
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
            format!("{} {unit} {action_label}", power.idle_minutes),
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
    // A toggle that turned the Stay Awake switch off (directly or via the
    // idle mutual exclusion) drops the timed overlay inside commit_config.
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
    let nosleep_was_on = lock(&CONFIG).as_ref().is_some_and(|c| c.nosleep_enabled);
    *lock(&CONFIG) = Some(loaded.config.clone());
    *lock(&CONFIG_DOC) = Some(loaded.document);
    *lock(&CONFIG_SOURCE) = loaded.source_text;
    CONFIG_LOAD_FAILED.store(false, Ordering::SeqCst);
    automation::reload_rules();
    drop(writer);
    // Same rule as commit_config: an external edit that turned the Stay
    // Awake switch off also drops the timed overlay; an edit that leaves
    // the switch untouched does not.
    if nosleep_was_on && !loaded.config.nosleep_enabled {
        sync_timed_with_manual();
    }
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
    /// The workstation is locked and the lock pause is enabled.
    pub lock_paused: bool,
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
    let lock_paused = config.nosleep_pause_on_lock && SESSION_LOCKED.load(Ordering::SeqCst);
    EffectivePowerState {
        requested,
        paused: overrides.pause_stay_awake,
        lock_paused,
        battery_allowed,
        awake: requested && !overrides.pause_stay_awake && !lock_paused && battery_allowed,
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

/// Verbose statuses for tooltips and narration.
pub(crate) fn power_status() -> (String, String) {
    power_status_impl(false)
}

/// Compact statuses for the panel's one-line overview: short on/off words
/// and tight parentheticals keep even attributed states on a single line.
fn power_status_compact() -> (String, String) {
    power_status_impl(true)
}

fn power_status_impl(compact: bool) -> (String, String) {
    let power = effective_power_state();
    let idle_action = cfg_map(|c| c.idle_action.clone());
    // Reason attribution: which task and/or the timed overlay is keeping the
    // machine awake. Shown only while actually awake.
    let mut reasons: Vec<String> = Vec::new();
    let sources = automation::overrides().stay_awake_sources;
    if !sources.is_empty() {
        let separator = if i18n_is_chinese() { "、" } else { ", " };
        let key = if compact {
            "panel_status_reason_task"
        } else {
            "status_reason_task"
        };
        reasons.push(t_args(key, &[&sources.join(separator)]));
    }
    if let Some((remaining, _)) = timed_nosleep_state() {
        let key = if compact {
            "panel_status_reason_timed"
        } else {
            "status_reason_timed"
        };
        reasons.push(t_args(key, &[&format_remaining(remaining)]));
    }
    let manual_on = cfg_map(|c| c.nosleep_enabled);
    // Reasons read as a natural list, not a formula.
    let reason_sep = if i18n_is_chinese() { "、" } else { ", " };
    let pick =
        |compact_key: &str, verbose_key: &str| t(if compact { compact_key } else { verbose_key });
    // When the saved switch is off, the machine is kept awake purely by task
    // rules and/or the timed overlay — say so directly instead of claiming
    // the switch is "enabled".
    let mut awake_status = if !power.requested {
        pick("status_short_off", "status_disabled")
    } else if power.paused {
        pick(
            "panel_status_paused_automation",
            "status_paused_by_automation",
        )
    } else if power.lock_paused {
        pick("panel_status_paused_lock", "status_paused_by_lock")
    } else if !power.battery_allowed {
        pick("panel_status_paused_battery", "status_paused_by_battery")
    } else if !manual_on && !reasons.is_empty() {
        let key = if compact {
            "panel_status_awake_overrides"
        } else {
            "status_awake_overrides"
        };
        t_args(key, &[&reasons.join(reason_sep)])
    } else if power.keep_screen {
        pick("panel_status_keep_screen", "status_enabled_keep_screen")
    } else {
        pick("status_short_on", "status_enabled")
    };
    if power.awake && manual_on && !reasons.is_empty() {
        awake_status.push_str(&t_args(
            "status_reason_suffix",
            &[&reasons.join(reason_sep)],
        ));
    }
    let idle_status = if !power.idle_requested {
        pick("status_short_off", "status_disabled")
    } else if power.awake {
        pick("panel_status_paused_nosleep", "status_paused_by_nosleep")
    } else if power.idle_paused {
        pick(
            "panel_status_paused_automation",
            "status_paused_by_automation",
        )
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
    // Cancel chips are only visible while their row has something armed. An
    // expired preset must also lose its accent outline: the chip never
    // repaints on its own, so invalidate it when the marker is cleared.
    let timed_armed = timed_nosleep_state().is_some();
    let snooze_armed = theme_engine::snooze_deadline().is_some();
    let previous_timed = TIMED_ARMED_CHIP.load(Ordering::SeqCst);
    if previous_timed != 0 && !timed_armed {
        TIMED_ARMED_CHIP.store(0, Ordering::SeqCst);
        invalidate_control(previous_timed);
    }
    let previous_snooze = SNOOZE_ARMED_CHIP.load(Ordering::SeqCst);
    if previous_snooze != 0 && !snooze_armed {
        SNOOZE_ARMED_CHIP.store(0, Ordering::SeqCst);
        invalidate_control(previous_snooze);
    }
    unsafe {
        let show = if timed_armed { SW_SHOW } else { SW_HIDE };
        let _ = ShowWindow(hwnd(&BTN_NOSLEEP_TIMED_CANCEL), show);
        let show = if snooze_armed { SW_SHOW } else { SW_HIDE };
        let _ = ShowWindow(hwnd(&BTN_THEME_SNOOZE_CANCEL), show);
    }
    let (nosleep_status, idle_status) = power_status_compact();
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
    // The switch-now chip names the side it will switch to; it flips after
    // every manual switch or system theme change.
    let panel = hwnd(&PANEL);
    if !panel.is_invalid() {
        let chip = unsafe { GetDlgItem(Some(panel), IDC_THEME_SWITCH as i32) }.unwrap_or_default();
        let label = theme_switch_chip_label();
        if !chip.is_invalid() && window_text(chip) != label {
            set_control_text(chip, &label);
        }
    }

    // Tray tooltip mirrors the effective power-management state. The tray
    // line reads the latched execution flag (apply_stay_awake owns it), the
    // paused wording reads the recomputed derivation.
    let power = effective_power_state();
    let effective_nosleep = NOSLEEP_EXECUTION_ON.load(Ordering::SeqCst);
    let idle_running = power.idle_requested && !power.idle_paused && !effective_nosleep;
    tray_update_tooltip(&build_tray_tooltip(effective_nosleep, idle_running, &power));
    tray::refresh_theme_icon();
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
/// showSource = true). While a snooze is armed it replaces the whole line so
/// the postponed state is unmistakable; otherwise the schedule shows with
/// its location source.
fn theme_schedule_text() -> String {
    // A running repair owns the line: the operation takes seconds (theme
    // apply plus two broadcast rounds), so an instantly visible state beats
    // a short-lived completion notice.
    if theme_engine::repair_running() {
        return t("theme_repair_started");
    }
    if let Some(notice) = theme_engine::active_notice() {
        return notice;
    }
    match theme_engine::snooze_deadline_text() {
        Some(until) => t_args("theme_schedule_snoozed_line", &[&until]),
        None => theme_schedule_summary(false),
    }
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

/// "Off" always means off: called when a save or an external reload turned
/// the Stay Awake switch off (commit_config and hot_reload_config catch
/// every such transition, including the monitor's mutual exclusion) and
/// after explicit nosleep-off requests. Monitor and automation edits never
/// call this — the timed overlay is independent of them and expires on its
/// own.
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
            // Battery-threshold triggers edge-detect on the polled percent;
            // on AC the battery percent is meaningless, so skip dispatch.
            if status.ACLineStatus != 0 {
                automation::on_battery(100);
            } else {
                automation::on_battery(percent);
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
        "lock" | "sleep" | "hibernate" | "shutdown" | "restart" | "screen_off" | "logoff"
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
            "screen_off" => {
                // Broadcast-only: SC_MONITORPOWER(2) powers the display(s)
                // down. Any input wakes them again; a hung window must not
                // stall the request, hence the timeout variant. The call has
                // no useful failure signal, so it always reports success.
                SendMessageTimeoutW(
                    HWND_BROADCAST,
                    WM_SYSCOMMAND,
                    WPARAM(SC_MONITORPOWER as usize),
                    LPARAM(2),
                    SMTO_ABORTIFHUNG,
                    1000,
                    None,
                );
                Ok(())
            }
            "logoff" => {
                // Logging off ends this session (and this process); no
                // shutdown privilege is required for it.
                ExitWindowsEx(EWX_LOGOFF, Default::default()).map_err(|error| error.to_string())
            }
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
/// Devtools helper: opens the system quick-actions popup for capture.
#[cfg(feature = "devtools")]
pub fn devtools_open_quick_menu() {
    unsafe {
        show_system_controls_menu(hwnd(&PANEL));
    }
}

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
    assert!(tray::add(hwnd(&HIDDEN)), "test tray");
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
    tray::remove();
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

    #[test]
    fn stay_awake_off_transitions_drop_the_timed_overlay_by_any_path() {
        // The overlay is cancelled by expiry, an explicit cancel, or the
        // saved switch turning off — through a panel/IPC save or an external
        // reload. A save or reload that does not touch the switch leaves it
        // armed even while the switch is already off.
        let _test = crate::runtime::lock(&CONFIG_TEST_LOCK);
        let dir = std::env::temp_dir().join(format!(
            "idletrigger-timed-overlay-{}-{}",
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
        let previous_timed = crate::runtime::lock(&NOSLEEP_TIMED).take();
        let arm = || {
            *crate::runtime::lock(&NOSLEEP_TIMED) = Some(TimedNosleep {
                until: std::time::Instant::now() + std::time::Duration::from_secs(60),
                keep_screen: false,
            });
        };

        // Unrelated save while the switch is already off: the overlay survives.
        arm();
        edit_config(|c| c.automation_enabled = !c.automation_enabled).unwrap();
        assert!(timed_nosleep_state().is_some());

        // Turning the switch on never drops; turning it directly off does.
        edit_config(|c| c.nosleep_enabled = true).unwrap();
        assert!(timed_nosleep_state().is_some());
        edit_config(|c| c.nosleep_enabled = false).unwrap();
        assert!(timed_nosleep_state().is_none());

        // The monitor's mutual exclusion (idle on forces the switch off) drops.
        edit_config(|c| c.nosleep_enabled = true).unwrap();
        arm();
        edit_config(|c| {
            c.idle_enabled = true;
            c.nosleep_enabled = false;
        })
        .unwrap();
        assert!(timed_nosleep_state().is_none());

        // An external edit turning the switch off drops; an external edit
        // that leaves the already-off switch alone does not.
        edit_config(|c| {
            c.nosleep_enabled = true;
            c.idle_enabled = false;
        })
        .unwrap();
        arm();
        let external = std::fs::read_to_string(&path)
            .unwrap()
            .replace("nosleep_enabled = true", "nosleep_enabled = false");
        std::fs::write(&path, external).unwrap();
        hot_reload_config().unwrap();
        assert!(timed_nosleep_state().is_none());
        arm();
        let touched = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{touched}# touch\n")).unwrap();
        hot_reload_config().unwrap();
        assert!(timed_nosleep_state().is_some());

        *crate::runtime::lock(&NOSLEEP_TIMED) = previous_timed;
        *crate::runtime::lock(&CONFIG) = old_config;
        *crate::runtime::lock(&CONFIG_DOC) = old_doc;
        *crate::runtime::lock(&CONFIG_PATH) = old_path;
        *crate::runtime::lock(&CONFIG_SOURCE) = old_source;
        CONFIG_LOAD_FAILED.store(old_failed, Ordering::SeqCst);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
