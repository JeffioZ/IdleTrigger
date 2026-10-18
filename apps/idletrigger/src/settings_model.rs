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

#[cfg(test)]
mod tests {
    use super::*;

    /// One representative Config mutation per merge invocation. Every closure
    /// provably changes its field against any baseline, so a drifted default
    /// cannot turn a scenario vacuous.
    type Rep = (&'static str, Box<dyn Fn(&mut Config)>);

    fn representatives() -> Vec<Rep> {
        let mut reps: Vec<Rep> = Vec::new();
        reps.push((
            "keep_screen",
            Box::new(|c: &mut Config| c.keep_screen_on = !c.keep_screen_on),
        ));
        reps.push((
            "battery_allowed",
            Box::new(|c: &mut Config| c.nosleep_on_battery = !c.nosleep_on_battery),
        ));
        reps.push((
            "pause_on_lock",
            Box::new(|c: &mut Config| c.nosleep_pause_on_lock = !c.nosleep_pause_on_lock),
        ));
        reps.push((
            "battery_threshold",
            Box::new(|c: &mut Config| {
                c.nosleep_battery_threshold = if c.nosleep_battery_threshold == 77 {
                    78
                } else {
                    77
                }
            }),
        ));
        reps.push((
            "idle_timeout",
            Box::new(|c: &mut Config| {
                c.idle_timeout_minutes = if c.idle_timeout_minutes == 91 { 92 } else { 91 }
            }),
        ));
        reps.push((
            "idle_action",
            Box::new(|c: &mut Config| {
                let index = IDLE_ACTIONS
                    .iter()
                    .position(|action| *action == c.idle_action)
                    .unwrap_or(0);
                c.idle_action = IDLE_ACTIONS[(index + 2) % IDLE_ACTIONS.len()].to_string();
            }),
        ));
        reps.push((
            "idle_warning",
            Box::new(|c: &mut Config| {
                c.idle_warning_seconds = if c.idle_warning_seconds == 501 {
                    502
                } else {
                    501
                }
            }),
        ));
        reps.push((
            "idle_enhanced",
            Box::new(|c: &mut Config| c.idle_enhanced_monitor = !c.idle_enhanced_monitor),
        ));
        reps.push((
            "theme_mode",
            Box::new(|c: &mut Config| {
                c.theme_mode = if c.theme_mode == "sunrise" {
                    "fixed"
                } else {
                    "sunrise"
                }
                .to_string()
            }),
        ));
        reps.push((
            "schedule_times",
            Box::new(|c: &mut Config| {
                c.theme_light_time = if c.theme_light_time == "07:31" {
                    "07:32"
                } else {
                    "07:31"
                }
                .to_string()
            }),
        ));
        reps.push((
            "location",
            Box::new(|c: &mut Config| c.theme_ip_location_enabled = !c.theme_ip_location_enabled),
        ));
        reps.push((
            "theme_battery",
            Box::new(|c: &mut Config| c.theme_dark_on_battery = !c.theme_dark_on_battery),
        ));
        reps.push((
            "theme_fullscreen",
            Box::new(|c: &mut Config| c.theme_skip_fullscreen = !c.theme_skip_fullscreen),
        ));
        reps.push((
            "wallpaper_value",
            Box::new(|c: &mut Config| {
                c.theme_light_wallpaper = format!("{}#external", c.theme_light_wallpaper)
            }),
        ));
        reps.push((
            "wallpaper_lib",
            Box::new(|c: &mut Config| c.theme_wallpapers.push("external-library.png".into())),
        ));
        reps.push((
            "cursor_light",
            Box::new(|c: &mut Config| {
                c.theme_light_cursor_scheme = if c.theme_light_cursor_scheme == "External Light" {
                    "External Light 2"
                } else {
                    "External Light"
                }
                .to_string()
            }),
        ));
        reps.push((
            "cursor_dark",
            Box::new(|c: &mut Config| {
                c.theme_dark_cursor_scheme = if c.theme_dark_cursor_scheme == "External Dark" {
                    "External Dark 2"
                } else {
                    "External Dark"
                }
                .to_string()
            }),
        ));
        reps.push((
            "language",
            Box::new(|c: &mut Config| {
                c.language = if c.language == "zh-CN" {
                    "auto"
                } else {
                    "zh-CN"
                }
                .to_string()
            }),
        ));
        reps.push((
            "lock_keys_enabled",
            Box::new(|c: &mut Config| c.lock_keys_enabled = !c.lock_keys_enabled),
        ));
        reps.push((
            "lock_fullscreen",
            Box::new(|c: &mut Config| c.lock_keys_skip_fullscreen = !c.lock_keys_skip_fullscreen),
        ));
        reps.push((
            "hotkeys",
            Box::new(|c: &mut Config| c.hotkeys_enabled = !c.hotkeys_enabled),
        ));
        reps.push((
            "logging",
            Box::new(|c: &mut Config| c.logging_enabled = !c.logging_enabled),
        ));
        reps.push((
            "update_auto",
            Box::new(|c: &mut Config| c.update_check_enabled = !c.update_check_enabled),
        ));
        reps
    }

    struct Row {
        name: &'static str,
        edit: Box<dyn Fn(&mut Draft)>,
        /// Representatives belonging to the same merge invocation: these are
        /// the external edits that must conflict instead of surviving.
        same_group: Vec<&'static str>,
        conflict: Option<&'static str>,
    }

    fn rows() -> Vec<Row> {
        let mut rows: Vec<Row> = Vec::new();
        rows.push(Row {
            name: "keep_screen",
            edit: Box::new(|d: &mut Draft| d.keep_screen = !d.keep_screen),
            same_group: vec!["keep_screen"],
            conflict: None,
        });
        rows.push(Row {
            name: "battery_allowed",
            edit: Box::new(|d: &mut Draft| d.battery_allowed = !d.battery_allowed),
            same_group: vec!["battery_allowed"],
            conflict: None,
        });
        rows.push(Row {
            name: "pause_on_lock",
            edit: Box::new(|d: &mut Draft| d.pause_on_lock = !d.pause_on_lock),
            same_group: vec!["pause_on_lock"],
            conflict: None,
        });
        rows.push(Row {
            name: "battery_threshold",
            edit: Box::new(|d: &mut Draft| {
                let value = d.battery_threshold.unwrap_or(0);
                d.battery_threshold = Some(if value == 65 { 66 } else { 65 });
            }),
            same_group: vec!["battery_threshold"],
            conflict: Some("settings_battery_threshold"),
        });
        rows.push(Row {
            name: "idle_timeout",
            edit: Box::new(|d: &mut Draft| {
                let value = d.idle_timeout.unwrap_or(0);
                d.idle_timeout = Some(if value == 45 { 46 } else { 45 });
            }),
            same_group: vec!["idle_timeout"],
            conflict: Some("settings_idle_timeout_minutes"),
        });
        rows.push(Row {
            name: "idle_action",
            edit: Box::new(|d: &mut Draft| {
                d.idle_action_idx = (d.idle_action_idx + 1) % IDLE_ACTIONS.len();
            }),
            same_group: vec!["idle_action"],
            conflict: Some("settings_idle_action"),
        });
        rows.push(Row {
            name: "idle_warning",
            edit: Box::new(|d: &mut Draft| {
                let value = d.warning.unwrap_or(0);
                d.warning = Some(if value == 301 { 302 } else { 301 });
            }),
            same_group: vec!["idle_warning"],
            conflict: Some("settings_idle_warning_seconds"),
        });
        rows.push(Row {
            name: "idle_enhanced",
            edit: Box::new(|d: &mut Draft| d.idle_enhanced = !d.idle_enhanced),
            same_group: vec!["idle_enhanced"],
            conflict: None,
        });
        rows.push(Row {
            name: "theme_mode",
            edit: Box::new(|d: &mut Draft| d.theme_mode_idx = 1 - d.theme_mode_idx),
            same_group: vec!["theme_mode", "schedule_times"],
            conflict: Some("settings_theme_schedule_group"),
        });
        rows.push(Row {
            name: "light_time",
            edit: Box::new(|d: &mut Draft| {
                let pick = ["05:11", "05:12", "05:13"]
                    .into_iter()
                    .find(|time| *time != d.light_time && *time != d.dark_time)
                    .unwrap();
                d.light_time = pick.to_string();
            }),
            same_group: vec!["theme_mode", "schedule_times"],
            conflict: Some("settings_theme_schedule_group"),
        });
        rows.push(Row {
            name: "dark_time",
            edit: Box::new(|d: &mut Draft| {
                let pick = ["21:11", "21:12", "21:13"]
                    .into_iter()
                    .find(|time| *time != d.light_time && *time != d.dark_time)
                    .unwrap();
                d.dark_time = pick.to_string();
            }),
            same_group: vec!["theme_mode", "schedule_times"],
            conflict: Some("settings_theme_schedule_group"),
        });
        rows.push(Row {
            name: "location",
            edit: Box::new(|d: &mut Draft| d.location_idx = 1 - d.location_idx),
            same_group: vec!["location"],
            conflict: None,
        });
        rows.push(Row {
            name: "theme_battery",
            edit: Box::new(|d: &mut Draft| d.theme_battery = !d.theme_battery),
            same_group: vec!["theme_battery"],
            conflict: None,
        });
        rows.push(Row {
            name: "theme_fullscreen",
            edit: Box::new(|d: &mut Draft| d.theme_fullscreen = !d.theme_fullscreen),
            same_group: vec!["theme_fullscreen"],
            conflict: None,
        });
        rows.push(Row {
            name: "light_wallpaper",
            edit: Box::new(|d: &mut Draft| {
                d.light_wallpaper = format!("{}#mine", d.light_wallpaper);
            }),
            same_group: vec!["wallpaper_value", "wallpaper_lib"],
            conflict: Some("settings_row_wallpaper"),
        });
        rows.push(Row {
            name: "dark_wallpaper",
            edit: Box::new(|d: &mut Draft| {
                d.dark_wallpaper = format!("{}#mine", d.dark_wallpaper);
            }),
            same_group: vec!["wallpaper_value", "wallpaper_lib"],
            conflict: Some("settings_row_wallpaper"),
        });
        rows.push(Row {
            name: "wallpapers",
            edit: Box::new(|d: &mut Draft| d.wallpapers.push("library-entry.png".into())),
            same_group: vec!["wallpaper_value", "wallpaper_lib"],
            conflict: Some("settings_row_wallpaper"),
        });
        rows.push(Row {
            name: "light_cursor",
            edit: Box::new(|d: &mut Draft| {
                d.light_cursor = format!("{}#mine", d.light_cursor);
            }),
            same_group: vec!["cursor_light"],
            conflict: Some("settings_row_cursor"),
        });
        rows.push(Row {
            name: "dark_cursor",
            edit: Box::new(|d: &mut Draft| {
                d.dark_cursor = format!("{}#mine", d.dark_cursor);
            }),
            same_group: vec!["cursor_dark"],
            conflict: Some("settings_row_cursor"),
        });
        rows.push(Row {
            name: "language",
            edit: Box::new(|d: &mut Draft| {
                let current = LANG_VALUES[d.language_idx.min(2)];
                let target = if current == "zh-CN" { "auto" } else { "zh-CN" };
                d.language_idx = LANG_VALUES
                    .iter()
                    .position(|value| *value != current && *value != target)
                    .unwrap();
            }),
            same_group: vec!["language"],
            conflict: Some("settings_language"),
        });
        rows.push(Row {
            name: "lock_keys_caps",
            edit: Box::new(|d: &mut Draft| d.caps = !d.caps),
            same_group: vec!["lock_keys_enabled"],
            conflict: Some("settings_lock_keys"),
        });
        rows.push(Row {
            name: "lock_keys_num",
            edit: Box::new(|d: &mut Draft| d.num = !d.num),
            same_group: vec!["lock_keys_enabled"],
            conflict: Some("settings_lock_keys"),
        });
        rows.push(Row {
            name: "lock_keys_scroll",
            edit: Box::new(|d: &mut Draft| d.scroll = !d.scroll),
            same_group: vec!["lock_keys_enabled"],
            conflict: Some("settings_lock_keys"),
        });
        rows.push(Row {
            name: "lock_fullscreen",
            edit: Box::new(|d: &mut Draft| d.lock_fullscreen = !d.lock_fullscreen),
            same_group: vec!["lock_fullscreen"],
            conflict: None,
        });
        rows.push(Row {
            name: "hotkeys",
            edit: Box::new(|d: &mut Draft| d.hotkeys = !d.hotkeys),
            same_group: vec!["hotkeys"],
            conflict: None,
        });
        rows.push(Row {
            name: "logging",
            edit: Box::new(|d: &mut Draft| d.logging = !d.logging),
            same_group: vec!["logging"],
            conflict: None,
        });
        rows.push(Row {
            name: "update_auto",
            edit: Box::new(|d: &mut Draft| d.update_auto = !d.update_auto),
            same_group: vec!["update_auto"],
            conflict: None,
        });
        rows
    }

    /// The save invariant, per draft field: the projection changes something,
    /// every projected field belongs to a merge group, external edits to
    /// unrelated groups survive the save, and an external edit inside the
    /// same group conflicts until the user confirms the current values.
    #[test]
    fn projection_covers_every_merge_group_and_preserves_unrelated_live_edits() {
        let base = Config::default();
        let reps = representatives();
        for row in rows() {
            let mut draft = Draft::from_config(&base, false);
            (row.edit)(&mut draft);
            let edited = draft_config(&draft, &base);
            assert_ne!(edited, base, "vacuous edit row: {}", row.name);

            // Merging onto a pristine live config reproduces the projection
            // exactly: no projected field may fall outside a merge group.
            let mut pristine = base.clone();
            let conflicts = merge_settings(&base, &edited, &mut pristine, None);
            assert!(conflicts.is_empty(), "row {}: {conflicts:?}", row.name);
            assert_eq!(
                pristine, edited,
                "projection outside merge groups: {}",
                row.name
            );

            // External edits to every other group survive this save.
            let mut live = base.clone();
            let mut expectation = edited.clone();
            for (name, mutate) in &reps {
                if row.same_group.contains(name) {
                    continue;
                }
                mutate(&mut live);
                mutate(&mut expectation);
            }
            assert_ne!(live, base, "vacuous representative set: {}", row.name);
            let conflicts = merge_settings(&base, &edited, &mut live, None);
            assert!(conflicts.is_empty(), "row {}: {conflicts:?}", row.name);
            assert_eq!(live, expectation, "unrelated live edit lost: {}", row.name);

            // An external edit inside the same merge invocation conflicts...
            let mut conflicted = base.clone();
            for (name, mutate) in &reps {
                if row.same_group.contains(name) {
                    mutate(&mut conflicted);
                }
            }
            assert_ne!(conflicted, base, "vacuous same-group rep: {}", row.name);
            match row.conflict {
                Some(label) => {
                    let conflicts = merge_settings(&base, &edited, &mut conflicted, None);
                    assert_eq!(conflicts, vec![label], "row {}", row.name);
                    // ...and a confirmation captured at those exact live values
                    // applies the edit over them.
                    let approved = conflicted.clone();
                    let mut confirmed = conflicted.clone();
                    let conflicts = merge_settings(&base, &edited, &mut confirmed, Some(&approved));
                    assert!(conflicts.is_empty(), "row {}: {conflicts:?}", row.name);
                    assert_eq!(
                        confirmed, edited,
                        "confirmation must apply the edit: {}",
                        row.name
                    );
                }
                None => {
                    // Single-switch groups have only two values: the external
                    // edit necessarily equals the user's choice and applies.
                    let conflicts = merge_settings(&base, &edited, &mut conflicted, None);
                    assert!(conflicts.is_empty(), "row {}: {conflicts:?}", row.name);
                    assert_eq!(
                        conflicted, edited,
                        "coinciding external edit must apply: {}",
                        row.name
                    );
                }
            }
        }
    }

    #[test]
    fn draft_roundtrip_preserves_a_valid_config() {
        let base = Config {
            keep_screen_on: true,
            nosleep_on_battery: true,
            nosleep_pause_on_lock: true,
            nosleep_battery_threshold: 42,
            idle_timeout_minutes: 77,
            idle_action: "sleep".into(),
            idle_warning_seconds: 301,
            idle_enhanced_monitor: true,
            theme_mode: "sunrise".into(),
            theme_light_time: "06:31".into(),
            theme_dark_time: "18:47".into(),
            theme_ip_location_enabled: true,
            theme_dark_on_battery: true,
            theme_skip_fullscreen: true,
            theme_light_wallpaper: "C:\\light.png".into(),
            theme_dark_wallpaper: "C:\\dark.png".into(),
            theme_wallpapers: vec!["C:\\light.png".into(), "C:\\dark.png".into()],
            theme_light_cursor_scheme: "Aero".into(),
            theme_dark_cursor_scheme: "Windows Black".into(),
            language: "zh-CN".into(),
            lock_keys_enabled: true,
            lock_keys_caps_enabled: true,
            lock_keys_num_enabled: false,
            lock_keys_scroll_enabled: true,
            lock_keys_skip_fullscreen: true,
            hotkeys_enabled: true,
            logging_enabled: true,
            update_check_enabled: true,
            ..Config::default()
        };
        assert_eq!(draft_config(&Draft::from_config(&base, false), &base), base);
        // Hidden schedule fields (sunrise mode) keep the baseline times even
        // when the invisible edits hold garbage.
        let mut draft = Draft::from_config(&base, false);
        draft.light_time = "garbage".into();
        draft.dark_time = draft.light_time.clone();
        let projected = draft_config(&draft, &base);
        assert_eq!(projected.theme_light_time, base.theme_light_time);
        assert_eq!(projected.theme_dark_time, base.theme_dark_time);
    }

    #[test]
    fn appearance_changes_are_side_specific_and_ignore_the_library() {
        let base = Config::default();
        let mut edited = base.clone();
        edited.theme_light_wallpaper = "C:\\light.png".into();
        let changes = Changes::between(&base, &edited);
        assert_eq!(changes.appearance.wallpaper, [true, false]);
        assert_eq!(changes.appearance.cursor, [false, false]);
        assert!(!changes.language);
        assert!(!changes.hotkeys);
        let mut library = base.clone();
        library.theme_wallpapers = vec!["C:\\new.png".into()];
        assert!(!Changes::between(&base, &library).appearance.any());
    }
}
