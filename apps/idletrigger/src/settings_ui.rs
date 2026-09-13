//! Settings window — Go settingspanel parity: four left navigation tabs,
//! per-page content on the right, and a footer with inline validation,
//! Save, and Cancel. Geometry mirrors the Go logical tokens (700×524
//! client, tabs 156×36 at x=24, content x=208..676).

use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

// Edit interior brushes (surface / disabled surface), live for the process.
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{HDC, HFONT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    EM_SETLIMITTEXT, TOOLTIPS_CLASSW, TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH,
    TTS_ALWAYSTIP, TTTOOLINFOW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, IsWindowEnabled, SetFocus,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use crate::{t_pub, theme};

// Control ids — Go settingspanel.go numbering.
const ID_TITLE: i32 = 100;
const ID_DESCRIPTION: i32 = 101;
const ID_TAB_POWER: i32 = 102;
const ID_TAB_THEME: i32 = 103;
const ID_TAB_APP: i32 = 104;
const ID_POWER_TITLE: i32 = 110;
const ID_KEEP_SCREEN: i32 = 111;
const ID_BATTERY_ALLOWED: i32 = 112;
const ID_BATTERY_LBL: i32 = 113;
const ID_BATTERY_THRESH: i32 = 114;
const ID_WARNING_LBL: i32 = 115;
const ID_WARNING_SECONDS: i32 = 116;
const ID_POWER_HINT: i32 = 117;
const ID_IDLE_TIMEOUT_LBL: i32 = 118;
const ID_IDLE_TIMEOUT: i32 = 119;
const ID_IDLE_ACTION_LBL: i32 = 120;
const ID_IDLE_ACTION: i32 = 121;
const ID_IDLE_ENHANCED: i32 = 122;
const ID_IDLE_TITLE: i32 = 123;
const ID_THEME_MODE_LBL: i32 = 131;
const ID_THEME_MODE: i32 = 132;
const ID_LIGHT_TIME_LBL: i32 = 133;
const ID_LIGHT_TIME: i32 = 134;
const ID_DARK_TIME_LBL: i32 = 135;
const ID_DARK_TIME: i32 = 136;
const ID_LOCATION_LBL: i32 = 137;
const ID_LOCATION_SOURCE: i32 = 138;
const ID_THEME_HINT: i32 = 143;
const ID_THEME_BATTERY: i32 = 144;
const ID_THEME_FULLSCREEN: i32 = 145;
const ID_THEME_SCHEDULE_TITLE: i32 = 148;
const ID_THEME_BEHAVIOR_TITLE: i32 = 149;
const ID_LANGUAGE_LBL: i32 = 151;
const ID_LANGUAGE: i32 = 152;
const ID_HOTKEYS: i32 = 153;
const ID_AUTOSTART: i32 = 154;
const ID_LOGGING: i32 = 155;
const ID_PROJECT_HOME: i32 = 157;
const ID_APP_GENERAL_TITLE: i32 = 159;
const ID_VALIDATION: i32 = 160;
const ID_APP_ABOUT_TITLE: i32 = 161;
const ID_CANCEL: i32 = 162;
const ID_SAVE: i32 = 163;
const ID_THEME_LOCATION_STATUS: i32 = 164;
const ID_PROJECT_HOME_LBL: i32 = 165;
const ID_VERSION: i32 = 166;
const ID_LOCK_KEYS: i32 = 167;
const ID_TAB_NOTIFICATIONS: i32 = 168;
const ID_NOTIFICATIONS_TITLE: i32 = 169;
const ID_LOCK_CAPS: i32 = 170;
const ID_LOCK_NUM: i32 = 171;
const ID_LOCK_SCROLL: i32 = 172;
const ID_LOCK_FULLSCREEN: i32 = 173;
const ID_LOCK_PREVIEW: i32 = 174;
const ID_NOTIFICATIONS_HINT: i32 = 175;
const ID_NOTIFICATIONS_BEHAVIOR: i32 = 176;

// Layout tokens — Go controls.go build() constants.
const CLIENT_W: i32 = 700;
const CLIENT_H: i32 = 524;
const CONTENT_X: i32 = 208;
const CONTENT_RIGHT: i32 = 676;
const SECTION_TOP: i32 = 90;
const SECTION_TITLE_H: i32 = 20;
const SECTION_ITEM_GAP: i32 = 10;
const FUNCTION_GAP: i32 = 8;
const SECTION_GAP: i32 = 16;
const CHECK_H: i32 = 28;
const BTN_H: i32 = 36;
const FIELD_H: i32 = 34;
const DIALOG_BTN_W: i32 = 104;
const FOOTER_Y: i32 = CLIENT_H - 18 - BTN_H;

const IDLE_ACTIONS: [&str; 5] = ["lock", "sleep", "hibernate", "shutdown", "restart"];
const LANG_VALUES: [&str; 3] = ["auto", "en", "zh-CN"];
const PROJECT_URL: &str = "https://github.com/JeffioZ/IdleTrigger";

// Edit surfaces use the Go idFieldSurfaceBase offset from the inner edit id.
const FIELD_SURFACE_BASE: i32 = 500;

// Owner-drawn checkbox states (BM_SETCHECK is inert on owner-draw buttons).
static CHECKS: std::sync::Mutex<Option<std::collections::HashMap<i32, bool>>> =
    std::sync::Mutex::new(None);

fn checks() -> std::sync::MutexGuard<'static, Option<std::collections::HashMap<i32, bool>>> {
    CHECKS.lock().unwrap()
}

static SETTINGS_HWND: AtomicIsize = AtomicIsize::new(0);
static PAGE: AtomicI32 = AtomicI32::new(0);
static FONT_BODY: AtomicIsize = AtomicIsize::new(0);
static FONT_SECTION: AtomicIsize = AtomicIsize::new(0);
static FONT_TITLE: AtomicIsize = AtomicIsize::new(0);
static FONT_LINK: AtomicIsize = AtomicIsize::new(0);
static TOOLTIP_HWND: AtomicIsize = AtomicIsize::new(0);

fn s(v: i32) -> i32 {
    crate::scale_pub(v)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn current() -> HWND {
    HWND(SETTINGS_HWND.load(Ordering::SeqCst) as *mut _)
}

fn get(parent: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(parent), id).unwrap_or_default() }
}

fn set_text(parent: HWND, id: i32, text: &str) {
    unsafe {
        let target = get(parent, id);
        if !target.is_invalid() {
            let _ = SetWindowTextW(target, PCWSTR(wide(text).as_ptr()));
        }
    }
}

fn control_text(parent: HWND, id: i32) -> String {
    unsafe {
        let target = get(parent, id);
        if target.is_invalid() {
            return String::new();
        }
        let len = SendMessageW(target, WM_GETTEXTLENGTH, Some(WPARAM(0)), Some(LPARAM(0)))
            .0
            .max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let copied = SendMessageW(
            target,
            WM_GETTEXT,
            Some(WPARAM(buf.len())),
            Some(LPARAM(buf.as_mut_ptr() as isize)),
        )
        .0 as usize;
        String::from_utf16_lossy(&buf[..copied.min(len)])
    }
}

fn is_checked(_parent: HWND, id: i32) -> bool {
    checks()
        .as_ref()
        .and_then(|m| m.get(&id).copied())
        .unwrap_or(false)
}

/// Stores an owner-drawn checkbox state and repaints its control.
fn set_checked(parent: HWND, id: i32, value: bool) {
    checks()
        .get_or_insert_with(Default::default)
        .insert(id, value);
    unsafe {
        let control = get(parent, id);
        if !control.is_invalid() {
            // Erase before repaint: with erase=false, DPI-rounded corners can
            // retain stale pixels from the previous check state, making the
            // toggle appear to need two clicks.
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, true);
        }
    }
}

/// Toggles one owner-drawn checkbox.
fn toggle_checked(parent: HWND, id: i32) {
    let next = !is_checked(parent, id);
    set_checked(parent, id, next);
}

fn combo_sel(parent: HWND, id: i32) -> usize {
    crate::choice::selection(get(parent, id)).max(0) as usize
}

