//! Per-window scaling. DPI changes resize existing controls and preserve drafts.
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{
    DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::*;

thread_local! { static CURRENT: Cell<i32> = const { Cell::new(0) }; }
static TEXT_PERCENT: LazyLock<AtomicU32> = LazyLock::new(|| AtomicU32::new(query_text_percent()));
pub const TEXT_CHANGED: u32 = WM_APP + 0x51;
pub fn text_percent() -> u32 {
    TEXT_PERCENT.load(Ordering::SeqCst)
}
fn query_text_percent() -> u32 {
    unsafe {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
        let initialized = RoInitialize(RO_INIT_MULTITHREADED).is_ok();
        let factor = windows::UI::ViewManagement::UISettings::new()
            .and_then(|s| s.TextScaleFactor())
            .unwrap_or(1.0);
        if initialized {
            RoUninitialize();
        }
        if factor.is_finite() && (1.0..=2.25).contains(&factor) {
            (factor * 100.0).round() as u32
        } else {
            100
        }
    }
}
pub fn refresh_text_scale() {
    let next = query_text_percent();
    let old = TEXT_PERCENT.swap(next, Ordering::SeqCst);
    if old == next {
        return;
    }
    crate::choice::close(false);
    for window in [
        crate::hwnd(&crate::PANEL),
        crate::hwnd(&crate::WARNING),
        crate::hwnd(&crate::ACTION_WARN_HWND),
        crate::settings_ui::theme_hwnd(),
    ]
    .into_iter()
    .chain(crate::automation_ui::theme_hwnds())
    {
        if !window.is_invalid() {
            unsafe {
                SendMessageW(
                    window,
                    TEXT_CHANGED,
                    Some(WPARAM(old as usize)),
                    Some(LPARAM(next as isize)),
                );
            }
        }
    }
}
pub struct Scope(i32);
impl Scope {
    pub fn window(hwnd: HWND) -> Self {
        Self::enter(unsafe { GetDpiForWindow(hwnd) }.max(96) as i32)
    }
    fn enter(dpi: i32) -> Self {
        Self(CURRENT.replace(dpi))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.set(self.0);
    }
}
pub fn scale(value: i32) -> i32 {
    let dpi = CURRENT.get();
    let dpi = if dpi > 0 {
        dpi
    } else {
        crate::DPI_SCALE.load(std::sync::atomic::Ordering::SeqCst)
    };
    (value as i64 * dpi as i64 * text_percent() as i64 / 9600) as i32
}

// Shared GDI fonts live until process exit; each distinct font/DPI is created once.
static FONTS: LazyLock<Mutex<HashMap<String, isize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
fn cached_font(font: &LOGFONTW) -> HFONT {
    let key = format!("{font:?}");
    let mut fonts = FONTS.lock().unwrap();
    HFONT(
        *fonts
            .entry(key)
            .or_insert_with(|| unsafe { CreateFontIndirectW(font).0 as isize }) as *mut _,
    )
}
pub fn font(size: i32, weight: i32, underline: bool) -> HFONT {
    static METRICS: LazyLock<LOGFONTW> = LazyLock::new(|| unsafe {
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        let _ = SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            Some((&mut metrics as *mut NONCLIENTMETRICSW).cast()),
            Default::default(),
        );
        metrics.lfMessageFont
    });
    let mut font = *METRICS;
    font.lfHeight = -scale(size);
    font.lfWeight = if weight == 600 && crate::i18n_is_chinese() {
        700
    } else {
        weight
    };
    font.lfUnderline = underline as u8;
    cached_font(&font)
}

const SUBCLASS: usize = 0x49544450;
struct State {
    dpi: Cell<u32>,
}
pub fn window_dpi(hwnd: HWND) -> Option<u32> {
    unsafe {
        let mut data = 0;
        GetWindowSubclass(hwnd, Some(window_proc), SUBCLASS, Some(&mut data))
            .as_bool()
            .then(|| (*(data as *const State)).dpi.get())
    }
}
pub fn install(hwnd: HWND) {
    unsafe {
        if GetWindowSubclass(hwnd, Some(window_proc), SUBCLASS, None).as_bool() {
            return;
        }
        let state = Box::into_raw(Box::new(State {
            dpi: Cell::new(GetDpiForWindow(hwnd).max(96)),
        }));
        if !SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS, state as usize).as_bool() {
            drop(Box::from_raw(state));
        }
    }
}

