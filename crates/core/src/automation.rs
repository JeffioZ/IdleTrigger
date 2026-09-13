//! Automatic-task model, validation, and normalization.
//!
//! Faithful port of the Go `internal/automation` package: string-based
//! enums, the same TOML field names, the same limits, and the same
//! normalization rules, so existing `IdleTrigger.toml` rule lists and
//! `IdleTrigger.state.json` bookkeeping load unchanged.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const DEFAULT_IDLE_MINUTES: i32 = 30;
pub const DEFAULT_WARNING_SECONDS: i32 = 60;
pub const MIN_WARNING_SECONDS: i32 = 10;
pub const MAX_RULES: usize = 64;
pub const MAX_PROCESSES_PER_RULE: usize = 64;

pub const ACTION_STAY_AWAKE: &str = "stay_awake";
pub const ACTION_PAUSE_STAY_AWAKE: &str = "pause_stay_awake";
pub const ACTION_ENABLE_IDLE: &str = "enable_idle_monitor";
pub const ACTION_PAUSE_IDLE: &str = "pause_idle_monitor";
pub const ACTION_LOCK: &str = "lock";
pub const ACTION_SLEEP: &str = "sleep";
pub const ACTION_HIBERNATE: &str = "hibernate";
pub const ACTION_SHUTDOWN: &str = "shutdown";
pub const ACTION_RESTART: &str = "restart";

pub const TRIGGER_PROCESS_RUNNING: &str = "process_running";
pub const TRIGGER_PROCESS_STARTED: &str = "process_started";
pub const TRIGGER_PROCESS_EXITED: &str = "process_exited";
pub const TRIGGER_TIME_WINDOW: &str = "time_window";
pub const TRIGGER_ONCE: &str = "once";
pub const TRIGGER_DAILY: &str = "daily";
pub const TRIGGER_WEEKLY: &str = "weekly";

pub const MATCH_NAME: &str = "name";
pub const MATCH_PATH: &str = "path";

pub const LOGIC_ANY: &str = "any";
pub const LOGIC_ALL: &str = "all";
pub const LOGIC_NONE: &str = "none";

pub const BLOCKED_SKIP: &str = "skip";
pub const BLOCKED_WAIT: &str = "wait";

/// Identifies an application, never one ephemeral PID.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessTarget {
    #[serde(rename = "match")]
    pub kind: String,
    pub executable: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
}

impl ProcessTarget {
    pub fn key(&self) -> String {
        if self.kind == MATCH_PATH {
            format!("path:{}", normalize_path(&self.path).to_lowercase())
        } else {
            format!("name:{}", self.executable.to_lowercase())
        }
    }
}

/// One built-in automatic task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub action: String,
    pub trigger: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub time: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub end_time: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub date: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub days: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub process_logic: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub processes: Vec<ProcessTarget>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_screen_on: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub idle_minutes: i32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub warning_seconds: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blocked_policy: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_wait_minutes: i32,
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

/// Why one configured rule is unsafe to run.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleIssue {
    pub index: usize,
    pub rule_id: String,
    pub message: String,
}

pub fn is_state_action(action: &str) -> bool {
    matches!(
        action,
        ACTION_STAY_AWAKE | ACTION_PAUSE_STAY_AWAKE | ACTION_ENABLE_IDLE | ACTION_PAUSE_IDLE
    )
}

pub fn is_event_action(action: &str) -> bool {
    valid_action(action) && !is_state_action(action)
}

pub fn valid_action(action: &str) -> bool {
    matches!(
        action,
        ACTION_STAY_AWAKE
            | ACTION_PAUSE_STAY_AWAKE
            | ACTION_ENABLE_IDLE
            | ACTION_PAUSE_IDLE
            | ACTION_LOCK
            | ACTION_SLEEP
            | ACTION_HIBERNATE
            | ACTION_SHUTDOWN
            | ACTION_RESTART
    )
}

pub fn valid_trigger(trigger: &str) -> bool {
    matches!(
        trigger,
        TRIGGER_PROCESS_RUNNING
            | TRIGGER_PROCESS_STARTED
            | TRIGGER_PROCESS_EXITED
            | TRIGGER_TIME_WINDOW
            | TRIGGER_ONCE
            | TRIGGER_DAILY
            | TRIGGER_WEEKLY
    )
}

