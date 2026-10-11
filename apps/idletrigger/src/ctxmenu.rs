//! Registry sync engine for the Explorer context-menu integration.
//!
//! Every key this module owns sits under `HKCU\Software\Classes\<root>\shell`
//! and starts with the `IdleTrigger` verb prefix, so a prefix scan is the
//! complete ownership set: sync computes the desired key/value map from the
//! live configuration, deletes stale owned keys, and rewrites changed ones.
//! Nothing here ever touches keys owned by other software.

use idletrigger_core::ctx_menu as core_ctx;
use std::collections::BTreeMap;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY,
    REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW,
    RegDeleteValueW, RegEnumKeyExW, RegOpenKeyExW, RegQueryInfoKeyW, RegQueryValueExW,
    RegSetValueExW,
};
use windows::core::PCWSTR;

use crate::wide;

const CLASSES: &str = "Software\\Classes";
/// Roots the copy-path cascade registers under. `AllFilesystemObjects`
/// covers files and folders, `Drive` drive letters, `Directory\Background`
/// the empty area of a folder (and the desktop).
const COPY_ROOTS: [&str; 3] = ["AllFilesystemObjects", "Drive", "Directory\\Background"];
/// Every root any owned verb may live under (for the ownership scan).
const ALL_ROOTS: [&str; 5] = [
    "*",
    "Directory",
    "Directory\\Background",
    "AllFilesystemObjects",
    "Drive",
];

/// The well-known (undocumented) decoy that makes Explorer skip the
/// Windows 11 modern context menu host and show the classic menu.
const WIN11_CLASSIC_KEY: &str =
    "Software\\Classes\\CLSID\\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\\InprocServer32";
/// Ownership marker for the rollback key, kept outside the decoy so the
/// shell never sees unexpected values in it.
const WIN11_CLASSIC_MARKER: &str = "Software\\Classes\\IdleTrigger\\Win11Classic";

/// Top-level verb of the rules cascade (submenu mode) and the
/// ExtendedSubCommandsKey target holding the rule children.
const RULES_VERB: &str = "IdleTriggerRules";
/// ECF_SEPARATORAFTER on a real submenu item renders a separating line
/// after it (empty separator children never rendered and broke submenu
/// building).
const ECF_SEPARATOR_AFTER: u32 = 0x40;

/// Class-level cascade containers (deliberately OUTSIDE shell\ — a
/// container under shell\ renders as a phantom verb with its key name).
const RULES_MENU_KEY: &str = "IdleTriggerRulesMenu";
const COPY_MENU_KEY: &str = "IdleTriggerCopyMenu";
const COPY_VERB: &str = "IdleTriggerCopy";
/// Nilesoft Shell ordering: full path | parent | name family, with
/// separator lines between the groups (ECF works inside cascades).
const COPY_SUB_FORMATS: [(&str, &str); 6] = [
    (core_ctx::COPY_FORMAT_FULL, "ctx_copy_full"),
    (core_ctx::COPY_FORMAT_TARGET, "ctx_copy_target"),
    (core_ctx::COPY_FORMAT_DIR, "ctx_copy_dir"),
    (core_ctx::COPY_FORMAT_NAME, "ctx_copy_name"),
    (core_ctx::COPY_FORMAT_STEM, "ctx_copy_stem"),
    (core_ctx::COPY_FORMAT_EXT, "ctx_copy_ext"),
];
/// Separator positions inside the cascade (indices into COPY_SUB_FORMATS):
/// after "full" and after "dir".
const COPY_GROUP_SEPS: [usize; 2] = [1, 2];

/// One registry key with its values: `None` name = the default value.
type ValueMap = BTreeMap<Option<String>, String>;

// ---- Raw registry helpers (HKCU-relative paths only) ---------------------

fn open_read(path: &str, root: HKEY, wow64: bool) -> Option<HKEY> {
    let mut hkey = HKEY::default();
    let access = if wow64 {
        KEY_READ | KEY_WOW64_64KEY
    } else {
        KEY_READ
    };
    let opened =
        unsafe { RegOpenKeyExW(root, PCWSTR(wide(path).as_ptr()), None, access, &mut hkey) };
    (opened == ERROR_SUCCESS).then_some(hkey)
}

