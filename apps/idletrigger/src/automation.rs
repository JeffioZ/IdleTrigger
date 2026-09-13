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
});
static ACTIVE_RULES: Mutex<Vec<auto::Rule>> = Mutex::new(Vec::new());

pub fn overrides() -> auto::EffectiveState {
    OVERRIDES.lock().unwrap().clone()
}

pub static RULES: Mutex<Vec<auto::Rule>> = Mutex::new(Vec::new());

/// Validation issues from the last reload (Go State.Issues): the manager list
/// marks the affected rows as invalid instead of hiding them.
pub static ISSUES: Mutex<Vec<auto::RuleIssue>> = Mutex::new(Vec::new());
pub static STATE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
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
                RULES
                    .lock()
                    .unwrap()
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
    present: bool,
    absent_since: Option<std::time::Instant>,
}
impl Presence {
    fn update(&mut self, present: bool, now: std::time::Instant) -> bool {
        if present {
            self.present = true;
            self.absent_since = None;
        } else if self.present {
            let since = self.absent_since.get_or_insert(now);
            if now.saturating_duration_since(*since) >= EXIT_GRACE {
                self.present = false;
                self.absent_since = None;
            }
        }
        self.present
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
        Some(
            targets
                .into_iter()
                .map(|(key, target)| {
                    let present = self
                        .targets
                        .entry(key.clone())
                        .or_default()
                        .update(target_present(target, snapshot), now);
                    (key, present)
                })
                .collect(),
        )
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
static PROCESS_TRACKER: Mutex<ProcessTracker> = Mutex::new(ProcessTracker {
    targets: BTreeMap::new(),
    edges: BTreeMap::new(),
});
struct WaitingEvent {
    rule: auto::Rule,
    occurrence: String,
    deadline: std::time::Instant,
}
static WAITING_EVENTS: Mutex<BTreeMap<String, WaitingEvent>> = Mutex::new(BTreeMap::new());

const SCAN_INTERVAL: Duration = Duration::from_secs(5);
const EXIT_GRACE: Duration = Duration::from_secs(5);

pub fn reload_rules() {
    // Parse the whole document and pull only the rules key: array-of-tables
    // items do not round-trip reliably through Item::to_string alone.
    let parsed = {
        let doc_guard = crate::CONFIG_DOC.lock().unwrap();
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
    *ISSUES.lock().unwrap() = issues.clone();
    let enabled = crate::cfg_map(|c| c.automation_enabled);
    let mut active = ACTIVE_RULES.lock().unwrap();
    active.retain(|rule| {
        enabled
            && runtime
                .iter()
                .any(|current| current.enabled && current == rule)
    });
    publish_overrides(auto::aggregate_state(active.iter()));
    drop(active);
    if !enabled {
        WAITING_EVENTS.lock().unwrap().clear();
        *PROCESS_TRACKER.lock().unwrap() = ProcessTracker::default();
    }
    *RULES.lock().unwrap() = runtime;
    crate::log_line(&format!(
        "automation rules loaded: {} ({} issues)",
        RULES.lock().unwrap().len(),
        issues.len()
    ));
}

pub fn spawn(config_dir: PathBuf) {
    let state_path = config_dir.join("IdleTrigger.state.json");
    let (state, err) = auto::load_runtime_state(&state_path);
    if let Some(err) = err {
        crate::log_line(&format!("automation state: {err}"));
    }
    *RUNTIME_STATE.lock().unwrap() = state;
    *STATE_PATH.lock().unwrap() = Some(state_path);

    std::thread::Builder::new()
        .name("automation".into())
        .spawn(|| {
            let mut first_tick = true;
            loop {
                if crate::EXITING.load(Ordering::SeqCst) {
                    return;
                }
                if let Err(err) = tick(first_tick) {
                    crate::log_line(&format!("automation tick error: {err}"));
                }
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
    let _configuration = crate::CONFIG_WRITER.lock().unwrap();
    let enabled = crate::cfg_map(|c| c.automation_enabled);
    let rules = RULES.lock().unwrap().clone();
    if !enabled {
        WAITING_EVENTS.lock().unwrap().clear();
        *PROCESS_TRACKER.lock().unwrap() = ProcessTracker::default();
        clear_overrides();
        return Ok(());
    }
    let observations =
        PROCESS_TRACKER
            .lock()
            .unwrap()
            .observe(&rules, snapshot.as_ref(), monotonic_now);
    let condition = |rule: &auto::Rule| {
        rule.processes.is_empty()
            || observations
                .as_ref()
                .is_some_and(|present| evaluate_logic(rule, present))
    };

    let mut active = Vec::new();
    let waiting = std::mem::take(&mut *WAITING_EVENTS.lock().unwrap());
    for (id, pending) in waiting {
        let rule = &pending.rule;
        if !rules
            .iter()
            .any(|current| current == rule && current.enabled)
        {
            continue;
        }
        if monotonic_now >= pending.deadline {
            record_occurrence(&id, pending.occurrence);
        } else if condition(rule) {
            record_occurrence(&id, pending.occurrence.clone());
            fire_event(rule, rule_once_date(rule));
        } else {
            WAITING_EVENTS.lock().unwrap().insert(id, pending);
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
                if first_tick || WAITING_EVENTS.lock().unwrap().contains_key(&rule.id) {
                    continue; // Establish the first observation before scheduling events.
                }
                if day_matches(rule, &now) && schedule_due(&now, &rule.time) {
                    let occurrence = format!("{}T{}", now.date, rule.time);
                    let already = RUNTIME_STATE
                        .lock()
                        .unwrap()
                        .has_scheduled_occurrence(&rule.id, &occurrence);
                    if !already {
                        if condition(rule) {
                            record_occurrence(&rule.id, occurrence);
                            fire_event(rule, rule_once_date(rule));
                        } else if rule.blocked_policy == auto::BLOCKED_WAIT
                            && rule.max_wait_minutes > 0
                        {
                            WAITING_EVENTS.lock().unwrap().insert(
                                rule.id.clone(),
                                WaitingEvent {
                                    rule: rule.clone(),
                                    occurrence,
                                    deadline: monotonic_now
                                        + Duration::from_secs(rule.max_wait_minutes as u64 * 60),
                                },
                            );
                        } else {
                            record_occurrence(&rule.id, occurrence);
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
                let fire = PROCESS_TRACKER.lock().unwrap().edge(rule, any);
                if fire {
                    fire_event(rule, rule_once_date(rule));
                }
            }
            _ => {}
        }
    }
    *ACTIVE_RULES.lock().unwrap() = active.iter().map(|rule| (*rule).clone()).collect();
    publish_overrides(auto::aggregate_state(active));
    if snapshot.is_none() {
        Err("process snapshot failed".into())
    } else {
        Ok(())
    }
}

fn clear_overrides() {
    ACTIVE_RULES.lock().unwrap().clear();
    publish_overrides(auto::EffectiveState::default());
}

fn publish_overrides(state: auto::EffectiveState) {
    let mut current = OVERRIDES.lock().unwrap();
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

fn fire_event(rule: &auto::Rule, once_date: Option<String>) {
    dispatch_event(rule, once_date, || unsafe {
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

fn dispatch_event(rule: &auto::Rule, once_date: Option<String>, notify: impl FnOnce() -> bool) {
    if ACTION_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        crate::log_line(&format!(
            "automation event {} skipped (countdown busy)",
            rule.id
        ));
        return;
    }
    let seconds = rule.warning_seconds.max(auto::MIN_WARNING_SECONDS);
    let mut pending = PENDING_ACTION.lock().unwrap();
    *pending = Some(PendingAction {
        rule: Some(rule.clone()),
        action: rule.action.clone(),
        seconds,
        rule_id: rule.id.clone(),
        once_date,
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

fn record_occurrence(rule_id: &str, occurrence: String) {
    RUNTIME_STATE
        .lock()
        .unwrap()
        .record_scheduled_occurrence(rule_id, occurrence);
    persist_state();
}

/// Persists the runtime state; call after occurrence changes.
pub fn persist_state() {
    let Some(path) = STATE_PATH.lock().unwrap().clone() else {
        return;
    };
    let state = RUNTIME_STATE.lock().unwrap().clone();
    if let Err(err) = auto::save_runtime_state(&path, &state) {
        crate::log_line(&format!("automation state save failed: {err}"));
    }
}

/// Cancelling a one-shot rule disables only that specific rule (identified
/// by the rule_id carried in the pending action), not every once rule.
pub fn disable_rule_after_cancel(rule_id: &str) {
    let base = RULES.lock().unwrap().clone();
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
    let parse = |s: &str| -> i32 {
        s.get(..2).and_then(|v| v.parse().ok()).unwrap_or(0) * 60
            + s.get(3..5).and_then(|v| v.parse().ok()).unwrap_or(0)
    };
    let sched = parse(scheduled);
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
    let parse_hhmm = |v: &str| -> Option<i32> {
        let h: i32 = v.get(..2)?.parse().ok()?;
        let m: i32 = v.get(3..5)?.parse().ok()?;
        Some(h * 60 + m)
    };
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
    let rules = RULES.lock().unwrap().clone();
    for rule in &rules {
        if !rule.enabled
            || !auto::is_event_action(&rule.action)
            || rule.trigger == auto::TRIGGER_PROCESS_STARTED
            || rule.trigger == auto::TRIGGER_PROCESS_EXITED
        {
            continue;
        }
        let Some(minutes) = parse_hhmm(&rule.time) else {
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
    let parse = |s: &str| -> i32 {
        let h: i32 = s.get(..2).and_then(|v| v.parse().ok()).unwrap_or(0);
        let m: i32 = s.get(3..5).and_then(|v| v.parse().ok()).unwrap_or(0);
        h * 60 + m
    };
    let start = parse(&rule.time);
    let end = parse(&rule.end_time);
    let day_matches = |weekday| {
        rule.days.is_empty()
            || rule
                .days
                .iter()
                .any(|day| day == auto::weekday_key(weekday))
    };
    if start <= end {
        day_matches(now.weekday) && now.minutes >= start && now.minutes < end
    } else {
        // The after-midnight segment belongs to the day the window started.
        (now.minutes >= start && day_matches(now.weekday))
            || (now.minutes < end && day_matches((now.weekday + 6) % 7))
    }
}

fn target_present(target: &auto::ProcessTarget, snapshot: &Snapshot) -> bool {
    match target.kind.as_str() {
        auto::MATCH_PATH => snapshot.matches_path(target, query_process_path),
        _ => snapshot.names().contains(&target.executable.to_lowercase()),
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
    ) -> bool {
        let expected = normalize(&target.path);
        self.pids_by_name
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case(&target.executable))
            .any(|(_, pid)| resolve(*pid).is_some_and(|path| normalize(&path) == expected))
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
    fn scheduled_process_conditions_wait_and_expire_without_dispatch() {
        let _test = crate::CONFIG_TEST_LOCK.lock().unwrap();
        let previous_config = crate::CONFIG
            .lock()
            .unwrap()
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
        *RULES.lock().unwrap() = vec![rule.clone()];
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
        assert!(PENDING_ACTION.lock().unwrap().is_none());
        assert!(!ACTION_BUSY.load(Ordering::SeqCst));
        assert_eq!(WAITING_EVENTS.lock().unwrap().len(), 1);
        assert!(RUNTIME_STATE.lock().unwrap().last_occurrences.is_empty());
        tick_with_snapshot(false, snapshot(), now(), instant + Duration::from_secs(60)).unwrap();
        assert_eq!(
            WAITING_EVENTS.lock().unwrap()["condition-test"].deadline,
            instant + Duration::from_secs(300)
        );
        // A failed scan cannot postpone an elapsed waiting deadline.
        assert!(
            tick_with_snapshot(false, None, now(), instant + Duration::from_secs(300)).is_err()
        );
        assert!(WAITING_EVENTS.lock().unwrap().is_empty());
        assert!(PENDING_ACTION.lock().unwrap().is_none());
        assert_eq!(
            RUNTIME_STATE.lock().unwrap().last_occurrences["condition-test"],
            "2026-09-13T12:00"
        );

        // Failed delivery releases both resources. A subsequent action can
        // reserve the slot; a busy action never overwrites its payload.
        dispatch_event(&rule, None, || false);
        assert!(!ACTION_BUSY.load(Ordering::SeqCst));
        assert!(PENDING_ACTION.lock().unwrap().is_none());
        dispatch_event(&rule, None, || true);
        assert!(ACTION_BUSY.load(Ordering::SeqCst));
        let pending = PENDING_ACTION.lock().unwrap().clone().unwrap();
        assert!(pending.is_current());
        RULES.lock().unwrap()[0].name = "changed".into();
        assert!(!pending.is_current());
        RULES.lock().unwrap()[0] = rule.clone();
        crate::cfg_edit(|c| c.automation_enabled = false);
        assert!(!pending.is_current());
        crate::cfg_edit(|c| c.automation_enabled = true);
        let mut second = rule;
        second.id = "second".into();
        dispatch_event(&second, None, || panic!("busy dispatch must not notify"));
        assert_eq!(
            PENDING_ACTION.lock().unwrap().as_ref().unwrap().rule_id,
            "condition-test"
        );
        *PENDING_ACTION.lock().unwrap() = None;
        ACTION_BUSY.store(false, Ordering::SeqCst);

        // Time-only work still evaluates while the process scanner is unknown.
        second.processes.clear();
        second.action = auto::ACTION_STAY_AWAKE.into();
        second.trigger = auto::TRIGGER_TIME_WINDOW.into();
        second.time = "00:00".into();
        second.end_time = "23:59".into();
        *RULES.lock().unwrap() = vec![second.clone()];
        assert!(tick_with_snapshot(false, None, now(), instant).is_err());
        assert!(overrides().stay_awake);
        second.action = auto::ACTION_LOCK.into();
        second.trigger = auto::TRIGGER_DAILY.into();
        second.time = "12:00".into();
        *RULES.lock().unwrap() = vec![second];
        assert!(tick_with_snapshot(false, None, now(), instant).is_err());
        assert!(
            RUNTIME_STATE
                .lock()
                .unwrap()
                .has_scheduled_occurrence("second", "2026-09-13T12:00")
        );
        assert!(!ACTION_BUSY.load(Ordering::SeqCst)); // Null recipient is a failed dispatch.
        RULES.lock().unwrap().clear();
        *PROCESS_TRACKER.lock().unwrap() = ProcessTracker::default();
        clear_overrides();
        RUNTIME_STATE.lock().unwrap().last_occurrences.clear();
        *crate::CONFIG.lock().unwrap() = previous_config;
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
        assert!(snapshot.matches_path(&target, |pid| match pid {
            1 => Some("C:\\wrong\\app.exe".into()),
            2 => Some("D:\\CHOSEN\\APP.EXE".into()),
            _ => panic!("unrelated executable must not be queried"),
        }));
        assert!(snapshot.matches_path(&target, |pid| {
            (pid == 2).then(|| "D:\\chosen\\app.exe".into())
        }));
        assert!(!snapshot.matches_path(&target, |_| None));
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
    }
}
