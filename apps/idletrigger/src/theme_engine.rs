//! Day/Night theme engine: scheduled light/dark switching (fixed times or
//! sunrise/sunset), Windows Personalize registry writes, and battery-based
//! dark preference. The IP-location and fullscreen-pause refinements are
//! follow-ups; the fixed/sunrise-by-offset core matches the Go contract.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM, WPARAM};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, RegCloseKey, RegOpenKeyExW,
    RegSetValueExW,
};
use windows::Win32::System::StationsAndDesktops::{
    BSF_POSTMESSAGE, BSM_APPLICATIONS, BroadcastSystemMessageW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};
use windows::core::PCWSTR;

static THEME_THREAD_RUNNING: AtomicBool = AtomicBool::new(false);
/// Current computed mode: true = dark.
static IS_DARK_NOW: AtomicBool = AtomicBool::new(false);
/// Manual override until the next scheduled transition (Go contract).
static MANUAL_OVERRIDE: Mutex<Option<bool>> = Mutex::new(None);
/// When the manual override expires (the next scheduled transition moment);
/// None = no override active.
static MANUAL_UNTIL: Mutex<Option<i32>> = Mutex::new(None);
/// Cached sunrise/sunset in minutes-from-midnight, tagged with the day-of-year
/// they were solved for (re-solved after midnight).
static SUN_TIMES: Mutex<Option<(i32, i32, i32)>> = Mutex::new(None); // (rise, set, doy)

const PERSONALIZE_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
const BREED: &str = "SystemUsesLightTheme";
const APP_BREED: &str = "AppsUseLightTheme";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Local time snapshot (minutes + weekday).
pub struct LocalTime {
    pub minutes: i32,
}

fn local_time() -> LocalTime {
    unsafe {
        let st = windows::Win32::System::SystemInformation::GetLocalTime();
        LocalTime {
            minutes: st.wHour as i32 * 60 + st.wMinute as i32,
        }
    }
}

/// NOAA solar approximation for sunrise/sunset in minutes-from-midnight.
/// Inputs: latitude/longitude degrees, day-of-year. Returns (sunrise, sunset).
/// Accuracy is ±5 minutes — adequate for theme switching.
pub fn solar_times(lat: f64, lon: f64, day_of_year: i32) -> Option<(i32, i32)> {
    if lat.abs() > 90.0 {
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
    let mut sunrise = solar_noon - day_len_min / 2.0;
    let mut sunset = solar_noon + day_len_min / 2.0;
    // Clamp into the day (Go wrap loops).
    while sunrise < 0.0 {
        sunrise += 1440.0;
    }
    while sunset < 0.0 {
        sunset += 1440.0;
    }
    while sunrise >= 1440.0 {
        sunrise -= 1440.0;
    }
    while sunset >= 1440.0 {
        sunset -= 1440.0;
    }
    Some((sunrise.round() as i32, sunset.round() as i32))
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
            } else {
                tzi.StandardBias
            };
        -bias
    }
}

/// Fallback location resolution per the Go contract: fixed defaults when no
/// IP location is configured (IP lookup is a follow-up refinement).
fn location_fallback() -> (f64, f64) {
    // Beijing-ish defaults; the Go fallback order ends at a default location.
    (39.9042, 116.4074)
}

/// Today's day-of-year for callers outside the scheduler (panel subtitle).
pub fn day_of_year_today() -> i32 {
    day_of_year_now()
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
fn light_window() -> Option<(i32, i32)> {
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
    // Sunrise mode: solve solar times, cached per day-of-year (re-solved
    // after midnight like the Go scheduler, which recomputes every tick).
    let doy = day_of_year_now();
    if let Some((rise, set, cached_doy)) = *SUN_TIMES.lock().unwrap()
        && cached_doy == doy
    {
        return Some((rise, set));
    }
    // Go resolution order: optional IP location first, then fallback.
    let (lat, lon) = if ip_enabled {
        crate::iplocate::resolve().unwrap_or_else(location_fallback)
    } else {
        location_fallback()
    };
    let times = solar_times(lat, lon, doy);
    if let Some((rise, set)) = times {
        *SUN_TIMES.lock().unwrap() = Some((rise, set, doy));
    }
    times
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
fn apply_windows_theme(dark: bool) {
    unsafe {
        let key = wide(PERSONALIZE_KEY);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            KEY_READ | KEY_WRITE,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return;
        }
        let value = (!dark) as u32; // light theme flag = !dark
        let bytes = value.to_le_bytes();
        for name in [BREED, APP_BREED] {
            let name_w = wide(name);
            // Personalize values are REG_DWORD (Go SetDWordValue) — writing
            // any other type makes the system theme switch silently fail.
            let _ = RegSetValueExW(hkey, PCWSTR(name_w.as_ptr()), None, REG_DWORD, Some(&bytes));
        }
        let _ = RegCloseKey(hkey);
        // Notify running apps + the shell so they repaint.
        let mut targets = BSM_APPLICATIONS;
        let _ = BroadcastSystemMessageW(
            BSF_POSTMESSAGE,
            Some(&mut targets),
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(wide("ImmersiveColorSet").as_ptr() as isize),
        );
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            1000,
            None,
        );
    }
}