pub fn show() {
    unsafe {
        if current().is_invalid() {
            create();
        }
        let hwnd = current();
        if hwnd.is_invalid() {
            return;
        }
        let _ = EnableWindow(crate::hwnd(&crate::PANEL), false);
        // Paint the full frame before showing so the theme never flashes
        // (Go first-frame presenter parity).
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Devtools capture support: the settings window handle.
#[cfg(feature = "devtools")]
pub fn devtools_capture_hwnd() -> HWND {
    current()
}

/// Theme refresh support: the settings window when open.
pub fn theme_hwnd() -> HWND {
    current()
}

/// Devtools capture support: switches the visible page.
#[cfg(feature = "devtools")]
pub fn devtools_select_page(page: i32) {
    let hwnd = current();
    if hwnd.is_invalid() {
        return;
    }
    PAGE.store(page, Ordering::SeqCst);
    apply_dependent_states(hwnd);
}

fn create() {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module handle");
        let body = crate::make_font_pub(14, 400);
        let section = crate::make_font_pub(14, 600);
        let title_font = crate::make_font_pub(17, 600);
        let link_font = make_link_font(14);
        FONT_BODY.store(body.0 as isize, Ordering::SeqCst);
        FONT_SECTION.store(section.0 as isize, Ordering::SeqCst);
        FONT_TITLE.store(title_font.0 as isize, Ordering::SeqCst);
        FONT_LINK.store(link_font.0 as isize, Ordering::SeqCst);

        register_class(instance);

        let style = WINDOW_STYLE(
            WS_OVERLAPPEDWINDOW.0 & !WS_MAXIMIZEBOX.0 & !WS_THICKFRAME.0 & !WS_MINIMIZEBOX.0,
        );
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: s(CLIENT_W),
            bottom: s(CLIENT_H),
        };
        let _ = AdjustWindowRectEx(&mut frame, style, false, WINDOW_EX_STYLE(0));
        let win_w = frame.right - frame.left;
        let win_h = frame.bottom - frame.top;
        let (x, y) = center_on_parent(win_w, win_h);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("IdleTriggerSettings"),
            PCWSTR(wide(&t_pub("settings_title")).as_ptr()),
            style,
            x,
            y,
            win_w,
            win_h,
            Some(crate::hwnd(&crate::PANEL)),
            None,
            Some(instance.into()),
            None,
        )
        .expect("settings window");
        SETTINGS_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        theme::apply_to_window(hwnd);
        crate::set_window_icons_pub(hwnd);

        build_controls(hwnd, body, section, title_font);
        populate(hwnd);
        PAGE.store(0, Ordering::SeqCst);
        apply_dependent_states(hwnd);
        theme::retheme_children(hwnd);
        create_tooltip(hwnd);
        retheme_tooltip();
    }
}

/// Message-font derivative with an underline, for the project-home link
/// (Go DrawTextLink renders the URL underlined).
fn make_link_font(size_px: i32) -> HFONT {
    unsafe {
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
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
            metrics.lfMessageFont.lfHeight = -s(size_px);
            metrics.lfMessageFont.lfUnderline = 1;
            return windows::Win32::Graphics::Gdi::CreateFontIndirectW(&metrics.lfMessageFont);
        }
        HFONT::default()
    }
}

/// Re-themes the tooltip colors after a light/dark switch (Go ApplyTooltip).
fn retheme_tooltip() {
    unsafe {
        let tip = HWND(TOOLTIP_HWND.load(Ordering::SeqCst) as *mut _);
        if tip.is_invalid() {
            return;
        }
        theme::apply_control_theme(tip);
        let _ = SendMessageW(
            tip,
            windows::Win32::UI::Controls::TTM_SETTIPBKCOLOR,
            Some(WPARAM(theme::tooltip_bg_color() as usize)),
            Some(LPARAM(0)),
        );
        let _ = SendMessageW(
            tip,
            windows::Win32::UI::Controls::TTM_SETTIPTEXTCOLOR,
            Some(WPARAM(theme::tooltip_text_color() as usize)),
            Some(LPARAM(0)),
        );
    }
}

/// Full theme refresh: caption, control visual styles, tooltip, repaint
/// (Go applyTheme on WM_SETTINGCHANGE / WM_SYSCOLORCHANGE / WM_THEMECHANGED).
fn refresh_theme(hwnd: HWND) {
    unsafe {
        theme::refresh_from_registry();
        theme::apply_to_window(hwnd);
        theme::retheme_children(hwnd);
        retheme_tooltip();
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, true);
    }
}

