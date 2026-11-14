//! IP geolocation via ipwho.is over WinHTTP without an extra HTTP crate.
//! Success caches for 24 hours in memory; failures retry after 30 minutes.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

#[derive(Clone)]
struct Location {
    coordinates: (f64, f64),
    label: String,
}

static CACHE: Mutex<Option<Location>> = Mutex::new(None);
static LAST_SUCCESS: AtomicI64 = AtomicI64::new(0);
static LAST_FAILURE: AtomicI64 = AtomicI64::new(0);
static QUERYING: AtomicBool = AtomicBool::new(false);

const SUCCESS_TTL_SECS: i64 = 24 * 60 * 60;
const FAILURE_RETRY_SECS: i64 = 30 * 60;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Returns cached or freshly fetched coordinates; None when unavailable and
/// the retry window hasn't elapsed. Blocking: the 5s timeouts apply to
/// individual network phases, not the whole request. Background threads only.
fn resolve() -> Option<(f64, f64)> {
    let now = now_secs();
    if let Some(hit) = cached() {
        return Some(hit);
    }
    if now - LAST_FAILURE.load(Ordering::SeqCst) < FAILURE_RETRY_SECS {
        return None;
    }
    let Some(location) = fetch_ipwho() else {
        LAST_FAILURE.store(now, Ordering::SeqCst);
        return None;
    };
    let coordinates = location.coordinates;
    *crate::runtime::lock(&CACHE) = Some(location);
    LAST_SUCCESS.store(now, Ordering::SeqCst);
    Some(coordinates)
}

/// Starts at most one lookup and returns immediately, including on the UI thread.
pub fn request() -> Option<(f64, f64)> {
    if let Some(hit) = cached() {
        return Some(hit);
    }
    if now_secs() - LAST_FAILURE.load(Ordering::SeqCst) < FAILURE_RETRY_SECS
        || QUERYING.swap(true, Ordering::SeqCst)
    {
        return None;
    }
    if std::thread::Builder::new()
        .name("ip-location".into())
        .spawn(|| {
            // Reset outside the caught body: a panic must not leave the
            // locator stuck in "querying" forever.
            let _ = crate::runtime::catch_and_log("ip-location", || {
                let _ = resolve();
            });
            QUERYING.store(false, Ordering::SeqCst);
            crate::theme_engine::wake();
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    Some(crate::hwnd(&crate::HIDDEN)),
                    crate::WM_REFRESH_UI,
                    windows::Win32::Foundation::WPARAM(0),
                    windows::Win32::Foundation::LPARAM(0),
                );
            }
        })
        .is_err()
    {
        LAST_FAILURE.store(now_secs(), Ordering::SeqCst);
        QUERYING.store(false, Ordering::SeqCst);
        crate::log_line("ip locate: worker thread could not start");
    }
    None
}

/// Last resolved coordinates, when the cache is still fresh (settings status).
pub fn cached() -> Option<(f64, f64)> {
    cached_location().map(|location| location.coordinates)
}

pub fn cached_label() -> Option<String> {
    cached_location().map(|location| location.label)
}

pub enum Status {
    Resolved(String),
    Querying,
    Failed,
    NotRequested,
}

pub fn status() -> Status {
    if let Some(label) = cached_label() {
        Status::Resolved(label)
    } else if QUERYING.load(Ordering::SeqCst) {
        Status::Querying
    } else if LAST_FAILURE.load(Ordering::SeqCst) != 0 {
        Status::Failed
    } else {
        Status::NotRequested
    }
}

fn cached_location() -> Option<Location> {
    let now = now_secs();
    if now - LAST_SUCCESS.load(Ordering::SeqCst) < SUCCESS_TTL_SECS {
        crate::runtime::lock(&CACHE).clone()
    } else {
        None
    }
}

fn fetch_ipwho() -> Option<Location> {
    // Failures used to vanish silently and left "IP lookup failed" without a
    // cause. They are throttled (30-minute retry), so one log line each is
    // safe and makes the settings-page status diagnosable.
    let fail = |what: String| crate::log_line(&format!("ip locate: {what}"));
    let response =
        match crate::net::http_get("https://ipwho.is/", (5000, 5000, 5000, 5000), 64 * 1024) {
            Ok(response) => response,
            Err(error) => {
                fail(error.to_string());
                return None;
            }
        };
    if response.status != 200 {
        fail(format!("HTTP {}", response.status));
        return None;
    }
    parse_ipwho(&String::from_utf8_lossy(&response.body))
}

/// Keep the display label alongside the coordinates used for solar calculations.
fn parse_ipwho(body: &str) -> Option<Location> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if !value.get("success")?.as_bool()? {
        return None;
    }
    let lat = value.get("latitude")?.as_f64()?;
    let lon = value.get("longitude")?.as_f64()?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let label = ["city", "region", "country"]
        .iter()
        .filter_map(|key| value.get(key)?.as_str())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    Some(Location {
        coordinates: (lat, lon),
        label: if label.is_empty() {
            format!("{lat:.2}, {lon:.2}")
        } else {
            label
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_distinguishes_unrequested_inflight_failure_and_cached_success() {
        let _guard = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        let previous = crate::runtime::lock(&CACHE).take();
        let success = LAST_SUCCESS.swap(0, Ordering::SeqCst);
        let failure = LAST_FAILURE.swap(0, Ordering::SeqCst);
        let querying = QUERYING.swap(false, Ordering::SeqCst);
        assert!(matches!(status(), Status::NotRequested));
        QUERYING.store(true, Ordering::SeqCst);
        assert!(matches!(status(), Status::Querying));
        assert!(request().is_none()); // No duplicate lookup while one is running.
        QUERYING.store(false, Ordering::SeqCst);
        LAST_FAILURE.store(now_secs(), Ordering::SeqCst);
        assert!(matches!(status(), Status::Failed));
        assert!(request().is_none()); // Preserve the failure retry interval.
        *crate::runtime::lock(&CACHE) = Some(Location {
            coordinates: (22.28, 114.17),
            label: "Hong Kong, China".into(),
        });
        LAST_SUCCESS.store(now_secs(), Ordering::SeqCst);
        assert!(matches!(status(), Status::Resolved(label) if label == "Hong Kong, China"));
        assert_eq!(request(), Some((22.28, 114.17)));
        *crate::runtime::lock(&CACHE) = previous;
        LAST_SUCCESS.store(success, Ordering::SeqCst);
        LAST_FAILURE.store(failure, Ordering::SeqCst);
        QUERYING.store(querying, Ordering::SeqCst);
    }

    #[test]
    fn location_retains_coordinates_and_formats_available_place_names() {
        let location = parse_ipwho(r#"{"success":true,"latitude":22.28,"longitude":114.17,"city":" Hong Kong ","region":" ","country":"China"}"#).unwrap();
        assert_eq!(location.coordinates, (22.28, 114.17));
        assert_eq!(location.label, "Hong Kong, China");
        let location =
            parse_ipwho(r#"{"success":true,"latitude":22.28,"longitude":114.17}"#).unwrap();
        assert_eq!(location.label, "22.28, 114.17");
        assert!(parse_ipwho(r#"{"success":false,"latitude":22,"longitude":114}"#).is_none());
        assert!(parse_ipwho(r#"{"success":true,"latitude":91,"longitude":114}"#).is_none());
    }
}
