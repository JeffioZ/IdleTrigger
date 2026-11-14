//! Shared WinHTTP transport.
//!
//! One process-lifetime session serves every request (update feed, checksums,
//! release download, IP lookup), so AUTOMATIC_PROXY discovery (WPAD/PAC) is
//! paid once instead of on every call. Requests run serialized behind one
//! lock, which keeps per-call session settings (timeouts) stable; traffic is
//! a few requests per day, so queuing never matters in practice.

use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use windows::Win32::Networking::WinHttp::{
    URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_INTERNET_SCHEME_HTTPS, WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_QUERY_CONTENT_LENGTH,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_RAW_HEADERS_CRLF, WINHTTP_QUERY_STATUS_CODE,
    WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::PCWSTR;

/// (resolve, connect, send, receive) in milliseconds, per WinHttpSetTimeouts.
pub(crate) type Timeouts = (i32, i32, i32, i32);

pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl HttpResponse {
    /// Case-insensitive lookup against the lowercase header pairs.
    pub(crate) fn header(&self, lowercase_name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(name, _)| name == lowercase_name)
            .map(|(_, value)| value.clone())
    }
}

pub(crate) enum NetError {
    /// Transport-level failure (open/crack/connect/send/receive/read);
    /// the payload is technical detail for logs and error strings.
    Network(String),
    /// Non-200 status where the caller asked the transport to gate.
    Status(u16),
    /// Response exceeded the caller's byte cap.
    Oversize,
    /// Destination write failed during a streamed download.
    Write(String),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Network(detail) => f.write_str(detail),
            NetError::Status(status) => write!(f, "HTTP {status}"),
            NetError::Oversize => f.write_str("response exceeded the size cap"),
            NetError::Write(detail) => write!(f, "write failed: {detail}"),
        }
    }
}

struct HandleGuard(*mut core::ffi::c_void);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

/// WinHTTP handles are thread-safe; the raw pointer just needs to cross
/// threads into the static. The handle is never closed — the session lives
/// for the process lifetime, like the CRT's Winsock bootstrap.
struct SessionHandle(*mut core::ffi::c_void);
// SAFETY: the handle is only used behind TRANSPORT's lock (or lazily created
// once via get_or_init), never mutated concurrently.
unsafe impl Send for SessionHandle {}
unsafe impl Sync for SessionHandle {}

static SESSION: OnceLock<Option<SessionHandle>> = OnceLock::new();
static TRANSPORT: Mutex<()> = Mutex::new(());

fn lock_transport() -> MutexGuard<'static, ()> {
    TRANSPORT.lock().unwrap_or_else(PoisonError::into_inner)
}

fn session_handle() -> Result<*mut core::ffi::c_void, NetError> {
    let session = SESSION
        .get_or_init(|| {
            let agent = format!("IdleTrigger/{}", crate::APP_VERSION);
            let handle = unsafe {
                WinHttpOpen(
                    PCWSTR(crate::wide(&agent).as_ptr()),
                    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    0,
                )
            };
            (!handle.is_null()).then_some(SessionHandle(handle))
        })
        .as_ref()
        .map(|handle| handle.0);
    session.ok_or_else(|| NetError::Network("WinHttpOpen failed".into()))
}

/// One connected GET request: session → URL crack → connect → open → send →
/// receive → status query. Shared by the buffered GET and the streamed
/// download, whose WinHTTP setup is identical; the connect guard keeps the
/// handle chain alive for as long as the request.
struct OpenRequest {
    _connect: HandleGuard,
    request: HandleGuard,
    status: u16,
}