unsafe fn build_controls(hwnd: HWND, font: HFONT, section_font: HFONT, title_font: HFONT) {
    unsafe {
        // Header.
        label(
            hwnd,
            ID_TITLE,
            &t_pub("settings_title"),
            title_font,
            (24, 16, 460, 24),
            false,
        );
        label(
            hwnd,
            ID_DESCRIPTION,
            &t_pub("settings_description"),
            font,
            (24, 42, 652, 20),
            false,
        );
        label(
            hwnd,
            ID_VERSION,
            &t_pub("settings_version").replace("%s", crate::APP_VERSION),
            font,
            (508, 18, 168, 22),
            true,
        );

        // Left navigation tabs (Go: 156×36 at x=24, 44px pitch).
        tab_button(
            hwnd,
            ID_TAB_POWER,
            &t_pub("settings_tab_power"),
            (24, 90, 156, BTN_H),
        );
        tab_button(
            hwnd,
            ID_TAB_THEME,
            &t_pub("settings_tab_theme"),
            (24, 134, 156, BTN_H),
        );
        tab_button(
            hwnd,
            ID_TAB_NOTIFICATIONS,
            &t_pub("settings_tab_notifications"),
            (24, 178, 156, BTN_H),
        );
        tab_button(
            hwnd,
            ID_TAB_APP,
            &t_pub("settings_tab_app"),
            (24, 222, 156, BTN_H),
        );

        // Power and idle page.
        label(
            hwnd,
            ID_POWER_TITLE,
            &t_pub("settings_power_title"),
            section_font,
            (CONTENT_X, SECTION_TOP, 468, SECTION_TITLE_H),
            false,
        );
        checkbox(
            hwnd,
            ID_KEEP_SCREEN,
            &t_pub("settings_keep_screen"),
            (CONTENT_X, 120, 468, CHECK_H),
        );
        checkbox(
            hwnd,
            ID_BATTERY_ALLOWED,
            &t_pub("settings_battery_allowed"),
            (CONTENT_X, 156, 468, CHECK_H),
        );
        label(
            hwnd,
            ID_BATTERY_LBL,
            &t_pub("settings_battery_threshold"),
            font,
            (CONTENT_X, 198, 330, 22),
            false,
        );
        edit(hwnd, ID_BATTERY_THRESH, (548, 190, 128, FIELD_H), true);
        label(
            hwnd,
            ID_POWER_HINT,
            &t_pub("settings_power_hint"),
            font,
            (CONTENT_X, 232, 468, 22),
            false,
        );
        label(
            hwnd,
            ID_IDLE_TITLE,
            &t_pub("settings_idle_title"),
            section_font,
            (CONTENT_X, 270, 468, SECTION_TITLE_H),
            false,
        );
        checkbox(
            hwnd,
            ID_IDLE_ENHANCED,
            &t_pub("menu_idle_enhanced"),
            (CONTENT_X, 300, 468, CHECK_H),
        );
        label(
            hwnd,
            ID_IDLE_TIMEOUT_LBL,
            &t_pub("settings_idle_timeout_minutes"),
            font,
            (CONTENT_X, 344, 330, 22),
            false,
        );
        edit(hwnd, ID_IDLE_TIMEOUT, (548, 336, 128, FIELD_H), true);
        label(
            hwnd,
            ID_WARNING_LBL,
            &t_pub("settings_idle_warning_seconds"),
            font,
            (CONTENT_X, 386, 330, 22),
            false,
        );
        edit(hwnd, ID_WARNING_SECONDS, (548, 378, 128, FIELD_H), true);
        label(
            hwnd,
            ID_IDLE_ACTION_LBL,
            &t_pub("settings_idle_action"),
            font,
            (CONTENT_X, 428, 330, 22),
            false,
        );
        combo(
            hwnd,
            ID_IDLE_ACTION,
            (548, 420, 128, FIELD_H),
            &idle_action_labels(),
        );

        // Day/night page.
        label(
            hwnd,
            ID_THEME_SCHEDULE_TITLE,
            &t_pub("settings_theme_schedule_group"),
            section_font,
            (CONTENT_X, SECTION_TOP, 468, SECTION_TITLE_H),
            false,
        );
        label(
            hwnd,
            ID_THEME_MODE_LBL,
            &t_pub("settings_theme_mode"),
            font,
            (CONTENT_X, 128, 220, 22),
            false,
        );
        combo(
            hwnd,
            ID_THEME_MODE,
            (456, 120, 220, FIELD_H),
            &[
                t_pub("settings_theme_fixed"),
                t_pub("settings_theme_sunrise"),
            ],
        );
        label(
            hwnd,
            ID_LIGHT_TIME_LBL,
            &t_pub("settings_light_time"),
            font,
            (CONTENT_X, 170, 104, 22),
            false,
        );
        edit(hwnd, ID_LIGHT_TIME, (316, 162, 104, FIELD_H), false);
        label(
            hwnd,
            ID_DARK_TIME_LBL,
            &t_pub("settings_dark_time"),
            font,
            (438, 170, 104, 22),
            false,
        );
        edit(hwnd, ID_DARK_TIME, (546, 162, 130, FIELD_H), false);
        label(
            hwnd,
            ID_LOCATION_LBL,
            &t_pub("settings_location_source"),
            font,
            (CONTENT_X, 170, 220, 22),
            false,
        );
        combo(
            hwnd,
            ID_LOCATION_SOURCE,
            (456, 162, 220, FIELD_H),
            &[
                t_pub("settings_location_auto"),
                t_pub("settings_location_ip"),
            ],
        );
        label(
            hwnd,
            ID_THEME_LOCATION_STATUS,
            "",
            font,
            (CONTENT_X, 204, 468, 22),
            false,
        );
        let theme_hint_h = if is_chinese() { 22 } else { 40 };
        label(
            hwnd,
            ID_THEME_HINT,
            &t_pub("settings_theme_hint"),
            font,
            (CONTENT_X, 234, 468, theme_hint_h),
            false,
        );
        let behavior_top = 234 + theme_hint_h + SECTION_GAP;
        label(
            hwnd,
            ID_THEME_BEHAVIOR_TITLE,
            &t_pub("settings_theme_behavior_group"),
            section_font,
            (CONTENT_X, behavior_top, 468, SECTION_TITLE_H),
            false,
        );
        checkbox(
            hwnd,
            ID_THEME_BATTERY,
            &t_pub("menu_theme_battery_dark"),
            (
                CONTENT_X,
                behavior_top + SECTION_TITLE_H + SECTION_ITEM_GAP,
                468,
                CHECK_H,
            ),
        );
        checkbox(
            hwnd,
            ID_THEME_FULLSCREEN,
            &t_pub("menu_theme_skip_fullscreen"),
            (
                CONTENT_X,
                behavior_top + SECTION_TITLE_H + SECTION_ITEM_GAP + CHECK_H + FUNCTION_GAP,
                468,
                CHECK_H,
            ),
        );

        // Application page.
        label(
            hwnd,
            ID_APP_GENERAL_TITLE,
            &t_pub("settings_app_general_group"),
            section_font,
            (CONTENT_X, SECTION_TOP, 468, SECTION_TITLE_H),
            false,
        );
        label(
            hwnd,
            ID_LANGUAGE_LBL,
            &t_pub("settings_language"),
            font,
            (CONTENT_X, 128, 220, 22),
            false,
        );
        combo(
            hwnd,
            ID_LANGUAGE,
            (456, 120, 220, FIELD_H),
            &[
                t_pub("menu_lang_auto"),
                t_pub("menu_lang_en"),
                t_pub("menu_lang_zh"),
            ],
        );
        checkbox(
            hwnd,
            ID_HOTKEYS,
            &t_pub("menu_hotkeys"),
            (CONTENT_X, 162, 468, CHECK_H),
        );
        checkbox(
            hwnd,
            ID_AUTOSTART,
            &t_pub("menu_autostart"),
            (CONTENT_X, 198, 468, CHECK_H),
        );
        checkbox(
            hwnd,
            ID_LOGGING,
            &t_pub("menu_logging"),
            (CONTENT_X, 234, 468, CHECK_H),
        );
        label(
            hwnd,
            ID_APP_ABOUT_TITLE,
            &t_pub("settings_app_about_group"),
            section_font,
            (CONTENT_X, 278, 468, SECTION_TITLE_H),
            false,
        );
        let project_label = t_pub("settings_project_home_label");
        let label_w = logical_text_width(hwnd, font, &project_label, 96) + 2;
        let url_w = logical_text_width(hwnd, font, PROJECT_URL, 376) + 2;
        let mut link_x = CONTENT_X + label_w + FUNCTION_GAP;
        if is_chinese() {
            // CJK advance boxes carry extra trailing space (Go optical fix).
            link_x -= 10;
        }
        label(
            hwnd,
            ID_PROJECT_HOME_LBL,
            &project_label,
            font,
            (CONTENT_X, 310, label_w, 24),
            false,
        );
        // Go renders the URL as an underlined accent-colored text link with
        // a hand cursor (DrawTextLink), not a button.
        link(
            hwnd,
            ID_PROJECT_HOME,
            PROJECT_URL,
            (link_x, 308, url_w.min(CONTENT_RIGHT - link_x), 24),
        );

        // Screen notifications page.
        label(
            hwnd,
            ID_NOTIFICATIONS_TITLE,
            &t_pub("settings_lock_keys"),
            section_font,
            (CONTENT_X, SECTION_TOP, 468, SECTION_TITLE_H),
            false,
        );
        checkbox(
            hwnd,
            ID_LOCK_KEYS,
            &t_pub("settings_lock_keys_enable"),
            (CONTENT_X, 120, 468, CHECK_H),
        );
        checkbox(
            hwnd,
            ID_LOCK_CAPS,
            "Caps Lock",
            (CONTENT_X + 24, 156, 444, CHECK_H),
        );
        checkbox(
            hwnd,
            ID_LOCK_NUM,
            "Num Lock",
            (CONTENT_X + 24, 192, 444, CHECK_H),
        );
        checkbox(
            hwnd,
            ID_LOCK_SCROLL,
            "Scroll Lock",
            (CONTENT_X + 24, 228, 444, CHECK_H),
        );
        label(
            hwnd,
            ID_NOTIFICATIONS_BEHAVIOR,
            &t_pub("settings_notification_behavior"),
            section_font,
            (CONTENT_X, 278, 468, SECTION_TITLE_H),
            false,
        );
        checkbox(
            hwnd,
            ID_LOCK_FULLSCREEN,
            &t_pub("settings_notification_fullscreen"),
            (CONTENT_X, 308, 468, CHECK_H),
        );
        label(
            hwnd,
            ID_NOTIFICATIONS_HINT,
            &t_pub("settings_notification_hint"),
            font,
            (CONTENT_X, 348, 468, 42),
            false,
        );
        push_button(
            hwnd,
            ID_LOCK_PREVIEW,
            &t_pub("settings_notification_preview"),
            (CONTENT_X, 406, 160, BTN_H),
        );

        // Footer: inline validation + Save/Cancel (Go footer geometry).
        label(
            hwnd,
            ID_VALIDATION,
            "",
            font,
            (CONTENT_X, FOOTER_Y + 8, 236, 24),
            false,
        );
        push_button(
            hwnd,
            ID_SAVE,
            &t_pub("common_save"),
            (CONTENT_RIGHT - DIALOG_BTN_W, FOOTER_Y, DIALOG_BTN_W, BTN_H),
        );
        push_button(
            hwnd,
            ID_CANCEL,
            &t_pub("common_cancel"),
            (
                CONTENT_RIGHT - 2 * DIALOG_BTN_W - 8,
                FOOTER_Y,
                DIALOG_BTN_W,
                BTN_H,
            ),
        );
    }
}

fn idle_action_labels() -> Vec<String> {
    IDLE_ACTIONS
        .iter()
        .map(|a| t_pub(&format!("menu_action_{a}")))
        .collect()
}

unsafe fn child(
    parent: HWND,
    class: windows::core::PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    id: i32,
    font: HFONT,
    b: (i32, i32, i32, i32),
) -> HWND {
    unsafe {
        let (x, y, w, h) = b;
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(wide(text).as_ptr()),
            style,
            s(x),
            s(y),
            s(w),
            s(h),
            Some(parent),
            Some(HMENU(id as isize as *mut _)),
            Some(instance.into()),
            None,
        )
        .unwrap_or_default();
        if !hwnd.is_invalid() {
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        hwnd
    }
}

unsafe fn label(
    parent: HWND,
    id: i32,
    text: &str,
    font: HFONT,
    b: (i32, i32, i32, i32),
    right: bool,
) -> HWND {
    // SS_RIGHT = 2 (Go labelRight uses the raw style value).
    let extra = if right { 2 } else { 0 };
    unsafe {
        // Left labels shrink to their measured text so a long translation can
        // never sit under a neighboring field (Go CheckboxHitWidth pattern);
        // right-aligned and full-width labels keep their grid rect.
        let mut b = b;
        if !right && b.2 > 0 && b.2 <= 220 {
            let measured = logical_text_width(parent, font, text, b.2);
            if measured > 0 {
                b.2 = measured;
            }
        }
        child(
            parent,
            windows::core::w!("STATIC"),
            text,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | extra),
            id,
            font,
            b,
        )
    }
}

/// Owner-drawn push button (Go bsOwnerDraw BUTTON + DrawButton).
unsafe fn push_button(parent: HWND, id: i32, text: &str, b: (i32, i32, i32, i32)) {
    unsafe {
        let hwnd = child(
            parent,
            windows::core::w!("BUTTON"),
            text,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            id,
            body_font(),
            b,
        );
        crate::nativeform::track(hwnd);
    }
}

