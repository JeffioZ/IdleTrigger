//! Context-menu execution: the `ctx` CLI arm, clipboard writes, rule
//! spawning, and single-selection clipboard operations through IPC.
//!
//! Registry verbs launch `IdleTrigger.exe ctx ...`; with the tray running,
//! the short-lived process forwards over the named pipe and the IPC server
//! thread answers with the operation result without a UI round trip.
//! Rule requests spawn their target directly from the
//! server thread. Without a running instance everything executes locally.

use idletrigger_core::ctx_menu as core_ctx;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};
use windows::core::PCWSTR;

use crate::wide;

/// IPC message arm prefix, kept in sync with `ipc::dispatch`.
const IPC_PREFIX: &str = "ctx:";
const COPY_SETTINGS_REQUEST: &str = "ctx:copy-settings";
const COPY_SETTINGS_REPLY: &str = "ok ctx-settings:";

/// The rule list published by every config commit (mirrors how automation
/// rules are reloaded from the document inside the writer boundary).
pub(crate) static CTX_RULES: Mutex<Vec<core_ctx::CtxRule>> = Mutex::new(Vec::new());

pub(crate) fn reload_rules() {
    let document = crate::runtime::lock(&crate::CONFIG_DOC).clone();
    let rules = document
        .and_then(|doc| core_ctx::read_rules(&doc).ok().map(|(rules, _)| rules))
        .unwrap_or_default();
    *crate::runtime::lock(&CTX_RULES) = rules;
}

/// The pipe protocol trims whitespace and splits on ':' — percent-encode the
/// path so trailing spaces and tabs survive (':' stays raw; the parser takes
/// the remainder after the third colon).
fn pct_encode(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '%' => output.push_str("%25"),
            ' ' => output.push_str("%20"),
            '\t' => output.push_str("%09"),
            '\r' => output.push_str("%0D"),
            '\n' => output.push_str("%0A"),
            _ => output.push(ch),
        }
    }
    output
}

fn pct_decode(text: &str) -> String {
    let bytes: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == '%' && index + 3 <= bytes.len() {
            let hex: String = bytes[index + 1..index + 3].iter().collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16)
                && let Some(ch) = char::from_u32(byte as u32)
            {
                out.push(ch);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

/// Lossless OsStr → String on Windows (unpaired surrogates yield None).
fn os_to_string(value: &OsStr) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    String::from_utf16(&value.encode_wide().collect::<Vec<u16>>()).ok()
}

fn path_of(value: &OsStr) -> String {
    os_to_string(value).unwrap_or_else(|| value.to_string_lossy().into_owned())
}

/// A real window owns the clipboard even in the short-lived CLI process.
pub(crate) fn copy_to_clipboard(text: &str) -> bool {
    use windows::Win32::Foundation::GlobalFree;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
    };
    unsafe {
        let Ok(owner) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("STATIC"),
            None,
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        ) else {
            return false;
        };
        let mut written = false;
        for _ in 0..10 {
            if OpenClipboard(Some(owner)).is_err() {
                std::thread::sleep(std::time::Duration::from_millis(30));
                continue;
            }
            written = (|| {
                let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
                let Ok(handle) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) else {
                    return false;
                };
                let dst = GlobalLock(handle);
                if dst.is_null() {
                    let _ = GlobalFree(Some(handle));
                    return false;
                }
                std::ptr::copy_nonoverlapping(
                    wide.as_ptr().cast::<u8>(),
                    dst.cast::<u8>(),
                    wide.len() * 2,
                );
                let _ = GlobalUnlock(handle);
                let ok = EmptyClipboard().is_ok()
                    && SetClipboardData(
                        windows::Win32::System::Ole::CF_UNICODETEXT.0 as u32,
                        Some(windows::Win32::Foundation::HANDLE(handle.0)),
                    )
                    .is_ok();
                if !ok {
                    let _ = GlobalFree(Some(handle));
                }
                ok
            })();
            let _ = CloseClipboard();
            if written {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let _ = DestroyWindow(owner);
        written
    }
}

