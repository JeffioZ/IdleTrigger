//! Day/Night theme engine: scheduled light/dark switching (fixed times or
//! sunrise/sunset), Windows Personalize registry writes, and battery-based
//! dark preference, asynchronous location lookup, and fullscreen pause.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM, WPARAM};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, RegCloseKey, RegOpenKeyExW,
    RegSetValueExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};
use windows::core::PCWSTR;

static THEME_THREAD_RUNNING: AtomicBool = AtomicBool::new(false);
static THEME_OPERATION: Mutex<()> = Mutex::new(());
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// Coalesces configuration, power, and location changes without doing work on the UI thread.
pub fn wake() {
    crate::theme_recovery::changed();
    *WAKE.0.lock().unwrap() = true;
    WAKE.1.notify_one();
}
/// Manual mode and its absolute local-time expiration minute.
static MANUAL_OVERRIDE: Mutex<Option<(bool, i64)>> = Mutex::new(None);

const PERSONALIZE_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
const BREED: &str = "SystemUsesLightTheme";
const APP_BREED: &str = "AppsUseLightTheme";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Local time snapshot (minutes + weekday).
pub struct LocalTime {
    pub minutes: i32,
    pub absolute_minutes: i64,
}

fn local_time() -> LocalTime {
    unsafe {
        let st = windows::Win32::System::SystemInformation::GetLocalTime();
        let mut ft = windows::Win32::Foundation::FILETIME::default();
        let _ = windows::Win32::System::Time::SystemTimeToFileTime(&st, &mut ft);
        LocalTime {
            minutes: st.wHour as i32 * 60 + st.wMinute as i32,
            absolute_minutes: (((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64) as i64
                / 600_000_000,
        }
    }
}

/// NOAA solar approximation for sunrise/sunset in minutes-from-midnight.
/// Inputs: latitude/longitude degrees, day-of-year. Returns (sunrise, sunset).
/// Accuracy is ±5 minutes — adequate for theme switching.
pub fn solar_times(lat: f64, lon: f64, day_of_year: i32) -> Option<(i32, i32)> {
    if !lat.is_finite()
        || !lon.is_finite()
        || lat.abs() > 90.0
        || lon.abs() > 180.0
        || !(1..=366).contains(&day_of_year)
    {
        return None;
    }
    let gamma = 2.0 * std::f64::consts::PI / 365.0 * (day_of_year as f64 - 1.0 + 0.5);
    let eq_time = 229.18
        * (0.000075 + 0.001868 * gamma.cos()
            - 0.032077 * gamma.sin()
            - 0.014615 * (2.0 * gamma).cos()
            - 0.040849 * (2.0 * gamma).sin());
    let decl = 0.006918 - 0.399912 * gamma.cos() + 0.070257 * gamma.sin()
        - 0.006758 * (2.0 * gamma).cos()
        + 0.000907 * (2.0 * gamma).sin()
        - 0.002697 * (3.0 * gamma).cos()
        + 0.00148 * (3.0 * gamma).sin();
    let lat_rad = lat.to_radians();

    let hour_angle_deg = ((std::f64::consts::PI / 180.0 * 90.833).cos()
        / (lat_rad.cos() * decl.cos())
        - lat_rad.tan() * decl.tan())
    .acos();
    if hour_angle_deg.is_nan() {
        return None; // polar day/night
    }
    let day_len_min = 4.0 * hour_angle_deg.to_degrees();
    // Solar noon is UTC minutes; Go adds the local zone offset (incl. DST)
    // before use — without it the schedule shifts by the whole timezone.
    let solar_noon_utc = 720.0 - 4.0 * lon - eq_time;
    let offset = local_utc_offset_minutes() as f64;
    let solar_noon = solar_noon_utc + offset;
    let sunrise = solar_noon - day_len_min;
    let sunset = solar_noon + day_len_min;
    // Clamp into the day (Go wrap loops).
    Some((
        (sunrise.round() as i32).rem_euclid(1440),
        (sunset.round() as i32).rem_euclid(1440),
    ))
}

/// Local-zone UTC offset in minutes, DST included (Go time.Time.Zone()).
fn local_utc_offset_minutes() -> i32 {
    use windows::Win32::System::SystemServices::TIME_ZONE_ID_DAYLIGHT;
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    unsafe {
        let mut tzi = TIME_ZONE_INFORMATION::default();
        let state = GetTimeZoneInformation(&mut tzi);
        let bias = tzi.Bias
            + if state == TIME_ZONE_ID_DAYLIGHT {
                tzi.DaylightBias
            } else if state == 1 {
                tzi.StandardBias
            } else {
                0
            };
        -bias
    }
}

/// Nonblocking location resolution: cached IP, timezone, UTC offset, default.
pub fn location(ip_enabled: bool) -> (f64, f64, &'static str) {
    if ip_enabled && let Some((lat, lon)) = crate::iplocate::request() {
        return (lat, lon, "theme_location_ip");
    }
    unsafe {
        let mut zone = windows::Win32::System::Time::DYNAMIC_TIME_ZONE_INFORMATION::default();
        let state = windows::Win32::System::Time::GetDynamicTimeZoneInformation(&mut zone);
        let len = zone
            .TimeZoneKeyName
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(zone.TimeZoneKeyName.len());
        let key = String::from_utf16_lossy(&zone.TimeZoneKeyName[..len]);
        let coordinates = match key.as_str() {
            "China Standard Time" => Some((39.9, 116.4)),
            "Taipei Standard Time" => Some((25.0, 121.5)),
            "Tokyo Standard Time" => Some((35.7, 139.7)),
            "Korea Standard Time" => Some((37.6, 127.0)),
            "Singapore Standard Time" => Some((1.35, 103.8)),
            "India Standard Time" => Some((28.6, 77.2)),
            "W. Europe Standard Time" => Some((52.5, 13.4)),
            "GMT Standard Time" => Some((51.5, -0.1)),
            "Central Europe Standard Time" => Some((48.2, 16.4)),
            "E. Europe Standard Time" => Some((50.4, 30.5)),
            "Russian Standard Time" => Some((55.8, 37.6)),
            "Eastern Standard Time" => Some((40.7, -74.0)),
            "Central Standard Time" => Some((41.9, -87.6)),
            "Mountain Standard Time" => Some((33.4, -112.0)),
            "Pacific Standard Time" => Some((34.0, -118.2)),
            "Alaskan Standard Time" => Some((61.2, -149.9)),
            "Hawaiian Standard Time" => Some((21.3, -157.8)),
            "E. South America Standard Time" => Some((-23.5, -46.6)),
            "Atlantic Standard Time" => Some((-34.6, -58.4)),
            "AUS Eastern Standard Time" => Some((-33.9, 151.2)),
            "AUS Central Standard Time" => Some((-34.9, 138.6)),
            "New Zealand Standard Time" => Some((-36.8, 174.8)),
            _ => None,
        };
        if let Some((lat, lon)) = coordinates {
            return (lat, lon, "theme_location_timezone");
        }
        if state <= 2 {
            return (
                35.0,
                (local_utc_offset_minutes() as f64 / 4.0).clamp(-180.0, 180.0),
                "theme_location_utc_offset",
            );
        }
        (39.9, 116.4, "theme_location_default")
    }
}

fn next_transition(absolute: i64, minutes: i32, window: Option<(i32, i32)>) -> i64 {
    let Some((light, dark)) = window else {
        return absolute + (1440 - minutes) as i64;
    };
    let delay = |boundary: i32| {
        let delta = (boundary - minutes).rem_euclid(1440);
        if delta == 0 { 1440 } else { delta }
    };
    absolute + delay(light).min(delay(dark)) as i64
}

fn day_of_year_now() -> i32 {
    unsafe {
        let st = windows::Win32::System::SystemInformation::GetLocalTime();
        // Simple cumulative days-per-month (non-leap approximation + Feb 29 bump).
        let leap = (st.wYear.is_multiple_of(4) && !st.wYear.is_multiple_of(100))
            || st.wYear.is_multiple_of(400);
        let mut doy = st.wDay as i32;
        for m in 1..st.wMonth {
            doy += match m {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 => {
                    if leap {
                        29
                    } else {
                        28
                    }
                }
                _ => 0,
            };
        }
        doy
    }
}

/// Computes the light window in minutes (start, end). Returns None when the
/// schedule cannot be determined.
pub fn solar_window(ip_enabled: bool) -> Option<(i32, i32)> {
    let (lat, lon, _) = location(ip_enabled);
    solar_times(lat, lon, day_of_year_now())
}

pub fn light_window() -> Option<(i32, i32)> {
    let (mode, light_str, dark_str, ip_enabled) = crate::cfg_map(|c| {
        (
            c.theme_mode.clone(),
            c.theme_light_time.clone(),
            c.theme_dark_time.clone(),
            c.theme_ip_location_enabled,
        )
    });
    if mode == "fixed" {
        let parse = |v: &str| -> Option<i32> {
            let h: i32 = v.get(..2)?.parse().ok()?;
            let m: i32 = v.get(3..5)?.parse().ok()?;
            Some(h * 60 + m)
        };
        return Some((parse(&light_str)?, parse(&dark_str)?));
    }
    solar_window(ip_enabled).or_else(|| {
        let parse = |v: &str| {
            v.get(..2)?
                .parse::<i32>()
                .ok()
                .zip(v.get(3..5)?.parse::<i32>().ok())
                .map(|(h, m)| h * 60 + m)
        };
        Some((parse(&light_str)?, parse(&dark_str)?))
    })
}
/// Whether the schedule says dark right now.
fn scheduled_dark() -> Option<bool> {
    let light = light_window()?;
    let now = local_time();
    let (start, end) = light;
    if start <= end {
        Some(!(now.minutes >= start && now.minutes < end))
    } else {
        // Window crossing midnight.
        Some(!(now.minutes >= start || now.minutes < end))
    }
}

/// Writes both Personalize values and notifies the shell.
fn apply_windows_theme(dark: bool) -> Result<(), String> {
    unsafe {
        let key = wide(PERSONALIZE_KEY);
        let mut hkey = HKEY::default();
        let opened = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            KEY_READ | KEY_WRITE,
            &mut hkey,
        );
        if opened != ERROR_SUCCESS {
            return Err(format!("open Personalize: {}", opened.0));
        }
        let value = (!dark) as u32; // light theme flag = !dark
        let bytes = value.to_le_bytes();
        let mut result = ERROR_SUCCESS;
        for name in [BREED, APP_BREED] {
            let name_w = wide(name);
            // Personalize values are REG_DWORD (Go SetDWordValue) — writing
            // any other type makes the system theme switch silently fail.
            result = RegSetValueExW(hkey, PCWSTR(name_w.as_ptr()), None, REG_DWORD, Some(&bytes));
            if result != ERROR_SUCCESS {
                break;
            }
        }
        let _ = RegCloseKey(hkey);
        // Notify running apps + the shell so they repaint.
        if result != ERROR_SUCCESS {
            return Err(format!("write Personalize: {}", result.0));
        }
        Ok(())
    }
}

fn notify_theme() {
    unsafe {
        let setting = wide("ImmersiveColorSet");
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(setting.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            1000,
            None,
        );
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            Some(crate::hwnd(&crate::HIDDEN)),
            windows::Win32::UI::WindowsAndMessaging::WM_THEMECHANGED,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

/// Poll target: applies the theme and updates our own follow layer.
fn tick() {
    let generation = crate::theme_recovery::generation();
    let enabled = crate::cfg_map(|c| c.theme_switch_enabled);
    if !enabled {
        return;
    }
    // Fullscreen / presentation pause, then windowed GPU activity (Go
    // DetectThemeSwitchPause: 4 levels, GPU is the windowed-game check).
    if crate::cfg_map(|c| c.theme_skip_fullscreen) && crate::display::foreground_is_fullscreen() {
        return;
    }
    if crate::cfg_map(|c| c.theme_skip_fullscreen) && crate::gpu_activity::foreground_gpu_active() {
        crate::log_line("theme engine: paused by foreground GPU activity");
        return;
    }
    let operation = THEME_OPERATION.lock().unwrap();
    if !crate::cfg_map(|c| c.theme_switch_enabled) {
        return;
    }
    let manual = {
        let mut manual = MANUAL_OVERRIDE.lock().unwrap();
        if manual.is_some_and(|(_, expiry)| local_time().absolute_minutes >= expiry) {
            *manual = None;
        }
        manual.map(|(dark, _)| dark)
    };
    let target = manual.or_else(scheduled_dark);
    let Some(target) = target else { return };

    // Battery-based dark preference (Go contract: battery → dark).
    let on_ac = crate::ON_AC.load(Ordering::SeqCst);
    let dark_on_battery = crate::cfg_map(|c| c.theme_dark_on_battery);
    let target = if !on_ac && dark_on_battery {
        true
    } else {
        target
    };

    let matches_target = [BREED, APP_BREED]
        .into_iter()
        .all(|name| crate::theme::read_light_preference(name) == Some(!target));
    if matches_target && !crate::theme_recovery::pending() {
        return;
    }
    drop(operation);
    let Some(prepared) = crate::theme_recovery::prepare(generation) else {
        return;
    };
    let operation = THEME_OPERATION.lock().unwrap();
    if !prepared.current() || !crate::cfg_map(|c| c.theme_switch_enabled) {
        return;
    }
    if crate::cfg_map(|c| c.theme_skip_fullscreen) && crate::display::foreground_is_fullscreen() {
        return;
    }
    if !matches_target && let Err(error) = apply_windows_theme(target) {
        crate::log_line(&format!("theme switch failed: {error}"));
        return;
    }
    if !matches_target {
        crate::theme_recovery::transition_applied();
    }
    drop(operation);
    notify_theme();
    std::thread::sleep(Duration::from_millis(1200));
    let operation = THEME_OPERATION.lock().unwrap();
    if prepared.current() && crate::cfg_map(|c| c.theme_switch_enabled) {
        crate::theme_recovery::finish(prepared, !matches_target);
    }
    drop(operation);
    crate::theme_repair::notify_theme_changed();
}

/// Spawns the theme scheduler thread (1-minute cadence is enough).
pub fn spawn() {
    if THEME_THREAD_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("theme-engine".into())
        .spawn(|| {
            loop {
                if crate::EXITING.load(Ordering::SeqCst) {
                    return;
                }
                tick();
                let pending = WAKE.0.lock().unwrap();
                let (mut pending, _) = WAKE
                    .1
                    .wait_timeout_while(pending, Duration::from_secs(60), |pending| !*pending)
                    .unwrap();
                *pending = false;
            }
        })
        .expect("spawn theme engine");
}

/// Sets a manual dark/light override until the next scheduled transition.
pub fn set_manual_override(dark: bool) {
    crate::theme_recovery::changed();
    let operation = THEME_OPERATION.lock().unwrap();
    let now = local_time();
    let expiry = next_transition(now.absolute_minutes, now.minutes, light_window());
    if let Err(error) = apply_windows_theme(dark) {
        drop(operation);
        crate::warn_dialog("", &error);
        return;
    }
    *MANUAL_OVERRIDE.lock().unwrap() = Some((dark, expiry));
    drop(operation);
    notify_theme();
    crate::theme::refresh_from_registry();
    crate::theme::apply_to_all();
    crate::log_line(&format!(
        "theme manual override: {} (until {:02}:{:02})",
        if dark { "dark" } else { "light" },
        (expiry % (24 * 60)) / 60,
        (expiry % (24 * 60)) % 60
    ));
}

/// Clears the manual override (returns control to the schedule).
#[allow(dead_code)]
pub fn clear_manual_override() {
    *MANUAL_OVERRIDE.lock().unwrap() = None;
}

/// The three panel theme buttons: enable is the master toggle; switch/repair
/// act on demand.
pub fn toggle_enabled() {
    if let Err(error) = crate::edit_config(|c| c.theme_switch_enabled = !c.theme_switch_enabled) {
        crate::warn_dialog("", &error);
        return;
    }
    crate::log_line("theme switching toggled");
}

pub fn manual_switch() {
    let current = crate::theme::is_dark();
    set_manual_override(!current);
}

static REPAIR_RUNNING: AtomicBool = AtomicBool::new(false);
static REPAIR_RESULT: Mutex<Option<Result<(), String>>> = Mutex::new(None);

pub fn repair() {
    if REPAIR_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::theme_recovery::changed();
    if let Err(error) = std::thread::Builder::new()
        .name("theme-repair".into())
        .spawn(|| {
            let operation = THEME_OPERATION.lock().unwrap();
            let result = if crate::theme_repair::full_dwm_refresh_available() {
                crate::theme_repair::refresh_dwm_colorization().map_err(|e| e.to_string())
            } else {
                Ok(())
            };
            crate::theme_recovery::manual_repair_completed();
            drop(operation);
            crate::theme_repair::notify_theme_changed();
            *REPAIR_RESULT.lock().unwrap() = Some(result);
            REPAIR_RUNNING.store(false, Ordering::SeqCst);
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    Some(crate::hwnd(&crate::HIDDEN)),
                    crate::WM_REFRESH_UI,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        })
    {
        REPAIR_RUNNING.store(false, Ordering::SeqCst);
        crate::warn_dialog("", &error.to_string());
    }
}

pub fn finish_repair() {
    let result = REPAIR_RESULT.lock().unwrap().take();
    if let Some(result) = result {
        crate::theme::refresh_from_registry();
        crate::theme::apply_to_all();
        match result {
            Ok(()) => crate::log_line("theme repair completed"),
            Err(error) => {
                crate::log_line(&format!("theme repair failed: {error}"));
                crate::warn_dialog("", &crate::t_args("theme_repair_failed", &[&error]));
            }
        }
    }
}
#[cfg(test)]
mod solar_tests {
    use super::*;
    #[test]
    fn equinox_at_equator_has_about_twelve_hours_of_daylight() {
        let (rise, set) = solar_times(0.0, 0.0, 80).unwrap();
        assert!((710..=740).contains(&(set - rise).rem_euclid(1440)));
    }
    #[test]
    fn invalid_coordinates_terminate_and_manual_hold_expires_across_midnight() {
        for (lat, lon) in [
            (f64::NAN, 0.0),
            (0.0, f64::INFINITY),
            (91.0, 0.0),
            (0.0, 181.0),
        ] {
            assert!(solar_times(lat, lon, 80).is_none());
        }
        assert_eq!(
            next_transition(10000, 23 * 60, Some((420, 1140))),
            10000 + 480
        );
        assert_eq!(next_transition(10000, 600, Some((420, 1140))), 10000 + 540);
        assert_eq!(next_transition(10000, 420, Some((420, 1140))), 10000 + 720);
    }
}
