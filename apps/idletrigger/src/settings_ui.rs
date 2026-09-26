//! Settings window with four left navigation tabs, per-page content on the
//! right, and a footer with inline validation, Save, and Cancel.
//! Layout uses logical coordinates and adapts to DPI and text scaling.

use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

// Edit interior brushes (surface / disabled surface), live for the process.
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{HDC, HFONT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::EM_SETLIMITTEXT;
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
const ID_PAUSE_ON_LOCK: i32 = 177;
// Appearance page: paired light/dark columns over a wallpaper library.
// Labels and combos each own a unique id — sharing ids makes GetDlgItem
// resolve only the first control and leaks the second across pages.
const ID_APPEARANCE_TITLE: i32 = 178;
const ID_ROW_WALL_LBL: i32 = 179;
const ID_LIGHT_WALL: i32 = 180;
const ID_DARK_WALL: i32 = 183;
const ID_ROW_CURSOR_LBL: i32 = 185;
const ID_LIGHT_CURSOR: i32 = 186;
const ID_DARK_CURSOR: i32 = 188;
const ID_TAB_APPEARANCE: i32 = 191;
const ID_APPEARANCE_HINT: i32 = 192;
const ID_CURSOR_INSTALL: i32 = 197;
const ID_COL_LIGHT: i32 = 198;
const ID_COL_DARK: i32 = 199;

/// Session state for the wallpaper library: seeded from the config when the
/// settings window opens, mutated by Add/Remove, committed on Save.
static WALLPAPER_LIBRARY: std::sync::LazyLock<std::sync::Mutex<Vec<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

// Layout tokens — Go controls.go build() constants.
const CLIENT_W: i32 = 700;
const CLIENT_H: i32 = 580;
const CONTENT_X: i32 = 208;
const CONTENT_RIGHT: i32 = 676;
const SECTION_TOP: i32 = 90;
const SECTION_TITLE_H: i32 = 20;
const SECTION_ITEM_GAP: i32 = 10;
const FUNCTION_GAP: i32 = 8;
const SECTION_GAP: i32 = 16;
const CHECK_H: i32 = 28;
const BTN_H: i32 = 36;
// Two-line validation row above the footer buttons: the English conflict
// message measures ~491px and Go's inline 236px label clipped it mid-sentence.
const VALIDATION_H: i32 = 48;
const FIELD_H: i32 = 34;
const DIALOG_BTN_W: i32 = 104;
const FOOTER_Y: i32 = CLIENT_H - 18 - BTN_H;

const IDLE_ACTIONS: [&str; 6] = [
    "lock",
    "sleep",
    "hibernate",
    "shutdown",
    "restart",
    "screen_off",
];
const LANG_VALUES: [&str; 3] = ["auto", "en", "zh-CN"];
const PROJECT_URL: &str = "https://github.com/JeffioZ/IdleTrigger";

// Edit surfaces use the Go idFieldSurfaceBase offset from the inner edit id.
const FIELD_SURFACE_BASE: i32 = 500;

// Owner-drawn checkbox states (BM_SETCHECK is inert on owner-draw buttons).
static CHECKS: std::sync::Mutex<Option<std::collections::HashMap<i32, bool>>> =
    std::sync::Mutex::new(None);

fn checks() -> std::sync::MutexGuard<'static, Option<std::collections::HashMap<i32, bool>>> {
    crate::runtime::lock(&CHECKS)
}

static SETTINGS_HWND: AtomicIsize = AtomicIsize::new(0);
static DRAFT_BASE: std::sync::Mutex<Option<idletrigger_core::config::Config>> =
    std::sync::Mutex::new(None);
static PAGE: AtomicI32 = AtomicI32::new(0);
static AUTOSTART_BASE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static TOOLTIP_HWND: AtomicIsize = AtomicIsize::new(0);

fn s(v: i32) -> i32 {
    crate::scale_pub(v)
}

use crate::wide;

fn current() -> HWND {
    HWND(SETTINGS_HWND.load(Ordering::SeqCst) as *mut _)
}

fn get(parent: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(parent), id).unwrap_or_default() }
}

