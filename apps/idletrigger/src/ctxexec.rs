//! Context-menu execution: the `ctx` CLI arm, clipboard writes, rule
//! spawning, and the multi-select copy aggregation fed by the IPC fast path.
//!
//! Registry verbs launch `IdleTrigger.exe ctx ...`; with the tray running,
//! the short-lived process forwards over the named pipe and the IPC server
//! thread answers without a UI round trip. Copy requests are collected and
//! flushed by a debounce timer on the hidden window (one clipboard write per
//! multi-selection); rule requests spawn their target directly from the
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

/// Collect-and-flush debounce window: Explorer launches one process per
/// selected item, all within a few hundred milliseconds.
pub const FLUSH_DELAY_MS: u32 = 450;

/// Pending copy requests from the still-spawning per-item processes.
static COLLECTED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

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
    text.replace('%', "%25")
        .replace(' ', "%20")
        .replace('\t', "%09")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
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

/// Writes UTF-16 text to the clipboard, retrying while another process
/// holds it open. A process without any window can still own the clipboard.
pub(crate) fn copy_to_clipboard(text: &str) -> bool {
    for _ in 0..10 {
        unsafe {
            if OpenClipboard(None).is_err() {
                std::thread::sleep(std::time::Duration::from_millis(30));
                continue;
            }
            let written = (|| {
                if EmptyClipboard().is_err() {
                    return false;
                }
                let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
                let Ok(handle) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) else {
                    return false;
                };
                let dst = GlobalLock(handle);
                if dst.is_null() {
                    return false;
                }
                std::ptr::copy_nonoverlapping(
                    wide.as_ptr().cast::<u8>(),
                    dst.cast::<u8>(),
                    wide.len() * 2,
                );
                let _ = GlobalUnlock(handle);
                SetClipboardData(
                    windows::Win32::System::Ole::CF_UNICODETEXT.0 as u32,
                    Some(windows::Win32::Foundation::HANDLE(handle.0)),
                )
                .is_ok()
            })();
            let _ = CloseClipboard();
            if written {
                return true;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    false
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
            let mut buffer = [0u16; 1024];
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

/// `IdleTrigger.exe ctx copy <full|name|dir> "<path>"` (registry verb) or
/// `IdleTrigger.exe ctx rule <id> "<path>"`. Always silent, always exits.
pub fn run_ctx(args: &[OsString]) -> i32 {
    let Some(kind) = args.first().and_then(|value| value.to_str()) else {
        return 0;
    };
    let Some(path_os) = args.get(2) else {
        return 0;
    };
    let param = args
        .get(1)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path = path_of(path_os);
    let forwarded = crate::ipc::send(&format!("{IPC_PREFIX}{kind}:{param}:{}", pct_encode(&path)));
    if forwarded.is_some() {
        // The running instance collected (copy) or spawned (rule) it.
        return 0;
    }
    match kind {
        "copy" => {
            let (quote, separator) = local_copy_settings();
            let text = core_ctx::format_paths(&[formatted(&param, &path)], &quote, &separator);
            copy_to_clipboard(&text);
        }
        "rule" => {
            if let Some(rule) = local_rule(&param) {
                spawn_rule(&rule, &path);
            }
        }
        _ => {}
    }
    0
}

fn local_rule(id: &str) -> Option<core_ctx::CtxRule> {
    let document = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("IdleTrigger.toml")))
        .and_then(|path| {
            let loaded = idletrigger_core::config::load(&path);
            loaded.load_error.is_none().then_some(loaded.document)
        })?;
    let (rules, _) = core_ctx::read_rules(&document).ok()?;
    rules.into_iter().find(|rule| rule.id == id)
}

/// IPC fast path: called on the pipe server thread before the UI queue, so
/// per-item copy bursts never wait behind window work.
pub(crate) fn handle_ipc(request: &str) -> Option<String> {
    let rest = request.strip_prefix(IPC_PREFIX)?;
    match rest.split_once(':') {
        Some(("copy", tail)) => {
            let (format, encoded) = tail.split_once(':')?;
            if !matches!(
                format,
                core_ctx::COPY_FORMAT_FULL
                    | core_ctx::COPY_FORMAT_NAME
                    | core_ctx::COPY_FORMAT_DIR
                    | core_ctx::COPY_FORMAT_STEM
                    | core_ctx::COPY_FORMAT_EXT
                    | core_ctx::COPY_FORMAT_TARGET
            ) {
                return Some("err: bad copy format".into());
            }
            let path = pct_decode(encoded);
            crate::runtime::lock(&COLLECTED).push((format.to_string(), path));
            // (Re)arm the debounce timer; the flush happens on the UI thread.
            if !crate::hwnd(&crate::HIDDEN).is_invalid() {
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(crate::hwnd(&crate::HIDDEN)),
                        crate::WM_CTX_COLLECT,
                        windows::Win32::Foundation::WPARAM(0),
                        windows::Win32::Foundation::LPARAM(0),
                    );
                }
            }
            Some("ok ctx".into())
        }
        Some(("rule", tail)) => {
            let (id, encoded) = tail.split_once(':')?;
            let path = pct_decode(encoded);
            let rules = crate::runtime::lock(&CTX_RULES).clone();
            let executed = rules
                .iter()
                .find(|rule| rule.id == id)
                .is_some_and(|rule| spawn_rule(rule, &path));
            if executed {
                Some("ok ctx".into())
            } else {
                crate::log_line(&format!("ctx rule {id} not applied (missing or filtered)"));
                Some("ok ctx".into())
            }
        }
        _ => Some("err: unknown ctx request".into()),
    }
}

/// Debounce expiry: one clipboard write for the whole multi-selection. The
/// largest group wins on the off chance two formats were requested in the
/// same window (one click always means one format).
pub(crate) fn flush_collected() {
    let pending = std::mem::take(&mut *crate::runtime::lock(&COLLECTED));
    if pending.is_empty() {
        return;
    }
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (format, path) in pending {
        if let Some(group) = groups.iter_mut().find(|(key, _)| key == &format) {
            group.1.push(path);
        } else {
            groups.push((format, vec![path]));
        }
    }
    let Some((format, paths)) = groups.into_iter().max_by_key(|(_, paths)| paths.len()) else {
        return;
    };
    let (quote, separator) = crate::cfg_map(|config| {
        (
            config.ctx_copy_quote.clone(),
            config.ctx_copy_separator.clone(),
        )
    });
    let formatted: Vec<String> = paths.iter().map(|path| formatted(&format, path)).collect();
    let text = core_ctx::format_paths(&formatted, &quote, &separator);
    if !copy_to_clipboard(&text) {
        crate::log_line("ctx copy: clipboard stayed busy");
    }
}

/// Launches a rule's target for one selected path. `ShellExecuteW` handles
/// .bat/.cmd and document associations; `hidden_console` rides the show
/// command (console hosts honor SW_HIDE).
pub(crate) fn spawn_rule(rule: &core_ctx::CtxRule, path: &str) -> bool {
    if !rule.enabled {
        return false;
    }
    let selection = PathBuf::from(path);
    if !core_ctx::extension_matches(&rule.patterns, &selection) {
        crate::log_line(&format!("ctx rule {} skipped: pattern mismatch", rule.id));
        return false;
    }
    let args = core_ctx::substitute_args(&rule.args, path);
    let directory = if rule.working_dir.is_empty() {
        selection
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        rule.working_dir.clone()
    };
    let show = if rule.hidden_console {
        SW_HIDE
    } else {
        SW_SHOWNORMAL
    };
    let file = wide(&rule.command);
    let params = wide(&args);
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
