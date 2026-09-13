//! Developer-tools modes, compiled only with `--features devtools` and
//! gated at runtime by IDLETRIGGER_DEVTOOLS=1. Mirrors the Go devtools
//! contract: runtime overrides only, config never rewritten.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

pub static FORCE_LOG: AtomicBool = AtomicBool::new(false);
pub static INPUT_TRACE: AtomicBool = AtomicBool::new(false);
pub static IDLE_MONITOR_TEST: AtomicBool = AtomicBool::new(false);
pub static IDLE_TEST_SECONDS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static WARNING_PREVIEW: AtomicBool = AtomicBool::new(false);
pub static CAPTURE_PANEL: AtomicBool = AtomicBool::new(false);
static ACTION_WARNING_PREVIEW: AtomicBool = AtomicBool::new(false);
static THEME_OVERRIDE: AtomicI32 = AtomicI32::new(0); // 0 none, 1 dark, 2 light

/// Parses IDLETRIGGER_DEVTOOLS and its derived variables. Returns false
/// when the master switch is absent (all derived vars then ignored, with a
/// log note naming the ignored variables).
pub fn load() -> bool {
    let Ok(master) = std::env::var("IDLETRIGGER_DEVTOOLS") else {
        return false;
    };
    if master != "1" {
        return false;
    }
    let mut active = false;
    if std::env::var("IDLETRIGGER_DEVTOOLS_LOG").as_deref() == Ok("1") {
        FORCE_LOG.store(true, Ordering::SeqCst);
        active = true;
    }
    if std::env::var("IDLETRIGGER_DEVTOOLS_INPUT_TRACE").as_deref() == Ok("1") {
        INPUT_TRACE.store(true, Ordering::SeqCst);
        FORCE_LOG.store(true, Ordering::SeqCst); // trace implies debug log
        active = true;
    }
    if let Ok(secs) = std::env::var("IDLETRIGGER_DEVTOOLS_IDLE_MONITOR_SECONDS")
        && let Ok(secs) = secs.parse::<u32>()
        && (10..=600).contains(&secs)
    {
        IDLE_MONITOR_TEST.store(true, Ordering::SeqCst);
        IDLE_TEST_SECONDS.store(secs, Ordering::SeqCst);
        active = true;
    }
    if std::env::var("IDLETRIGGER_DEVTOOLS_WARNING_PREVIEW").as_deref() == Ok("1") {
        WARNING_PREVIEW.store(true, Ordering::SeqCst);
        active = true;
    }
    if std::env::var("IDLETRIGGER_DEVTOOLS_ACTION_WARNING").as_deref() == Ok("1") {
        ACTION_WARNING_PREVIEW.store(true, Ordering::SeqCst);
        active = true;
    }
    if std::env::var("IDLETRIGGER_DEVTOOLS_CAPTURE_PANEL").as_deref() == Ok("1") {
        CAPTURE_PANEL.store(true, Ordering::SeqCst);
        active = true;
    }
    if let Ok(theme) = std::env::var("IDLETRIGGER_DEVTOOLS_THEME") {
        match theme.as_str() {
            "dark" => {
                THEME_OVERRIDE.store(1, Ordering::SeqCst);
                active = true;
            }
            "light" => {
                THEME_OVERRIDE.store(2, Ordering::SeqCst);
                active = true;
            }
            _ => {}
        }
    }
    active
}

/// Applies the forced theme after the registry read so the override wins
/// (capture support; Go settingspanel themeOverride parity).
pub fn apply_theme_override() {
    match THEME_OVERRIDE.load(Ordering::SeqCst) {
        1 => crate::theme::force_dark(true),
        2 => crate::theme::force_dark(false),
        _ => {}
    }
}