/// Navigation tab: an owner-drawn button latched active to the selected
/// page (Go tab buttons draw with the Active accent fill).
unsafe fn tab_button(parent: HWND, id: i32, text: &str, b: (i32, i32, i32, i32)) {
    unsafe { push_button(parent, id, text, b) };
}

/// Owner-drawn checkbox: check state lives in CHECKS (BM_SETCHECK has no
/// effect on owner-draw buttons), painted via paint::draw_checkbox.
unsafe fn checkbox(parent: HWND, id: i32, text: &str, b: (i32, i32, i32, i32)) {
    unsafe {
        let hwnd = child(
            parent,
            windows::core::w!("BUTTON"),
            text,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            id,
            body_font(),
            b,
        );
        crate::nativeform::track(hwnd);
    }
}

/// Text hyperlink rendered as an owner-drawn button (Go DrawTextLink keeps
/// native keyboard focus and pressed feedback).
unsafe fn link(parent: HWND, id: i32, text: &str, b: (i32, i32, i32, i32)) {
    unsafe {
        let hwnd = child(
            parent,
            windows::core::w!("BUTTON"),
            text,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            id,
            body_font(),
            b,
        );
        crate::nativeform::track(hwnd);
    }
}

/// Edit field: an owner-drawn STATIC surface paints the rounded border
/// (paint::draw_field), with a borderless EDIT inset inside it (Go
/// editWithStyle + ControlSurface parity).
unsafe fn edit(parent: HWND, id: i32, b: (i32, i32, i32, i32), numeric: bool) {
    unsafe {
        let (x, y, w, _) = b;
        // Surface static owns the field visuals; id mirrors Go's
        // idFieldSurfaceBase scheme.
        child(
            parent,
            windows::core::w!("STATIC"),
            "",
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0 | 13), // SS_OWNERDRAW
            FIELD_SURFACE_BASE + id,
            body_font(),
            (x, y, w, FIELD_H),
        );
        let extra = if numeric { ES_NUMBER as u32 } else { 0 };
        child(
            parent,
            windows::core::w!("EDIT"),
            "",
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32 | extra),
            id,
            body_font(),
            (x + 2, y + 7, w - 4, 20),
        );
        // Cap input length: 5 digits for numbers, 5 chars for HH:MM.
        let _ = SendMessageW(
            get(parent, id),
            EM_SETLIMITTEXT,
            Some(WPARAM(5)),
            Some(LPARAM(0)),
        );
    }
}

/// Go choice control: an owner-drawn button with a custom drop-down popup
/// (nativeform choicepopup parity).
fn combo(parent: HWND, id: i32, b: (i32, i32, i32, i32), items: &[String]) {
    {
        let (x, y, w, _) = b;
        let pairs: Vec<(String, String)> = items
            .iter()
            .map(|label| (label.clone(), label.clone()))
            .collect();
        crate::choice::create(parent, id, (x, y, w, FIELD_H), &pairs, body_font());
    }
}

fn body_font() -> HFONT {
    HFONT(FONT_BODY.load(Ordering::SeqCst) as *mut _)
}

fn is_chinese() -> bool {
    let lang = crate::cfg_map(|c| c.language.clone());
    match lang.as_str() {
        "en" => false,
        "zh-CN" => true,
        _ => unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() == 0x0804 },
    }
}

/// Physical text width converted back to logical pixels (Go logicalTextWidth).
fn logical_text_width(parent: HWND, font: HFONT, text: &str, fallback: i32) -> i32 {
    use windows::Win32::Graphics::Gdi::{GetTextExtentPoint32W, ReleaseDC, SelectObject};
    unsafe {
        let hdc = windows::Win32::Graphics::Gdi::GetDC(Some(parent));
        if hdc.is_invalid() {
            return fallback;
        }
        let old = SelectObject(hdc, windows::Win32::Graphics::Gdi::HGDIOBJ(font.0));
        let chars: Vec<u16> = text.encode_utf16().collect();
        let mut size = windows::Win32::Foundation::SIZE::default();
        let measured = GetTextExtentPoint32W(hdc, &chars, &mut size).as_bool();
        SelectObject(hdc, old);
        let _ = ReleaseDC(Some(parent), hdc);
        if !measured || size.cx <= 0 {
            return fallback;
        }
        let dpi = s(96);
        if dpi <= 0 {
            return fallback;
        }
        (size.cx as i64 * 96 + (dpi / 2) as i64) as i32 / dpi
    }
}

unsafe fn register_class(instance: windows::Win32::Foundation::HMODULE) {
    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance.into(),
            lpszClassName: windows::core::w!("IdleTriggerSettings"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            // MAKEINTRESOURCE(1) = winresource's "1 ICON" entry.
            #[allow(clippy::manual_dangling_ptr)]
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1usize as *const u16))
                .unwrap_or_default(),
            hbrBackground: theme::bg_brush(),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
}

fn populate(hwnd: HWND) {
    let (
        keep_screen,
        battery_allowed,
        battery_threshold,
        idle_timeout,
        idle_action,
        warning,
        idle_enhanced,
        theme_mode,
        light_time,
        dark_time,
        ip_enabled,
        theme_battery,
        theme_fullscreen,
        language,
        lock_keys,
        caps,
        num,
        scroll,
        lock_fullscreen,
        hotkeys,
        autostart,
        logging,
    ) = crate::cfg_map(|c| {
        (
            c.keep_screen_on,
            c.nosleep_on_battery,
            c.nosleep_battery_threshold,
            c.idle_timeout_minutes,
            c.idle_action.clone(),
            c.idle_warning_seconds,
            c.idle_enhanced_monitor,
            c.theme_mode.clone(),
            c.theme_light_time.clone(),
            c.theme_dark_time.clone(),
            c.theme_ip_location_enabled,
            c.theme_dark_on_battery,
            c.theme_skip_fullscreen,
            c.language.clone(),
            c.lock_keys_enabled,
            c.lock_keys_caps_enabled,
            c.lock_keys_num_enabled,
            c.lock_keys_scroll_enabled,
            c.lock_keys_skip_fullscreen,
            c.hotkeys_enabled,
            crate::system::autostart_is_enabled(),
            c.logging_enabled,
        )
    });

    for (id, value) in [
        (ID_KEEP_SCREEN, keep_screen),
        (ID_BATTERY_ALLOWED, battery_allowed),
        (ID_IDLE_ENHANCED, idle_enhanced),
        (ID_THEME_BATTERY, theme_battery),
        (ID_THEME_FULLSCREEN, theme_fullscreen),
        (ID_LOCK_KEYS, lock_keys),
        (ID_LOCK_CAPS, caps),
        (ID_LOCK_NUM, num),
        (ID_LOCK_SCROLL, scroll),
        (ID_LOCK_FULLSCREEN, lock_fullscreen),
        (ID_HOTKEYS, hotkeys),
        (ID_AUTOSTART, autostart),
        (ID_LOGGING, logging),
    ] {
        checks()
            .get_or_insert_with(Default::default)
            .insert(id, value);
    }

    set_text(hwnd, ID_BATTERY_THRESH, &battery_threshold.to_string());
    set_text(hwnd, ID_IDLE_TIMEOUT, &idle_timeout.to_string());
    set_text(hwnd, ID_WARNING_SECONDS, &warning.to_string());
    set_text(hwnd, ID_LIGHT_TIME, &light_time);
    set_text(hwnd, ID_DARK_TIME, &dark_time);

    let mode_idx = if theme_mode == "sunrise" { 1 } else { 0 };
    crate::choice::select_index(get(hwnd, ID_THEME_MODE), mode_idx);
    crate::choice::select_index(
        get(hwnd, ID_LOCATION_SOURCE),
        if ip_enabled { 1 } else { 0 },
    );
    let action_idx = IDLE_ACTIONS
        .iter()
        .position(|a| *a == idle_action)
        .unwrap_or(0);
    crate::choice::select_index(get(hwnd, ID_IDLE_ACTION), action_idx as i32);
    let lang_idx = match language.as_str() {
        "en" => 1,
        "zh-CN" => 2,
        _ => 0,
    };
    crate::choice::select_index(get(hwnd, ID_LANGUAGE), lang_idx);

    set_text(
        hwnd,
        ID_THEME_LOCATION_STATUS,
        &location_status_text(ip_enabled),
    );
}

