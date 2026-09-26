//! Automatic-task runtime: low-frequency scan loop and rule evaluation.
//!
//! State actions (stay awake / idle monitor overrides) are runtime overrides
//! that never rewrite the user's manual toggles. Event actions go through
//! the cancellable countdown window. Rules come from the normalized
//! `idletrigger-core` model; occurrence bookkeeping lives in
//! `IdleTrigger.state.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use idletrigger_core::automation as auto;

/// Runtime state-action overrides layered on top of the manual config.
static OVERRIDES: Mutex<auto::EffectiveState> = Mutex::new(auto::EffectiveState {
    stay_awake: false,
    keep_screen_on: false,
    pause_stay_awake: false,
    enable_idle: false,
    idle_minutes: auto::DEFAULT_IDLE_MINUTES,
    pause_idle: false,
    stay_awake_sources: Vec::new(),
});
static ACTIVE_RULES: Mutex<Vec<auto::Rule>> = Mutex::new(Vec::new());
/// Rules whose battery threshold already fired; cleared on recovery so each
/// threshold crossing triggers exactly once until the percentage recovers.
static BATTERY_FIRED: std::sync::LazyLock<Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

pub fn overrides() -> auto::EffectiveState {
    crate::runtime::lock(&OVERRIDES).clone()
}

#[cfg(test)]
pub(crate) fn set_test_overrides(state: auto::EffectiveState) {
    *crate::runtime::lock(&OVERRIDES) = state;
}

pub static RULES: Mutex<Vec<auto::Rule>> = Mutex::new(Vec::new());

/// Validation issues from the last reload (Go State.Issues): the manager list
/// marks the affected rows as invalid instead of hiding them.
pub static ISSUES: Mutex<Vec<auto::RuleIssue>> = Mutex::new(Vec::new());
pub static STATE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
// Recovering lock: a torn record inserts a duplicate occurrence at worst —
// the same-day dedup absorbs it and the next persist rewrites whole state.
pub static RUNTIME_STATE: Mutex<auto::RuntimeState> = Mutex::new(auto::RuntimeState {
    last_occurrences: BTreeMap::new(),
});

/// An event action waiting for the countdown window.
#[derive(Clone)]
pub struct PendingAction {
    pub rule: Option<auto::Rule>,
    pub action: String,
    pub seconds: i32,
    pub rule_id: String,
    /// Some(date) when the rule is `once`; cancel disables the rule.
    pub once_date: Option<String>,
}

impl PendingAction {
    pub fn is_current(&self) -> bool {
        #[cfg(feature = "devtools")]
        if self.rule.is_none() && crate::devtools::preview_only() {
            return true;
        }
        crate::cfg_map(|c| c.automation_enabled)
            && self.rule.as_ref().is_some_and(|expected| {
                crate::runtime::lock(&RULES)
                    .iter()
                    .any(|rule| rule.enabled && rule == expected)
            })
    }
}

pub static PENDING_ACTION: Mutex<Option<PendingAction>> = Mutex::new(None);
pub static ACTION_BUSY: AtomicBool = AtomicBool::new(false);

// One baseline per rule, and absence grace per target shared by conditions
// and edges. A short restart must not create a second process-start event.
#[derive(Default)]
struct ProcessTracker {
    targets: BTreeMap<String, Presence>,
    edges: BTreeMap<String, (auto::Rule, bool)>,
}
#[derive(Default)]
struct Presence {
    /// Last confirmed observation; `None` until the first resolvable scan.
    known: Option<bool>,
    absent_since: Option<std::time::Instant>,
    /// The latest scan could not query every same-name instance.
    uncertain: bool,
}
impl Presence {
    /// `None` means absence is not confirmable (a same-name instance could
    /// not be queried): hold the last known state instead of manufacturing
    /// an exit. Confirmed absence still travels the exit grace window.
    fn update(&mut self, observed: Option<bool>, now: std::time::Instant) -> bool {
        match observed {
            Some(true) => {
                self.known = Some(true);
                self.absent_since = None;
            }
            Some(false) => {
                if self.known == Some(true) {
                    let since = self.absent_since.get_or_insert(now);
                    if now.saturating_duration_since(*since) >= EXIT_GRACE {
                        self.known = Some(false);
                        self.absent_since = None;
                    }
                } else {
                    self.known = Some(false);
                }
            }
            None => {}
        }
        self.uncertain = observed.is_none();
        self.known.unwrap_or(false)
    }
}
impl ProcessTracker {
    fn observe(
        &mut self,
        rules: &[auto::Rule],
        snapshot: Option<&Snapshot>,
        now: std::time::Instant,
    ) -> Option<BTreeMap<String, bool>> {
        self.edges
            .retain(|_, (previous, _)| rules.iter().any(|r| r.enabled && r == previous));
        let targets: BTreeMap<_, _> = rules
            .iter()
            .filter(|r| r.enabled)
            .flat_map(|r| &r.processes)
            .map(|t| (t.key(), t))
            .collect();
        self.targets.retain(|key, _| targets.contains_key(key));
        let snapshot = snapshot?;
        let mut observations = BTreeMap::new();
        for (key, target) in targets {
            let entry = self.targets.entry(key.clone()).or_default();
            let was_uncertain = entry.uncertain;
            let present = entry.update(target_present(target, snapshot), now);
            if entry.uncertain && !was_uncertain {
                crate::log_line(&format!(
                    "automation: could not query every {} instance; holding last state",
                    target.executable
                ));
            }
            observations.insert(key, present);
        }
        Some(observations)
    }

