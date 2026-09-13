//! Global hotkeys: Ctrl+Win+Shift+S sleep, Win+Shift+L lock,
//! Win+Shift+N toggle Stay Awake — identical to the Go bindings.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
};

pub const HOTKEY_IDS: [u32; 3] = [10, 11, 12];

struct Binding {
    vk: u16,
    modifiers: u32,
}

fn bindings() -> [Binding; 3] {
    [
        Binding {
            vk: b'S' as u16,
            modifiers: MOD_CONTROL.0 | MOD_WIN.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0,
        },
        Binding {
            vk: b'L' as u16,
            modifiers: MOD_WIN.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0,
        },
        Binding {
            vk: b'N' as u16,
            modifiers: MOD_WIN.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0,
        },
    ]
}

/// Registers on the calling thread (must own the hidden window).
pub fn register_all() -> Vec<&'static str> {
    let mut failed = Vec::new();
    unsafe {
        for (index, binding) in bindings().into_iter().enumerate() {
            let ok = RegisterHotKey(
                Some(crate::hwnd(&crate::HIDDEN)),
                HOTKEY_IDS[index] as i32,
                windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS(binding.modifiers),
                binding.vk as u32,
            )
            .is_ok();
            if !ok {
                failed.push(["Ctrl+Win+Shift+S", "Win+Shift+L", "Win+Shift+N"][index]);
            }
        }
    }
    failed
}

pub fn unregister_all() {
    unsafe {
        for id in HOTKEY_IDS {
            let _ = UnregisterHotKey(Some(crate::hwnd(&crate::HIDDEN)), id as i32);
        }
    }
}

// ---- Autostart --------------------------------------------------------

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::PCWSTR;

const VALUE_NAME: &str = "IdleTrigger";
const SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

pub fn autostart_is_enabled() -> bool {
    unsafe {
        let subkey = wide(SUBKEY);
        let name = wide(VALUE_NAME);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return false;
        }
        let mut size: u32 = 0;
        let result = RegQueryValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            None,
            None,
            None,
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);
        result == ERROR_SUCCESS
    }
}

pub fn autostart_enable(exe_path: &str) -> bool {
    unsafe {
        let subkey = wide(SUBKEY);
        let name = wide(VALUE_NAME);
        let mut hkey = HKEY::default();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        ) != ERROR_SUCCESS
        {
            return false;
        }
        let value: Vec<u8> = format!("\"{exe_path}\"")
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        let ok = RegSetValueExW(hkey, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&value))
            == ERROR_SUCCESS;
        let _ = RegCloseKey(hkey);
        ok
    }
}

pub fn autostart_disable() -> bool {
    unsafe {
        let subkey = wide(SUBKEY);
        let name = wide(VALUE_NAME);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_WRITE,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            // Nothing to remove counts as success.
            return true;
        }
        let result = RegDeleteValueW(hkey, PCWSTR(name.as_ptr()));
        let _ = RegCloseKey(hkey);
        result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND
    }
}