/// Applies IDLETRIGGER_DEVTOOLS_CONFIG_KV key=value overrides to the runtime
/// config (capture support; never written back).
pub fn apply_config_overrides(config: &mut idletrigger_core::config::Config) {
    for pair in std::env::var("IDLETRIGGER_DEVTOOLS_CONFIG_KV")
        .unwrap_or_default()
        .split(';')
        .filter(|kv| !kv.is_empty())
    {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key {
            "theme_mode" => config.theme_mode = value.to_string(),
            "theme_ip_location_enabled" => {
                config.theme_ip_location_enabled = value == "1" || value == "true"
            }
            "lock_keys_enabled" => config.lock_keys_enabled = value == "1" || value == "true",
            _ => {}
        }
    }
}

/// Applies the idle-monitor test override: force monitor on with the test
/// threshold, Lock action, 5s warning. Runtime-only — the config values the
/// user sees are untouched and never written back.
pub fn apply_idle_test(config: &mut idletrigger_core::config::Config) {
    if IDLE_MONITOR_TEST.load(Ordering::SeqCst) {
        config.idle_enabled = true;
        config.idle_timeout_minutes =
            ((IDLE_TEST_SECONDS.load(Ordering::SeqCst) / 60) as i32).max(1);
        config.idle_action = "lock".into();
        config.idle_warning_seconds = 5;
        config.nosleep_enabled = false; // mutual exclusion
    }
    if FORCE_LOG.load(Ordering::SeqCst) {
        config.logging_enabled = true;
    }
}

/// Shows the warning preview once the UI is up (devtools mode only).
pub fn maybe_show_warning_preview() {
    if WARNING_PREVIEW.load(Ordering::SeqCst) {
        crate::popups_show_warning_preview();
    }
    if ACTION_WARNING_PREVIEW.load(Ordering::SeqCst) {
        // Countdown preview (automation system-action warning): a fake
        // pending action posted to the UI thread like the scheduler does.
        let mut pending = crate::automation::PendingAction {
            action: "restart".into(),
            seconds: 10,
            once_date: None,
            rule_id: "devtools-preview".into(),
        };
        pending.seconds = 10;
        *crate::automation::PENDING_ACTION.lock().unwrap() = Some(pending);
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(crate::hwnd(&crate::HIDDEN)),
                crate::WM_ACTION_SHOW,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            );
        }
    }
}

/// Input-trace hook: logs user-visible input state transitions (lock keys,
/// idle resets) when the mode is active. A full low-level keyboard hook
/// (WH_KEYBOARD_LL with vkCode + LLKHF_INJECTED) is deliberately not
/// installed in the tray process — keylogging-adjacent behavior — so trace
/// coverage is state transitions, not per-key codes. Documented deviation
/// from the Go implementation.
pub fn trace_input(event: &str) {
    if INPUT_TRACE.load(Ordering::SeqCst) {
        crate::log_line(&format!("input-trace: {event}"));
    }
}

/// Captures the panel window to a BMP beside the EXE (capture-panel mode).
/// Call after the UI is up; safe from any thread.
pub fn capture_panel_once() {
    if !CAPTURE_PANEL.load(Ordering::SeqCst) {
        return;
    }
    let panel = crate::hwnd(&crate::PANEL);
    let out = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .map(|dir| dir.join("IdleTrigger-panel-capture.bmp"));
    let Some(out) = out else { return };
    match crate::capture_client_bmp_pub(panel, &out) {
        Ok(()) => crate::log_line(&format!("panel captured -> {}", out.display())),
        Err(err) => crate::log_line(&format!("panel capture failed: {err}")),
    }
}

/// Capture-mode timer id (panel-owned).
pub const CAPTURE_TIMER: usize = 99;
static CAPTURE_STEP: AtomicI32 = AtomicI32::new(0);

/// Starts the capture walk on a panel timer: panel first, then the four
/// settings pages (Go CapturePage parity), then process exit. Ticks leave
/// a full interval for paint before each shot. The interval defaults to
/// 500ms and can be raised via IDLETRIGGER_DEVTOOLS_CAPTURE_STEP_MS.
pub fn start_capture_sequence() {
    if !CAPTURE_PANEL.load(Ordering::SeqCst) {
        return;
    }
    let interval = std::env::var("IDLETRIGGER_DEVTOOLS_CAPTURE_STEP_MS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| (200..=10_000).contains(v))
        .unwrap_or(500);
    let panel = crate::hwnd(&crate::PANEL);
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetTimer(
            Some(panel),
            CAPTURE_TIMER,
            interval,
            None,
        );
    }
}