pub fn normalize_rules(rules: Vec<Rule>) -> Vec<Rule> {
    if rules.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(rules.len());
    let mut seen_ids: BTreeMap<String, ()> = BTreeMap::new();
    for (index, mut r) in rules.into_iter().enumerate() {
        r.id = r.id.trim().to_string();
        if r.id.is_empty() {
            r.id = format!("rule-{}", index + 1);
        }
        let id_key = r.id.to_lowercase();
        if seen_ids.contains_key(&id_key) {
            r.id = format!("{}-{}", r.id, index + 1);
        }
        seen_ids.insert(r.id.to_lowercase(), ());
        r.name = r.name.trim().to_string();
        if r.name.is_empty() {
            r.name = r.id.clone();
        }
        if !matches!(&r.process_logic[..], LOGIC_ANY | LOGIC_ALL | LOGIC_NONE) {
            r.process_logic = LOGIC_ANY.to_string();
        }
        if !matches!(&r.blocked_policy[..], BLOCKED_SKIP | BLOCKED_WAIT) {
            r.blocked_policy = BLOCKED_SKIP.to_string();
        }
        if r.idle_minutes <= 0 || r.idle_minutes > 7 * 24 * 60 {
            r.idle_minutes = DEFAULT_IDLE_MINUTES;
        }
        if is_event_action(&r.action)
            && (r.warning_seconds < MIN_WARNING_SECONDS || r.warning_seconds > 3600)
        {
            r.warning_seconds = DEFAULT_WARNING_SECONDS;
        } else if !(0..=3600).contains(&r.warning_seconds) {
            r.warning_seconds = 0;
        }
        if !(0..=7 * 24 * 60).contains(&r.max_wait_minutes) {
            r.max_wait_minutes = 0;
        }
        r.days = normalize_days(&r.days);
        // Older time-window rules used an empty day list to mean every day.
        if r.trigger == TRIGGER_TIME_WINDOW && r.days.is_empty() {
            r.days = ALL_DAYS.iter().map(|d| d.to_string()).collect();
        }
        r.processes = normalize_targets(r.processes);
        out.push(r);
    }
    out
}

pub const ALL_DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

pub fn normalize_days(days: &[String]) -> Vec<String> {
    let order = |day: &str| ALL_DAYS.iter().position(|d| *d == day);
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for value in days {
        let day = value.trim().to_lowercase();
        if order(&day).is_none() || seen.contains(&day) {
            continue;
        }
        seen.insert(day.clone());
        out.push(day);
    }
    out.sort_by_key(|d| order(d).unwrap());
    out
}

fn normalize_path(path: &str) -> String {
    // Windows path normalization without pulling in a path crate: collapse
    // slashes and drop trailing separators, matching filepath.Clean closely
    // enough for stable keys.
    let mut cleaned = path.replace('/', "\\");
    while cleaned.ends_with('\\') && cleaned.len() > 3 {
        cleaned.pop();
    }
    cleaned
}

pub fn normalize_targets(targets: Vec<ProcessTarget>) -> Vec<ProcessTarget> {
    if targets.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<ProcessTarget> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut name_covered = std::collections::BTreeSet::new();
    for mut t in targets {
        t.executable = t.executable.trim().to_string();
        // Keep only the file name like filepath.Base.
        if let Some(pos) = t.executable.rfind(['\\', '/']) {
            t.executable = t.executable[pos + 1..].to_string();
        }
        t.path = t.path.trim().to_string();
        if t.kind != MATCH_PATH {
            t.kind = MATCH_NAME.to_string();
            t.path = String::new();
        }
        if t.executable.is_empty() || t.executable == "." {
            continue;
        }
        if t.kind == MATCH_PATH {
            if !t.path.starts_with('\\') && !path_is_abs_drive(&t.path) {
                continue;
            }
            t.path = normalize_path(&t.path);
            let base = t
                .path
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or_default()
                .to_string();
            if !base.eq_ignore_ascii_case(&t.executable) {
                continue;
            }
        } else {
            name_covered.insert(t.executable.to_lowercase());
        }
        let key = t.key();
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        out.push(t);
    }
    // A name target already includes all exact paths with that executable.
    out.retain(|target| {
        !(target.kind == MATCH_PATH && name_covered.contains(&target.executable.to_lowercase()))
    });
    out.sort_by(|a, b| {
        let key = |t: &ProcessTarget| (t.executable.to_lowercase(), t.key());
        key(a).cmp(&key(b))
    });
    out
}

