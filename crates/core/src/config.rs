//! IdleTrigger configuration: TOML load, validate, save.
//!
//! Field names and the annotated template are kept identical to the Go
//! version so existing `IdleTrigger.toml` files load unchanged. Saving
//! edits the loaded document in place via `toml_edit`, preserving every
//! other key and comment; the file is never rewritten on load.

use std::path::Path;

/// The annotated template, identical to `IdleTrigger.example.toml`.
const TEMPLATE: &str = include_str!("../../../IdleTrigger.example.toml");

#[derive(Debug, Clone)]
pub struct Config {
    pub language: String,
    pub logging_enabled: bool,
    pub nosleep_enabled: bool,
    pub keep_screen_on: bool,
    pub nosleep_on_battery: bool,
    pub nosleep_battery_threshold: i32,
    pub idle_enabled: bool,
    pub idle_timeout_minutes: i32,
    pub idle_action: String,
    pub idle_warning_seconds: i32,
    pub idle_enhanced_monitor: bool,
    pub automation_enabled: bool,
    pub hotkeys_enabled: bool,
    pub lock_keys_enabled: bool,
    pub lock_keys_caps_enabled: bool,
    pub lock_keys_num_enabled: bool,
    pub lock_keys_scroll_enabled: bool,
    pub lock_keys_skip_fullscreen: bool,
    pub theme_switch_enabled: bool,
    pub theme_mode: String,
    pub theme_light_time: String,
    pub theme_dark_time: String,
    pub theme_ip_location_enabled: bool,
    pub theme_dark_on_battery: bool,
    pub theme_skip_fullscreen: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: "auto".to_string(),
            logging_enabled: false,
            nosleep_enabled: false,
            keep_screen_on: false,
            nosleep_on_battery: false,
            nosleep_battery_threshold: 20,
            idle_enabled: false,
            idle_timeout_minutes: 30,
            idle_action: "sleep".to_string(),
            idle_warning_seconds: 30,
            idle_enhanced_monitor: false,
            automation_enabled: true,
            hotkeys_enabled: false,
            lock_keys_enabled: false,
            lock_keys_caps_enabled: true,
            lock_keys_num_enabled: true,
            lock_keys_scroll_enabled: true,
            lock_keys_skip_fullscreen: true,
            theme_switch_enabled: false,
            theme_mode: "sunrise".to_string(),
            theme_light_time: "07:00".to_string(),
            theme_dark_time: "19:00".to_string(),
            theme_ip_location_enabled: false,
            theme_dark_on_battery: true,
            theme_skip_fullscreen: true,
        }
    }
}

impl Config {
    /// Clamp helper used after loading user values.
    pub fn sanitized(mut self) -> Self {
        if !matches!(self.language.as_str(), "auto" | "en" | "zh-CN") {
            self.language = "auto".into();
        }
        if !matches!(
            self.idle_action.as_str(),
            "sleep" | "hibernate" | "shutdown" | "lock" | "restart"
        ) {
            self.idle_action = "lock".into();
        }
        self.idle_timeout_minutes = self.idle_timeout_minutes.clamp(1, 7 * 24 * 60);
        self.idle_warning_seconds = self.idle_warning_seconds.clamp(0, 3600);
        self.nosleep_battery_threshold = self.nosleep_battery_threshold.clamp(0, 100);
        if !matches!(self.theme_mode.as_str(), "fixed" | "sunrise") {
            self.theme_mode = "sunrise".into();
        }
        self
    }
}

/// The loaded configuration plus the parsed document used for round-trips.
pub struct Loaded {
    pub config: Config,
    /// Human-readable load problem; defaults were used for broken fields.
    pub load_error: Option<String>,
    /// The document the file was parsed into (template-based when absent).
    pub document: toml_edit::DocumentMut,
    /// True when no config file existed yet.
    pub created_from_template: bool,
}