/// Go theme_location.go status line for the day/night page.
fn location_status_text(ip_enabled: bool) -> String {
    if ip_enabled {
        if let Some((lat, lon)) = crate::iplocate::cached() {
            let label = format!("{lat:.2}, {lon:.2}");
            return t_pub("settings_location_ip_resolved").replace("%s", &label);
        }
        return t_pub("settings_location_ip_pending")
            .replace("%s", &t_pub("settings_location_auto"));
    }
    t_pub("settings_location_auto_status").replace("%s", &t_pub("settings_location_auto"))
}

/// Page membership — Go pageControlIDs().
fn page_ids(page: i32) -> &'static [i32] {
    match page {
        0 => &[
            ID_POWER_TITLE,
            ID_KEEP_SCREEN,
            ID_BATTERY_ALLOWED,
            ID_BATTERY_LBL,
            ID_BATTERY_THRESH,
            ID_POWER_HINT,
            ID_IDLE_TITLE,
            ID_IDLE_TIMEOUT_LBL,
            ID_IDLE_TIMEOUT,
            ID_IDLE_ACTION_LBL,
            ID_IDLE_ACTION,
            ID_WARNING_LBL,
            ID_WARNING_SECONDS,
            ID_IDLE_ENHANCED,
        ],
        1 => &[
            ID_THEME_SCHEDULE_TITLE,
            ID_THEME_BEHAVIOR_TITLE,
            ID_THEME_MODE_LBL,
            ID_THEME_MODE,
            ID_LIGHT_TIME_LBL,
            ID_LIGHT_TIME,
            ID_DARK_TIME_LBL,
            ID_DARK_TIME,
            ID_LOCATION_LBL,
            ID_LOCATION_SOURCE,
            ID_THEME_LOCATION_STATUS,
            ID_THEME_BATTERY,
            ID_THEME_FULLSCREEN,
            ID_THEME_HINT,
        ],
        2 => &[
            ID_APP_GENERAL_TITLE,
            ID_APP_ABOUT_TITLE,
            ID_LANGUAGE_LBL,
            ID_LANGUAGE,
            ID_HOTKEYS,
            ID_AUTOSTART,
            ID_LOGGING,
            ID_PROJECT_HOME_LBL,
            ID_PROJECT_HOME,
        ],
        _ => &[
            ID_NOTIFICATIONS_TITLE,
            ID_LOCK_KEYS,
            ID_LOCK_CAPS,
            ID_LOCK_NUM,
            ID_LOCK_SCROLL,
            ID_NOTIFICATIONS_BEHAVIOR,
            ID_LOCK_FULLSCREEN,
            ID_NOTIFICATIONS_HINT,
            ID_LOCK_PREVIEW,
        ],
    }
}

/// Visibility + enable cascades — Go applyDependentStates().
fn apply_dependent_states(hwnd: HWND) {
    unsafe {
        let page = PAGE.load(Ordering::SeqCst);
        for p in 0..4 {
            for id in page_ids(p) {
                let _ = ShowWindow(get(hwnd, *id), if p == page { SW_SHOW } else { SW_HIDE });
                // Edit surfaces hide/show together with their inner edit.
                if *id >= ID_BATTERY_THRESH && *id <= ID_DARK_TIME {
                    let _ = ShowWindow(
                        get(hwnd, FIELD_SURFACE_BASE + *id),
                        if p == page { SW_SHOW } else { SW_HIDE },
                    );
                }
            }
        }
        if page == 0 {
            let battery = is_checked(hwnd, ID_BATTERY_ALLOWED);
            let _ = EnableWindow(get(hwnd, ID_BATTERY_THRESH), battery);
            let _ = EnableWindow(get(hwnd, FIELD_SURFACE_BASE + ID_BATTERY_THRESH), battery);
        }
        if page == 1 {
            let sunrise = combo_sel(hwnd, ID_THEME_MODE) == 1;
            for id in [
                ID_LIGHT_TIME_LBL,
                ID_LIGHT_TIME,
                ID_DARK_TIME_LBL,
                ID_DARK_TIME,
            ] {
                let _ = ShowWindow(get(hwnd, id), if sunrise { SW_HIDE } else { SW_SHOW });
                // Field surfaces hide together with their inner edits.
                if id == ID_LIGHT_TIME || id == ID_DARK_TIME {
                    let _ = ShowWindow(
                        get(hwnd, FIELD_SURFACE_BASE + id),
                        if sunrise { SW_HIDE } else { SW_SHOW },
                    );
                }
            }
            for id in [
                ID_LOCATION_LBL,
                ID_LOCATION_SOURCE,
                ID_THEME_LOCATION_STATUS,
                ID_THEME_HINT,
            ] {
                let _ = ShowWindow(get(hwnd, id), if sunrise { SW_SHOW } else { SW_HIDE });
            }
        }
        let lock_keys = is_checked(hwnd, ID_LOCK_KEYS);
        for id in [
            ID_LOCK_CAPS,
            ID_LOCK_NUM,
            ID_LOCK_SCROLL,
            ID_LOCK_FULLSCREEN,
        ] {
            let _ = EnableWindow(get(hwnd, id), lock_keys);
        }
        // Repaint the tab buttons so the selected one shows the Active fill.
        for id in [ID_TAB_POWER, ID_TAB_THEME, ID_TAB_APP, ID_TAB_NOTIFICATIONS] {
            let control = get(hwnd, id);
            if !control.is_invalid() {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, false);
            }
        }
    }
}

// ---- Draft collection, validation, save, cancel --------------------------

struct Draft {
    keep_screen: bool,
    battery_allowed: bool,
    battery_threshold: Option<i32>,
    idle_timeout: Option<i32>,
    warning: Option<i32>,
    idle_action_idx: usize,
    idle_enhanced: bool,
    theme_mode_idx: usize,
    light_time: String,
    dark_time: String,
    location_idx: usize,
    theme_battery: bool,
    theme_fullscreen: bool,
    language_idx: usize,
    lock_keys: bool,
    caps: bool,
    num: bool,
    scroll: bool,
    lock_fullscreen: bool,
    hotkeys: bool,
    autostart: bool,
    logging: bool,
}

fn collect_draft(hwnd: HWND) -> Draft {
    let battery_text = control_text(hwnd, ID_BATTERY_THRESH);
    // Go: an empty threshold reads as 0 when the battery rule is off.
    let battery_threshold =
        if !is_checked(hwnd, ID_BATTERY_ALLOWED) && battery_text.trim().is_empty() {
            Some(0)
        } else {
            battery_text.trim().parse().ok()
        };
    Draft {
        keep_screen: is_checked(hwnd, ID_KEEP_SCREEN),
        battery_allowed: is_checked(hwnd, ID_BATTERY_ALLOWED),
        battery_threshold,
        idle_timeout: control_text(hwnd, ID_IDLE_TIMEOUT).trim().parse().ok(),
        warning: control_text(hwnd, ID_WARNING_SECONDS).trim().parse().ok(),
        idle_action_idx: combo_sel(hwnd, ID_IDLE_ACTION).min(IDLE_ACTIONS.len() - 1),
        idle_enhanced: is_checked(hwnd, ID_IDLE_ENHANCED),
        theme_mode_idx: combo_sel(hwnd, ID_THEME_MODE),
        light_time: control_text(hwnd, ID_LIGHT_TIME),
        dark_time: control_text(hwnd, ID_DARK_TIME),
        location_idx: combo_sel(hwnd, ID_LOCATION_SOURCE),
        theme_battery: is_checked(hwnd, ID_THEME_BATTERY),
        theme_fullscreen: is_checked(hwnd, ID_THEME_FULLSCREEN),
        language_idx: combo_sel(hwnd, ID_LANGUAGE).min(2),
        lock_keys: is_checked(hwnd, ID_LOCK_KEYS),
        caps: is_checked(hwnd, ID_LOCK_CAPS),
        num: is_checked(hwnd, ID_LOCK_NUM),
        scroll: is_checked(hwnd, ID_LOCK_SCROLL),
        lock_fullscreen: is_checked(hwnd, ID_LOCK_FULLSCREEN),
        hotkeys: is_checked(hwnd, ID_HOTKEYS),
        autostart: is_checked(hwnd, ID_AUTOSTART),
        logging: is_checked(hwnd, ID_LOGGING),
    }
}

/// Go parseSettingsDraft validation order. Returns (page, control, message).
fn validate_draft(draft: &Draft) -> Option<(i32, i32, &'static str)> {
    match draft.battery_threshold {
        Some(v) if (0..=100).contains(&v) => {}
        _ => return Some((0, ID_BATTERY_THRESH, "settings_error_battery_threshold")),
    }
    match draft.idle_timeout {
        Some(v) if (1..=7 * 24 * 60).contains(&v) => {}
        _ => return Some((0, ID_IDLE_TIMEOUT, "settings_error_idle_timeout")),
    }
    match draft.warning {
        Some(v) if (0..=3600).contains(&v) => {}
        _ => return Some((0, ID_WARNING_SECONDS, "settings_error_warning_seconds")),
    }
    if !valid_time(&draft.light_time) {
        return Some((1, ID_LIGHT_TIME, "settings_error_light_time"));
    }
    if !valid_time(&draft.dark_time) {
        return Some((1, ID_DARK_TIME, "settings_error_dark_time"));
    }
    None
}

