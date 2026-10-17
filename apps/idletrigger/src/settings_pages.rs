//! Settings page specifications: control ids, layout tokens, and the five
//! declarative page specs plus their label/row helpers. The window shell,
//! value population, and message dispatch stay in settings_ui; this module
//! is the single source of what each page contains and where it sits,
//! shared by creation and the language refresh.

use windows::Win32::Foundation::HWND;

use crate::kit::controls::logical_text_width;
use crate::kit::layout::{CARD_GAP, CARD_PAD_X, CARD_PAD_Y, GAP, LABEL_GAP, TITLE_GAP, row_slot};
use crate::kit::spec::{ControlSpec, FontRole, Text};
use crate::t_pub;

// Control ids — Go settingspanel.go numbering, extended in the Rust port.
pub(crate) const ID_TAB_POWER: i32 = 102;
pub(crate) const ID_TAB_THEME: i32 = 103;
pub(crate) const ID_TAB_APP: i32 = 104;
pub(crate) const ID_POWER_TITLE: i32 = 110;
pub(crate) const ID_KEEP_SCREEN: i32 = 111;
pub(crate) const ID_BATTERY_ALLOWED: i32 = 112;
pub(crate) const ID_BATTERY_LBL: i32 = 113;
pub(crate) const ID_BATTERY_THRESH: i32 = 114;
pub(crate) const ID_WARNING_LBL: i32 = 115;
pub(crate) const ID_WARNING_SECONDS: i32 = 116;
pub(crate) const ID_IDLE_TIMEOUT_LBL: i32 = 118;
pub(crate) const ID_IDLE_TIMEOUT: i32 = 119;
pub(crate) const ID_IDLE_ACTION_LBL: i32 = 120;
pub(crate) const ID_IDLE_ACTION: i32 = 121;
pub(crate) const ID_IDLE_ENHANCED: i32 = 122;
pub(crate) const ID_IDLE_TITLE: i32 = 123;
pub(crate) const ID_THEME_MODE_LBL: i32 = 131;
pub(crate) const ID_THEME_MODE: i32 = 132;
pub(crate) const ID_LIGHT_TIME_LBL: i32 = 133;
pub(crate) const ID_LIGHT_TIME: i32 = 134;
pub(crate) const ID_DARK_TIME_LBL: i32 = 135;
pub(crate) const ID_DARK_TIME: i32 = 136;
pub(crate) const ID_LOCATION_LBL: i32 = 137;
pub(crate) const ID_LOCATION_SOURCE: i32 = 138;
pub(crate) const ID_THEME_HINT: i32 = 143;
pub(crate) const ID_THEME_BATTERY: i32 = 144;
pub(crate) const ID_THEME_FULLSCREEN: i32 = 145;
pub(crate) const ID_THEME_SCHEDULE_TITLE: i32 = 148;
pub(crate) const ID_THEME_BEHAVIOR_TITLE: i32 = 149;
pub(crate) const ID_LANGUAGE_LBL: i32 = 151;
pub(crate) const ID_LANGUAGE: i32 = 152;
pub(crate) const ID_HOTKEYS: i32 = 153;
pub(crate) const ID_AUTOSTART: i32 = 154;
pub(crate) const ID_LOGGING: i32 = 155;
pub(crate) const ID_PROJECT_HOME: i32 = 157;
pub(crate) const ID_APP_GENERAL_TITLE: i32 = 159;
pub(crate) const ID_VALIDATION: i32 = 160;
pub(crate) const ID_APP_ABOUT_TITLE: i32 = 161;
pub(crate) const ID_CANCEL: i32 = 162;
pub(crate) const ID_SAVE: i32 = 163;
pub(crate) const ID_THEME_LOCATION_STATUS: i32 = 164;
pub(crate) const ID_PROJECT_HOME_LBL: i32 = 165;
pub(crate) const ID_VERSION: i32 = 166;
pub(crate) const ID_LOCK_KEYS: i32 = 167;
pub(crate) const ID_TAB_NOTIFICATIONS: i32 = 168;
pub(crate) const ID_NOTIFICATIONS_TITLE: i32 = 169;
pub(crate) const ID_LOCK_CAPS: i32 = 170;
pub(crate) const ID_LOCK_NUM: i32 = 171;
pub(crate) const ID_LOCK_SCROLL: i32 = 172;
pub(crate) const ID_LOCK_FULLSCREEN: i32 = 173;
pub(crate) const ID_LOCK_PREVIEW: i32 = 174;
pub(crate) const ID_NOTIFICATIONS_HINT: i32 = 175;
pub(crate) const ID_NOTIFICATIONS_BEHAVIOR: i32 = 176;
pub(crate) const ID_PAUSE_ON_LOCK: i32 = 177;
// Appearance page: paired light/dark columns over a wallpaper library.
// Labels and combos each own a unique id — sharing ids makes GetDlgItem
// resolve only the first control and leaks the second across pages.
pub(crate) const ID_ROW_WALL_LBL: i32 = 179;
pub(crate) const ID_LIGHT_WALL: i32 = 180;
pub(crate) const ID_DARK_WALL: i32 = 183;
pub(crate) const ID_ROW_CURSOR_LBL: i32 = 185;
pub(crate) const ID_LIGHT_CURSOR: i32 = 186;
pub(crate) const ID_DARK_CURSOR: i32 = 188;
pub(crate) const ID_TAB_APPEARANCE: i32 = 191;
pub(crate) const ID_COL_LIGHT: i32 = 198;
pub(crate) const ID_COL_DARK: i32 = 199;
// Header page title/subtitle pairs: every page repeats its nav label plus a
// one-line intro in the window-header spot; page switching swaps the pair.
pub(crate) const ID_PAGE_TITLE_POWER: i32 = 200;
pub(crate) const ID_PAGE_SUB_POWER: i32 = 201;
pub(crate) const ID_PAGE_TITLE_THEME: i32 = 202;
pub(crate) const ID_PAGE_SUB_THEME: i32 = 203;
pub(crate) const ID_PAGE_TITLE_APPEARANCE: i32 = 204;
pub(crate) const ID_PAGE_SUB_APPEARANCE: i32 = 205;
pub(crate) const ID_PAGE_TITLE_NOTIFICATIONS: i32 = 206;
pub(crate) const ID_PAGE_SUB_NOTIFICATIONS: i32 = 207;
pub(crate) const ID_PAGE_TITLE_APP: i32 = 208;
pub(crate) const ID_PAGE_SUB_APP: i32 = 209;
// Appearance restore pair: pre-change snapshot + factory defaults.
pub(crate) const ID_RESTORE_PREV: i32 = 210;
pub(crate) const ID_RESTORE_DEFAULT: i32 = 211;
pub(crate) const ID_RESTORE_HINT: i32 = 212;
// App page update block lives inside the general card (below logging): the
// auto-check switch plus a manual check button (same one-click flow as the
// panel link).
pub(crate) const ID_APP_UPDATE_AUTO: i32 = 214;
pub(crate) const ID_APP_UPDATE_CHECK: i32 = 215;