fn read_sz(path: &str, name: Option<&str>) -> Option<String> {
    read_sz_at(HKEY_CURRENT_USER, path, name, false)
}

fn read_sz_at(root: HKEY, path: &str, name: Option<&str>, wow64: bool) -> Option<String> {
    let hkey = open_read(path, root, wow64)?;
    let result = (|| {
        let value_name = wide(name.unwrap_or(""));
        let name_ptr = if name.is_some() {
            PCWSTR(value_name.as_ptr())
        } else {
            PCWSTR::null()
        };
        let mut kind = windows::Win32::System::Registry::REG_VALUE_TYPE::default();
        let mut size = 0u32;
        let status = unsafe {
            RegQueryValueExW(hkey, name_ptr, None, Some(&mut kind), None, Some(&mut size))
        };
        if status != ERROR_SUCCESS || size > 1 << 20 || !size.is_multiple_of(2) {
            return None;
        }
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        let status = unsafe {
            RegQueryValueExW(
                hkey,
                name_ptr,
                None,
                None,
                Some(data.as_mut_ptr()),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let text: Vec<u16> = data[..size as usize]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|ch| *ch != 0)
            .collect();
        Some(String::from_utf16_lossy(&text))
    })();
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    result
}

fn subkeys(path: &str) -> Vec<String> {
    subkeys_at(HKEY_CURRENT_USER, path, false)
}

fn subkeys_at(root: HKEY, path: &str, wow64: bool) -> Vec<String> {
    let Some(hkey) = open_read(path, root, wow64) else {
        return Vec::new();
    };
    let result = (|| {
        let mut count = 0u32;
        let status = unsafe {
            RegQueryInfoKeyW(
                hkey,
                None,
                None,
                None,
                Some(&mut count),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
        };
        if status != ERROR_SUCCESS {
            return Vec::new();
        }
        let mut names = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut buffer = [0u16; 256];
            let mut len = buffer.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(
                    hkey,
                    index,
                    Some(windows::core::PWSTR(buffer.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if status == ERROR_SUCCESS {
                names.push(String::from_utf16_lossy(&buffer[..len as usize]));
            }
        }
        names
    })();
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    result
}

fn write_dword(path: &str, name: Option<&str>, value: u32) -> bool {
    let mut hkey = HKEY::default();
    let created = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(path).as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        )
    };
    if created != ERROR_SUCCESS {
        return false;
    }
    let value_name = wide(name.unwrap_or(""));
    let name_ptr = if name.is_some() {
        PCWSTR(value_name.as_ptr())
    } else {
        PCWSTR::null()
    };
    let ok = unsafe {
        windows::Win32::System::Registry::RegSetValueExW(
            hkey,
            name_ptr,
            None,
            windows::Win32::System::Registry::REG_DWORD,
            Some(&value.to_le_bytes()),
        )
    } == ERROR_SUCCESS;
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    ok
}

fn delete_value(path: &str, name: &str) -> bool {
    let mut hkey = HKEY::default();
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(path).as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut hkey,
        )
    };
    if opened != ERROR_SUCCESS {
        return false;
    }
    // An empty name addresses the key's default value.
    let value_name = wide(name);
    let ptr = if name.is_empty() {
        PCWSTR::null()
    } else {
        PCWSTR(value_name.as_ptr())
    };
    let ok = unsafe { RegDeleteValueW(hkey, ptr) } == ERROR_SUCCESS;
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    ok
}

fn write_sz(path: &str, name: Option<&str>, value: &str) -> bool {
    let mut hkey = HKEY::default();
    let created = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(path).as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        )
    };
    if created != ERROR_SUCCESS {
        return false;
    }
    let value_name = wide(name.unwrap_or(""));
    let name_ptr = if name.is_some() {
        PCWSTR(value_name.as_ptr())
    } else {
        PCWSTR::null()
    };
    let mut data: Vec<u8> = value
        .encode_utf16()
        .chain([0])
        .flat_map(|ch| ch.to_le_bytes())
        .collect();
    data.truncate(1 << 20);
    let ok = unsafe { RegSetValueExW(hkey, name_ptr, None, REG_SZ, Some(&data)) } == ERROR_SUCCESS;
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    ok
}

