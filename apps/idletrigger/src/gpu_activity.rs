//! Foreground GPU activity, sampled through the SDK PDH layout.
use std::time::Duration;
use windows::Win32::System::Performance::*;
use windows::core::PCWSTR;

const COUNTER_PATH: &str = "\\GPU Engine(*)\\Utilization Percentage";
struct QueryGuard(PDH_HQUERY);
impl Drop for QueryGuard {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.0);
        }
    }
}

pub fn foreground_gpu_active() -> bool {
    let Ok(pid) = get_foreground_pid() else {
        return false;
    };
    let path: Vec<u16> = COUNTER_PATH.encode_utf16().chain([0]).collect();
    unsafe {
        let mut query = PDH_HQUERY::default();
        if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
            return false;
        }
        let _guard = QueryGuard(query);
        let mut counter = PDH_HCOUNTER::default();
        if PdhAddEnglishCounterW(query, PCWSTR(path.as_ptr()), 0, &mut counter) != 0 {
            return false;
        }
        // Rate counters require a baseline before either measured interval.
        if PdhCollectQueryData(query) != 0 {
            return false;
        }
        for _ in 0..2 {
            std::thread::sleep(Duration::from_millis(500));
            if get_foreground_pid() != Ok(pid) || PdhCollectQueryData(query) != 0 {
                return false;
            }
            if utilization(counter, pid) < 15.0 {
                return false;
            }
        }
        true
    }
}

fn matches_engine(name: &str, pid: u32) -> bool {
    name.starts_with(&format!("pid_{pid}_"))
        && (name.ends_with("engtype_3D") || name.ends_with("engtype_Graphics"))
}

unsafe fn utilization(counter: PDH_HCOUNTER, pid: u32) -> f64 {
    unsafe {
        let mut size = 0;
        let mut count = 0;
        if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None)
            != PDH_MORE_DATA
            || size == 0
        {
            return 0.0;
        }
        // Typed storage supplies SDK alignment and includes space for names.
        let stride = size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        let mut storage =
            vec![PDH_FMT_COUNTERVALUE_ITEM_W::default(); (size as usize).div_ceil(stride)];
        if PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut size,
            &mut count,
            Some(storage.as_mut_ptr()),
        ) != 0
            || count as usize > storage.len()
            || size as usize > storage.len() * stride
            || (count as usize) * stride > size as usize
        {
            return 0.0;
        }
        let start = storage.as_ptr() as usize;
        let end = start + size as usize;
        let mut total = 0.0;
        for item in storage.iter().take(count as usize) {
            if !matches!(
                item.FmtValue.CStatus,
                PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
            ) {
                continue;
            }
            let addr = item.szName.0 as usize;
            if addr < start || addr >= end || !addr.is_multiple_of(align_of::<u16>()) {
                continue;
            }
            let chars = std::slice::from_raw_parts(item.szName.0, (end - addr) / 2);
            let Some(len) = chars.iter().position(|c| *c == 0) else {
                continue;
            };
            if matches_engine(&String::from_utf16_lossy(&chars[..len]), pid) {
                let value = item.FmtValue.Anonymous.doubleValue;
                if value.is_finite() && value >= 0.0 {
                    total += value;
                }
            }
        }
        total
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn engine_matches_whole_pid_and_supported_engine() {
        assert!(matches_engine(
            "pid_12_luid_0x0_phys_0_eng_1_engtype_3D",
            12
        ));
        assert!(!matches_engine(
            "pid_123_luid_0x0_phys_0_eng_1_engtype_3D",
            12
        ));
        assert!(!matches_engine(
            "pid_12_luid_0x0_phys_0_eng_1_engtype_Copy",
            12
        ));
    }
}
