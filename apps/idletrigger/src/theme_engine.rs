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

/// Snoozed automatic switching: an absolute-minute deadline after which the
/// schedule resumes. Runtime-only, so a restart clears it. Manual switches
/// bypass this suppression entirely (they never consult the schedule).
static SNOOZE: Mutex<Option<i64>> = Mutex::new(None);
#[derive(Default)]
struct ManualRequests {
    pending: Option<bool>,
    target: Option<bool>,
}

impl ManualRequests {
    fn toggle(&mut self, current: bool) {
        let target = !self.target.unwrap_or(current);
        self.pending = Some(target);
        self.target = Some(target);
    }

    fn complete(&mut self) {
        if self.pending.is_none() {
            self.target = None;
        }
    }
}

static MANUAL_REQUESTS: Mutex<ManualRequests> = Mutex::new(ManualRequests {
    pending: None,
    target: None,
});
static MANUAL_ERROR: Mutex<Option<String>> = Mutex::new(None);
static THEME_OPERATION: Mutex<()> = Mutex::new(());
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// Coalesces configuration, power, and location changes without doing work on the UI thread.
pub fn wake() {
    crate::theme_recovery::changed();
    *crate::runtime::lock(&WAKE.0) = true;
    WAKE.1.notify_one();
}
/// Manual mode and its absolute local-time expiration minute.
static MANUAL_OVERRIDE: Mutex<Option<(bool, i64)>> = Mutex::new(None);

const BREED: &str = "SystemUsesLightTheme";
const APP_BREED: &str = "AppsUseLightTheme";

use crate::theme::PERSONALIZE_KEY;
use crate::wide;
use idletrigger_core::automation::parse_hhmm;

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
        return Some((parse_hhmm(&light_str)?, parse_hhmm(&dark_str)?));
    }
    solar_window(ip_enabled).or_else(|| Some((parse_hhmm(&light_str)?, parse_hhmm(&dark_str)?)))
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

/// Writes and verifies both Personalize values.
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
        let result = apply_preferences(
            dark,
            |name| {
                let mut value = 0u32;
                let mut kind = REG_DWORD;
                let mut size = 4;
                let status = windows::Win32::System::Registry::RegQueryValueExW(
                    hkey,
                    PCWSTR(wide(name).as_ptr()),
                    None,
                    Some(&mut kind),
                    Some((&mut value as *mut u32).cast()),
                    Some(&mut size),
                );
                if status == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
                    return Ok(None);
                }
                if status != ERROR_SUCCESS || kind != REG_DWORD || size != 4 || value > 1 {
                    return Err(format!(
                        "read Personalize {name}: invalid value or error {}",
                        status.0
                    ));
                }
                Ok(Some(value))
            },
            |name, value| {
                let status = match value {
                    Some(value) => RegSetValueExW(
                        hkey,
                        PCWSTR(wide(name).as_ptr()),
                        None,
                        REG_DWORD,
                        Some(&value.to_le_bytes()),
                    ),
                    None => windows::Win32::System::Registry::RegDeleteValueW(
                        hkey,
                        PCWSTR(wide(name).as_ptr()),
                    ),
                };
                if status == ERROR_SUCCESS
                    || (value.is_none()
                        && status == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND)
                {
                    Ok(())
                } else {
                    Err(format!("write Personalize {name}: {}", status.0))
                }
            },
        );
        let _ = RegCloseKey(hkey);
        result
    }
}