fn set_text(parent: HWND, id: i32, text: &str) {
    unsafe {
        let target = get(parent, id);
        if !target.is_invalid() && control_text(parent, id) != text {
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
            crate::accessibility::check(control, value);
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
    let _dpi = crate::dpi::Scope::window(crate::hwnd(&crate::PANEL));
    unsafe {
        if current().is_invalid() {
            create();
        }
        let hwnd = current();
        if hwnd.is_invalid() {
            return;
        }
        request_location_preview(hwnd);
        let _ = EnableWindow(crate::hwnd(&crate::PANEL), false);
        crate::FirstFrameGate::begin(hwnd).reveal();
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

pub fn default_button() -> HWND {
    get(theme_hwnd(), ID_SAVE)
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
        crate::dpi::install(hwnd);
        theme::apply_to_window(hwnd);
        crate::set_window_icons_pub(hwnd);

        build_controls(hwnd, body, section, title_font);
        populate(hwnd);
        PAGE.store(0, Ordering::SeqCst);
        apply_dependent_states(hwnd);
        theme::retheme_children(hwnd);
        create_tooltip(hwnd);
        retheme_tooltip();
        crate::viewport::fit(hwnd);
    }
}

/// Re-themes the tooltip colors after a light/dark switch (Go ApplyTooltip).
pub(crate) fn retheme_tooltip() {
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
fn refresh_theme() {
    crate::request_theme_refresh();
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
            ID_TAB_APPEARANCE,
            &t_pub("settings_tab_appearance"),
            (24, 178, 156, BTN_H),
        );
        tab_button(
            hwnd,
            ID_TAB_NOTIFICATIONS,
            &t_pub("settings_tab_notifications"),
            (24, 222, 156, BTN_H),
        );
        tab_button(
            hwnd,
            ID_TAB_APP,
            &t_pub("settings_tab_app"),
            (24, 266, 156, BTN_H),
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
            (CONTENT_X, 156, 224, CHECK_H),
        );
        checkbox(
            hwnd,
            ID_PAUSE_ON_LOCK,
            &t_pub("settings_pause_on_lock"),
            (CONTENT_X + 240, 156, 228, CHECK_H),
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

        // Appearance page (its own tab): paired light/dark columns for
        // wallpaper and cursor schemes, over a shared wallpaper library.
        // Layout grid: row labels 208..272, light column 280..470, dark
        // column 478..668.
        label(
            hwnd,
            ID_APPEARANCE_TITLE,
            &t_pub("settings_theme_appearance_group"),
            section_font,
            (CONTENT_X, SECTION_TOP, 468, SECTION_TITLE_H),
            false,
        );
        label(
            hwnd,
            ID_APPEARANCE_HINT,
            &t_pub("settings_appearance_hint"),
            font,
            (CONTENT_X, SECTION_TOP + SECTION_TITLE_H + 10, 468, 40),
            false,
        );
        // Column headers above the paired picks.
        label(
            hwnd,
            ID_COL_LIGHT,
            &t_pub("settings_light_side"),
            section_font,
            (280, 172, 190, 22),
            false,
        );
        label(
            hwnd,
            ID_COL_DARK,
            &t_pub("settings_dark_side"),
            section_font,
            (478, 172, 190, 22),
            false,
        );
        // Wallpaper row: pick from the library per side.
        label(
            hwnd,
            ID_ROW_WALL_LBL,
            &t_pub("settings_row_wallpaper"),
            font,
            (CONTENT_X, 206, 64, 22),
            false,
        );
        for (id, x) in [(ID_LIGHT_WALL, 280), (ID_DARK_WALL, 478)] {
            combo_items(hwnd, id, (x, 198, 190, FIELD_H), &wallpaper_pick_items());
        }
        // Cursor row: installed schemes per side.
        label(
            hwnd,
            ID_ROW_CURSOR_LBL,
            &t_pub("settings_row_cursor"),
            font,
            (CONTENT_X, 252, 64, 22),
            false,
        );
        for (id, x) in [(ID_LIGHT_CURSOR, 280), (ID_DARK_CURSOR, 478)] {
            combo(hwnd, id, (x, 244, 190, FIELD_H), &cursor_choice_labels());
        }
        // One installer for both cursor columns; the dropdowns refresh on
        // open, so the freshly installed scheme shows up right away.
        push_button(
            hwnd,
            ID_CURSOR_INSTALL,
            &t_pub("settings_install_inf"),
            (CONTENT_X, 298, 224, FIELD_H),
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

        // Footer: full-width two-line validation above the buttons (Go kept
        // it inline next to them and truncated long bilingual messages).
        label(
            hwnd,
            ID_VALIDATION,
            "",
            font,
            (CONTENT_X, FOOTER_Y - VALIDATION_H - 8, 468, VALIDATION_H),
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
    let extra = 0x0080 | if right { 0x4002 } else { 0 }; // SS_NOPREFIX, SS_RIGHT | SS_ENDELLIPSIS
    unsafe {
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

/// Choice creation from full row items (headers allowed).
fn combo_items(parent: HWND, id: i32, b: (i32, i32, i32, i32), rows: &[crate::choice::ChoiceItem]) {
    let (x, y, w, _) = b;
    crate::choice::create_rows(parent, id, (x, y, w, FIELD_H), rows, body_font());
}

/// Cursor dropdown rows: a leading "no linkage" entry plus installed schemes.
fn cursor_choice_labels() -> Vec<String> {
    let mut items = vec![t_pub("settings_appearance_none")];
    items.extend(crate::theme_engine::cursor_schemes());
    items
}

/// Picker rows for one wallpaper side: "No change", the recently used
/// images (file names), and a trailing Browse action that opens the picker.
pub fn wallpaper_pick_items() -> Vec<crate::choice::ChoiceItem> {
    let none = t_pub("settings_appearance_none");
    let mut rows = vec![crate::choice::ChoiceItem::option("", &none)];
    let library = crate::runtime::lock(&WALLPAPER_LIBRARY).clone();
    for path in &library {
        let label = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        // Duplicate file names fall back to the full path so rows stay
        // distinguishable.
        let label = if library
            .iter()
            .filter(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy() == label)
            })
            .count()
            > 1
        {
            path.clone()
        } else {
            label
        };
        rows.push(crate::choice::ChoiceItem::option(path, &label));
    }
    rows.push(crate::choice::ChoiceItem::option(
        "__browse__",
        &t_pub("settings_wall_browse"),
    ));
    rows
}

/// Records a wallpaper as recently used: deduplicated, most recent last,
/// capped so the dropdown stays scannable.
fn remember_wallpaper(path: &str) {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return;
    }
    let mut library = crate::runtime::lock(&WALLPAPER_LIBRARY);
    library.retain(|p| !p.eq_ignore_ascii_case(trimmed));
    library.push(trimmed.to_string());
    let len = library.len();
    if len > 8 {
        library.drain(..len - 8);
    }
}

/// Replaces the rows of one choice, keeping the selection by value.
fn refresh_choice_rows(hwnd: HWND, id: i32, rows: &[crate::choice::ChoiceItem]) {
    let selected = crate::choice::value(get(hwnd, id));
    let index = rows
        .iter()
        .position(|r| !r.header && r.value == selected)
        .map(|i| i as i32)
        .unwrap_or(0);
    crate::choice::set_rows(get(hwnd, id), rows);
    crate::choice::select_index(get(hwnd, id), index);
}

/// Reloads both cursor dropdowns, keeping current selections by label.
fn refresh_cursor_choices(hwnd: HWND) {
    let rows: Vec<crate::choice::ChoiceItem> = cursor_choice_labels()
        .iter()
        .map(|label| crate::choice::ChoiceItem::option(label, label))
        .collect();
    refresh_choice_rows(hwnd, ID_LIGHT_CURSOR, &rows);
    refresh_choice_rows(hwnd, ID_DARK_CURSOR, &rows);
}

/// Reloads both wallpaper dropdowns from the recent list.
fn refresh_wallpaper_choices(hwnd: HWND) {
    let picks = wallpaper_pick_items();
    refresh_choice_rows(hwnd, ID_LIGHT_WALL, &picks);
    refresh_choice_rows(hwnd, ID_DARK_WALL, &picks);
}

/// The Browse footer row was picked on one side: open the picker, remember
/// the file, and select it on that side.
fn browse_wallpaper_for_side(hwnd: HWND, id: i32) {
    let Some(path) = browse_wallpaper_file(hwnd) else {
        // Keep the previous selection when the dialog is cancelled.
        refresh_wallpaper_choices(hwnd);
        return;
    };
    remember_wallpaper(&path);
    refresh_wallpaper_choices(hwnd);
    let rows = wallpaper_pick_items();
    let index = rows
        .iter()
        .position(|r| r.value.eq_ignore_ascii_case(&path))
        .map(|i| i as i32)
        .unwrap_or(0);
    crate::choice::select_index(get(hwnd, id), index);
}

/// Wallpaper picker: the formats Windows accepts as desktop backgrounds,
/// plus an all-files fallback. Returns the chosen path.
/// Installs a pointer-scheme .inf through the system installer (the
/// context-menu "Install" verb). The scheme appears in the dropdowns when
/// they next open; we never parse or execute inf content ourselves.
fn install_cursor_inf(owner: HWND) {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OPENFILENAMEW};
    unsafe {
        let mut filter: Vec<u16> = t_pub("settings_inf_filter").encode_utf16().collect();
        filter.push(0);
        filter.extend("*.inf".encode_utf16());
        filter.push(0);
        filter.push(0);
        let mut file = vec![0u16; 32768];
        let title = wide(&t_pub("settings_inf_browse_title"));
        let mut dialog = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: windows::core::PCWSTR(filter.as_ptr()),
            nFilterIndex: 1,
            lpstrFile: windows::core::PWSTR(file.as_mut_ptr()),
            nMaxFile: file.len() as u32,
            lpstrTitle: windows::core::PCWSTR(title.as_ptr()),
            Flags: windows::Win32::UI::Controls::Dialogs::OPEN_FILENAME_FLAGS(
                0x0000_0004 // OFN_HIDEREADONLY
                    | 0x0000_0008 // OFN_NOCHANGEDIR
                    | 0x0000_0800 // OFN_PATHMUSTEXIST
                    | 0x0000_1000 // OFN_FILEMUSTEXIST
                    | 0x0008_0000 // OFN_EXPLORER
                    | 0x0200_0000, // OFN_DONTADDTORECENT
            ),
            ..Default::default()
        };
        if !GetOpenFileNameW(&mut dialog).as_bool() {
            return;
        }
        let end = file.iter().position(|c| *c == 0).unwrap_or(file.len());
        let path = String::from_utf16_lossy(&file[..end]);
        // The system handles elevation prompts and file copy itself.
        let wide_path = wide(&path);
        let _ = windows::Win32::UI::Shell::ShellExecuteW(
            Some(owner),
            windows::core::w!("install"),
            PCWSTR(wide_path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        refresh_cursor_choices(owner);
    }
}

fn browse_wallpaper_file(owner: HWND) -> Option<String> {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OPENFILENAMEW};
    unsafe {
        let mut filter: Vec<u16> = t_pub("settings_wallpaper_filter").encode_utf16().collect();
        filter.push(0);
        filter.extend("*.jpg;*.jpeg;*.png;*.bmp;*.gif;*.tif;*.tiff;*.webp;*.heic".encode_utf16());
        filter.push(0);
        let all = t_pub("settings_all_files");
        filter.extend(all.encode_utf16());
        filter.push(0);
        filter.extend("*.*".encode_utf16());
        filter.push(0);
        filter.push(0);
        let mut file = vec![0u16; 32768];
        let title = wide(&t_pub("settings_wallpaper_browse_title"));
        let mut dialog = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: windows::core::PCWSTR(filter.as_ptr()),
            nFilterIndex: 1,
            lpstrFile: windows::core::PWSTR(file.as_mut_ptr()),
            nMaxFile: file.len() as u32,
            lpstrTitle: windows::core::PCWSTR(title.as_ptr()),
            Flags: windows::Win32::UI::Controls::Dialogs::OPEN_FILENAME_FLAGS(
                0x0000_0004 // OFN_HIDEREADONLY
                    | 0x0000_0008 // OFN_NOCHANGEDIR
                    | 0x0000_0800 // OFN_PATHMUSTEXIST
                    | 0x0000_1000 // OFN_FILEMUSTEXIST
                    | 0x0008_0000 // OFN_EXPLORER
                    | 0x0200_0000, // OFN_DONTADDTORECENT
            ),
            ..Default::default()
        };
        if !GetOpenFileNameW(&mut dialog).as_bool() {
            return None;
        }
        let end = file.iter().position(|c| *c == 0).unwrap_or(file.len());
        Some(String::from_utf16_lossy(&file[..end]))
    }
}

fn body_font() -> HFONT {
    crate::make_font_pub(14, 400)
}

fn is_chinese() -> bool {
    crate::i18n_is_chinese()
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
        pause_on_lock,
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
        light_wallpaper,
        dark_wallpaper,
        light_cursor,
        dark_cursor,
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
        *crate::runtime::lock(&DRAFT_BASE) = Some(c.clone());
        (
            c.keep_screen_on,
            c.nosleep_on_battery,
            c.nosleep_pause_on_lock,
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
            c.theme_light_wallpaper.clone(),
            c.theme_dark_wallpaper.clone(),
            c.theme_light_cursor_scheme.clone(),
            c.theme_dark_cursor_scheme.clone(),
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

    AUTOSTART_BASE.store(autostart, Ordering::SeqCst);
    for (id, value) in [
        (ID_KEEP_SCREEN, keep_screen),
        (ID_BATTERY_ALLOWED, battery_allowed),
        (ID_PAUSE_ON_LOCK, pause_on_lock),
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
        set_checked(hwnd, id, value);
    }

    set_text(hwnd, ID_BATTERY_THRESH, &battery_threshold.to_string());
    set_text(hwnd, ID_IDLE_TIMEOUT, &idle_timeout.to_string());
    set_text(hwnd, ID_WARNING_SECONDS, &warning.to_string());
    set_text(hwnd, ID_LIGHT_TIME, &light_time);
    set_text(hwnd, ID_DARK_TIME, &dark_time);
    *crate::runtime::lock(&WALLPAPER_LIBRARY) = crate::cfg_map(|c| c.theme_wallpapers.clone());
    refresh_wallpaper_choices(hwnd);
    // Select the configured light/dark entries by their file-name label.
    let pick_label = |path: &str| {
        std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    let _ = &pick_label; // label helper retained for cursor rows if needed
    for (id, path) in [
        (ID_LIGHT_WALL, &light_wallpaper),
        (ID_DARK_WALL, &dark_wallpaper),
    ] {
        // Rows carry the path as the value; missing files fall back to
        // no-change (the file may be on an unplugged drive).
        let index = wallpaper_pick_items()
            .iter()
            .position(|r| r.value == *path && std::path::Path::new(path).is_file())
            .map(|i| i as i32)
            .unwrap_or(0);
        crate::choice::select_index(get(hwnd, id), index);
    }
    // Cursor dropdowns: 0 = no linkage, otherwise the scheme's row; a
    // scheme that no longer exists falls back to no linkage.
    let schemes = crate::theme_engine::cursor_schemes();
    let scheme_index = |name: &str| -> i32 {
        schemes
            .iter()
            .position(|s| s.eq_ignore_ascii_case(name.trim()))
            .map_or(0, |i| i as i32 + 1)
    };
    crate::choice::select_index(get(hwnd, ID_LIGHT_CURSOR), scheme_index(&light_cursor));
    crate::choice::select_index(get(hwnd, ID_DARK_CURSOR), scheme_index(&dark_cursor));

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
        let key = match crate::iplocate::status() {
            crate::iplocate::Status::Resolved(label) => {
                return t_pub("settings_location_ip_resolved").replace("%s", &label);
            }
            crate::iplocate::Status::Querying => "settings_location_ip_pending",
            crate::iplocate::Status::Failed => "settings_location_ip_failed",
            crate::iplocate::Status::NotRequested => "settings_location_ip_not_requested",
        };
        return t_pub(key).replace("%s", &t_pub(crate::theme_engine::location(false).2));
    }
    t_pub("settings_location_auto_status")
        .replace("%s", &t_pub(crate::theme_engine::location(false).2))
}

/// Preview the selected source without saving the settings draft.
fn request_location_preview(hwnd: HWND) {
    if combo_sel(hwnd, ID_LOCATION_SOURCE) == 1 {
        crate::iplocate::request();
    }
    refresh_location_status();
}

/// Refresh only the runtime label; never repopulate an open settings draft.
pub fn refresh_location_status() {
    let hwnd = current();
    if !hwnd.is_invalid() {
        set_text(
            hwnd,
            ID_THEME_LOCATION_STATUS,
            &location_status_text(combo_sel(hwnd, ID_LOCATION_SOURCE) == 1),
        );
    }
}

/// Page membership — Go pageControlIDs().
fn page_ids(page: i32) -> &'static [i32] {
    match page {
        0 => &[
            ID_POWER_TITLE,
            ID_KEEP_SCREEN,
            ID_BATTERY_ALLOWED,
            ID_PAUSE_ON_LOCK,
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
            ID_APPEARANCE_TITLE,
            ID_APPEARANCE_HINT,
            ID_COL_LIGHT,
            ID_COL_DARK,
            ID_ROW_WALL_LBL,
            ID_LIGHT_WALL,
            ID_DARK_WALL,
            ID_ROW_CURSOR_LBL,
            ID_LIGHT_CURSOR,
            ID_DARK_CURSOR,
            ID_CURSOR_INSTALL,
        ],
        4 => &[
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
        let mut visibility = std::collections::BTreeMap::new();
        let mut show = |id, visible| {
            visibility.insert(id, visible);
        };
        let page = PAGE.load(Ordering::SeqCst);
        for p in 0..5 {
            for id in page_ids(p) {
                show(*id, p == page);
                // Edit surfaces hide/show together with their inner edit.
                if field_surface_of(*id).is_some() {
                    show(FIELD_SURFACE_BASE + *id, p == page);
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
                show(id, !sunrise);
                // Field surfaces hide together with their inner edits.
                if id == ID_LIGHT_TIME || id == ID_DARK_TIME {
                    show(FIELD_SURFACE_BASE + id, !sunrise);
                }
            }
            for id in [
                ID_LOCATION_LBL,
                ID_LOCATION_SOURCE,
                ID_THEME_LOCATION_STATUS,
                ID_THEME_HINT,
            ] {
                show(id, sunrise);
            }
        }
        for (id, visible) in visibility {
            crate::nativeform::set_visible_deferred(get(hwnd, id), visible);
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
        for id in [
            ID_TAB_POWER,
            ID_TAB_THEME,
            ID_TAB_APP,
            ID_TAB_NOTIFICATIONS,
            ID_TAB_APPEARANCE,
        ] {
            let control = get(hwnd, id);
            if !control.is_invalid() {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, false);
            }
        }
        if IsWindowVisible(hwnd).as_bool() {
            crate::present_layout(hwnd);
        }
    }
}

// ---- Draft collection, validation, save, cancel --------------------------

struct Draft {
    keep_screen: bool,
    battery_allowed: bool,
    pause_on_lock: bool,
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
    light_wallpaper: String,
    dark_wallpaper: String,
    wallpapers: Vec<String>,
    light_cursor: String,
    dark_cursor: String,
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
    // Scheme row -> name, 0 = no linkage.
    let scheme_at = |row: usize| -> String {
        if row == 0 {
            return String::new();
        }
        crate::theme_engine::cursor_schemes()
            .get(row - 1)
            .cloned()
            .unwrap_or_default()
    };
    // The row value already is the wallpaper path ("" = no change).
    let wall_at = |id: i32| -> String {
        let value = crate::choice::value(get(hwnd, id));
        if value == "__browse__" {
            String::new()
        } else {
            value
        }
    };
    Draft {
        keep_screen: is_checked(hwnd, ID_KEEP_SCREEN),
        battery_allowed: is_checked(hwnd, ID_BATTERY_ALLOWED),
        pause_on_lock: is_checked(hwnd, ID_PAUSE_ON_LOCK),
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
        light_wallpaper: wall_at(ID_LIGHT_WALL),
        dark_wallpaper: wall_at(ID_DARK_WALL),
        wallpapers: crate::runtime::lock(&WALLPAPER_LIBRARY).clone(),
        // Map the dropdown row back to a scheme name (0 = empty = off).
        light_cursor: scheme_at(combo_sel(hwnd, ID_LIGHT_CURSOR)),
        dark_cursor: scheme_at(combo_sel(hwnd, ID_DARK_CURSOR)),
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
        Some(v) if (idletrigger_core::automation::MIN_WARNING_SECONDS..=3600).contains(&v) => {}
        _ => return Some((0, ID_WARNING_SECONDS, "settings_error_warning_seconds")),
    }
    // The fixed-schedule times are hidden and unused in sunrise mode;
    // validating them would report against invisible fields. Leftover junk
    // values only persist until the user switches back to fixed mode (which
    // validates again) or the next full config load (which sanitizes).
    if draft.theme_mode_idx != 1 {
        if !valid_time(&draft.light_time) {
            return Some((1, ID_LIGHT_TIME, "settings_error_light_time"));
        }
        if !valid_time(&draft.dark_time) {
            return Some((1, ID_DARK_TIME, "settings_error_dark_time"));
        }
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
fn draft_differs(draft: &Draft) -> bool {
    let base = crate::runtime::lock(&DRAFT_BASE)
        .clone()
        .unwrap_or_else(|| crate::cfg_map(Clone::clone));

    let (
        cfg_keep,
        cfg_batt,
        cfg_pause_lock,
        cfg_thresh,
        cfg_timeout,
        cfg_action,
        cfg_warning,
        cfg_enhanced,
    ) = {
        let c = &base;
        (
            c.keep_screen_on,
            c.nosleep_on_battery,
            c.nosleep_pause_on_lock,
            c.nosleep_battery_threshold,
            c.idle_timeout_minutes,
            c.idle_action.clone(),
            c.idle_warning_seconds,
            c.idle_enhanced_monitor,
        )
    };
    let (
        cfg_mode,
        cfg_light,
        cfg_dark,
        cfg_ip,
        cfg_batt_dark,
        cfg_fullscreen,
        cfg_light_wall,
        cfg_dark_wall,
        cfg_light_cursor,
        cfg_dark_cursor,
        cfg_lang,
    ) = {
        let c = &base;
        (
            c.theme_mode.clone(),
            c.theme_light_time.clone(),
            c.theme_dark_time.clone(),
            c.theme_ip_location_enabled,
            c.theme_dark_on_battery,
            c.theme_skip_fullscreen,
            c.theme_light_wallpaper.clone(),
            c.theme_dark_wallpaper.clone(),
            c.theme_light_cursor_scheme.clone(),
            c.theme_dark_cursor_scheme.clone(),
            c.language.clone(),
        )
    };
    let (cfg_lock, cfg_caps, cfg_num, cfg_scroll, cfg_lock_full, cfg_hotkeys, cfg_logging) = {
        let c = &base;
        (
            c.lock_keys_enabled,
            c.lock_keys_caps_enabled,
            c.lock_keys_num_enabled,
            c.lock_keys_scroll_enabled,
            c.lock_keys_skip_fullscreen,
            c.hotkeys_enabled,
            c.logging_enabled,
        )
    };

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
        || draft.pause_on_lock != cfg_pause_lock
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
        || draft.light_wallpaper != cfg_light_wall
        || draft.dark_wallpaper != cfg_dark_wall
        || draft.wallpapers != base.theme_wallpapers
        || draft.light_cursor != cfg_light_cursor
        || draft.dark_cursor != cfg_dark_cursor
        || draft.language_idx != lang_idx
        || draft.lock_keys != cfg_lock
        || draft.caps != cfg_caps
        || draft.num != cfg_num
        || draft.scroll != cfg_scroll
        || draft.lock_fullscreen != cfg_lock_full
        || draft.hotkeys != cfg_hotkeys
        || draft.autostart != AUTOSTART_BASE.load(Ordering::SeqCst)
        || draft.logging != cfg_logging
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

        let autostart_was = crate::system::autostart_is_enabled();
        let base = crate::runtime::lock(&DRAFT_BASE).clone();
        if let Err(err) = crate::commit_config(|c, _| {
            if base.as_ref() != Some(c) {
                return Err(t_pub("settings_save_conflict"));
            }
            c.keep_screen_on = draft.keep_screen;
            c.nosleep_on_battery = draft.battery_allowed;
            c.nosleep_pause_on_lock = draft.pause_on_lock;
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
            c.theme_light_wallpaper = draft.light_wallpaper.clone();
            c.theme_dark_wallpaper = draft.dark_wallpaper.clone();
            c.theme_wallpapers = draft.wallpapers.clone();
            c.theme_light_cursor_scheme = draft.light_cursor.clone();
            c.theme_dark_cursor_scheme = draft.dark_cursor.clone();
            c.language = LANG_VALUES[draft.language_idx].to_string();
            c.lock_keys_enabled = draft.lock_keys;
            c.lock_keys_caps_enabled = draft.caps;
            c.lock_keys_num_enabled = draft.num;
            c.lock_keys_scroll_enabled = draft.scroll;
            c.lock_keys_skip_fullscreen = draft.lock_fullscreen;
            c.hotkeys_enabled = draft.hotkeys;
            c.logging_enabled = draft.logging;
            Ok(())
        }) {
            set_text(hwnd, ID_VALIDATION, &err);
            return;
        }
        *crate::runtime::lock(&DRAFT_BASE) = Some(crate::cfg_map(Clone::clone));
        let mut failures = Vec::new();
        crate::log_line("settings changed");
        // Appearance settings take effect for the current side right away:
        // configuring them shows the result, no switch needed.
        crate::theme_engine::apply_current_side();
        crate::apply_stay_awake();
        crate::apply_language(&crate::cfg_map(|c| c.language.clone()));
        crate::refresh_checkboxes();
        crate::refresh_status();
        // Settings do not directly change the active palette. Theme changes
        // arrive through the normal notification path; retheming here can
        // cloak/reveal this newly created dialog just before we destroy it.

        {
            crate::system::unregister_all();
            if draft.hotkeys {
                let failed = crate::system::register_all();
                if !failed.is_empty() {
                    failures.push(format!("{}: {}", t_pub("menu_hotkeys"), failed.join(", ")));
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
                } else {
                    failures.push(t_pub("menu_autostart"));
                }
            } else if crate::system::autostart_disable() {
                crate::log_line("autostart disabled from settings");
            } else {
                failures.push(t_pub("menu_autostart"));
            }
        }
        AUTOSTART_BASE.store(crate::system::autostart_is_enabled(), Ordering::SeqCst);
        if !failures.is_empty() {
            let details = failures.join(", ");
            crate::log_line(&format!("settings system integration failed: {details}"));
            set_text(
                hwnd,
                ID_VALIDATION,
                &t_pub("settings_system_apply_failed").replace("%s", &details),
            );
            return;
        }
        let _ = DestroyWindow(hwnd);
    }
}

fn close_request() {
    unsafe {
        let hwnd = current();
        if draft_differs(&collect_draft(hwnd)) {
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
    // (control id, i18n key) pairs — Go settingsTooltipBindings().
    static TOOLS: &[(i32, &str)] = &[
        (ID_KEEP_SCREEN, "tip_keep_screen"),
        (ID_BATTERY_ALLOWED, "tip_nosleep_battery"),
        (ID_PAUSE_ON_LOCK, "tip_pause_on_lock"),
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
        (ID_ROW_WALL_LBL, "tip_theme_wallpaper"),
        (ID_LIGHT_WALL, "tip_theme_wallpaper"),
        (ID_DARK_WALL, "tip_theme_wallpaper"),
        (ID_ROW_CURSOR_LBL, "tip_theme_cursor"),
        (ID_LIGHT_CURSOR, "tip_theme_cursor"),
        (ID_DARK_CURSOR, "tip_theme_cursor"),
        (ID_CURSOR_INSTALL, "tip_theme_cursor"),
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
    let bindings: Vec<(usize, &str)> = TOOLS.iter().map(|(id, key)| (*id as usize, *key)).collect();
    crate::nativeform::form_tooltips(hwnd, &bindings);
    // Keep the module handle in sync for retheme_tooltip; form_tooltips
    // stores the tooltip as a window property.
    TOOLTIP_HWND.store(
        unsafe { GetPropW(hwnd, windows::core::w!("IdleTriggerFormTooltip")) }.0 as isize,
        Ordering::SeqCst,
    );
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    crate::guarded_proc("settings", hwnd, msg, wparam, lparam, move || unsafe {
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
                    CBN_SELCHANGE if idc == ID_THEME_MODE || idc == ID_LOCATION_SOURCE => {
                        request_location_preview(hwnd);
                        apply_dependent_states(hwnd);
                        LRESULT(0)
                    }
                    CBN_SELCHANGE if idc == ID_LIGHT_WALL || idc == ID_DARK_WALL => {
                        // The Browse footer opens the picker and selects the
                        // chosen file on this side.
                        if crate::choice::value(get(hwnd, idc)) == "__browse__" {
                            browse_wallpaper_for_side(hwnd, idc);
                            set_text(hwnd, ID_VALIDATION, "");
                        }
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
                refresh_theme();
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
                    let text = if id == ID_VALIDATION && GetWindowTextLengthW(child_hwnd) > 0 {
                        palette.danger_surface_text
                    } else if disabled {
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
                let validation = get(hwnd, ID_VALIDATION);
                let is_error = child_hwnd == validation && GetWindowTextLengthW(validation) > 0;
                let color = if is_error {
                    // Validation messages render in the error color.
                    theme::palette().danger_surface_text
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
            WM_DESTROY => {
                SETTINGS_HWND.store(0, Ordering::SeqCst);
                TOOLTIP_HWND.store(0, Ordering::SeqCst);
                PAGE.store(0, Ordering::SeqCst);
                // Drop owner-draw checkbox states so a recreated window does
                // not resurrect check marks on reused control ids.
                *checks() = None;
                let _ = EnableWindow(crate::hwnd(&crate::PANEL), true);
                crate::refresh_status();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    })
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
            crate::paint::draw_field(
                dc,
                bounds,
                p,
                p.window_bg,
                state,
                crate::paint::control_radius(),
            );
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
            ID_TAB_POWER | ID_TAB_THEME | ID_TAB_APP | ID_TAB_NOTIFICATIONS | ID_TAB_APPEARANCE
        ) {
            let mut state = crate::nativeform::control_state(item.control, item.state);
            let page = PAGE.load(Ordering::SeqCst);
            state.active = id
                == [
                    ID_TAB_POWER,
                    ID_TAB_THEME,
                    ID_TAB_APPEARANCE,
                    ID_TAB_NOTIFICATIONS,
                    ID_TAB_APP,
                ][page as usize];
            crate::paint::draw_button(
                dc,
                bounds,
                body_font(),
                &label,
                p,
                p.window_bg,
                state,
                crate::paint::control_radius(),
            );
        } else if id == ID_SAVE {
            // Save is the form's default action: accent fill (Go state.Active).
            let mut state = crate::nativeform::control_state(item.control, item.state);
            state.active = true;
            crate::paint::draw_button(
                dc,
                bounds,
                body_font(),
                &label,
                p,
                p.window_bg,
                state,
                crate::paint::control_radius(),
            );
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
            crate::paint::draw_button(
                dc,
                bounds,
                body_font(),
                &label,
                p,
                p.window_bg,
                state,
                crate::paint::control_radius(),
            );
        }
    }
}

fn handle_click(hwnd: HWND, idc: i32) {
    unsafe {
        match idc {
            ID_TAB_POWER | ID_TAB_THEME | ID_TAB_APP | ID_TAB_NOTIFICATIONS | ID_TAB_APPEARANCE => {
                let page = match idc {
                    ID_TAB_POWER => 0,
                    ID_TAB_THEME => 1,
                    ID_TAB_APPEARANCE => 2,
                    ID_TAB_NOTIFICATIONS => 3,
                    _ => 4,
                };
                PAGE.store(page, Ordering::SeqCst);
                apply_dependent_states(hwnd);
            }
            ID_KEEP_SCREEN | ID_BATTERY_ALLOWED | ID_PAUSE_ON_LOCK | ID_IDLE_ENHANCED
            | ID_THEME_BATTERY | ID_THEME_FULLSCREEN | ID_HOTKEYS | ID_AUTOSTART | ID_LOGGING
            | ID_LOCK_KEYS | ID_LOCK_CAPS | ID_LOCK_NUM | ID_LOCK_SCROLL | ID_LOCK_FULLSCREEN => {
                // Owner-drawn checkboxes keep state in CHECKS.
                toggle_checked(hwnd, idc);
                apply_dependent_states(hwnd);
                set_text(hwnd, ID_VALIDATION, "");
            }
            ID_THEME_MODE | ID_LOCATION_SOURCE | ID_IDLE_ACTION | ID_LANGUAGE | ID_LIGHT_CURSOR
            | ID_DARK_CURSOR | ID_LIGHT_WALL | ID_DARK_WALL => {
                // Refresh scheme/wallpaper rows right before opening so an
                // install that just finished or a library edit shows up.
                if idc == ID_LIGHT_CURSOR || idc == ID_DARK_CURSOR {
                    refresh_cursor_choices(hwnd);
                }
                crate::choice::toggle(get(hwnd, idc), hwnd, idc);
            }
            ID_CURSOR_INSTALL => install_cursor_inf(hwnd),
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

/// Refresh captions and choice labels while retaining every draft value.
pub fn refresh_language() {
    let hwnd = current();
    if hwnd.is_invalid() {
        return;
    }
    let _dpi = crate::dpi::Scope::window(hwnd);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide(&t_pub("settings_title")).as_ptr()));
    }
    for (id, key) in [
        (ID_TITLE, "settings_title"),
        (ID_DESCRIPTION, "settings_description"),
        (ID_TAB_POWER, "settings_tab_power"),
        (ID_TAB_THEME, "settings_tab_theme"),
        (ID_TAB_NOTIFICATIONS, "settings_tab_notifications"),
        (ID_TAB_APP, "settings_tab_app"),
        (ID_TAB_APPEARANCE, "settings_tab_appearance"),
        (ID_APPEARANCE_HINT, "settings_appearance_hint"),
        (ID_POWER_TITLE, "settings_power_title"),
        (ID_KEEP_SCREEN, "settings_keep_screen"),
        (ID_BATTERY_ALLOWED, "settings_battery_allowed"),
        (ID_PAUSE_ON_LOCK, "settings_pause_on_lock"),
        (ID_BATTERY_LBL, "settings_battery_threshold"),
        (ID_POWER_HINT, "settings_power_hint"),
        (ID_IDLE_TITLE, "settings_idle_title"),
        (ID_IDLE_ENHANCED, "menu_idle_enhanced"),
        (ID_IDLE_TIMEOUT_LBL, "settings_idle_timeout_minutes"),
        (ID_WARNING_LBL, "settings_idle_warning_seconds"),
        (ID_IDLE_ACTION_LBL, "settings_idle_action"),
        (ID_THEME_SCHEDULE_TITLE, "settings_theme_schedule_group"),
        (ID_THEME_MODE_LBL, "settings_theme_mode"),
        (ID_LIGHT_TIME_LBL, "settings_light_time"),
        (ID_DARK_TIME_LBL, "settings_dark_time"),
        (ID_LOCATION_LBL, "settings_location_source"),
        (ID_THEME_HINT, "settings_theme_hint"),
        (ID_THEME_BEHAVIOR_TITLE, "settings_theme_behavior_group"),
        (ID_THEME_BATTERY, "menu_theme_battery_dark"),
        (ID_THEME_FULLSCREEN, "menu_theme_skip_fullscreen"),
        (ID_APPEARANCE_TITLE, "settings_theme_appearance_group"),
        (ID_APPEARANCE_HINT, "settings_appearance_hint"),
        (ID_COL_LIGHT, "settings_light_side"),
        (ID_COL_DARK, "settings_dark_side"),
        (ID_ROW_WALL_LBL, "settings_row_wallpaper"),
        (ID_ROW_CURSOR_LBL, "settings_row_cursor"),
        (ID_CURSOR_INSTALL, "settings_install_inf"),
        (ID_APP_GENERAL_TITLE, "settings_app_general_group"),
        (ID_LANGUAGE_LBL, "settings_language"),
        (ID_HOTKEYS, "menu_hotkeys"),
        (ID_AUTOSTART, "menu_autostart"),
        (ID_LOGGING, "menu_logging"),
        (ID_APP_ABOUT_TITLE, "settings_app_about_group"),
        (ID_NOTIFICATIONS_TITLE, "settings_lock_keys"),
        (ID_LOCK_KEYS, "settings_lock_keys_enable"),
        (ID_NOTIFICATIONS_BEHAVIOR, "settings_notification_behavior"),
        (ID_LOCK_FULLSCREEN, "settings_notification_fullscreen"),
        (ID_NOTIFICATIONS_HINT, "settings_notification_hint"),
        (ID_LOCK_PREVIEW, "settings_notification_preview"),
        (ID_SAVE, "common_save"),
        (ID_CANCEL, "common_cancel"),
        (ID_PROJECT_HOME_LBL, "settings_project_home_label"),
    ] {
        set_text(hwnd, id, &t_pub(key));
    }
    set_text(
        hwnd,
        ID_VERSION,
        &t_pub("settings_version").replace("%s", crate::APP_VERSION),
    );
    for (id, labels) in [
        (ID_IDLE_ACTION, idle_action_labels()),
        (ID_LIGHT_CURSOR, cursor_choice_labels()),
        (ID_DARK_CURSOR, cursor_choice_labels()),
        (
            ID_THEME_MODE,
            vec![
                t_pub("settings_theme_fixed"),
                t_pub("settings_theme_sunrise"),
            ],
        ),
        (
            ID_LOCATION_SOURCE,
            vec![
                t_pub("settings_location_auto"),
                t_pub("settings_location_ip"),
            ],
        ),
        (
            ID_LANGUAGE,
            vec![
                t_pub("menu_lang_auto"),
                t_pub("menu_lang_en"),
                t_pub("menu_lang_zh"),
            ],
        ),
    ] {
        let control = get(hwnd, id);
        let selected = crate::choice::selection(control);
        let items = labels
            .into_iter()
            .map(|label| (label.clone(), label))
            .collect::<Vec<_>>();
        crate::choice::set_items(control, &items);
        crate::choice::select_index(control, selected);
    }
    set_text(
        hwnd,
        ID_THEME_LOCATION_STATUS,
        &location_status_text(combo_sel(hwnd, ID_LOCATION_SOURCE) == 1),
    );
    // Language-dependent geometry must follow the new text: the theme hint
    // is two lines in English but one in Chinese, which shifts the behavior
    // group, and the project-home row is measured from the label. Like the
    // panel's own refresh_language, this only moves windows — drafts live in
    // control state, not geometry.
    let theme_hint_h = if is_chinese() { 22 } else { 40 };
    let behavior_top = 234 + theme_hint_h + SECTION_GAP;
    for (id, x, y, w, h) in [
        (ID_THEME_HINT, CONTENT_X, 234, 468, theme_hint_h),
        (
            ID_THEME_BEHAVIOR_TITLE,
            CONTENT_X,
            behavior_top,
            468,
            SECTION_TITLE_H,
        ),
        (
            ID_THEME_BATTERY,
            CONTENT_X,
            behavior_top + SECTION_TITLE_H + SECTION_ITEM_GAP,
            468,
            CHECK_H,
        ),
        (
            ID_THEME_FULLSCREEN,
            CONTENT_X,
            behavior_top + SECTION_TITLE_H + SECTION_ITEM_GAP + CHECK_H + FUNCTION_GAP,
            468,
            CHECK_H,
        ),
    ] {
        unsafe {
            let _ = SetWindowPos(
                get(hwnd, id),
                None,
                s(x),
                s(y),
                s(w),
                s(h),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    let project_label = t_pub("settings_project_home_label");
    let label_w = logical_text_width(hwnd, body_font(), &project_label, 96) + 2;
    let url_w = logical_text_width(hwnd, body_font(), PROJECT_URL, 376) + 2;
    let mut link_x = CONTENT_X + label_w + FUNCTION_GAP;
    if is_chinese() {
        // CJK advance boxes carry extra trailing space (Go optical fix).
        link_x -= 10;
    }
    for (control, x, y, w, h) in [
        (get(hwnd, ID_PROJECT_HOME_LBL), CONTENT_X, 310, label_w, 24),
        (
            get(hwnd, ID_PROJECT_HOME),
            link_x,
            308,
            url_w.min(CONTENT_RIGHT - link_x),
            24,
        ),
    ] {
        unsafe {
            let _ = SetWindowPos(
                control,
                None,
                s(x),
                s(y),
                s(w),
                s(h),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    unsafe {
        create_tooltip(hwnd);
    }
    refresh_theme();
}

#[cfg(test)]
mod locale_tests {
    use super::*;
    #[test]
    fn relabeling_preserves_native_edits_choice_and_save_baseline() {
        let _guard = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        let old_config = crate::runtime::lock(&crate::CONFIG).replace(Default::default());
        let old_locale = crate::I18N
            .write()
            .unwrap()
            .replace(idletrigger_core::i18n::I18n::load("en"));
        create();
        let window = current();
        let edit = get(window, ID_IDLE_TIMEOUT);
        set_text(window, ID_IDLE_TIMEOUT, "073");
        crate::choice::select_index(get(window, ID_LANGUAGE), 2);
        let baseline = crate::runtime::lock(&DRAFT_BASE).clone();
        *crate::I18N.write().unwrap() = Some(idletrigger_core::i18n::I18n::load("zh-CN"));
        refresh_language();
        assert_eq!(get(window, ID_IDLE_TIMEOUT), edit);
        assert_eq!(control_text(window, ID_IDLE_TIMEOUT), "073");
        assert_eq!(control_text(window, ID_TITLE), "设置");
        assert_eq!(combo_sel(window, ID_LANGUAGE), 2);
        assert_eq!(*crate::runtime::lock(&DRAFT_BASE), baseline);
        PAGE.store(1, Ordering::SeqCst);
        crate::choice::select_index(get(window, ID_THEME_MODE), 1);
        crate::choice::select_index(get(window, ID_LOCATION_SOURCE), 1);
        apply_dependent_states(window);
        refresh_location_status();
        assert!(control_text(window, ID_THEME_LOCATION_STATUS).starts_with("IP 定位"));
        let visible =
            |id| unsafe { GetWindowLongW(get(window, id), GWL_STYLE) as u32 & WS_VISIBLE.0 != 0 };
        assert!(visible(ID_LOCATION_SOURCE));
        assert!(!visible(ID_LIGHT_TIME));
        assert!(!visible(FIELD_SURFACE_BASE + ID_LIGHT_TIME));
        assert!(!visible(ID_IDLE_TIMEOUT));
        crate::choice::select_index(get(window, ID_THEME_MODE), 0);
        apply_dependent_states(window);
        assert!(!visible(ID_LOCATION_SOURCE));
        assert!(visible(ID_LIGHT_TIME));
        assert!(visible(FIELD_SURFACE_BASE + ID_LIGHT_TIME));
        assert_eq!(control_text(window, ID_IDLE_TIMEOUT), "073");
        unsafe {
            DestroyWindow(window).unwrap();
        }
        *crate::runtime::lock(&crate::CONFIG) = old_config;
        *crate::I18N.write().unwrap() = old_locale;
    }
}
