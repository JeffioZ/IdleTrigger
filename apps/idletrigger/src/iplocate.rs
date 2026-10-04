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
    use windows::Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
        WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
        WinHttpSendRequest, WinHttpSetTimeouts,
    };
    use windows::core::PCWSTR;

    // RAII close for the three WinHTTP handles: no early-return path can
    // leak (the manual close cascade this replaced had four copies).
    struct Guard(*mut core::ffi::c_void);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                let _ = windows::Win32::Networking::WinHttp::WinHttpCloseHandle(self.0);
            }
        }
    }
    // Failures used to vanish silently and left "IP lookup failed" without a
    // cause. They are throttled (30-minute retry), so one log line each is
    // safe and makes the settings-page status diagnosable.
    let fail = |what: String| crate::log_line(&format!("ip locate: {what}"));

    unsafe {
        let agent = wide(&format!("IdleTrigger/{}", crate::APP_VERSION));
        let session = Guard(WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            fail("WinHttpOpen failed".into());
            return None;
        }
        if let Err(err) = WinHttpSetTimeouts(session.0, 5000, 5000, 5000, 5000) {
            fail(format!("WinHttpSetTimeouts failed: {err}"));
            return None;
        }
        // The endpoint is a compile-time constant: no URL cracking needed.
        let connect = Guard(WinHttpConnect(
            session.0,
            PCWSTR(wide("ipwho.is").as_ptr()),
            443, // HTTPS default port; the endpoint is a fixed constant
            0,
        ));
        if connect.0.is_null() {
            fail("WinHttpConnect failed".into());
            return None;
        }
        let request = Guard(WinHttpOpenRequest(
            connect.0,
            PCWSTR(wide("GET").as_ptr()),
            PCWSTR(wide("/").as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            fail("WinHttpOpenRequest failed".into());
            return None;
        }
        if let Err(err) = WinHttpSendRequest(request.0, None, None, 0, 0, 0) {
            fail(format!("request send failed: {err}"));
            return None;
        }
        if let Err(err) = WinHttpReceiveResponse(request.0, std::ptr::null_mut()) {
            fail(format!("request receive failed: {err}"));
            return None;
        }

        let mut status: u32 = 0;
        let mut status_size = size_of::<u32>() as u32;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut status_size,
            std::ptr::null_mut(),
        )
        .is_err()
        {
            fail("status query failed".into());
            return None;
        }
        if status != 200 {
            fail(format!("HTTP {status}"));
            return None;
        }
        let mut body = Vec::new();
        let mut complete = false;
        loop {
            let mut available: u32 = 0;
            if WinHttpQueryDataAvailable(request.0, &mut available).is_err() {
                break;
            }
            if available == 0 {
                complete = true;
                break;
            }
            if available as usize > 64 * 1024 - body.len() {
                fail("response exceeded the 64 KiB cap".into());
                break;
            }
            let mut chunk = vec![0u8; available as usize];
            let mut read: u32 = 0;
            if WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), available, &mut read).is_err()
                || read == 0
            {
                break;
            }
            chunk.truncate(read as usize);
            body.extend_from_slice(&chunk);
        }
        if !complete {
            fail("response read ended early".into());
            return None;
        }
        parse_ipwho(&String::from_utf8_lossy(&body))
    }
}

use crate::wide;

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
