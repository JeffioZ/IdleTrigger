//! Registry sync engine for the Explorer context-menu integration.
//!
//! Owned keys are prefixed verbs/cascades plus the six fixed copy-handler
//! CLSIDs under HKCU\Software\Classes. Sync computes the desired map from
//! live configuration, deletes stale owned keys, and rewrites changed ones.
//! Nothing here ever touches keys owned by other software.

use idletrigger_core::ctx_menu as core_ctx;
use std::collections::BTreeMap;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY,
    KEY_WRITE, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW, RegOpenKeyExW, RegQueryInfoKeyW,
    RegQueryValueExW, RegSetValueExW,
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
#[derive(Clone, Debug, PartialEq, Eq)]
enum RegistryValue {
    Text(String),
    Dword(u32),
}
type ValueMap = BTreeMap<Option<String>, RegistryValue>;
fn text_value(values: &mut ValueMap, name: Option<&str>, value: impl Into<String>) {
    values.insert(name.map(str::to_string), RegistryValue::Text(value.into()));
}

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
    read_sz_at(HKEY_CURRENT_USER, path, name, true)
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
    subkeys_at(HKEY_CURRENT_USER, path, true)
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
            KEY_SET_VALUE | KEY_WOW64_64KEY,
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
            KEY_SET_VALUE | KEY_WOW64_64KEY,
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
            KEY_SET_VALUE | KEY_WOW64_64KEY,
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
    let Some((parent, name)) = path.rsplit_once('\\') else {
        return false;
    };
    let mut key = HKEY::default();
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(parent).as_ptr()),
            None,
            KEY_WRITE
                | KEY_READ
                | KEY_WOW64_64KEY
                | windows::Win32::System::Registry::REG_SAM_FLAGS(0x00010000),
            &mut key,
        )
    };
    if opened == ERROR_FILE_NOT_FOUND {
        return true;
    }
    if opened != ERROR_SUCCESS {
        return false;
    }
    let status = unsafe { RegDeleteTreeW(key, PCWSTR(wide(name).as_ptr())) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND
}

