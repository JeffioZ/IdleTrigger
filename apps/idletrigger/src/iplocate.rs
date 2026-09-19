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
    *CACHE.lock().unwrap() = Some(location);
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
            resolve();
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
        CACHE.lock().unwrap().clone()
    } else {
        None
    }
}

fn fetch_ipwho() -> Option<Location> {
    use windows::Win32::Networking::WinHttp::{
        URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
        WINHTTP_INTERNET_SCHEME_HTTPS, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl,
        WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpReadData,
        WinHttpReceiveResponse, WinHttpSendRequest,
    };
    use windows::core::PCWSTR;

    unsafe {
        let session = WinHttpOpen(
            PCWSTR(wide("IdleTrigger/1.0").as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        );
        if session.is_null() {
            return None;
        }
        if windows::Win32::Networking::WinHttp::WinHttpSetTimeouts(session, 5000, 5000, 5000, 5000)
            .is_err()
        {
            let _ = WinHttpCloseHandle(session);
            return None;
        }
        let url_wide: Vec<u16> = "https://ipwho.is/".encode_utf16().collect();
        let mut parts = URL_COMPONENTS {
            dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
            dwHostNameLength: u32::MAX,
            ..Default::default()
        };
        if WinHttpCrackUrl(&url_wide, 0, &mut parts).is_err()
            || parts.lpszHostName.is_null()
            || parts.dwHostNameLength == 0
        {
            let _ = WinHttpCloseHandle(session);
            return None;
        }
        let host = String::from_utf16_lossy(std::slice::from_raw_parts(
            parts.lpszHostName.0,
            parts.dwHostNameLength as usize,
        ));
        let connect = WinHttpConnect(session, PCWSTR(wide(&host).as_ptr()), parts.nPort, 0);
        if connect.is_null() {
            let _ = WinHttpCloseHandle(session);
            return None;
        }
        let flags = if parts.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS {
            WINHTTP_FLAG_SECURE
        } else {
            windows::Win32::Networking::WinHttp::WINHTTP_OPEN_REQUEST_FLAGS(0)
        };
        let request = WinHttpOpenRequest(
            connect,
            PCWSTR(wide("GET").as_ptr()),
            PCWSTR(wide("/").as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        );
        if request.is_null() {
            let _ = WinHttpCloseHandle(connect);
            let _ = WinHttpCloseHandle(session);
            return None;
        }

        let ok = WinHttpSendRequest(request, None, None, 0, 0, 0).is_ok()
            && WinHttpReceiveResponse(request, std::ptr::null_mut()).is_ok();
        if !ok {
            let _ = WinHttpCloseHandle(request);
            let _ = WinHttpCloseHandle(connect);
            let _ = WinHttpCloseHandle(session);
            return None;
        }

        let mut status: u32 = 0;
        let mut status_size = size_of::<u32>() as u32;
        if windows::Win32::Networking::WinHttp::WinHttpQueryHeaders(
            request,
            windows::Win32::Networking::WinHttp::WINHTTP_QUERY_STATUS_CODE
                | windows::Win32::Networking::WinHttp::WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut status_size,
            std::ptr::null_mut(),
        )
        .is_err()
            || status != 200
        {
            let _ = WinHttpCloseHandle(request);
            let _ = WinHttpCloseHandle(connect);
            let _ = WinHttpCloseHandle(session);
            return None;
        }
        let mut body = Vec::new();
        let mut complete = false;
        loop {
            let mut available: u32 = 0;
            if WinHttpQueryDataAvailable(request, &mut available).is_err() {
                break;
            }
            if available == 0 {
                complete = true;
                break;
            }
            if available as usize > 64 * 1024 - body.len() {
                break;
            }
            let mut chunk = vec![0u8; available as usize];
            let mut read: u32 = 0;
            if WinHttpReadData(request, chunk.as_mut_ptr().cast(), available, &mut read).is_err()
                || read == 0
            {
                break;
            }
            chunk.truncate(read as usize);
            body.extend_from_slice(&chunk);
            if body.len() > 64 * 1024 {
                break; // sanity cap
            }
        }
        let _ = WinHttpCloseHandle(request);
        let _ = WinHttpCloseHandle(connect);
        let _ = WinHttpCloseHandle(session);

        if complete {
            parse_ipwho(&String::from_utf8_lossy(&body))
        } else {
            None
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
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
        let _guard = crate::CONFIG_TEST_LOCK.lock().unwrap();
        let previous = CACHE.lock().unwrap().take();
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
        *CACHE.lock().unwrap() = Some(Location {
            coordinates: (22.28, 114.17),
            label: "Hong Kong, China".into(),
        });
        LAST_SUCCESS.store(now_secs(), Ordering::SeqCst);
        assert!(matches!(status(), Status::Resolved(label) if label == "Hong Kong, China"));
        assert_eq!(request(), Some((22.28, 114.17)));
        *CACHE.lock().unwrap() = previous;
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