    fn edge(&mut self, rule: &auto::Rule, present: bool) -> bool {
        let previous = self.edges.insert(rule.id.clone(), (rule.clone(), present));
        let Some((previous_rule, previous)) = previous else {
            return false;
        };
        previous_rule == *rule
            && match rule.trigger.as_str() {
                auto::TRIGGER_PROCESS_STARTED => !previous && present,
                auto::TRIGGER_PROCESS_EXITED => previous && !present,
                _ => false,
            }
    }
}
// Recovering lock: a torn update costs at most one extra or missed
// process-start/exit edge; the next 5s scan rebuilds the baseline.
static PROCESS_TRACKER: Mutex<ProcessTracker> = Mutex::new(ProcessTracker {
    targets: BTreeMap::new(),
    edges: BTreeMap::new(),
});
struct WaitingEvent {
    rule: auto::Rule,
    occurrence: String,
    deadline: std::time::Instant,
}
// Recovering lock: a torn take-and-reinsert drops a pending event that was
// already en route — the same outcome as the deadline expiring.
static WAITING_EVENTS: Mutex<BTreeMap<String, WaitingEvent>> = Mutex::new(BTreeMap::new());

const SCAN_INTERVAL: Duration = Duration::from_secs(5);
const EXIT_GRACE: Duration = Duration::from_secs(5);

pub fn reload_rules() {
    // Parse the whole document and pull only the rules key: array-of-tables
    // items do not round-trip reliably through Item::to_string alone.
    let parsed = {
        let doc_guard = crate::runtime::lock(&crate::CONFIG_DOC);
        let Some(doc) = doc_guard.as_ref() else {
            return;
        };
        idletrigger_core::rule_document::read(doc)
    };
    let (runtime, issues) = match parsed {
        Ok(parsed) => parsed,
        Err(err) => {
            crate::log_line(&format!("automation rules parse error: {err}"));
            (
                Vec::new(),
                vec![auto::RuleIssue {
                    index: 0,
                    rule_id: String::new(),
                    message: err,
                }],
            )
        }
    };
    *crate::runtime::lock(&ISSUES) = issues.clone();
    let enabled = crate::cfg_map(|c| c.automation_enabled);
    let mut active = crate::runtime::lock(&ACTIVE_RULES);
    active.retain(|rule| {
        enabled
            && runtime
                .iter()
                .any(|current| current.enabled && current == rule)
    });
    publish_overrides(auto::aggregate_state(active.iter()));
    drop(active);
    if !enabled {
        crate::runtime::lock(&WAITING_EVENTS).clear();
        *crate::runtime::lock(&PROCESS_TRACKER) = ProcessTracker::default();
    }
    // Battery markers belong to live rules only: a deleted or disabled rule
    // must not leave a stale fired entry behind (it would suppress the
    // first crossing of a later rule reusing the id).
    crate::runtime::lock(&BATTERY_FIRED).retain(|id| {
        runtime
            .iter()
            .any(|r| &r.id == id && r.enabled && r.trigger == auto::TRIGGER_BATTERY_BELOW)
    });
    *crate::runtime::lock(&RULES) = runtime;
    crate::log_line(&format!(
        "automation rules loaded: {} ({} issues)",
        crate::runtime::lock(&RULES).len(),
        issues.len()
    ));
}