fn path_is_abs_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
        && bytes[0].is_ascii_alphabetic()
}

/// The single normalization and validation entry point. Invalid rules stay
/// present and receive one deterministic diagnostic each.
pub fn prepare_rules(rules: &[Rule]) -> (Vec<Rule>, Vec<RuleIssue>) {
    let normalized = normalize_rules(rules.to_vec());
    let mut issues = Vec::new();

    let mut seen = std::collections::BTreeSet::new();
    for (index, raw) in rules.iter().enumerate() {
        let rule_id_at = |i: usize| normalized.get(i).map(|r| r.id.clone()).unwrap_or_default();
        let mut add_issue = |message: String| {
            issues.push(RuleIssue {
                index,
                rule_id: rule_id_at(index),
                message,
            });
        };
        if index >= MAX_RULES {
            add_issue(format!(
                "automation_rules may contain at most {MAX_RULES} rules"
            ));
            continue;
        }
        let id = raw.id.trim();
        if id.is_empty() {
            add_issue(format!("automation_rules[{index}].id is required"));
            continue;
        }
        let key = id.to_lowercase();
        if seen.contains(&key) {
            add_issue(format!("duplicate automation rule id {id:?}"));
            continue;
        }
        seen.insert(key);
        if let Err(err) = validate_prepared_rule(raw, &normalized[index]) {
            add_issue(format!("automation rule {id:?}: {err}"));
        }
    }
    (normalized, issues)
}

/// Runtime list with only invalid entries disabled, preserving valid ones.
pub fn runtime_rules(rules: Vec<Rule>, issues: &[RuleIssue]) -> Vec<Rule> {
    let mut out = rules;
    for issue in issues {
        if let Some(rule) = out.get_mut(issue.index) {
            rule.enabled = false;
        }
    }
    out
}

fn validate_prepared_rule(raw: &Rule, normalized: &Rule) -> Result<(), String> {
    if !valid_action(&raw.action) {
        return Err(format!("invalid action {:?}", raw.action));
    }
    if !valid_trigger(&raw.trigger) {
        return Err(format!("invalid trigger {:?}", raw.trigger));
    }
    if !raw.process_logic.is_empty()
        && !matches!(&raw.process_logic[..], LOGIC_ANY | LOGIC_ALL | LOGIC_NONE)
    {
        return Err(format!("invalid process_logic {:?}", raw.process_logic));
    }
    if !raw.blocked_policy.is_empty()
        && !matches!(&raw.blocked_policy[..], BLOCKED_SKIP | BLOCKED_WAIT)
    {
        return Err(format!("invalid blocked_policy {:?}", raw.blocked_policy));
    }
    if raw.idle_minutes < 0
        || raw.idle_minutes > 7 * 24 * 60
        || (raw.action == ACTION_ENABLE_IDLE && raw.idle_minutes == 0)
    {
        return Err("idle_minutes must be between 1 and 10080".into());
    }
    if raw.warning_seconds < 0
        || raw.warning_seconds > 3600
        || (is_event_action(&raw.action) && raw.warning_seconds < MIN_WARNING_SECONDS)
    {
        return Err(format!(
            "warning_seconds must be between {MIN_WARNING_SECONDS} and 3600"
        ));
    }
    if !(0..=7 * 24 * 60).contains(&raw.max_wait_minutes) {
        return Err("max_wait_minutes must be between 0 and 10080".into());
    }
    if is_event_action(&raw.action)
        && !raw.processes.is_empty()
        && raw.trigger != TRIGGER_PROCESS_STARTED
        && raw.trigger != TRIGGER_PROCESS_EXITED
        && raw.blocked_policy == BLOCKED_WAIT
        && raw.max_wait_minutes == 0
    {
        return Err("max_wait_minutes must be between 1 and 10080 when waiting".into());
    }
    if raw.processes.len() > MAX_PROCESSES_PER_RULE {
        return Err(format!(
            "may contain at most {MAX_PROCESSES_PER_RULE} process targets"
        ));
    }
    if raw.days.iter().any(|d| {
        !matches!(
            d.trim().to_lowercase().as_str(),
            "sun" | "mon" | "tue" | "wed" | "thu" | "fri" | "sat"
        )
    }) {
        return Err("contains an invalid weekday".into());
    }
    for target in &raw.processes {
        validate_process_target(target).map_err(|e| format!("invalid process target: {e}"))?;
    }
    validate_action_trigger(normalized)
}

