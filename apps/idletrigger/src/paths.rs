//! Data-location resolution and the one-time relocation of EXE-adjacent
//! runtime files into `%LOCALAPPDATA%`.
//!
//! The tray owns the move (single winning instance, once per machine);
//! out-of-process readers (`ctxexec`, CLI probes) only ever resolve, in the
//! same order the tray would use. When AppData is unusable the session
//! falls back to legacy EXE-adjacent mode with the old file names, so a
//! later launch still recognizes those files and retries the move.

use std::io::Read;
use std::path::{Path, PathBuf};

const APP_DIR: &str = "IdleTrigger";
// Rollback probe only: a later-era build may already manage this
// machine's data under the successor folder name. When it exists (and the
// current-era folder has no configuration), follow it so downgrades keep
// the user's real configuration instead of a defaults reset.
const FUTURE_DIR: &str = "CandyKit";
const CONFIG: &str = "config.toml";
const STATE: &str = "state.json";
const LEGACY_CONFIG: &str = "IdleTrigger.toml";
const LEGACY_STATE: &str = "IdleTrigger.state.json";

/// Where this session keeps its writable runtime data.
pub(crate) struct DataLocation {
    dir: PathBuf,
    /// Legacy mode: files stay beside the EXE under the old names (AppData
    /// unavailable, or a devtools run without an explicit data override).
    legacy: bool,
}

impl DataLocation {
    pub(crate) fn config_file(&self) -> PathBuf {
        self.dir
            .join(if self.legacy { LEGACY_CONFIG } else { CONFIG })
    }

    pub(crate) fn state_file(&self) -> PathBuf {
        self.dir
            .join(if self.legacy { LEGACY_STATE } else { STATE })
    }
}

/// The new name for a legacy EXE-adjacent file, if it is one of ours.
/// Prefix rules cover the rotation sibling (`.log.1`), the save backup
/// (`.bak`), and crash leftovers (hidden `.{name}-{pid}.tmp`,
/// `.corrupt-{unix}` state copies).
fn mapped_name(name: &str) -> Option<String> {
    let (prefix, replacement) = if name.starts_with(".IdleTrigger.toml") {
        (".IdleTrigger.toml", ".config.toml")
    } else if name.starts_with("IdleTrigger.toml") {
        ("IdleTrigger.toml", CONFIG)
    } else if name.starts_with("IdleTrigger.log") {
        ("IdleTrigger.log", "app.log")
    } else if name.starts_with("IdleTrigger.state.json") {
        ("IdleTrigger.state.json", STATE)
    } else if name.starts_with(".IdleTrigger-state") {
        // Crash leftover of the state file's atomic save.
        (".IdleTrigger-state", ".state")
    } else {
        return None;
    };
    Some(format!("{replacement}{}", &name[prefix.len()..]))
}

fn local_app_data() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    unsafe {
        let raw = SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None).ok()?;
        if raw.is_null() {
            return None;
        }
        let path = raw.to_string().ok();
        CoTaskMemFree(Some(raw.as_ptr() as _));
        path.map(PathBuf::from)
    }
}

#[cfg(feature = "devtools")]
fn devtools_scratch_dir() -> Option<PathBuf> {
    if std::env::var("IDLETRIGGER_DEVTOOLS").as_deref() != Ok("1") {
        return None;
    }
    std::env::var("IDLETRIGGER_DEVTOOLS_APPDATA")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(not(feature = "devtools"))]
fn devtools_scratch_dir() -> Option<PathBuf> {
    None
}

#[cfg(feature = "devtools")]
fn devtools_legacy_mode() -> bool {
    std::env::var("IDLETRIGGER_DEVTOOLS").as_deref() == Ok("1") && devtools_scratch_dir().is_none()
}

#[cfg(not(feature = "devtools"))]
fn devtools_legacy_mode() -> bool {
    false
}

