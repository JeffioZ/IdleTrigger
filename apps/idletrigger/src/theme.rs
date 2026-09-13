//! Theme follow: system light/dark detection, palette values from the Go
//! `internal/ui/colors` palette, and DWM title-bar theming (attribute 20
//! with a fallback to legacy attribute 19 for Windows 10 1809–1909).

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
const LIGHT_BG: u32 = 0x00FAF8F6; // RGB(246,248,250)
const LIGHT_TEXT: u32 = 0x00241E19; // RGB(25,30,36)
const LIGHT_SUBTLE: u32 = 0x005E5246; // RGB(70,82,94)
const DARK_BG: u32 = 0x002A2420; // RGB(32,36,42)
const DARK_TEXT: u32 = 0x00FAF7F4; // RGB(244,247,250)
const DARK_SUBTLE: u32 = 0x00DCD4CC; // RGB(204,212,220)

// Choice-popup-batch fields carry allow(dead_code) until that batch lands.
/// Full Go palette (colors/palette.go Palette) for owner-drawn controls.
/// Values are COLORREF (0x00BBGGRR), converted to ARGB at the GDI+ edge.
#[allow(dead_code)]
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
    border: 0x00958D7D,           // RGB(125,137,149)
    subtle_border: 0x004D443C,    // RGB(60,68,77)
    text: 0x00FAF7F4,             // RGB(244,247,250)
    text2: 0x00DCD4CC,            // RGB(204,212,220)
    muted: 0x00BFB5AA,            // RGB(170,181,191)
    disabled_text: 0x008D8277,    // RGB(119,130,141)
    disabled_surface: 0x00312C28, // RGB(40,44,49)
    accent: 0x00B4780A,           // RGB(10,120,180)
    accent_hover: 0x00CB8B0C,     // RGB(12,139,203)
    accent_pressed: 0x009D6806,   // RGB(6,104,157)
    selected: 0x00B4780A,
    selected_hover: 0x00CB8B0C,
    accent_text: 0x00FFFFFF,
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
#[allow(dead_code)]
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
    if is_dark() {
        &DARK_PALETTE
    } else {
        &LIGHT_PALETTE
    }
}

static DARK: AtomicBool = AtomicBool::new(false);

/// Send/Sync wrapper for a GDI brush handle. HBRUSH values are process-wide
/// kernel-backed handles usable from any thread; the raw pointer type just
/// isn't annotated as such.
#[derive(Clone, Copy)]
struct SendBrush(HBRUSH);
unsafe impl Send for SendBrush {}
unsafe impl Sync for SendBrush {}

/// Pre-created light/dark brushes, filled on first use. Created once, never
/// destroyed — no rebuild, no leak, safe from any thread.
static BRUSHES: OnceLock<(SendBrush, SendBrush)> = OnceLock::new();

pub fn is_dark() -> bool {
    DARK.load(Ordering::SeqCst)
}

pub fn bg_color() -> u32 {
    if is_dark() { DARK_BG } else { LIGHT_BG }
}

pub fn text_color() -> u32 {
    if is_dark() { DARK_TEXT } else { LIGHT_TEXT }
}

pub fn subtle_color() -> u32 {
    if is_dark() { DARK_SUBTLE } else { LIGHT_SUBTLE }
}

/// Brush for window backgrounds and WM_CTLCOLORSTATIC.
pub fn bg_brush() -> HBRUSH {
    let brushes = BRUSHES.get_or_init(|| unsafe {
        (
            SendBrush(CreateSolidBrush(COLORREF(LIGHT_BG))),
            SendBrush(CreateSolidBrush(COLORREF(DARK_BG))),
        )
    });
    (if is_dark() { brushes.1 } else { brushes.0 }).0
}

/// Devtools capture support: force a theme regardless of the system value.
#[cfg(feature = "devtools")]
pub fn force_dark(value: bool) {
    DARK.store(value, Ordering::SeqCst);
}

/// Reads the registry light/dark preference. Returns true when it changed.
pub fn refresh_from_registry() -> bool {
    let dark = read_apps_use_light_theme()
        .map(|light| !light)
        .unwrap_or(false);
    dark != DARK.swap(dark, Ordering::SeqCst)
}

fn read_apps_use_light_theme() -> Option<bool> {
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"
        .encode_utf16()
        .chain([0])
        .collect();
    let value: Vec<u16> = "AppsUseLightTheme".encode_utf16().chain([0]).collect();
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
        let ok = RegQueryValueExW(
            hkey,
            PCWSTR(value.as_ptr()),
            None,
            None,
            Some(data.as_mut_ptr()),
            Some(&mut size),
        ) == ERROR_SUCCESS;
        let _ = RegCloseKey(hkey);
        if ok && size >= 4 {
            Some(u32::from_le_bytes(data) != 0)
        } else {
            None
        }
    }
}