/// One requested copy shape: the raw path formatted per the menu entry.
fn formatted(format: &str, path: &str) -> String {
    let path = Path::new(path);
    match format {
        core_ctx::COPY_FORMAT_NAME => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        core_ctx::COPY_FORMAT_DIR => path
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        // Nilesoft Shell parity: name family without extension / extension
        // only (dot kept — ".pdf" pastes better than "pdf").
        core_ctx::COPY_FORMAT_STEM => path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        core_ctx::COPY_FORMAT_EXT => path
            .extension()
            .map(|ext| format!(".{}", ext.to_string_lossy()))
            .unwrap_or_default(),
        // Nilesoft sel.lnk parity: shortcuts copy the real target path;
        // anything else falls back to the full path so the menu item never
        // yields garbage.
        core_ctx::COPY_FORMAT_TARGET => {
            let is_lnk = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"));
            if is_lnk {
                resolve_lnk(&path.to_string_lossy())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned())
            } else {
                path.to_string_lossy().into_owned()
            }
        }
        _ => path.to_string_lossy().into_owned(),
    }
}

/// Resolves a .lnk to its target path via the shell's own link parser
/// (short-lived COM apartment; both the pipe server thread and the local
/// fallback process arrive uninitialized).
fn resolve_lnk(path: &str) -> Option<String> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize, STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, SLGP_UNCPRIORITY, ShellLink};
    use windows::core::Interface;

    unsafe {
        // RPC_E_CHANGED_MODE (~0x80010106): the thread already runs in the
        // other apartment — the link parser still works, just skip uninit.
        let changed_mode: i32 = 0x8001_0106u32 as i32;
        let apartment = CoInitializeEx(None, COINIT_APARTMENTTHREADED).0;
        let owned = if apartment >= 0 {
            true
        } else if apartment == changed_mode {
            false
        } else {
            return None;
        };
        let result = (|| {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            let wide = windows::core::HSTRING::from(path);
            // IPersistFile::Load is the shell's own .lnk reader.
            let file: windows::Win32::System::Com::IPersistFile = link.cast().ok()?;
            file.Load(&wide, STGM_READ).ok()?;
            let mut buffer = [0u16; 32768];
            let mut find = windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW::default();
            // SLGP_UNCPRIORITY keeps mapped-drive forms readable.
            link.GetPath(&mut buffer, &mut find, SLGP_UNCPRIORITY.0 as u32)
                .ok()?;
            let len = buffer.iter().position(|ch| *ch == 0).unwrap_or(0);
            if len == 0 {
                return None;
            }
            Some(String::from_utf16_lossy(&buffer[..len]))
        })();
        if owned {
            CoUninitialize();
        }
        result
    }
}

/// Loads copy-path preferences from the config beside the EXE — the local
/// fallback path runs before (or without) the tray instance.
fn local_copy_settings() -> (String, String) {
    let config = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("IdleTrigger.toml")))
        .and_then(|path| {
            let loaded = idletrigger_core::config::load(&path);
            loaded.load_error.is_none().then_some(loaded.config)
        })
        .unwrap_or_default();
    (config.ctx_copy_quote, config.ctx_copy_separator)
}

fn selection_copy_settings() -> Option<(String, String)> {
    let reply = crate::ipc::send(COPY_SETTINGS_REQUEST);
    copy_settings_reply(reply.as_deref(), || {
        // No reply can also mean a busy pipe or a tray that is still starting.
        // Fall back only after checking both current and legacy singletons.
        match crate::single_instance::running() {
            Ok(false) => Some(local_copy_settings()),
            _ => None,
        }
    })
}

fn copy_settings_reply(
    reply: Option<&str>,
    without_tray: impl FnOnce() -> Option<(String, String)>,
) -> Option<(String, String)> {
    let Some(reply) = reply else {
        return without_tray();
    };
    let (quote, separator) = reply.strip_prefix(COPY_SETTINGS_REPLY)?.split_once(':')?;
    if !matches!(quote, "auto" | "always" | "none") || !matches!(separator, "newline" | "space") {
        return None;
    }
    Some((quote.into(), separator.into()))
}

