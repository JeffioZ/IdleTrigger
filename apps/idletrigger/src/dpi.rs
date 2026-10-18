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
/// Synchronous notification after shared scaling, before the final repaint.
pub const RESCALED: u32 = WM_APP + 0x52;
pub fn text_percent() -> u32 {
    TEXT_PERCENT.load(Ordering::SeqCst)
}
fn query_text_percent() -> u32 {
    let factor = query_text_factor().unwrap_or(1.0);
    if factor.is_finite() && (1.0..=2.25).contains(&factor) {
        (factor * 100.0).round() as u32
    } else {
        100
    }
}

fn query_text_factor() -> Option<f64> {
    unsafe {
        use windows::Win32::System::WinRT::{
            RO_INIT_MULTITHREADED, RoActivateInstance, RoInitialize, RoUninitialize,
        };
        use windows::core::Interface;
        let initialized = RoInitialize(RO_INIT_MULTITHREADED).is_ok();
        // UISettings::new caches its activation factory beyond this apartment's
        // lifetime. Activate directly so every COM reference is released before
        // RoUninitialize, including after repeated WM_SETTINGCHANGE broadcasts.
        let factor = RoActivateInstance(&windows::core::HSTRING::from(
            "Windows.UI.ViewManagement.UISettings",
        ))
        .and_then(|instance| instance.cast::<windows::UI::ViewManagement::UISettings>())
        .and_then(|s| s.TextScaleFactor())
        .ok(); // Drop any COM-backed error before closing the apartment too.
        if initialized {
            RoUninitialize();
        }
        factor
    }
}

#[cfg(test)]
#[test]
fn text_scale_survives_repeated_runtime_shutdown() {
    const CHILD: &str = "IDLETRIGGER_TEST_TEXT_SCALE_CHILD";
    let mode = std::env::var(CHILD).ok();
    if mode.is_none() {
        for mode in ["uninitialized", "sta", "mta"] {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "dpi::text_scale_survives_repeated_runtime_shutdown",
                    "--nocapture",
                ])
                .env(CHILD, mode)
                .status()
                .unwrap();
            assert!(
                status.success(),
                "text-scale child ({mode}) failed: {status}"
            );
        }
        return;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetErrorMode(mode: u32) -> u32;
    }
    unsafe { SetErrorMode(0x0002) };
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
    };
    let apartment = match mode.as_deref() {
        Some("sta") => Some(COINIT_APARTMENTTHREADED),
        Some("mta") => Some(COINIT_MULTITHREADED),
        _ => None,
    };
    if let Some(apartment) = apartment {
        unsafe { CoInitializeEx(None, apartment).ok().unwrap() };
    }
    for _ in 0..32 {
        let factor = query_text_factor().expect("native text-scale lookup must succeed");
        assert!(factor.is_finite() && (1.0..=2.25).contains(&factor));
        assert!((100..=225).contains(&query_text_percent()));
        unsafe { windows::Win32::System::Com::CoFreeUnusedLibrariesEx(0, None) };
    }
    if apartment.is_some() {
        unsafe { CoUninitialize() };
    }
}
pub fn refresh_text_scale() {
    // LOCK_NOTIFY_HWND is deliberately absent: the layered card re-renders
    // its bitmap on every show, so the next appearance picks up the new
    // scale; broadcasting at it mid-animation would compose a stale frame.
    let next = query_text_percent();
    let old = TEXT_PERCENT.swap(next, Ordering::SeqCst);
    if old == next {
        return;
    }
    crate::choice::close(false);
    // Same audience as the theme refresh: main-owned surfaces, then the
    // form windows via the kit registry. Embedded panes scale with their
    // parent, so their controls receive the change exactly once.
    let mut windows = vec![
        crate::hwnd(&crate::PANEL),
        crate::warning::hwnd_of_warning(),
        crate::hwnd(&crate::ACTION_WARN_HWND),
    ];
    {
        use crate::kit::registry::Kind;
        windows.extend(
            [Kind::Settings, Kind::ContextRules, Kind::Automation]
                .map(crate::kit::registry::hwnd_of)
                .into_iter()
                .flatten(),
        );
    }
    windows.extend(crate::kit::registry::hwnd_of(
        crate::kit::registry::Kind::ProcessPicker,
    ));
    for window in windows {
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
    (value as i64 * factor() as i64 / 9600) as i32
}

/// DPI and text scaling combined, without rounding to a physical pixel.
pub fn factor() -> u32 {
    let dpi = CURRENT.get();
    let dpi = if dpi > 0 {
        dpi
    } else {
        crate::DPI_SCALE.load(std::sync::atomic::Ordering::SeqCst)
    };
    dpi as u32 * text_percent()
}

/// Retain the source measurement across scale changes. Only a deliberate
/// layout change replaces it; repeated integer rounding must not accumulate.
#[derive(Clone, Copy)]
pub struct ScaledValue {
    value: i32,
    scale: u32,
}
impl ScaledValue {
    pub fn new(value: i32, scale: u32) -> Self {
        Self {
            value,
            scale: scale.max(1),
        }
    }
    fn at(self, scale: u32) -> i32 {
        (self.value as i64 * scale as i64 / self.scale as i64) as i32
    }
    pub fn rescale(&mut self, actual: i32, old: u32, new: u32) -> i32 {
        if self.at(old) != actual {
            *self = Self::new(actual, old);
        }
        self.at(new)
    }
}

// Shared GDI fonts live until process exit; each distinct font/DPI is created once.
static FONTS: LazyLock<Mutex<HashMap<String, isize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
// Keep the requested logical size: a 14 px font created at 125% is 17 px,
// but must return to 14 px (not 13 px) on a 100% display.
static FONT_SIZES: LazyLock<Mutex<HashMap<(isize, u32), i32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
fn cached_font(font: &LOGFONTW) -> HFONT {
    let key = format!("{font:?}");
    let mut fonts = crate::runtime::lock(&FONTS);
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
    let handle = cached_font(&font);
    crate::runtime::lock(&FONT_SIZES).insert((handle.0 as isize, factor()), -size);
    handle
}

const CONTROL_SUBCLASS: usize = 0x49544443;
#[derive(Clone, Copy)]
struct FontState {
    handle: isize,
    metrics: LOGFONTW,
    height: ScaledValue,
    width: ScaledValue,
}
impl FontState {
    unsafe fn read(hwnd: HWND, scale: u32) -> Option<Self> {
        unsafe {
            let handle = SendMessageW(hwnd, WM_GETFONT, None, None).0;
            let mut metrics = LOGFONTW::default();
            if handle == 0
                || GetObjectW(
                    HGDIOBJ(handle as *mut _),
                    size_of::<LOGFONTW>() as i32,
                    Some((&mut metrics as *mut LOGFONTW).cast()),
                ) == 0
            {
                return None;
            }
            let height = crate::runtime::lock(&FONT_SIZES)
                .get(&(handle, scale))
                .map(|size| ScaledValue::new(*size, 9600))
                .unwrap_or_else(|| ScaledValue::new(metrics.lfHeight, scale));
            Some(Self {
                handle,
                metrics,
                height,
                width: ScaledValue::new(metrics.lfWidth, scale),
            })
        }
    }
}

struct ControlState {
    rect: Cell<[ScaledValue; 4]>,
    font: Cell<Option<FontState>>,
    container: Cell<bool>,
    edit_margin: Cell<Option<i32>>,
}

unsafe fn control_rect(hwnd: HWND) -> Option<[i32; 4]> {
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).ok()?;
        let mut origin = POINT {
            x: rect.left,
            y: rect.top,
        };
        let _ = ScreenToClient(GetParent(hwnd).ok()?, &mut origin);
        Some([
            origin.x,
            origin.y,
            rect.right - rect.left,
            rect.bottom - rect.top,
        ])
    }
}

