//! IdleTrigger configuration: TOML load, validate, save.
//!
//! Stable field names allow existing `IdleTrigger.toml` files to load.
//! Saving edits the loaded document in place via `toml_edit`, preserving
//! other keys and comments; loading never rewrites the file.

use std::path::Path;

/// The annotated template, identical to `IdleTrigger.example.toml`.
const TEMPLATE: &str = include_str!("../../../IdleTrigger.example.toml");

#[derive(Debug, Clone, PartialEq)]
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
        // System actions always carry a cancellable countdown: the idle path
        // shares the automation rules' minimum instead of allowing 0 (silent).
        self.idle_warning_seconds = self
            .idle_warning_seconds
            .clamp(crate::automation::MIN_WARNING_SECONDS, 3600);
        self.nosleep_battery_threshold = self.nosleep_battery_threshold.clamp(0, 100);
        if !matches!(self.theme_mode.as_str(), "fixed" | "sunrise") {
            self.theme_mode = "sunrise".into();
        }
        if !crate::automation::valid_hhmm(&self.theme_light_time) {
            self.theme_light_time = "07:00".into();
        }
        if !crate::automation::valid_hhmm(&self.theme_dark_time) {
            self.theme_dark_time = "19:00".into();
        }
        self
    }
}

/// The loaded configuration plus the parsed document used for round-trips.
pub struct Loaded {
    /// Exact input used by this load, for optimistic external-edit detection.
    pub source_text: Option<String>,
    pub config: Config,
    /// Human-readable load problem; defaults were used for broken fields.
    pub load_error: Option<String>,
    /// Present-but-mistyped fields that fell back to defaults. Softer than
    /// `load_error`: the document still parses, and saving once rewrites
    /// the offenders with correct types, so saving stays available.
    pub field_errors: Option<String>,
    /// The document the file was parsed into (template-based when absent).
    pub document: toml_edit::DocumentMut,
    /// True when no config file existed yet.
    pub created_from_template: bool,
}

pub fn load(path: &Path) -> Loaded {
    let defaults = Config::default();
    let base = Loaded {
        source_text: None,
        config: Config::default().sanitized(),
        load_error: None,
        field_errors: None,
        document: template_document(),
        created_from_template: true,
    };
    match std::fs::read_to_string(path) {
        Ok(text) => match text.parse::<toml_edit::DocumentMut>() {
            Ok(document) => {
                let mut bad_fields = Vec::new();
                let config = read_config(&document, defaults, &mut bad_fields).sanitized();
                let field_errors = (!bad_fields.is_empty()).then(|| bad_fields.join(", "));
                Loaded {
                    source_text: Some(text),
                    config,
                    load_error: None,
                    field_errors,
                    document,
                    created_from_template: false,
                }
            }
            Err(err) => Loaded {
                source_text: Some(text),
                load_error: Some(format!("parse error: {err}")),
                ..base
            },
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => base,
        Err(err) => Loaded {
            load_error: Some(format!("read error: {err}")),
            ..base
        },
    }
}

fn template_document() -> toml_edit::DocumentMut {
    TEMPLATE
        .parse::<toml_edit::DocumentMut>()
        .expect("embedded template must parse")
}

/// Reads one typed field; a present-but-mistyped value is recorded so the
/// loader can tell the user which fields fell back to defaults.
fn typed<'a, T>(
    document: &'a toml_edit::DocumentMut,
    key: &str,
    cast: impl FnOnce(&'a toml_edit::Item) -> Option<T>,
    bad_fields: &mut Vec<String>,
) -> Option<T> {
    let item = document.get(key)?;
    match cast(item) {
        Some(value) => Some(value),
        None => {
            bad_fields.push(key.to_string());
            None
        }
    }
}

fn as_bool(
    document: &toml_edit::DocumentMut,
    key: &str,
    bad_fields: &mut Vec<String>,
) -> Option<bool> {
    typed(document, key, |v| v.as_bool(), bad_fields)
}

fn as_int(
    document: &toml_edit::DocumentMut,
    key: &str,
    bad_fields: &mut Vec<String>,
) -> Option<i64> {
    typed(document, key, |v| v.as_integer(), bad_fields)
}

fn as_str<'a>(
    document: &'a toml_edit::DocumentMut,
    key: &str,
    bad_fields: &mut Vec<String>,
) -> Option<&'a str> {
    typed(document, key, |v| v.as_str(), bad_fields)
}

