//! Shared registry value readers (HKCU/HKLM, optional 64-bit view).
//!
//! One probe-based implementation per value type; callers keep their own
//! root/view choice via `wow64`, matching each site's previous behavior.

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, KEY_READ, KEY_WOW64_64KEY, REG_DWORD, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE, RegCloseKey,
    RegOpenKeyExW, RegQueryValueExW,
};
use windows::core::PCWSTR;

/// String values larger than this are treated as malformed.
const MAX_VALUE_BYTES: u32 = 1 << 20;

/// Reads one REG_SZ/REG_EXPAND_SZ value through an already-open key;
/// `name` of None reads the key's default value. NUL-terminated UTF-16.
pub(crate) fn read_sz_open(hkey: HKEY, name: Option<&str>) -> Option<String> {
    let value_name = crate::wide(name.unwrap_or(""));
    let name_ptr = if name.is_some() {
        PCWSTR(value_name.as_ptr())
    } else {
        PCWSTR::null()
    };
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    let status =
        unsafe { RegQueryValueExW(hkey, name_ptr, None, Some(&mut kind), None, Some(&mut size)) };
    if status != ERROR_SUCCESS || size > MAX_VALUE_BYTES || !size.is_multiple_of(2) {
        return None;
    }
    if kind != REG_SZ && kind != REG_EXPAND_SZ {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    let status = unsafe {
        RegQueryValueExW(
            hkey,
            name_ptr,
            None,
            None,
            Some(data.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let text: Vec<u16> = data[..size as usize]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|ch| *ch != 0)
        .collect();
    Some(String::from_utf16_lossy(&text))
}

/// Opens `subkey` under `root` for reading (64-bit view when `wow64`) and
/// runs `body`; the key is always closed again.
fn with_key<T>(
    root: HKEY,
    subkey: &str,
    wow64: bool,
    body: impl FnOnce(HKEY) -> Option<T>,
) -> Option<T> {
    let subkey = crate::wide(subkey);
    let mut hkey = HKEY::default();
    let access = if wow64 {
        KEY_READ | KEY_WOW64_64KEY
    } else {
        KEY_READ
    };
    let opened = unsafe { RegOpenKeyExW(root, PCWSTR(subkey.as_ptr()), None, access, &mut hkey) };
    if opened != ERROR_SUCCESS {
        return None;
    }
    let result = body(hkey);
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    result
}

/// Reads one string value under `subkey`; `name` of None reads the default.
pub(crate) fn read_sz(root: HKEY, subkey: &str, name: Option<&str>, wow64: bool) -> Option<String> {
    with_key(root, subkey, wow64, |hkey| read_sz_open(hkey, name))
}

/// Reads one REG_DWORD under `subkey`; only a well-formed 4-byte DWORD counts.
pub(crate) fn read_dword(root: HKEY, subkey: &str, name: &str, wow64: bool) -> Option<u32> {
    let name = crate::wide(name);
    with_key(root, subkey, wow64, |hkey| {
        let mut data = [0u8; 4];
        let mut size = 4u32;
        let mut kind = REG_VALUE_TYPE::default();
        let ok = unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                Some(data.as_mut_ptr()),
                Some(&mut size),
            )
        } == ERROR_SUCCESS;
        (ok && size == 4 && kind == REG_DWORD).then(|| u32::from_le_bytes(data))
    })
}

/// Reads one raw binary value under `subkey`, any value type.
pub(crate) fn read_binary(root: HKEY, subkey: &str, name: &str, wow64: bool) -> Option<Vec<u8>> {
    let name = crate::wide(name);
    with_key(root, subkey, wow64, |hkey| {
        let mut kind = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        let status = unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                None,
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS || size > MAX_VALUE_BYTES {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        let status = unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR(name.as_ptr()),
                None,
                None,
                Some(data.as_mut_ptr()),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        data.truncate(size as usize);
        Some(data)
    })
}