/// Panic-hook target folder: the AppData data folder (or a devtools
/// scratch). `None` means the hook should fall back to the EXE-adjacent
/// legacy location — panic recording must not depend on the resolver.
pub(crate) fn panic_file_dir() -> Option<PathBuf> {
    if let Some(scratch) = devtools_scratch_dir() {
        return Some(scratch);
    }
    if devtools_legacy_mode() {
        return None;
    }
    local_app_data().map(|base| base.join(APP_DIR))
}

/// Read-only resolution for out-of-process readers (CLI language probe,
/// context-menu host). Mirrors the tray's preference order but never
/// moves anything: current-era folder, successor folder (rollback probe),
/// legacy EXE-adjacent file, then the canonical default.
pub(crate) fn read_config_path() -> PathBuf {
    let base = local_app_data();
    if let Some(base) = &base {
        let current = base.join(APP_DIR).join(CONFIG);
        if current.is_file() {
            return current;
        }
        let future = base.join(FUTURE_DIR).join(CONFIG);
        if future.is_file() {
            return future;
        }
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        let legacy = exe_dir.join(LEGACY_CONFIG);
        if legacy.is_file() {
            return legacy;
        }
    }
    base.map(|base| base.join(APP_DIR).join(CONFIG))
        .unwrap_or_else(|| PathBuf::from(LEGACY_CONFIG))
}

/// Tray-only relocation plus resolution. Runs exactly once per session,
/// after the single-instance guard and before the first configuration
/// read. Notes are human-readable diagnostics for the startup log (this
/// runs before the log exists).
pub(crate) fn establish(exe_dir: &Path) -> (DataLocation, Vec<String>) {
    let mut notes = Vec::new();
    if let Some(scratch) = devtools_scratch_dir() {
        let ready = app_data_dir_ready(&scratch);
        let relocated = ready && relocate(&scratch, exe_dir, &mut notes);
        if relocated {
            return (
                DataLocation {
                    dir: scratch,
                    legacy: false,
                },
                notes,
            );
        }
        notes.push("devtools data folder rejected the relocation; using EXE-adjacent files".into());
        return (
            DataLocation {
                dir: exe_dir.to_path_buf(),
                legacy: true,
            },
            notes,
        );
    }
    if devtools_legacy_mode() {
        return (
            DataLocation {
                dir: exe_dir.to_path_buf(),
                legacy: true,
            },
            notes,
        );
    }
    let Some(base) = local_app_data() else {
        notes.push("AppData (Local) unavailable; keeping data files beside the EXE".into());
        return (
            DataLocation {
                dir: exe_dir.to_path_buf(),
                legacy: true,
            },
            notes,
        );
    };
    let (dir, dir_note) = resolve_app_dir(&base);
    if let Some(note) = dir_note {
        notes.push(note);
    }
    let ready = app_data_dir_ready(&dir);
    let relocated = ready && relocate(&dir, exe_dir, &mut notes);
    if !relocated {
        notes.push(if ready {
            format!(
                "relocation into {} incomplete; keeping data files beside the EXE for this session",
                dir.display()
            )
        } else {
            format!(
                "{} is not writable; keeping data files beside the EXE",
                dir.display()
            )
        });
        return (
            DataLocation {
                dir: exe_dir.to_path_buf(),
                legacy: true,
            },
            notes,
        );
    }
    (DataLocation { dir, legacy: false }, notes)
}

/// The folder must both exist (or be creatable) and accept writes; a
/// created-but-read-only AppData path must fall back to legacy mode, not
/// trap the session's saves.
fn app_data_dir_ready(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".write-probe");
    let writable = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&probe)
        .is_ok();
    let _ = std::fs::remove_file(&probe);
    writable
}