fn read_config(
    document: &toml_edit::DocumentMut,
    defaults: Config,
    bad_fields: &mut Vec<String>,
) -> Config {
    Config {
        language: as_str(document, "language", bad_fields)
            .unwrap_or(&defaults.language)
            .to_string(),
        logging_enabled: as_bool(document, "logging_enabled", bad_fields)
            .unwrap_or(defaults.logging_enabled),
        nosleep_enabled: as_bool(document, "nosleep_enabled", bad_fields)
            .unwrap_or(defaults.nosleep_enabled),
        keep_screen_on: as_bool(document, "keep_screen_on", bad_fields)
            .unwrap_or(defaults.keep_screen_on),
        nosleep_on_battery: as_bool(document, "nosleep_on_battery", bad_fields)
            .unwrap_or(defaults.nosleep_on_battery),
        nosleep_battery_threshold: as_int(document, "nosleep_battery_threshold", bad_fields)
            .map(|v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
            .unwrap_or(defaults.nosleep_battery_threshold),
        idle_enabled: as_bool(document, "idle_enabled", bad_fields)
            .unwrap_or(defaults.idle_enabled),
        idle_timeout_minutes: as_int(document, "idle_timeout_minutes", bad_fields)
            .map(|v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
            .unwrap_or(defaults.idle_timeout_minutes),
        idle_action: as_str(document, "idle_action", bad_fields)
            .unwrap_or(&defaults.idle_action)
            .to_string(),
        idle_warning_seconds: as_int(document, "idle_warning_seconds", bad_fields)
            .map(|v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
            .unwrap_or(defaults.idle_warning_seconds),
        idle_enhanced_monitor: as_bool(document, "idle_enhanced_monitor", bad_fields)
            .unwrap_or(defaults.idle_enhanced_monitor),
        automation_enabled: as_bool(document, "automation_enabled", bad_fields)
            .unwrap_or(defaults.automation_enabled),
        hotkeys_enabled: as_bool(document, "hotkeys_enabled", bad_fields)
            .unwrap_or(defaults.hotkeys_enabled),
        lock_keys_enabled: as_bool(document, "lock_keys_enabled", bad_fields)
            .unwrap_or(defaults.lock_keys_enabled),
        lock_keys_caps_enabled: as_bool(document, "lock_keys_caps_enabled", bad_fields)
            .unwrap_or(defaults.lock_keys_caps_enabled),
        lock_keys_num_enabled: as_bool(document, "lock_keys_num_enabled", bad_fields)
            .unwrap_or(defaults.lock_keys_num_enabled),
        lock_keys_scroll_enabled: as_bool(document, "lock_keys_scroll_enabled", bad_fields)
            .unwrap_or(defaults.lock_keys_scroll_enabled),
        lock_keys_skip_fullscreen: as_bool(document, "lock_keys_skip_fullscreen", bad_fields)
            .unwrap_or(defaults.lock_keys_skip_fullscreen),
        theme_switch_enabled: as_bool(document, "theme_switch_enabled", bad_fields)
            .unwrap_or(defaults.theme_switch_enabled),
        theme_mode: as_str(document, "theme_mode", bad_fields)
            .unwrap_or(&defaults.theme_mode)
            .to_string(),
        theme_light_time: as_str(document, "theme_light_time", bad_fields)
            .unwrap_or(&defaults.theme_light_time)
            .to_string(),
        theme_dark_time: as_str(document, "theme_dark_time", bad_fields)
            .unwrap_or(&defaults.theme_dark_time)
            .to_string(),
        theme_ip_location_enabled: as_bool(document, "theme_ip_location_enabled", bad_fields)
            .unwrap_or(defaults.theme_ip_location_enabled),
        theme_dark_on_battery: as_bool(document, "theme_dark_on_battery", bad_fields)
            .unwrap_or(defaults.theme_dark_on_battery),
        theme_skip_fullscreen: as_bool(document, "theme_skip_fullscreen", bad_fields)
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
            if (old.is_bool() && old.as_bool() == value.as_bool())
                || (old.is_integer() && old.as_integer() == value.as_integer())
                || (old.is_str() && old.as_str() == value.as_str())
            {
                return;
            }
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
    let text = document.to_string();
    if std::fs::read_to_string(path).is_ok_and(|original| original == text) {
        return Ok(());
    }
    let result = (|| {
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    Ok(())
}

#[cfg(test)]
mod save_tests {
    use super::*;
    #[test]
    fn idle_warning_below_minimum_sanitizes_upward() {
        // 0 used to mean "silent execution"; the idle path now shares the
        // automation rules' cancellable-countdown minimum.
        let config = Config {
            idle_warning_seconds: 0,
            ..Config::default()
        }
        .sanitized();
        assert_eq!(
            config.idle_warning_seconds,
            crate::automation::MIN_WARNING_SECONDS
        );
    }
    #[test]
    fn mistyped_fields_surface_in_load_error_and_fall_back() {
        let path = std::env::temp_dir().join(format!(
            "idletrigger-mistyped-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &path,
            "idle_timeout_minutes = \"30\"
",
        )
        .unwrap();
        let loaded = load(&path);
        std::fs::remove_file(&path).unwrap();
        // Field-level mistakes are a soft warning, not a load failure: the
        // document parses and one save repairs the offenders.
        assert!(loaded.load_error.is_none());
        assert!(
            loaded
                .field_errors
                .as_deref()
                .is_some_and(|err| err.contains("idle_timeout_minutes"))
        );
        assert_eq!(loaded.config.idle_timeout_minutes, 30); // default, not the string
    }
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
    fn every_config_field_roundtrips_through_save() {
        // Every field set non-default with no `..Default` spread: adding a
        // Config field must extend this literal (compilation fails otherwise),
        // so a field missed in read_config or the save path fails here instead
        // of silently keeping its default.
        let config = Config {
            language: "en".into(),
            logging_enabled: true,
            nosleep_enabled: true,
            keep_screen_on: true,
            nosleep_on_battery: true,
            nosleep_battery_threshold: 55,
            idle_enabled: true,
            idle_timeout_minutes: 120,
            idle_action: "hibernate".into(),
            idle_warning_seconds: 45,
            idle_enhanced_monitor: true,
            automation_enabled: false,
            hotkeys_enabled: true,
            lock_keys_enabled: true,
            lock_keys_caps_enabled: false,
            lock_keys_num_enabled: false,
            lock_keys_scroll_enabled: false,
            lock_keys_skip_fullscreen: false,
            theme_switch_enabled: true,
            theme_mode: "fixed".into(),
            theme_light_time: "06:30".into(),
            theme_dark_time: "20:30".into(),
            theme_ip_location_enabled: true,
            theme_dark_on_battery: false,
            theme_skip_fullscreen: false,
        };
        let path = std::env::temp_dir().join(format!(
            "idletrigger-full-field-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut doc: toml_edit::DocumentMut = "custom = 7\n".parse().unwrap();
        save(&path, &mut doc, &config).unwrap();
        let loaded = load(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(loaded.config, config);
        assert_eq!(loaded.document["custom"].as_integer(), Some(7));
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
