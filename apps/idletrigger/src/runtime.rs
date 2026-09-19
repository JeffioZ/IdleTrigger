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

pub(crate) fn cfg_map<T>(f: impl FnOnce(&config::Config) -> T) -> T {
    let guard = CONFIG.lock().unwrap();
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
    let mut guard = CONFIG.lock().unwrap();
    f(guard.as_mut().expect("config initialized"))
}

pub(crate) fn log_line(msg: &str) {
    use std::io::Write;
    // Recover from poisoning instead of skipping the write: the window-proc
    // panic guard logs through here, possibly right after another thread
    // poisoned LOG_FILE mid-write.
    let mut guard = LOG_FILE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard
        .as_ref()
        .and_then(|file| file.metadata().ok())
        .is_some_and(|meta| meta.len() >= 5 * 1024 * 1024)
    {
        guard.take();
        if let Some(path) = CONFIG_PATH
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .and_then(|path| path.parent())
        {
            *guard = open_log(&path.join("IdleTrigger.log")).ok();
        }
    }
    if let Some(file) = guard.as_mut() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "[{now}] [{}] {msg}", crate::APP_VERSION);
        let _ = file.flush();
    }
}

pub(crate) fn init_log(exe_dir: &std::path::Path) {
    use std::io::Write;
    let path = exe_dir.join("IdleTrigger.log");
    let mut current = LOG_FILE.lock().unwrap();
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
        let path = CONFIG_PATH.lock().unwrap().clone();
        if let Some(dir) = path.as_ref().and_then(|path| path.parent()) {
            init_log(dir);
        }
    } else {
        LOG_FILE.lock().unwrap().take();
    }
}

pub(crate) fn commit_config(
    edit: impl FnOnce(&mut config::Config, &mut toml_edit::DocumentMut) -> Result<(), String>,
) -> Result<(), String> {
    let writer = CONFIG_WRITER.lock().unwrap();
    if CONFIG_LOAD_FAILED.load(Ordering::SeqCst) {
        return Err(crate::t_pub("warning_config_recovery"));
    }
    let path = CONFIG_PATH
        .lock()
        .unwrap()
        .clone()
        .ok_or("configuration path unavailable")?;
    let source = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err.to_string()),
    };
    if source != *CONFIG_SOURCE.lock().unwrap() {
        return Err(crate::t_pub("settings_save_conflict"));
    }
    let mut candidate = CONFIG
        .lock()
        .unwrap()
        .clone()
        .ok_or("configuration unavailable")?;
    let mut doc = CONFIG_DOC
        .lock()
        .unwrap()
        .clone()
        .ok_or("configuration document unavailable")?;
    edit(&mut candidate, &mut doc)?;
    config::save(&path, &mut doc, &candidate)
        .map_err(|e| crate::t_pub("msg_config_save_failed").replacen("%s", &e.to_string(), 1))?;
    *CONFIG_SOURCE.lock().unwrap() = Some(doc.to_string());
    *CONFIG.lock().unwrap() = Some(candidate);
    *CONFIG_DOC.lock().unwrap() = Some(doc);
    // Rule publication is inside the writer boundary, preventing a later
    // reload from being overwritten by an older save's publication.
    crate::automation::reload_rules();
    drop(writer);
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
