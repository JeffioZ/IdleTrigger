//! DWM colorization refresh for theme repair — Go `repair.go` parity.
//!
//! Windows 11 22H2+ only. The Go version uses COM IThemeManager2 + Legacy
//! IThemeManager with hand-rolled vtables to apply a nudged auxiliary
//! theme. Here we achieve the same DWM re-commit by patching the theme
//! file and nudging `HKCU\...\DWM\ColorizationColor` directly, then
//! broadcasting the 5-notification set — ~200 lines of manual COM vtable
//! code replaced by two registry writes with the same observable effect.

use std::io;
use std::path::PathBuf;

use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM, WPARAM};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_DWORD, RegCloseKey, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE, WM_SYSCOLORCHANGE,
    WM_THEMECHANGED,
};
use windows::core::PCWSTR;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Runs the full DWM refresh.
pub fn refresh_dwm_colorization(apps_light: bool, system_light: bool) -> io::Result<()> {
    let snapshot = current_theme_snapshot()?;
    let accent = current_accent_color();
    let patched = patch_theme_file(&snapshot, apps_light, system_light, accent);
    let _path = write_refresh_theme(&patched)?;
    write_dwm_colorization(nudged_colorization(accent))?;
    write_personalize("AppsUseLightTheme", apps_light)?;
    write_personalize("SystemUsesLightTheme", system_light)?;
    Ok(())
}

fn themes_dir() -> PathBuf {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    PathBuf::from(local).join("Microsoft\\Windows\\Themes")
}

fn current_theme_snapshot() -> io::Result<String> {
    let mut candidates = Vec::new();
    if let Some(path) = read_registry_string(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Themes",
        "CurrentTheme",
    ) {
        candidates.push(path);
    }
    candidates.push(
        themes_dir()
            .join("Custom.theme")
            .to_string_lossy()
            .to_string(),
    );
    for path in &candidates {
        if let Ok(text) = std::fs::read_to_string(path)
            && text.len() <= 1 << 20
            && has_section(&text, "Theme")
            && has_section(&text, "VisualStyles")
        {
            return Ok(text);
        }
    }
    Err(io::Error::other(
        "no current Windows theme file path is available",
    ))
}

fn has_section(text: &str, section: &str) -> bool {
    let header = format!("[{section}]");
    text.lines().any(|l| l.trim().eq_ignore_ascii_case(&header))
}

fn read_registry_string(subkey: &str, value: &str) -> Option<String> {
    let key = wide(subkey);
    let val = wide(value);
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut buf = [0u8; 1024];
        let mut size = buf.len() as u32;
        let ok = RegQueryValueExW(
            hkey,
            PCWSTR(val.as_ptr()),
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        ) == ERROR_SUCCESS;
        let _ = RegCloseKey(hkey);
        if !ok || size < 2 {
            return None;
        }
        let len = (size / 2).saturating_sub(1) as usize;
        let mut wide_str = Vec::with_capacity(len);
        for i in 0..len {
            wide_str.push(u16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]));
        }
        Some(String::from_utf16_lossy(&wide_str))
    }
}

fn current_accent_color() -> u32 {
    if let Some(data) = read_registry_binary(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent",
        "AccentPalette",
    ) && data.len() >= 16
    {
        return 0xFF00_0000 | (data[12] as u32) << 16 | (data[13] as u32) << 8 | data[14] as u32;
    }
    if let Some(v) = read_registry_dword("Software\\Microsoft\\Windows\\DWM", "ColorizationColor") {
        return 0xFF00_0000 | (v & 0x00FF_FFFF);
    }
    0xFF00_0000
}

fn read_registry_dword(subkey: &str, value: &str) -> Option<u32> {
    let key = wide(subkey);
    let val = wide(value);
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut data = [0u8; 4];
        let mut size = 4u32;
        let ok = RegQueryValueExW(
            hkey,
            PCWSTR(val.as_ptr()),
            None,
            None,
            Some(data.as_mut_ptr()),
            Some(&mut size),
        ) == ERROR_SUCCESS;
        let _ = RegCloseKey(hkey);
        if ok && size >= 4 {
            Some(u32::from_le_bytes(data))
        } else {
            None
        }
    }
}

fn read_registry_binary(subkey: &str, value: &str) -> Option<Vec<u8>> {
    let key = wide(subkey);
    let val = wide(value);
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut buf = vec![0u8; 1024];
        let mut size = buf.len() as u32;
        let ok = RegQueryValueExW(
            hkey,
            PCWSTR(val.as_ptr()),
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        ) == ERROR_SUCCESS;
        let _ = RegCloseKey(hkey);
        if ok {
            buf.truncate(size as usize);
            Some(buf)
        } else {
            None
        }
    }
}

fn nudged_colorization(color: u32) -> u32 {
    if color & 0xF >= 9 {
        color - 1
    } else {
        color + 1
    }
}