/// Restore attempted values if a write or read-back fails. Untouched values
/// stay untouched, and missing values remain absent after rollback.
fn apply_preferences(
    dark: bool,
    mut read: impl FnMut(&str) -> Result<Option<u32>, String>,
    mut write: impl FnMut(&str, Option<u32>) -> Result<(), String>,
) -> Result<(), String> {
    let names = [BREED, APP_BREED];
    let previous = [read(names[0])?, read(names[1])?];
    let desired = Some(u32::from(!dark));
    let mut attempted = 0;
    let result = (|| {
        for name in names {
            attempted += 1;
            write(name, desired)?;
        }
        for name in names {
            if read(name)? != desired {
                return Err(format!("verify Personalize {name}: value not retained"));
            }
        }
        Ok(())
    })();
    if let Err(mut error) = result {
        for (name, value) in names.into_iter().zip(previous).take(attempted).rev() {
            if let Err(rollback) = write(name, value) {
                error.push_str(&format!("; rollback failed: {rollback}"));
            }
        }
        return Err(error);
    }
    Ok(())
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
    let operation = crate::runtime::lock(&THEME_OPERATION);
    if !crate::cfg_map(|c| c.theme_switch_enabled) {
        return;
    }
    let manual = {
        let mut manual = crate::runtime::lock(&MANUAL_OVERRIDE);
        if manual.is_some_and(|(_, expiry)| local_time().absolute_minutes >= expiry) {
            *manual = None;
        }
        manual.map(|(dark, _)| dark)
    };
    let target = manual.or_else(|| {
        // Snooze suppresses only the schedule; a pending manual request or
        // active manual override still applies.
        if snooze_active() {
            None
        } else {
            scheduled_dark()
        }
    });
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
    let operation = crate::runtime::lock(&THEME_OPERATION);
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
        apply_appearance(target);
        crate::theme_recovery::transition_applied();
    }
    drop(operation);
    notify_theme();
    // Let the theme-change broadcast settle before recovery finishes, but
    // stay interruptible: a queued manual switch must not wait out the full
    // window. The wake flag itself is consumed by the loop's own wait.
    {
        let pending = crate::runtime::lock(&WAKE.0);
        let _ = WAKE
            .1
            .wait_timeout_while(pending, Duration::from_millis(1200), |pending| !*pending);
    }
    let operation = crate::runtime::lock(&THEME_OPERATION);
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
                crate::runtime::catch_and_log("theme-engine", || {
                    let manual = crate::runtime::lock(&MANUAL_REQUESTS).pending.take();
                    if let Some(dark) = manual {
                        let result = set_manual_override(dark);
                        crate::runtime::lock(&MANUAL_REQUESTS).complete();
                        if let Err(error) = result {
                            *crate::runtime::lock(&MANUAL_ERROR) = Some(error);
                            unsafe {
                                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                                    Some(crate::hwnd(&crate::HIDDEN)),
                                    crate::WM_REFRESH_UI,
                                    WPARAM(0),
                                    LPARAM(0),
                                );
                            }
                        } else {
                            notify_theme();
                        }
                    }
                    tick();
                    let pending = crate::runtime::lock(&WAKE.0);
                    // Recover from poisoning like every other lock: a dead
                    // engine thread is worse than one more 60s poll cycle.
                    let (mut pending, _) = WAKE
                        .1
                        .wait_timeout_while(pending, Duration::from_secs(60), |pending| !*pending)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    *pending = false;
                });
            }
        })
        .expect("spawn theme engine");
}

/// Sets a manual dark/light override until the next scheduled transition.
fn set_manual_override(dark: bool) -> Result<(), String> {
    crate::theme_recovery::changed();
    let operation = crate::runtime::lock(&THEME_OPERATION);
    let now = local_time();
    let expiry = next_transition(now.absolute_minutes, now.minutes, light_window());
    apply_windows_theme(dark)?;
    apply_appearance(dark);
    *crate::runtime::lock(&MANUAL_OVERRIDE) = Some((dark, expiry));
    drop(operation);
    crate::request_theme_refresh();
    crate::log_line(&format!(
        "theme manual override: {} (until {:02}:{:02})",
        if dark { "dark" } else { "light" },
        (expiry % (24 * 60)) / 60,
        (expiry % (24 * 60)) % 60
    ));
    Ok(())
}

/// True while automatic switching is snoozed; an expired deadline clears.
fn snooze_active() -> bool {
    let mut snooze = crate::runtime::lock(&SNOOZE);
    match *snooze {
        Some(deadline) if local_time().absolute_minutes >= deadline => {
            *snooze = None;
            false
        }
        Some(_) => true,
        None => false,
    }
}

/// Snooze deadline in absolute minutes for status text, without mutating.
pub fn snooze_deadline() -> Option<i64> {
    let deadline = (*crate::runtime::lock(&SNOOZE))?;
    if local_time().absolute_minutes < deadline {
        Some(deadline)
    } else {
        None
    }
}

