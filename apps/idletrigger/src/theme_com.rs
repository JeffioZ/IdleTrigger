//! Windows theme-manager interfaces used by the Windows 11 repair coordinator.
//!
//! `ThemeManager` is Auto Dark Mode's `IThemeManager2`: undocumented, with
//! vtable slots (init@3, current@11, select@12, custom@13, add_and_select@20,
//! update_custom@26) from community reverse engineering, verified against
//! the builds this repair path runs on (Windows 11 22621+, gated by
//! `full_dwm_refresh_available`). A future Windows build that reshuffles
//! these slots would surface as a failed HRESULT here — collected into the
//! repair error report — not as silent corruption; revisit the ordinals if
//! that error pattern appears.
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
    unused14: [usize; 6],
    add_and_select: unsafe extern "system" fn(*mut c_void, usize, *const u16, u32, u32) -> HRESULT,
    unused21: [usize; 5],
    update_custom: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}
/// `ThemeApplyFlags::IgnoreBackground | IgnoreCursor | IgnoreDesktopIcons |
/// IgnoreSound | IgnoreScreensaver`: everything except color — the section
/// set Auto Dark Mode's DwmRefresh round trip keeps untouched.
pub const IGNORE_MUTABLE_SECTIONS: u32 = 1 | 2 | 4 | 16 | 32;
/// `ThemePackFlags::Silent`: hides progress UI and sounds.
pub const PACK_SILENT: u32 = 1 << 2;

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
    original: i32,
    _apartment: Apartment,
}
impl Session {
    pub fn new() -> std::io::Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            let apartment = Apartment;
            // COM error details may themselves own interfaces. Convert errors
            // while the apartment is alive, just as we release the managers.
            let result = (|| -> windows::core::Result<_> {
                let manager: ThemeManager = CoCreateInstance(
                    &GUID::from_u128(0x9324da94_50ec_4a14_a770_e90ca03e7c8f),
                    None,
                    CLSCTX_ALL,
                )?;
                (manager.vtable().init)(manager.as_raw(), 0).ok()?;
                let mut original = 0;
                (manager.vtable().current)(manager.as_raw(), &mut original).ok()?;
                let mut custom = 0;
                if (manager.vtable().custom)(manager.as_raw(), &mut custom).is_ok()
                    && custom == original
                {
                    (manager.vtable().update_custom)(manager.as_raw()).ok()?;
                }
                Ok((manager, original))
            })();
            let (manager, original) =
                result.map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok(Self {
                manager,
                original,
                _apartment: apartment,
            })
        }
    }
    /// Applies a theme file by path with per-section ignore flags. Unlike
    /// the legacy `ApplyTheme`, ignored sections are never touched — the
    /// property that keeps the colorization round trip away from wallpaper,
    /// cursors, sounds and screensavers.
    pub fn add_and_select(
        &self,
        path: &str,
        apply_flags: u32,
        pack_flags: u32,
    ) -> windows::core::Result<()> {
        let path = BSTR::from(path);
        unsafe {
            (self.manager.vtable().add_and_select)(
                self.manager.as_raw(),
                0,
                path.as_ptr(),
                apply_flags,
                pack_flags,
            )
            .ok()
        }
    }
    pub fn restore(&self) -> windows::core::Result<()> {
        // Preserve wallpaper, cursor, desktop icons, sounds and screensaver.
        unsafe {
            (self.manager.vtable().select)(
                self.manager.as_raw(),
                0,
                self.original,
                1,
                IGNORE_MUTABLE_SECTIONS,
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
            std::mem::offset_of!(ThemeManagerVtbl, add_and_select),
            20 * pointer
        );
        assert_eq!(
            std::mem::offset_of!(ThemeManagerVtbl, update_custom),
            26 * pointer
        );
    }
}