fn key_exists(path: &str) -> bool {
    let Some(key) = open_read(path, HKEY_CURRENT_USER, true) else {
        return false;
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    true
}

fn read_dword(path: &str, name: &str) -> Option<u32> {
    let key = open_read(path, HKEY_CURRENT_USER, true)?;
    let mut kind = windows::Win32::System::Registry::REG_VALUE_TYPE::default();
    let mut value = 0u32;
    let mut size = 4u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(wide(name).as_ptr()),
            None,
            Some(&mut kind),
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    (status == ERROR_SUCCESS && size == 4 && kind == windows::Win32::System::Registry::REG_DWORD)
        .then_some(value)
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

/// Builds the complete desired tree, including its empty container keys.
fn desired_entries() -> Result<BTreeMap<String, ValueMap>, String> {
    if crate::runtime::CTX_SYNC_BLOCKED.load(std::sync::atomic::Ordering::SeqCst)
        || crate::runtime::CONFIG_LOAD_FAILED.load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err(crate::t_pub("ctx_config_blocked"));
    }
    let (enabled, copy, submenu) = crate::runtime::lock(&crate::CONFIG)
        .as_ref()
        .map(|c| (c.ctx_menu_enabled, c.ctx_copy_enabled, c.ctx_rules_submenu))
        .ok_or_else(|| crate::t_pub("ctx_config_blocked"))?;
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    // A valid explicit off state must still remove registrations even if
    // unused rules are malformed. Never infer off from startup defaults.
    let rules = if enabled {
        let doc = crate::runtime::lock(&crate::CONFIG_DOC);
        let doc = doc
            .as_ref()
            .ok_or_else(|| crate::t_pub("ctx_config_blocked"))?;
        core_ctx::read_rules(doc)?.0
    } else {
        Vec::new()
    };
    if exe.is_empty() {
        return Err(crate::t_pub("ctx_config_blocked"));
    }
    Ok(build_entries(enabled, copy, submenu, &exe, &rules))
}

fn build_entries(
    enabled: bool,
    copy: bool,
    submenu: bool,
    exe: &str,
    rules: &[core_ctx::CtxRule],
) -> BTreeMap<String, ValueMap> {
    let mut desired = BTreeMap::new();
    if !enabled || exe.is_empty() {
        return desired;
    }
    let app_icon = format!("{exe},-1");
    if copy {
        for index in 0..crate::ctxselection::FORMATS.len() {
            let class = format!(
                "{CLASSES}\\CLSID\\{}",
                crate::ctxselection::class_name(index)
            );
            let mut values = ValueMap::new();
            text_value(&mut values, None, "IdleTrigger copy selection");
            desired.insert(class.clone(), values);
            let mut server = ValueMap::new();
            text_value(&mut server, None, format!("\"{exe}\" ctx-selection"));
            text_value(&mut server, Some("ServerExecutable"), exe);
            desired.insert(format!("{class}\\LocalServer32"), server);
        }
        for root in COPY_ROOTS {
            let placeholder = if root == "Directory\\Background" {
                "%V"
            } else {
                "%L"
            };
            let container = format!("{CLASSES}\\{root}\\{COPY_MENU_KEY}");
            let mut values = ValueMap::new();
            text_value(&mut values, None, crate::t("ctx_menu_copy_title"));
            text_value(&mut values, Some("Icon"), &app_icon);
            text_value(&mut values, Some("MultiSelectModel"), "Player");
            text_value(
                &mut values,
                Some("ExtendedSubCommandsKey"),
                format!("{root}\\{COPY_MENU_KEY}"),
            );
            desired.insert(format!("{CLASSES}\\{root}\\shell\\{COPY_VERB}"), values);
            desired.insert(container.clone(), ValueMap::new());
            desired.insert(format!("{container}\\shell"), ValueMap::new());
            for (index, (format, key)) in COPY_SUB_FORMATS.iter().enumerate() {
                if *format == core_ctx::COPY_FORMAT_TARGET && root != "AllFilesystemObjects" {
                    continue;
                }
                let child = format!("{container}\\shell\\{index:03}_{key}");
                let mut values = ValueMap::new();
                text_value(&mut values, None, crate::t(key));
                text_value(&mut values, Some("Icon"), &app_icon);
                text_value(&mut values, Some("MultiSelectModel"), "Player");
                if *format == core_ctx::COPY_FORMAT_TARGET {
                    text_value(
                        &mut values,
                        Some("AppliesTo"),
                        core_ctx::applies_to_aqs(&[".lnk".into()]).unwrap(),
                    );
                }
                if COPY_GROUP_SEPS.contains(&index) {
                    values.insert(
                        Some("CommandFlags".into()),
                        RegistryValue::Dword(ECF_SEPARATOR_AFTER),
                    );
                }
                desired.insert(child.clone(), values);
                let mut command = ValueMap::new();
                text_value(
                    &mut command,
                    None,
                    // The invalid-path sentinel prevents a drive root's
                    // trailing slash from escaping the argument's quote.
                    format!("\"{exe}\" ctx copy {format} \"{placeholder}|\""),
                );
                // Background verbs retain %V; file/drive verbs receive the
                // complete IShellItemArray in one out-of-process COM call.
                if root != "Directory\\Background" {
                    let class_index = crate::ctxselection::FORMATS
                        .iter()
                        .position(|candidate| candidate == format)
                        .expect("known copy format");
                    text_value(
                        &mut command,
                        Some("DelegateExecute"),
                        crate::ctxselection::class_name(class_index),
                    );
                }
                desired.insert(format!("{child}\\command"), command);
            }
        }
    }
    let rules: Vec<_> = rules
        .iter()
        .take(core_ctx::MAX_CTX_RULES)
        .filter(|r| r.enabled)
        .collect();
    let submenu = submenu || rules.iter().any(|r| r.separator);
    for root in ["*", "Directory", "Directory\\Background"] {
        let applicable: Vec<_> = rules
            .iter()
            .enumerate()
            .filter(|(_, r)| r.separator || core_ctx::scope_roots(&r.scope).contains(&root))
            .collect();
        if !applicable.iter().any(|(_, r)| !r.separator) {
            continue;
        }
        let menu = if submenu {
            let container = format!("{CLASSES}\\{root}\\{RULES_MENU_KEY}");
            let mut values = ValueMap::new();
            text_value(&mut values, None, crate::t("ctx_rules_menu_title"));
            text_value(&mut values, Some("Icon"), &app_icon);
            text_value(
                &mut values,
                Some("ExtendedSubCommandsKey"),
                format!("{root}\\{RULES_MENU_KEY}"),
            );
            desired.insert(format!("{CLASSES}\\{root}\\shell\\{RULES_VERB}"), values);
            desired.insert(container.clone(), ValueMap::new());
            desired.insert(format!("{container}\\shell"), ValueMap::new());
            format!("{container}\\shell")
        } else {
            format!("{CLASSES}\\{root}\\shell")
        };
        let mut previous: Option<String> = None;
        let mut separator = false;
        for (index, rule) in applicable {
            if rule.separator {
                separator = true;
                continue;
            }
            if submenu
                && separator
                && let Some(previous) = &previous
            {
                desired.get_mut(previous).unwrap().insert(
                    Some("CommandFlags".into()),
                    RegistryValue::Dword(ECF_SEPARATOR_AFTER),
                );
            }
            separator = false;
            let child = format!("{menu}\\IdleTriggerRule{index:03}_{}", rule.id);
            let mut values = ValueMap::new();
            text_value(&mut values, None, &rule.title);
            text_value(&mut values, Some("Icon"), rule_icon(rule, &app_icon));
            text_value(&mut values, Some("MultiSelectModel"), "Document");
            if let Some(aqs) = core_ctx::applies_to_aqs(&rule.patterns) {
                text_value(&mut values, Some("AppliesTo"), aqs);
            }
            desired.insert(child.clone(), values);
            let placeholder = if root == "Directory\\Background" {
                "%V"
            } else {
                "%L"
            };
            let mut command = ValueMap::new();
            text_value(
                &mut command,
                None,
                format!("\"{exe}\" ctx rule {} \"{placeholder}|\"", rule.id),
            );
            desired.insert(format!("{child}\\command"), command);
            previous = Some(child);
        }
    }
    desired
}

fn collect_tree(path: String, keys: &mut Vec<String>) {
    for child in subkeys(&path) {
        collect_tree(format!("{path}\\{child}"), keys);
    }
    keys.push(path);
}

/// Enumerate only our verbs, cascade containers and fixed COM classes.
fn owned_keys() -> Vec<String> {
    let mut owned = Vec::new();
    for index in 0..crate::ctxselection::FORMATS.len() {
        let path = format!(
            "{CLASSES}\\CLSID\\{}",
            crate::ctxselection::class_name(index)
        );
        if key_exists(&path) {
            collect_tree(path, &mut owned);
        }
    }
    for root in ALL_ROOTS {
        let shell = format!("{CLASSES}\\{root}\\shell");
        for name in subkeys(&shell) {
            if name
                .to_ascii_lowercase()
                .starts_with(&core_ctx::VERB_PREFIX.to_ascii_lowercase())
            {
                collect_tree(format!("{shell}\\{name}"), &mut owned);
            }
        }
        for name in [COPY_MENU_KEY, RULES_MENU_KEY] {
            let path = format!("{CLASSES}\\{root}\\{name}");
            if key_exists(&path) {
                collect_tree(path, &mut owned);
            }
        }
    }
    owned
}

fn current_values(path: &str) -> ValueMap {
    let mut values = ValueMap::new();
    for name in [
        None,
        Some("Icon"),
        Some("AppliesTo"),
        Some("ExtendedSubCommandsKey"),
        Some("MUIVerb"),
        Some("MultiSelectModel"),
        Some("DelegateExecute"),
        Some("ServerExecutable"),
    ] {
        if let Some(value) = read_sz(path, name) {
            text_value(&mut values, name, value);
        }
    }
    if let Some(value) = read_dword(path, "CommandFlags") {
        values.insert(Some("CommandFlags".into()), RegistryValue::Dword(value));
    }
    values
}

static SYNC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn contains_registry_path(desired: &BTreeMap<String, ValueMap>, path: &str) -> bool {
    desired.keys().any(|key| key.eq_ignore_ascii_case(path))
}

pub fn sync(reason: &str) -> (bool, usize) {
    let _writer = crate::runtime::lock(&crate::CONFIG_WRITER);
    let _sync = crate::runtime::lock(&SYNC_LOCK);
    let desired = match desired_entries() {
        Ok(desired) => desired,
        Err(err) => {
            crate::log_line(&format!("ctx sync ({reason}) blocked: {err}"));
            return registration_status();
        }
    };
    let mut failed = false;
    let mut changed = false;
    // Publish children/commands before removing stale keys. A failed write
    // must not remove the previous working menu.
    let is_class = |path: &String| path.starts_with(&format!("{CLASSES}\\CLSID\\"));
    // Make COM activation available before publishing DelegateExecute verbs.
    let entries = desired
        .iter()
        .rev()
        .filter(|(path, _)| is_class(path))
        .chain(desired.iter().rev().filter(|(path, _)| !is_class(path)));
    for (path, values) in entries {
        if failed && !is_class(path) {
            break;
        }
        let current = current_values(path);
        for (name, value) in values {
            if current.get(name) == Some(value) {
                continue;
            }
            let written = match value {
                RegistryValue::Text(value) => write_sz(path, name.as_deref(), value),
                RegistryValue::Dword(value) => write_dword(path, name.as_deref(), *value),
            };
            failed |= !written;
            changed |= written;
        }
        for name in current.keys().filter(|name| !values.contains_key(*name)) {
            let removed = delete_value(path, name.as_deref().unwrap_or(""));
            failed |= !removed;
            changed |= removed;
        }
    }
    if !failed {
        for path in owned_keys() {
            if !contains_registry_path(&desired, &path) {
                let removed = delete_tree(&path);
                failed |= !removed;
                changed |= removed;
            }
        }
    }
    if changed {
        unsafe {
            windows::Win32::UI::Shell::SHChangeNotify(
                windows::Win32::UI::Shell::SHCNE_ASSOCCHANGED,
                windows::Win32::UI::Shell::SHCNF_IDLIST,
                None,
                None,
            );
        }
    }
    crate::log_line(&format!("ctx sync ({reason}): failed={failed}"));
    registration_status()
}

pub fn registration_healthy() -> bool {
    let _writer = crate::runtime::lock(&crate::CONFIG_WRITER);
    let _sync = crate::runtime::lock(&SYNC_LOCK);
    let Ok(desired) = desired_entries() else {
        return false;
    };
    desired
        .iter()
        .all(|(path, values)| key_exists(path) && current_values(path) == *values)
        && owned_keys()
            .iter()
            .all(|path| contains_registry_path(&desired, path))
}

pub fn registration_blocked() -> bool {
    let _writer = crate::runtime::lock(&crate::CONFIG_WRITER);
    desired_entries().is_err()
}

/// Count commands, including cascades; a container is never a rule.
pub fn registration_status() -> (bool, usize) {
    let mut copy = false;
    let mut ids = std::collections::BTreeSet::new();
    for path in owned_keys() {
        if !path.ends_with("\\command") {
            continue;
        }
        let Some(command) = read_sz(&path, None) else {
            continue;
        };
        if command.contains(" ctx copy ") {
            copy = true;
        }
        if let Some((_, tail)) = command.split_once(" ctx rule ")
            && let Some(id) = tail.split_whitespace().next()
        {
            ids.insert(id.to_ascii_lowercase());
        }
    }
    (copy, ids.len())
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
    Legacy,
    Unavailable,
}

// A timestamp is a conservative change detector, not proof of ownership.
// Only the expected empty default value, with no children, is removable.
fn classic_stamp() -> Result<Option<String>, String> {
    let mut key = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(WIN11_CLASSIC_KEY).as_ptr()),
            None,
            KEY_READ | KEY_WOW64_64KEY,
            &mut key,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(crate::t_pub("ctx_registry_failed"));
    }
    let mut children = 0;
    let mut values = 0;
    let mut stamp = windows::Win32::Foundation::FILETIME::default();
    let queried = unsafe {
        RegQueryInfoKeyW(
            key,
            None,
            None,
            None,
            Some(&mut children),
            None,
            None,
            Some(&mut values),
            None,
            None,
            None,
            Some(&mut stamp),
        )
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if queried != ERROR_SUCCESS {
        return Err(crate::t_pub("ctx_registry_failed"));
    }
    if children != 0 || values != 1 || read_sz(WIN11_CLASSIC_KEY, None).as_deref() != Some("") {
        return Ok(Some("foreign".into()));
    }
    Ok(Some(format!(
        "{}:{}",
        stamp.dwHighDateTime, stamp.dwLowDateTime
    )))
}