/// Advances the capture walk; returns true when the timer was consumed.
pub fn handle_capture_timer() -> bool {
    if !CAPTURE_PANEL.load(Ordering::SeqCst) {
        return false;
    }
    let step = CAPTURE_STEP.fetch_add(1, Ordering::SeqCst);
    crate::log_line(&format!("capture step {} fired", step));
    let Some(out_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
    else {
        std::process::exit(0);
    };
    let shoot = |hwnd: windows::Win32::Foundation::HWND, out: std::path::PathBuf, what: &str| {
        match crate::capture_client_bmp_pub(hwnd, &out) {
            Ok(()) => crate::log_line(&format!("{what} captured -> {}", out.display())),
            Err(err) => crate::log_line(&format!("{what} capture failed: {err}")),
        }
    };
    match step {
        0 => {
            shoot(
                crate::hwnd(&crate::PANEL),
                out_dir.join("IdleTrigger-panel-capture.bmp"),
                "panel",
            );
            crate::settings_ui::show();
        }
        1 | 3 | 5 | 7 => crate::settings_ui::devtools_select_page(step / 2),
        2 | 4 | 6 | 8 => {
            let page = step / 2 - 1;
            shoot(
                crate::settings_ui::devtools_capture_hwnd(),
                out_dir.join(format!("IdleTrigger-settings-page{page}-capture.bmp")),
                &format!("settings page {page}"),
            );
        }
        9 => {
            if std::env::var("IDLETRIGGER_DEVTOOLS_CLICK_LIST").as_deref() == Ok("1") {
                crate::automation_ui::devtools_seed_demo_rule();
            }
            crate::automation_ui::show();
        }
        10 => {
            let mgr = crate::automation_ui::theme_hwnds()[0];
            if std::env::var("IDLETRIGGER_DEVTOOLS_CLICK_LIST").as_deref() == Ok("1") {
                // Simulate a user click on the first rule row, then let the
                // next tick capture the post-click state.
                crate::automation_ui::devtools_click_list();
            }
            shoot(
                mgr,
                out_dir.join("IdleTrigger-manager-capture.bmp"),
                "automation manager",
            );
        }
        11 => crate::automation_ui::devtools_show_editor(),
        12 => {
            let editor = crate::automation_ui::theme_hwnds()[1];
            shoot(
                editor,
                out_dir.join("IdleTrigger-editor-capture.bmp"),
                "rule editor",
            );
        }
        13 => crate::automation_ui::devtools_open_trigger_choice(),
        14 => {
            let popup = crate::choice::open_popup();
            shoot(
                popup,
                out_dir.join("IdleTrigger-choice-popup-capture.bmp"),
                "choice popup",
            );
        }
        15 => {
            // Close any open choice popup first (it captures mouse input
            // and would prevent the timer from firing).
            crate::choice::close(false);
            crate::automation_ui::devtools_show_picker();
        }
        16 => {
            let picker = crate::automation_ui::theme_hwnds()[2];
            shoot(
                picker,
                out_dir.join("IdleTrigger-picker-capture.bmp"),
                "process picker",
            );
        }
        _ => {
            crate::log_line("capture sequence complete");
            std::process::exit(0);
        }
    }
    // Optionally freeze the walk after a step so a live window can be
    // exercised by hand (real mouse input) without the timer advancing.
    if let Ok(stop_after) = std::env::var("IDLETRIGGER_DEVTOOLS_CAPTURE_STOP_AFTER")
        && let Ok(stop_after) = stop_after.parse::<usize>()
        && step == stop_after as i32
    {
        CAPTURE_PANEL.store(false, Ordering::SeqCst);
        crate::log_line("capture walk frozen for manual interaction");
    }
    true
}