fn open_get_request(url: &str, timeouts: Timeouts) -> Result<OpenRequest, NetError> {
    let session = session_handle()?;
    unsafe {
        WinHttpSetTimeouts(session, timeouts.0, timeouts.1, timeouts.2, timeouts.3)
            .map_err(|error| NetError::Network(error.to_string()))?;

        let url_wide: Vec<u16> = url.encode_utf16().collect();
        let mut parts = URL_COMPONENTS {
            dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
            dwHostNameLength: u32::MAX,
            dwUrlPathLength: u32::MAX,
            dwExtraInfoLength: u32::MAX,
            ..Default::default()
        };
        WinHttpCrackUrl(&url_wide, 0, &mut parts)
            .map_err(|error| NetError::Network(error.to_string()))?;
        if parts.lpszHostName.is_null() || parts.dwHostNameLength == 0 {
            return Err(NetError::Network("invalid URL".into()));
        }
        let host = String::from_utf16_lossy(std::slice::from_raw_parts(
            parts.lpszHostName.0,
            parts.dwHostNameLength as usize,
        ));
        let mut path = String::new();
        if !parts.lpszUrlPath.is_null() && parts.dwUrlPathLength > 0 {
            path.push_str(&String::from_utf16_lossy(std::slice::from_raw_parts(
                parts.lpszUrlPath.0,
                parts.dwUrlPathLength as usize,
            )));
        }
        if !parts.lpszExtraInfo.is_null() && parts.dwExtraInfoLength > 0 {
            path.push_str(&String::from_utf16_lossy(std::slice::from_raw_parts(
                parts.lpszExtraInfo.0,
                parts.dwExtraInfoLength as usize,
            )));
        }
        if path.is_empty() {
            path.push('/');
        }
        let connect = HandleGuard(WinHttpConnect(
            session,
            PCWSTR(crate::wide(&host).as_ptr()),
            parts.nPort,
            0,
        ));
        if connect.0.is_null() {
            return Err(NetError::Network("WinHttpConnect failed".into()));
        }
        let flags = if parts.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS {
            WINHTTP_FLAG_SECURE
        } else {
            WINHTTP_OPEN_REQUEST_FLAGS(0)
        };
        let request = HandleGuard(WinHttpOpenRequest(
            connect.0,
            PCWSTR(crate::wide("GET").as_ptr()),
            PCWSTR(crate::wide(&path).as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        ));
        if request.0.is_null() {
            return Err(NetError::Network("WinHttpOpenRequest failed".into()));
        }
        WinHttpSendRequest(request.0, None, None, 0, 0, 0)
            .map_err(|error| NetError::Network(error.to_string()))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|error| NetError::Network(error.to_string()))?;

        let mut status: u32 = 0;
        let mut status_size = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut status_size,
            std::ptr::null_mut(),
        )
        .map_err(|error| NetError::Network(error.to_string()))?;
        Ok(OpenRequest {
            _connect: connect,
            request,
            status: status as u16,
        })
    }
}