/// Deadline as local HH:MM for the panel schedule subtitle.
pub fn snooze_deadline_text() -> Option<String> {
    let deadline = snooze_deadline()?;
    let now = local_time();
    let minutes =
        ((now.minutes as i64 + (deadline - now.absolute_minutes)).rem_euclid(24 * 60)) as i32;
    Some(format!("{:02}:{:02}", minutes / 60, minutes % 60))
}

/// Arms (or re-arms) the snooze with a delay in minutes.
pub fn snooze(delta_minutes: i64) {
    *crate::runtime::lock(&SNOOZE) = Some(local_time().absolute_minutes + delta_minutes.max(1));
    crate::log_line(&format!("theme schedule snoozed for {delta_minutes} min"));
}

/// Snoozes until the next light-theme boundary ("until morning"): the dark
/// switch due tonight is postponed, and morning resolves to light anyway.
pub fn snooze_until_morning() {
    let now = local_time();
    let light = light_window().map(|(start, _)| start).unwrap_or(7 * 60);
    let delta = if now.minutes < light {
        light - now.minutes
    } else {
        24 * 60 - now.minutes + light
    };
    snooze(delta as i64 + 1);
}

pub fn snooze_cancel() {
    if crate::runtime::lock(&SNOOZE).take().is_some() {
        crate::log_line("theme schedule snooze cancelled");
    }
}

// ---- Appearance linkage (wallpaper + cursor schemes) -----------------------

const CURSORS_KEY: &str = "Control Panel\\Cursors";
const SCHEMES_KEY: &str = "Control Panel\\Cursors\\Schemes";
/// The 15 named cursor values a scheme defines, in the scheme-blob order.
const CURSOR_NAMES: [&str; 15] = [
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
];

/// Applies the wallpaper and cursor scheme configured for the given side.
/// Each item is best-effort: a failure logs and never blocks the theme
/// switch (the registry Personalize write already succeeded).
fn apply_appearance(dark: bool) {
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
    if !wallpaper.trim().is_empty()
        && let Err(error) = apply_wallpaper(&wallpaper)
    {
        crate::log_line(&format!("wallpaper switch failed: {error}"));
    }
    if !scheme.trim().is_empty()
        && let Err(error) = apply_cursor_scheme(&scheme)
    {
        crate::log_line(&format!("cursor scheme switch failed: {error}"));
    }
}