pub fn spawn(config_dir: PathBuf) {
    let state_path = config_dir.join("IdleTrigger.state.json");
    let (state, err) = auto::load_runtime_state(&state_path);
    if let Some(err) = err {
        crate::log_line(&format!("automation state: {err}"));
        // Preserve the corrupt file for inspection instead of silently
        // resetting every occurrence checkpoint.
        let backup = state_path.with_extension(format!(
            "json.corrupt-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        ));
        if std::fs::rename(&state_path, &backup).is_ok() {
            crate::log_line(&format!(
                "automation state file preserved as {}",
                backup.display()
            ));
        }
    }
    *crate::runtime::lock(&RUNTIME_STATE) = state;
    *crate::runtime::lock(&STATE_PATH) = Some(state_path);

    std::thread::Builder::new()
        .name("automation".into())
        .spawn(|| {
            let mut first_tick = true;
            loop {
                if crate::EXITING.load(Ordering::SeqCst) {
                    return;
                }
                crate::runtime::catch_and_log("automation", || {
                    if let Err(err) = tick(first_tick) {
                        crate::log_line(&format!("automation tick error: {err}"));
                    }
                });
                first_tick = false;
                std::thread::sleep(SCAN_INTERVAL);
            }
        })
        .expect("spawn automation thread");
}

fn tick(first_tick: bool) -> Result<(), String> {
    tick_with_snapshot(
        first_tick,
        Snapshot::take(),
        local_now()?,
        std::time::Instant::now(),
    )
}

fn tick_with_snapshot(
    first_tick: bool,
    snapshot: Option<Snapshot>,
    now: LocalNow,
    monotonic_now: std::time::Instant,
) -> Result<(), String> {
    // Reload publishes config/rules under the same writer lock. A scan may
    // not publish obsolete overrides or queue an event after a rule edit.
    let _configuration = crate::runtime::lock(&crate::CONFIG_WRITER);
    let enabled = crate::cfg_map(|c| c.automation_enabled);
    let rules = crate::runtime::lock(&RULES).clone();
    if !enabled {
        crate::runtime::lock(&WAITING_EVENTS).clear();
        *crate::runtime::lock(&PROCESS_TRACKER) = ProcessTracker::default();
        clear_overrides();
        return Ok(());
    }
    let observations =
        crate::runtime::lock(&PROCESS_TRACKER).observe(&rules, snapshot.as_ref(), monotonic_now);
    let condition = |rule: &auto::Rule| {
        rule.processes.is_empty()
            || observations
                .as_ref()
                .is_some_and(|present| evaluate_logic(rule, present))
    };

    let mut active = Vec::new();
    let waiting = std::mem::take(&mut *crate::runtime::lock(&WAITING_EVENTS));
    for (id, pending) in waiting {
        let rule = &pending.rule;
        if !rules
            .iter()
            .any(|current| current == rule && current.enabled)
        {
            continue;
        }
        if monotonic_now >= pending.deadline {
            record_occurrence(rule, pending.occurrence);
        } else if condition(rule) {
            fire_event(rule, Some(pending.occurrence.clone()));
        } else {
            crate::runtime::lock(&WAITING_EVENTS).insert(id, pending);
        }
    }
    for rule in &rules {
        if !rule.enabled {
            continue;
        }
        match rule.trigger.as_str() {
            auto::TRIGGER_PROCESS_RUNNING => {
                if condition(rule) {
                    active.push(rule);
                }
            }
            auto::TRIGGER_TIME_WINDOW => {
                if in_time_window(rule, &now) && condition(rule) {
                    active.push(rule);
                }
            }
            auto::TRIGGER_ONCE | auto::TRIGGER_DAILY | auto::TRIGGER_WEEKLY => {
                if first_tick || crate::runtime::lock(&WAITING_EVENTS).contains_key(&rule.id) {
                    continue; // Establish the first observation before scheduling events.
                }
                if day_matches(rule, &now) && schedule_due(&now, &rule.time) {
                    let occurrence = format!("{}T{}", now.date, rule.time);
                    let already = crate::runtime::lock(&RUNTIME_STATE)
                        .has_scheduled_occurrence(&rule.id, &occurrence);
                    if !already {
                        if condition(rule) {
                            fire_event(rule, Some(occurrence));
                        } else if rule.blocked_policy == auto::BLOCKED_WAIT
                            && rule.max_wait_minutes > 0
                        {
                            crate::runtime::lock(&WAITING_EVENTS).insert(
                                rule.id.clone(),
                                WaitingEvent {
                                    rule: rule.clone(),
                                    occurrence,
                                    deadline: monotonic_now
                                        + Duration::from_secs(rule.max_wait_minutes as u64 * 60),
                                },
                            );
                        } else {
                            record_occurrence(rule, occurrence);
                        }
                    }
                }
            }
            auto::TRIGGER_PROCESS_STARTED | auto::TRIGGER_PROCESS_EXITED => {
                let Some(present) = observations.as_ref() else {
                    continue;
                };
                let any = rule
                    .processes
                    .iter()
                    .any(|t| present.get(&t.key()) == Some(&true));
                let fire = crate::runtime::lock(&PROCESS_TRACKER).edge(rule, any);
                if fire {
                    fire_event(rule, None);
                }
            }
            _ => {}
        }
    }
    *crate::runtime::lock(&ACTIVE_RULES) = active.iter().map(|rule| (*rule).clone()).collect();
    publish_overrides(auto::aggregate_state(active));
    if snapshot.is_none() {
        Err("process snapshot failed".into())
    } else {
        Ok(())
    }
}

fn clear_overrides() {
    crate::runtime::lock(&ACTIVE_RULES).clear();
    publish_overrides(auto::EffectiveState::default());
}

/// WTS session events fire matching rules immediately: no occurrence
/// checkpoint (like process edges) and no schedule window to gate them.
pub fn on_session_event(locked: bool) {
    let trigger = if locked {
        auto::TRIGGER_SESSION_LOCKED
    } else {
        auto::TRIGGER_SESSION_UNLOCKED
    };
    let rules = crate::runtime::lock(&RULES).clone();
    for rule in rules.iter().filter(|r| r.enabled && r.trigger == trigger) {
        fire_event(rule, None);
    }
}

/// Resume from sleep fires matching rules (no checkpoint, like session
/// events).
pub fn on_resume() {
    let rules = crate::runtime::lock(&RULES).clone();
    for rule in rules
        .iter()
        .filter(|r| r.enabled && r.trigger == auto::TRIGGER_ON_RESUME)
    {
        fire_event(rule, None);
    }
}

/// Battery transitions edge-detect per rule: a rule fires once when the
/// percentage drops below its level, then re-arms when it climbs back to the
/// level. Replaces the armed bookkeeping with a small fired set.
pub fn on_battery(percent: i32) {
    let rules = crate::runtime::lock(&RULES).clone();
    let mut fired = crate::runtime::lock(&BATTERY_FIRED);
    for rule in rules
        .iter()
        .filter(|r| r.enabled && r.trigger == auto::TRIGGER_BATTERY_BELOW && r.battery_level > 0)
    {
        if percent < rule.battery_level {
            if fired.insert(rule.id.clone()) {
                fire_event(rule, None);
            }
        } else {
            fired.remove(&rule.id);
        }
    }
}

fn publish_overrides(state: auto::EffectiveState) {
    let mut current = crate::runtime::lock(&OVERRIDES);
    let changed = *current != state;
    *current = state;
    drop(current);
    if changed {
        request_refresh();
    }
}
fn request_refresh() {
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            Some(crate::hwnd(&crate::HIDDEN)),
            crate::WM_REFRESH_UI,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(0),
        );
    }
}

fn fire_event(rule: &auto::Rule, occurrence: Option<String>) {
    dispatch_event(rule, occurrence, || unsafe {
        let hidden = crate::hwnd(&crate::HIDDEN);
        // A null HWND posts a thread message successfully, but no window will
        // consume it. Treat a missing recipient as a failed dispatch.
        !hidden.is_invalid()
            && windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(hidden),
                crate::WM_ACTION_SHOW,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            )
            .is_ok()
    });
}

