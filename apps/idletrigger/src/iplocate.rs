//! IP geolocation via ipwho.is over WinHTTP — no Go net/http, no extra
//! crates. Success caches for 24h in memory; failures retry after 30 min.
//! Contract parity with Go `internal/feature/theme/location.go`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

static CACHE: Mutex<Option<(f64, f64)>> = Mutex::new(None);
static LAST_SUCCESS: AtomicI64 = AtomicI64::new(0);
static LAST_FAILURE: AtomicI64 = AtomicI64::new(0);

const SUCCESS_TTL_SECS: i64 = 24 * 60 * 60;
const FAILURE_RETRY_SECS: i64 = 30 * 60;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Returns cached or freshly fetched coordinates; None when unavailable and
/// the retry window hasn't elapsed. Blocking (up to ~5s) — background
/// threads only.
pub fn resolve() -> Option<(f64, f64)> {
    let now = now_secs();
    if let Some(hit) = *CACHE.lock().unwrap()
        && now - LAST_SUCCESS.load(Ordering::SeqCst) < SUCCESS_TTL_SECS
    {
        return Some(hit);
    }
    if now - LAST_FAILURE.load(Ordering::SeqCst) < FAILURE_RETRY_SECS {
        return None;
    }
    let (lat, lon) = fetch_ipwho()?;
    *CACHE.lock().unwrap() = Some((lat, lon));
    LAST_SUCCESS.store(now, Ordering::SeqCst);
    Some((lat, lon))
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
        let url_wide: Vec<u16> = "https://ipwho.is/".encode_utf16().collect();
        let mut parts = URL_COMPONENTS {
            dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
            ..Default::default()
        };
        if WinHttpCrackUrl(&url_wide, 0, &mut parts).is_err() {
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

        let mut body = Vec::new();
        loop {
            let mut available: u32 = 0;
            if WinHttpQueryDataAvailable(request, &mut available).is_err() || available == 0 {
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

        parse_ipwho(&String::from_utf8_lossy(&body))
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
    Some((lat, lon))
}