/// Preference between the two possible data folders under AppData:
/// the current era's folder, unless only the successor folder holds a
/// configuration (downgrade after a successor-era build managed this
/// machine).
fn resolve_app_dir(base: &Path) -> (PathBuf, Option<String>) {
    let current = base.join(APP_DIR);
    if current.join(CONFIG).is_file() {
        return (current, None);
    }
    let future = base.join(FUTURE_DIR);
    if future.join(CONFIG).is_file() {
        let note = format!(
            "found existing data under {}; following it",
            future.display()
        );
        return (future, Some(note));
    }
    (current, None)
}

/// One-way, idempotent move of legacy EXE-adjacent files into `dir`.
/// Existing files under `dir` always win (a stale EXE-side file is not
/// evidence of newer content); leftover sources are removed either way,
/// which is what makes "no legacy files beside the EXE" the natural no-op
/// on later launches.
///
/// Returns false when a critical data file (the main configuration or the
/// automation state) has no copy at the destination afterwards: the caller
/// must keep this session in legacy mode, so the untouched source keeps
/// serving reads and saves until the move succeeds on a later launch.
fn relocate(dir: &Path, exe_dir: &Path, notes: &mut Vec<String>) -> bool {
    let mut critical_ready = true;
    let Ok(entries) = std::fs::read_dir(exe_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let Some(name) = entry.file_name().into_string().ok() else {
            continue;
        };
        let Some(new_name) = mapped_name(&name) else {
            continue;
        };
        let critical = name == LEGACY_CONFIG || name == LEGACY_STATE;
        let src = exe_dir.join(&name);
        let dst = dir.join(&new_name);
        if dst.exists() {
            // AppData wins. Byte-identical is the common crash-between-
            // copy-and-delete case; differing bytes are an older
            // process's output whose mtime proves nothing.
            let identical = same_bytes(&src, &dst);
            if let Err(err) = std::fs::remove_file(&src) {
                notes.push(format!(
                    "relocation: keeping duplicate {} beside the EXE ({err})",
                    name
                ));
            } else if !identical {
                notes.push(format!(
                    "relocation: discarded superseded {name} beside the EXE"
                ));
            }
        } else if let Err(err) = copy_with_mtime(&src, &dst) {
            notes.push(format!("relocation: {name} stays beside the EXE ({err})"));
            // A half-written copy that cannot be removed would still pass
            // the dst check below, then win next launch's AppData-first
            // rule over the intact source.
            if critical && dst.is_file() && std::fs::remove_file(&dst).is_err() {
                critical_ready = false;
            }
        } else if !same_bytes(&src, &dst) {
            notes.push(format!(
                "relocation: {name} copy failed verification; retrying next launch"
            ));
            if critical && std::fs::remove_file(&dst).is_err() {
                critical_ready = false;
            }
        } else if let Err(err) = std::fs::remove_file(&src) {
            notes.push(format!(
                "relocation: moved {name} but could not remove the original ({err})"
            ));
        }
        if critical && !dst.is_file() {
            critical_ready = false;
        }
    }
    critical_ready
}

fn copy_with_mtime(src: &Path, dst: &Path) -> std::io::Result<()> {
    let modified = std::fs::metadata(src)?.modified().ok();
    std::fs::copy(src, dst)?;
    if let (Some(modified), Ok(file)) =
        (modified, std::fs::OpenOptions::new().write(true).open(dst))
    {
        let _ = file.set_modified(modified);
    }
    Ok(())
}