/// A complete Shell selection is formatted before opening the clipboard.
/// Never send individual paths through IPC: that would lose batch identity.
pub(crate) fn copy_selection(format: &str, paths: &[String]) -> bool {
    if paths.is_empty() || !valid_copy_format(format) {
        return false;
    }
    let Some((quote, separator)) = selection_copy_settings() else {
        return false;
    };
    let values: Vec<_> = paths.iter().map(|path| formatted(format, path)).collect();
    copy_to_clipboard(&core_ctx::format_paths(&values, &quote, &separator))
}

/// Registry entry point. Failure is visible and returns nonzero; an
/// ambiguous IPC reply is never replayed locally.
pub fn run_ctx(args: &[OsString]) -> i32 {
    let language = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent().map(|dir| {
                idletrigger_core::config::load(&dir.join("IdleTrigger.toml"))
                    .config
                    .language
            })
        })
        .unwrap_or_else(|| "auto".into());
    crate::apply_language(&language);
    let Some(kind) = args.first().and_then(|value| value.to_str()) else {
        return 1;
    };
    let Some(path_os) = args.get(2) else {
        return 1;
    };
    let param = args
        .get(1)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path = path_of(path_os);
    // New registry commands append a non-path character inside the quotes
    // so a root such as C:\ does not consume the closing quote. Old verbs
    // and direct CLI calls without the sentinel remain supported.
    let path = path.strip_suffix('|').unwrap_or(&path).to_string();
    let success =
        match crate::ipc::send(&format!("{IPC_PREFIX}{kind}:{param}:{}", pct_encode(&path))) {
            Some(reply) => reply == "ok ctx",
            None if matches!(crate::single_instance::running(), Ok(false)) => match kind {
                "copy" if valid_copy_format(&param) => {
                    let (quote, separator) = local_copy_settings();
                    copy_to_clipboard(&core_ctx::format_paths(
                        &[formatted(&param, &path)],
                        &quote,
                        &separator,
                    ))
                }
                "rule" => local_rule(&param).is_some_and(|rule| spawn_rule(&rule, &path)),
                _ => false,
            },
            None => false,
        };
    if success {
        0
    } else {
        crate::warn_dialog("", &crate::t_pub("ctx_action_failed"));
        1
    }
}

fn valid_copy_format(format: &str) -> bool {
    matches!(
        format,
        core_ctx::COPY_FORMAT_FULL
            | core_ctx::COPY_FORMAT_NAME
            | core_ctx::COPY_FORMAT_DIR
            | core_ctx::COPY_FORMAT_STEM
            | core_ctx::COPY_FORMAT_EXT
            | core_ctx::COPY_FORMAT_TARGET
    )
}

fn local_rule(id: &str) -> Option<core_ctx::CtxRule> {
    let document = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("IdleTrigger.toml")))
        .and_then(|path| {
            let loaded = idletrigger_core::config::load(&path);
            (loaded.load_error.is_none()
                && loaded.field_errors.is_none()
                && loaded.config.ctx_menu_enabled)
                .then_some(loaded.document)
        })?;
    let (rules, issues) = core_ctx::read_rules(&document).ok()?;
    rules
        .into_iter()
        .enumerate()
        .find(|(index, rule)| rule.id == id && !issues.iter().any(|issue| issue.index == *index))
        .map(|(_, rule)| rule)
}

/// Read one saved rule from the running tray; never replay a failed IPC locally.
pub(crate) fn rule_selection(id: &str, paths: &[String]) -> bool {
    let request = |offset| crate::ipc::send(&format!("ctx:rule-settings:{offset}:{id}"));
    let rule = match request(0) {
        Some(reply) => read_rule_chunks(id, reply, request),
        None => match crate::single_instance::running() {
            Ok(false) => local_rule(id),
            _ => None,
        },
    };
    rule.is_some_and(|rule| spawn_selection(&rule, paths))
}

const MAX_RULE_REPLY: usize = 256 * 1024;
fn rule_revision(json: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    json.hash(&mut hash);
    format!("{:x}", hash.finish())
}