fn valid_time(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return false;
    }
    if !bytes[..2].iter().all(u8::is_ascii_digit) || !bytes[3..].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let hour: u32 = value[..2].parse().unwrap_or(99);
    let minute: u32 = value[3..].parse().unwrap_or(99);
    hour < 24 && minute < 60
}

/// Whether the open draft differs from the live config (Go cancel() check).
fn draft_differs(hwnd: HWND, draft: &Draft) -> bool {
    let (cfg_keep, cfg_batt, cfg_thresh, cfg_timeout, cfg_action, cfg_warning, cfg_enhanced) =
        crate::cfg_map(|c| {
            (
                c.keep_screen_on,
                c.nosleep_on_battery,
                c.nosleep_battery_threshold,
                c.idle_timeout_minutes,
                c.idle_action.clone(),
                c.idle_warning_seconds,
                c.idle_enhanced_monitor,
            )
        });
    let (cfg_mode, cfg_light, cfg_dark, cfg_ip, cfg_batt_dark, cfg_fullscreen, cfg_lang) =
        crate::cfg_map(|c| {
            (
                c.theme_mode.clone(),
                c.theme_light_time.clone(),
                c.theme_dark_time.clone(),
                c.theme_ip_location_enabled,
                c.theme_dark_on_battery,
                c.theme_skip_fullscreen,
                c.language.clone(),
            )
        });
    let (cfg_lock, cfg_caps, cfg_num, cfg_scroll, cfg_lock_full, cfg_hotkeys, cfg_logging) =
        crate::cfg_map(|c| {
            (
                c.lock_keys_enabled,
                c.lock_keys_caps_enabled,
                c.lock_keys_num_enabled,
                c.lock_keys_scroll_enabled,
                c.lock_keys_skip_fullscreen,
                c.hotkeys_enabled,
                c.logging_enabled,
            )
        });

    let action_idx = IDLE_ACTIONS
        .iter()
        .position(|a| *a == cfg_action)
        .unwrap_or(0);
    let mode_idx = if cfg_mode == "sunrise" { 1 } else { 0 };
    let lang_idx = match cfg_lang.as_str() {
        "en" => 1,
        "zh-CN" => 2,
        _ => 0,
    };

    draft.keep_screen != cfg_keep
        || draft.battery_allowed != cfg_batt
        || draft.battery_threshold != Some(cfg_thresh)
        || draft.idle_timeout != Some(cfg_timeout)
        || draft.idle_action_idx != action_idx
        || draft.warning != Some(cfg_warning)
        || draft.idle_enhanced != cfg_enhanced
        || draft.theme_mode_idx != mode_idx
        || draft.light_time != cfg_light
        || draft.dark_time != cfg_dark
        || draft.location_idx != cfg_ip as usize
        || draft.theme_battery != cfg_batt_dark
        || draft.theme_fullscreen != cfg_fullscreen
        || draft.language_idx != lang_idx
        || draft.lock_keys != cfg_lock
        || draft.caps != cfg_caps
        || draft.num != cfg_num
        || draft.scroll != cfg_scroll
        || draft.lock_fullscreen != cfg_lock_full
        || draft.hotkeys != cfg_hotkeys
        || draft.autostart != crate::system::autostart_is_enabled()
        || draft.logging != cfg_logging
        || control_text(hwnd, ID_BATTERY_THRESH).trim() != cfg_thresh.to_string()
}

fn save() {
    unsafe {
        let hwnd = current();
        let draft = collect_draft(hwnd);
        if let Some((page, id, key)) = validate_draft(&draft) {
            set_text(hwnd, ID_VALIDATION, &t_pub(key));
            PAGE.store(page, Ordering::SeqCst);
            apply_dependent_states(hwnd);
            let target = get(hwnd, id);
            if !target.is_invalid() {
                let _ = SetFocus(Some(target));
            }
            return;
        }

        let hotkeys_changed = crate::cfg_map(|c| c.hotkeys_enabled) != draft.hotkeys;
        let autostart_was = crate::system::autostart_is_enabled();
        crate::cfg_edit(|c| {
            c.keep_screen_on = draft.keep_screen;
            c.nosleep_on_battery = draft.battery_allowed;
            c.nosleep_battery_threshold = draft.battery_threshold.unwrap_or(0);
            c.idle_timeout_minutes = draft.idle_timeout.unwrap_or(30);
            c.idle_action = IDLE_ACTIONS[draft.idle_action_idx].to_string();
            c.idle_warning_seconds = draft.warning.unwrap_or(30);
            c.idle_enhanced_monitor = draft.idle_enhanced;
            c.theme_mode = if draft.theme_mode_idx == 1 {
                "sunrise"
            } else {
                "fixed"
            }
            .to_string();
            c.theme_light_time = draft.light_time.clone();
            c.theme_dark_time = draft.dark_time.clone();
            c.theme_ip_location_enabled = draft.location_idx == 1;
            c.theme_dark_on_battery = draft.theme_battery;
            c.theme_skip_fullscreen = draft.theme_fullscreen;
            c.language = LANG_VALUES[draft.language_idx].to_string();
            c.lock_keys_enabled = draft.lock_keys;
            c.lock_keys_caps_enabled = draft.caps;
            c.lock_keys_num_enabled = draft.num;
            c.lock_keys_scroll_enabled = draft.scroll;
            c.lock_keys_skip_fullscreen = draft.lock_fullscreen;
            c.hotkeys_enabled = draft.hotkeys;
            c.logging_enabled = draft.logging;
        });
        crate::log_line("settings changed");
        crate::persist_config();
        crate::apply_stay_awake();
        crate::apply_language(&crate::cfg_map(|c| c.language.clone()));
        crate::refresh_checkboxes();
        crate::refresh_status();
        crate::theme::apply_to_all();

        if hotkeys_changed {
            crate::system::unregister_all();
            if draft.hotkeys {
                let failed = crate::system::register_all();
                if !failed.is_empty() {
                    crate::log_line(&format!("hotkeys failed: {}", failed.join(", ")));
                }
            }
        }
        if draft.autostart != autostart_was {
            if draft.autostart {
                let exe = std::env::current_exe()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                if crate::system::autostart_enable(&exe) {
                    crate::log_line("autostart enabled from settings");
                }
            } else if crate::system::autostart_disable() {
                crate::log_line("autostart disabled from settings");
            }
        }
        let _ = DestroyWindow(hwnd);
    }
}

fn close_request() {
    unsafe {
        let hwnd = current();
        if draft_differs(hwnd, &collect_draft(hwnd)) {
            // Go settingspanel confirm: Yes/No + warning + default No.
            let choice = MessageBoxW(
                Some(hwnd),
                PCWSTR(wide(&t_pub("settings_discard_confirm")).as_ptr()),
                PCWSTR(wide(&t_pub("settings_discard_title")).as_ptr()),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            );
            if choice != IDYES {
                return;
            }
        }
        let _ = DestroyWindow(hwnd);
    }
}