unsafe fn control_state(hwnd: HWND, scale: u32) -> Option<&'static ControlState> {
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(control_proc), CONTROL_SUBCLASS, Some(&mut data)).as_bool()
        {
            return Some(&*(data as *const ControlState));
        }
        let rect = control_rect(hwnd)?;
        let state = Box::into_raw(Box::new(ControlState {
            rect: Cell::new(rect.map(|v| ScaledValue::new(v, scale))),
            font: Cell::new(FontState::read(hwnd, scale)),
            container: Cell::new(false),
            edit_margin: Cell::new(None),
        }));
        if !SetWindowSubclass(hwnd, Some(control_proc), CONTROL_SUBCLASS, state as usize).as_bool()
        {
            drop(Box::from_raw(state));
            return None;
        }
        Some(&*state)
    }
}

/// Logical creation/flow/spec coordinates supersede a physical fallback.
pub fn remember_rect(hwnd: HWND, (x, y, w, h): (i32, i32, i32, i32)) {
    unsafe {
        if let Some(state) = control_state(hwnd, factor()) {
            state
                .rect
                .set([x, y, w, h].map(|v| ScaledValue::new(v, 9600)));
        }
    }
}

/// Explicitly opt an embedded form into descendant scaling. Native
/// controls' private children (list headers, scrollbars, etc.) stay native.
pub fn track_container(hwnd: HWND) {
    unsafe {
        if let Some(state) = control_state(hwnd, factor()) {
            state.container.set(true);
        }
    }
}

/// EDIT resets its margins on WM_SETFONT. Retain the logical padding so
/// native font changes and DPI changes restore the same field appearance.
pub fn set_edit_margins(hwnd: HWND, logical: i32) {
    unsafe {
        if let Some(state) = control_state(hwnd, factor()) {
            state.edit_margin.set(Some(logical));
            apply_edit_margins(hwnd, logical);
        }
    }
}

unsafe fn apply_edit_margins(hwnd: HWND, logical: i32) {
    let _dpi = Scope::window(hwnd);
    let margin = scale(logical) as usize;
    unsafe {
        SendMessageW(
            hwnd,
            0x00D3,
            Some(WPARAM(3)),
            Some(LPARAM((margin | (margin << 16)) as isize)),
        ); // EM_SETMARGINS, EC_LEFTMARGIN | EC_RIGHTMARGIN
    }
}