fn delete_tree(path: &str) -> bool {
    let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(wide(path).as_ptr())) };
    status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND
}

fn key_exists(path: &str) -> bool {
    open_read(path, HKEY_CURRENT_USER, false).is_some()
}

// ---- Desired-key computation ---------------------------------------------

/// Icon reference for a rule: explicit override, the target program's own
/// icon for exe/dll/ico targets, the console icon for scripts, our app icon
/// otherwise.
fn rule_icon(rule: &core_ctx::CtxRule, app_icon: &str) -> String {
    if !rule.icon.is_empty() {
        return rule.icon.clone();
    }
    let target = rule.command.to_ascii_lowercase();
    if target.ends_with(".exe") || target.ends_with(".dll") || target.ends_with(".ico") {
        return format!("{},0", rule.command);
    }
    if (target.ends_with(".bat") || target.ends_with(".cmd"))
        && let Ok(root) = std::env::var("SystemRoot")
    {
        return format!("{root}\\System32\\cmd.exe,0");
    }
    app_icon.to_string()
}

/// Separator verb paths from the last desired_entries() pass: their
/// CommandFlags dword rides outside the string-only ValueMap.
static SEPARATOR_FLAGS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Builds the complete desired key/value map. Empty = unregister everything.
fn desired_entries() -> BTreeMap<String, ValueMap> {
    let mut desired: BTreeMap<String, ValueMap> = BTreeMap::new();
    // Separator children whose CommandFlags dword the write pass adds.
    let mut separator_flags: Vec<String> = Vec::new();
    let (enabled, copy_enabled) =
        crate::cfg_map(|config| (config.ctx_menu_enabled, config.ctx_copy_enabled));
    if !enabled {
        return desired;
    }
    let exe = std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    if exe.is_empty() {
        return desired;
    }
    let app_icon = format!("{exe},-1");
    let base = format!("{CLASSES}\\");

    if copy_enabled {
        let title = crate::t("ctx_menu_copy_title");
        for root in COPY_ROOTS {
            let placeholder = if root == "Directory\\Background" {
                "%V"
            } else {
                "%L"
            };
            let shell = format!("{base}{root}\\shell");
            let mut values = ValueMap::new();
            values.insert(None, title.clone());
            values.insert(Some("Icon".into()), app_icon.clone());
            values.insert(
                Some("ExtendedSubCommandsKey".into()),
                format!("{root}\\{COPY_MENU_KEY}"),
            );
            desired.insert(format!("{shell}\\{COPY_VERB}"), values);
            // Empirically settled layout: the VALUE form pointing at a
            // CLASS-LEVEL container (outside shell\) renders the
            // cascade AND never enumerates the container as a phantom
            // verb (a shell\ sibling did; a nested subkey never
            // rendered at all).
            let menu = format!("{base}{root}\\{COPY_MENU_KEY}\\shell");
            for (index, (format, key)) in COPY_SUB_FORMATS.into_iter().enumerate() {
                let sub = format!("{menu}\\{key}");
                let mut values = ValueMap::new();
                values.insert(None, crate::t(key));
                values.insert(Some("Icon".into()), app_icon.clone());
                // "Target path" only makes sense on shortcuts; AppliesTo
                // filters it to .lnk selections so regular files never see
                // it.
                if format == core_ctx::COPY_FORMAT_TARGET {
                    values.insert(
                        Some("AppliesTo".into()),
                        core_ctx::applies_to_aqs(&[".lnk".into()]).unwrap_or_default(),
                    );
                }
                desired.insert(sub.clone(), values);
                desired.insert(
                    format!("{sub}\\command"),
                    ValueMap::from([(
                        None,
                        format!("\"{exe}\" ctx copy {format} \"{placeholder}\""),
                    )]),
                );
                if COPY_GROUP_SEPS.contains(&index) {
                    // Group-end separator: ECF_SEPARATORAFTER rides ON this
                    // item; empty separator children never rendered and
                    // broke submenu building.
                    separator_flags.push(sub.clone());
                }
            }
        }
    }

    let rules: Vec<_> = crate::runtime::lock(&crate::ctxexec::CTX_RULES)
        .iter()
        .filter(|rule| rule.enabled)
        .cloned()
        .collect();
    let submenu = crate::cfg_map(|c| c.ctx_rules_submenu) || rules.iter().any(|r| r.separator);
    if submenu {
        // One cascade per cover root; per-rule scope shaping stays a flat
        // mode feature (invoke-time pattern matching still applies).
        for root in COPY_ROOTS {
            let shell = format!("{base}{root}\\shell");
            let verb = format!("{shell}\\{RULES_VERB}");
            desired.insert(
                verb.clone(),
                ValueMap::from([
                    (None, crate::t("ctx_rules_menu_title")),
                    (Some("Icon".into()), app_icon.clone()),
                    (
                        Some("ExtendedSubCommandsKey".into()),
                        format!("{root}\\{RULES_MENU_KEY}"),
                    ),
                ]),
            );
            // Same nested-cascade fix as the copy menu: a sibling
            // container under shell\\ rendered as a phantom verb.
            let menu = format!("{base}{root}\\{RULES_MENU_KEY}\\shell");
            let mut last_child = String::new();
            for rule in &rules {
                let child = format!("{menu}\\IdleTriggerRule{}", rule.id);
                if rule.separator {
                    // Flag the PRECEDING item with ECF_SEPARATORAFTER;
                    // a leading separator has nothing to attach to.
                    if !last_child.is_empty() {
                        separator_flags.push(last_child.clone());
                    }
                    continue;
                }
                let mut values = ValueMap::new();
                values.insert(None, rule.title.clone());
                values.insert(Some("Icon".into()), rule_icon(rule, &app_icon));
                desired.insert(child.clone(), values);
                last_child = child.clone();
                desired.insert(
                    format!("{child}\\command"),
                    ValueMap::from([(None, format!("\"{exe}\" ctx rule {} \"%L\"", rule.id))]),
                );
            }
        }
        // CommandFlags must be REG_DWORD; the string-only ValueMap cannot
        // carry it, so separator children get the flag in the write pass.
        SEPARATOR_FLAGS.lock().unwrap().clear();
        SEPARATOR_FLAGS
            .lock()
            .unwrap()
            .extend(separator_flags.iter().cloned());
        return desired;
    }
    for rule in &rules {
        let aqs = core_ctx::applies_to_aqs(&rule.patterns);
        for root in core_ctx::scope_roots(&rule.scope) {
            let placeholder = if *root == "Directory\\Background" {
                "%V"
            } else {
                "%L"
            };
            let verb = format!("{base}{root}\\shell\\IdleTriggerRule{}", rule.id);
            let mut values = ValueMap::new();
            values.insert(None, rule.title.clone());
            values.insert(Some("Icon".into()), rule_icon(rule, &app_icon));
            if let Some(filter) = &aqs {
                values.insert(Some("AppliesTo".into()), filter.clone());
            }
            desired.insert(verb.clone(), values);
            desired.insert(
                format!("{verb}\\command"),
                ValueMap::from([(
                    None,
                    format!("\"{exe}\" ctx rule {} \"{placeholder}\"", rule.id),
                )]),
            );
        }
    }
    SEPARATOR_FLAGS.lock().unwrap().clear();
    SEPARATOR_FLAGS
        .lock()
        .unwrap()
        .extend(separator_flags.iter().cloned());
    desired
}