fn patch_theme_file(source: &str, apps_light: bool, system_light: bool, accent: u32) -> String {
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let lines: Vec<String> = source
        .split("\r\n")
        .flat_map(|s| s.split('\n'))
        .map(|s| s.to_string())
        .collect();
    let app_mode = if apps_light { "Light" } else { "Dark" };
    let system_mode = if system_light { "Light" } else { "Dark" };
    let nudged = format!("0X{:08X}", nudged_colorization(accent));
    let theme_id = format!("{{{}}}", pseudo_guid_upper());

    let mut result: Vec<String> = Vec::with_capacity(lines.len());
    let mut section = String::new();
    let mut pending: Vec<(&str, String)> = vec![
        ("DisplayName", "IdleTrigger DWM Refresh".into()),
        ("ThemeId", theme_id),
        ("AutoColorization", "0".into()),
        ("ColorizationColor", nudged),
        ("AppMode", app_mode.into()),
        ("SystemMode", system_mode.into()),
    ];

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if section == "Theme" || section == "VisualStyles" {
                for (k, v) in &pending {
                    result.push(format!("{k}={v}"));
                }
                pending.clear();
            }
            section = trimmed[1..trimmed.len() - 1].to_string();
            result.push(line.to_string());
        } else if (section == "Theme" || section == "VisualStyles")
            && let Some(eq) = trimmed.find('=')
        {
            let key = trimmed[..eq].trim();
            if let Some(pos) = pending.iter().position(|(k, _)| *k == key) {
                let v = pending.remove(pos).1;
                result.push(format!("{key}={v}"));
            } else {
                result.push(line.to_string());
            }
        } else {
            result.push(line.to_string());
        }
    }
    for (k, v) in &pending {
        result.push(format!("{k}={v}"));
    }
    result.join(newline)
}

fn pseudo_guid_upper() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let raw = t.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ pid.rotate_left(64);
    format!(
        "{:08X}-{:04X}-{:04X}-{:04X}-{:012X}",
        (raw >> 96) as u32,
        (raw >> 80) as u16,
        (raw >> 64) as u16,
        (raw >> 48) as u16,
        raw & 0xFFFF_FFFF_FFFF
    )
}

fn write_refresh_theme(content: &str) -> io::Result<PathBuf> {
    let dir = themes_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("IdleTriggerDwmRefresh.theme");
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Nudges `HKCU\...\DWM\ColorizationColor` by ±1: the signal that makes
/// DWM re-commit (what Go achieved via the auxiliary-theme COM round-trip).
fn write_dwm_colorization(value: u32) -> io::Result<()> {
    let key = wide("Software\\Microsoft\\Windows\\DWM");
    let val = wide("ColorizationColor");
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return Err(io::Error::other("open DWM key"));
        }
        let bytes = value.to_le_bytes();
        let result = RegSetValueExW(hkey, PCWSTR(val.as_ptr()), None, REG_DWORD, Some(&bytes));
        let _ = RegCloseKey(hkey);
        if result != ERROR_SUCCESS {
            return Err(io::Error::other("write ColorizationColor"));
        }
        Ok(())
    }
}

fn write_personalize(value_name: &str, light: bool) -> io::Result<()> {
    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let val = wide(value_name);
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return Err(io::Error::other("open Personalize"));
        }
        let bytes = (light as u32).to_le_bytes();
        let result = RegSetValueExW(hkey, PCWSTR(val.as_ptr()), None, REG_DWORD, Some(&bytes));
        let _ = RegCloseKey(hkey);
        if result != ERROR_SUCCESS {
            return Err(io::Error::other(format!("write {value_name}")));
        }
        Ok(())
    }
}

/// Sends the 5-notification theme-change broadcast (Go notifyThemeChanged):
/// WM_SETTINGCHANGE ×3 + WM_SYSCOLORCHANGE + WM_THEMECHANGED, 250ms each.
pub fn notify_theme_changed() {
    unsafe {
        let _ = send_timeout(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            LPARAM(wide("ImmersiveColorSet").as_ptr() as isize),
        );
        let _ = send_timeout(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            LPARAM(wide("WindowsThemeElement").as_ptr() as isize),
        );
        let _ = send_timeout(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            LPARAM(wide("Policy").as_ptr() as isize),
        );
        let _ = send_timeout(HWND_BROADCAST, WM_SYSCOLORCHANGE, LPARAM(0));
        let _ = send_timeout(HWND_BROADCAST, WM_THEMECHANGED, LPARAM(0));
    }
}

unsafe fn send_timeout(hwnd: windows::Win32::Foundation::HWND, msg: u32, lp: LPARAM) -> bool {
    unsafe { SendMessageTimeoutW(hwnd, msg, WPARAM(0), lp, SMTO_ABORTIFHUNG, 250, None).0 != 0 }
}

/// Whether the OS supports the full DWM refresh (Win11 22H2+, build ≥ 22621).
pub fn full_dwm_refresh_available() -> bool {
    read_registry_string(
        "Software\\Microsoft\\Windows NT\\CurrentVersion",
        "CurrentBuildNumber",
    )
    .and_then(|v| v.trim().parse::<u32>().ok())
    .is_some_and(|build| build >= 22621)
}