/// Keep each reply within the pipe's 4 KiB message bound, without splitting UTF-8.
fn rule_chunk(rule: &core_ctx::CtxRule, offset: usize) -> Option<String> {
    let json = serde_json::to_string(rule).ok()?;
    if json.len() > MAX_RULE_REPLY || offset >= json.len() || !json.is_char_boundary(offset) {
        return None;
    }
    let mut end = (offset + 1800).min(json.len());
    while !json.is_char_boundary(end) {
        end -= 1;
    }
    // Hex avoids the pipe's whitespace trimming changing a chunk boundary.
    let chunk = idletrigger_core::encoding::hex_lower(&json.as_bytes()[offset..end]);
    Some(format!(
        "ok ctx-rule:{}:{}:{chunk}",
        rule_revision(&json),
        json.len()
    ))
}

fn read_rule_chunks(
    id: &str,
    mut reply: String,
    request: impl Fn(usize) -> Option<String>,
) -> Option<core_ctx::CtxRule> {
    let mut json = String::new();
    let mut version = None;
    loop {
        let mut parts = reply.strip_prefix("ok ctx-rule:")?.splitn(3, ':');
        let revision = parts.next()?;
        let total = parts.next()?.parse::<usize>().ok()?;
        let encoded = parts.next()?;
        if !encoded.is_ascii() || encoded.len() % 2 != 0 {
            return None;
        }
        let bytes = (0..encoded.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&encoded[i..i + 2], 16).ok())
            .collect::<Option<Vec<_>>>()?;
        let chunk = String::from_utf8(bytes).ok()?;
        if total > MAX_RULE_REPLY || chunk.is_empty() || json.len() + chunk.len() > total {
            return None;
        }
        let snapshot = (revision.to_owned(), total);
        if version.as_ref().is_some_and(|old| old != &snapshot) {
            return None;
        }
        version = Some(snapshot);
        json.push_str(&chunk);
        if json.len() == total {
            if rule_revision(&json) != revision {
                return None;
            }
            return serde_json::from_str::<core_ctx::CtxRule>(&json)
                .ok()
                .filter(|rule| rule.id == id);
        }
        reply = request(json.len())?;
    }
}

/// IPC fast path: called on the pipe server thread before the UI queue, so
/// per-item copy bursts never wait behind window work.
pub(crate) fn handle_ipc(request: &str) -> Option<String> {
    if request == COPY_SETTINGS_REQUEST {
        return Some(crate::cfg_map(|config| {
            format!(
                "{COPY_SETTINGS_REPLY}{}:{}",
                config.ctx_copy_quote, config.ctx_copy_separator
            )
        }));
    }
    let rest = request.strip_prefix(IPC_PREFIX)?;
    match rest.split_once(':') {
        Some(("rule-settings", tail)) => {
            let (offset, id) = tail.split_once(':')?;
            let offset = offset.parse::<usize>().ok()?;
            if !crate::cfg_map(|config| config.ctx_menu_enabled) {
                return Some("err: context menu disabled".into());
            }
            let rules = crate::runtime::lock(&CTX_RULES);
            let reply = rules
                .iter()
                .find(|rule| rule.id == id && rule.enabled && !rule.separator)
                .and_then(|rule| rule_chunk(rule, offset));
            Some(reply.unwrap_or_else(|| "err: rule unavailable or configuration too large".into()))
        }
        Some(("copy", tail)) => {
            let (format, encoded) = tail.split_once(':')?;
            if !valid_copy_format(format) {
                return Some("err: bad copy format".into());
            }
            let path = pct_decode(encoded);
            let (quote, separator) =
                crate::cfg_map(|c| (c.ctx_copy_quote.clone(), c.ctx_copy_separator.clone()));
            let text = core_ctx::format_paths(&[formatted(format, &path)], &quote, &separator);
            Some(
                if copy_to_clipboard(&text) {
                    "ok ctx"
                } else {
                    "err: clipboard unavailable"
                }
                .into(),
            )
        }
        Some(("rule", tail)) => {
            let (id, encoded) = tail.split_once(':')?;
            let path = pct_decode(encoded);
            let rules = crate::runtime::lock(&CTX_RULES).clone();
            let executed = crate::cfg_map(|config| config.ctx_menu_enabled)
                && rules
                    .iter()
                    .find(|rule| rule.id == id)
                    .is_some_and(|rule| spawn_rule(rule, &path));
            if executed {
                Some("ok ctx".into())
            } else {
                crate::log_line(&format!("ctx rule {id} not applied (missing or filtered)"));
                Some("err: rule missing, filtered, or failed to launch".into())
            }
        }
        _ => Some("err: unknown ctx request".into()),
    }
}

