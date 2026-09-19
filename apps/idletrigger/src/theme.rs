//! System light/dark detection, application palettes, and DWM title-bar
//! theming (attribute 20, falling back to 19 on older Windows 10 builds).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS, HWND, LPARAM};
use windows::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, HBRUSH, InvalidateRect};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetClassNameW};
use windows::core::{BOOL, PCWSTR};

// Go palette tokens (colors/palette.go).

/// Full Go palette (colors/palette.go Palette) for owner-drawn controls.
/// Values are COLORREF (0x00BBGGRR), converted to ARGB at the GDI+ edge.
pub struct Palette {
    pub window_bg: u32,
    pub surface: u32,
    pub elevated: u32,
    pub hover_surface: u32,
    pub border: u32,
    pub subtle_border: u32,
    pub text: u32,
    pub text2: u32,
    pub muted: u32,
    pub disabled_text: u32,
    pub disabled_surface: u32,
    pub accent: u32,
    pub accent_hover: u32,
    pub accent_pressed: u32,
    pub selected: u32,
    pub selected_hover: u32,
    pub accent_text: u32,
    pub link: u32,
    pub link_hover: u32,
    pub link_pressed: u32,
    pub focus: u32,
    pub danger_bg: u32,
    pub danger_hover: u32,
    pub danger_pressed: u32,
    pub danger_border: u32,
    pub danger_hover_border: u32,
    pub danger_pressed_border: u32,
    pub danger_text: u32,
    pub danger_surface_text: u32,
    pub danger_focus: u32,
    pub tooltip_bg: u32,
    pub tooltip_text: u32,
}

const LIGHT_PALETTE: Palette = Palette {
    window_bg: 0x00FAF8F6,        // RGB(246,248,250)
    surface: 0x00FFFFFF,          // RGB(255,255,255)
    elevated: 0x00FFFDFB,         // RGB(251,253,255)
    hover_surface: 0x00F9F4EA,    // RGB(234,244,249)
    border: 0x009C9084,           // RGB(132,144,156)
    subtle_border: 0x00EDE7E1,    // RGB(225,231,237)
    text: 0x00241E19,             // RGB(25,30,36)
    text2: 0x005E5246,            // RGB(70,82,94)
    muted: 0x00766C63,            // RGB(99,108,118)
    disabled_text: 0x0093897E,    // RGB(126,137,147)
    disabled_surface: 0x00F5F2EE, // RGB(238,242,245)
    accent: 0x00B57600,           // RGB(0,118,181)
    accent_hover: 0x00A36A00,     // RGB(0,106,163)
    accent_pressed: 0x00855500,   // RGB(0,85,133)
    selected: 0x00B57600,
    selected_hover: 0x00A36A00,
    accent_text: 0x00FFFFFF,
    link: 0x00B57600,
    link_hover: 0x00A36A00,
    link_pressed: 0x00855500,
    focus: 0x00865A00,                 // RGB(0,90,134)
    danger_bg: 0x003934AE,             // RGB(174,52,57)
    danger_hover: 0x004446C8,          // RGB(200,68,68)
    danger_pressed: 0x002D2784,        // RGB(132,39,45)
    danger_border: 0x008084EF,         // RGB(239,132,128)
    danger_hover_border: 0x00AFB4FF,   // RGB(255,180,175)
    danger_pressed_border: 0x006A69D5, // RGB(213,105,106)
    danger_text: 0x00FBFAFF,           // RGB(255,250,251)
    danger_surface_text: 0x002F279A,   // RGB(154,39,47)
    danger_focus: 0x00E3E5FF,          // RGB(255,229,227)
    tooltip_bg: 0x00FFFDFB,            // RGB(251,253,255)
    tooltip_text: 0x00241E19,
};

