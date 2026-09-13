//! Native tooltip layer for the control panel (Go tooltips.go parity):
//! one tooltip control hosting a tool per panel control, refreshed when
//! language or runtime state changes.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{
    TOOLTIPS_CLASSW, TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH,
    TTM_SETTIPBKCOLOR, TTM_SETTIPTEXTCOLOR, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP, TTTOOLINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetDlgItem, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SendMessageW, SetWindowPos, WINDOW_STYLE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::PCWSTR;

static TOOLTIP_HWND: AtomicIsize = AtomicIsize::new(0);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Creates the shared tooltip control for `panel` and registers a tool per
/// control id with its Go-parity i18n key.
pub fn create_for_panel(panel: HWND) {
    unsafe {
        let tip = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            TOOLTIPS_CLASSW,
            PCWSTR::null(),
            WINDOW_STYLE(WS_POPUP.0 | TTS_ALWAYSTIP),
            0,
            0,
            0,
            0,
            Some(panel),
            None,
            Some(
                windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                    .unwrap_or_default()
                    .into(),
            ),
            None,
        )
        .unwrap_or_default();
        if tip.is_invalid() {
            return;
        }
        TOOLTIP_HWND.store(tip.0 as isize, Ordering::SeqCst);
        // Multi-line tips wrap at ~360 logical px like the Go tooltips.
        let _ = SendMessageW(
            tip,
            TTM_SETMAXTIPWIDTH,
            Some(WPARAM(0)),
            Some(LPARAM(crate::scale_pub(360) as isize)),
        );
        retheme();
        // Go ApplyTooltip borrows the panel font (14/400 message font).
        let _ = SendMessageW(
            tip,
            0x0030, // WM_SETFONT
            Some(WPARAM(crate::make_font_pub(14, 400).0 as usize)),
            Some(LPARAM(0)),
        );
        // Keep the tip above other windows so it isn't hidden by the panel.
        let _ = SetWindowPos(
            tip,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
        );

        // (control id, i18n key) pairs — Go tooltips.go mapping.
        let tools = [
            (crate::IDC_NOSLEEP, "tip_nosleep"),
            (crate::IDC_IDLE, "tip_idle"),
            (crate::IDC_AUTOMATION, "tip_automation_master"),
            (crate::IDC_SYSTEM_BUTTON, "tip_quick_actions"),
            (crate::IDC_SETTINGS_BUTTON, "tip_settings"),
            (crate::IDC_THEME_ENABLE, "tip_theme"),
            (crate::IDC_THEME_SWITCH, "tip_theme_switch"),
            (crate::IDC_THEME_REPAIR, "tip_theme_repair"),
            (crate::IDC_MANAGE_BUTTON, "tip_automation"),
            (crate::IDC_EXIT_BUTTON, "tip_exit"),
        ];
        for (id, key) in tools {
            add_tool(panel, id, key);
        }
    }
}

unsafe fn add_tool(panel: HWND, id: usize, key: &str) {
    unsafe {
        let tip = HWND(TOOLTIP_HWND.load(Ordering::SeqCst) as *mut _);
        if tip.is_invalid() {
            return;
        }
        let target = get_panel_child(panel, id);
        if target.is_invalid() {
            return;
        }
        let mut tool = TTTOOLINFOW {
            cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_IDISHWND | TTF_SUBCLASS,
            hwnd: panel,
            uId: target.0 as usize,
            ..Default::default()
        };
        let text = wide(&crate::t_pub(key));
        tool.lpszText = windows::core::PWSTR(text.as_ptr() as *mut _);
        let _ = SendMessageW(
            tip,
            TTM_ADDTOOLW,
            Some(WPARAM(0)),
            Some(LPARAM(&tool as *const _ as isize)),
        );
    }
}

/// Re-themes the shared tooltip after a light/dark switch (Go ApplyTooltip).
pub fn retheme() {
    unsafe {
        let tip = HWND(TOOLTIP_HWND.load(Ordering::SeqCst) as *mut _);
        if tip.is_invalid() {
            return;
        }
        crate::theme::apply_control_theme(tip);
        let _ = SendMessageW(
            tip,
            TTM_SETTIPBKCOLOR,
            Some(WPARAM(crate::theme::tooltip_bg_color() as usize)),
            Some(LPARAM(0)),
        );
        let _ = SendMessageW(
            tip,
            TTM_SETTIPTEXTCOLOR,
            Some(WPARAM(crate::theme::tooltip_text_color() as usize)),
            Some(LPARAM(0)),
        );
    }
}

