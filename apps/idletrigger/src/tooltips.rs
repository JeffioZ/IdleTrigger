//! Native tooltip layer for the control panel:
//! one tooltip control hosting a tool per panel control, refreshed when
//! language or runtime state changes.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::Controls::{
    TOOLTIPS_CLASSW, TTF_ABSOLUTE, TTF_IDISHWND, TTF_SUBCLASS, TTF_TRACK, TTM_ADDTOOLW,
    TTM_GETTOOLINFOW, TTM_POP, TTM_SETMAXTIPWIDTH, TTM_SETTIPBKCOLOR, TTM_SETTIPTEXTCOLOR,
    TTM_SETTOOLINFOW, TTM_TRACKACTIVATE, TTM_TRACKPOSITION, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP,
    TTTOOLINFOW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetDlgItem, GetWindowRect, IsWindow, SendMessageW, WINDOW_STYLE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::PCWSTR;

static TOOLTIP_HWND: AtomicIsize = AtomicIsize::new(0);

use crate::wide;

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
            Some(WPARAM(crate::panel_font_body().0 as usize)),
            Some(LPARAM(0)),
        );
        // The WS_EX_TOPMOST style from creation keeps the tip above other
        // windows; nothing further to do here.

        // (control id, caption source) pairs — Go tooltips.go mapping plus
        // the segmented control, chip strips, and header action links. ONE
        // table drives both the initial registration and every per-second
        // refresh, so the two sites cannot drift apart.
        for (id, source) in TOOLS {
            add_tool(panel, id, &source.render());
        }
    }
}