/// Reads the whole raw header block once and splits it into lowercase pairs.
fn query_headers_block(request: *mut core::ffi::c_void) -> Result<Vec<(String, String)>, NetError> {
    unsafe {
        let mut buffer = vec![0u16; 16 * 1024];
        let mut size = (buffer.len() * std::mem::size_of::<u16>()) as u32;
        if WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            PCWSTR::null(),
            Some(buffer.as_mut_ptr().cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .is_err()
        {
            // Proxy chains can pad the block past the 16K default; the failed
            // query reports the required size — retry once, memory-bounded.
            let needed = (size as usize).div_ceil(std::mem::size_of::<u16>());
            if needed <= buffer.len() || needed > 256 * 1024 {
                return Err(NetError::Network("WinHttpQueryHeaders failed".into()));
            }
            buffer = vec![0u16; needed];
            size = (buffer.len() * std::mem::size_of::<u16>()) as u32;
            WinHttpQueryHeaders(
                request,
                WINHTTP_QUERY_RAW_HEADERS_CRLF,
                PCWSTR::null(),
                Some(buffer.as_mut_ptr().cast()),
                &mut size,
                std::ptr::null_mut(),
            )
            .map_err(|error| NetError::Network(error.to_string()))?;
        }
        let text = String::from_utf16_lossy(&buffer);
        let mut headers = Vec::new();
        for line in text.split("\r\n") {
            let Some((name, value)) = line.split_once(':') else {
                continue; // Status line and malformed lines.
            };
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
        Ok(headers)
    }
}

/// One-shot GET that buffers the body (size-capped) and captures the response
/// headers as lowercase name/value pairs. Redirects are followed by default,
/// which releases/download needs to reach the CDN. HTTP statuses are returned
/// as-is and judged by the caller.
pub(crate) fn http_get(
    url: &str,
    timeouts: Timeouts,
    max_bytes: usize,
) -> Result<HttpResponse, NetError> {
    let _transport = lock_transport();
    let opened = open_get_request(url, timeouts)?;
    let request = opened.request.0;
    unsafe {
        let headers = query_headers_block(request)?;
        let mut body = Vec::new();
        loop {
            let mut available: u32 = 0;
            if WinHttpQueryDataAvailable(request, &mut available).is_err() {
                return Err(NetError::Network("WinHttpQueryDataAvailable failed".into()));
            }
            if available == 0 {
                break;
            }
            if body.len() + available as usize > max_bytes.saturating_add(1) {
                return Err(NetError::Oversize);
            }
            let mut chunk = vec![0u8; available as usize];
            let mut read: u32 = 0;
            if WinHttpReadData(request, chunk.as_mut_ptr().cast(), available, &mut read).is_err()
                || read == 0
            {
                return Err(NetError::Network("WinHttpReadData failed".into()));
            }
            chunk.truncate(read as usize);
            body.extend_from_slice(&chunk);
        }
        if body.len() > max_bytes {
            return Err(NetError::Oversize);
        }
        Ok(HttpResponse {
            status: opened.status,
            headers,
            body,
        })
    }
}

/// Streams a large response to `path`, aborting past `max_bytes` and
/// reporting (done, total) progress. Only HTTP 200 is accepted; the
/// destination is removed on failure.
pub(crate) fn http_download(
    url: &str,
    timeouts: Timeouts,
    path: &std::path::Path,
    max_bytes: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), NetError> {
    let _transport = lock_transport();
    let opened = open_get_request(url, timeouts)?;
    if opened.status != 200 {
        return Err(NetError::Status(opened.status));
    }
    let request = opened.request.0;
    // 'body breaks flow into `result` so the cleanup epilogue below sees every
    // failure path; a plain return (like the code this replaced) would strand
    // a partial file after the destination had been created.
    let result = 'body: {
        unsafe {
            let mut total: u64 = 0;
            let mut total_size: u32 = 0;
            let mut total_size_bytes = std::mem::size_of::<u32>() as u32;
            if WinHttpQueryHeaders(
                request,
                WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some((&mut total_size as *mut u32).cast()),
                &mut total_size_bytes,
                std::ptr::null_mut(),
            )
            .is_ok()
            {
                total = u64::from(total_size);
                if total > max_bytes {
                    break 'body Err(NetError::Oversize);
                }
            }

            let mut file = match std::fs::File::create(path) {
                Ok(file) => file,
                Err(error) => break 'body Err(NetError::Write(error.to_string())),
            };
            let mut done: u64 = 0;
            loop {
                let mut available: u32 = 0;
                if WinHttpQueryDataAvailable(request, &mut available).is_err() {
                    break 'body Err(NetError::Network("WinHttpQueryDataAvailable failed".into()));
                }
                if available == 0 {
                    break;
                }
                if done + u64::from(available) > max_bytes {
                    break 'body Err(NetError::Oversize);
                }
                let mut chunk = vec![0u8; available as usize];
                let mut read: u32 = 0;
                if WinHttpReadData(request, chunk.as_mut_ptr().cast(), available, &mut read)
                    .is_err()
                    || read == 0
                {
                    break 'body Err(NetError::Network("WinHttpReadData failed".into()));
                }
                chunk.truncate(read as usize);
                if let Err(error) = std::io::Write::write_all(&mut file, &chunk) {
                    break 'body Err(NetError::Write(error.to_string()));
                }
                done += read as u64;
                progress(done, total);
            }
            match std::io::Write::flush(&mut file) {
                Ok(()) => Ok(()),
                Err(error) => Err(NetError::Write(error.to_string())),
            }
        }
    };
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}