/// Session state for the wallpaper library: seeded from the config when the
/// settings window opens, mutated by Add/Remove, committed on Save.
pub(crate) static WALLPAPER_LIBRARY: std::sync::LazyLock<std::sync::Mutex<Vec<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

// Layout tokens — Go controls.go build() constants, now riding the shared
// panel grammar from the layout module for card sections and row rhythm.
pub(crate) const CLIENT_W: i32 = 676;
pub(crate) const CLIENT_H: i32 = 510;
pub(crate) const CONTENT_X: i32 = 184;
pub(crate) const CONTENT_RIGHT: i32 = CLIENT_W - 24;
pub(crate) const SECTION_TOP: i32 = 90;
pub(crate) const SECTION_TITLE_H: i32 = 20;
pub(crate) const CHECK_H: i32 = 28;
pub(crate) const BTN_H: i32 = 36;
// Two-line validation row above the footer buttons: the English conflict
// message measures ~491px and Go's inline 236px label clipped it mid-sentence.
pub(crate) const VALIDATION_H: i32 = 26;
pub(crate) const FIELD_H: i32 = 34;
// Unified numeric field width: half the old 128 column - the values are
// short numbers, and one width across the page reads as a system.
pub(crate) const FIELD_W: i32 = 64;
pub(crate) const DIALOG_BTN_W: i32 = 104;
pub(crate) const FOOTER_Y: i32 = CLIENT_H - 12 - BTN_H;
// Panel card grammar: cards span the content column, rows sit one
// CARD_PAD_X inside them, right-aligned controls end at ROW_RIGHT.
pub(crate) const CARD_W: i32 = CONTENT_RIGHT - CONTENT_X; // 468
pub(crate) const ROW_X: i32 = CONTENT_X + CARD_PAD_X; // 220
pub(crate) const ROW_RIGHT: i32 = CONTENT_RIGHT - CARD_PAD_X; // 664
pub(crate) const ROW_W: i32 = ROW_RIGHT - ROW_X; // 444
// Owner-drawn section card surfaces (CARD_BASE..=CARD_BASE + 7), one per
// section across the five pages; drawn by the WM_DRAWITEM dispatch.
pub(crate) const CARD_BASE: i32 = 620;