const DARK_PALETTE: Palette = Palette {
    window_bg: 0x002A2420,        // RGB(32,36,42)
    surface: 0x0036302B,          // RGB(43,48,54)
    elevated: 0x00433B34,         // RGB(52,59,67)
    hover_surface: 0x00453D36,    // RGB(54,61,69)
    border: 0x008E8072,           // RGB(114,128,142)
    subtle_border: 0x004D443C,    // RGB(60,68,77)
    text: 0x00FAF7F4,             // RGB(244,247,250)
    text2: 0x00DCD4CC,            // RGB(204,212,220)
    muted: 0x00B7AA9F,            // RGB(159,170,183)
    disabled_text: 0x008D8277,    // RGB(119,130,141)
    disabled_surface: 0x00312C28, // RGB(40,44,49)
    accent: 0x00B4780A,           // RGB(10,120,180)
    accent_hover: 0x00CB8B0C,     // RGB(12,139,203)
    accent_pressed: 0x009D6806,   // RGB(6,104,157)
    selected: 0x00B4780A,
    selected_hover: 0x00B67B08, // Keep white button labels readable on hover.
    accent_text: 0x00FFFFFF,
    link: 0x00ECBC59,                  // RGB(89,188,236), text on dark surfaces.
    link_hover: 0x00FAD585,            // RGB(133,213,250)
    link_pressed: 0x00DDA736,          // RGB(54,167,221)
    focus: 0x00EDCD51,                 // RGB(81,205,237)
    danger_bg: 0x00423FB8,             // RGB(184,63,66)
    danger_hover: 0x004446C8,          // RGB(200,68,68)
    danger_pressed: 0x00373293,        // RGB(147,50,55)
    danger_border: 0x008082E7,         // RGB(231,130,128)
    danger_hover_border: 0x00A6AAF1,   // RGB(241,170,166)
    danger_pressed_border: 0x006A69D5, // RGB(213,105,106)
    danger_text: 0x00FBFAFF,           // RGB(255,250,251)
    danger_surface_text: 0x009796F2,   // RGB(242,150,151)
    danger_focus: 0x00E3E5FF,          // RGB(255,229,227)
    tooltip_bg: 0x00433B34,            // RGB(52,59,67)
    tooltip_text: 0x00FAF7F4,
};

/// One (surface, disabled-surface) brush pair, shareable across threads.
#[derive(Clone, Copy)]
pub struct SurfaceBrushPair(
    pub windows::Win32::Graphics::Gdi::HBRUSH,
    pub windows::Win32::Graphics::Gdi::HBRUSH,
);
unsafe impl Send for SurfaceBrushPair {}
unsafe impl Sync for SurfaceBrushPair {}

/// Both themes' edit-interior brush pairs, built once (a single-theme cache
/// keeps a stale white surface in dark mode after theme flips).
pub fn surface_brush_pairs() -> (SurfaceBrushPair, SurfaceBrushPair) {
    if let Some(p) = crate::theme_contrast::palette() {
        let pair = SurfaceBrushPair(cached_brush(p.surface), cached_brush(p.disabled_surface));
        return (pair, pair);
    }
    use std::sync::OnceLock;
    static PAIRS: OnceLock<(SurfaceBrushPair, SurfaceBrushPair)> = OnceLock::new();
    *PAIRS.get_or_init(|| unsafe {
        (
            SurfaceBrushPair(
                windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(LIGHT_PALETTE.surface)),
                windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(
                    LIGHT_PALETTE.disabled_surface,
                )),
            ),
            SurfaceBrushPair(
                windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(DARK_PALETTE.surface)),
                windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(
                    DARK_PALETTE.disabled_surface,
                )),
            ),
        )
    })
}

/// Live palette for the active theme.
pub fn palette() -> &'static Palette {
    if let Some(p) = crate::theme_contrast::palette() {
        return p;
    }
    if is_dark() {
        &DARK_PALETTE
    } else {
        &LIGHT_PALETTE
    }
}

static DARK: AtomicBool = AtomicBool::new(false);

pub fn is_dark() -> bool {
    DARK.load(Ordering::SeqCst)
}

pub fn bg_color() -> u32 {
    palette().window_bg
}

