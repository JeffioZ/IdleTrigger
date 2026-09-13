//! IP geolocation via ipwho.is over WinHTTP — no Go net/http, no extra
//! crates. Success caches for 24h in memory; failures retry after 30 min.
//! Contract parity with Go `internal/feature/theme/location.go`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

static CACHE: Mutex<Option<(f64, f64)>> = Mutex::new(None);
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
    if let Some(hit) = *CACHE.lock().unwrap()
        && now - LAST_SUCCESS.load(Ordering::SeqCst) < SUCCESS_TTL_SECS
    {
        return Some(hit);
    }
    if now - LAST_FAILURE.load(Ordering::SeqCst) < FAILURE_RETRY_SECS {
        return None;
    }
    let Some((lat, lon)) = fetch_ipwho() else {
        LAST_FAILURE.store(now, Ordering::SeqCst);
        return None;
    };
    *CACHE.lock().unwrap() = Some((lat, lon));
    LAST_SUCCESS.store(now, Ordering::SeqCst);
    Some((lat, lon))
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
        QUERYING.store(false, Ordering::SeqCst);
    }
    None
}

/// Last resolved coordinates, when the cache is still fresh (settings status).
pub fn cached() -> Option<(f64, f64)> {
    let now = now_secs();
    if now - LAST_SUCCESS.load(Ordering::SeqCst) < SUCCESS_TTL_SECS {
        *CACHE.lock().unwrap()
    } else {
        None
    }
}

fn fetch_ipwho() -> Option<(f64, f64)> {
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

/// Minimal JSON field extraction for `{"success":true,"latitude":x,"longitude":y}`.
fn parse_ipwho(body: &str) -> Option<(f64, f64)> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if !value.get("success")?.as_bool()? {
        return None;
    }
    let lat = value.get("latitude")?.as_f64()?;
    let lon = value.get("longitude")?.as_f64()?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some((lat, lon))
}