unsafe fn add_tool(panel: HWND, id: usize, text: &str) {
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
        let wide_text = wide(text);
        tool.lpszText = windows::core::PWSTR(wide_text.as_ptr() as *mut _);
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

/// A panel tool's caption source: a plain i18n key, or one of the stateful
/// tips that compose their text at call time (Go mixing of static keys and
/// per-render strings). Replaces the old `starts_with("tip_")` guesswork.
enum TipSource {
    Key(&'static str),
    State(fn() -> String),
}

impl TipSource {
    fn render(self) -> String {
        match self {
            TipSource::Key(key) => crate::t_pub(key),
            TipSource::State(render) => render(),
        }
    }
}

fn state_power_tip_on() -> String {
    state_power_tip(true)
}

fn state_power_tip_off() -> String {
    state_power_tip(false)
}

fn toggle_automation_tip() -> String {
    toggle_state_tip(crate::IDC_AUTOMATION, "tip_automation_master")
}

fn toggle_theme_tip() -> String {
    let mut body = crate::t_pub("tip_theme");
    // Same rule as every other sub-setting: armed (on) states get a line;
    // an off sub-setting does nothing, so it stays silent.
    if crate::cfg_map(|c| c.theme_dark_on_battery) {
        body.push('\n');
        body.push_str(&crate::t_pub("tip_theme_battery_dark_on"));
    }
    if crate::cfg_map(|c| c.theme_skip_fullscreen) {
        body.push('\n');
        body.push_str(&crate::t_pub("tip_theme_skip_fullscreen_on"));
    }
    toggle_state_tip_body(crate::IDC_THEME_ENABLE, body)
}

// The power summary line is compact on the panel; its tooltip carries the
// verbose overview. The other status lines draw with an end ellipsis and
// their tooltips carry the full untruncated live text.
fn status_power_summary() -> String {
    crate::power_overview_verbose()
}

fn status_automation_summary() -> String {
    crate::status_line_text(crate::IDC_AUTOMATION_SUMMARY)
}

fn status_theme_schedule() -> String {
    crate::status_line_text(crate::IDC_THEME_SCHEDULE)
}

/// Every panel tool: control id plus its caption source. Action buttons
/// describe what they do; only real toggles carry the enabled/disabled
/// state line.
const TOOLS: [(usize, TipSource); 28] = [
    (
        crate::ctx_ui::FIELD_ID_TITLE,
        TipSource::Key("tip_ctx_f_title"),
    ),
    (
        crate::ctx_ui::FIELD_ID_PROGRAM,
        TipSource::Key("tip_ctx_f_program"),
    ),
    (
        crate::ctx_ui::FIELD_ID_ARGS,
        TipSource::Key("tip_ctx_f_args"),
    ),
    (
        crate::ctx_ui::FIELD_ID_ICON,
        TipSource::Key("tip_ctx_f_icon"),
    ),
    (
        crate::ctx_ui::FIELD_ID_MATCH,
        TipSource::Key("tip_ctx_f_match"),
    ),
    (
        crate::ctx_ui::FIELD_SCOPE,
        TipSource::Key("tip_ctx_f_scope"),
    ),
    (
        crate::ctx_ui::FIELD_CONSOLE,
        TipSource::Key("tip_ctx_f_console"),
    ),
    (
        crate::IDC_POWER_SUMMARY,
        TipSource::State(status_power_summary),
    ),
    (crate::IDC_NOSLEEP, TipSource::State(state_power_tip_on)),
    (crate::IDC_IDLE, TipSource::State(state_power_tip_off)),
    (
        crate::IDC_AUTOMATION,
        TipSource::State(toggle_automation_tip),
    ),
    (
        crate::IDC_AUTOMATION_SUMMARY,
        TipSource::State(status_automation_summary),
    ),
    (crate::IDC_SETTINGS_BUTTON, TipSource::Key("tip_settings")),
    (crate::IDC_THEME_ENABLE, TipSource::State(toggle_theme_tip)),
    (
        crate::IDC_THEME_SCHEDULE,
        TipSource::State(status_theme_schedule),
    ),
    (crate::IDC_THEME_SWITCH, TipSource::Key("tip_theme_switch")),
    (crate::IDC_THEME_REPAIR, TipSource::Key("tip_theme_repair")),
    (crate::IDC_MANAGE_BUTTON, TipSource::Key("tip_automation")),
    (crate::IDC_CTX_BUTTON, TipSource::Key("tip_ctx")),
    (crate::IDC_EXIT_BUTTON, TipSource::Key("tip_exit")),
    (
        crate::IDC_NOSLEEP_TIMED_30M,
        TipSource::Key("tip_nosleep_timed_presets"),
    ),
    (
        crate::IDC_NOSLEEP_TIMED_1H,
        TipSource::Key("tip_nosleep_timed_presets"),
    ),
    (
        crate::IDC_NOSLEEP_TIMED_2H,
        TipSource::Key("tip_nosleep_timed_presets"),
    ),
    (
        crate::IDC_NOSLEEP_TIMED_CANCEL,
        TipSource::Key("tip_nosleep_timed_cancel"),
    ),
    (
        crate::IDC_THEME_SNOOZE_30M,
        TipSource::Key("tip_theme_snooze_presets"),
    ),
    (
        crate::IDC_THEME_SNOOZE_1H,
        TipSource::Key("tip_theme_snooze_presets"),
    ),
    (
        crate::IDC_THEME_SNOOZE_MORNING,
        TipSource::Key("tip_theme_snooze_presets"),
    ),
    (
        crate::IDC_THEME_SNOOZE_CANCEL,
        TipSource::Key("tip_theme_snooze_cancel"),
    ),
];

/// Updates every tool's text (language change / runtime state change).
/// Called every second from `refresh_status`, so the final rendered text of
/// each tool is cached and `TTM_UPDATETIPTEXTW` is only sent on change —
/// comparing the rendered string (not the i18n key) means a language switch
/// naturally invalidates the cache. The tools are HWND-keyed and live for
/// the panel's lifetime, so an unchanged text never needs re-sending.
pub fn refresh_all(panel: HWND) {
    unsafe {
        let tip = HWND(TOOLTIP_HWND.load(Ordering::SeqCst) as *mut _);
        if tip.is_invalid() {
            return;
        }
        // Text updates during switch flights re-register tools and make a
        // visible tooltip flicker; the cache replays them after.
        if SUPPRESS.load(Ordering::SeqCst) {
            return;
        }
        // The wrap width was fixed at creation; keep it in step with the
        // current DPI instead of wrapping at the startup resolution forever.
        static LAST_WIDTH: AtomicIsize = AtomicIsize::new(-1);
        let width = crate::scale_pub(360) as isize;
        if LAST_WIDTH.swap(width, Ordering::SeqCst) != width {
            let _ = SendMessageW(
                tip,
                TTM_SETMAXTIPWIDTH,
                Some(WPARAM(0)),
                Some(LPARAM(width)),
            );
        }
        const TOOL_COUNT: usize = TOOLS.len();
        static LAST: std::sync::Mutex<[Option<String>; TOOL_COUNT]> =
            std::sync::Mutex::new([const { None }; TOOL_COUNT]);
        let mut last = crate::runtime::lock(&LAST);
        for (slot, (id, source)) in TOOLS.into_iter().enumerate() {
            let text = source.render();
            if last[slot].as_deref() == Some(text.as_str()) {
                continue;
            }
            let target = get_panel_child(panel, id);
            if target.is_invalid() {
                // Cache only after a successful send: a skipped slot must
                // stay pending so a later refresh populates it.
                continue;
            }
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
            last[slot] = Some(text);
        }
    }
}

/// Whether a previous focus-tip activation belongs to `control` (so a
/// KILLFOCUS can hide it even though focus tips only ever show for the
/// controls that opted in).
pub(crate) fn focus_tip_active(control: HWND) -> bool {
    FOCUS_TIP.load(Ordering::SeqCst) == control.0 as isize
}

static FOCUS_TIP: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
/// While true (a switch toggle or its flight is painting), text refreshes
/// pause so updating a visible tooltip cannot flicker it. Never TTM_POP
/// from here: while ANY tool is track-active (a focus tip), popping it
/// wedges comctl32 — tracked tools ignore the mouse, so nothing ever
/// re-activates it and tooltips stop appearing at all. Pops belong in
/// set_focus_tip's deactivation, after TRACKACTIVATE FALSE.
static SUPPRESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn set_suppressed(value: bool) {
    SUPPRESS.store(value, Ordering::SeqCst);
}

/// Pops the tooltip for a keyboard-focused control. The tool is fetched
/// first because TTM_SETTOOLINFOW rewrites it wholesale: the live flags and
/// text must survive the track-mode dance. Tracked placement is anchored
/// under the control and clamped into its monitor's work area — without an
/// explicit TTM_TRACKPOSITION a tracked tip appears wherever the cursor
/// happened to be, which can sit outside the panel entirely.
pub fn set_focus_tip(control: HWND, active: bool) {
    unsafe {
        let tip = HWND(TOOLTIP_HWND.load(Ordering::SeqCst) as *mut _);
        if tip.is_invalid() {
            return;
        }
        let panel = crate::hwnd(&crate::PANEL);
        if panel.is_invalid() {
            return;
        }
        let mut text = [0u16; 1024];
        let mut tool = TTTOOLINFOW {
            cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_IDISHWND,
            hwnd: panel,
            uId: control.0 as usize,
            lpszText: windows::core::PWSTR(text.as_mut_ptr()),
            ..Default::default()
        };
        if SendMessageW(
            tip,
            TTM_GETTOOLINFOW,
            None,
            Some(LPARAM(&mut tool as *mut _ as isize)),
        )
        .0 == 0
        {
            return; // the control has no registered tool
        }
        if active {
            FOCUS_TIP.store(control.0 as isize, Ordering::SeqCst);
            tool.uFlags |= TTF_TRACK | TTF_ABSOLUTE;
            let _ = SendMessageW(
                tip,
                TTM_SETTOOLINFOW,
                None,
                Some(LPARAM(&tool as *const _ as isize)),
            );
            let (x, y) = track_position(control);
            let _ = SendMessageW(
                tip,
                TTM_TRACKPOSITION,
                None,
                Some(LPARAM(
                    (((y as isize) & 0xFFFF) << 16) | ((x as isize) & 0xFFFF),
                )),
            );
        }
        let _ = SendMessageW(
            tip,
            TTM_TRACKACTIVATE,
            Some(WPARAM(active as usize)),
            Some(LPARAM(&tool as *const _ as isize)),
        );
        if !active {
            // Restore hover semantics: a tool left with TTF_TRACK keeps
            // ignoring the mouse, which is how one tip could monopolize the
            // whole tooltip control.
            tool.uFlags &= !(TTF_TRACK | TTF_ABSOLUTE);
            let _ = SendMessageW(
                tip,
                TTM_SETTOOLINFOW,
                None,
                Some(LPARAM(&tool as *const _ as isize)),
            );
            if focus_tip_active(control) {
                FOCUS_TIP.store(0, Ordering::SeqCst);
            }
            // The tool is no longer track-active, so a plain pop cannot
            // strand it. Some owner states (e.g. the panel hiding while the
            // tip shows) make comctl ignore EVERY hide request — TTM_POP
            // included — leaving the window afloat forever; hide the window
            // directly when the message-level hide did not take.
            let _ = SendMessageW(tip, TTM_POP, None, None);
            if windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(tip).as_bool() {
                let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                    tip,
                    windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
                );
            }
        }
    }
}

/// Anchor point for a tracked focus tip: under the control's bottom-left,
/// clamped into the control's monitor work area.
unsafe fn track_position(control: HWND) -> (i32, i32) {
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(control, &mut rect).is_err() {
            return (0, 0);
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let mut work = rect;
        if GetMonitorInfoW(
            MonitorFromWindow(control, MONITOR_DEFAULTTONEAREST),
            &mut info,
        )
        .as_bool()
        {
            work = info.rcWork;
        }
        let gap = crate::scale_pub(4);
        // Pin max at min first: a degenerate work area would otherwise make
        // clamp panic on min > max (popups.rs keeps the same guard).
        let x = rect.left.clamp(work.left, work.right.max(work.left) - 1);
        let y = (rect.bottom + gap).clamp(work.top, work.bottom.max(work.top) - 1);
        (x, y)
    }
}

/// Queues a focus-tip dismissal onto the panel's message loop. Dismissing
/// synchronously from inside ShowWindow/DestroyWindow dispatch poisons the
/// tooltip control — every later hide becomes a no-op and the visible tip
/// wedges until restart — so the actual deactivation must run in a plain
/// message dispatch, which posted delivery guarantees. The handler re-checks
/// the recorded tip, so a dismissal that lost its race with a newer
/// activation is a harmless no-op.
pub(crate) fn dismiss_focus_tip(control: HWND) {
    let panel = crate::hwnd(&crate::PANEL);
    if panel.is_invalid() {
        return;
    }
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            Some(panel),
            crate::WM_FOCUSTIP_DISMISS,
            WPARAM(0),
            LPARAM(control.0 as isize),
        );
    }
}

