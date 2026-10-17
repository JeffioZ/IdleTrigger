//! Automation editing domain: the rule vocabulary shared by the manager,
//! editor, and picker - label/key mapping, summaries, validation, date/time
//! parsing, templates, persistence, and process-target descriptions. Pure
//! data in, data out; no HWNDs live here.

use idletrigger_core::automation as auto;

use crate::automation_ui::{
    ED_BATTERY, ED_CHOOSE, ED_DATE, ED_DAYS_MON, ED_END_TIME, ED_IDLE_MIN, ED_MAX_WAIT, ED_TIME,
    ED_WARNING,
};
use crate::t_pub;

/// Executable-description cache shared by the picker's enrichment and the
/// editor's summaries (populated lazily from version resources).
pub(crate) static DESCRIPTIONS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, String>>,
> = std::sync::LazyLock::new(Default::default);

/// Go cleanSingleLine: collapse line breaks and trim.
pub(crate) fn clean_single_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().to_string()
}

pub(crate) fn fill_template(template: &str, values: &[&str]) -> String {
    idletrigger_core::i18n::format(template, values)
}

pub(crate) fn action_label(value: &str) -> String {
    let key = match value {
        auto::ACTION_STAY_AWAKE => "automation_action_stay_awake",
        auto::ACTION_PAUSE_STAY_AWAKE => "automation_action_pause_stay_awake",
        auto::ACTION_ENABLE_IDLE => "automation_action_enable_idle",
        auto::ACTION_PAUSE_IDLE => "automation_action_pause_idle",
        auto::ACTION_LOCK => "menu_lock",
        auto::ACTION_SLEEP => "menu_sleep",
        auto::ACTION_HIBERNATE => "menu_hibernate",
        auto::ACTION_SHUTDOWN => "menu_shutdown",
        auto::ACTION_RESTART => "menu_restart",
        auto::ACTION_SCREEN_OFF => "menu_action_screen_off",
        auto::ACTION_LOGOFF => "menu_action_logoff",
        _ => return value.to_string(),
    };
    t_pub(key)
}

pub(crate) fn trigger_label(value: &str) -> String {
    let key = match value {
        auto::TRIGGER_PROCESS_RUNNING => "trigger_process_running",
        auto::TRIGGER_PROCESS_STARTED => "trigger_process_started",
        auto::TRIGGER_PROCESS_EXITED => "trigger_process_exited",
        auto::TRIGGER_TIME_WINDOW => "trigger_time_window",
        auto::TRIGGER_ONCE => "trigger_once",
        auto::TRIGGER_DAILY => "trigger_daily",
        auto::TRIGGER_WEEKLY => "trigger_weekly",
        auto::TRIGGER_SESSION_LOCKED => "trigger_session_locked",
        auto::TRIGGER_SESSION_UNLOCKED => "trigger_session_unlocked",
        auto::TRIGGER_ON_RESUME => "trigger_on_resume",
        auto::TRIGGER_BATTERY_BELOW => "trigger_battery_below",
        _ => return value.to_string(),
    };
    t_pub(key)
}

pub(crate) fn logic_label(value: &str) -> String {
    let key = match value {
        auto::LOGIC_ANY => "automation_process_any",
        auto::LOGIC_ALL => "automation_process_all",
        auto::LOGIC_NONE => "automation_process_none",
        _ => return value.to_string(),
    };
    t_pub(key)
}

pub(crate) fn blocked_label(value: &str) -> String {
    t_pub(if value == auto::BLOCKED_WAIT {
        "automation_blocked_wait"
    } else {
        "automation_blocked_skip"
    })
}

