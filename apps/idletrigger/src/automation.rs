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
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

use idletrigger_core::automation as auto;

/// Runtime state-action overrides layered on top of the manual config.
pub struct Overrides {
    pub nosleep_on: AtomicBool,
    pub keep_screen: AtomicBool,
    pub nosleep_paused: AtomicBool,
    pub idle_on: AtomicBool,
    pub idle_minutes: AtomicI32,
    pub idle_paused: AtomicBool,
}

pub static OVR: Overrides = Overrides {
    nosleep_on: AtomicBool::new(false),
    keep_screen: AtomicBool::new(false),
    nosleep_paused: AtomicBool::new(false),
    idle_on: AtomicBool::new(false),
    idle_minutes: AtomicI32::new(30),
    idle_paused: AtomicBool::new(false),
};

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
    pub action: String,
    pub seconds: i32,
    pub rule_id: String,
    /// Some(date) when the rule is `once`; cancel disables the rule.
    pub once_date: Option<String>,
}

pub static PENDING_ACTION: Mutex<Option<PendingAction>> = Mutex::new(None);
pub static ACTION_BUSY: AtomicBool = AtomicBool::new(false);

// Per-rule edge tracking.
static STATE_ACTIVE: Mutex<BTreeMap<String, bool>> = Mutex::new(BTreeMap::new());
static WAS_ANY_PRESENT: Mutex<BTreeMap<String, bool>> = Mutex::new(BTreeMap::new());
static LAST_PRESENT_AT: Mutex<BTreeMap<String, std::time::Instant>> = Mutex::new(BTreeMap::new());
static EXIT_FIRED: Mutex<BTreeMap<String, bool>> = Mutex::new(BTreeMap::new());

const SCAN_INTERVAL: Duration = Duration::from_secs(5);
const EXIT_GRACE: Duration = Duration::from_secs(5);

