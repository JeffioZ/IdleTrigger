//! Dynamic MSAA annotations preserve native button input while exposing checkbox semantics.
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::*;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled};
use windows::Win32::UI::WindowsAndMessaging::{EVENT_OBJECT_STATECHANGE, OBJID_CLIENT};

static CHECKS: LazyLock<Mutex<HashMap<isize, (bool, i32)>>> = LazyLock::new(Default::default);

fn services(run: impl FnOnce(&IAccPropServices) -> windows::core::Result<()>) -> bool {
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = CoCreateInstance(&CAccPropServices, None, CLSCTX_INPROC_SERVER)
            .and_then(|service| run(&service))
            .is_ok(); // Release COM-backed error details before CoUninitialize too.
        if initialized {
            CoUninitialize();
        }
        result
    }
}

pub fn check(hwnd: HWND, checked: bool) {
    if hwnd.is_invalid() {
        return;
    }
    unsafe {
        let state = 0x0010_0000
            | if checked { 0x10 } else { 0 }
            | if !IsWindowEnabled(hwnd).as_bool() {
                1
            } else {
                0
            }
            | if GetFocus() == hwnd { 4 } else { 0 };
        let previous = crate::runtime::lock(&CHECKS)
            .get(&(hwnd.0 as isize))
            .copied();
        if previous == Some((checked, state)) {
            return;
        }
        if services(|service| {
            service.SetHwndProp(
                hwnd,
                OBJID_CLIENT.0 as u32,
                0,
                PROPID_ACC_ROLE,
                &VARIANT::from(0x2ci32),
            )?;
            service.SetHwndProp(
                hwnd,
                OBJID_CLIENT.0 as u32,
                0,
                PROPID_ACC_STATE,
                &VARIANT::from(state),
            )
        }) {
            crate::runtime::lock(&CHECKS).insert(hwnd.0 as isize, (checked, state));
            NotifyWinEvent(EVENT_OBJECT_STATECHANGE, hwnd, OBJID_CLIENT.0, 0);
        }
    }
}

pub fn refresh(hwnd: HWND) {
    let checked = crate::runtime::lock(&CHECKS)
        .get(&(hwnd.0 as isize))
        .map(|entry| entry.0);
    if let Some(checked) = checked {
        check(hwnd, checked);
    }
}

pub fn clear(hwnd: HWND) {
    let existed = crate::runtime::lock(&CHECKS)
        .remove(&(hwnd.0 as isize))
        .is_some();
    if existed {
        services(|service| unsafe {
            service.ClearHwndProps(
                hwnd,
                OBJID_CLIENT.0 as u32,
                0,
                &[PROPID_ACC_ROLE, PROPID_ACC_STATE],
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::Interface;
    #[test]
    fn native_accessible_object_reports_check_state_and_disable_changes() {
        let _guard = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::*;
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            let button = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("BUTTON"),
                windows::core::w!("Test checkbox"),
                WS_POPUP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                0,
                0,
                100,
                30,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            crate::nativeform::track(button);
            check(button, false);
            let mut raw = std::ptr::null_mut();
            AccessibleObjectFromWindow(button, OBJID_CLIENT.0 as u32, &IAccessible::IID, &mut raw)
                .unwrap();
            let accessible = IAccessible::from_raw(raw);
            let child = VARIANT::from(0i32);
            assert_eq!(
                i32::try_from(&accessible.get_accRole(&child).unwrap()).unwrap(),
                0x2c
            );
            assert_eq!(
                i32::try_from(&accessible.get_accState(&child).unwrap()).unwrap() & 0x10,
                0
            );
            check(button, true);
            assert_ne!(
                i32::try_from(&accessible.get_accState(&child).unwrap()).unwrap() & 0x10,
                0
            );
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(button, false);
            assert_ne!(
                i32::try_from(&accessible.get_accState(&child).unwrap()).unwrap() & 1,
                0
            );
            drop(accessible);
            DestroyWindow(button).unwrap();
            assert!(!crate::runtime::lock(&CHECKS).contains_key(&(button.0 as isize)));
            CoUninitialize();
        }
    }
}