/// Mouse input on `control`: a keyboard focus tip yields (the Windows
/// keyboard-cue convention), unless the pointer rests on the tip's own
/// control where it doubles as the hover tip.
pub(crate) fn yield_focus_tip(control: HWND) {
    let stored = FOCUS_TIP.load(Ordering::SeqCst);
    if stored != 0 && stored != control.0 as isize {
        dismiss_focus_tip(HWND(stored as *mut _));
    }
}

/// Dismisses any focus tip (panel hiding, destruction).
pub(crate) fn hide_focus_tip() {
    let stored = FOCUS_TIP.load(Ordering::SeqCst);
    if stored != 0 {
        dismiss_focus_tip(HWND(stored as *mut _));
    }
}

/// Self-healing net run from the panel timer: a recorded focus tip must
/// belong to the control that currently holds focus. Anything else leaked
/// from a missed path and is dismissed here, so a stale track activation
/// can never outlive its cause by more than a second.
pub(crate) fn reconcile_focus_tip() {
    let stored = FOCUS_TIP.load(Ordering::SeqCst);
    if stored == 0 {
        return;
    }
    unsafe {
        let control = HWND(stored as *mut _);
        if !IsWindow(Some(control)).as_bool() || GetFocus() != control {
            dismiss_focus_tip(control);
        }
    }
}

