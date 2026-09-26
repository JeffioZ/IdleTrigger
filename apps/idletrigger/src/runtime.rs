//! Runtime state layer: the loaded configuration (values, TOML document,
//! source snapshot), the serialized write transaction around it, and the
//! log file. Sibling modules reach all of it through the crate-root
//! re-export, exactly as they did when it lived in main.rs.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use idletrigger_core::config;

pub(crate) static CONFIG: Mutex<Option<config::Config>> = Mutex::new(None);
pub(crate) static CONFIG_DOC: Mutex<Option<toml_edit::DocumentMut>> = Mutex::new(None);
pub(crate) static CONFIG_WRITER: Mutex<()> = Mutex::new(());
#[cfg(test)]
pub(crate) static CONFIG_TEST_LOCK: Mutex<()> = Mutex::new(());
pub(crate) static CONFIG_SOURCE: Mutex<Option<String>> = Mutex::new(None);
pub(crate) static CONFIG_LOAD_FAILED: AtomicBool = AtomicBool::new(false);
pub(crate) static CONFIG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
pub(crate) static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Locks a runtime mutex, recovering from poisoning. A panic caught by the
/// window-proc guard may have poisoned the lock mid-hold; published state is
/// only ever swapped in from prepared clones, so the last consistent value
/// stays usable. Without recovery every later access would panic again and
/// degrade the process into a zombie that only logs.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Extracts a human-readable message from a caught panic payload.
pub(crate) fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|value| (*value).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "opaque panic payload".into())
}

/// Runs a background-thread body, logging instead of letting the panic kill
/// the thread silently — the GUI subsystem has no stderr to surface it on,
/// so an unwrapped panic would just stop the subsystem. Returns `true` when
/// the body panicked, so one-shot threads can reset their busy flags.
pub(crate) fn catch_and_log(name: &str, body: impl FnOnce()) -> bool {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(()) => false,
        Err(payload) => {
            log_line(&format!("{name} thread panic: {}", panic_message(payload)));
            true
        }
    }
}

pub(crate) fn cfg_map<T>(f: impl FnOnce(&config::Config) -> T) -> T {
    let guard = lock(&CONFIG);
    #[cfg(feature = "devtools")]
    if crate::devtools::ENABLED.load(Ordering::SeqCst) {
        let mut runtime = guard.as_ref().expect("config initialized").clone();
        drop(guard);
        crate::devtools::apply_idle_test(&mut runtime);
        crate::devtools::apply_config_overrides(&mut runtime);
        return f(&runtime);
    }
    f(guard.as_ref().expect("config initialized"))
}

#[cfg(test)]
pub(crate) fn cfg_edit<T>(f: impl FnOnce(&mut config::Config) -> T) -> T {
    let mut guard = lock(&CONFIG);
    f(guard.as_mut().expect("config initialized"))
}

pub(crate) fn log_line(msg: &str) {
    use std::io::Write;
    // The recovering lock keeps the panic path logging: the window-proc
    // guard may log right after another thread poisoned LOG_FILE mid-write.
    let mut guard = lock(&LOG_FILE);
    if guard
        .as_ref()
        .and_then(|file| file.metadata().ok())
        .is_some_and(|meta| meta.len() >= 5 * 1024 * 1024)
    {
        guard.take();
        if let Some(path) = lock(&CONFIG_PATH).as_ref().and_then(|path| path.parent()) {
            *guard = open_log(&path.join("IdleTrigger.log")).ok();
        }
    }
    if let Some(file) = guard.as_mut() {
        let _ = writeln!(
            file,
            "[{}] [{}] {msg}",
            local_timestamp(),
            crate::APP_VERSION
        );
        let _ = file.flush();
    }
}

/// Wall-clock local time for log lines; far easier to correlate with user
/// reports than raw Unix seconds.
fn local_timestamp() -> String {
    let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond
    )
}

pub(crate) fn init_log(exe_dir: &std::path::Path) {
    use std::io::Write;
    let path = exe_dir.join("IdleTrigger.log");
    let mut current = lock(&LOG_FILE);
    if current.is_some() {
        return;
    }
    if let Ok(mut file) = open_log(&path) {
        let _ = writeln!(file, "---- session {} ----", std::process::id());
        *current = Some(file);
    }
}

pub(crate) fn open_log(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    if std::fs::metadata(path).is_ok_and(|meta| meta.len() >= 5 * 1024 * 1024) {
        std::fs::rename(path, path.with_extension("log.1"))?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

pub(crate) fn sync_logging() {
    if cfg_map(|c| c.logging_enabled) {
        let path = lock(&CONFIG_PATH).clone();
        if let Some(dir) = path.as_ref().and_then(|path| path.parent()) {
            init_log(dir);
        }
    } else {
        lock(&LOG_FILE).take();
    }
}

pub(crate) fn commit_config(
    edit: impl FnOnce(&mut config::Config, &mut toml_edit::DocumentMut) -> Result<(), String>,
) -> Result<(), String> {
    let writer = lock(&CONFIG_WRITER);
    if CONFIG_LOAD_FAILED.load(Ordering::SeqCst) {
        return Err(crate::t_pub("warning_config_recovery"));
    }
    let path = lock(&CONFIG_PATH)
        .clone()
        .ok_or("configuration path unavailable")?;
    let source = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err.to_string()),
    };
    if source != *lock(&CONFIG_SOURCE) {
        return Err(crate::t_pub("settings_save_conflict"));
    }
    let mut candidate = lock(&CONFIG).clone().ok_or("configuration unavailable")?;
    let mut doc = lock(&CONFIG_DOC)
        .clone()
        .ok_or("configuration document unavailable")?;
    let nosleep_was_on = candidate.nosleep_enabled;
    edit(&mut candidate, &mut doc)?;
    let stay_awake_turned_off = nosleep_was_on && !candidate.nosleep_enabled;
    config::save(&path, &mut doc, &candidate)
        .map_err(|e| crate::t_pub("msg_config_save_failed").replacen("%s", &e.to_string(), 1))?;
    *lock(&CONFIG_SOURCE) = Some(doc.to_string());
    *lock(&CONFIG) = Some(candidate);
    *lock(&CONFIG_DOC) = Some(doc);
    // Rule publication is inside the writer boundary, preventing a later
    // reload from being overwritten by an older save's publication.
    crate::automation::reload_rules();
    drop(writer);
    // "Off" always means off: a save that turned the Stay Awake switch off —
    // a direct toggle or the monitor's mutual exclusion — also drops the
    // timed overlay. Unrelated saves (monitor off, automation, rules) leave
    // the runtime overlay alone; it expires on its own.
    if stay_awake_turned_off {
        crate::sync_timed_with_manual();
    }
    crate::theme_engine::wake();
    sync_logging();
    log_line("configuration saved");
    Ok(())
}

pub(crate) fn edit_config(edit: impl FnOnce(&mut config::Config)) -> Result<(), String> {
    commit_config(|config, _| {
        edit(config);
        Ok(())
    })
}

pub(crate) fn save_automation_rules(
    base: &[idletrigger_core::automation::Rule],
    rules: &[idletrigger_core::automation::Rule],
) -> Result<(), String> {
    commit_config(|_, doc| {
        idletrigger_core::rule_document::update(doc, base, rules).map_err(|err| {
            if err == "automation_changed_external" {
                crate::t_pub(&err)
            } else {
                err
            }
        })
    })
}