fn validate_process_target(target: &ProcessTarget) -> Result<(), String> {
    if !target.kind.is_empty() && target.kind != MATCH_NAME && target.kind != MATCH_PATH {
        return Err(format!("invalid match {:?}", target.kind));
    }
    let executable = target.executable.trim();
    if executable.is_empty() || executable == "." {
        return Err("executable is required".into());
    }
    if target.kind == MATCH_PATH {
        let path = target.path.trim();
        if !path.starts_with('\\') && !path_is_abs_drive(path) {
            return Err("path must be absolute".into());
        }
        let base = path.rsplit(['\\', '/']).next().unwrap_or_default();
        if !base.eq_ignore_ascii_case(executable) {
            return Err("path does not match executable".into());
        }
    }
    Ok(())
}

fn validate_action_trigger(r: &Rule) -> Result<(), String> {
    if is_state_action(&r.action) {
        if r.trigger != TRIGGER_PROCESS_RUNNING && r.trigger != TRIGGER_TIME_WINDOW {
            return Err(format!(
                "state action {:?} requires process_running or time_window",
                r.action
            ));
        }
    } else if r.trigger == TRIGGER_PROCESS_RUNNING || r.trigger == TRIGGER_TIME_WINDOW {
        return Err(format!(
            "event action {:?} requires once, daily, weekly, process_started, or process_exited",
            r.action
        ));
    }
    if matches!(
        r.trigger.as_str(),
        TRIGGER_PROCESS_RUNNING | TRIGGER_PROCESS_STARTED | TRIGGER_PROCESS_EXITED
    ) && r.processes.is_empty()
    {
        return Err(format!(
            "trigger {:?} requires at least one process",
            r.trigger
        ));
    }
    if r.trigger == TRIGGER_ONCE && !valid_date(&r.date) {
        return Err(format!("invalid date {:?}", r.date));
    }
    if matches!(
        r.trigger.as_str(),
        TRIGGER_ONCE | TRIGGER_DAILY | TRIGGER_WEEKLY | TRIGGER_TIME_WINDOW
    ) && !valid_hhmm(&r.time)
    {
        return Err(format!("invalid time {:?}", r.time));
    }
    if r.trigger == TRIGGER_TIME_WINDOW && !valid_hhmm(&r.end_time) {
        return Err(format!("invalid end_time {:?}", r.end_time));
    }
    if matches!(r.trigger.as_str(), TRIGGER_WEEKLY | TRIGGER_TIME_WINDOW)
        && normalize_days(&r.days).is_empty()
    {
        return Err("weekly and time-window triggers require at least one day".into());
    }
    Ok(())
}

pub fn valid_hhmm(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 5
        && bytes[2] == b':'
        && bytes[..2].iter().all(u8::is_ascii_digit)
        && bytes[3..].iter().all(u8::is_ascii_digit)
        && value[..2].parse::<u32>().map(|h| h < 24).unwrap_or(false)
        && value[3..].parse::<u32>().map(|m| m < 60).unwrap_or(false)
}

fn valid_date(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    let y4 = parts[0].len() == 4 && parts[0].bytes().all(|b| b.is_ascii_digit());
    let m2 = parts[1].len() == 2 && parts[1].bytes().all(|b| b.is_ascii_digit());
    let d2 = parts[2].len() == 2 && parts[2].bytes().all(|b| b.is_ascii_digit());
    if !(y4 && m2 && d2) {
        return false;
    }
    let month: u32 = parts[1].parse().unwrap_or(0);
    let day: u32 = parts[2].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

/// Weekday key for a day-of-week where 0 = Sunday (chrono-free).
pub fn weekday_key(day_of_week_sunday_zero: usize) -> &'static str {
    ALL_DAYS[day_of_week_sunday_zero % 7]
}

/// Scheduler bookkeeping kept outside `IdleTrigger.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub last_occurrences: BTreeMap<String, String>,
}

pub fn load_runtime_state(path: &Path) -> (RuntimeState, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(state) => (state, None),
            Err(err) => (
                RuntimeState::default(),
                Some(format!("parse automation state: {err}")),
            ),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (RuntimeState::default(), None),
        Err(err) => (
            RuntimeState::default(),
            Some(format!("read automation state: {err}")),
        ),
    }
}