/// Drops the shared tooltip state when the panel window is destroyed.
pub(crate) fn on_panel_destroyed() {
    TOOLTIP_HWND.store(0, Ordering::SeqCst);
    FOCUS_TIP.store(0, Ordering::SeqCst);
    SUPPRESS.store(false, Ordering::SeqCst);
}

/// The shared tooltip control's window (regression tests).
#[cfg(test)]
pub(crate) fn tooltip_hwnd() -> HWND {
    HWND(TOOLTIP_HWND.load(Ordering::SeqCst) as *mut _)
}

/// Power rows share the toggles' 当前-status frame: the switch itself sits
/// next to this tooltip, so its state needs no line of its own.
fn state_power_tip(nosleep: bool) -> String {
    let body = crate::t_pub(if nosleep { "tip_nosleep" } else { "tip_idle" });
    let (awake_status, idle_status) = crate::power_status();
    let (runtime, full_body) = if nosleep {
        // Stay awake: the pause sub-settings live only in Settings, and the
        // runtime line explains them just after they pause the feature.
        let (pause_on_lock, battery_allowed, battery_threshold) = crate::cfg_map(|c| {
            (
                c.nosleep_pause_on_lock,
                c.nosleep_on_battery,
                c.nosleep_battery_threshold,
            )
        });
        let mut full_body = body;
        if pause_on_lock {
            full_body.push('\n');
            full_body.push_str(&crate::t_pub("tip_nosleep_pause_lock_on"));
        }
        if battery_allowed {
            full_body.push('\n');
            full_body.push_str(&crate::t_args(
                "tip_nosleep_battery_allowed",
                &[&battery_threshold.to_string()],
            ));
        }
        (awake_status, full_body)
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
        let plan = crate::t_args(
            "tip_idle_manual_plan_warning",
            &[&minutes.to_string(), &action_label, &warning.to_string()],
        );
        // While idle monitoring runs, the runtime line above already states
        // the plan as a sentence (with the effective minutes), so the plan
        // line and the enhanced note appear only in the other states.
        let mut full_body = body;
        if !crate::idle_status_active() {
            full_body.push('\n');
            full_body.push_str(&plan);
            if crate::cfg_map(|c| c.idle_enhanced_monitor) {
                full_body.push('\n');
                full_body.push_str(&crate::t_pub("tip_idle_enhanced_on"));
            }
        }
        (idle_status, full_body)
    };
    crate::t_args("tip_toggle_state", &[&runtime, &full_body])
}

/// Go withStateTooltip: state line then description for toggle controls.
fn toggle_state_tip(id: usize, body_key: &str) -> String {
    toggle_state_tip_body(id, crate::t_pub(body_key))
}

fn toggle_state_tip_body(id: usize, body: String) -> String {
    let active = crate::toggle_value(id);
    let state_key = if active {
        "tip_state_enabled"
    } else {
        "tip_state_disabled"
    };
    crate::t_args("tip_toggle_state", &[&crate::t_pub(state_key), &body])
}

unsafe fn get_panel_child(panel: HWND, id: usize) -> HWND {
    unsafe { GetDlgItem(Some(panel), id as i32).unwrap_or_default() }
}