fn same_bytes(a: &Path, b: &Path) -> bool {
    let (Ok(mut file_a), Ok(mut file_b)) = (std::fs::File::open(a), std::fs::File::open(b)) else {
        return false;
    };
    let (mut chunk_a, mut chunk_b) = ([0u8; 64 * 1024], [0u8; 64 * 1024]);
    loop {
        let (Ok(read_a), Ok(read_b)) = (file_a.read(&mut chunk_a), file_b.read(&mut chunk_b))
        else {
            return false;
        };
        if read_a != read_b {
            return false;
        }
        if read_a == 0 {
            return true;
        }
        if chunk_a[..read_a] != chunk_b[..read_b] {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "idletrigger-paths-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn mapped_names_cover_family_and_leave_others_alone() {
        assert_eq!(
            mapped_name("IdleTrigger.toml").as_deref(),
            Some("config.toml")
        );
        assert_eq!(
            mapped_name("IdleTrigger.toml.bak").as_deref(),
            Some("config.toml.bak")
        );
        assert_eq!(
            mapped_name(".IdleTrigger.toml-1234.tmp").as_deref(),
            Some(".config.toml-1234.tmp")
        );
        assert_eq!(mapped_name("IdleTrigger.log").as_deref(), Some("app.log"));
        assert_eq!(
            mapped_name("IdleTrigger.log.1").as_deref(),
            Some("app.log.1")
        );
        assert_eq!(
            mapped_name("IdleTrigger.state.json").as_deref(),
            Some("state.json")
        );
        assert_eq!(
            mapped_name("IdleTrigger.state.json.corrupt-77").as_deref(),
            Some("state.json.corrupt-77")
        );
        assert_eq!(
            mapped_name(".IdleTrigger-state-1234.json.tmp").as_deref(),
            Some(".state-1234.json.tmp")
        );
        assert_eq!(mapped_name("IdleTrigger-x64.exe"), None);
        assert_eq!(mapped_name("IdleTrigger.example.toml"), None);
        assert_eq!(mapped_name("config.toml"), None);
    }

    #[test]
    fn relocate_moves_the_family_and_clears_the_source() {
        let root = temp_root("move");
        let exe_dir = root.join("exe");
        let data_dir = root.join("data");
        fs::create_dir_all(&exe_dir).unwrap();
        fs::create_dir_all(&data_dir).unwrap();
        write(&exe_dir.join("IdleTrigger.toml"), "toml");
        write(&exe_dir.join("IdleTrigger.toml.bak"), "bak");
        write(&exe_dir.join("IdleTrigger.log"), "log");
        write(&exe_dir.join("IdleTrigger.log.1"), "log1");
        write(&exe_dir.join("IdleTrigger.state.json"), "state");
        write(&exe_dir.join("IdleTrigger-x64.exe"), "binary");

        let mut notes = Vec::new();
        assert!(relocate(&data_dir, &exe_dir, &mut notes));

        assert_eq!(
            fs::read_to_string(data_dir.join("config.toml")).unwrap(),
            "toml"
        );
        assert_eq!(
            fs::read_to_string(data_dir.join("config.toml.bak")).unwrap(),
            "bak"
        );
        assert_eq!(fs::read_to_string(data_dir.join("app.log")).unwrap(), "log");
        assert_eq!(
            fs::read_to_string(data_dir.join("app.log.1")).unwrap(),
            "log1"
        );
        assert_eq!(
            fs::read_to_string(data_dir.join("state.json")).unwrap(),
            "state"
        );
        // Only the untouched family outsider remains beside the EXE.
        assert_eq!(
            fs::read_to_string(exe_dir.join("IdleTrigger-x64.exe")).unwrap(),
            "binary"
        );
        for name in [
            "IdleTrigger.toml",
            "IdleTrigger.toml.bak",
            "IdleTrigger.log",
            "IdleTrigger.log.1",
            "IdleTrigger.state.json",
        ] {
            assert!(!exe_dir.join(name).exists(), "{name} should be gone");
        }
        assert!(notes.is_empty(), "{notes:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn relocate_is_idempotent_when_nothing_remains() {
        let root = temp_root("idempotent");
        let exe_dir = root.join("exe");
        let data_dir = root.join("data");
        fs::create_dir_all(&exe_dir).unwrap();
        fs::create_dir_all(&data_dir).unwrap();
        let mut notes = Vec::new();
        assert!(relocate(&data_dir, &exe_dir, &mut notes));
        assert!(relocate(&data_dir, &exe_dir, &mut notes));
        assert!(notes.is_empty(), "{notes:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn incomplete_relocation_reports_unready() {
        let root = temp_root("incomplete");
        let exe_dir = root.join("exe");
        fs::create_dir_all(&exe_dir).unwrap();
        write(&exe_dir.join("IdleTrigger.toml"), "toml");
        write(&exe_dir.join("IdleTrigger.log"), "log");
        // A data path nested under a plain file can never be created.
        let blocker = root.join("blocker");
        write(&blocker, "file");
        let bad_dir = blocker.join("data");

        let mut notes = Vec::new();
        assert!(!relocate(&bad_dir, &exe_dir, &mut notes));

        // The sources stay intact for the legacy-mode session and retry.
        assert!(exe_dir.join("IdleTrigger.toml").is_file());
        assert!(exe_dir.join("IdleTrigger.log").is_file());
        assert!(!notes.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn existing_appdata_files_win_over_stale_sources() {
        let root = temp_root("conflict");
        let exe_dir = root.join("exe");
        let data_dir = root.join("data");
        fs::create_dir_all(&exe_dir).unwrap();
        fs::create_dir_all(&data_dir).unwrap();
        // A stale EXE-side file written later by an old version after the
        // move (defaults + one edit) must not clobber the real config.
        write(&exe_dir.join("IdleTrigger.toml"), "stale-but-newer");
        write(&data_dir.join("config.toml"), "real");

        let mut notes = Vec::new();
        relocate(&data_dir, &exe_dir, &mut notes);

        assert_eq!(
            fs::read_to_string(data_dir.join("config.toml")).unwrap(),
            "real"
        );
        assert!(!exe_dir.join("IdleTrigger.toml").exists());
        assert_eq!(notes.len(), 1, "{notes:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn identical_source_after_failed_delete_is_silent() {
        let root = temp_root("identical");
        let exe_dir = root.join("exe");
        let data_dir = root.join("data");
        fs::create_dir_all(&exe_dir).unwrap();
        fs::create_dir_all(&data_dir).unwrap();
        write(&exe_dir.join("IdleTrigger.log"), "same");
        write(&data_dir.join("app.log"), "same");

        let mut notes = Vec::new();
        relocate(&data_dir, &exe_dir, &mut notes);

        assert!(notes.is_empty(), "{notes:?}");
        assert!(!exe_dir.join("IdleTrigger.log").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_prefers_current_folder_then_successor_probe() {
        let root = temp_root("resolve");
        let empty = root.join("empty");
        fs::create_dir_all(&empty).unwrap();
        // Fresh machine: neither folder has data → current-era folder.
        let (dir, note) = resolve_app_dir(&empty);
        assert_eq!(dir, empty.join(APP_DIR));
        assert!(note.is_none());

        // Downgrade probe: only the successor folder has a config.
        let mixed = root.join("mixed");
        let future = mixed.join(FUTURE_DIR);
        fs::create_dir_all(&future).unwrap();
        write(&future.join(CONFIG), "future");
        let (dir, note) = resolve_app_dir(&mixed);
        assert_eq!(dir, future);
        assert!(note.is_some());

        // Current folder wins once it holds a config of its own.
        let current = mixed.join(APP_DIR);
        fs::create_dir_all(&current).unwrap();
        write(&current.join(CONFIG), "current");
        let (dir, note) = resolve_app_dir(&mixed);
        assert_eq!(dir, current);
        assert!(note.is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn legacy_names_stay_beside_the_exe_in_legacy_mode() {
        let dir = PathBuf::from("somewhere");
        let location = DataLocation {
            dir: dir.clone(),
            legacy: true,
        };
        assert_eq!(location.config_file(), dir.join("IdleTrigger.toml"));
        assert_eq!(location.state_file(), dir.join("IdleTrigger.state.json"));
        let location = DataLocation {
            dir: dir.clone(),
            legacy: false,
        };
        assert_eq!(location.config_file(), dir.join("config.toml"));
        assert_eq!(location.state_file(), dir.join("state.json"));
    }
}