pub fn text_color() -> u32 {
    palette().text
}

pub fn subtle_color() -> u32 {
    palette().text2
}

/// Brush for window backgrounds and WM_CTLCOLORSTATIC.
pub fn bg_brush() -> HBRUSH {
    cached_brush(bg_color())
}

fn cached_brush(color: u32) -> HBRUSH {
    static CACHE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<u32, isize>>> =
        std::sync::LazyLock::new(Default::default);
    HBRUSH(
        *CACHE
            .lock()
            .unwrap()
            .entry(color)
            .or_insert_with(|| unsafe { CreateSolidBrush(COLORREF(color)).0 as isize })
            as *mut _,
    )
}

/// Devtools capture support: force a theme regardless of the system value.
#[cfg(any(feature = "devtools", test))]
pub fn force_dark(value: bool) {
    DARK.store(value, Ordering::SeqCst);
}

/// Reads the registry light/dark preference. Returns true when it changed.
pub fn refresh_from_registry() -> bool {
    let contrast = crate::theme_contrast::refresh();
    let dark = read_light_preference("AppsUseLightTheme")
        .map(|light| !light)
        .unwrap_or(false);
    (dark != DARK.swap(dark, Ordering::SeqCst)) | contrast
}

/// HKCU subkey holding the system light/dark preferences.
pub(crate) const PERSONALIZE_KEY: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";

/// Reads a REG_DWORD under HKCU; only a well-formed 4-byte DWORD counts.
pub(crate) fn read_registry_dword(subkey: &str, value: &str) -> Option<u32> {
    let subkey = crate::wide(subkey);
    let value = crate::wide(value);
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut data = [0u8; 4];
        let mut size = 4u32;
        let mut kind = windows::Win32::System::Registry::REG_VALUE_TYPE::default();
        let ok = RegQueryValueExW(
            hkey,
            PCWSTR(value.as_ptr()),
            None,
            Some(&mut kind),
            Some(data.as_mut_ptr()),
            Some(&mut size),
        ) == ERROR_SUCCESS;
        let _ = RegCloseKey(hkey);
        (ok && size == 4 && kind == windows::Win32::System::Registry::REG_DWORD)
            .then(|| u32::from_le_bytes(data))
    }
}

pub fn read_light_preference(name: &str) -> Option<bool> {
    read_registry_dword(PERSONALIZE_KEY, name).map(|value| value != 0)
}

pub fn tooltip_bg_color() -> u32 {
    palette().tooltip_bg
}

pub fn tooltip_text_color() -> u32 {
    palette().tooltip_text
}