/// Keys under our verb prefix across every root we ever wrote to.
fn owned_keys() -> Vec<String> {
    let mut owned = Vec::new();
    for root in ALL_ROOTS {
        let shell = format!("{CLASSES}\\{root}\\shell");
        for name in subkeys(&shell) {
            if name.starts_with(core_ctx::VERB_PREFIX) {
                owned.push(format!("{shell}\\{name}"));
            }
        }
        // Class-level cascade containers ride OUTSIDE shell\; without this
        // scan their stale children (e.g. retired separator keys) survive
        // every sync.
        let class = format!("{CLASSES}\\{root}");
        for name in subkeys(&class) {
            if name.starts_with(core_ctx::VERB_PREFIX) {
                owned.push(format!("{class}\\{name}"));
            }
        }
    }
    owned
}

/// Reads the values of one owned key (default + the names we write).
fn current_values(path: &str) -> ValueMap {
    let mut values = ValueMap::new();
    if let Some(default) = read_sz(path, None) {
        values.insert(None, default);
    }
    for name in ["Icon", "AppliesTo", "ExtendedSubCommandsKey", "MUIVerb"] {
        if let Some(value) = read_sz(path, Some(name)) {
            values.insert(Some(name.to_string()), value);
        }
    }
    values
}

/// Diff-applies the desired state. Returns (verbs, rules) registered after
/// the pass — the registry's answer, not the config's claim.
pub fn sync(reason: &str) -> (bool, usize) {
    let desired = desired_entries();
    if SEPARATOR_FLAGS.lock().unwrap().is_empty() {
        // Flat mode: nothing carries the dword; also drop the marks so a
        // later submenu pass rewrites flags cleanly.
    }
    for owned in owned_keys() {
        if !desired.contains_key(&owned) {
            let _ = delete_tree(&owned);
        }
    }
    let separator_flags = SEPARATOR_FLAGS.lock().unwrap().clone();
    for (path, values) in &desired {
        let current = current_values(path);
        if current != *values {
            for (name, value) in values {
                if !write_sz(path, name.as_deref(), value) {
                    crate::log_line(&format!("ctx sync: write failed at {path}"));
                }
            }
            // A managed value that disappeared from the desired set must be
            // DELETED, not left behind — the old ExtendedSubCommandsKey
            // value outlived its sibling container and kept pointing there.
            // The default value lives under the None key in the map; a
            // cascade verb requires it GONE once MUIVerb carries the title.
            let mut stale: Vec<Option<String>> = Vec::new();
            for name in ["Icon", "AppliesTo", "ExtendedSubCommandsKey", "MUIVerb"] {
                let key = Some(name.to_string());
                if current.contains_key(&key) && !values.contains_key(&key) {
                    stale.push(key);
                }
            }
            if current.contains_key(&None) && !values.contains_key(&None) {
                stale.push(None);
            }
            for key in stale {
                let _ = delete_value(path, key.as_deref().unwrap_or(""));
            }
        }
        if separator_flags.iter().any(|mark| mark == path)
            && !write_dword(path, Some("CommandFlags"), ECF_SEPARATOR_AFTER)
        {
            crate::log_line(&format!("ctx sync: separator flag failed at {path}"));
        }
    }
    let status = registration_status();
    crate::log_line(&format!(
        "ctx sync ({reason}): copy={} rules={}",
        status.0, status.1
    ));
    status
}