/// Launches a rule's target for one selected path. `ShellExecuteW` handles
/// .bat/.cmd and document associations; `hidden_console` rides the show
/// command (console hosts honor SW_HIDE).
pub(crate) fn spawn_rule(rule: &core_ctx::CtxRule, path: &str) -> bool {
    spawn_selection(rule, &[path.to_owned()])
}

fn selection_allowed(rule: &core_ctx::CtxRule, paths: &[String]) -> bool {
    if !rule.enabled || rule.separator {
        return false;
    }
    if paths.is_empty()
        || (rule.selection_mode == core_ctx::SelectionMode::Single && paths.len() != 1)
        || !core_ctx::prepare_rules(vec![rule.clone()]).1.is_empty()
    {
        return false;
    }
    paths.iter().all(|path| {
        let selection = PathBuf::from(path);
        let allowed = match rule.scope.as_str() {
            core_ctx::SCOPE_FILE => selection.is_file(),
            core_ctx::SCOPE_DIR | core_ctx::SCOPE_BACKGROUND => selection.is_dir(),
            _ => selection.exists(),
        };
        allowed && core_ctx::extension_matches(&rule.patterns, &selection)
    })
}

fn spawn_selection(rule: &core_ctx::CtxRule, paths: &[String]) -> bool {
    if !selection_allowed(rule, paths) {
        return false;
    }
    let batches: Vec<&[String]> = if rule.selection_mode == core_ctx::SelectionMode::Together {
        vec![paths]
    } else {
        paths.chunks(1).collect()
    };
    let Some(arguments) = batches
        .iter()
        .map(|batch| core_ctx::rule_selection_args(rule, batch))
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    // Validate every launch before any side effects; Shell/CRT command lines are bounded.
    if arguments.iter().any(|args| {
        args.contains('\0')
            || rule.command.encode_utf16().count() + args.encode_utf16().count() + 4 > 32767
    }) {
        return false;
    }
    let mut success = true;
    for (batch, args) in batches.iter().zip(arguments) {
        success &= launch_rule(rule, &batch[0], &args);
    }
    success
}