unsafe fn resize_children(parent: HWND, old: u32, new: u32) {
    unsafe {
        let mut child = GetWindow(parent, GW_CHILD).unwrap_or_default();
        while !child.is_invalid() {
            let next = GetWindow(child, GW_HWNDNEXT).unwrap_or_default();
            let mut rect = RECT::default();
            if GetWindowRect(child, &mut rect).is_ok() {
                let mut origin = POINT {
                    x: rect.left,
                    y: rect.top,
                };
                let _ = ScreenToClient(parent, &mut origin);
                let ratio = |v: i32| (v as i64 * new as i64 / old.max(1) as i64) as i32;
                let _ = SetWindowPos(
                    child,
                    None,
                    ratio(origin.x),
                    ratio(origin.y),
                    ratio(rect.right - rect.left),
                    ratio(rect.bottom - rect.top),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let old_font = SendMessageW(child, WM_GETFONT, None, None).0;
                let mut font = LOGFONTW::default();
                if old_font != 0
                    && GetObjectW(
                        HGDIOBJ(old_font as *mut _),
                        size_of::<LOGFONTW>() as i32,
                        Some((&mut font as *mut LOGFONTW).cast()),
                    ) != 0
                {
                    font.lfHeight = ratio(font.lfHeight);
                    let font = cached_font(&font);
                    SendMessageW(
                        child,
                        WM_SETFONT,
                        Some(WPARAM(font.0 as usize)),
                        Some(LPARAM(1)),
                    );
                }
            }
            child = next;
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        let state = &*(data as *const State);
        let _scope = Scope::enter(state.dpi.get() as i32);
        if msg == TEXT_CHANGED {
            let old = (wp.0 as u32).max(1);
            let new = (lp.0 as u32).max(1);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            resize_children(hwnd, old, new);
            let mut frame = RECT {
                right: (client.right as i64 * new as i64 / old as i64) as i32,
                bottom: (client.bottom as i64 * new as i64 / old as i64) as i32,
                ..Default::default()
            };
            let _ = AdjustWindowRectEx(
                &mut frame,
                WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32),
                false,
                WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE) as u32),
            );
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                frame.right - frame.left,
                frame.bottom - frame.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            crate::automation_ui::dpi_changed(hwnd);
            let _ = RedrawWindow(
                Some(hwnd),
                None,
                None,
                RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_FRAME,
            );
            return LRESULT(0);
        }
        if msg == WM_DPICHANGED && lp.0 != 0 {
            crate::choice::close(false);
            let new = (wp.0 as u32 & 0xffff).max(96);
            let old = state.dpi.replace(new);
            let _new_scope = Scope::enter(new as i32);
            resize_children(hwnd, old, new);
            let rect = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
            crate::automation_ui::dpi_changed(hwnd);
            let _ = RedrawWindow(
                Some(hwnd),
                None,
                None,
                RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_FRAME,
            );
            return LRESULT(0);
        }
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(window_proc), id);
            drop(Box::from_raw(data as *mut State));
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dpi_change_keeps_window_draft_and_modal_owner_alive() {
        let _guard = crate::CONFIG_TEST_LOCK.lock().unwrap();
        unsafe {
            let owner = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WS_POPUP,
                0,
                0,
                400,
                400,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let form = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WS_POPUP,
                0,
                0,
                300,
                200,
                Some(owner),
                None,
                None,
                None,
            )
            .unwrap();
            install(form);
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("EDIT"),
                windows::core::w!("unsaved draft"),
                WS_CHILD | WS_VISIBLE,
                10,
                10,
                100,
                20,
                Some(form),
                None,
                None,
                None,
            )
            .unwrap();
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(owner, false);
            let old = GetDpiForWindow(form).max(96);
            let next = old * 2;
            let rect = RECT {
                left: 0,
                top: 0,
                right: 600,
                bottom: 400,
            };
            SendMessageW(
                form,
                WM_DPICHANGED,
                Some(WPARAM((next | (next << 16)) as usize)),
                Some(LPARAM(&rect as *const RECT as isize)),
            );
            assert!(IsWindow(Some(edit)).as_bool());
            assert!(!windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(owner).as_bool());
            let mut text = [0u16; 32];
            let len = GetWindowTextW(edit, &mut text);
            assert_eq!(
                String::from_utf16_lossy(&text[..len as usize]),
                "unsaved draft"
            );
            let mut bounds = RECT::default();
            GetWindowRect(edit, &mut bounds).unwrap();
            assert_eq!(bounds.right - bounds.left, 200);
            SendMessageW(form, TEXT_CHANGED, Some(WPARAM(100)), Some(LPARAM(225)));
            GetWindowRect(edit, &mut bounds).unwrap();
            assert_eq!(bounds.right - bounds.left, 450);
            let len = GetWindowTextW(edit, &mut text);
            assert_eq!(
                String::from_utf16_lossy(&text[..len as usize]),
                "unsaved draft"
            );
            DestroyWindow(form).unwrap();
            DestroyWindow(owner).unwrap();
        }
    }
}