/// What is actually registered right now, from the registry's point of view.
pub fn registration_status() -> (bool, usize) {
    let copy = ALL_ROOTS
        .iter()
        .any(|root| key_exists(&format!("{CLASSES}\\{root}\\shell\\{COPY_VERB}")));
    let mut rule_ids: Vec<String> = Vec::new();
    for root in ALL_ROOTS {
        for name in subkeys(&format!("{CLASSES}\\{root}\\shell")) {
            if let Some(id) = name.strip_prefix("IdleTriggerRule")
                && !rule_ids.contains(&id.to_string())
            {
                rule_ids.push(id.to_string());
            }
        }
    }
    (copy, rule_ids.len())
}

// ---- Windows 11 classic-menu rollback ------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicState {
    /// Modern menu active (system default).
    Off,
    /// Rollback key present and written by IdleTrigger.
    ByUs,
    /// Rollback key present from another tool; we never fight over it.
    ByOther,
}

pub fn classic_state() -> ClassicState {
    let present = key_exists(WIN11_CLASSIC_KEY);
    if !present {
        return ClassicState::Off;
    }
    if read_sz(WIN11_CLASSIC_MARKER, Some("Value")).is_some() {
        ClassicState::ByUs
    } else {
        ClassicState::ByOther
    }
}