fn launch_rule(rule: &core_ctx::CtxRule, path: &str, args: &str) -> bool {
    let selection = PathBuf::from(path);
    let directory = if rule.working_dir.is_empty() {
        if selection.is_dir() {
            path.to_string()
        } else {
            selection
                .parent()
                .map(|parent| parent.to_string_lossy().into_owned())
                .unwrap_or_default()
        }
    } else {
        rule.working_dir.clone()
    };
    let show = if rule.hidden_console {
        SW_HIDE
    } else {
        SW_SHOWNORMAL
    };
    let file = wide(&rule.command);
    let params = wide(args);
    let dir = wide(&directory);
    let result = unsafe {
        ShellExecuteW(
            None,
            None,
            PCWSTR(file.as_ptr()),
            PCWSTR(params.as_ptr()),
            PCWSTR(dir.as_ptr()),
            show,
        )
    };
    let ok = result.0 as usize > 32;
    if !ok {
        crate::log_line(&format!(
            "ctx rule {} failed to launch {} (code {})",
            rule.id, rule.command, result.0 as usize
        ));
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_chunks_preserve_long_unicode_and_reject_mid_read_changes() {
        let rule = core_ctx::CtxRule {
            id: "batch".into(),
            title: "tool".into(),
            command: "tool.exe".into(),
            args: format!("%* {}", "中文 %1 : spaces  ".repeat(700)),
            selection_mode: core_ctx::SelectionMode::Together,
            ..Default::default()
        };
        let first = rule_chunk(&rule, 0).unwrap();
        assert!(first.len() + 2 <= 4096);
        assert_eq!(
            read_rule_chunks("batch", first.clone(), |offset| {
                let chunk = rule_chunk(&rule, offset)?;
                assert!(chunk.len() + 2 <= 4096);
                Some(chunk)
            }),
            Some(rule.clone())
        );
        let changed = core_ctx::CtxRule {
            title: "changed".into(),
            ..rule.clone()
        };
        assert!(
            read_rule_chunks("batch", first.clone(), |offset| rule_chunk(
                &changed, offset
            ))
            .is_none()
        );
        assert!(
            read_rule_chunks("different", first.clone(), |offset| rule_chunk(
                &rule, offset
            ))
            .is_none()
        );
        assert!(read_rule_chunks("batch", first, |_| None).is_none());
        assert!(
            read_rule_chunks("batch", "err: disabled".into(), |_| panic!(
                "must not retry"
            ))
            .is_none()
        );
    }

    #[test]
    fn selection_validation_rejects_entire_mixed_batch_before_launch() {
        let executable = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let folder = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut rule = core_ctx::CtxRule {
            title: "test".into(),
            command: "unused.exe".into(),
            args: "%*".into(),
            selection_mode: core_ctx::SelectionMode::Together,
            ..Default::default()
        };
        assert!(selection_allowed(&rule, std::slice::from_ref(&executable)));
        assert!(!selection_allowed(
            &rule,
            &[executable.clone(), folder.clone()]
        ));
        rule.scope = core_ctx::SCOPE_FILE_DIR.into();
        assert!(selection_allowed(
            &rule,
            &[executable.clone(), folder.clone()]
        ));
        rule.selection_mode = core_ctx::SelectionMode::Single;
        assert!(!selection_allowed(&rule, &[executable.clone(), folder]));
        rule.patterns = vec!["*.txt".into()];
        assert!(!selection_allowed(&rule, &[executable]));
        assert!(!selection_allowed(&rule, &[]));
    }

    #[test]
    fn copy_settings_use_runtime_snapshot_and_never_mask_ipc_errors() {
        let _test = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        let old = crate::runtime::lock(&crate::CONFIG).replace(idletrigger_core::config::Config {
            ctx_copy_quote: "none".into(),
            ctx_copy_separator: "space".into(),
            ..Default::default()
        });
        let reply = handle_ipc(COPY_SETTINGS_REQUEST).unwrap();
        *crate::runtime::lock(&crate::CONFIG) = old;
        let must_not_read_disk = || panic!("connected instance must not fall back to disk");
        assert_eq!(
            copy_settings_reply(Some(&reply), must_not_read_disk),
            Some(("none".into(), "space".into()))
        );
        for reply in [
            "err: IPC response unavailable",
            "err: unknown ctx request",
            "ok ctx-settings:none:bogus",
            "ok ctx-settings:bogus:newline",
            "",
        ] {
            assert!(copy_settings_reply(Some(reply), must_not_read_disk).is_none());
        }
        assert_eq!(
            copy_settings_reply(None, || Some(("always".into(), "newline".into()))),
            Some(("always".into(), "newline".into()))
        );
        assert!(copy_settings_reply(None, || None).is_none());
    }

    #[test]
    fn percent_roundtrip_keeps_spaces_and_trailing_tabs() {
        let original = "C:\\a b\\c d.txt\t";
        assert_eq!(pct_decode(&pct_encode(original)), original);
        assert_eq!(pct_decode("plain"), "plain");
        assert_eq!(pct_decode("100% sure"), "100% sure");
    }

    #[test]
    fn copy_formats_shape_the_path() {
        assert_eq!(formatted("full", "C:\\a\\b.txt"), "C:\\a\\b.txt");
        assert_eq!(formatted("name", "C:\\a\\b.txt"), "b.txt");
        assert_eq!(formatted("dir", "C:\\a\\b.txt"), "C:\\a");
        // Drive roots have no file_name/parent components: fall back whole.
        assert_eq!(formatted("name", "E:\\"), "E:\\");
    }
}