fn dispatch_event(rule: &auto::Rule, occurrence: Option<String>, notify: impl FnOnce() -> bool) {
    if let Some(occurrence) = occurrence
        && !record_occurrence(rule, occurrence)
    {
        return;
    }
    if ACTION_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        crate::log_line(&format!(
            "automation event {} skipped (countdown busy)",
            rule.id
        ));
        // Surface the skip through the save-error dialog channel: dropping
        // a scheduled action silently is hard to diagnose from the tray.
        crate::runtime::lock(&SAVE_ERRORS).insert(
            format!("{}-busy", rule.id),
            crate::t_args("automation_event_skipped_busy", &[&rule.name]),
        );
        request_refresh();
        return;
    }
    let seconds = rule.warning_seconds.max(auto::MIN_WARNING_SECONDS);
    let mut pending = crate::runtime::lock(&PENDING_ACTION);
    *pending = Some(PendingAction {
        rule: Some(rule.clone()),
        action: rule.action.clone(),
        seconds,
        rule_id: rule.id.clone(),
        once_date: rule_once_date(rule),
    });
    if !notify() {
        *pending = None;
        ACTION_BUSY.store(false, Ordering::SeqCst);
        drop(pending);
        crate::log_line(&format!(
            "automation event {} skipped (dispatch failed)",
            rule.id
        ));
    }
}

static SAVE_ERRORS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

fn record_occurrence(rule: &auto::Rule, occurrence: String) -> bool {
    // Remember cancelled occurrences in this process too: do not retry the
    // same action or show another error on every scan during its grace window.
    crate::runtime::lock(&RUNTIME_STATE).record_scheduled_occurrence(&rule.id, occurrence);
    match persist_state() {
        Ok(()) => true,
        Err(error) => {
            crate::log_line(&format!(
                "automation event {} cancelled: state save failed: {error}",
                rule.id
            ));
            crate::runtime::lock(&SAVE_ERRORS).insert(
                rule.id.clone(),
                crate::t_args(
                    "automation_checkpoint_failed",
                    &[&rule.name, &error.to_string()],
                ),
            );
            request_refresh();
            false
        }
    }
}

/// Display worker failures on the UI thread, with no scheduler locks held.
pub fn show_save_errors() {
    let errors = std::mem::take(&mut *crate::runtime::lock(&SAVE_ERRORS));
    if !errors.is_empty() {
        crate::warn_dialog("", &errors.into_values().collect::<Vec<_>>().join("\n\n"));
    }
}

fn persist_state() -> std::io::Result<()> {
    let path = crate::runtime::lock(&STATE_PATH)
        .clone()
        .ok_or_else(|| std::io::Error::other("automation state path unavailable"))?;
    let state = crate::runtime::lock(&RUNTIME_STATE).clone();
    auto::save_runtime_state(&path, &state)
}

/// Cancelling a one-shot rule disables only that specific rule (identified
/// by the rule_id carried in the pending action), not every once rule.
pub fn disable_rule_after_cancel(rule_id: &str) {
    let base = crate::runtime::lock(&RULES).clone();
    let mut rules = base.clone();
    let Some(rule) = rules
        .iter_mut()
        .find(|r| r.id == rule_id && r.trigger == auto::TRIGGER_ONCE && r.enabled)
    else {
        return;
    };
    rule.enabled = false;
    if let Err(err) = crate::save_automation_rules(&base, &rules) {
        crate::warn_dialog("", &err);
    }
}
/// A schedule is "due" when now is within the 2-minute grace past the
/// scheduled minute. The occurrence key prevents refiring within the same
/// day. This window also applies after startup or resume; older times are skipped.
fn schedule_due(now: &LocalNow, scheduled: &str) -> bool {
    let Some(sched) = auto::parse_hhmm(scheduled) else {
        return false;
    };
    let diff = now.minutes - sched;
    (0..2).contains(&diff)
}

fn rule_once_date(rule: &auto::Rule) -> Option<String> {
    if rule.trigger == auto::TRIGGER_ONCE {
        Some(rule.date.clone())
    } else {
        None
    }
}

pub struct LocalNow {
    pub date: String,
    pub minutes: i32,
    pub weekday: usize, // 0 = Sunday
}

fn local_now() -> Result<LocalNow, String> {
    unsafe {
        let st = windows::Win32::System::SystemInformation::GetLocalTime();
        Ok(LocalNow {
            date: format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay),
            minutes: st.wHour as i32 * 60 + st.wMinute as i32,
            weekday: st.wDayOfWeek as usize,
        })
    }
}

// Days-civil conversions (Howard Hinnant) for date + offset arithmetic.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y } as i32, m, d)
}

fn weekday_of(days: i64) -> usize {
    // 1970-01-01 was a Thursday (weekday 4 with Sunday = 0).
    (days + 4).rem_euclid(7) as usize
}

