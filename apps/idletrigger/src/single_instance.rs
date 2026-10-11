//! One resident application per Windows session, independent of EXE path.
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, GetLastError, HANDLE,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE,
};
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

/// Read-only presence check: a busy or inaccessible pipe is not proof that
/// the tray is absent. Do not create singleton objects in a copy helper.
pub(crate) fn running() -> Result<bool, String> {
    let mut session = 0;
    unsafe { ProcessIdToSessionId(std::process::id(), &mut session) }.map_err(|e| e.to_string())?;
    running_named(
        &format!("Local\\IdleTrigger-{session}"),
        "Local\\IdleTriggerSingleton",
    )
}

fn running_named(mutex: &str, legacy_event: &str) -> Result<bool, String> {
    for (name, is_mutex) in [(mutex, true), (legacy_event, false)] {
        let name = crate::wide(name);
        let result = unsafe {
            if is_mutex {
                OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(name.as_ptr()))
            } else {
                OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(name.as_ptr()))
            }
        };
        match result {
            Ok(handle) => {
                let _ = unsafe { CloseHandle(handle) };
                return Ok(true);
            }
            Err(err)
                if err.code() == windows::core::HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) => {}
            Err(err) => return Err(err.to_string()),
        }
    }
    Ok(false)
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
        assert!(running_named(&mutex, &event).unwrap());
        assert!(acquire_named(&mutex, &event).unwrap().is_none());
        unsafe { CloseHandle(old) }.unwrap();
        let old =
            unsafe { CreateEventW(None, true, false, PCWSTR(wide(&event).as_ptr())) }.unwrap();
        assert!(running_named(&mutex, &event).unwrap());
        assert!(acquire_named(&mutex, &event).unwrap().is_none());
        unsafe { CloseHandle(old) }.unwrap();
        let guard = acquire_named(&mutex, &event).unwrap().unwrap();
        assert!(acquire_named(&mutex, &event).unwrap().is_none());
        drop(guard);
        assert!(!running_named(&mutex, &event).unwrap());
        assert!(acquire_named(&mutex, &event).unwrap().is_some());
        assert!(acquire_named("invalid\0name", &event).is_err());
    }
}