pub(crate) fn action_keys() -> Vec<&'static str> {
    // Go actions[] order: state actions first, then system actions.
    vec![
        auto::ACTION_STAY_AWAKE,
        auto::ACTION_PAUSE_STAY_AWAKE,
        auto::ACTION_ENABLE_IDLE,
        auto::ACTION_PAUSE_IDLE,
        auto::ACTION_LOCK,
        auto::ACTION_SLEEP,
        auto::ACTION_HIBERNATE,
        auto::ACTION_SHUTDOWN,
        auto::ACTION_RESTART,
        auto::ACTION_SCREEN_OFF,
        auto::ACTION_LOGOFF,
    ]
}

/// Go setTriggerOptions: triggers are filtered by the selected action type.
pub(crate) fn trigger_keys_for(action: &str) -> Vec<&'static str> {
    if auto::is_state_action(action) {
        vec![auto::TRIGGER_PROCESS_RUNNING, auto::TRIGGER_TIME_WINDOW]
    } else {
        vec![
            auto::TRIGGER_ONCE,
            auto::TRIGGER_DAILY,
            auto::TRIGGER_WEEKLY,
            auto::TRIGGER_PROCESS_STARTED,
            auto::TRIGGER_PROCESS_EXITED,
            auto::TRIGGER_SESSION_LOCKED,
            auto::TRIGGER_SESSION_UNLOCKED,
            auto::TRIGGER_ON_RESUME,
            auto::TRIGGER_BATTERY_BELOW,
        ]
    }
}

pub(crate) fn logic_keys() -> Vec<&'static str> {
    vec![auto::LOGIC_ANY, auto::LOGIC_ALL, auto::LOGIC_NONE]
}

pub(crate) fn blocked_keys() -> Vec<&'static str> {
    vec![auto::BLOCKED_SKIP, auto::BLOCKED_WAIT]
}

/// Go ruleSummary: localized "action · trigger detail" one-liner.
pub(crate) fn rule_summary(rule: &auto::Rule) -> String {
    let action = action_label(&rule.action);
    let count = rule.processes.len().to_string();
    match rule.trigger.as_str() {
        auto::TRIGGER_PROCESS_RUNNING => fill_template(
            &t_pub("automation_summary_process_running"),
            &[&action, &count],
        ),
        auto::TRIGGER_PROCESS_STARTED => fill_template(
            &t_pub("automation_summary_process_started"),
            &[&action, &count],
        ),
        auto::TRIGGER_PROCESS_EXITED => fill_template(
            &t_pub("automation_summary_process_exited"),
            &[&action, &count],
        ),
        auto::TRIGGER_TIME_WINDOW => fill_template(
            &t_pub("automation_summary_time_window"),
            &[
                &action,
                &rule.time,
                &rule.end_time,
                &day_summary(&rule.days),
            ],
        ),
        auto::TRIGGER_ONCE => fill_template(
            &t_pub("automation_summary_once"),
            &[&action, &rule.date, &rule.time],
        ),
        auto::TRIGGER_DAILY => {
            fill_template(&t_pub("automation_summary_daily"), &[&action, &rule.time])
        }
        auto::TRIGGER_WEEKLY => fill_template(
            &t_pub("automation_summary_weekly"),
            &[&action, &day_summary(&rule.days), &rule.time],
        ),
        auto::TRIGGER_SESSION_LOCKED => {
            fill_template(&t_pub("automation_summary_session"), &[&action])
        }
        auto::TRIGGER_SESSION_UNLOCKED => {
            fill_template(&t_pub("automation_summary_session_unlocked"), &[&action])
        }
        auto::TRIGGER_ON_RESUME => {
            fill_template(&t_pub("automation_summary_on_resume"), &[&action])
        }
        auto::TRIGGER_BATTERY_BELOW => fill_template(
            &t_pub("automation_summary_battery"),
            &[&action, &rule.battery_level.to_string()],
        ),
        _ => action,
    }
}