/// Wallpaper via SPI_SETDESKWALLPAPER; the file must exist (Windows copies
/// or converts it into the transcoded wallpaper cache).
fn apply_wallpaper(path: &str) -> Result<(), String> {
    if !std::path::Path::new(path).is_file() {
        return Err(format!("wallpaper file not found: {path}"));
    }
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

/// Lists installed cursor-scheme names (HKCU ... Cursors\Schemes values).
pub fn cursor_schemes() -> Vec<String> {
    unsafe {
        let schemes = wide(SCHEMES_KEY);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(schemes.as_ptr()),
            None,
            KEY_READ,
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
        names.sort_by_key(|n| n.to_lowercase());
        names
    }
}

/// Applies a named scheme: read its value blob (15 REG_EXPAND_SZ paths
/// joined by NULs), write each into HKCU ... Cursors, then SPI_SETCURSORS.
fn apply_cursor_scheme(name: &str) -> Result<(), String> {
    unsafe {
        let schemes = wide(SCHEMES_KEY);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(schemes.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return Err("open Cursors\\Schemes failed".into());
        }
        let value_name = wide(name);
        let mut kind = windows::Win32::System::Registry::REG_VALUE_TYPE(0);
        let mut size = 0u32;
        let query = windows::Win32::System::Registry::RegQueryValueExW(
            hkey,
            PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        );
        if query != ERROR_SUCCESS {
            let _ = RegCloseKey(hkey);
            return Err(format!("scheme {name:?} not found"));
        }
        let mut blob = vec![0u8; size as usize];
        let read = windows::Win32::System::Registry::RegQueryValueExW(
            hkey,
            PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut kind),
            Some(blob.as_mut_ptr()),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);
        if read != ERROR_SUCCESS
            || !matches!(
                kind,
                windows::Win32::System::Registry::REG_EXPAND_SZ
                    | windows::Win32::System::Registry::REG_SZ
            )
        {
            return Err(format!("scheme {name:?} unreadable"));
        }
        blob.truncate(size as usize);
        // Split the NUL-separated UTF-16 blob into paths.
        let units: Vec<u16> = blob
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .collect();
        let paths: Vec<String> = units
            .split(|&u| u == 0)
            .take(CURSOR_NAMES.len())
            .map(String::from_utf16_lossy)
            .collect();
        if paths.len() < CURSOR_NAMES.len() {
            return Err(format!(
                "scheme {name:?} defines {} of {} cursors",
                paths.len(),
                CURSOR_NAMES.len()
            ));
        }

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
        // Write every cursor of the scheme; a scheme may also reset the
        // (Base)SchemeDefault marker so the picker shows the scheme name.
        let mut failed = Vec::new();
        for (value, path) in CURSOR_NAMES.iter().zip(&paths) {
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
        let scheme_marker = wide("(Scheme Default)");
        let name_wide = wide(name);
        let name_bytes: Vec<u8> = name_wide
            .iter()
            .flat_map(|u| u.to_le_bytes())
            .chain([0u8, 0])
            .collect();
        let _ = RegSetValueExW(
            target,
            PCWSTR(scheme_marker.as_ptr()),
            None,
            windows::Win32::System::Registry::REG_SZ,
            Some(&name_bytes),
        );
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
    enqueue_manual_switch(&MANUAL_REQUESTS, || {
        crate::theme::read_light_preference(APP_BREED)
            .map(|light| !light)
            .unwrap_or_else(crate::theme::is_dark)
    });
    wake();
}

fn enqueue_manual_switch(requests: &Mutex<ManualRequests>, read_current: impl FnOnce() -> bool) {
    // Keep completion from clearing target between reading the registry and
    // choosing the next request. This lock never covers a theme operation.
    let mut requests = crate::runtime::lock(requests);
    let current = read_current();
    // Only enqueue here: the scheduler/repair lock can cover a slow Windows
    // theme-file operation, and must never be acquired by a button callback.
    requests.toggle(current);
}

pub fn finish_manual_switch() {
    let error = crate::runtime::lock(&MANUAL_ERROR).take();
    if let Some(error) = error {
        crate::log_line(&format!("manual theme switch failed: {error}"));
        crate::warn_dialog("", &error);
    }
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
            crate::runtime::catch_and_log("theme-repair", || {
                let operation = crate::runtime::lock(&THEME_OPERATION);
                let result = if crate::theme_repair::full_dwm_refresh_available() {
                    crate::theme_repair::refresh_dwm_colorization().map_err(|e| e.to_string())
                } else {
                    Ok(())
                };
                crate::theme_recovery::manual_repair_completed();
                drop(operation);
                crate::theme_repair::notify_theme_changed();
                *crate::runtime::lock(&REPAIR_RESULT) = Some(result);
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(crate::hwnd(&crate::HIDDEN)),
                        crate::WM_REFRESH_UI,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            });
            // Free the button whenever the repair thread ends — success or
            // panic. The success path used to reset inside the body; the
            // reset must not depend on how the body exited.
            REPAIR_RUNNING.store(false, Ordering::SeqCst);
        })
    {
        REPAIR_RUNNING.store(false, Ordering::SeqCst);
        crate::warn_dialog("", &error.to_string());
    }
}