/// Updates every tool's text (language change / runtime state change).
pub fn refresh_all(panel: HWND) {
    unsafe {
        let tip = HWND(TOOLTIP_HWND.load(Ordering::SeqCst) as *mut _);
        if tip.is_invalid() {
            return;
        }
        let tools: [(usize, String); 10] = [
            (crate::IDC_NOSLEEP, state_power_tip(true)),
            (crate::IDC_IDLE, state_power_tip(false)),
            (
                crate::IDC_AUTOMATION,
                toggle_state_tip(crate::IDC_AUTOMATION, "tip_automation_master"),
            ),
            (crate::IDC_SYSTEM_BUTTON, crate::t_pub("tip_quick_actions")),
            (crate::IDC_SETTINGS_BUTTON, crate::t_pub("tip_settings")),
            (
                crate::IDC_THEME_ENABLE,
                toggle_state_tip(crate::IDC_THEME_ENABLE, "tip_theme"),
            ),
            (
                crate::IDC_THEME_SWITCH,
                toggle_state_tip(crate::IDC_THEME_SWITCH, "tip_theme_switch"),
            ),
            (
                crate::IDC_THEME_REPAIR,
                toggle_state_tip(crate::IDC_THEME_REPAIR, "tip_theme_repair"),
            ),
            (
                crate::IDC_MANAGE_BUTTON,
                toggle_state_tip(crate::IDC_MANAGE_BUTTON, "tip_automation"),
            ),
            (crate::IDC_EXIT_BUTTON, crate::t_pub("tip_exit")),
        ];
        for (id, key_or_text) in tools {
            let target = get_panel_child(panel, id);
            if target.is_invalid() {
                continue;
            }
            let text = if key_or_text.starts_with("tip_") || key_or_text.starts_with("automation") {
                crate::t_pub(&key_or_text)
            } else {
                key_or_text.to_string()
            };
            let mut tool = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                uFlags: TTF_IDISHWND,
                hwnd: panel,
                uId: target.0 as usize,
                ..Default::default()
            };
            let wide_text = wide(&text);
            tool.lpszText = windows::core::PWSTR(wide_text.as_ptr() as *mut _);
            let _ = SendMessageW(
                tip,
                TTM_UPDATETIPTEXTW,
                Some(WPARAM(0)),
                Some(LPARAM(&tool as *const _ as isize)),
            );
        }
    }
}

/// Go withPowerStatusTooltip: 手动设置 state. 运行状态 status. body
fn state_power_tip(nosleep: bool) -> String {
    let manual_key = if crate::cfg_map(|c| {
        if nosleep {
            c.nosleep_enabled
        } else {
            c.idle_enabled
        }
    }) {
        "tip_state_enabled"
    } else {
        "tip_state_disabled"
    };
    let body = crate::t_pub(if nosleep { "tip_nosleep" } else { "tip_idle" });
    let (runtime, full_body) = if nosleep {
        (crate::t_pub("tip_state_disabled"), body)
    } else {
        // Idle: append the manual plan line (Go idleTooltipBody).
        let (minutes, action, warning) = crate::cfg_map(|c| {
            (
                c.idle_timeout_minutes,
                c.idle_action.clone(),
                c.idle_warning_seconds,
            )
        });
        let action_label = crate::t_pub(&format!("menu_action_{action}"));
        let plan = crate::t_pub("tip_idle_manual_plan_warning")
            .replacen("%d", &minutes.to_string(), 1)
            .replacen("%s", &action_label, 1)
            .replacen("%d", &warning.to_string(), 1);
        (
            crate::t_pub("tip_state_disabled"),
            format!("{body}\\n{plan}"),
        )
    };
    crate::t_pub("tip_power_setting_status")
        .replacen("%s", &crate::t_pub(manual_key), 1)
        .replacen("%s", &runtime, 1)
        .replacen("%s", &full_body, 1)
}

/// Go withStateTooltip: state line then description for toggle controls.
fn toggle_state_tip(id: usize, body_key: &str) -> String {
    let active = crate::toggle_value(id);
    let state_key = if active {
        "tip_state_enabled"
    } else {
        "tip_state_disabled"
    };
    crate::t_pub("tip_toggle_state")
        .replacen("%s", &crate::t_pub(state_key), 1)
        .replacen("%s", &crate::t_pub(body_key), 1)
}

unsafe fn get_panel_child(panel: HWND, id: usize) -> HWND {
    unsafe { GetDlgItem(Some(panel), id as i32).unwrap_or_default() }
}