/// Next upcoming wall-clock action across enabled rules (Go nextScheduled),
/// formatted "YYYY-MM-DD HH:MM". Process start/exit triggers never schedule.
pub fn next_scheduled() -> Option<String> {
    let now = local_now().ok()?;
    let today_days = days_from_civil(
        now.date
            .get(..4)
            .and_then(|v| v.parse().ok())
            .unwrap_or(2000),
        now.date.get(5..7).and_then(|v| v.parse().ok()).unwrap_or(1),
        now.date
            .get(8..10)
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
    );
    let mut best: Option<(i64, i32)> = None; // (days, minutes)
    let rules = crate::runtime::lock(&RULES).clone();
    for rule in &rules {
        if !rule.enabled
            || !auto::is_event_action(&rule.action)
            || rule.trigger == auto::TRIGGER_PROCESS_STARTED
            || rule.trigger == auto::TRIGGER_PROCESS_EXITED
        {
            continue;
        }
        let Some(minutes) = auto::parse_hhmm(&rule.time) else {
            continue;
        };
        let mut consider = |days: i64| {
            let better = match best {
                None => true,
                Some((bdays, bminutes)) => (days, minutes) < (bdays, bminutes),
            };
            if better {
                best = Some((days, minutes));
            }
        };
        if rule.trigger == auto::TRIGGER_ONCE {
            let Some(day_days) = (|| {
                let days = days_from_civil(
                    rule.date.get(..4)?.parse().ok()?,
                    rule.date.get(5..7)?.parse().ok()?,
                    rule.date.get(8..10)?.parse().ok()?,
                );
                Some(days)
            })() else {
                continue;
            };
            if day_days < today_days || (day_days == today_days && minutes <= now.minutes) {
                continue;
            }
            consider(day_days);
            continue;
        }
        for offset in 0..=7i64 {
            let day_days = today_days + offset;
            if offset == 0 && minutes <= now.minutes {
                continue;
            }
            if rule.trigger == auto::TRIGGER_WEEKLY {
                let key = auto::weekday_key(weekday_of(day_days));
                if !rule.days.iter().any(|d| d.eq_ignore_ascii_case(key)) {
                    continue;
                }
            }
            consider(day_days);
        }
    }
    let (days, minutes) = best?;
    let (dy, dm, dd) = civil_from_days(days);
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        dy,
        dm,
        dd,
        minutes / 60,
        minutes % 60
    ))
}

fn day_matches(rule: &auto::Rule, now: &LocalNow) -> bool {
    match rule.trigger.as_str() {
        auto::TRIGGER_ONCE => now.date == rule.date,
        auto::TRIGGER_DAILY => true,
        auto::TRIGGER_WEEKLY => rule
            .days
            .contains(&auto::weekday_key(now.weekday).to_string()),
        _ => false,
    }
}

fn in_time_window(rule: &auto::Rule, now: &LocalNow) -> bool {
    let (Some(start), Some(end)) = (
        auto::parse_hhmm(&rule.time),
        auto::parse_hhmm(&rule.end_time),
    ) else {
        return false;
    };
    let day_matches = |weekday| {
        rule.days.is_empty()
            || rule
                .days
                .iter()
                .any(|day| day == auto::weekday_key(weekday))
    };
    if start == end {
        day_matches(now.weekday)
    } else if start < end {
        day_matches(now.weekday) && now.minutes >= start && now.minutes < end
    } else {
        // The after-midnight segment belongs to the day the window started.
        (now.minutes >= start && day_matches(now.weekday))
            || (now.minutes < end && day_matches((now.weekday + 6) % 7))
    }
}

/// `Some(true)` when a same-name instance resolved to the expected path and
/// `Some(false)` when absence is confirmed (no same-name instances at all,
/// or every one resolved and none matched). `None` when at least one
/// same-name instance could not be queried: a failed query is not evidence
/// of absence, so the caller holds the last state.
fn target_present(target: &auto::ProcessTarget, snapshot: &Snapshot) -> Option<bool> {
    match target.kind.as_str() {
        auto::MATCH_PATH => snapshot.matches_path(target, query_process_path),
        _ => Some(snapshot.names().contains(&target.executable.to_lowercase())),
    }
}

fn normalize(path: &str) -> String {
    let mut cleaned = path.replace('/', "\\").to_lowercase();
    while cleaned.ends_with('\\') && cleaned.len() > 3 {
        cleaned.pop();
    }
    cleaned
}

fn evaluate_logic(rule: &auto::Rule, present: &BTreeMap<String, bool>) -> bool {
    let hits: Vec<bool> = rule
        .processes
        .iter()
        .map(|t| present.get(&t.key()) == Some(&true))
        .collect();
    match rule.process_logic.as_str() {
        auto::LOGIC_ALL => !hits.is_empty() && hits.iter().all(|h| *h),
        auto::LOGIC_NONE => hits.iter().all(|h| !*h),
        _ => hits.iter().any(|h| *h),
    }
}

// ---- Process scanner -------------------------------------------------

use std::collections::BTreeSet;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

/// One snapshot of running processes: lowercase executable names, plus the
/// set of PIDs per name for on-demand path resolution.
pub struct Snapshot {
    names: BTreeSet<String>,
    pids_by_name: Vec<(String, u32)>,
}

