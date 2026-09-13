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

/// Repair only an existing registration; never opt the user into autostart.
pub fn autostart_ensure_current() -> Result<(), String> {
    unsafe {
        let mut key = HKEY::default();
        let opened = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(SUBKEY).as_ptr()),
            None,
            KEY_READ,
            &mut key,
        );
        if opened == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        if opened != ERROR_SUCCESS {
            return Err(format!("open autostart: {}", opened.0));
        }
        let result = (|| {
            let mut size = 0;
            let name = wide(VALUE_NAME);
            let mut kind = windows::Win32::System::Registry::REG_VALUE_TYPE::default();
            let status = RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                None,
                Some(&mut size),
            );
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            if status != ERROR_SUCCESS || kind != REG_SZ || size > 128 * 1024 {
                return Err("invalid autostart registration".into());
            }
            let mut data = vec![0u8; size as usize];
            let status = RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                None,
                Some(data.as_mut_ptr()),
                Some(&mut size),
            );
            if status != ERROR_SUCCESS || !size.is_multiple_of(2) {
                return Err("could not read autostart registration".into());
            }
            data.truncate(size as usize);
            let current = String::from_utf16(
                &data
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .take_while(|c| *c != 0)
                    .collect::<Vec<_>>(),
            )
            .map_err(|e| e.to_string())?;
            let path = std::env::current_exe()
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .into_owned();
            if current != format!("\"{path}\" --minimized") && !autostart_enable(&path) {
                return Err("could not update autostart registration".into());
            }
            Ok(())
        })();
        let _ = RegCloseKey(key);
        result
    }
}

pub fn autostart_enable(exe_path: &str) -> bool {
    if exe_path.is_empty() || exe_path.contains(['\0', '"']) {
        return false;
    }
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
        let value: Vec<u8> = format!("\"{exe_path}\" --minimized")
            .encode_utf16()
            .chain([0])
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
        let opened = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_WRITE,
            &mut hkey,
        );
        if opened != ERROR_SUCCESS {
            // Nothing to remove counts as success.
            return opened == ERROR_FILE_NOT_FOUND;
        }
        let result = RegDeleteValueW(hkey, PCWSTR(name.as_ptr()));
        let _ = RegCloseKey(hkey);
        result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND
    }
}
/// OS build from the machine registry; HKCU does not contain this version key.
pub fn windows_build() -> u32 {
    static BUILD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *BUILD.get_or_init(|| unsafe {
        use windows::Win32::System::Registry::*;
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            windows::core::w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            None,
            KEY_READ | KEY_WOW64_64KEY,
            &mut key,
        ) != windows::Win32::Foundation::ERROR_SUCCESS
        {
            return 0;
        }
        let mut bytes = [0u8; 64];
        let mut size = bytes.len() as u32;
        let result = RegQueryValueExW(
            key,
            windows::core::w!("CurrentBuildNumber"),
            None,
            None,
            Some(bytes.as_mut_ptr()),
            Some(&mut size),
        );
        let _ = RegCloseKey(key);
        if result != windows::Win32::Foundation::ERROR_SUCCESS {
            return 0;
        }
        let text: Vec<u16> = bytes[..size as usize]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .take_while(|c| *c != 0)
            .collect();
        String::from_utf16_lossy(&text).parse().unwrap_or(0)
    })
}
