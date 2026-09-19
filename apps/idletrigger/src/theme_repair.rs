//! Windows 11 22H2+ DWM colorization repair: apply an auxiliary theme through
//! COM, restore the original theme and color preferences, then notify apps.

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

use crate::theme::{PERSONALIZE_KEY, read_registry_dword};
use crate::wide;

/// Runs the full DWM refresh.
pub fn refresh_dwm_colorization() -> io::Result<()> {
    let session = crate::theme_com::Session::new()?;
    let snapshot = current_theme_snapshot()?;
    let apps_light = read_registry_dword(PERSONALIZE_KEY, "AppsUseLightTheme")
        .map(|v| v != 0)
        .unwrap_or_else(|| theme_mode(&snapshot, "AppMode"));
    let system_light = read_registry_dword(PERSONALIZE_KEY, "SystemUsesLightTheme")
        .map(|v| v != 0)
        .unwrap_or_else(|| theme_mode(&snapshot, "SystemMode"));
    let accent = current_accent_color()
        .ok_or_else(|| io::Error::other("Windows accent color is unavailable"))?;
    let original_dwm =
        read_registry_dword("Software\\Microsoft\\Windows\\DWM", "ColorizationColor");
    let patched = patch_theme_file(&snapshot, apps_light, system_light, accent);
    let path = write_refresh_theme(&patched)?;
    let applied = session.apply(&path.to_string_lossy());
    if applied.is_ok() {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    // Always attempt recovery, including when applying the helper theme failed.
    let mut errors = Vec::new();
    if let Err(error) = applied {
        errors.push(format!("apply helper theme: {error}"));
    }
    if let Err(error) = session.restore()
        && let Err(retry) = session.restore()
    {
        errors.push(format!("restore original theme: {error}; retry: {retry}"));
    }
    if let Some(color) = original_dwm
        && let Err(error) = write_dwm_colorization(color)
    {
        errors.push(error.to_string());
    }
    for (key, light) in [
        ("AppsUseLightTheme", apps_light),
        ("SystemUsesLightTheme", system_light),
    ] {
        if let Err(error) = write_personalize(key, light) {
            errors.push(error.to_string());
        } else if read_registry_dword(PERSONALIZE_KEY, key) != Some(light as u32) {
            errors.push(format!("Windows did not retain {key}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(errors.join("; ")))
    }
}

fn theme_mode(source: &str, key: &str) -> bool {
    let mut section = false;
    for line in source.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line.eq_ignore_ascii_case("[VisualStyles]");
        } else if section
            && let Some((name, value)) = line.split_once('=')
            && name.trim().eq_ignore_ascii_case(key)
        {
            return !value.trim().eq_ignore_ascii_case("Dark");
        }
    }
    true
}
fn themes_dir() -> io::Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| io::Error::other("LOCALAPPDATA is unavailable"))?;
    Ok(local.join("Microsoft\\Windows\\Themes"))
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
        themes_dir()?
            .join("Custom.theme")
            .to_string_lossy()
            .to_string(),
    );
    for path in &candidates {
        if let Ok(text) = read_theme_snapshot(path)
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

fn read_theme_snapshot(path: &str) -> io::Result<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((1 << 20) + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1 << 20 {
        return Err(io::Error::other("theme snapshot exceeds the size limit"));
    }
    if bytes.starts_with(&[0xff, 0xfe]) {
        if !bytes.len().is_multiple_of(2) {
            return Err(io::Error::other("incomplete UTF-16 theme"));
        }
        let text: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect();
        String::from_utf16(&text).map_err(io::Error::other)
    } else {
        String::from_utf8(bytes)
            .map(|s| s.trim_start_matches('\u{feff}').to_string())
            .map_err(io::Error::other)
    }
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

fn current_accent_color() -> Option<u32> {
    if let Some(data) = read_registry_binary(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent",
        "AccentPalette",
    ) && data.len() >= 16
    {
        return Some(
            0xFF00_0000 | (data[12] as u32) << 16 | (data[13] as u32) << 8 | data[14] as u32,
        );
    }
    if let Some(v) = read_registry_dword("Software\\Microsoft\\Windows\\DWM", "ColorizationColor") {
        return Some(0xFF00_0000 | (v & 0x00FF_FFFF));
    }
    None
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
    let mut lines: Vec<String> = source.lines().map(str::to_string).collect();
    let id = format!("{{{}}}", pseudo_guid_upper());
    let color = format!("0X{:08X}", nudged_colorization(accent));
    for (section, values) in [
        (
            "Theme",
            vec![("DisplayName", "IdleTrigger DWM Refresh"), ("ThemeId", &id)],
        ),
        (
            "VisualStyles",
            vec![
                ("AutoColorization", "0"),
                ("ColorizationColor", &color),
                ("AppMode", if apps_light { "Light" } else { "Dark" }),
                ("SystemMode", if system_light { "Light" } else { "Dark" }),
            ],
        ),
    ] {
        let header = format!("[{section}]");
        let Some(start) = lines
            .iter()
            .position(|line| line.trim().eq_ignore_ascii_case(&header))
            .map(|i| i + 1)
        else {
            continue;
        };
        let end = (start..lines.len())
            .find(|i| lines[*i].trim().starts_with('['))
            .unwrap_or(lines.len());
        let mut missing = Vec::new();
        for (key, value) in values {
            let mut found = false;
            for line in &mut lines[start..end] {
                if line
                    .split_once('=')
                    .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(key))
                {
                    *line = format!("{key}={value}");
                    found = true;
                }
            }
            if !found {
                missing.push(format!("{key}={value}"));
            }
        }
        lines.splice(end..end, missing);
    }
    lines.join(newline) + newline
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
    let dir = themes_dir()?;
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
    let key = wide(PERSONALIZE_KEY);
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
    crate::system::windows_build() >= 22621
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_theme_updates_only_the_correct_sections_and_preserves_modes() {
        let source = "[theme]\r\nDisplayName=Original\r\n[Control Panel\\Desktop]\r\nWallpaper=keep.jpg\r\n[visualstyles]\r\nappmode=Dark\r\nSystemMode=Light\r\nPath=keep.msstyles\r\n[Sounds]\r\nAppMode=untouched\r\n";
        let patched = patch_theme_file(source, false, true, 0xff123456);
        assert!(patched.contains("Wallpaper=keep.jpg\r\n"));
        assert!(patched.contains("Path=keep.msstyles\r\n"));
        assert!(patched.contains("[Sounds]\r\nAppMode=untouched\r\n"));
        let visual = patched
            .split("[visualstyles]")
            .nth(1)
            .unwrap()
            .split("[Sounds]")
            .next()
            .unwrap();
        assert!(visual.contains("ColorizationColor=0XFF123457"));
        assert!(!visual.contains("DisplayName="));
        assert!(!theme_mode(&patched, "AppMode"));
        assert!(theme_mode(&patched, "SystemMode"));
    }
}
