//! Wallpaper and pointer linkage, complete pre-change snapshots, and restore.
//! Native operations run on the theme worker; this module owns no windows.

use crate::wide;
use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, KEY_WRITE, RegCloseKey,
    RegOpenKeyExW, RegSetValueExW,
};
use windows::Win32::UI::Shell::{
    DESKTOP_WALLPAPER_POSITION, DWPOS_FILL, DesktopWallpaper, IDesktopWallpaper,
};
use windows::core::PCWSTR;

// ---- Appearance linkage (wallpaper + cursor schemes) -----------------------

const CURSORS_KEY: &str = "Control Panel\\Cursors";
const SCHEMES_KEY: &str = "Control Panel\\Cursors\\Schemes";
const SYSTEM_SCHEMES_KEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Control Panel\\Cursors\\Schemes";
const SCHEME_SOURCES: [(HKEY, &str); 2] = [
    (HKEY_CURRENT_USER, SCHEMES_KEY),
    (HKEY_LOCAL_MACHINE, SYSTEM_SCHEMES_KEY),
];
// Keep the first 15 roles in the original snapshot order for compatibility.
const LEGACY_CURSOR_COUNT: usize = 15;
const CURSOR_NAMES: [&str; 17] = [
    "Arrow",
    "Help",
    "AppStarting",
    "Wait",
    "NWPen",
    "No",
    "SizeNS",
    "SizeWE",
    "Crosshair",
    "IBeam",
    "SizeNWSE",
    "SizeNESW",
    "SizeAll",
    "UpArrow",
    "Hand",
    "Pin",
    "Person",
];
// Windows' comma-separated scheme order differs from our saved snapshot order.
const SCHEME_TO_SNAPSHOT: [usize; 17] = [0, 1, 2, 3, 6, 7, 8, 9, 4, 5, 10, 11, 12, 13, 14, 15, 16];