pub fn save_runtime_state(path: &Path, state: &RuntimeState) -> std::io::Result<()> {
    let mut data = serde_json::to_string_pretty(state).unwrap_or_default();
    data.push('\n');
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = parent.join(format!(
        ".IdleTrigger-state-{}.json.tmp",
        std::process::id()
    ));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, action: &str, trigger: &str) -> Rule {
        Rule {
            id: id.into(),
            name: String::new(),
            enabled: true,
            action: action.into(),
            trigger: trigger.into(),
            time: String::new(),
            end_time: String::new(),
            date: String::new(),
            days: Vec::new(),
            process_logic: String::new(),
            processes: Vec::new(),
            keep_screen_on: false,
            idle_minutes: 0,
            warning_seconds: MIN_WARNING_SECONDS,
            blocked_policy: String::new(),
            max_wait_minutes: 0,
        }
    }

    #[test]
    fn normalizes_ids_names_and_defaults() {
        let mut r = rule("  ", ACTION_SLEEP, TRIGGER_DAILY);
        r.name = "  ".into();
        r.time = "08:30".into();
        r.warning_seconds = 3600; // out of range: normalize clamps to default
        let (normalized, issues) = prepare_rules(&[r]);
        // The raw empty id is still reported like in Go, while normalization
        // assigns a stable fallback id and name.
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("id is required"));
        assert_eq!(normalized[0].id, "rule-1");
        assert_eq!(normalized[0].name, "rule-1");
        assert_eq!(normalized[0].warning_seconds, 3600); // upper bound is legal
    }

    #[test]
    fn state_action_requires_state_trigger() {
        let r = rule("a", ACTION_STAY_AWAKE, TRIGGER_DAILY);
        let (_, issues) = prepare_rules(&[r]);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("state action"));
    }

    #[test]
    fn event_action_requires_event_trigger() {
        let r = rule("a", ACTION_LOCK, TRIGGER_TIME_WINDOW);
        let (_, issues) = prepare_rules(&[r]);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn process_trigger_requires_targets() {
        let r = rule("a", ACTION_STAY_AWAKE, TRIGGER_PROCESS_RUNNING);
        let (_, issues) = prepare_rules(&[r]);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("at least one process"));
    }

    #[test]
    fn time_window_defaults_to_all_days() {
        let mut r = rule("a", ACTION_STAY_AWAKE, TRIGGER_TIME_WINDOW);
        r.time = "09:00".into();
        r.end_time = "17:00".into();
        let (normalized, issues) = prepare_rules(&[r]);
        assert!(issues.is_empty());
        assert_eq!(normalized[0].days.len(), 7);
    }

    #[test]
    fn name_target_covers_path_target() {
        let targets = vec![
            ProcessTarget {
                kind: "name".into(),
                executable: "App.EXE".into(),
                path: String::new(),
            },
            ProcessTarget {
                kind: "path".into(),
                executable: "app.exe".into(),
                path: "C:\\Tools\\app.exe".into(),
            },
        ];
        let out = normalize_targets(targets);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, MATCH_NAME);
    }

    #[test]
    fn relative_path_target_is_dropped() {
        let targets = vec![ProcessTarget {
            kind: "path".into(),
            executable: "app.exe".into(),
            path: "app.exe".into(),
        }];
        assert!(normalize_targets(targets).is_empty());
    }

    #[test]
    fn hhmm_validation() {
        assert!(valid_hhmm("23:59"));
        assert!(valid_hhmm("00:00"));
        assert!(!valid_hhmm("24:00"));
        assert!(!valid_hhmm("7:30"));
        assert!(!valid_hhmm("07:60"));
        assert!(!valid_hhmm("0700"));
    }

    #[test]
    fn runtime_rules_disables_only_invalid() {
        let mut ok = rule("ok", ACTION_LOCK, TRIGGER_DAILY);
        ok.time = "08:00".into();
        let bad = rule("bad", ACTION_LOCK, ACTION_LOCK); // invalid trigger
        let (normalized, issues) = prepare_rules(&[ok, bad]);
        let runtime = runtime_rules(normalized, &issues);
        assert!(runtime[0].enabled);
        assert!(!runtime[1].enabled);
    }

    #[test]
    fn duplicate_ids_reported() {
        let rules = vec![
            rule("dup", ACTION_LOCK, TRIGGER_DAILY),
            rule("dup", ACTION_LOCK, TRIGGER_DAILY),
        ];
        let (_, issues) = prepare_rules(&rules);
        assert!(issues.iter().any(|i| i.message.contains("duplicate")));
    }
}