unsafe fn open_project_home(hwnd: HWND) {
    unsafe {
        let _ = windows::Win32::UI::Shell::ShellExecuteW(
            Some(hwnd),
            windows::core::w!("open"),
            PCWSTR(wide(PROJECT_URL).as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

unsafe fn create_tooltip(hwnd: HWND) {
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
            Some(hwnd),
            None,
            Some(GetModuleHandleW(None).unwrap_or_default().into()),
            None,
        )
        .unwrap_or_default();
        if tip.is_invalid() {
            return;
        }
        TOOLTIP_HWND.store(tip.0 as isize, Ordering::SeqCst);
        let _ = SendMessageW(
            tip,
            TTM_SETMAXTIPWIDTH,
            Some(WPARAM(0)),
            Some(LPARAM(s(380) as isize)),
        );
        // (control id, i18n key) pairs — Go settingsTooltipBindings().
        let tools: &[(i32, &str)] = &[
            (ID_KEEP_SCREEN, "tip_keep_screen"),
            (ID_BATTERY_ALLOWED, "tip_nosleep_battery"),
            (ID_BATTERY_LBL, "tip_nosleep_battery_threshold"),
            (ID_BATTERY_THRESH, "tip_nosleep_battery_threshold"),
            (ID_IDLE_TIMEOUT_LBL, "tip_idle_timeout"),
            (ID_IDLE_TIMEOUT, "tip_idle_timeout"),
            (ID_IDLE_ACTION_LBL, "tip_idle_action"),
            (ID_IDLE_ACTION, "tip_idle_action"),
            (ID_WARNING_LBL, "tip_idle_warning_seconds"),
            (ID_WARNING_SECONDS, "tip_idle_warning_seconds"),
            (ID_IDLE_ENHANCED, "tip_idle_enhanced"),
            (ID_THEME_MODE_LBL, "tip_theme_mode"),
            (ID_THEME_MODE, "tip_theme_mode"),
            (ID_LIGHT_TIME_LBL, "tip_theme_light_time"),
            (ID_LIGHT_TIME, "tip_theme_light_time"),
            (ID_DARK_TIME_LBL, "tip_theme_dark_time"),
            (ID_DARK_TIME, "tip_theme_dark_time"),
            (ID_LOCATION_LBL, "tip_theme_location_source"),
            (ID_LOCATION_SOURCE, "tip_theme_location_source"),
            (ID_THEME_LOCATION_STATUS, "tip_theme_location_status"),
            (ID_THEME_BATTERY, "tip_battery_theme"),
            (ID_THEME_FULLSCREEN, "tip_fullscreen"),
            (ID_LANGUAGE_LBL, "tip_language"),
            (ID_LANGUAGE, "tip_language"),
            (ID_LOCK_KEYS, "tip_lock_keys"),
            (ID_LOCK_CAPS, "tip_lock_key_selection"),
            (ID_LOCK_NUM, "tip_lock_key_selection"),
            (ID_LOCK_SCROLL, "tip_lock_key_selection"),
            (ID_LOCK_FULLSCREEN, "tip_notification_fullscreen"),
            (ID_LOCK_PREVIEW, "tip_notification_preview"),
            (ID_HOTKEYS, "tip_hotkeys"),
            (ID_AUTOSTART, "tip_autostart"),
            (ID_LOGGING, "tip_logging"),
            (ID_PROJECT_HOME, "tip_project_home"),
            (ID_CANCEL, "tip_settings_cancel"),
            (ID_SAVE, "tip_settings_save"),
        ];
        for (id, key) in tools {
            let target = get(hwnd, *id);
            if target.is_invalid() {
                continue;
            }
            let text = wide(&t_pub(key));
            let mut tool = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                uFlags: TTF_IDISHWND | TTF_SUBCLASS,
                hwnd,
                uId: target.0 as usize,
                ..Default::default()
            };
            tool.lpszText = windows::core::PWSTR(text.as_ptr() as *mut _);
            let _ = SendMessageW(
                tip,
                TTM_ADDTOOLW,
                Some(WPARAM(0)),
                Some(LPARAM(&tool as *const _ as isize)),
            );
        }
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                let idc = (wparam.0 & 0xFFFF) as i32;
                let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                match code {
                    EN_CHANGE => {
                        set_text(hwnd, ID_VALIDATION, "");
                        LRESULT(0)
                    }
                    // Repaint the field surface so the border follows focus
                    // (Go ControlSurface focus tracking).
                    EN_SETFOCUS | EN_KILLFOCUS => {
                        if let Some(surface) = field_surface_of(idc) {
                            let control = get(hwnd, surface);
                            if !control.is_invalid() {
                                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(
                                    Some(control),
                                    None,
                                    false,
                                );
                            }
                        }
                        LRESULT(0)
                    }
                    CBN_SELCHANGE if idc == ID_THEME_MODE => {
                        apply_dependent_states(hwnd);
                        LRESULT(0)
                    }
                    BN_CLICKED => {
                        handle_click(hwnd, idc);
                        LRESULT(0)
                    }
                    _ => DefWindowProcW(hwnd, msg, wparam, lparam),
                }
            }
            WM_DRAWITEM => {
                if let Some(item) = crate::nativeform::draw_item(lparam) {
                    draw_settings_item(hwnd, &item);
                    LRESULT(1)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_CLOSE => {
                close_request();
                LRESULT(0)
            }
            WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
                refresh_theme(hwnd);
                LRESULT(0)
            }
            WM_SETCURSOR => {
                // Hand cursor over the project-home link (Go wmSetCursor).
                let target = HWND(wparam.0 as *mut _);
                if target == get(hwnd, ID_PROJECT_HOME)
                    && let Ok(hand) = LoadCursorW(None, IDC_HAND)
                {
                    let _ = SetCursor(Some(hand));
                    return LRESULT(1);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_ERASEBKGND => {
                crate::popups::erase_theme_bg(hwnd, wparam);
                LRESULT(1)
            }
            WM_CTLCOLOREDIT => {
                // Edit interiors paint on the palette surface, not the
                // default white (Go surfaceBrush path).
                let p = theme::palette();
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                let child = HWND(lparam.0 as *mut _);
                let disabled = !IsWindowEnabled(child).as_bool();
                let (fill, color) = if disabled {
                    (p.disabled_surface, p.disabled_text)
                } else {
                    (p.surface, p.text)
                };
                let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                    hdc,
                    windows::Win32::Foundation::COLORREF(color),
                );
                let _ = windows::Win32::Graphics::Gdi::SetBkColor(
                    hdc,
                    windows::Win32::Foundation::COLORREF(fill),
                );
                let (light, dark) = theme::surface_brush_pairs();
                let pair = if theme::is_dark() { dark } else { light };
                let brush = if disabled { pair.1 } else { pair.0 };
                LRESULT(brush.0 as isize)
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                let child_hwnd = HWND(lparam.0 as *mut _);
                // Go wmCtlColorStatic: section titles get PrimaryText, muted
                // labels (version/hints/status) get MutedText, everything
                // else gets SecondaryText; disabled controls get DisabledText.
                if msg == WM_CTLCOLORSTATIC {
                    // Disabled edit interiors paint on the disabled surface,
                    // not the window background — this check MUST come before
                    // the text tier logic (Go disabledBrush path).
                    if !IsWindowEnabled(child_hwnd).as_bool() {
                        let mut buffer = [0u16; 8];
                        let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameW(
                            child_hwnd,
                            &mut buffer,
                        );
                        let class =
                            String::from_utf16_lossy(&buffer[..len.max(0) as usize]).to_uppercase();
                        if class == "EDIT" {
                            let palette = theme::palette();
                            let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                                hdc,
                                windows::Win32::Foundation::COLORREF(palette.disabled_text),
                            );
                            let _ = windows::Win32::Graphics::Gdi::SetBkColor(
                                hdc,
                                windows::Win32::Foundation::COLORREF(palette.disabled_surface),
                            );
                            let (light, dark) = theme::surface_brush_pairs();
                            let pair = if theme::is_dark() { dark } else { light };
                            return LRESULT(pair.1.0 as isize);
                        }
                    }
                    let palette = theme::palette();
                    let id = GetWindowLongPtrW(child_hwnd, GWL_ID) as i32;
                    let is_section = matches!(
                        id,
                        ID_TITLE
                            | ID_POWER_TITLE
                            | ID_IDLE_TITLE
                            | ID_THEME_SCHEDULE_TITLE
                            | ID_THEME_BEHAVIOR_TITLE
                            | ID_APP_GENERAL_TITLE
                            | ID_APP_ABOUT_TITLE
                            | ID_NOTIFICATIONS_TITLE
                            | ID_NOTIFICATIONS_BEHAVIOR
                    );
                    let is_muted = matches!(
                        id,
                        ID_DESCRIPTION
                            | ID_VERSION
                            | ID_POWER_HINT
                            | ID_THEME_HINT
                            | ID_THEME_LOCATION_STATUS
                            | ID_VALIDATION
                            | ID_NOTIFICATIONS_HINT
                    );
                    let disabled = !IsWindowEnabled(child_hwnd).as_bool();
                    let text = if disabled {
                        palette.disabled_text
                    } else if is_section {
                        palette.text
                    } else if is_muted {
                        palette.muted
                    } else {
                        palette.text2
                    };
                    let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                        hdc,
                        windows::Win32::Foundation::COLORREF(text),
                    );
                    let _ = windows::Win32::Graphics::Gdi::SetBkColor(
                        hdc,
                        windows::Win32::Foundation::COLORREF(theme::bg_color()),
                    );
                    return LRESULT(theme::bg_brush().0 as isize);
                }
                // Disabled edit interiors paint on the disabled surface, not
                // the window background (Go disabledBrush path).
                if !IsWindowEnabled(child_hwnd).as_bool() && msg == WM_CTLCOLORSTATIC {
                    let mut buffer = [0u16; 8];
                    let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameW(
                        child_hwnd,
                        &mut buffer,
                    );
                    let class =
                        String::from_utf16_lossy(&buffer[..len.max(0) as usize]).to_uppercase();
                    if class == "EDIT" {
                        let palette = theme::palette();
                        let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                            hdc,
                            windows::Win32::Foundation::COLORREF(palette.disabled_text),
                        );
                        let _ = windows::Win32::Graphics::Gdi::SetBkColor(
                            hdc,
                            windows::Win32::Foundation::COLORREF(palette.disabled_surface),
                        );
                        let (light, dark) = theme::surface_brush_pairs();
                        let pair = if theme::is_dark() { dark } else { light };
                        return LRESULT(pair.1.0 as isize);
                    }
                }
                let validation = get(hwnd, ID_VALIDATION);
                let is_error = child_hwnd == validation && GetWindowTextLengthW(validation) > 0;
                let color = if is_error {
                    // Validation messages render in the error color.
                    0x001C_2BC8
                } else {
                    theme::text_color()
                };
                let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                    hdc,
                    windows::Win32::Foundation::COLORREF(color),
                );
                let _ = windows::Win32::Graphics::Gdi::SetBkColor(
                    hdc,
                    windows::Win32::Foundation::COLORREF(theme::bg_color()),
                );
                LRESULT(theme::bg_brush().0 as isize)
            }
            WM_DPICHANGED => {
                // Rebuild at the new scale on next open.
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                SETTINGS_HWND.store(0, Ordering::SeqCst);
                TOOLTIP_HWND.store(0, Ordering::SeqCst);
                PAGE.store(0, Ordering::SeqCst);
                for slot in [&FONT_BODY, &FONT_SECTION, &FONT_TITLE, &FONT_LINK] {
                    let font = HFONT(slot.load(Ordering::SeqCst) as *mut _);
                    if !font.is_invalid() {
                        let _ = windows::Win32::Graphics::Gdi::DeleteObject(
                            windows::Win32::Graphics::Gdi::HGDIOBJ(font.0),
                        );
                    }
                    slot.store(0, Ordering::SeqCst);
                }
                let _ = EnableWindow(crate::hwnd(&crate::PANEL), true);
                crate::refresh_status();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Surface static id for an edit id, when one exists.
fn field_surface_of(edit_id: i32) -> Option<i32> {
    if matches!(
        edit_id,
        ID_BATTERY_THRESH | ID_IDLE_TIMEOUT | ID_WARNING_SECONDS | ID_LIGHT_TIME | ID_DARK_TIME
    ) {
        Some(FIELD_SURFACE_BASE + edit_id)
    } else {
        None
    }
}

/// WM_DRAWITEM dispatcher (Go settingspanel window.go drawItem).
fn draw_settings_item(hwnd: HWND, item: &crate::nativeform::DrawItem) {
    // Buffered blit first (Go drawItemBuffered); fall back to direct paint.
    unsafe {
        if crate::nativeform::draw_buffered(item.dc, &item.bounds, |dc, bounds| {
            draw_settings_item_impl(hwnd, item, dc, bounds);
        }) {
            return;
        }
        draw_settings_item_impl(hwnd, item, item.dc, &item.bounds);
    }
}

fn draw_settings_item_impl(hwnd: HWND, item: &crate::nativeform::DrawItem, dc: HDC, bounds: &RECT) {
    let p = theme::palette();
    let scale = crate::scale_pub(96);
    let id = item.control_id;
    let label = unsafe {
        let len = GetWindowTextLengthW(item.control);
        if len <= 0 {
            String::new()
        } else {
            let mut buf = vec![0u16; len as usize + 1];
            let copied = GetWindowTextW(item.control, &mut buf);
            String::from_utf16_lossy(&buf[..copied.max(0) as usize])
        }
    };
    unsafe {
        if id >= FIELD_SURFACE_BASE {
            // Edit field surface: focus/hover border via draw_field.
            let edit = get(hwnd, id - FIELD_SURFACE_BASE);
            let mut state = crate::nativeform::control_state(item.control, item.state);
            state.focused = GetFocus() == edit;
            state.disabled = !IsWindowEnabled(edit).as_bool();
            crate::paint::draw_field(dc, bounds, p, p.window_bg, state, 6);
        } else if id == ID_PROJECT_HOME {
            let state = crate::nativeform::control_state(item.control, item.state);
            crate::paint::draw_text_link(
                dc,
                bounds,
                body_font(),
                &label,
                p,
                p.window_bg,
                state,
                scale,
            );
        } else if crate::choice::is_choice(item.control) {
            let state = crate::nativeform::control_state(item.control, item.state);
            crate::choice::draw_button(item.control, dc, bounds, state);
        } else if matches!(
            id,
            ID_TAB_POWER | ID_TAB_THEME | ID_TAB_APP | ID_TAB_NOTIFICATIONS
        ) {
            let mut state = crate::nativeform::control_state(item.control, item.state);
            let page = PAGE.load(Ordering::SeqCst);
            state.active =
                id == [ID_TAB_POWER, ID_TAB_THEME, ID_TAB_APP, ID_TAB_NOTIFICATIONS][page as usize];
            crate::paint::draw_button(dc, bounds, body_font(), &label, p, p.window_bg, state, 6);
        } else if id == ID_SAVE {
            // Save is the form's default action: accent fill (Go state.Active).
            let mut state = crate::nativeform::control_state(item.control, item.state);
            state.active = true;
            crate::paint::draw_button(dc, bounds, body_font(), &label, p, p.window_bg, state, 6);
        } else if checks().as_ref().is_some_and(|m| m.contains_key(&id)) {
            let mut state = crate::nativeform::control_state(item.control, item.state);
            state.active = is_checked(hwnd, id);
            crate::paint::draw_checkbox(
                dc,
                bounds,
                body_font(),
                &label,
                p,
                p.window_bg,
                state,
                scale,
                16,
            );
        } else {
            let state = crate::nativeform::control_state(item.control, item.state);
            crate::paint::draw_button(dc, bounds, body_font(), &label, p, p.window_bg, state, 6);
        }
    }
}

fn handle_click(hwnd: HWND, idc: i32) {
    unsafe {
        match idc {
            ID_TAB_POWER | ID_TAB_THEME | ID_TAB_APP | ID_TAB_NOTIFICATIONS => {
                let page = match idc {
                    ID_TAB_POWER => 0,
                    ID_TAB_THEME => 1,
                    ID_TAB_APP => 2,
                    _ => 3,
                };
                PAGE.store(page, Ordering::SeqCst);
                apply_dependent_states(hwnd);
            }
            ID_KEEP_SCREEN | ID_BATTERY_ALLOWED | ID_IDLE_ENHANCED | ID_THEME_BATTERY
            | ID_THEME_FULLSCREEN | ID_HOTKEYS | ID_AUTOSTART | ID_LOGGING | ID_LOCK_KEYS
            | ID_LOCK_CAPS | ID_LOCK_NUM | ID_LOCK_SCROLL | ID_LOCK_FULLSCREEN => {
                // Owner-drawn checkboxes keep state in CHECKS.
                toggle_checked(hwnd, idc);
                apply_dependent_states(hwnd);
                set_text(hwnd, ID_VALIDATION, "");
            }
            ID_THEME_MODE | ID_LOCATION_SOURCE | ID_IDLE_ACTION | ID_LANGUAGE => {
                crate::choice::toggle(get(hwnd, idc), hwnd, idc);
            }
            ID_LOCK_PREVIEW => crate::popups::show(crate::popups::VK_CAPITAL, true),
            ID_PROJECT_HOME => open_project_home(hwnd),
            ID_SAVE => save(),
            ID_CANCEL => close_request(),
            _ => {}
        }
    }
}

/// Center on the owner, then clamp into its monitor work area (Go
/// PlaceWindow: CenteredRect + ConstrainRect).
fn center_on_parent(win_w: i32, win_h: i32) -> (i32, i32) {
    unsafe {
        let parent = crate::hwnd(&crate::PANEL);
        let mut center = RECT {
            left: 0,
            top: 0,
            right: GetSystemMetrics(SM_CXSCREEN),
            bottom: GetSystemMetrics(SM_CYSCREEN),
        };
        let mut wr = RECT::default();
        if GetWindowRect(parent, &mut wr).is_ok() && wr.right > wr.left && wr.bottom > wr.top {
            center = wr;
        }
        let mut x = center.left + (center.right - center.left - win_w) / 2;
        let mut y = center.top + (center.bottom - center.top - win_h) / 2;
        let work = crate::display::work_area_for(parent);
        let work_w = work.right - work.left;
        let work_h = work.bottom - work.top;
        if win_w >= work_w {
            x = work.left;
        } else {
            x = x.clamp(work.left, work.right - win_w);
        }
        if win_h >= work_h {
            y = work.top;
        } else {
            y = y.clamp(work.top, work.bottom - win_h);
        }
        (x, y)
    }
}