pub(crate) const IDLE_ACTIONS: [&str; 6] = [
    idletrigger_core::automation::ACTION_LOCK,
    idletrigger_core::automation::ACTION_SLEEP,
    idletrigger_core::automation::ACTION_HIBERNATE,
    idletrigger_core::automation::ACTION_SHUTDOWN,
    idletrigger_core::automation::ACTION_RESTART,
    idletrigger_core::automation::ACTION_SCREEN_OFF,
];
pub(crate) const LANG_VALUES: [&str; 3] = ["auto", "en", "zh-CN"];
pub(crate) const PROJECT_URL: &str = "https://github.com/JeffioZ/IdleTrigger";

pub(crate) fn idle_action_labels() -> Vec<String> {
    IDLE_ACTIONS
        .iter()
        .map(|a| t_pub(&format!("menu_action_{a}")))
        .collect()
}

/// Power/idle page as a declarative spec: creation and the language
/// refresh both run through it, so captions and combo items can never
/// drift between the two sites.
pub(crate) fn power_page_spec() -> Vec<crate::kit::spec::ControlSpec> {
    let mut specs = vec![
        ControlSpec::card(CARD_BASE, (CONTENT_X, 114, CARD_W, 152)),
        ControlSpec::label(
            ID_POWER_TITLE,
            Text::Key("settings_power_title"),
            FontRole::Section,
            (CONTENT_X, SECTION_TOP, CARD_W, SECTION_TITLE_H),
        ),
        ControlSpec::checkbox(
            ID_KEEP_SCREEN,
            "settings_keep_screen",
            (ROW_X, 122, ROW_W, CHECK_H),
        ),
        // Row order: screen-lock pause rides directly under Keep awake;
        // the battery pair keeps its label+field closest together.
        ControlSpec::checkbox(
            ID_PAUSE_ON_LOCK,
            "settings_pause_on_lock",
            (ROW_X, 156, ROW_W, CHECK_H),
        ),
        ControlSpec::checkbox(
            ID_BATTERY_ALLOWED,
            "settings_battery_allowed",
            (ROW_X, 190, ROW_W, CHECK_H),
        ),
        ControlSpec::label(
            ID_BATTERY_LBL,
            Text::Key("settings_battery_threshold"),
            FontRole::Body,
            (ROW_X, 230, 640 - 64 - 8 - ROW_X, 22),
        ),
        ControlSpec::edit(ID_BATTERY_THRESH, (640 - 64, 224, FIELD_W, FIELD_H), true),
        ControlSpec::card(CARD_BASE + 1, (CONTENT_X, 300, CARD_W, 124)),
        ControlSpec::label(
            ID_IDLE_TITLE,
            Text::Key("settings_idle_title"),
            FontRole::Section,
            (CONTENT_X, 276, CARD_W, SECTION_TITLE_H),
        ),
        ControlSpec::checkbox(
            ID_IDLE_ENHANCED,
            "menu_idle_enhanced",
            (ROW_X, 308, ROW_W, CHECK_H),
        ),
    ];
    // The two timing numbers share a row (half-width label+field pairs,
    // the same split the power card's switches use); the action dropdown
    // keeps its own row.
    for (index, (label_id, edit_id, key)) in [
        (
            ID_IDLE_TIMEOUT_LBL,
            ID_IDLE_TIMEOUT,
            "settings_idle_timeout_minutes",
        ),
        (
            ID_WARNING_LBL,
            ID_WARNING_SECONDS,
            "settings_idle_warning_seconds",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (slot_x, slot_w) = row_slot(ROW_W, 2, index as i32);
        let x = ROW_X + slot_x;
        specs.push(ControlSpec::notify_label(
            label_id,
            key,
            (x, 348, slot_w - FIELD_W - 8, 22),
        ));
        specs.push(ControlSpec::edit(
            edit_id,
            (x + slot_w - FIELD_W, 342, FIELD_W, FIELD_H),
            true,
        ));
    }
    specs.push(ControlSpec::label(
        ID_IDLE_ACTION_LBL,
        Text::Key("settings_idle_action"),
        FontRole::Body,
        (ROW_X, 388, 308, 22),
    ));
    specs.push(ControlSpec::combo(
        ID_IDLE_ACTION,
        (512, 382, 128, FIELD_H),
        idle_action_labels,
    ));
    specs
}

pub(crate) fn theme_mode_labels() -> Vec<String> {
    vec![
        t_pub("settings_theme_fixed"),
        t_pub("settings_theme_sunrise"),
    ]
}

pub(crate) fn location_labels() -> Vec<String> {
    vec![
        t_pub("settings_location_auto"),
        t_pub("settings_location_ip"),
    ]
}

pub(crate) fn language_labels() -> Vec<String> {
    vec![
        t_pub("menu_lang_auto"),
        t_pub("menu_lang_en"),
        t_pub("menu_lang_zh"),
    ]
}

/// Wallpaper rows for one side at creation; the library dropdowns refresh
/// by value in their own path (file names are not translations).
pub(crate) fn wall_pick_rows() -> Vec<crate::choice::ChoiceItem> {
    wallpaper_pick_items("")
}

/// Day/night page: the hint wraps to two lines in English, shifting the
/// behavior section — rects recompute from theme_flow on every call, so
/// creation and the language refresh share one geometry source.
pub(crate) fn theme_page_spec() -> Vec<crate::kit::spec::ControlSpec> {
    let flow = theme_flow();
    vec![
        ControlSpec::card(
            CARD_BASE + 2,
            (CONTENT_X, 114, CARD_W, flow.schedule_card_h),
        ),
        ControlSpec::label(
            ID_THEME_SCHEDULE_TITLE,
            Text::Key("settings_theme_schedule_group"),
            FontRole::Section,
            (CONTENT_X, SECTION_TOP, CARD_W, SECTION_TITLE_H),
        ),
        ControlSpec::label(
            ID_THEME_MODE_LBL,
            Text::Key("settings_theme_mode"),
            FontRole::Body,
            (ROW_X, 128, 208, 22),
        ),
        ControlSpec::combo(ID_THEME_MODE, (420, 122, 220, FIELD_H), theme_mode_labels),
        ControlSpec::label(
            ID_LIGHT_TIME_LBL,
            Text::Key("settings_light_time"),
            FontRole::Body,
            (ROW_X, 168, 104, 22),
        ),
        ControlSpec::edit(ID_LIGHT_TIME, (308, 162, 104, FIELD_H), false),
        ControlSpec::label(
            ID_DARK_TIME_LBL,
            Text::Key("settings_dark_time"),
            FontRole::Body,
            (428, 168, 104, 22),
        ),
        ControlSpec::edit(ID_DARK_TIME, (536, 162, 104, FIELD_H), false),
        ControlSpec::label(
            ID_LOCATION_LBL,
            Text::Key("settings_location_source"),
            FontRole::Body,
            (ROW_X, 168, 208, 22),
        ),
        ControlSpec::combo(
            ID_LOCATION_SOURCE,
            (420, 162, 220, FIELD_H),
            location_labels,
        ),
        // The status line's text is dynamic (location state); the spec
        // owns its geometry only.
        ControlSpec::label(
            ID_THEME_LOCATION_STATUS,
            Text::None,
            FontRole::Body,
            (ROW_X, 202, ROW_W, 22),
        ),
        ControlSpec::label(
            ID_THEME_HINT,
            Text::Key("settings_theme_hint"),
            FontRole::Body,
            (ROW_X, 230, ROW_W, flow.hint_h),
        ),
        ControlSpec::card(
            CARD_BASE + 3,
            (
                CONTENT_X,
                flow.behavior_card_y,
                CARD_W,
                2 * CARD_PAD_Y + 2 * CHECK_H + LABEL_GAP,
            ),
        ),
        ControlSpec::label(
            ID_THEME_BEHAVIOR_TITLE,
            Text::Key("settings_theme_behavior_group"),
            FontRole::Section,
            (CONTENT_X, flow.behavior_title_y, CARD_W, SECTION_TITLE_H),
        ),
        ControlSpec::checkbox(
            ID_THEME_BATTERY,
            "menu_theme_battery_dark",
            (ROW_X, flow.battery_y, ROW_W, CHECK_H),
        ),
        ControlSpec::checkbox(
            ID_THEME_FULLSCREEN,
            "menu_theme_skip_fullscreen",
            (ROW_X, flow.fullscreen_y, ROW_W, CHECK_H),
        ),
    ]
}

/// Appearance page: paired light/dark columns for wallpaper and cursor
/// schemes inside one card, over a shared wallpaper library.
pub(crate) fn appearance_page_spec() -> Vec<crate::kit::spec::ControlSpec> {
    let (slot0, w0) = row_slot(ROW_W, 2, 0);
    let (slot1, w1) = row_slot(ROW_W, 2, 1);
    vec![
        ControlSpec::card(CARD_BASE + 4, (CONTENT_X, SECTION_TOP, CARD_W, 184)),
        ControlSpec::label(
            ID_COL_LIGHT,
            Text::Key("settings_light_side"),
            FontRole::Section,
            (264, 98, 182, 22),
        ),
        ControlSpec::label(
            ID_COL_DARK,
            Text::Key("settings_dark_side"),
            FontRole::Section,
            (458, 98, 182, 22),
        ),
        ControlSpec::label(
            ID_ROW_WALL_LBL,
            Text::Key("settings_row_wallpaper"),
            FontRole::Body,
            (ROW_X, 132, 56, 22),
        ),
        ControlSpec::combo_rows(ID_LIGHT_WALL, (264, 126, 182, FIELD_H), wall_pick_rows),
        ControlSpec::combo_rows(ID_DARK_WALL, (458, 126, 182, FIELD_H), wall_pick_rows),
        ControlSpec::label(
            ID_ROW_CURSOR_LBL,
            Text::Key("settings_row_cursor"),
            FontRole::Body,
            (ROW_X, 172, 56, 22),
        ),
        ControlSpec::combo_rows(
            ID_LIGHT_CURSOR,
            (264, 166, 182, FIELD_H),
            cursor_choice_rows,
        ),
        ControlSpec::combo_rows(ID_DARK_CURSOR, (458, 166, 182, FIELD_H), cursor_choice_rows),
        // Restore pair: back to the state captured before the first
        // day/night application, or to system factory defaults.
        ControlSpec::button(
            ID_RESTORE_PREV,
            "settings_restore_prev",
            (ROW_X + slot0, 206, w0, FIELD_H),
        ),
        ControlSpec::button(
            ID_RESTORE_DEFAULT,
            "settings_restore_default",
            (ROW_X + slot1, 206, w1, FIELD_H),
        ),
        // Muted caption under the disabled restore button: explains when a
        // pre-change snapshot appears (tooltips cannot fire on disabled
        // controls, so the reason must stay visible).
        ControlSpec::label(
            ID_RESTORE_HINT,
            Text::Key("settings_restore_hint"),
            FontRole::Body,
            (ROW_X, 244, ROW_W, 22),
        ),
    ]
}

/// App page: the update-check button's caption follows the updater phase,
/// so it is create-only; the project-home row is measured from the label.
pub(crate) fn app_page_spec(hwnd: HWND) -> Vec<crate::kit::spec::ControlSpec> {
    let project_label = t_pub("settings_project_home_label");
    let label_w = logical_text_width(hwnd, crate::kit::fonts::body(), &project_label, 96) + 2;
    let url_w = logical_text_width(hwnd, crate::kit::fonts::body(), PROJECT_URL, 376) + 2;
    let mut link_x = CONTENT_X + label_w + GAP;
    if crate::i18n_is_chinese() {
        // CJK advance boxes carry extra trailing space (Go optical fix).
        link_x -= 10;
    }
    vec![
        ControlSpec::card(CARD_BASE + 7, (CONTENT_X, 114, CARD_W, 228)),
        ControlSpec::label(
            ID_APP_GENERAL_TITLE,
            Text::Key("settings_app_general_group"),
            FontRole::Section,
            (CONTENT_X, SECTION_TOP, CARD_W, SECTION_TITLE_H),
        ),
        ControlSpec::label(
            ID_LANGUAGE_LBL,
            Text::Key("settings_language"),
            FontRole::Body,
            (ROW_X, 128, 208, 22),
        ),
        ControlSpec::combo(ID_LANGUAGE, (420, 122, 220, FIELD_H), language_labels),
        ControlSpec::checkbox(ID_HOTKEYS, "menu_hotkeys", (ROW_X, 162, ROW_W, CHECK_H)),
        ControlSpec::checkbox(ID_AUTOSTART, "menu_autostart", (ROW_X, 196, ROW_W, CHECK_H)),
        ControlSpec::checkbox(ID_LOGGING, "menu_logging", (ROW_X, 230, ROW_W, CHECK_H)),
        ControlSpec::checkbox(
            ID_APP_UPDATE_AUTO,
            "settings_update_auto",
            (ROW_X, 264, ROW_W, CHECK_H),
        ),
        // Manual check button: caption follows the updater phase (检查更新 →
        // 检查中… → 更新到 vX → 下载更新 N%), drawn on the card surface.
        // Wider than the footer buttons: the phase captions ("Check for
        // updates", "Downloading 100%") must not wrap inside
        // draw_button_label's word-break on wide-Latin system fonts.
        ControlSpec::button_init(
            ID_APP_UPDATE_CHECK,
            "menu_check_updates",
            (ROW_RIGHT - 164, 298, 164, BTN_H),
        ),
        // The about block is one quiet row, not a card: a 40px face around
        // a single link read as filler chrome.
        ControlSpec::label(
            ID_APP_ABOUT_TITLE,
            Text::Key("settings_app_about_group"),
            FontRole::Section,
            (CONTENT_X, 352, CARD_W, SECTION_TITLE_H),
        ),
        ControlSpec::label(
            ID_PROJECT_HOME_LBL,
            Text::Key("settings_project_home_label"),
            FontRole::Body,
            (CONTENT_X, 384, label_w, 22),
        ),
        // Go renders the URL as an underlined accent-colored text link with
        // a hand cursor (DrawTextLink), not a button.
        ControlSpec::link(
            ID_PROJECT_HOME,
            PROJECT_URL,
            (link_x, 384, url_w.min(CONTENT_RIGHT - link_x), 22),
        ),
    ]
}

/// Screen notifications page: the hint is one line in Chinese and two in
/// English, which grows the behavior card and moves the preview button.
pub(crate) fn notifications_page_spec() -> Vec<crate::kit::spec::ControlSpec> {
    let notif_hint_h = if crate::i18n_is_chinese() { 22 } else { 42 };
    vec![
        ControlSpec::card(CARD_BASE + 5, (CONTENT_X, 114, CARD_W, 146)),
        ControlSpec::label(
            ID_NOTIFICATIONS_TITLE,
            Text::Key("settings_lock_keys"),
            FontRole::Section,
            (CONTENT_X, SECTION_TOP, CARD_W, SECTION_TITLE_H),
        ),
        ControlSpec::checkbox(
            ID_LOCK_KEYS,
            "settings_lock_keys_enable",
            (ROW_X, 122, ROW_W, CHECK_H),
        ),
        ControlSpec::checkbox_text(
            ID_LOCK_CAPS,
            Text::Literal("Caps Lock"),
            (ROW_X + 24, 156, ROW_W - 24, CHECK_H),
        ),
        ControlSpec::checkbox_text(
            ID_LOCK_NUM,
            Text::Literal("Num Lock"),
            (ROW_X + 24, 190, ROW_W - 24, CHECK_H),
        ),
        ControlSpec::checkbox_text(
            ID_LOCK_SCROLL,
            Text::Literal("Scroll Lock"),
            (ROW_X + 24, 224, ROW_W - 24, CHECK_H),
        ),
        ControlSpec::card(CARD_BASE + 6, (CONTENT_X, 294, CARD_W, 92 + notif_hint_h)),
        ControlSpec::label(
            ID_NOTIFICATIONS_BEHAVIOR,
            Text::Key("settings_notification_behavior"),
            FontRole::Section,
            (CONTENT_X, 270, CARD_W, SECTION_TITLE_H),
        ),
        ControlSpec::checkbox(
            ID_LOCK_FULLSCREEN,
            "settings_notification_fullscreen",
            (ROW_X, 302, ROW_W, CHECK_H),
        ),
        ControlSpec::label(
            ID_NOTIFICATIONS_HINT,
            Text::Key("settings_notification_hint"),
            FontRole::Body,
            (ROW_X, 336, ROW_W, notif_hint_h),
        ),
        // The preview button follows the hint: the one-line Chinese hint
        // sits higher than the two-line English one.
        ControlSpec::button(
            ID_LOCK_PREVIEW,
            "settings_notification_preview",
            (ROW_X, 336 + notif_hint_h + 6, 160, BTN_H),
        ),
    ]
}

/// The theme page's language-dependent vertical flow: the schedule hint is
/// one line in Chinese and two in English, which moves the behavior
/// section. Shared by control creation and the language refresh so the two
/// sites cannot drift apart.
pub(crate) struct ThemeFlow {
    hint_h: i32,
    schedule_card_h: i32,
    behavior_title_y: i32,
    behavior_card_y: i32,
    battery_y: i32,
    fullscreen_y: i32,
}

pub(crate) fn theme_flow() -> ThemeFlow {
    let hint_h = if crate::i18n_is_chinese() { 22 } else { 40 };
    // Schedule card: top 114, last row (the hint) spans 230..230+hint_h,
    // then the bottom card pad closes the face.
    let schedule_card_h = 230 + hint_h + CARD_PAD_Y - 114;
    let behavior_title_y = 114 + schedule_card_h + CARD_GAP;
    let behavior_card_y = behavior_title_y + SECTION_TITLE_H + TITLE_GAP;
    ThemeFlow {
        hint_h,
        schedule_card_h,
        behavior_title_y,
        behavior_card_y,
        battery_y: behavior_card_y + CARD_PAD_Y,
        fullscreen_y: behavior_card_y + CARD_PAD_Y + CHECK_H + LABEL_GAP,
    }
}

/// Cursor dropdown rows: a leading "no linkage" entry, the installed
/// schemes (value = scheme name), and a trailing Install action that runs
/// the .inf installer.
pub(crate) fn cursor_choice_rows() -> Vec<crate::choice::ChoiceItem> {
    let mut rows = vec![crate::choice::ChoiceItem::option(
        "",
        &t_pub("settings_appearance_none"),
    )];
    for scheme in crate::theme_engine::cursor_schemes() {
        rows.push(crate::choice::ChoiceItem::option(&scheme, &scheme));
    }
    rows.push(crate::choice::ChoiceItem::option(
        "__install__",
        &t_pub("settings_install_inf"),
    ));
    rows
}

/// Picker rows for one wallpaper side: "No change", the recently used
/// images (file names), and a trailing Browse action that opens the picker.
pub(crate) fn wallpaper_pick_items(current: &str) -> Vec<crate::choice::ChoiceItem> {
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
    if !current.is_empty() && current != "__browse__" && !current.starts_with("__remove__") {
        rows.push(crate::choice::ChoiceItem::option(
            &format!("__remove__{current}"),
            &t_pub("settings_wall_remove_row"),
        ));
    }
    rows
}