unsafe extern "system" fn control_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_SETFONT {
            let margin = (*(data as *const ControlState)).edit_margin.get();
            let result = DefSubclassProc(hwnd, msg, wp, lp);
            if let Some(margin) = margin {
                apply_edit_margins(hwnd, margin);
            }
            return result;
        }
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(control_proc), id);
            drop(Box::from_raw(data as *mut ControlState));
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

const SUBCLASS: usize = 0x49544450;
struct State {
    dpi: Cell<u32>,
    client: Cell<[ScaledValue; 2]>,
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
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let state = Box::into_raw(Box::new(State {
            dpi: Cell::new(GetDpiForWindow(hwnd).max(96)),
            client: Cell::new([client.right, client.bottom].map(|v| ScaledValue::new(v, factor()))),
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
            if let Some((state, actual)) = control_state(child, old).zip(control_rect(child)) {
                let mut rect = state.rect.get();
                let scaled: [i32; 4] =
                    std::array::from_fn(|i| rect[i].rescale(actual[i], old, new));
                state.rect.set(rect);
                let _ = SetWindowPos(
                    child,
                    None,
                    scaled[0],
                    scaled[1],
                    scaled[2],
                    scaled[3],
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
                );
                let handle = SendMessageW(child, WM_GETFONT, None, None).0;
                let font = state
                    .font
                    .get()
                    .filter(|f| f.handle == handle)
                    .or_else(|| FontState::read(child, old));
                if let Some(mut font) = font {
                    font.metrics.lfHeight = font.height.at(new);
                    font.metrics.lfWidth = font.width.at(new);
                    let handle = cached_font(&font.metrics);
                    font.handle = handle.0 as isize;
                    state.font.set(Some(font));
                    SendMessageW(
                        child,
                        WM_SETFONT,
                        Some(WPARAM(handle.0 as usize)),
                        Some(LPARAM(0)),
                    );
                }
                if state.container.get() {
                    crate::viewport::rescale(child, old, new, || resize_children(child, old, new));
                }
            }
            child = next;
        }
        crate::nativeform::refresh_tooltip_geometry(parent);
    }
}

/// Shared relayout tail after a scale change: re-derive runtime-placed
/// children, re-read the title-bar icons at the new scale, and force one
/// full repaint (both the text-scale and the DPI handlers end here).
unsafe fn finish_rescale(hwnd: HWND) {
    unsafe {
        SendMessageW(hwnd, RESCALED, None, None);
        crate::set_window_icons_pub(hwnd);
        let _ = RedrawWindow(
            Some(hwnd),
            None,
            None,
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_FRAME,
        );
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
            let _frame = crate::FrameTransition::begin(hwnd);
            crate::choice::close(false);
            let old = (wp.0 as u32).max(1);
            let new = (lp.0 as u32).max(1);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let old = old * state.dpi.get();
            let new = new * state.dpi.get();
            resize_children(hwnd, old, new);
            let mut size = state.client.get();
            let mut frame = RECT {
                right: size[0].rescale(client.right, old, new),
                bottom: size[1].rescale(client.bottom, old, new),
                ..Default::default()
            };
            state.client.set(size);
            // The viewport already scaled its retained content before
            // forwarding this message. Shrinking text should restore that
            // size, not shrink an already work-area-clipped client again.
            if let Some((width, height)) = crate::viewport::content_size(hwnd) {
                frame.right = frame.right.max(width);
                frame.bottom = frame.bottom.max(height);
            }
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
            finish_rescale(hwnd);
            return LRESULT(0);
        }
        if msg == WM_DPICHANGED && lp.0 != 0 {
            let _frame = crate::FrameTransition::begin(hwnd);
            crate::choice::close(false);
            let new = (wp.0 as u32 & 0xffff).max(96);
            let old = state.dpi.replace(new);
            let _new_scope = Scope::enter(new as i32);
            resize_children(hwnd, old * text_percent(), new * text_percent());
            // resize_children rescales children from their creation-time
            // geometry; the panel's update links are placed at runtime from
            // phase state instead, so re-derive them at the new scale.
            if hwnd == crate::hwnd(&crate::PANEL) {
                // Keep scale()'s no-scope fallback (UI-thread paths outside
                // window procs) tracking the panel's monitor instead of the
                // DPI captured when the panel was created, so a monitor
                // hot-plug cannot leave it pointing at a departed monitor's
                // DPI.
                crate::DPI_SCALE.store(new as i32, Ordering::SeqCst);
                crate::request_update_refresh();
            }
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
            finish_rescale(hwnd);
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
        let _guard = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
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
            let _ = ShowWindow(form, SW_SHOWNOACTIVATE);
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
            assert!(IsWindowVisible(form).as_bool());
            let mut cloaked = 1u32;
            windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                form,
                windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
                (&mut cloaked as *mut u32).cast(),
                size_of::<u32>() as u32,
            )
            .unwrap();
            assert_eq!(
                cloaked, 0,
                "DPI/text-scale transition must reveal the final frame"
            );
            DestroyWindow(form).unwrap();
            DestroyWindow(owner).unwrap();
        }
    }
}
