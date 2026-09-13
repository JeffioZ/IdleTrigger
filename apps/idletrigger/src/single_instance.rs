//! One resident application per Windows session, independent of EXE path.
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW};
use windows::core::PCWSTR;

pub struct Guard(Vec<HANDLE>);
impl Drop for Guard {
    fn drop(&mut self) {
        for handle in self.0.drain(..) {
            let _ = unsafe { CloseHandle(handle) };
        }
    }
}

pub fn acquire() -> Result<Option<Guard>, String> {
    let mut session = 0;
    unsafe { ProcessIdToSessionId(std::process::id(), &mut session) }.map_err(|e| e.to_string())?;
    acquire_named(
        &format!("Local\\IdleTrigger-{session}"),
        "Local\\IdleTriggerSingleton",
    )
}

fn acquire_named(mutex: &str, legacy_event: &str) -> Result<Option<Guard>, String> {
    let mut guard = Guard(Vec::new());
    for (name, is_mutex) in [(mutex, true), (legacy_event, false)] {
        if name.contains('\0') {
            return Err("instance name contains NUL".into());
        }
        let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
        unsafe {
            let handle = if is_mutex {
                CreateMutexW(None, false, PCWSTR(name.as_ptr()))
            } else {
                // Compatibility with earlier releases using an Event object.
                CreateEventW(None, true, false, PCWSTR(name.as_ptr()))
            }
            .map_err(|e| e.to_string())?;
            let existed = GetLastError() == ERROR_ALREADY_EXISTS;
            guard.0.push(handle);
            if existed {
                return Ok(None);
            }
        }
    }
    Ok(Some(guard))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_both_object_types_and_releases_all_handles() {
        let mutex = format!("Local\\IdleTrigger-mutex-test-{}", std::process::id());
        let event = format!("Local\\IdleTrigger-event-test-{}", std::process::id());
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<_>>();
        let old = unsafe { CreateMutexW(None, false, PCWSTR(wide(&mutex).as_ptr())) }.unwrap();
        assert!(acquire_named(&mutex, &event).unwrap().is_none());
        unsafe { CloseHandle(old) }.unwrap();
        let old =
            unsafe { CreateEventW(None, true, false, PCWSTR(wide(&event).as_ptr())) }.unwrap();
        assert!(acquire_named(&mutex, &event).unwrap().is_none());
        unsafe { CloseHandle(old) }.unwrap();
        let guard = acquire_named(&mutex, &event).unwrap().unwrap();
        assert!(acquire_named(&mutex, &event).unwrap().is_none());
        drop(guard);
        assert!(acquire_named(&mutex, &event).unwrap().is_some());
        assert!(acquire_named("invalid\0name", &event).is_err());
    }
}
