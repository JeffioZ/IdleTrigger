//! Settings values, validation and three-way merge. No HWNDs or runtime globals.
use idletrigger_core::config::Config;

pub(crate) const IDLE_ACTIONS: [&str; 6] = [
    idletrigger_core::automation::ACTION_LOCK,
    idletrigger_core::automation::ACTION_SLEEP,
    idletrigger_core::automation::ACTION_HIBERNATE,
    idletrigger_core::automation::ACTION_SHUTDOWN,
    idletrigger_core::automation::ACTION_RESTART,
    idletrigger_core::automation::ACTION_SCREEN_OFF,
];
pub(crate) const LANG_VALUES: [&str; 3] = ["auto", "en", "zh-CN"];

#[derive(PartialEq)]
pub(crate) struct Draft {
    pub keep_screen: bool,
    pub battery_allowed: bool,
    pub pause_on_lock: bool,
    pub battery_threshold: Option<i32>,
    pub idle_timeout: Option<i32>,
    pub warning: Option<i32>,
    pub idle_action_idx: usize,
    pub idle_enhanced: bool,
    pub theme_mode_idx: usize,
    pub light_time: String,
    pub dark_time: String,
    pub location_idx: usize,
    pub theme_battery: bool,
    pub theme_fullscreen: bool,
    pub light_wallpaper: String,
    pub dark_wallpaper: String,
    pub wallpapers: Vec<String>,
    pub light_cursor: String,
    pub dark_cursor: String,
    pub language_idx: usize,
    pub lock_keys: bool,
    pub caps: bool,
    pub num: bool,
    pub scroll: bool,
    pub lock_fullscreen: bool,
    pub hotkeys: bool,
    pub autostart: bool,
    pub logging: bool,
    pub update_auto: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum ValidationField {
    NotificationKeys,
    BatteryThreshold,
    IdleTimeout,
    Warning,
    LightTime,
    DarkTime,
}

/// Validation identifies fields; page selection and focus belong to the window.
pub(crate) fn validate_draft(draft: &Draft) -> Option<(ValidationField, &'static str)> {
    if draft.lock_keys && !(draft.caps || draft.num || draft.scroll) {
        return Some((
            ValidationField::NotificationKeys,
            "settings_error_notification_keys",
        ));
    }
    match draft.battery_threshold {
        Some(v) if (0..=100).contains(&v) => {}
        _ => {
            return Some((
                ValidationField::BatteryThreshold,
                "settings_error_battery_threshold",
            ));
        }
    }
    match draft.idle_timeout {
        Some(v) if (1..=idletrigger_core::automation::MAX_IDLE_MINUTES).contains(&v) => {}
        _ => return Some((ValidationField::IdleTimeout, "settings_error_idle_timeout")),
    }
    match draft.warning {
        Some(v) if (idletrigger_core::automation::MIN_WARNING_SECONDS..=3600).contains(&v) => {}
        _ => return Some((ValidationField::Warning, "settings_error_warning_seconds")),
    }
    // The fixed-schedule times are hidden and unused in sunrise mode;
    // validating them would report against invisible fields. Saving keeps
    // the baseline times if these hidden edits are incomplete.
    if draft.theme_mode_idx != 1 {
        if !valid_time(&draft.light_time) {
            return Some((ValidationField::LightTime, "settings_error_light_time"));
        }
        if !valid_time(&draft.dark_time) {
            return Some((ValidationField::DarkTime, "settings_error_dark_time"));
        }
        // Equal boundaries make the light window empty — the schedule would
        // read as dark all day while the configuration still looks valid.
        if draft.light_time == draft.dark_time {
            return Some((ValidationField::DarkTime, "settings_error_same_time"));
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

impl Draft {
    pub fn from_config(c: &Config, autostart: bool) -> Self {
        Self {
            keep_screen: c.keep_screen_on,
            battery_allowed: c.nosleep_on_battery,
            pause_on_lock: c.nosleep_pause_on_lock,
            battery_threshold: Some(c.nosleep_battery_threshold),
            idle_timeout: Some(c.idle_timeout_minutes),
            warning: Some(c.idle_warning_seconds),
            idle_action_idx: IDLE_ACTIONS
                .iter()
                .position(|a| *a == c.idle_action)
                .unwrap_or(0),
            idle_enhanced: c.idle_enhanced_monitor,
            theme_mode_idx: usize::from(c.theme_mode == "sunrise"),
            light_time: c.theme_light_time.clone(),
            dark_time: c.theme_dark_time.clone(),
            location_idx: usize::from(c.theme_ip_location_enabled),
            theme_battery: c.theme_dark_on_battery,
            theme_fullscreen: c.theme_skip_fullscreen,
            light_wallpaper: c.theme_light_wallpaper.clone(),
            dark_wallpaper: c.theme_dark_wallpaper.clone(),
            wallpapers: c.theme_wallpapers.clone(),
            light_cursor: c.theme_light_cursor_scheme.clone(),
            dark_cursor: c.theme_dark_cursor_scheme.clone(),
            language_idx: LANG_VALUES
                .iter()
                .position(|value| *value == c.language)
                .unwrap_or(0),
            lock_keys: c.lock_keys_enabled,
            caps: c.lock_keys_caps_enabled,
            num: c.lock_keys_num_enabled,
            scroll: c.lock_keys_scroll_enabled,
            lock_fullscreen: c.lock_keys_skip_fullscreen,
            hotkeys: c.hotkeys_enabled,
            autostart,
            logging: c.logging_enabled,
            update_auto: c.update_check_enabled,
        }
    }
}

pub(crate) fn draft_differs(draft: &Draft, base: &Config, autostart: bool) -> bool {
    *draft != Draft::from_config(base, autostart)
}

/// Project validated controls onto their opening baseline. Fields outside
/// Settings are never part of the edit, including runtime restore snapshots.
pub(crate) fn draft_config(draft: &Draft, base: &Config) -> Config {
    let mut c = base.clone();
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
    if valid_time(&draft.light_time)
        && valid_time(&draft.dark_time)
        && draft.light_time != draft.dark_time
    {
        c.theme_light_time = draft.light_time.clone();
        c.theme_dark_time = draft.dark_time.clone();
    }
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
    c.update_check_enabled = draft.update_auto;
    c
}

/// Three-way merge against the current writer-owned candidate. Coupled
/// fields stay together so independently valid edits cannot produce equal
/// schedule boundaries or an enabled notification with no selected keys.
/// Confirmation applies only to the conflicting values read at that point;
/// another writer changing those values requires a fresh confirmation.
pub(crate) fn merge_settings(
    base: &Config,
    edited: &Config,
    live: &mut Config,
    approved: Option<&Config>,
) -> Vec<&'static str> {
    let mut conflicts = Vec::new();
    macro_rules! merge {
        ($label:literal, $($field:ident),+ $(,)?) => {
            if false $(|| edited.$field != base.$field)+ {
                let changed_elsewhere = false $(|| live.$field != base.$field)+;
                let already_matches = true $(&& live.$field == edited.$field)+;
                let confirmed = approved.is_some_and(|known| true $(&& live.$field == known.$field)+);
                if changed_elsewhere && !already_matches && !confirmed {
                    conflicts.push($label);
                } else {
                    $(live.$field.clone_from(&edited.$field);)+
                }
            }
        };
    }
    merge!("settings_keep_screen", keep_screen_on);
    merge!("settings_battery_allowed", nosleep_on_battery);
    merge!("settings_pause_on_lock", nosleep_pause_on_lock);
    merge!("settings_battery_threshold", nosleep_battery_threshold);
    merge!("settings_idle_timeout_minutes", idle_timeout_minutes);
    merge!("settings_idle_action", idle_action);
    merge!("settings_idle_warning_seconds", idle_warning_seconds);
    merge!("settings_idle_title", idle_enhanced_monitor);
    merge!(
        "settings_theme_schedule_group",
        theme_mode,
        theme_light_time,
        theme_dark_time
    );
    merge!("settings_location_source", theme_ip_location_enabled);
    merge!("settings_theme_behavior_group", theme_dark_on_battery);
    merge!("settings_theme_behavior_group", theme_skip_fullscreen);
    merge!(
        "settings_row_wallpaper",
        theme_light_wallpaper,
        theme_dark_wallpaper,
        theme_wallpapers
    );
    merge!("settings_row_cursor", theme_light_cursor_scheme);
    merge!("settings_row_cursor", theme_dark_cursor_scheme);
    merge!("settings_language", language);
    merge!(
        "settings_lock_keys",
        lock_keys_enabled,
        lock_keys_caps_enabled,
        lock_keys_num_enabled,
        lock_keys_scroll_enabled
    );
    merge!("settings_lock_keys", lock_keys_skip_fullscreen);
    merge!("menu_hotkeys", hotkeys_enabled);
    merge!("menu_logging", logging_enabled);
    merge!("settings_update_auto", update_check_enabled);
    conflicts.dedup();
    conflicts
}

/// Side-specific effects of a successful settings transaction. Changing
/// the recent-image library alone does not reapply the desktop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AppearanceChanges {
    pub wallpaper: [bool; 2], // light, dark
    pub cursor: [bool; 2],
}

impl AppearanceChanges {
    pub const ALL: Self = Self {
        wallpaper: [true; 2],
        cursor: [true; 2],
    };

    pub fn any(self) -> bool {
        self.wallpaper
            .into_iter()
            .chain(self.cursor)
            .any(|changed| changed)
    }

    pub fn merge(&mut self, other: Self) {
        for side in 0..2 {
            self.wallpaper[side] |= other.wallpaper[side];
            self.cursor[side] |= other.cursor[side];
        }
    }
}

#[derive(Default)]
pub(crate) struct Changes {
    pub appearance: AppearanceChanges,
    pub hotkeys: bool,
    pub language: bool,
}

impl Changes {
    pub fn between(before: &Config, after: &Config) -> Self {
        Self {
            appearance: AppearanceChanges {
                wallpaper: [
                    before.theme_light_wallpaper != after.theme_light_wallpaper,
                    before.theme_dark_wallpaper != after.theme_dark_wallpaper,
                ],
                cursor: [
                    before.theme_light_cursor_scheme != after.theme_light_cursor_scheme,
                    before.theme_dark_cursor_scheme != after.theme_dark_cursor_scheme,
                ],
            },
            hotkeys: before.hotkeys_enabled != after.hotkeys_enabled,
            language: before.language != after.language,
        }
    }
}