/// Go daySummary: 每天 / 工作日 / 周一、周二… (weekday order mon..sun).
pub(crate) fn day_summary(days: &[String]) -> String {
    let week = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let contains = |key: &str| days.iter().any(|d| d.trim().eq_ignore_ascii_case(key));

    if week.iter().all(|d| contains(d)) {
        return t_pub("automation_days_everyday");
    }
    let workdays = !contains("sat") && !contains("sun") && week[..5].iter().all(|d| contains(d));
    if workdays {
        return t_pub("automation_days_workdays");
    }

    let mut labels = Vec::new();
    for day in week {
        if !contains(day) {
            continue;
        }
        let mut label = t_pub(&format!("automation_day_{day}"));
        if crate::i18n_is_chinese() {
            label = format!("周{label}");
        }
        labels.push(label);
    }
    labels.join(&t_pub("automation_days_separator"))
}

pub(crate) fn save_rules_to_config(
    base: &[auto::Rule],
    rules: Vec<auto::Rule>,
) -> Result<(), String> {
    crate::save_automation_rules(base, &rules)?;
    Ok(())
}

pub(crate) fn template_rule(index: usize) -> auto::Rule {
    let mut rule = default_rule();
    match index {
        0 => {
            rule.name = t_pub("automation_template_process_awake");
            rule.action = auto::ACTION_STAY_AWAKE.into();
            rule.trigger = auto::TRIGGER_PROCESS_RUNNING.into();
        }
        1 => {
            rule.name = t_pub("automation_template_night_shutdown");
            rule.action = auto::ACTION_SHUTDOWN.into();
            rule.trigger = auto::TRIGGER_DAILY.into();
            rule.time = "23:30".into();
            rule.warning_seconds = 60;
        }
        2 => {
            rule.name = t_pub("automation_template_work_awake");
            rule.action = auto::ACTION_STAY_AWAKE.into();
            rule.trigger = auto::TRIGGER_TIME_WINDOW.into();
            rule.time = "09:00".into();
            rule.end_time = "18:00".into();
            rule.days = ["mon", "tue", "wed", "thu", "fri"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        }
        3 => {
            rule.name = t_pub("automation_template_low_battery");
            rule.action = auto::ACTION_HIBERNATE.into();
            rule.trigger = auto::TRIGGER_BATTERY_BELOW.into();
            rule.battery_level = 20;
            rule.warning_seconds = 60;
        }
        4..=6 => {
            rule.action = if index == 6 {
                auto::ACTION_SLEEP
            } else {
                auto::ACTION_SHUTDOWN
            }
            .into();
            rule.trigger = auto::TRIGGER_ONCE.into();
            let minutes = if index == 5 { 60 } else { 30 };
            if let Some((date, time)) = scheduled_after(minutes) {
                rule.date = date;
                rule.time = time;
            } else {
                // Leave invalid fields for the editor to flag, never silently
                // substitute a different shutdown time if conversion fails.
                rule.date.clear();
                rule.time.clear();
            }
            // Let the editor derive a lasting name from the actual date/time,
            // rather than leaving "in 30 minutes" on yesterday's task.
            rule.warning_seconds = 60;
        }
        _ => {}
    }
    rule
}

/// Resolve elapsed time in UTC before converting to local calendar time.
/// Round up to the editor's minute precision so a preset never fires early.
pub(crate) fn scheduled_after(minutes: u64) -> Option<(String, String)> {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos()
        .div_ceil(1_000_000_000) as u64;
    let ticks = ((seconds + minutes * 60).div_ceil(60) * 60 + 11_644_473_600) * 10_000_000;
    let ft = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&ft, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some((
        format!("{:04}-{:02}-{:02}", local.wYear, local.wMonth, local.wDay),
        format!("{:02}:{:02}", local.wHour, local.wMinute),
    ))
}

/// Go defaultRule(): stay_awake + process_running + sensible times.
pub(crate) fn default_rule() -> auto::Rule {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let date = format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay);
    let next_hour = format!("{:02}:{:02}", (st.wHour as i32 + 1) % 24, st.wMinute);
    let two_hours = format!("{:02}:{:02}", (st.wHour as i32 + 2) % 24, st.wMinute);
    auto::Rule {
        id: format!("rule-{now:x}"),
        name: String::new(),
        enabled: true,
        action: auto::ACTION_STAY_AWAKE.into(),
        trigger: "process_running".into(),
        time: next_hour,
        end_time: two_hours,
        date,
        days: vec![
            "mon".into(),
            "tue".into(),
            "wed".into(),
            "thu".into(),
            "fri".into(),
        ],
        process_logic: auto::LOGIC_ANY.into(),
        processes: Vec::new(),
        keep_screen_on: false,
        idle_minutes: auto::DEFAULT_IDLE_MINUTES,
        warning_seconds: auto::DEFAULT_WARNING_SECONDS,
        blocked_policy: auto::BLOCKED_SKIP.into(),
        max_wait_minutes: 60,
        battery_level: 0,
    }
}