/// Poll target: applies the theme and updates our own follow layer.
fn tick() {
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
    let target = {
        let mut manual = MANUAL_OVERRIDE.lock().unwrap();
        let mut until = MANUAL_UNTIL.lock().unwrap();
        // Manual override holds until the next scheduled transition moment;
        // after that, control returns to the schedule (Go manualUntil).
        if let (Some(_), Some(expiry)) = (*manual, *until) {
            let now = local_time();
            if now.minutes >= expiry {
                *manual = None;
                *until = None;
            }
        }
        match *manual {
            Some(v) => Some(v),
            None => {
                *until = None;
                scheduled_dark()
            }
        }
    };
    let Some(target) = target else { return };

    // Battery-based dark preference (Go contract: battery → dark).
    let on_ac = crate::ON_AC.load(Ordering::SeqCst);
    let dark_on_battery = crate::cfg_map(|c| c.theme_dark_on_battery);
    let target = if !on_ac && dark_on_battery {
        true
    } else {
        target
    };

    let previous = IS_DARK_NOW.swap(target, Ordering::SeqCst);
    if previous != target {
        crate::log_line(&format!(
            "theme engine: switching to {}",
            if target { "dark" } else { "light" }
        ));
        apply_windows_theme(target);
        crate::theme::refresh_from_registry();
        crate::theme::apply_to_all();
    }
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
                std::thread::sleep(Duration::from_secs(60));
            }
        })
        .expect("spawn theme engine");
}

/// Sets a manual dark/light override until the next scheduled transition.
pub fn set_manual_override(dark: bool) {
    *MANUAL_OVERRIDE.lock().unwrap() = Some(dark);
    // Override holds until the next scheduled transition moment (Go
    // manualUntil): the light/dark boundary we are currently inside.
    let expiry = match light_window() {
        Some((light_start, dark_start)) => {
            let now = local_time();
            let boundary = if dark { dark_start } else { light_start };
            if now.minutes < boundary {
                boundary
            } else {
                boundary + 24 * 60
            }
        }
        None => 24 * 60, // no schedule: hold through the end of the day
    };
    *MANUAL_UNTIL.lock().unwrap() = Some(expiry);
    IS_DARK_NOW.store(dark, Ordering::SeqCst);
    apply_windows_theme(dark);
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
    crate::cfg_edit(|c| c.theme_switch_enabled = !c.theme_switch_enabled);
    crate::persist_config();
    crate::log_line("theme switching toggled");
}

pub fn manual_switch() {
    let current = IS_DARK_NOW.load(Ordering::SeqCst);
    set_manual_override(!current);
}

pub fn repair() {
    let target = IS_DARK_NOW.load(Ordering::SeqCst);
    let (apps_light, system_light) =
        crate::cfg_map(|c| (!c.theme_dark_on_battery || !target, !target));
    // On Win11 22H2+, run the full DWM colorization refresh first.
    if crate::theme_repair::full_dwm_refresh_available() {
        match crate::theme_repair::refresh_dwm_colorization(apps_light, system_light) {
            Ok(()) => crate::log_line("theme repair: full DWM colorization refresh completed"),
            Err(err) => crate::log_line(&format!("theme repair DWM refresh failed: {err}")),
        }
    }
    // Always apply the registry preference + broadcast.
    apply_windows_theme(target);
    crate::theme_repair::notify_theme_changed();
    crate::theme::refresh_from_registry();
    crate::theme::apply_to_all();
    crate::log_line("theme repair applied");
}