pub fn load(path: &Path) -> Loaded {
    let defaults = Config::default();
    match std::fs::read_to_string(path) {
        Ok(text) => match text.parse::<toml_edit::DocumentMut>() {
            Ok(document) => Loaded {
                config: read_config(&document, defaults).sanitized(),
                load_error: None,
                document,
                created_from_template: false,
            },
            Err(err) => Loaded {
                config: defaults.sanitized(),
                load_error: Some(format!("parse error: {err}")),
                document: template_document(),
                created_from_template: true,
            },
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Loaded {
            config: defaults.sanitized(),
            load_error: None,
            document: template_document(),
            created_from_template: true,
        },
        Err(err) => Loaded {
            config: defaults.sanitized(),
            load_error: Some(format!("read error: {err}")),
            document: template_document(),
            created_from_template: true,
        },
    }
}

fn template_document() -> toml_edit::DocumentMut {
    TEMPLATE
        .parse::<toml_edit::DocumentMut>()
        .expect("embedded template must parse")
}

fn read_config(document: &toml_edit::DocumentMut, defaults: Config) -> Config {
    let as_bool = |key: &str| document.get(key).and_then(|v| v.as_bool());
    let as_int = |key: &str| document.get(key).and_then(|v| v.as_integer());
    let as_str = |key: &str| document.get(key).and_then(|v| v.as_str());
    Config {
        language: as_str("language").unwrap_or(&defaults.language).to_string(),
        logging_enabled: as_bool("logging_enabled").unwrap_or(defaults.logging_enabled),
        nosleep_enabled: as_bool("nosleep_enabled").unwrap_or(defaults.nosleep_enabled),
        keep_screen_on: as_bool("keep_screen_on").unwrap_or(defaults.keep_screen_on),
        nosleep_on_battery: as_bool("nosleep_on_battery").unwrap_or(defaults.nosleep_on_battery),
        nosleep_battery_threshold: as_int("nosleep_battery_threshold")
            .map(|v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
            .unwrap_or(defaults.nosleep_battery_threshold),
        idle_enabled: as_bool("idle_enabled").unwrap_or(defaults.idle_enabled),
        idle_timeout_minutes: as_int("idle_timeout_minutes")
            .map(|v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
            .unwrap_or(defaults.idle_timeout_minutes),
        idle_action: as_str("idle_action")
            .unwrap_or(&defaults.idle_action)
            .to_string(),
        idle_warning_seconds: as_int("idle_warning_seconds")
            .map(|v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
            .unwrap_or(defaults.idle_warning_seconds),
        idle_enhanced_monitor: as_bool("idle_enhanced_monitor")
            .unwrap_or(defaults.idle_enhanced_monitor),
        automation_enabled: as_bool("automation_enabled").unwrap_or(defaults.automation_enabled),
        hotkeys_enabled: as_bool("hotkeys_enabled").unwrap_or(defaults.hotkeys_enabled),
        lock_keys_enabled: as_bool("lock_keys_enabled").unwrap_or(defaults.lock_keys_enabled),
        lock_keys_caps_enabled: as_bool("lock_keys_caps_enabled")
            .unwrap_or(defaults.lock_keys_caps_enabled),
        lock_keys_num_enabled: as_bool("lock_keys_num_enabled")
            .unwrap_or(defaults.lock_keys_num_enabled),
        lock_keys_scroll_enabled: as_bool("lock_keys_scroll_enabled")
            .unwrap_or(defaults.lock_keys_scroll_enabled),
        lock_keys_skip_fullscreen: as_bool("lock_keys_skip_fullscreen")
            .unwrap_or(defaults.lock_keys_skip_fullscreen),
        theme_switch_enabled: as_bool("theme_switch_enabled")
            .unwrap_or(defaults.theme_switch_enabled),
        theme_mode: as_str("theme_mode")
            .unwrap_or(&defaults.theme_mode)
            .to_string(),
        theme_light_time: as_str("theme_light_time")
            .unwrap_or(&defaults.theme_light_time)
            .to_string(),
        theme_dark_time: as_str("theme_dark_time")
            .unwrap_or(&defaults.theme_dark_time)
            .to_string(),
        theme_ip_location_enabled: as_bool("theme_ip_location_enabled")
            .unwrap_or(defaults.theme_ip_location_enabled),
        theme_dark_on_battery: as_bool("theme_dark_on_battery")
            .unwrap_or(defaults.theme_dark_on_battery),
        theme_skip_fullscreen: as_bool("theme_skip_fullscreen")
            .unwrap_or(defaults.theme_skip_fullscreen),
    }
}

/// Writes `config` back into `document` and saves atomically (tmp + rename).
pub fn save(
    path: &Path,
    document: &mut toml_edit::DocumentMut,
    config: &Config,
) -> std::io::Result<()> {
    let mut candidate = document.clone();
    save_candidate(path, &mut candidate, config)?;
    *document = candidate;
    Ok(())
}

fn save_candidate(
    path: &Path,
    document: &mut toml_edit::DocumentMut,
    config: &Config,
) -> std::io::Result<()> {
    let set_value = |doc: &mut toml_edit::DocumentMut, key: &str, mut value: toml_edit::Value| {
        if let Some(old) = doc.get(key).and_then(toml_edit::Item::as_value) {
            *value.decor_mut() = old.decor().clone();
        }
        doc[key] = toml_edit::Item::Value(value);
    };
    let set_bool = |doc: &mut toml_edit::DocumentMut, key: &str, value: bool| {
        set_value(doc, key, value.into())
    };
    let set_int = |doc: &mut toml_edit::DocumentMut, key: &str, value: i32| {
        set_value(doc, key, i64::from(value).into())
    };
    let set_str = |doc: &mut toml_edit::DocumentMut, key: &str, value: &str| {
        set_value(doc, key, value.into())
    };
    set_bool(document, "logging_enabled", config.logging_enabled);
    set_bool(document, "nosleep_enabled", config.nosleep_enabled);
    set_bool(document, "keep_screen_on", config.keep_screen_on);
    set_bool(document, "nosleep_on_battery", config.nosleep_on_battery);
    set_int(
        document,
        "nosleep_battery_threshold",
        config.nosleep_battery_threshold,
    );
    set_bool(document, "idle_enabled", config.idle_enabled);
    set_int(
        document,
        "idle_timeout_minutes",
        config.idle_timeout_minutes,
    );
    set_str(document, "idle_action", &config.idle_action);
    set_int(
        document,
        "idle_warning_seconds",
        config.idle_warning_seconds,
    );
    set_bool(
        document,
        "idle_enhanced_monitor",
        config.idle_enhanced_monitor,
    );
    set_bool(document, "automation_enabled", config.automation_enabled);
    set_bool(document, "hotkeys_enabled", config.hotkeys_enabled);
    set_bool(document, "lock_keys_enabled", config.lock_keys_enabled);
    set_bool(
        document,
        "lock_keys_caps_enabled",
        config.lock_keys_caps_enabled,
    );
    set_bool(
        document,
        "lock_keys_num_enabled",
        config.lock_keys_num_enabled,
    );
    set_bool(
        document,
        "lock_keys_scroll_enabled",
        config.lock_keys_scroll_enabled,
    );
    set_bool(
        document,
        "lock_keys_skip_fullscreen",
        config.lock_keys_skip_fullscreen,
    );
    set_bool(
        document,
        "theme_switch_enabled",
        config.theme_switch_enabled,
    );
    set_str(document, "theme_mode", &config.theme_mode);
    set_str(document, "theme_light_time", &config.theme_light_time);
    set_str(document, "theme_dark_time", &config.theme_dark_time);
    set_bool(
        document,
        "theme_ip_location_enabled",
        config.theme_ip_location_enabled,
    );
    set_bool(
        document,
        "theme_dark_on_battery",
        config.theme_dark_on_battery,
    );
    set_bool(
        document,
        "theme_skip_fullscreen",
        config.theme_skip_fullscreen,
    );
    set_str(document, "language", &config.language);

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    tmp.push(format!(
        ".{}-{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config"),
        std::process::id()
    ));
    std::fs::write(&tmp, document.to_string())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod save_tests {
    use super::*;
    #[test]
    fn save_load_preserves_comments_and_ui_limits() {
        let path = std::env::temp_dir().join(format!(
            "idletrigger-roundtrip-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut doc: toml_edit::DocumentMut =
            "# user note\nnosleep_enabled = false # inline note\ncustom = 42\n"
                .parse()
                .unwrap();
        let config = Config {
            nosleep_enabled: true,
            idle_timeout_minutes: 10080,
            idle_warning_seconds: 3600,
            nosleep_battery_threshold: 0,
            ..Config::default()
        };
        save(&path, &mut doc, &config).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let loaded = load(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(text.contains("nosleep_enabled = true # inline note"));
        assert!(text.contains("# user note"));
        assert_eq!(loaded.document["custom"].as_integer(), Some(42));
        assert_eq!(loaded.config.idle_timeout_minutes, 10080);
        assert_eq!(loaded.config.idle_warning_seconds, 3600);
        assert_eq!(loaded.config.nosleep_battery_threshold, 0);
    }
    #[test]
    fn failed_save_does_not_publish_candidate_document() {
        let mut doc: toml_edit::DocumentMut =
            "nosleep_enabled = false # preserved\n".parse().unwrap();
        let original = doc.to_string();
        let config = Config {
            nosleep_enabled: true,
            ..Config::default()
        };
        let missing = std::env::temp_dir().join(format!(
            "idletrigger-no-parent-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(save(&missing.join("config.toml"), &mut doc, &config).is_err());
        assert_eq!(doc.to_string(), original);
    }
}