/// Applies the wallpaper and cursor scheme configured for the given side.
/// Each item is best-effort: a failure logs and never blocks the theme
/// switch (the registry Personalize write already succeeded).
pub fn apply(dark: bool, changes: crate::settings_model::AppearanceChanges) -> Result<(), String> {
    crate::background_io::ensure_running()?;
    let (wallpaper, scheme) = crate::cfg_map(|c| {
        (
            if dark {
                c.theme_dark_wallpaper.clone()
            } else {
                c.theme_light_wallpaper.clone()
            },
            if dark {
                c.theme_dark_cursor_scheme.clone()
            } else {
                c.theme_light_cursor_scheme.clone()
            },
        )
    });
    let wallpaper = if changes.wallpaper[usize::from(dark)] {
        wallpaper
    } else {
        String::new()
    };
    let scheme = if changes.cursor[usize::from(dark)] {
        scheme
    } else {
        String::new()
    };
    // Capture the pre-switch state before the first modification lands.
    let (wallpaper_ready, cursor_ready) =
        ensure_restore_snapshot(!wallpaper.trim().is_empty(), !scheme.trim().is_empty());
    let mut errors = Vec::new();
    if !wallpaper.trim().is_empty()
        && let Err(error) = wallpaper_ready.and_then(|()| apply_wallpaper(&wallpaper))
    {
        crate::log_line(&format!("wallpaper switch failed: {error}"));
        errors.push(error);
    }
    if !scheme.trim().is_empty()
        && let Err(error) = cursor_ready.and_then(|()| apply_cursor_scheme(&scheme))
    {
        crate::log_line(&format!("cursor scheme switch failed: {error}"));
        errors.push(error);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Wallpaper via SPI_SETDESKWALLPAPER; the file must exist (Windows copies
/// or converts it into the transcoded wallpaper cache). Flags stay 0: this
/// path is the transient day/night linkage, deliberately NOT persisting to
/// the user's theme ini (the restore path and the Windows default both go
/// through IDesktopWallpaper, which does persist).
fn apply_wallpaper(path: &str) -> Result<(), String> {
    crate::background_io::require_file(std::path::Path::new(path))?;
    crate::background_io::ensure_running()?;
    let wide_path = wide(path);
    let ok = unsafe {
        windows::Win32::UI::WindowsAndMessaging::SystemParametersInfoW(
            windows::Win32::UI::WindowsAndMessaging::SPI_SETDESKWALLPAPER,
            0,
            Some(wide_path.as_ptr() as *mut _),
            windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_ok() {
        Ok(())
    } else {
        Err("SystemParametersInfoW(SPI_SETDESKWALLPAPER) rejected the request".into())
    }
}

/// User schemes take precedence over identically named built-in schemes.
pub fn cursor_schemes() -> Vec<String> {
    let mut names = Vec::new();
    for (root, path) in SCHEME_SOURCES {
        names.extend(cursor_scheme_names(root, path));
    }
    let mut seen = std::collections::HashSet::new();
    names.retain(|name| seen.insert(name.to_lowercase()));
    names.sort_by_key(|name| name.to_lowercase());
    names
}

fn cursor_scheme_names(root: HKEY, path: &str) -> Vec<String> {
    unsafe {
        let schemes = wide(path);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            root,
            PCWSTR(schemes.as_ptr()),
            None,
            KEY_READ | KEY_WOW64_64KEY,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return Vec::new();
        }
        let mut names = Vec::new();
        for index in 0.. {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            let name_ptr = windows::core::PWSTR(name.as_mut_ptr());
            let status = windows::Win32::System::Registry::RegEnumValueW(
                hkey,
                index,
                Some(name_ptr),
                &mut len,
                None,
                None,
                None,
                None,
            );
            if status == windows::Win32::Foundation::ERROR_NO_MORE_ITEMS {
                break;
            }
            if status == ERROR_SUCCESS {
                names.push(String::from_utf16_lossy(&name[..len as usize]));
            }
            if index > 512 {
                break;
            }
        }
        let _ = RegCloseKey(hkey);
        names
    }
}

fn read_cursor_scheme(name: &str) -> Result<(Vec<String>, u32), String> {
    for (index, (root, path)) in SCHEME_SOURCES.into_iter().enumerate() {
        let value = unsafe {
            let mut hkey = HKEY::default();
            let opened = RegOpenKeyExW(
                root,
                PCWSTR(wide(path).as_ptr()),
                None,
                KEY_READ | KEY_WOW64_64KEY,
                &mut hkey,
            );
            if opened == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
                continue;
            }
            if opened != ERROR_SUCCESS {
                return Err(format!("open cursor schemes: {}", opened.0));
            }
            let value = read_registry_string(hkey, name);
            let _ = RegCloseKey(hkey);
            value?
        };
        if !value.is_empty() {
            return parse_cursor_scheme(&value).map(|paths| (paths, index as u32 + 1));
        }
    }
    Err(format!("scheme {name:?} not found"))
}

fn parse_cursor_scheme(value: &str) -> Result<Vec<String>, String> {
    // Built-in schemes may append an indirect display-name resource after
    // the paths. It is metadata, never a cursor (also on older 15-role schemes).
    let fields: Vec<&str> = value
        .split(',')
        .map(str::trim)
        .take_while(|field| !field.starts_with('@'))
        .collect();
    if fields.len() < LEGACY_CURSOR_COUNT {
        return Err(format!("cursor scheme defines only {} roles", fields.len()));
    }
    Ok(SCHEME_TO_SNAPSHOT
        .iter()
        .map(|&index| fields.get(index).copied().unwrap_or_default().to_string())
        .collect())
}

fn apply_cursor_scheme(name: &str) -> Result<(), String> {
    let (paths, _) = read_cursor_scheme(name)?;
    write_cursor_state(&paths, name)
}

/// Writes the full per-role cursor state and refreshes the live cursors:
/// the shared sink for scheme application, snapshot restore, and defaults.
fn write_cursor_state(paths: &[String], scheme_name: &str) -> Result<(), String> {
    crate::background_io::ensure_running()?;
    if paths.len() != LEGACY_CURSOR_COUNT && paths.len() != CURSOR_NAMES.len() {
        return Err(format!(
            "cursor state defines {} of {} roles",
            paths.len(),
            CURSOR_NAMES.len()
        ));
    }
    unsafe {
        let cursors = wide(CURSORS_KEY);
        let mut target = HKEY::default();
        let opened = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(cursors.as_ptr()),
            None,
            KEY_WRITE,
            &mut target,
        );
        if opened != ERROR_SUCCESS {
            return Err(format!("open Cursors: {}", opened.0));
        }
        // The unnamed value, not a literal "(Scheme Default)" value, is
        // Windows' selected scheme marker.
        let mut failed = Vec::new();
        for (value, path) in CURSOR_NAMES.iter().zip(paths) {
            let wide_value = wide(value);
            let wide_path = wide(path);
            let bytes: Vec<u8> = wide_path
                .iter()
                .flat_map(|u| u.to_le_bytes())
                .chain([0u8, 0])
                .collect();
            let status = RegSetValueExW(
                target,
                PCWSTR(wide_value.as_ptr()),
                None,
                windows::Win32::System::Registry::REG_EXPAND_SZ,
                Some(&bytes),
            );
            if status != ERROR_SUCCESS {
                failed.push(format!("{value}:{}", status.0));
            }
        }
        let name_wide = wide(scheme_name);
        let name_bytes: Vec<u8> = name_wide
            .iter()
            .flat_map(|u| u.to_le_bytes())
            .chain([0u8, 0])
            .collect();
        let status = RegSetValueExW(
            target,
            PCWSTR::null(),
            None,
            windows::Win32::System::Registry::REG_SZ,
            Some(&name_bytes),
        );
        if status != ERROR_SUCCESS {
            failed.push(format!("scheme name:{}", status.0));
        }
        let source = if scheme_name.is_empty() {
            0u32
        } else {
            read_cursor_scheme(scheme_name).map_or(0, |(_, source)| source)
        };
        let status = RegSetValueExW(
            target,
            PCWSTR(wide("Scheme Source").as_ptr()),
            None,
            windows::Win32::System::Registry::REG_DWORD,
            Some(&source.to_le_bytes()),
        );
        if status != ERROR_SUCCESS {
            failed.push(format!("scheme source:{}", status.0));
        }
        let _ = RegCloseKey(target);
        if !failed.is_empty() {
            return Err(failed.join(", "));
        }
        let ok = windows::Win32::UI::WindowsAndMessaging::SystemParametersInfoW(
            windows::Win32::UI::WindowsAndMessaging::SPI_SETCURSORS,
            0,
            None,
            windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        if ok.is_ok() {
            Ok(())
        } else {
            Err("SystemParametersInfoW(SPI_SETCURSORS) rejected the request".into())
        }
    }
}

// ============ Appearance restore: pre-switch snapshot + defaults ============

/// The system default cursor set: per-role REG_EXPAND_SZ paths as shipped
/// with a fresh Windows profile (CURSOR_NAMES order).
const DEFAULT_CURSOR_PATHS: [&str; 17] = [
    "%SystemRoot%\\cursors\\aero_arrow.cur",
    "%SystemRoot%\\cursors\\aero_helpsel.cur",
    "%SystemRoot%\\cursors\\aero_working.ani",
    "%SystemRoot%\\cursors\\aero_busy.ani",
    "%SystemRoot%\\cursors\\aero_pen.cur",
    "%SystemRoot%\\cursors\\aero_unavail.cur",
    "%SystemRoot%\\cursors\\aero_ns.cur",
    "%SystemRoot%\\cursors\\aero_ew.cur",
    "%SystemRoot%\\cursors\\aero_cross.cur",
    "%SystemRoot%\\cursors\\aero_ibeam.cur",
    "%SystemRoot%\\cursors\\aero_nwse.cur",
    "%SystemRoot%\\cursors\\aero_nesw.cur",
    "%SystemRoot%\\cursors\\aero_move.cur",
    "%SystemRoot%\\cursors\\aero_up.cur",
    "%SystemRoot%\\cursors\\aero_link.cur",
    "",
    "",
];

/// Fill-position names in DESKTOP_WALLPAPER_POSITION order.
const WALLPAPER_POSITIONS: [&str; 6] = ["center", "tile", "stretch", "fit", "fill", "span"];

/// Parsed wallpaper snapshot. Slideshow and Spotlight setups degrade to
/// the current frame as a static picture: Windows exposes no API to re-arm
/// either mode, so restore is honestly "the picture of that moment".
#[derive(Clone, Debug, PartialEq)]
enum WallpaperSnapshot {
    Solid(u8, u8, u8),
    Picture {
        position: String,
        pairs: Vec<(String, String)>,
    },
}

impl WallpaperSnapshot {
    /// Tab-separated encoding: paths and monitor device paths never contain
    /// tabs, so no escaping is needed.
    fn encode(&self) -> String {
        match self {
            WallpaperSnapshot::Solid(r, g, b) => format!("solid\t{r}\t{g}\t{b}"),
            WallpaperSnapshot::Picture { position, pairs } => {
                let mut parts = vec!["picture".to_string(), position.clone()];
                for (device, image) in pairs {
                    parts.push(device.clone());
                    parts.push(image.clone());
                }
                parts.join("\t")
            }
        }
    }

    fn decode(encoded: &str) -> Option<Self> {
        let mut parts = encoded.split('\t');
        match parts.next()? {
            "solid" => {
                let r: u8 = parts.next()?.parse().ok()?;
                let g: u8 = parts.next()?.parse().ok()?;
                let b: u8 = parts.next()?.parse().ok()?;
                Some(WallpaperSnapshot::Solid(r, g, b))
            }
            "picture" => {
                let position = parts.next()?.to_string();
                let mut pairs = Vec::new();
                while let Some(device) = parts.next() {
                    let image = parts.next()?;
                    if !device.is_empty() && !image.is_empty() {
                        pairs.push((device.to_string(), image.to_string()));
                    }
                }
                if pairs.is_empty() {
                    return None;
                }
                Some(WallpaperSnapshot::Picture { position, pairs })
            }
            _ => None,
        }
    }
}

/// The original 15-role format remains readable. New snapshots additionally
/// preserve Pin/Person; the existing role order is unchanged.
fn parse_cursor_snapshot(encoded: &str) -> Option<(String, Vec<String>)> {
    let parts: Vec<&str> = encoded.split('\t').collect();
    if parts.len() != LEGACY_CURSOR_COUNT + 1 && parts.len() != CURSOR_NAMES.len() + 1 {
        return None;
    }
    Some((
        parts[0].to_string(),
        parts[1..].iter().map(|p| p.to_string()).collect(),
    ))
}

/// One-shot STA guard: CoInitializeEx balanced by CoUninitialize.
struct ComApartment(());

impl ComApartment {
    fn enter() -> Result<Self, String> {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr.is_ok() {
            Ok(ComApartment(()))
        } else {
            Err(format!("CoInitializeEx failed: {hr}"))
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn desktop_wallpaper() -> Result<(ComApartment, IDesktopWallpaper), String> {
    let apartment = ComApartment::enter()?;
    let api = unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL) }
        .map_err(|error| format!("DesktopWallpaper COM: {error}"))?;
    Ok((apartment, api))
}

/// Reads a CoTaskMem-allocated wide string, then frees it.
fn take_pwstr(pw: windows::core::PWSTR) -> Option<String> {
    unsafe {
        if pw.is_null() {
            return None;
        }
        let mut len = 0usize;
        while *pw.as_ptr().add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(pw.as_ptr(), len));
        CoTaskMemFree(Some(pw.as_ptr().cast()));
        Some(text)
    }
}

/// Captures the current wallpaper state. A setup without any per-monitor
/// image is a solid-color background.
fn snapshot_wallpaper() -> Result<String, String> {
    let (_apartment, api) = desktop_wallpaper()?;
    unsafe {
        let count = api
            .GetMonitorDevicePathCount()
            .map_err(|error| format!("monitor count: {error}"))?;
        if count == 0 {
            return Err("no monitors available for wallpaper snapshot".into());
        }
        let mut pairs = Vec::new();
        for index in 0..count {
            let device = api
                .GetMonitorDevicePathAt(index)
                .map_err(|error| format!("monitor {index}: {error}"))?;
            let device = take_pwstr(device)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("monitor {index} has no device path"))?;
            let device_z = wide(&device);
            // A successful empty string means solid color; an HRESULT failure
            // must never be turned into a successful, permanently saved snapshot.
            let image = api
                .GetWallpaper(PCWSTR(device_z.as_ptr()))
                .map_err(|error| format!("wallpaper on {device}: {error}"))?;
            let image = take_pwstr(image)
                .ok_or_else(|| format!("wallpaper on {device}: missing result"))?;
            if !image.is_empty() {
                pairs.push((device, image));
            }
        }
        let snapshot = if pairs.is_empty() {
            let color = api
                .GetBackgroundColor()
                .map_err(|error| format!("background color: {error}"))?;
            WallpaperSnapshot::Solid(
                (color.0 & 0xFF) as u8,
                ((color.0 >> 8) & 0xFF) as u8,
                ((color.0 >> 16) & 0xFF) as u8,
            )
        } else {
            if pairs.len() != count as usize {
                // The existing snapshot format cannot preserve a mixture of
                // solid-color and picture monitors. Leave the original intact.
                return Err("cannot snapshot mixed solid-color and picture backgrounds".into());
            }
            let position = api
                .GetPosition()
                .map_err(|error| format!("wallpaper position: {error}"))?;
            let position = WALLPAPER_POSITIONS
                .get(position.0 as usize)
                .ok_or_else(|| "unknown wallpaper position".to_string())?;
            WallpaperSnapshot::Picture {
                position: position.to_string(),
                pairs,
            }
        };
        Ok(snapshot.encode())
    }
}

/// Applies a parsed wallpaper snapshot. Monitors that no longer exist and
/// images that vanished are skipped, but reported as an incomplete restore.
fn apply_wallpaper_snapshot(snapshot: &WallpaperSnapshot) -> Result<(), String> {
    crate::background_io::ensure_running()?;
    let (_apartment, api) = desktop_wallpaper()?;
    match snapshot {
        WallpaperSnapshot::Solid(r, g, b) => unsafe {
            api.SetWallpaper(PCWSTR::null(), PCWSTR::null())
                .map_err(|error| format!("clear wallpaper: {error}"))?;
            let color = COLORREF((*r as u32) | ((*g as u32) << 8) | ((*b as u32) << 16));
            api.SetBackgroundColor(color)
                .map_err(|error| format!("background color: {error}"))?;
            Ok(())
        },
        WallpaperSnapshot::Picture { position, pairs } => {
            let mut applied = 0usize;
            let mut skipped = Vec::new();
            for (device, image) in pairs {
                if let Err(error) = crate::background_io::require_file(std::path::Path::new(image))
                {
                    skipped.push(error);
                    continue;
                }
                crate::background_io::ensure_running()?;
                let device_z = wide(device);
                let image_z = wide(image);
                if unsafe { api.SetWallpaper(PCWSTR(device_z.as_ptr()), PCWSTR(image_z.as_ptr())) }
                    .is_ok()
                {
                    applied += 1;
                } else {
                    skipped.push(device.clone());
                }
            }
            if let Some(index) = WALLPAPER_POSITIONS
                .iter()
                .position(|name| *name == *position)
            {
                unsafe { api.SetPosition(DESKTOP_WALLPAPER_POSITION(index as i32)) }
                    .map_err(|error| format!("wallpaper position: {error}"))?;
            }
            if applied == 0 {
                return Err(format!(
                    "no monitor restorable (skipped: {})",
                    skipped.join(", ")
                ));
            }
            if !skipped.is_empty() {
                return Err(format!(
                    "restored {applied} monitor(s); could not restore: {}",
                    skipped.join(", ")
                ));
            }
            Ok(())
        }
    }
}

/// Captures the live per-role cursor state, including the picker's
/// scheme-name marker (empty when no scheme is selected).
pub fn snapshot_cursor() -> Result<String, String> {
    unsafe {
        let cursors = wide(CURSORS_KEY);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(cursors.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return Err("open Cursors for snapshot".into());
        }
        let snapshot = (|| {
            let mut parts = vec![read_registry_string(hkey, "")?];
            for name in CURSOR_NAMES {
                parts.push(read_registry_string(hkey, name)?);
            }
            Ok(parts.join("\t"))
        })();
        let _ = RegCloseKey(hkey);
        snapshot
    }
}

/// Reads one REG_SZ/REG_EXPAND_SZ value (empty when missing, as for a default
/// cursor). Other read errors abort capture. EXPAND_SZ stays unexpanded, preserving %SystemRoot%
/// forms for faithful re-writing.
fn read_registry_string(hkey: HKEY, name: &str) -> Result<String, String> {
    unsafe {
        let wide_name = wide(name);
        let mut kind = windows::Win32::System::Registry::REG_VALUE_TYPE(0);
        let mut size = 0u32;
        let query = windows::Win32::System::Registry::RegQueryValueExW(
            hkey,
            PCWSTR(wide_name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        );
        if query == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
            return Ok(String::new());
        }
        if query != ERROR_SUCCESS {
            return Err(format!("read cursor {name}: {}", query.0));
        }
        if !matches!(
            kind,
            windows::Win32::System::Registry::REG_SZ
                | windows::Win32::System::Registry::REG_EXPAND_SZ
        ) {
            return Err(format!("cursor {name} is not a string"));
        }
        let mut buffer = vec![0u8; size as usize];
        let read = windows::Win32::System::Registry::RegQueryValueExW(
            hkey,
            PCWSTR(wide_name.as_ptr()),
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr()),
            Some(&mut size),
        );
        if read != ERROR_SUCCESS
            || !matches!(
                kind,
                windows::Win32::System::Registry::REG_SZ
                    | windows::Win32::System::Registry::REG_EXPAND_SZ
            )
        {
            return Err(format!("read cursor {name}: {}", read.0));
        }
        buffer.truncate(size as usize);
        if !buffer.len().is_multiple_of(2) {
            return Err(format!("cursor {name} contains incomplete UTF-16"));
        }
        let units: Vec<u16> = buffer
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .collect();
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        String::from_utf16(&units[..end]).map_err(|error| format!("cursor {name}: {error}"))
    }
}

/// True when at least one pre-change snapshot exists (UI enable state).
pub fn has_restore_snapshot() -> bool {
    crate::cfg_map(|c| !c.theme_restore_wallpaper.is_empty() || !c.theme_restore_cursor.is_empty())
}

/// Per-side restore outcome: None = that side has no snapshot (nothing was
/// attempted); Some(Err) = the restore was attempted and failed.
pub type RestoreSides = (Option<Result<(), String>>, Option<Result<(), String>>);

/// Restores the state captured before the first day/night application.
pub fn restore_previous() -> RestoreSides {
    let (wallpaper, cursor) = crate::cfg_map(|c| {
        (
            c.theme_restore_wallpaper.clone(),
            c.theme_restore_cursor.clone(),
        )
    });
    let wallpaper = (!wallpaper.is_empty()).then(|| {
        WallpaperSnapshot::decode(&wallpaper)
            .ok_or_else(|| "wallpaper snapshot malformed".to_string())
            .and_then(|snapshot| apply_wallpaper_snapshot(&snapshot))
    });
    let cursor = (!cursor.is_empty()).then(|| {
        parse_cursor_snapshot(&cursor)
            .ok_or_else(|| "cursor snapshot malformed".to_string())
            .and_then(|(name, paths)| write_cursor_state(&paths, &name))
    });
    (wallpaper, cursor)
}

/// Factory defaults: the Windows image wallpaper plus the aero cursor set.
pub fn restore_default() -> RestoreSides {
    (
        Some(apply_default_wallpaper()),
        Some(apply_default_cursor()),
    )
}

fn apply_default_wallpaper() -> Result<(), String> {
    use std::os::windows::ffi::OsStringExt;
    let mut directory = vec![0u16; 260];
    let len = loop {
        let len = unsafe {
            windows::Win32::System::SystemInformation::GetSystemWindowsDirectoryW(Some(
                &mut directory,
            ))
        } as usize;
        if len == 0 {
            return Err(format!(
                "Windows directory: {}",
                std::io::Error::last_os_error()
            ));
        }
        if len < directory.len() {
            break len;
        }
        directory.resize(len + 1, 0);
    };
    let image = std::path::PathBuf::from(std::ffi::OsString::from_wide(&directory[..len]))
        .join(r"Web\Wallpaper\Windows\img0.jpg");
    crate::background_io::require_file(&image)?;
    crate::background_io::ensure_running()?;
    let (_apartment, api) = desktop_wallpaper()?;
    let image_z = wide(&image.to_string_lossy());
    unsafe {
        api.SetWallpaper(PCWSTR::null(), PCWSTR(image_z.as_ptr()))
            .map_err(|error| format!("default wallpaper: {error}"))?;
        api.SetPosition(DWPOS_FILL)
            .map_err(|error| format!("wallpaper position: {error}"))?;
    }
    Ok(())
}

fn apply_default_cursor() -> Result<(), String> {
    let paths: Vec<String> = DEFAULT_CURSOR_PATHS.iter().map(|p| p.to_string()).collect();
    write_cursor_state(&paths, "")
}

/// Captures the pre-switch state right before the first wallpaper/cursor
/// modification and keeps it (sticky) until the config is hand-cleared:
/// the data source for the appearance page's restore action.
/// A side may change only after its original state is safely persisted.
fn ensure_restore_snapshot(
    wallpaper_change: bool,
    cursor_change: bool,
) -> (Result<(), String>, Result<(), String>) {
    let (wallpaper_exists, original_cursor) = crate::cfg_map(|c| {
        (
            !c.theme_restore_wallpaper.is_empty(),
            c.theme_restore_cursor.clone(),
        )
    });
    let legacy_cursor = parse_cursor_snapshot(&original_cursor)
        .filter(|(_, paths)| paths.len() == LEGACY_CURSOR_COUNT);
    let want_wallpaper = wallpaper_change && !wallpaper_exists;
    let want_cursor = cursor_change && (original_cursor.is_empty() || legacy_cursor.is_some());
    if !want_wallpaper && !want_cursor {
        return (Ok(()), Ok(()));
    }
    let wallpaper = want_wallpaper
        .then(|| crate::background_io::read("wallpaper snapshot", snapshot_wallpaper))
        .transpose();
    let cursor = want_cursor
        .then(|| {
            crate::background_io::read("cursor snapshot", move || {
                let current = snapshot_cursor()?;
                if let Some((name, mut paths)) = legacy_cursor {
                    // Older releases never changed these two roles. Capture
                    // them before this release can, retaining all old originals.
                    let (_, current_paths) =
                        parse_cursor_snapshot(&current).ok_or("cursor snapshot malformed")?;
                    paths.extend_from_slice(&current_paths[LEGACY_CURSOR_COUNT..]);
                    Ok(format!("{name}\t{}", paths.join("\t")))
                } else {
                    Ok(current)
                }
            })
        })
        .transpose();
    if let Err(error) = crate::background_io::ensure_running() {
        return (Err(error.clone()), Err(error));
    }
    let saved = if matches!(wallpaper, Ok(Some(_))) || matches!(cursor, Ok(Some(_))) {
        crate::edit_config(|c| {
            if c.theme_restore_wallpaper.is_empty()
                && let Ok(Some(value)) = &wallpaper
            {
                c.theme_restore_wallpaper = value.clone();
            }
            if (c.theme_restore_cursor.is_empty() || c.theme_restore_cursor == original_cursor)
                && let Ok(Some(value)) = &cursor
            {
                c.theme_restore_cursor = value.clone();
            }
        })
        .map_err(|error| format!("restore snapshot save failed: {error}"))
    } else {
        Ok(())
    };
    let ready = |snapshot: Result<Option<String>, String>| {
        snapshot.and_then(|value| {
            if value.is_some() {
                saved.clone()
            } else {
                Ok(())
            }
        })
    };
    (ready(wallpaper), ready(cursor))
}

#[cfg(test)]
mod restore_snapshot_tests {
    use super::*;

    #[test]
    fn wallpaper_snapshot_round_trips() {
        let solid = WallpaperSnapshot::Solid(10, 20, 30);
        assert_eq!(WallpaperSnapshot::decode(&solid.encode()), Some(solid));
        let picture = WallpaperSnapshot::Picture {
            position: "fit".into(),
            pairs: vec![
                (
                    "\\\\?\\DISPLAY#ABC#{guid}".into(),
                    "C:\\pics\\wall paper;1.jpg".into(),
                ),
                ("\\\\?\\DISPLAY#DEF#{guid}".into(), "D:\\other.png".into()),
            ],
        };
        assert_eq!(WallpaperSnapshot::decode(&picture.encode()), Some(picture));
    }

    #[test]
    fn wallpaper_snapshot_decode_rejects_junk() {
        assert!(WallpaperSnapshot::decode("").is_none());
        assert!(WallpaperSnapshot::decode("nonsense").is_none());
        assert!(WallpaperSnapshot::decode("solid\t1\t2").is_none());
        assert!(WallpaperSnapshot::decode("solid\t256\t0\t0").is_none());
        // A dangling monitor path without its image is malformed.
        assert!(WallpaperSnapshot::decode("picture\tfill\t\\\\?\\DISPLAY#A#{g}").is_none());
        // No usable pairs.
        assert!(WallpaperSnapshot::decode("picture\tfill").is_none());
    }

    #[test]
    fn cursor_snapshot_round_trips_with_empty_roles() {
        for count in [LEGACY_CURSOR_COUNT, CURSOR_NAMES.len()] {
            let parts: Vec<String> = std::iter::once("My Scheme".to_string())
                .chain((0..count).map(|index| {
                    if index % 2 == 0 {
                        format!("%SystemRoot%\\cursors\\aero{index}.cur")
                    } else {
                        String::new()
                    }
                }))
                .collect();
            let encoded = parts.join("\t");
            let Some((name, paths)) = parse_cursor_snapshot(&encoded) else {
                panic!("valid snapshot must parse");
            };
            assert_eq!(name, "My Scheme");
            assert_eq!(paths.len(), count);
            assert_eq!(paths[1], "");
            assert_eq!(format!("{name}\t{}", paths.join("\t")), encoded);
        }
    }

    #[test]
    fn cursor_snapshot_decode_rejects_wrong_field_count() {
        assert!(parse_cursor_snapshot("").is_none());
        assert!(parse_cursor_snapshot("only\tthree\tfields").is_none());
        let short: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        assert!(parse_cursor_snapshot(&short.join("\t")).is_none());
    }
    #[test]
    fn cursor_scheme_maps_windows_order_and_ignores_display_resource() {
        let scheme = "%SystemRoot%\\cursors\\aero_arrow.cur,help.cur,working.ani,busy.ani,cross.cur,beam.cur,pen.cur,no.cur,ns.cur,ew.cur,nwse.cur,nesw.cur,move.cur,up.cur,link.cur,pin.cur,person.cur,@%SystemRoot%\\system32\\main.cpl,-1020";
        let paths = parse_cursor_scheme(scheme).unwrap();
        let role =
            |name| paths[CURSOR_NAMES.iter().position(|role| *role == name).unwrap()].as_str();
        assert_eq!(role("Arrow"), "%SystemRoot%\\cursors\\aero_arrow.cur");
        assert_eq!(role("Crosshair"), "cross.cur");
        assert_eq!(role("IBeam"), "beam.cur");
        assert_eq!(role("NWPen"), "pen.cur");
        assert_eq!(role("No"), "no.cur");
        assert_eq!(role("SizeNS"), "ns.cur");
        assert_eq!(role("SizeWE"), "ew.cur");
        // The NESW/NWSE pair is the one a mapping can silently swap.
        assert_eq!(role("SizeNESW"), "nesw.cur");
        assert_eq!(role("SizeNWSE"), "nwse.cur");
        assert_eq!(role("Hand"), "link.cur");
        assert_eq!(role("Pin"), "pin.cur");
        assert_eq!(role("Person"), "person.cur");
    }

    #[test]
    fn cursor_scheme_accepts_legacy_roles_and_rejects_incomplete_data() {
        let paths =
            parse_cursor_scheme(",,,,,,,,,,,,,,,@%SystemRoot%\\system32\\main.cpl,-1020").unwrap();
        assert_eq!(paths, vec![String::new(); CURSOR_NAMES.len()]);
        assert!(parse_cursor_scheme("arrow.cur,help.cur").is_err());
        assert!(parse_cursor_scheme("").is_err());
    }
}