pub fn classic_state() -> ClassicState {
    let stamp = match classic_stamp() {
        Ok(None) => return ClassicState::Off,
        Err(_) => return ClassicState::Unavailable,
        Ok(Some(stamp)) if stamp == "foreign" => return ClassicState::ByOther,
        Ok(Some(stamp)) => stamp,
    };
    classify_classic_marker(
        &stamp,
        read_sz(WIN11_CLASSIC_MARKER, Some("Value")).as_deref(),
    )
}

fn classify_classic_marker(stamp: &str, marker: Option<&str>) -> ClassicState {
    match marker {
        Some("1") => ClassicState::Legacy,
        Some(value) if value == format!("v2:{stamp}") => ClassicState::ByUs,
        _ => ClassicState::ByOther,
    }
}

fn delete_classic_key() -> bool {
    let status = unsafe {
        windows::Win32::System::Registry::RegDeleteKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(WIN11_CLASSIC_KEY).as_ptr()),
            KEY_WOW64_64KEY.0,
            None,
        )
    };
    status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND
}

/// Applies the rollback without restarting Explorer; the caller owns the
/// disruptive restart (and its confirmation).
pub fn set_classic(on: bool) -> Result<(), String> {
    if on {
        if classic_state() != ClassicState::Off {
            return Err(crate::t_pub("ctx_win11_other_tool"));
        }
        // Retire stale ownership before creating a new target. If this fails,
        // leave Windows untouched rather than inheriting an old marker.
        if !delete_tree(WIN11_CLASSIC_MARKER) {
            return Err(crate::t_pub("ctx_registry_failed"));
        }
        // Disposition closes the read-then-create race with another tool.
        let mut key = HKEY::default();
        let mut disposition =
            windows::Win32::System::Registry::REG_CREATE_KEY_DISPOSITION::default();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(wide(WIN11_CLASSIC_KEY).as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE | KEY_WOW64_64KEY,
                None,
                &mut key,
                Some(&mut disposition),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(crate::t_pub("ctx_registry_failed"));
        }
        unsafe {
            let _ = RegCloseKey(key);
        }
        if disposition != windows::Win32::System::Registry::REG_CREATED_NEW_KEY {
            return Err(crate::t_pub("ctx_win11_other_tool"));
        }
        if !write_sz(WIN11_CLASSIC_KEY, None, "") {
            return Err(crate::t_pub("ctx_registry_failed"));
        }
        let stamp = classic_stamp()?.ok_or_else(|| crate::t_pub("ctx_registry_failed"))?;
        if stamp == "foreign"
            || !write_sz(WIN11_CLASSIC_MARKER, Some("Value"), &format!("v2:{stamp}"))
        {
            // Roll back only the unchanged key we just populated, never a
            // recursive tree. If ownership is uncertain, explain partial apply.
            if stamp != "foreign"
                && classic_stamp().ok().flatten().as_deref() == Some(&stamp)
                && delete_classic_key()
            {
                return Err(crate::t_pub("ctx_registry_failed"));
            }
            return Err(crate::t_pub("ctx_classic_partial"));
        }
        Ok(())
    } else {
        if !matches!(classic_state(), ClassicState::ByUs | ClassicState::Legacy) {
            return Err(crate::t_pub("ctx_win11_other_tool"));
        }
        // Upgrade a legacy marker before removing the target. Even if marker
        // cleanup later fails, it cannot claim an unrelated recreated key.
        let stamp = classic_stamp()?.ok_or_else(|| crate::t_pub("ctx_registry_failed"))?;
        if stamp == "foreign"
            || !write_sz(WIN11_CLASSIC_MARKER, Some("Value"), &format!("v2:{stamp}"))
        {
            return Err(crate::t_pub("ctx_registry_failed"));
        }
        if classic_stamp()?.as_deref() != Some(&stamp) {
            return Err(crate::t_pub("ctx_win11_other_tool"));
        }
        if !delete_classic_key() {
            return Err(crate::t_pub("ctx_registry_failed"));
        }
        // Target removal succeeded. An inert timestamp marker must not prevent
        // the caller from restarting Explorer to apply that successful change.
        if !delete_tree(WIN11_CLASSIC_MARKER) {
            crate::log_line("classic-menu target removed; stale marker cleanup failed");
        }
        Ok(())
    }
}

