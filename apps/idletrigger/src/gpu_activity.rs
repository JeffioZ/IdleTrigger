//! GPU activity detection for theme-switch pause — Go `gpu_activity.go`
//! parity. Uses PDH with the English counter path `\GPU Engine(*)\Utilization
//! Percentage` to measure the foreground process's 3D+graphics engine
//! utilization. Two samples at 500ms; both ≥15% = sustained activity
//! (windowed games pause theme switching without being fullscreen).

use std::time::Duration;

use windows::core::PCWSTR;

// PDH raw FFI: the windows crate has no PDH bindings.
#[link(name = "pdh")]
unsafe extern "system" {
    fn PdhOpenQueryW(
        szdatasource: PCWSTR,
        dwuserdata: *mut core::ffi::c_void,
        phquery: *mut isize,
    ) -> i32;
    fn PdhAddEnglishCounterW(
        hquery: isize,
        szfullcounterpath: PCWSTR,
        dwuserdata: *mut core::ffi::c_void,
        phcounter: *mut isize,
    ) -> i32;
    fn PdhCollectQueryData(hquery: isize) -> i32;
    fn PdhGetFormattedCounterArrayW(
        hcounter: isize,
        dwwtype: u32,
        lpdwbuffersize: *mut u32,
        lpdwitemcount: *mut u32,
        itembuffer: *mut u8,
    ) -> i32;
    fn PdhCloseQuery(hquery: isize) -> i32;
}

const PDH_FMT_DOUBLE: u32 = 0x00000200;
const ERROR_SUCCESS: i32 = 0;
const PDH_MORE_DATA: i32 = 0x800007D2u32 as i32;
const COUNTER_PATH: &str = "\\GPU Engine(*)\\Utilization Percentage";
const SUSTAINED_THRESHOLD: f64 = 15.0;
const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);
const SAMPLE_COUNT: usize = 2;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct PdhFmtCounterValueDouble {
    csize: u32,
    union: f64,
    cstatus: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct PdhFmtCounterItemW {
    szname: *mut u16,
    fmtvalue: PdhFmtCounterValueDouble,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// True when the current foreground process shows sustained GPU activity
/// (windowed games/apps). Call from a background thread only — this blocks
/// for ~1s (2 × 500ms samples).
pub fn foreground_gpu_active() -> bool {
    let Ok(foreground_pid) = get_foreground_pid() else {
        return false;
    };
    let path = wide(COUNTER_PATH);

    unsafe {
        let mut query: isize = 0;
        if PdhOpenQueryW(PCWSTR::null(), std::ptr::null_mut(), &mut query) != ERROR_SUCCESS {
            return false;
        }
        let _guard = scopeguard_close(query);

        let mut counter: isize = 0;
        if PdhAddEnglishCounterW(
            query,
            PCWSTR(path.as_ptr()),
            std::ptr::null_mut(),
            &mut counter,
        ) != ERROR_SUCCESS
        {
            return false;
        }

        // Collect 2 samples with 500ms gap.
        let mut values = [0f64; SAMPLE_COUNT];
        for (i, slot) in values.iter_mut().enumerate() {
            if i > 0 {
                std::thread::sleep(SAMPLE_INTERVAL);
            }
            if PdhCollectQueryData(query) != ERROR_SUCCESS {
                return false;
            }
            // Re-check foreground between samples (Go re-checks PID each sample).
            if get_foreground_pid().is_ok_and(|pid| pid != foreground_pid) {
                return false;
            }
            *slot = counter_utilization_for_pid(counter, foreground_pid);
        }

        values.iter().all(|v| *v >= SUSTAINED_THRESHOLD)
    }
}

struct QueryGuard(isize);
impl Drop for QueryGuard {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.0);
        }
    }
}

fn scopeguard_close(query: isize) -> QueryGuard {
    QueryGuard(query)
}

/// Sums engtype_3D + engtype_Graphics utilization for the given PID.
unsafe fn counter_utilization_for_pid(counter: isize, pid: u32) -> f64 {
    unsafe {
        // First call to get buffer size.
        let mut buf_size: u32 = 0;
        let mut item_count: u32 = 0;
        let hr = PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut buf_size,
            &mut item_count,
            std::ptr::null_mut(),
        );
        if hr != PDH_MORE_DATA || buf_size == 0 {
            return 0.0;
        }
        let mut buffer = vec![0u8; buf_size as usize];
        let hr = PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut buf_size,
            &mut item_count,
            buffer.as_mut_ptr(),
        );
        if hr != ERROR_SUCCESS || item_count == 0 {
            return 0.0;
        }

        let item_size = std::mem::size_of::<PdhFmtCounterItemW>();
        let mut total = 0f64;
        for i in 0..item_count as usize {
            let offset = i * item_size;
            if offset + item_size > buffer.len() {
                break;
            }
            let item = &*(buffer.as_ptr().add(offset) as *const PdhFmtCounterItemW);
            if item.szname.is_null() {
                continue;
            }
            // Instance name format: "pid_X;type_..." — check for pid and engine type.
            let name = read_wide(item.szname);
            if !name.contains(&format!("pid_{pid}")) {
                continue;
            }
            if name.contains("engtype_3D") || name.contains("engtype_Graphics") {
                total += item.fmtvalue.union;
            }
        }
        total
    }
}

unsafe fn read_wide(ptr: *mut u16) -> String {
    unsafe {
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        String::from_utf16_lossy(slice)
    }
}

fn get_foreground_pid() -> Result<u32, ()> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    unsafe {
        let fg: HWND = GetForegroundWindow();
        if fg.is_invalid() {
            return Err(());
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == 0 {
            return Err(());
        }
        Ok(pid)
    }
}