// Go palette accents and tooltip colors (colors/palette.go) — COLORREF
// (0x00BBGGRR).
#[allow(dead_code)]
const LIGHT_ACCENT: u32 = 0x00B5_7600; // RGB(0, 118, 181)
#[allow(dead_code)]
const DARK_ACCENT: u32 = 0x00B4_780A; // RGB(10, 120, 180)
const LIGHT_TOOLTIP_BG: u32 = 0x00FF_FDFB; // RGB(251, 253, 255)
const LIGHT_TOOLTIP_TEXT: u32 = 0x0024_1E19; // RGB(25, 30, 36)
const DARK_TOOLTIP_BG: u32 = 0x0043_3B34; // RGB(52, 59, 67)
const DARK_TOOLTIP_TEXT: u32 = 0x00FA_F7F4; // RGB(244, 247, 250)

#[allow(dead_code)]
pub fn accent_color() -> u32 {
    if is_dark() { DARK_ACCENT } else { LIGHT_ACCENT }
}

pub fn tooltip_bg_color() -> u32 {
    if is_dark() {
        DARK_TOOLTIP_BG
    } else {
        LIGHT_TOOLTIP_BG
    }
}

pub fn tooltip_text_color() -> u32 {
    if is_dark() {
        DARK_TOOLTIP_TEXT
    } else {
        LIGHT_TOOLTIP_TEXT
    }
}

/// Applies the immersive dark-mode title bar and forces a repaint. Attribute
/// 20 covers Windows 10 20H1+ and Windows 11; 1809–1909 only know attribute
/// 19, so we fall back when 20 reports failure (Go two-step behavior).
pub fn apply_to_window(hwnd: HWND) {
    unsafe {
        set_preferred_app_mode_allow_dark();
        if is_dark() {
            allow_dark_for_window(hwnd, true);
        }
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
    set_process_menu_theme(is_dark());
    let mut windows = vec![
        crate::hwnd(&crate::PANEL),
        crate::hwnd(&crate::WARNING),
        crate::hwnd(&crate::ACTION_WARN_HWND),
        crate::hwnd(&crate::LOCK_NOTIFY_HWND),
        crate::settings_ui::theme_hwnd(),
    ];
    windows.extend(crate::automation_ui::theme_hwnds());
    for hwnd in windows {
        if !hwnd.is_invalid() {
            apply_to_window(hwnd);
            retheme_children(hwnd);
        }
    }
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
    let dark = is_dark();
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
    unsafe {
        windows::Win32::System::LibraryLoader::GetProcAddress(
            module,
            windows::core::PCSTR(ordinal as usize as *const u8),
        )
    }
}

/// uxtheme ordinal 133 (AllowDarkModeForWindow) — the companion switch that
/// lets DarkMode_Explorer classes render on a window. Go darkmode.AllowWindow
/// parity; harmless when the export is absent.
fn allow_dark_for_window(hwnd: HWND, allow: bool) {
    type AllowDarkModeForWindow = unsafe extern "system" fn(HWND, BOOL) -> BOOL;
    static FN: OnceLock<Option<AllowDarkModeForWindow>> = OnceLock::new();
    let func = *FN.get_or_init(|| unsafe {
        let uxtheme =
            windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::w!("uxtheme.dll"))
                .ok()?;
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
        let uxtheme =
            windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::w!("uxtheme.dll"))
                .ok()?;
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
        let uxtheme =
            windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::w!("uxtheme.dll"))
                .ok();
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
    crate::log_line(&format!("set_process_menu_theme called, dark={}", dark));
    unsafe {
        type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
        let uxtheme =
            windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::w!("uxtheme.dll"))
                .ok();
        if let Some(ux) = uxtheme
            && let Some(func) = get_proc_by_ordinal(ux, 135)
                .map(|p| std::mem::transmute::<_, SetPreferredAppMode>(p))
        {
            let mode = if dark { 2 } else { 3 }; // ForceDark or ForceLight
            let result = func(mode);
            crate::log_line(&format!(
                "SetPreferredAppMode({}) returned {}",
                mode, result
            ));
        }
    }
    prepare_popup_menu(HWND(std::ptr::null_mut()), dark);
}

/// SetPreferredAppMode with an explicit preference (AllowDark=1 when dark,
/// ForceLight=3 when light) — popup menus need the exact mode, not just
/// permission.
fn set_preferred_app_mode_for(dark: bool) {
    type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
    static FN: OnceLock<Option<SetPreferredAppMode>> = OnceLock::new();
    let func = *FN.get_or_init(|| unsafe {
        let uxtheme =
            windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::w!("uxtheme.dll"))
                .ok()?;
        get_proc_by_ordinal(uxtheme, 135).map(|p| std::mem::transmute(p))
    });
    if let Some(func) = func {
        // Go forcedThemePreference: ForceDark=2 when dark, ForceLight=3 when
        // light (AllowDark=1 alone does not repaint existing menus).
        let mode = if dark { 2 } else { 3 };
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