pub fn finish_repair() {
    let result = crate::runtime::lock(&REPAIR_RESULT).take();
    if let Some(result) = result {
        crate::request_theme_repair_refresh();
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
    #[test]
    fn snooze_deadline_reports_only_while_active() {
        // Regression: the branches were once inverted, so an armed snooze
        // reported None everywhere and the panel showed no feedback at all.
        let now = local_time();
        *crate::runtime::lock(&SNOOZE) = Some(now.absolute_minutes + 30);
        assert!(snooze_deadline().is_some());
        assert!(snooze_deadline_text().is_some());
        *crate::runtime::lock(&SNOOZE) = Some(now.absolute_minutes - 1);
        assert!(snooze_deadline().is_none());
        assert!(snooze_deadline_text().is_none());
        *crate::runtime::lock(&SNOOZE) = None;
        assert!(snooze_deadline().is_none());
    }

    #[test]
    fn completion_cannot_clear_the_target_after_a_click_reads_the_old_theme() {
        use std::sync::{Arc, Mutex, mpsc};
        let queue = Arc::new(Mutex::new(super::ManualRequests {
            pending: None,
            target: Some(true), // First request is being applied in the worker.
        }));
        let (read, has_read) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        let click_queue = queue.clone();
        let click = std::thread::spawn(move || {
            super::enqueue_manual_switch(&click_queue, || {
                read.send(()).unwrap();
                resumed.recv().unwrap();
                false // Registry snapshot from just before the first write.
            });
        });
        has_read
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let completion_blocked =
            matches!(queue.try_lock(), Err(std::sync::TryLockError::WouldBlock));
        resume.send(()).unwrap();
        click.join().unwrap();
        let mut queue = crate::runtime::lock(&queue);
        queue.complete();
        assert!(
            completion_blocked,
            "completion could invalidate the click's baseline"
        );
        assert_eq!(
            queue.pending,
            Some(false),
            "second click must undo the first request"
        );
    }

    #[test]
    fn manual_requests_keep_latest_intent_while_a_switch_is_running() {
        let mut requests = super::ManualRequests::default();
        requests.toggle(false);
        assert_eq!(requests.pending.take(), Some(true));
        requests.toggle(false); // The first switch has not reached the registry yet.
        assert_eq!(requests.pending, Some(false));
        requests.complete();
        assert_eq!(requests.target, Some(false));
        assert_eq!(requests.pending.take(), Some(false));
        requests.complete();
        requests.toggle(true); // A later external theme change becomes the baseline.
        assert_eq!(requests.pending, Some(false));
    }

    #[test]
    fn manual_click_does_not_wait_for_the_background_operation_lock() {
        let _test = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        let previous = std::mem::take(&mut *crate::runtime::lock(&super::MANUAL_REQUESTS));
        let operation = crate::runtime::lock(&super::THEME_OPERATION);
        let (done, result) = std::sync::mpsc::channel();
        let click = std::thread::spawn(move || {
            super::manual_switch();
            done.send(()).unwrap();
        });
        let responsive = result
            .recv_timeout(std::time::Duration::from_secs(1))
            .is_ok();
        drop(operation);
        click.join().unwrap();
        *crate::runtime::lock(&super::MANUAL_REQUESTS) = previous;
        assert!(
            responsive,
            "button callback waited for the theme repair lock"
        );
    }

    #[test]
    fn preference_first_write_failure_does_not_restore_untouched_value() {
        use std::cell::RefCell;
        let values = RefCell::new([Some(1), Some(1)]);
        let mut writes = Vec::new();
        let result = super::apply_preferences(
            true,
            |name| Ok(values.borrow()[usize::from(name == super::APP_BREED)]),
            |name, value| {
                writes.push(name.to_owned());
                if writes.len() == 1 {
                    // An external change to the untouched preference must survive.
                    *values.borrow_mut() = [Some(0), Some(0)];
                    return Err("injected failure".into());
                }
                values.borrow_mut()[usize::from(name == super::APP_BREED)] = value;
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err(), "injected failure");
        assert_eq!(writes, [super::BREED, super::BREED]);
        assert_eq!(*values.borrow(), [Some(1), Some(0)]);
    }

    #[test]
    fn preference_transaction_restores_partial_writes_and_failed_verification() {
        use std::cell::RefCell;
        for fail_write in [true, false] {
            let values = RefCell::new([Some(1), None]);
            let calls = std::cell::Cell::new(0);
            let index = |name: &str| usize::from(name == super::APP_BREED);
            let result = super::apply_preferences(
                true,
                |name| Ok(values.borrow()[index(name)]),
                |name, value| {
                    calls.set(calls.get() + 1);
                    if calls.get() == 2 {
                        if fail_write {
                            return Err("injected failure".into());
                        }
                        return Ok(()); // Simulate Windows not retaining the write.
                    }
                    values.borrow_mut()[index(name)] = value;
                    Ok(())
                },
            );
            assert!(result.is_err());
            assert_eq!(*values.borrow(), [Some(1), None]);
        }
    }
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
