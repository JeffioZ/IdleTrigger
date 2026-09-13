//! Windows theme-manager interfaces used by the Windows 11 repair coordinator.
use std::ffi::c_void;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::{BSTR, GUID, HRESULT, IUnknown_Vtbl, Interface};

#[repr(transparent)]
#[derive(Clone)]
struct ThemeManager(windows::core::IUnknown);
unsafe impl Interface for ThemeManager {
    type Vtable = ThemeManagerVtbl;
    const IID: GUID = GUID::from_u128(0xc1e8c83e_845d_4d95_81db_e283fdffc000);
}
#[repr(C)]
pub struct ThemeManagerVtbl {
    base: IUnknown_Vtbl,
    init: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    unused4: [usize; 7],
    current: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    select: unsafe extern "system" fn(*mut c_void, usize, i32, i32, u32, usize) -> HRESULT,
    custom: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    unused14: [usize; 12],
    update_custom: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}
#[repr(transparent)]
#[derive(Clone)]
struct LegacyThemeManager(windows::core::IUnknown);
unsafe impl Interface for LegacyThemeManager {
    type Vtable = LegacyThemeManagerVtbl;
    const IID: GUID = GUID::from_u128(0x0646ebbe_c1b7_4045_8fd0_ffd65d3fc792);
}
#[repr(C)]
pub struct LegacyThemeManagerVtbl {
    base: IUnknown_Vtbl,
    unused3: usize,
    apply: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

pub struct Session {
    manager: ThemeManager,
    legacy: LegacyThemeManager,
    original: i32,
    _apartment: Apartment,
}
impl Session {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            let apartment = Apartment;
            let manager: ThemeManager = CoCreateInstance(
                &GUID::from_u128(0x9324da94_50ec_4a14_a770_e90ca03e7c8f),
                None,
                CLSCTX_ALL,
            )?;
            (manager.vtable().init)(manager.as_raw(), 0).ok()?;
            let legacy = CoCreateInstance(
                &GUID::from_u128(0xc04b329e_5823_4415_9c93_ba44688947b0),
                None,
                CLSCTX_ALL,
            )?;
            let mut original = 0;
            (manager.vtable().current)(manager.as_raw(), &mut original).ok()?;
            let mut custom = 0;
            if (manager.vtable().custom)(manager.as_raw(), &mut custom).is_ok()
                && custom == original
            {
                (manager.vtable().update_custom)(manager.as_raw()).ok()?;
            }
            Ok(Self {
                manager,
                legacy,
                original,
                _apartment: apartment,
            })
        }
    }
    pub fn apply(&self, path: &str) -> windows::core::Result<()> {
        let path = BSTR::from(path);
        unsafe { (self.legacy.vtable().apply)(self.legacy.as_raw(), path.as_ptr()).ok() }
    }
    pub fn restore(&self) -> windows::core::Result<()> {
        // Preserve wallpaper, cursor, desktop icons, sounds and screensaver.
        const COLOR_ONLY: u32 = 1 | 2 | 4 | 16 | 32;
        unsafe {
            (self.manager.vtable().select)(
                self.manager.as_raw(),
                0,
                self.original,
                1,
                COLOR_ONLY,
                0,
            )
            .ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vtable_slots_match_the_supported_theme_manager_contract() {
        let pointer = size_of::<usize>();
        assert_eq!(std::mem::offset_of!(ThemeManagerVtbl, init), 3 * pointer);
        assert_eq!(
            std::mem::offset_of!(ThemeManagerVtbl, current),
            11 * pointer
        );
        assert_eq!(std::mem::offset_of!(ThemeManagerVtbl, select), 12 * pointer);
        assert_eq!(std::mem::offset_of!(ThemeManagerVtbl, custom), 13 * pointer);
        assert_eq!(
            std::mem::offset_of!(ThemeManagerVtbl, update_custom),
            26 * pointer
        );
        assert_eq!(
            std::mem::offset_of!(LegacyThemeManagerVtbl, apply),
            4 * pointer
        );
    }
}