impl Snapshot {
    pub fn take() -> Option<Snapshot> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
            let mut names = BTreeSet::new();
            let mut pids_by_name = Vec::new();
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let first = windows::Win32::System::Diagnostics::ToolHelp::Process32FirstW(
                snapshot, &mut entry,
            );
            if let Err(err) = &first {
                let _ = CloseHandle(snapshot);
                return (err.code()
                    == windows::core::HRESULT::from_win32(
                        windows::Win32::Foundation::ERROR_NO_MORE_FILES.0,
                    ))
                .then_some(Snapshot {
                    names,
                    pids_by_name,
                });
            }
            {
                loop {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|c| *c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
                    if !name.is_empty() {
                        names.insert(name.clone());
                        pids_by_name.push((name, entry.th32ProcessID));
                    }
                    if let Err(err) = windows::Win32::System::Diagnostics::ToolHelp::Process32NextW(
                        snapshot, &mut entry,
                    ) {
                        if err.code()
                            != windows::core::HRESULT::from_win32(
                                windows::Win32::Foundation::ERROR_NO_MORE_FILES.0,
                            )
                        {
                            let _ = CloseHandle(snapshot);
                            return None; // A partial list must not manufacture process exits.
                        }
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
            Some(Snapshot {
                names,
                pids_by_name,
            })
        }
    }

    /// Lowercase executable names currently running.
    pub fn names(&self) -> &BTreeSet<String> {
        &self.names
    }

    /// (lowercase name, pid) pairs for grouping and self-exclusion.
    pub fn pid_names(&self) -> &[(String, u32)] {
        &self.pids_by_name
    }

    pub fn count_target(&self, target: &auto::ProcessTarget) -> u32 {
        self.pids_by_name
            .iter()
            .filter(|(name, pid)| {
                *pid != std::process::id()
                    && name.eq_ignore_ascii_case(&target.executable)
                    && (target.kind != "path"
                        || query_process_path(*pid)
                            .is_some_and(|p| normalize(&p) == normalize(&target.path)))
            })
            .count() as u32
    }

    fn matches_path(
        &self,
        target: &auto::ProcessTarget,
        resolve: impl Fn(u32) -> Option<String>,
    ) -> Option<bool> {
        let expected = normalize(&target.path);
        let mut same_name = false;
        let mut unresolvable = false;
        let mut matched = false;
        for (name, pid) in &self.pids_by_name {
            if !name.eq_ignore_ascii_case(&target.executable) {
                continue;
            }
            same_name = true;
            match resolve(*pid) {
                Some(path) => {
                    if normalize(&path) == expected {
                        matched = true;
                    }
                }
                None => unresolvable = true,
            }
        }
        if matched {
            Some(true)
        } else if same_name && unresolvable {
            None
        } else {
            Some(false)
        }
    }

    /// Absolute lowercase path of the first process with this executable
    /// name, resolved through `PROCESS_QUERY_LIMITED_INFORMATION`.
    pub fn path_of(&self, executable_lower: &str) -> Option<String> {
        for (name, pid) in &self.pids_by_name {
            if name != executable_lower {
                continue;
            }
            if let Some(path) = query_process_path(*pid) {
                return Some(path.to_lowercase());
            }
        }
        None
    }
}

fn query_process_path(pid: u32) -> Option<String> {
    unsafe {
        let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = vec![0u16; 32768];
        let mut size = buffer.len() as u32;
        let ok = windows::Win32::System::Threading::QueryFullProcessImageNameW(
            handle,
            Default::default(),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if ok {
            Some(String::from_utf16_lossy(&buffer[..size as usize]))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod date_tests {
    use super::*;
    #[test]
    fn civil_dates_roundtrip_across_epoch_and_leap_boundaries() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(weekday_of(days_from_civil(2026, 9, 13)), 0);
        for date in [
            (1969, 12, 31),
            (1970, 1, 1),
            (2000, 2, 29),
            (2026, 9, 13),
            (2100, 3, 1),
        ] {
            assert_eq!(
                civil_from_days(days_from_civil(date.0, date.1, date.2)),
                date
            );
        }
        assert_eq!(
            civil_from_days(days_from_civil(2024, 2, 28) + 1),
            (2024, 2, 29)
        );
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;
    #[test]
    fn checkpoint_failure_blocks_dispatch_and_success_is_saved_before_notification() {
        let _test = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        let old_state = std::mem::take(&mut *crate::runtime::lock(&RUNTIME_STATE));
        let old_path = crate::runtime::lock(&STATE_PATH).take();
        let old_errors = std::mem::take(&mut *crate::runtime::lock(&SAVE_ERRORS));
        let directory = std::env::temp_dir().join(format!(
            "IdleTrigger-checkpoint-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let path = directory.join("IdleTrigger.state.json");
        // A directory at the destination makes the atomic replacement fail.
        std::fs::create_dir_all(&path).unwrap();
        *crate::runtime::lock(&STATE_PATH) = Some(path.clone());
        let rule = auto::Rule {
            id: "checkpoint-test".into(),
            name: "Checkpoint test".into(),
            action: auto::ACTION_LOCK.into(),
            trigger: auto::TRIGGER_DAILY.into(),
            ..Default::default()
        };
        let occurrence = "2026-09-13T12:00";
        dispatch_event(&rule, Some(occurrence.into()), || {
            panic!("failed save must not dispatch")
        });
        assert!(!ACTION_BUSY.load(Ordering::SeqCst));
        assert!(crate::runtime::lock(&PENDING_ACTION).is_none());
        assert!(
            crate::runtime::lock(&RUNTIME_STATE).has_scheduled_occurrence(&rule.id, occurrence)
        );
        assert!(crate::runtime::lock(&SAVE_ERRORS).contains_key(&rule.id));
        assert!(path.is_dir());

        std::fs::remove_dir(&path).unwrap();
        let next = "2026-09-14T12:00";
        let mut notified = false;
        dispatch_event(&rule, Some(next.into()), || {
            let (saved, error) = auto::load_runtime_state(&path);
            assert!(error.is_none());
            assert!(saved.has_scheduled_occurrence(&rule.id, next));
            notified = true;
            false // Do not deliver a real action to the UI.
        });
        assert!(notified);
        assert!(!ACTION_BUSY.load(Ordering::SeqCst));
        assert!(crate::runtime::lock(&PENDING_ACTION).is_none());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
        *crate::runtime::lock(&RUNTIME_STATE) = old_state;
        *crate::runtime::lock(&STATE_PATH) = old_path;
        *crate::runtime::lock(&SAVE_ERRORS) = old_errors;
    }

    #[test]
    fn scheduled_process_conditions_wait_and_expire_without_dispatch() {
        let _test = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        let previous_config = crate::runtime::lock(&crate::CONFIG)
            .replace(idletrigger_core::config::Config::default());
        crate::cfg_edit(|c| c.automation_enabled = true);
        let rule: auto::Rule = toml_edit::de::from_str(
            r#"
id = "condition-test"
name = "condition test"
enabled = true
action = "lock"
trigger = "daily"
time = "12:00"
process_logic = "any"
blocked_policy = "wait"
max_wait_minutes = 5
[[processes]]
match = "name"
executable = "missing.exe"
"#,
        )
        .unwrap();
        *crate::runtime::lock(&RULES) = vec![rule.clone()];
        let now = || LocalNow {
            date: "2026-09-13".into(),
            minutes: 720,
            weekday: 0,
        };
        let snapshot = || {
            Some(Snapshot {
                names: BTreeSet::new(),
                pids_by_name: vec![],
            })
        };
        let instant = std::time::Instant::now();
        tick_with_snapshot(false, snapshot(), now(), instant).unwrap();
        assert!(crate::runtime::lock(&PENDING_ACTION).is_none());
        assert!(!ACTION_BUSY.load(Ordering::SeqCst));
        assert_eq!(crate::runtime::lock(&WAITING_EVENTS).len(), 1);
        assert!(
            crate::runtime::lock(&RUNTIME_STATE)
                .last_occurrences
                .is_empty()
        );
        tick_with_snapshot(false, snapshot(), now(), instant + Duration::from_secs(60)).unwrap();
        assert_eq!(
            crate::runtime::lock(&WAITING_EVENTS)["condition-test"].deadline,
            instant + Duration::from_secs(300)
        );
        // A failed scan cannot postpone an elapsed waiting deadline.
        assert!(
            tick_with_snapshot(false, None, now(), instant + Duration::from_secs(300)).is_err()
        );
        assert!(crate::runtime::lock(&WAITING_EVENTS).is_empty());
        assert!(crate::runtime::lock(&PENDING_ACTION).is_none());
        assert_eq!(
            crate::runtime::lock(&RUNTIME_STATE).last_occurrences["condition-test"],
            "2026-09-13T12:00"
        );

        // Failed delivery releases both resources. A subsequent action can
        // reserve the slot; a busy action never overwrites its payload.
        dispatch_event(&rule, None, || false);
        assert!(!ACTION_BUSY.load(Ordering::SeqCst));
        assert!(crate::runtime::lock(&PENDING_ACTION).is_none());
        dispatch_event(&rule, None, || true);
        assert!(ACTION_BUSY.load(Ordering::SeqCst));
        let pending = crate::runtime::lock(&PENDING_ACTION).clone().unwrap();
        assert!(pending.is_current());
        crate::runtime::lock(&RULES)[0].name = "changed".into();
        assert!(!pending.is_current());
        crate::runtime::lock(&RULES)[0] = rule.clone();
        crate::cfg_edit(|c| c.automation_enabled = false);
        assert!(!pending.is_current());
        crate::cfg_edit(|c| c.automation_enabled = true);
        let mut second = rule;
        second.id = "second".into();
        dispatch_event(&second, None, || panic!("busy dispatch must not notify"));
        assert_eq!(
            crate::runtime::lock(&PENDING_ACTION)
                .as_ref()
                .unwrap()
                .rule_id,
            "condition-test"
        );
        *crate::runtime::lock(&PENDING_ACTION) = None;
        ACTION_BUSY.store(false, Ordering::SeqCst);

        // Time-only work still evaluates while the process scanner is unknown.
        second.processes.clear();
        second.action = auto::ACTION_STAY_AWAKE.into();
        second.trigger = auto::TRIGGER_TIME_WINDOW.into();
        second.time = "00:00".into();
        second.end_time = "23:59".into();
        *crate::runtime::lock(&RULES) = vec![second.clone()];
        assert!(tick_with_snapshot(false, None, now(), instant).is_err());
        assert!(overrides().stay_awake);
        second.action = auto::ACTION_LOCK.into();
        second.trigger = auto::TRIGGER_DAILY.into();
        second.time = "12:00".into();
        *crate::runtime::lock(&RULES) = vec![second];
        assert!(tick_with_snapshot(false, None, now(), instant).is_err());
        assert!(
            crate::runtime::lock(&RUNTIME_STATE)
                .has_scheduled_occurrence("second", "2026-09-13T12:00")
        );
        assert!(!ACTION_BUSY.load(Ordering::SeqCst)); // Null recipient is a failed dispatch.
        crate::runtime::lock(&RULES).clear();
        *crate::runtime::lock(&PROCESS_TRACKER) = ProcessTracker::default();
        clear_overrides();
        crate::runtime::lock(&RUNTIME_STATE)
            .last_occurrences
            .clear();
        crate::runtime::lock(&SAVE_ERRORS).clear();
        *crate::runtime::lock(&crate::CONFIG) = previous_config;
    }

    #[test]
    fn process_baselines_grace_and_rule_changes_use_known_observations() {
        let mut rule: auto::Rule = toml_edit::de::from_str(
            r#"
id = "edge"
name = "edge"
enabled = true
action = "lock"
trigger = "process_started"
[[processes]]
match = "name"
executable = "app.exe"
"#,
        )
        .unwrap();
        let mut tracker = ProcessTracker::default();
        let instant = std::time::Instant::now();
        let rules = |r: &auto::Rule| vec![r.clone()];
        let snapshot = |present: bool| Snapshot {
            names: if present {
                BTreeSet::from(["app.exe".into()])
            } else {
                BTreeSet::new()
            },
            pids_by_name: vec![],
        };
        let key = rule.processes[0].key();
        assert!(tracker.observe(&rules(&rule), None, instant).is_none());
        let known = tracker
            .observe(&rules(&rule), Some(&snapshot(true)), instant)
            .unwrap();
        assert!(!tracker.edge(&rule, known[&key])); // First successful scan is only a baseline.
        let update = |tracker: &mut ProcessTracker, rule: &auto::Rule, present, seconds| {
            let known = tracker
                .observe(
                    &rules(rule),
                    Some(&snapshot(present)),
                    instant + Duration::from_secs(seconds),
                )
                .unwrap();
            (known[&key], tracker.edge(rule, known[&key]))
        };
        assert_eq!(update(&mut tracker, &rule, false, 60), (true, false));
        assert_eq!(update(&mut tracker, &rule, false, 64), (true, false));
        assert_eq!(update(&mut tracker, &rule, true, 64), (true, false));
        assert_eq!(update(&mut tracker, &rule, false, 70), (true, false));
        assert_eq!(update(&mut tracker, &rule, false, 75), (false, false));
        assert_eq!(update(&mut tracker, &rule, true, 76), (true, true));
        rule.trigger = auto::TRIGGER_PROCESS_EXITED.into();
        assert_eq!(update(&mut tracker, &rule, true, 77), (true, false));
        assert_eq!(update(&mut tracker, &rule, false, 90), (true, false));
        assert_eq!(update(&mut tracker, &rule, false, 95), (false, true));
        assert_eq!(update(&mut tracker, &rule, false, 96), (false, false));
        rule.enabled = false;
        tracker.observe(&rules(&rule), None, instant);
        assert!(tracker.edges.is_empty());
        rule.enabled = true;
        assert_eq!(update(&mut tracker, &rule, true, 100), (true, false));
    }

    #[test]
    fn path_condition_checks_every_same_name_instance() {
        let snapshot = Snapshot {
            names: BTreeSet::from(["app.exe".into()]),
            pids_by_name: vec![
                ("app.exe".into(), 1),
                ("app.exe".into(), 2),
                ("other.exe".into(), 3),
            ],
        };
        let target = auto::ProcessTarget {
            kind: auto::MATCH_PATH.into(),
            executable: "App.EXE".into(),
            path: "D:/chosen/app.exe".into(),
        };
        assert_eq!(
            snapshot.matches_path(&target, |pid| match pid {
                1 => Some("C:\\wrong\\app.exe".into()),
                2 => Some("D:\\CHOSEN\\APP.EXE".into()),
                _ => panic!("unrelated executable must not be queried"),
            }),
            Some(true)
        );
        assert_eq!(
            snapshot.matches_path(&target, |pid| {
                (pid == 2).then(|| "D:\\chosen\\app.exe".into())
            }),
            Some(true)
        );
        // Every same-name query failing is uncertainty, not confirmed
        // absence: the caller must hold its previous state.
        assert_eq!(snapshot.matches_path(&target, |_| None), None);
    }

    #[test]
    fn unresolvable_paths_hold_presence_instead_of_exiting() {
        let mut presence = Presence::default();
        let now = std::time::Instant::now();
        // An uncertain first observation is a conservative absent baseline
        // (no retroactive process-start event once it resolves).
        assert!(!presence.update(None, now));
        assert!(presence.update(Some(true), now));
        // Query failures are not evidence of absence: hold past the grace.
        assert!(presence.update(None, now + Duration::from_secs(60)));
        // Confirmed absence still exits through the normal grace window.
        assert!(presence.update(Some(false), now + Duration::from_secs(61)));
        assert!(!presence.update(Some(false), now + Duration::from_secs(70)));
    }

    #[test]
    fn overnight_window_uses_start_day_and_schedule_grace_is_exclusive() {
        let rule: auto::Rule = toml_edit::de::from_str(
            r#"
id = "night"
name = "night"
enabled = true
action = "stay_awake"
trigger = "time_window"
time = "23:00"
end_time = "02:00"
days = ["sat"]
"#,
        )
        .unwrap();
        let at = |weekday, minutes| LocalNow {
            date: "2026-09-13".into(),
            weekday,
            minutes,
        };
        assert!(in_time_window(&rule, &at(6, 23 * 60)));
        assert!(in_time_window(&rule, &at(0, 60)));
        assert!(!in_time_window(&rule, &at(6, 60)));
        assert!(!in_time_window(&rule, &at(0, 120)));
        assert!(!in_time_window(&rule, &at(0, 23 * 60)));
        assert!(schedule_due(&at(0, 721), "12:00"));
        assert!(!schedule_due(&at(0, 722), "12:00"));
        let mut all_day = rule;
        all_day.end_time = all_day.time.clone();
        for minute in [0, 60, 23 * 60, 1439] {
            assert!(in_time_window(&all_day, &at(6, minute)));
            assert!(!in_time_window(&all_day, &at(0, minute)));
        }
    }

    #[test]
    fn malformed_times_never_due_and_never_match_a_window() {
        let rule: auto::Rule = toml_edit::de::from_str(
            r#"
id = "broken"
name = "broken"
enabled = true
action = "lock"
trigger = "time_window"
time = "ab:cd"
end_time = "99:99"
days = []
"#,
        )
        .unwrap();
        let at = |weekday: usize, minutes: i32| LocalNow {
            date: "2026-09-19".into(),
            minutes,
            weekday,
        };
        assert!(!schedule_due(&at(0, 0), "ab:cd"));
        assert!(!schedule_due(&at(0, 0), ""));
        for minute in [0, 1, 721, 1439] {
            assert!(!in_time_window(&rule, &at(6, minute)));
        }
    }
}