/// Go validateDraft: returns the first error message (with its control).
pub(crate) fn validate_draft(draft: &auto::Rule) -> Option<(usize, String)> {
    let t = |key: &str| t_pub(key);
    let trigger = draft.trigger.as_str();
    if trigger == auto::TRIGGER_BATTERY_BELOW && !(1..=95).contains(&draft.battery_level) {
        return Some((ED_BATTERY, t("automation_error_battery_level")));
    }
    let needs_process = matches!(
        trigger,
        "process_running" | "process_started" | "process_exited"
    );
    if needs_process && draft.processes.is_empty() {
        return Some((ED_CHOOSE, t("automation_error_process_required")));
    }
    if trigger == auto::TRIGGER_ONCE && !parse_date(&draft.date) {
        return Some((ED_DATE, t("automation_error_date")));
    }
    if matches!(
        trigger,
        auto::TRIGGER_ONCE | auto::TRIGGER_DAILY | auto::TRIGGER_WEEKLY | auto::TRIGGER_TIME_WINDOW
    ) && !parse_time(&draft.time)
    {
        return Some((ED_TIME, t("automation_error_time")));
    }
    if trigger == auto::TRIGGER_TIME_WINDOW && !parse_time(&draft.end_time) {
        return Some((ED_END_TIME, t("automation_error_end_time")));
    }
    if matches!(trigger, auto::TRIGGER_WEEKLY | auto::TRIGGER_TIME_WINDOW) && draft.days.is_empty()
    {
        return Some((ED_DAYS_MON, t("automation_error_days")));
    }
    if draft.action == "enable_idle_monitor"
        && (draft.idle_minutes <= 0 || draft.idle_minutes > auto::MAX_IDLE_MINUTES)
    {
        return Some((ED_IDLE_MIN, t("automation_error_idle_minutes")));
    }
    if auto::is_event_action(&draft.action)
        && (draft.warning_seconds < auto::MIN_WARNING_SECONDS || draft.warning_seconds > 3600)
    {
        return Some((ED_WARNING, t("automation_error_warning")));
    }
    if auto::is_event_action(&draft.action)
        && !draft.processes.is_empty()
        && trigger != "process_started"
        && trigger != "process_exited"
        && draft.blocked_policy == auto::BLOCKED_WAIT
        && (draft.max_wait_minutes <= 0 || draft.max_wait_minutes > auto::MAX_IDLE_MINUTES)
    {
        return Some((ED_MAX_WAIT, t("automation_error_max_wait")));
    }
    None
}

pub(crate) fn parse_date(value: &str) -> bool {
    auto::valid_date(value)
}

pub(crate) fn parse_time(value: &str) -> bool {
    auto::valid_hhmm(value)
}

pub(crate) fn described_target(target: &auto::ProcessTarget) -> String {
    let descriptions = crate::runtime::lock(&DESCRIPTIONS);
    match descriptions
        .get(&target.key())
        .filter(|value| !value.is_empty())
    {
        Some(description) => format!("{} ({description})", target.executable),
        None => target.executable.clone(),
    }
}