/// Applies the rollback without restarting Explorer; the caller owns the
/// disruptive restart (and its confirmation).
pub fn set_classic(on: bool) -> Result<(), String> {
    if on {
        if matches!(classic_state(), ClassicState::ByOther) {
            // Another tool already rolled back; just note our intent.
            let _ = write_sz(WIN11_CLASSIC_MARKER, Some("Value"), "1");
            return Ok(());
        }
        if !write_sz(WIN11_CLASSIC_KEY, None, "") {
            return Err("could not write the classic-menu key".into());
        }
        let _ = write_sz(WIN11_CLASSIC_MARKER, Some("Value"), "1");
        Ok(())
    } else {
        if !matches!(classic_state(), ClassicState::ByUs) {
            return Ok(());
        }
        if !delete_tree(WIN11_CLASSIC_KEY) {
            return Err("could not remove the classic-menu key".into());
        }
        let _ = delete_tree(WIN11_CLASSIC_MARKER);
        Ok(())
    }
}

/// Restart the shell so the rollback takes effect. Explorer windows close
/// and the taskbar reloads — callers must have warned the user.
pub fn restart_explorer() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new(
        std::env::var("SystemRoot").unwrap_or_default() + "\\System32\\taskkill.exe",
    )
    .args(["/f", "/im", "explorer.exe"])
    .creation_flags(CREATE_NO_WINDOW)
    .status();
    std::thread::sleep(std::time::Duration::from_millis(400));
    let _ = std::process::Command::new(
        std::env::var("SystemRoot").unwrap_or_default() + "\\explorer.exe",
    )
    .spawn();
}

// ---- Coexistence warnings -------------------------------------------------

/// Third-party menu managers that may hide or duplicate our entries. Only
/// known managers (which rebuild the whole menu) are named: ordinary
/// extension DLLs (TortoiseGit, cloud drives, ...) coexist fine and flagging
/// them would be noise.
pub fn detect_conflicts() -> Vec<String> {
    let mut hits: Vec<String> = Vec::new();
    let roots = [
        "*\\shellex\\ContextMenuHandlers",
        "Directory\\shellex\\ContextMenuHandlers",
        "Folder\\shellex\\ContextMenuHandlers",
        "Directory\\Background\\shellex\\ContextMenuHandlers",
        "AllFilesystemObjects\\shellex\\ContextMenuHandlers",
    ];
    for root in roots {
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            let wow64 = hive == HKEY_LOCAL_MACHINE;
            let path = format!("{CLASSES}\\{root}");
            for handler in subkeys_at(hive, &path, wow64) {
                let clsid = read_sz_at(hive, &format!("{path}\\{handler}"), None, wow64)
                    .unwrap_or_default();
                // The CLSID key path keeps the braces: stripping them makes
                // every probe miss (the first draft never found anything).
                if !clsid.starts_with('{') || !clsid.ends_with('}') {
                    continue;
                }
                let dll = [
                    format!("CLSID\\{clsid}\\InprocServer32"),
                    format!("Wow6432Node\\CLSID\\{clsid}\\InprocServer32"),
                ]
                .iter()
                .find_map(|probe| read_sz_at(hive, &format!("{CLASSES}\\{probe}"), None, wow64))
                .unwrap_or_default()
                .to_ascii_lowercase();
                if dll.is_empty() {
                    continue;
                }
                let label = if dll.contains("nilesoft") || dll.ends_with("\\shell.dll") {
                    "nilesoft"
                } else if dll.contains("openxx") {
                    "openxx"
                } else {
                    continue;
                };
                if !hits.iter().any(|hit| hit == label) {
                    hits.push(label.to_string());
                }
            }
        }
    }
    hits
}