/// Applies the immersive dark-mode title bar and forces a repaint. Attribute
/// 20 covers Windows 10 20H1+ and Windows 11; 1809–1909 only know attribute
/// 19, so we fall back when 20 reports failure (Go two-step behavior).
pub fn apply_to_window(hwnd: HWND) {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{GCLP_HBRBACKGROUND, GCLP_HMODULE};
        #[cfg(target_pointer_width = "64")]
        use windows::Win32::UI::WindowsAndMessaging::{GetClassLongPtrW, SetClassLongPtrW};
        #[cfg(target_pointer_width = "32")]
        use windows::Win32::UI::WindowsAndMessaging::{
            GetClassLongW as GetClassLongPtrW, SetClassLongW as SetClassLongPtrW,
        };
        // Keep USER32's fallback erase color in sync while a retained window
        // is hidden. Only change classes owned by this executable that already
        // have a background; native controls and layered cards own theirs.
        if let Ok(module) = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
            && GetClassLongPtrW(hwnd, GCLP_HMODULE) as *mut core::ffi::c_void == module.0
            && GetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND) != 0
        {
            // Palette brushes are cached for process lifetime, not owned by
            // the window class, so the replaced brush must not be deleted.
            SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, bg_brush().0 as _);
        }
        set_preferred_app_mode_allow_dark();
        allow_dark_for_window(hwnd, is_dark());
        let value: i32 = if is_dark() { 1 } else { 0 };
        let attr20 = DwmSetWindowAttribute(
            hwnd,
            windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(DWMWA_USE_IMMERSIVE_DARK_MODE as i32),
            &value as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
        if attr20.is_err() {
            let _ = DwmSetWindowAttribute(
                hwnd,
                windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(
                    DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY as i32,
                ),
                &value as *const i32 as *const core::ffi::c_void,
                std::mem::size_of::<i32>() as u32,
            );
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

pub fn apply_to_all() {
    apply_to_all_with_force(false);
}

/// A Windows theme-file repair can invalidate native styles without changing
/// our semantic palette (Go RefreshThemeAfterSystemRepair).
pub fn apply_to_all_after_repair() {
    apply_to_all_with_force(true);
}

fn apply_to_all_with_force(force: bool) {
    // DWM/SetWindowTheme can synchronously deliver another theme message.
    // Do not enter the refresh again while native controls are being updated.
    thread_local! { static REFRESHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
    if REFRESHING.replace(true) {
        return;
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            REFRESHING.set(false);
        }
    }
    let _reset = Reset;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::UI::WindowsAndMessaging::{GetPropW, IsWindow, SetPropW};
    // Palettes have process lifetime (including high-contrast palettes).
    // Window properties are discarded by USER32 when an HWND is destroyed.
    let palette_key = HANDLE(palette() as *const Palette as *mut _);
    let dark_key = HANDLE((1 + usize::from(is_dark())) as *mut _);
    let palette_prop = windows::core::w!("IdleTrigger.ThemePalette");
    let dark_prop = windows::core::w!("IdleTrigger.ThemeDark");
    let mut windows = vec![
        crate::hwnd(&crate::PANEL),
        crate::hwnd(&crate::WARNING),
        crate::hwnd(&crate::ACTION_WARN_HWND),
        crate::hwnd(&crate::LOCK_NOTIFY_HWND),
        crate::settings_ui::theme_hwnd(),
    ];
    windows.extend(crate::automation_ui::theme_hwnds());
    windows.retain(|hwnd| unsafe {
        !hwnd.is_invalid()
            && IsWindow(Some(*hwnd)).as_bool()
            && (force
                || GetPropW(*hwnd, palette_prop) != palette_key
                || GetPropW(*hwnd, dark_prop) != dark_key)
    });
    if windows.is_empty() {
        return;
    }
    crate::choice::close(false);
    let frames: Vec<_> = windows
        .iter()
        .filter_map(|w| crate::FrameTransition::begin(*w))
        .collect();
    set_process_menu_theme(is_dark());
    let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
        .unwrap_or_default();
    for hwnd in windows {
        let _dpi = crate::dpi::Scope::window(hwnd);
        apply_to_window(hwnd);
        retheme_children(hwnd);
        crate::set_window_icons(hwnd, instance);
        unsafe {
            let _ = SetPropW(hwnd, palette_prop, Some(palette_key));
            let _ = SetPropW(hwnd, dark_prop, Some(dark_key));
        }
    }
    crate::tooltips::retheme();
    crate::settings_ui::retheme_tooltip();
    crate::automation_ui::refresh_theme();
    drop(frames);
}

/// Go nativeform.ApplyControl parity: picks the visual-style class so
/// standard controls (checkbox glyphs, buttons, combo faces) follow the
/// active light/dark mode. COMBOBOX uses the CFD variant; everything else
/// uses Explorer.
pub fn apply_control_theme(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }
    let mut buffer = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
    if len <= 0 {
        return;
    }
    let class = String::from_utf16_lossy(&buffer[..len as usize]).to_uppercase();
    let dark = is_dark() && crate::theme_contrast::palette().is_none();
    let name = if dark {
        if class == "COMBOBOX" {
            "DarkMode_CFD"
        } else {
            "DarkMode_Explorer"
        }
    } else {
        "Explorer"
    };
    set_preferred_app_mode_allow_dark();
    allow_dark_for_window(hwnd, dark);
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let _ = unsafe {
        windows::Win32::UI::Controls::SetWindowTheme(hwnd, PCWSTR(wide.as_ptr()), PCWSTR::null())
    };
}

unsafe extern "system" fn retheme_child_proc(hwnd: HWND, _lparam: LPARAM) -> BOOL {
    unsafe {
        apply_control_theme(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
    BOOL(1)
}

/// Re-applies the control visual theme to every direct and indirect child
/// (Go applyTheme loop over p.controls).
pub fn retheme_children(hwnd: HWND) {
    let _ = unsafe { EnumChildWindows(Some(hwnd), Some(retheme_child_proc), LPARAM(0)) };
}

/// GetProcAddress by ordinal (MAKEINTRESOURCEA equivalent): pass the ordinal
/// as a raw pointer value, not a string.
unsafe fn get_proc_by_ordinal(
    module: windows::Win32::Foundation::HMODULE,
    ordinal: u16,
) -> windows::Win32::Foundation::FARPROC {
    if crate::system::windows_build() < 17763 {
        return None;
    }
    unsafe {
        windows::Win32::System::LibraryLoader::GetProcAddress(
            module,
            windows::core::PCSTR(ordinal as usize as *const u8),
        )
    }
}

fn uxtheme() -> Option<windows::Win32::Foundation::HMODULE> {
    static MODULE: OnceLock<isize> = OnceLock::new();
    let module = *MODULE.get_or_init(|| unsafe {
        windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::w!("uxtheme.dll"))
            .map(|module| module.0 as isize)
            .unwrap_or(0)
    });
    (module != 0).then_some(windows::Win32::Foundation::HMODULE(module as *mut _))
}

/// uxtheme ordinal 133 (AllowDarkModeForWindow) — the companion switch that
/// lets DarkMode_Explorer classes render on a window. Go darkmode.AllowWindow
/// parity; harmless when the export is absent.
fn allow_dark_for_window(hwnd: HWND, allow: bool) {
    type AllowDarkModeForWindow = unsafe extern "system" fn(HWND, BOOL) -> BOOL;
    static FN: OnceLock<Option<AllowDarkModeForWindow>> = OnceLock::new();
    let func = *FN.get_or_init(|| unsafe {
        let uxtheme = uxtheme()?;
        get_proc_by_ordinal(uxtheme, 133).map(|p| std::mem::transmute(p))
    });
    if let Some(func) = func {
        let _ = unsafe { func(hwnd, BOOL::from(allow)) };
    }
}

/// uxtheme ordinal 135 (SetPreferredAppMode, 1 = AllowDark) — process-wide
/// opt-in without which DarkMode_Explorer control classes stay light.
fn set_preferred_app_mode_allow_dark() {
    type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
    static FN: OnceLock<Option<SetPreferredAppMode>> = OnceLock::new();
    static CALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let func = *FN.get_or_init(|| unsafe {
        let uxtheme = uxtheme()?;
        get_proc_by_ordinal(uxtheme, 135).map(|p| std::mem::transmute(p))
    });
    if let Some(func) = func
        && !CALLED.swap(true, Ordering::SeqCst)
    {
        let _ = unsafe { func(1) };
    }
}

/// Prepares native popup menus for the active theme immediately before
/// TrackPopupMenu (Go darkmode.PreparePopupMenu): refresh the immersive
/// color policy, set the process preference, opt the owner window in, and
/// flush cached menu themes so a long-lived tray process cannot keep stale
/// menu colors.
pub fn prepare_popup_menu(owner: HWND, dark: bool) {
    type NoArg = unsafe extern "system" fn();
    type RefreshPolicy = unsafe extern "system" fn();
    static FLUSH: OnceLock<Option<RefreshPolicy>> = OnceLock::new();
    static REFRESH: OnceLock<Option<RefreshPolicy>> = OnceLock::new();
    unsafe {
        let uxtheme = uxtheme();
        let Some(uxtheme) = uxtheme else { return };
        // RefreshImmersiveColorPolicyState — ordinal 104.
        let refresh = *REFRESH.get_or_init(|| {
            get_proc_by_ordinal(uxtheme, 104).map(|p| std::mem::transmute::<_, NoArg>(p))
        });
        if let Some(func) = refresh {
            func();
        }
        set_preferred_app_mode_for(dark);
        allow_dark_for_window(owner, dark);
        // FlushMenuThemes — ordinal 136.
        let flush = *FLUSH.get_or_init(|| {
            get_proc_by_ordinal(uxtheme, 136).map(|p| std::mem::transmute::<_, NoArg>(p))
        });
        if let Some(func) = flush {
            func();
        }
    }
}

/// Process-wide menu theme (Go PreparePopupMenu minus the owner window):
/// refresh the immersive color policy, set the process preference, and flush
/// cached menu themes. Called at startup and on every theme flip so the tray
/// menu (owned by the tray-icon crate) follows dark/light.
pub fn set_process_menu_theme(dark: bool) {
    prepare_popup_menu(HWND::default(), dark);
}

/// SetPreferredAppMode with an explicit preference (AllowDark=1 when dark,
/// ForceLight=3 when light) — popup menus need the exact mode, not just
/// permission.
fn set_preferred_app_mode_for(dark: bool) {
    type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
    static FN: OnceLock<Option<SetPreferredAppMode>> = OnceLock::new();
    let func = *FN.get_or_init(|| unsafe {
        let uxtheme = uxtheme()?;
        get_proc_by_ordinal(uxtheme, 135).map(|p| std::mem::transmute(p))
    });
    if let Some(func) = func {
        // Go forcedThemePreference: ForceDark=2 when dark, ForceLight=3 when
        // light (AllowDark=1 alone does not repaint existing menus).
        let mode = if crate::system::windows_build() < 18362 {
            i32::from(dark)
        } else if dark {
            2
        } else {
            3
        };
        let _ = unsafe { func(mode) };
    }
}

/// Whether a WM_SETTINGCHANGE payload is the color-set broadcast.
pub fn is_color_set_change(lparam: isize) -> bool {
    if lparam == 0 {
        return false;
    }
    unsafe {
        let mut buffer = [0u16; 64];
        let mut index = 0;
        while index < buffer.len() {
            let wide = *(lparam as *const u16).add(index);
            if wide == 0 {
                break;
            }
            buffer[index] = wide;
            index += 1;
        }
        let name = String::from_utf16_lossy(&buffer[..index]);
        name.eq_ignore_ascii_case("ImmersiveColorSet")
    }
}

const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY: u32 = 19;

#[cfg(test)]
mod visual_tests {
    use super::*;

    fn contrast(a: u32, b: u32) -> f64 {
        fn luminance(color: u32) -> f64 {
            [0, 8, 16]
                .into_iter()
                .zip([0.2126, 0.7152, 0.0722])
                .map(|(shift, weight)| {
                    let channel = ((color >> shift) & 255) as f64 / 255.0;
                    weight
                        * if channel <= 0.04045 {
                            channel / 12.92
                        } else {
                            ((channel + 0.055) / 1.055).powf(2.4)
                        }
                })
                .sum()
        }
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn form_colors_remain_readable_in_both_themes_and_selected_states() {
        for p in [&LIGHT_PALETTE, &DARK_PALETTE] {
            for background in [p.selected, p.selected_hover, p.accent_pressed] {
                assert!(contrast(p.accent_text, background) >= 4.5);
            }
            assert!(contrast(p.muted, p.window_bg) >= 4.5);
            assert!(contrast(p.border, p.surface) >= 3.0);
            assert!(contrast(p.focus, p.surface) >= 3.0);
            for background in [p.danger_hover, p.danger_pressed] {
                assert!(contrast(p.danger_focus, background) >= 3.0);
            }
            for ink in [p.link, p.link_hover, p.link_pressed] {
                assert!(contrast(ink, p.window_bg) >= 4.5);
            }
        }
    }
}