pub fn reload_rules() {
    // Parse the whole document and pull only the rules key: array-of-tables
    // items do not round-trip reliably through Item::to_string alone.
    #[derive(serde::Deserialize)]
    struct RulesDoc {
        #[serde(default)]
        automation_rules: Vec<auto::Rule>,
    }
    let full_text = {
        let doc_guard = crate::CONFIG_DOC.lock().unwrap();
        let Some(doc) = doc_guard.as_ref() else {
            return;
        };
        doc.to_string()
    };
    let parsed: RulesDoc = match toml_edit::de::from_str(&full_text) {
        Ok(parsed) => parsed,
        Err(err) => {
            crate::log_line(&format!("automation rules parse error: {err}"));
            RulesDoc {
                automation_rules: Vec::new(),
            }
        }
    };
    let (normalized, issues) = auto::prepare_rules(&parsed.automation_rules);
    *ISSUES.lock().unwrap() = issues.clone();
    let runtime = auto::runtime_rules(normalized, &issues);
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
    let enabled = crate::cfg_map(|c| c.automation_enabled);
    let snapshot = Snapshot::take();
    let rules = RULES.lock().unwrap().clone();
    if !enabled {
        clear_overrides();
        return Ok(());
    }
    let Some(snapshot) = snapshot else {
        return Err("process snapshot failed".into());
    };

    let now = local_now()?;
    for rule in &rules {
        if !rule.enabled {
            continue;
        }
        match rule.trigger.as_str() {
            auto::TRIGGER_PROCESS_RUNNING => {
                let present = evaluate_logic(rule, &snapshot);
                if state_edge(&rule.id, present) {
                    apply_state_action(
                        &rule.action,
                        present,
                        rule.keep_screen_on,
                        rule.idle_minutes,
                    );
                }
            }
            auto::TRIGGER_TIME_WINDOW => {
                let inside = in_time_window(rule, &now);
                if state_edge(&rule.id, inside) {
                    apply_state_action(
                        &rule.action,
                        inside,
                        rule.keep_screen_on,
                        rule.idle_minutes,
                    );
                }
            }
            auto::TRIGGER_ONCE | auto::TRIGGER_DAILY | auto::TRIGGER_WEEKLY => {
                if first_tick {
                    continue; // never backfill missed schedules at startup
                }
                if day_matches(rule, &now) && schedule_due(&now, &rule.time) {
                    let key = format!("{}:{}", rule.id, now.date);
                    let already = RUNTIME_STATE
                        .lock()
                        .unwrap()
                        .last_occurrences
                        .contains_key(&key);
                    if !already {
                        record_occurrence(key.clone());
                        fire_event(rule, rule_once_date(rule));
                    }
                }
            }
            auto::TRIGGER_PROCESS_STARTED => {
                let any = evaluate_any(rule, &snapshot);
                let was = WAS_ANY_PRESENT
                    .lock()
                    .unwrap()
                    .get(&rule.id)
                    .copied()
                    .unwrap_or(false);
                if first_tick {
                    WAS_ANY_PRESENT.lock().unwrap().insert(rule.id.clone(), any);
                    continue; // baseline: existing processes never backfill
                }
                if any && !was {
                    fire_event(rule, rule_once_date(rule));
                }
                WAS_ANY_PRESENT.lock().unwrap().insert(rule.id.clone(), any);
            }
            auto::TRIGGER_PROCESS_EXITED => {
                let any = evaluate_any(rule, &snapshot);
                let already_fired = EXIT_FIRED
                    .lock()
                    .unwrap()
                    .get(&rule.id)
                    .copied()
                    .unwrap_or(false);
                if any {
                    LAST_PRESENT_AT
                        .lock()
                        .unwrap()
                        .insert(rule.id.clone(), std::time::Instant::now());
                    if already_fired {
                        EXIT_FIRED.lock().unwrap().insert(rule.id.clone(), false);
                    }
                    continue;
                }
                if first_tick {
                    continue;
                }
                let elapsed_ok = {
                    let last_present = LAST_PRESENT_AT.lock().unwrap();
                    match last_present.get(&rule.id) {
                        Some(seen) => seen.elapsed() >= EXIT_GRACE,
                        None => false,
                    }
                };
                if !already_fired && elapsed_ok {
                    EXIT_FIRED.lock().unwrap().insert(rule.id.clone(), true);
                    fire_event(rule, rule_once_date(rule));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn clear_overrides() {
    let was_on = OVR.nosleep_on.swap(false, Ordering::SeqCst)
        | OVR.idle_on.swap(false, Ordering::SeqCst)
        | OVR.nosleep_paused.swap(false, Ordering::SeqCst)
        | OVR.idle_paused.swap(false, Ordering::SeqCst);
    if was_on {
        request_refresh();
    }
}

fn state_edge(id: &str, active: bool) -> bool {
    let mut map = STATE_ACTIVE.lock().unwrap();
    let previous = map.get(id).copied();
    map.insert(id.to_string(), active);
    previous != Some(active)
}

fn apply_state_action(action: &str, on: bool, keep_screen: bool, idle_minutes: i32) {
    match action {
        auto::ACTION_STAY_AWAKE => {
            OVR.nosleep_on.store(on, Ordering::SeqCst);
            if on {
                OVR.keep_screen.store(keep_screen, Ordering::SeqCst);
            }
        }
        auto::ACTION_PAUSE_STAY_AWAKE => OVR.nosleep_paused.store(on, Ordering::SeqCst),
        auto::ACTION_ENABLE_IDLE => {
            OVR.idle_on.store(on, Ordering::SeqCst);
            if on {
                OVR.idle_minutes.store(idle_minutes, Ordering::SeqCst);
            }
        }
        auto::ACTION_PAUSE_IDLE => OVR.idle_paused.store(on, Ordering::SeqCst),
        _ => return,
    }
    crate::log_line(&format!("automation state override {action}={on}"));
    request_refresh();
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
    if ACTION_BUSY.load(Ordering::SeqCst) {
        if rule.blocked_policy == auto::BLOCKED_WAIT && rule.max_wait_minutes > 0 {
            // Simplified wait: the next tick retries while the slot is busy.
            unrecord_occurrence_if_daily(&rule.id);
            crate::log_line(&format!(
                "automation event {} postponed (countdown busy)",
                rule.id
            ));
            return;
        }
        crate::log_line(&format!(
            "automation event {} skipped (countdown busy)",
            rule.id
        ));
        return;
    }
    let seconds = rule.warning_seconds.max(auto::MIN_WARNING_SECONDS);
    *PENDING_ACTION.lock().unwrap() = Some(PendingAction {
        action: rule.action.clone(),
        seconds,
        rule_id: rule.id.clone(),
        once_date,
    });
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            Some(crate::hwnd(&crate::HIDDEN)),
            crate::WM_ACTION_SHOW,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(0),
        );
    }
}

fn unrecord_occurrence_if_daily(rule_id: &str) {
    let now = local_now().ok();
    if let Some(now) = now {
        let key = format!("{rule_id}:{}", now.date);
        RUNTIME_STATE.lock().unwrap().last_occurrences.remove(&key);
    }
}

fn record_occurrence(key: String) {
    let now = local_now().ok();
    let date = now.map(|n| n.date).unwrap_or_default();
    RUNTIME_STATE
        .lock()
        .unwrap()
        .last_occurrences
        .insert(key, date);
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
    let changed = {
        let mut rules = RULES.lock().unwrap();
        let mut changed = false;
        for rule in rules.iter_mut() {
            if rule.id == rule_id && rule.trigger == auto::TRIGGER_ONCE && rule.enabled {
                rule.enabled = false;
                changed = true;
            }
        }
        changed
    };
    if !changed {
        return;
    }
    let text = toml_edit::ser::to_string(&*RULES.lock().unwrap()).unwrap_or_default();
    if let Ok(item) = text.parse::<toml_edit::Item>() {
        let mut doc = crate::CONFIG_DOC.lock().unwrap();
        if let Some(doc) = doc.as_mut() {
            doc["automation_rules"] = item;
        }
    }
    crate::persist_config();
}

/// A schedule is "due" when now is within the 2-minute grace past the
/// scheduled minute. The occurrence key prevents refiring within the same
/// day, so the grace only bridges sleep/resume gaps.
fn schedule_due(now: &LocalNow, scheduled: &str) -> bool {
    let parse = |s: &str| -> i32 {
        s.get(..2).and_then(|v| v.parse().ok()).unwrap_or(0) * 60
            + s.get(3..5).and_then(|v| v.parse().ok()).unwrap_or(0)
    };
    let sched = parse(scheduled);
    let diff = now.minutes - sched;
    (0..=2).contains(&diff)
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
    era * 146_097 + doe
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
    if !rule
        .days
        .contains(&auto::weekday_key(now.weekday).to_string())
    {
        return false;
    }
    let parse = |s: &str| -> i32 {
        let h: i32 = s.get(..2).and_then(|v| v.parse().ok()).unwrap_or(0);
        let m: i32 = s.get(3..5).and_then(|v| v.parse().ok()).unwrap_or(0);
        h * 60 + m
    };
    let start = parse(&rule.time);
    let end = parse(&rule.end_time);
    if start <= end {
        now.minutes >= start && now.minutes < end
    } else {
        // Window crossing midnight.
        now.minutes >= start || now.minutes < end
    }
}

fn target_present(target: &auto::ProcessTarget, snapshot: &Snapshot) -> bool {
    match target.kind.as_str() {
        auto::MATCH_PATH => snapshot
            .path_of(&target.executable.to_lowercase())
            .map(|p| p == normalize(&target.path))
            .unwrap_or(false),
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

fn evaluate_logic(rule: &auto::Rule, snapshot: &Snapshot) -> bool {
    let hits: Vec<bool> = rule
        .processes
        .iter()
        .map(|t| target_present(t, snapshot))
        .collect();
    match rule.process_logic.as_str() {
        auto::LOGIC_ALL => !hits.is_empty() && hits.iter().all(|h| *h),
        auto::LOGIC_NONE => hits.iter().all(|h| !*h),
        _ => hits.iter().any(|h| *h),
    }
}

fn evaluate_any(rule: &auto::Rule, snapshot: &Snapshot) -> bool {
    rule.processes.iter().any(|t| target_present(t, snapshot))
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
            if windows::Win32::System::Diagnostics::ToolHelp::Process32FirstW(snapshot, &mut entry)
                .is_ok()
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
                    if !windows::Win32::System::Diagnostics::ToolHelp::Process32NextW(
                        snapshot, &mut entry,
                    )
                    .is_ok()
                    {
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
        let mut buffer = [0u16; 1024];
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