/// Restart the shell so the rollback takes effect. Explorer windows close
/// and the taskbar reloads — callers must have warned the user.
pub fn restart_explorer() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::time::{Duration, Instant};
    use windows::Win32::UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let failure = || crate::t_pub("ctx_restart_failed");
    let shell_pid = || unsafe {
        let shell = GetShellWindow();
        let mut pid = 0;
        if !shell.is_invalid() {
            GetWindowThreadProcessId(shell, Some(&mut pid));
        }
        pid
    };
    let old_pid = shell_pid();
    if old_pid == 0 {
        return Err(failure());
    }
    let mut session = 0;
    let mut ours = 0;
    unsafe {
        windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(old_pid, &mut session)
            .map_err(|_| failure())?;
        windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(std::process::id(), &mut ours)
            .map_err(|_| failure())?;
    }
    if session != ours {
        return Err(failure());
    }
    let root = std::env::var("SystemRoot").map_err(|_| failure())?;
    let mut task = std::process::Command::new(format!("{root}\\System32\\taskkill.exe"))
        .args([
            "/f",
            "/pid",
            &old_pid.to_string(),
            "/fi",
            &format!("SESSION eq {ours}"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|_| failure())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = task.try_wait().map_err(|_| failure())? {
            if !status.success() {
                return Err(failure());
            }
            break;
        }
        if Instant::now() >= deadline {
            if task.kill().is_ok() {
                let _ = task.wait();
            }
            return Err(failure());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Explorer commonly restarts itself. Give that path time before spawning
    // another process; a successful spawn alone does not prove recovery.
    let deadline = Instant::now() + Duration::from_secs(10);
    let launch_at = Instant::now() + Duration::from_secs(2);
    let mut launched = false;
    loop {
        let pid = shell_pid();
        if pid != 0 && pid != old_pid {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(failure());
        }
        if !launched && Instant::now() >= launch_at {
            std::process::Command::new(format!("{root}\\explorer.exe"))
                .spawn()
                .map_err(|_| failure())?;
            launched = true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_sync_distinguishes_invalid_config_from_explicit_off_and_empty() {
        use std::sync::atomic::Ordering;
        let _test = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        let _writer = crate::runtime::lock(&crate::CONFIG_WRITER);
        let old_config =
            crate::runtime::lock(&crate::CONFIG).replace(idletrigger_core::config::Config {
                ctx_menu_enabled: true,
                ctx_copy_enabled: false,
                ..Default::default()
            });
        let old_doc =
            crate::runtime::lock(&crate::CONFIG_DOC).replace("ctx_rules = 42".parse().unwrap());
        let old_blocked = crate::runtime::CTX_SYNC_BLOCKED.swap(false, Ordering::SeqCst);
        let old_failed = crate::runtime::CONFIG_LOAD_FAILED.swap(false, Ordering::SeqCst);
        assert!(desired_entries().is_err());
        crate::runtime::lock(&crate::CONFIG)
            .as_mut()
            .unwrap()
            .ctx_menu_enabled = false;
        assert!(desired_entries().unwrap().is_empty());
        crate::runtime::CTX_SYNC_BLOCKED.store(true, Ordering::SeqCst);
        assert!(desired_entries().is_err());
        crate::runtime::CTX_SYNC_BLOCKED.store(false, Ordering::SeqCst);
        crate::runtime::lock(&crate::CONFIG)
            .as_mut()
            .unwrap()
            .ctx_menu_enabled = true;
        *crate::runtime::lock(&crate::CONFIG_DOC) = Some("ctx_rules = []".parse().unwrap());
        assert!(desired_entries().unwrap().is_empty());
        *crate::runtime::lock(&crate::CONFIG) = old_config;
        *crate::runtime::lock(&crate::CONFIG_DOC) = old_doc;
        crate::runtime::CTX_SYNC_BLOCKED.store(old_blocked, Ordering::SeqCst);
        crate::runtime::CONFIG_LOAD_FAILED.store(old_failed, Ordering::SeqCst);
    }

    #[test]
    fn classic_marker_requires_matching_generation_or_explicit_legacy_confirmation() {
        assert_eq!(
            classify_classic_marker("10:20", Some("v2:10:20")),
            ClassicState::ByUs
        );
        assert_eq!(
            classify_classic_marker("10:21", Some("v2:10:20")),
            ClassicState::ByOther
        );
        assert_eq!(
            classify_classic_marker("10:20", Some("1")),
            ClassicState::Legacy
        );
        assert_eq!(
            classify_classic_marker("10:20", None),
            ClassicState::ByOther
        );
    }

    fn rule(id: &str, scope: &str) -> core_ctx::CtxRule {
        core_ctx::CtxRule {
            id: id.into(),
            title: id.into(),
            scope: scope.into(),
            command: "tool.exe".into(),
            ..Default::default()
        }
    }

    #[test]
    fn cascades_keep_scope_filters_background_parameters_and_containers() {
        let file = core_ctx::CtxRule {
            patterns: vec!["*.txt".into()],
            ..rule("file", core_ctx::SCOPE_FILE)
        };
        let map = build_entries(
            true,
            false,
            true,
            "app.exe",
            &[file, rule("bg", core_ctx::SCOPE_BACKGROUND)],
        );
        let file_path = format!("{CLASSES}\\*\\{RULES_MENU_KEY}\\shell\\IdleTriggerRule000_file");
        assert!(map[&file_path].contains_key(&Some("AppliesTo".into())));
        assert!(contains_registry_path(
            &map,
            &file_path.to_ascii_lowercase()
        ));
        assert!(!map.keys().any(|path| path.contains("Directory\\shell")));
        assert!(
            !map.keys()
                .any(|path| path.contains("Directory\\Background") && path.contains("_file"))
        );
        let bg = format!(
            "{CLASSES}\\Directory\\Background\\{RULES_MENU_KEY}\\shell\\IdleTriggerRule001_bg\\command"
        );
        assert_eq!(
            map[&bg][&None],
            RegistryValue::Text("\"app.exe\" ctx rule bg \"%V|\"".into())
        );
        for root in ["*", "Directory\\Background"] {
            assert!(map.contains_key(&format!("{CLASSES}\\{root}\\{RULES_MENU_KEY}")));
        }
    }

    #[test]
    fn reordering_changes_menu_order_and_trailing_separators_are_omitted() {
        let a = rule("a", core_ctx::SCOPE_FILE);
        let b = rule("b", core_ctx::SCOPE_FILE);
        for submenu in [false, true] {
            assert_ne!(
                build_entries(true, false, submenu, "app.exe", &[a.clone(), b.clone()]),
                build_entries(true, false, submenu, "app.exe", &[b.clone(), a.clone()])
            );
        }
        let separator = core_ctx::CtxRule {
            separator: true,
            ..Default::default()
        };
        let map = build_entries(
            true,
            false,
            true,
            "app.exe",
            &[a, separator.clone(), b, separator],
        );
        let flags: Vec<_> = map
            .iter()
            .filter(|(_, values)| values.contains_key(&Some("CommandFlags".into())))
            .collect();
        assert_eq!(flags.len(), 1);
        assert!(flags[0].0.ends_with("000_a"));
        assert!(build_entries(true, false, true, "app.exe", &[]).is_empty());
    }

    #[test]
    fn copy_entries_delegate_complete_selections_and_preserve_background_commands() {
        let map = build_entries(true, true, false, "app.exe", &[]);
        for (path, values) in &map {
            if path.contains("\\shell\\00") && !path.ends_with("\\command") {
                assert_eq!(
                    values[&Some("MultiSelectModel".into())],
                    RegistryValue::Text("Player".into())
                );
            }
            if path.ends_with("\\command") {
                assert_eq!(
                    values.contains_key(&Some("DelegateExecute".into())),
                    !path.contains("Directory\\Background")
                );
            }
        }
        for index in 0..crate::ctxselection::FORMATS.len() {
            let class = crate::ctxselection::class_name(index);
            let server = &map[&format!("{CLASSES}\\CLSID\\{class}\\LocalServer32")];
            assert_eq!(
                server[&None],
                RegistryValue::Text("\"app.exe\" ctx-selection".into())
            );
        }
        assert!(
            !map.keys()
                .any(|path| path.contains("Directory\\Background")
                    && path.contains("ctx_copy_target"))
        );
    }
}
