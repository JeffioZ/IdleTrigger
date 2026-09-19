//! Scroll tall native forms without recreating their controls or drafts.
use std::cell::Cell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{InvalidateRect, ScreenToClient};
use windows::Win32::UI::Shell::{
    DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const SUBCLASS: usize = 0x49545650;
struct State {
    width: Cell<i32>,
    x: Cell<i32>,
    restore_x: Cell<i32>,
    bar: Cell<HWND>,
    drag: Cell<i32>,
    hover: Cell<bool>,
    wheel: Cell<i32>,
    height: Cell<i32>,
    position: Cell<i32>,
    restore: Cell<i32>,
}
unsafe fn state(hwnd: HWND) -> Option<&'static State> {
    let mut data = 0;
    unsafe {
        GetWindowSubclass(hwnd, Some(window_proc), SUBCLASS, Some(&mut data))
            .as_bool()
            .then(|| &*(data as *const State))
    }
}

pub fn metrics(hwnd: HWND) -> Option<(i32, i32, i32)> {
    let _dpi = crate::dpi::Scope::window(hwnd);
    unsafe {
        let state = state(hwnd)?;
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client).ok()?;
        Some((
            state.height.get(),
            page_size(state, &client).1,
            state.position.get(),
        ))
    }
}

fn page_size(state: &State, client: &RECT) -> (i32, i32) {
    let bar = crate::scale_pub(14);
    let mut horizontal = state.width.get() > client.right;
    let mut vertical = state.height.get() > client.bottom;
    for _ in 0..2 {
        horizontal = state.width.get() > client.right - if vertical { bar } else { 0 };
        vertical = state.height.get() > client.bottom - if horizontal { bar } else { 0 };
    }
    (
        (client.right - if vertical { bar } else { 0 }).max(1),
        (client.bottom - if horizontal { bar } else { 0 }).max(1),
    )
}

fn scroll_x(hwnd: HWND, position: i32) {
    let _dpi = crate::dpi::Scope::window(hwnd);
    unsafe {
        let Some(state) = state(hwnd) else {
            return;
        };
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let page = page_size(state, &client).0;
        let next = position.clamp(0, (state.width.get() - page).max(0));
        let delta = state.x.replace(next) - next;
        move_children(hwnd, state.bar.get(), delta, 0);
        sync_horizontal(hwnd);
    }
}

unsafe fn move_children(hwnd: HWND, horizontal: HWND, dx: i32, dy: i32) {
    if dx == 0 && dy == 0 {
        return;
    }
    unsafe {
        let mut child = GetWindow(hwnd, GW_CHILD).unwrap_or_default();
        while !child.is_invalid() {
            let next = GetWindow(child, GW_HWNDNEXT).unwrap_or_default();
            if child != horizontal && !crate::list_style::is_scrollbar(child) {
                let mut rect = RECT::default();
                if GetWindowRect(child, &mut rect).is_ok() {
                    let mut origin = POINT {
                        x: rect.left,
                        y: rect.top,
                    };
                    let _ = ScreenToClient(hwnd, &mut origin);
                    let _ = SetWindowPos(
                        child,
                        None,
                        origin.x + dx,
                        origin.y + dy,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                }
            }
            child = next;
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

pub fn scroll_to(hwnd: HWND, position: i32) -> bool {
    unsafe {
        let Some(state) = state(hwnd) else {
            return false;
        };
        let (_, page, _) = metrics(hwnd).unwrap();
        let next = position.clamp(0, (state.height.get() - page).max(0));
        let delta = state.position.replace(next) - next;
        if delta == 0 {
            return true;
        }
        let mut child = GetWindow(hwnd, GW_CHILD).unwrap_or_default();
        while !child.is_invalid() {
            let next = GetWindow(child, GW_HWNDNEXT).unwrap_or_default();
            if child != state.bar.get() && !crate::list_style::is_scrollbar(child) {
                let mut rect = RECT::default();
                if GetWindowRect(child, &mut rect).is_ok() {
                    let mut origin = POINT {
                        x: rect.left,
                        y: rect.top,
                    };
                    let _ = ScreenToClient(hwnd, &mut origin);
                    let _ = SetWindowPos(
                        child,
                        None,
                        origin.x,
                        origin.y + delta,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                }
            }
            child = next;
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
        crate::list_style::refresh(hwnd);
        sync_horizontal(hwnd);
        true
    }
}

/// Restore content coordinates before a form recalculates its layout.
pub fn begin_layout(hwnd: HWND) {
    unsafe {
        if let Some(state) = state(hwnd) {
            state.restore.set(state.position.get());
            state.restore_x.set(state.x.get());
            scroll_x(hwnd, 0);
            scroll_to(hwnd, 0);
        }
    }
}

/// Called with the form's complete desired client size after layout.
pub fn fit(hwnd: HWND) {
    unsafe {
        let mut client = RECT::default();
        if GetClientRect(hwnd, &mut client).is_err() {
            return;
        }
        if state(hwnd).is_none() {
            let data = Box::into_raw(Box::new(State {
                width: Cell::new(client.right),
                x: Cell::new(0),
                restore_x: Cell::new(0),
                bar: Cell::new(HWND::default()),
                drag: Cell::new(-1),
                hover: Cell::new(false),
                wheel: Cell::new(0),
                height: Cell::new(client.bottom),
                position: Cell::new(0),
                restore: Cell::new(0),
            }));
            if !SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS, data as usize).as_bool() {
                drop(Box::from_raw(data));
                return;
            }
            crate::list_style::install(hwnd);
            if let Ok(bar) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WS_CHILD | WINDOW_STYLE(0x100),
                0,
                0,
                1,
                1,
                Some(hwnd),
                None,
                None,
                None,
            ) {
                if SetWindowSubclass(bar, Some(horizontal_proc), SUBCLASS, hwnd.0 as usize)
                    .as_bool()
                {
                    (*data).bar.set(bar);
                } else {
                    let _ = DestroyWindow(bar);
                }
            }
        }
        let state = state(hwnd).unwrap();
        state.height.set(client.bottom);
        state.width.set(client.right);
        constrain(hwnd);
        scroll_x(hwnd, state.restore_x.get());
        scroll_to(hwnd, state.restore.get());
        crate::list_style::refresh(hwnd);
        sync_horizontal(hwnd);
    }
}

/// Refit retained content after a monitor/work-area change, even at the same DPI.
/// Unlike `fit`, this keeps the full content size instead of recording the
/// already-constrained client size as the new layout size.
pub fn fit_work_area(hwnd: HWND, work: RECT) {
    let _dpi = crate::dpi::Scope::window(hwnd);
    unsafe {
        let Some(state) = state(hwnd) else {
            return;
        };
        let mut rect = RECT::default();
        let mut client = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() || GetClientRect(hwnd, &mut client).is_err() {
            return;
        }
        let width = (state.width.get() + rect.right - rect.left - client.right)
            .min(work.right - work.left)
            .max(1);
        let height = (state.height.get() + rect.bottom - rect.top - client.bottom)
            .min(work.bottom - work.top)
            .max(1);
        let x = state.x.get();
        let y = state.position.get();
        if SetWindowPos(
            hwnd,
            None,
            rect.left
                .clamp(work.left, (work.right - width).max(work.left)),
            rect.top
                .clamp(work.top, (work.bottom - height).max(work.top)),
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .is_ok()
        {
            scroll_x(hwnd, x);
            scroll_to(hwnd, y);
            crate::list_style::refresh(hwnd);
            sync_horizontal(hwnd);
        }
    }
}

unsafe fn constrain(hwnd: HWND) {
    unsafe {
        let work = crate::display::work_area_for(hwnd);
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return;
        }
        let width = (rect.right - rect.left).min(work.right - work.left).max(1);
        let height = (rect.bottom - rect.top).min(work.bottom - work.top).max(1);
        let x = rect
            .left
            .clamp(work.left, (work.right - width).max(work.left));
        let y = rect
            .top
            .clamp(work.top, (work.bottom - height).max(work.top));
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

pub fn reveal_control(hwnd: HWND, control: HWND) {
    unsafe {
        let Some((_, page, position)) = metrics(hwnd) else {
            return;
        };
        if !IsChild(hwnd, control).as_bool() {
            return;
        }
        let mut rect = RECT::default();
        if GetWindowRect(control, &mut rect).is_err() {
            return;
        }
        let mut origin = POINT {
            x: rect.left,
            y: rect.top,
        };
        let _ = ScreenToClient(hwnd, &mut origin);
        let bottom = origin.y + rect.bottom - rect.top;
        if let Some(state) = state(hwnd) {
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let width = page_size(state, &client).0;
            if origin.x < 0 {
                scroll_x(hwnd, state.x.get() + origin.x);
            } else if origin.x + rect.right - rect.left > width {
                scroll_x(
                    hwnd,
                    state.x.get() + origin.x + rect.right - rect.left - width,
                );
            }
        }
        if origin.y < 0 {
            scroll_to(hwnd, position + origin.y - crate::scale_pub(8));
        } else if bottom > page {
            scroll_to(hwnd, position + bottom - page + crate::scale_pub(8));
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
        let _dpi = crate::dpi::Scope::window(hwnd);
        if msg == WM_DPICHANGED || msg == crate::dpi::TEXT_CHANGED {
            let _frame = crate::FrameTransition::begin(hwnd);
            let old = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96);
            // The dpi subclass retains the previous DPI until it handles this message.
            let old = if msg == crate::dpi::TEXT_CHANGED {
                (wp.0 as u32).max(1)
            } else {
                crate::dpi::window_dpi(hwnd).unwrap_or(old)
            };
            let new = if msg == crate::dpi::TEXT_CHANGED {
                lp.0 as u32
            } else {
                (wp.0 as u32 & 0xffff).max(96)
            };
            let position = state.position.get();
            let x = state.x.get();
            scroll_x(hwnd, 0);
            scroll_to(hwnd, 0);
            state
                .height
                .set((state.height.get() as i64 * new as i64 / old as i64) as i32);
            state
                .width
                .set((state.width.get() as i64 * new as i64 / old as i64) as i32);
            let result = DefSubclassProc(hwnd, msg, wp, lp);
            constrain(hwnd);
            scroll_x(hwnd, (x as i64 * new as i64 / old as i64) as i32);
            scroll_to(hwnd, (position as i64 * new as i64 / old as i64) as i32);
            crate::list_style::refresh(hwnd);
            return result;
        }
        if msg == WM_COMMAND && (wp.0 >> 16) & 0xffff == 0x100 {
            reveal_control(hwnd, HWND(lp.0 as *mut _)); // EN_SETFOCUS
        }
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(window_proc), id);
            drop(Box::from_raw(data as *mut State));
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

fn sync_horizontal(hwnd: HWND) {
    let _dpi = crate::dpi::Scope::window(hwnd);
    unsafe {
        let Some(state) = state(hwnd) else {
            return;
        };
        let bar = state.bar.get();
        if bar.is_invalid() {
            return;
        }
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let (width, height) = page_size(state, &client);
        let _ = SetWindowPos(
            bar,
            Some(HWND::default()),
            0,
            height,
            width,
            crate::scale_pub(14),
            SWP_NOACTIVATE,
        );
        let _ = ShowWindow(
            bar,
            if state.width.get() > width {
                SW_SHOWNA
            } else {
                SW_HIDE
            },
        );
        let _ = InvalidateRect(Some(bar), None, false);
    }
}

pub fn horizontal_wheel(hwnd: HWND, msg: u32, wp: WPARAM) -> bool {
    let _dpi = crate::dpi::Scope::window(hwnd);
    unsafe {
        let Some(state) = state(hwnd) else {
            return false;
        };
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let (width, height) = page_size(state, &client);
        if state.width.get() <= width
            || (msg != WM_MOUSEHWHEEL && wp.0 & 4 == 0 && state.height.get() > height)
        {
            return false;
        }
        let delta = state.wheel.get() + (wp.0 >> 16) as i16 as i32;
        state.wheel.set(delta % 120);
        let sign = if msg == WM_MOUSEHWHEEL { 1 } else { -1 };
        scroll_x(
            hwnd,
            state.x.get() + sign * (delta / 120) * crate::scale_pub(36),
        );
        true
    }
}

unsafe extern "system" fn horizontal_proc(
    bar: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    unsafe {
        let parent = HWND(data as *mut _);
        let _dpi = crate::dpi::Scope::window(parent);
        let Some(state) = state(parent) else {
            return DefSubclassProc(bar, msg, wp, lp);
        };
        let mut bounds = RECT::default();
        let _ = GetClientRect(bar, &mut bounds);
        let mut client = RECT::default();
        let _ = GetClientRect(parent, &mut client);
        let page = page_size(state, &client).0;
        let max = (state.width.get() - page).max(0);
        let inset = crate::scale_pub(2);
        let track = RECT {
            left: inset,
            top: inset,
            right: bounds.right - inset,
            bottom: bounds.bottom - inset,
        };
        let (left, right) = crate::list_style::thumb(
            state.width.get(),
            page,
            state.x.get(),
            track.right - track.left,
            crate::scale_pub(24),
        );
        let thumb = RECT {
            left: track.left + left,
            right: track.left + right,
            ..track
        };
        let x = lp.0 as i16 as i32;
        match msg {
            WM_PAINT | WM_PRINTCLIENT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = if msg == WM_PAINT {
                    BeginPaint(bar, &mut ps)
                } else {
                    HDC(wp.0 as *mut _)
                };
                crate::paint::fill_rect(dc, &bounds, crate::theme::palette().window_bg);
                crate::list_style::draw_scrollbar(
                    dc,
                    &track,
                    &thumb,
                    crate::scale_pub(4),
                    state.hover.get(),
                    state.drag.get() >= 0,
                );
                if msg == WM_PAINT {
                    let _ = EndPaint(bar, &ps);
                }
                return LRESULT(0);
            }
            WM_LBUTTONDOWN => {
                if x >= thumb.left && x < thumb.right {
                    state.drag.set(x - thumb.left);
                    SetCapture(bar);
                } else {
                    scroll_x(
                        parent,
                        state.x.get() + if x < thumb.left { -page } else { page },
                    );
                }
            }
            WM_MOUSEMOVE => {
                state.hover.set(true);
                let mut event = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: bar,
                    ..Default::default()
                };
                let _ = TrackMouseEvent(&mut event);
                let travel = track.right - track.left - (thumb.right - thumb.left);
                if state.drag.get() >= 0 && travel > 0 {
                    scroll_x(
                        parent,
                        ((x - state.drag.get() - track.left).clamp(0, travel) as i64 * max as i64
                            / travel as i64) as i32,
                    );
                }
            }
            windows::Win32::UI::Controls::WM_MOUSELEAVE => state.hover.set(false),
            WM_LBUTTONUP | WM_CANCELMODE => {
                state.drag.set(-1);
                let _ = ReleaseCapture();
            }
            WM_CAPTURECHANGED => state.drag.set(-1),
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                horizontal_wheel(parent, msg, WPARAM(wp.0 | 4));
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(bar, Some(horizontal_proc), id);
                return DefSubclassProc(bar, msg, wp, lp);
            }
            _ => return DefSubclassProc(bar, msg, wp, lp),
        }
        let _ = InvalidateRect(Some(bar), None, false);
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn work_area_change_at_same_dpi_preserves_content_and_restores_size() {
        let _guard = crate::CONFIG_TEST_LOCK.lock().unwrap();
        unsafe {
            let work = crate::display::work_area_for(HWND::default());
            let width = (work.right - work.left).min(600);
            let height = (work.bottom - work.top).min(400);
            let form = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WS_POPUP | WS_CAPTION,
                work.left,
                work.top,
                width,
                height,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            crate::dpi::install(form);
            let mut client = RECT::default();
            GetClientRect(form, &mut client).unwrap();
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("EDIT"),
                windows::core::w!("keep this draft"),
                WS_CHILD | WS_VISIBLE,
                client.right - 160,
                client.bottom - 36,
                150,
                24,
                Some(form),
                None,
                None,
                None,
            )
            .unwrap();
            fit(form);
            let dpi = crate::dpi::window_dpi(form);
            let small = RECT {
                right: work.left + width / 2,
                bottom: work.top + height / 2,
                ..work
            };
            fit_work_area(form, small);
            let mut bounds = RECT::default();
            GetWindowRect(form, &mut bounds).unwrap();
            assert_eq!(bounds, small);
            assert_eq!(state(form).unwrap().width.get(), client.right);
            assert_eq!(metrics(form).unwrap().0, client.bottom);
            assert_ne!(
                GetWindowLongW(state(form).unwrap().bar.get(), GWL_STYLE) as u32 & WS_VISIBLE.0,
                0
            );
            reveal_control(form, edit);
            let position = metrics(form).unwrap().2;
            let x = state(form).unwrap().x.get();
            assert!(position > 0 && x > 0);
            GetWindowRect(edit, &mut bounds).unwrap();
            assert!(bounds.left >= small.left && bounds.right <= small.right);
            assert!(bounds.top >= small.top && bounds.bottom <= small.bottom);
            fit_work_area(form, small);
            assert_eq!(metrics(form).unwrap().2, position);
            assert_eq!(state(form).unwrap().x.get(), x);

            fit_work_area(form, work);
            GetWindowRect(form, &mut bounds).unwrap();
            assert_eq!(bounds.right - bounds.left, width);
            assert_eq!(bounds.bottom - bounds.top, height);
            assert_eq!(metrics(form).unwrap().2, 0);
            assert_eq!(state(form).unwrap().x.get(), 0);
            assert_eq!(
                GetWindowLongW(state(form).unwrap().bar.get(), GWL_STYLE) as u32 & WS_VISIBLE.0,
                0
            );
            assert_eq!(crate::dpi::window_dpi(form), dpi);
            let mut text = [0u16; 32];
            GetWindowTextW(edit, &mut text);
            assert!(String::from_utf16_lossy(&text).starts_with("keep this draft"));
            DestroyWindow(form).unwrap();
        }
    }

    #[test]
    fn oversized_form_scrolls_both_axes_and_preserves_draft_through_dpi_change() {
        let _guard = crate::CONFIG_TEST_LOCK.lock().unwrap();
        unsafe {
            let work = crate::display::work_area_for(HWND::default());
            let height = (work.bottom - work.top) * 2;
            let width = (work.right - work.left) * 2;
            let form = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WS_POPUP,
                work.left,
                work.top,
                width,
                height,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            crate::dpi::install(form);
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("EDIT"),
                windows::core::w!("keep this draft"),
                WS_CHILD | WS_VISIBLE,
                width - 180,
                height - 40,
                150,
                24,
                Some(form),
                None,
                None,
                None,
            )
            .unwrap();
            fit(form);
            let (_, page, position) = metrics(form).unwrap();
            assert!(page < height);
            assert_eq!(position, 0);
            reveal_control(form, edit);
            assert!(metrics(form).unwrap().2 > 0);
            assert!(state(form).unwrap().x.get() > 0);
            let mut text = [0u16; 32];
            GetWindowTextW(edit, &mut text);
            assert!(String::from_utf16_lossy(&text).starts_with("keep this draft"));
            let old = crate::dpi::window_dpi(form).unwrap();
            let next = old * 2;
            let recommended = RECT {
                left: work.left,
                top: work.top,
                right: work.left + width * 2,
                bottom: work.top + height * 2,
            };
            SendMessageW(
                form,
                WM_DPICHANGED,
                Some(WPARAM((next | next << 16) as usize)),
                Some(LPARAM(&recommended as *const _ as isize)),
            );
            assert_eq!(metrics(form).unwrap().0, height * 2);
            reveal_control(form, edit);
            let mut rect = RECT::default();
            GetWindowRect(edit, &mut rect).unwrap();
            assert!(rect.top >= work.top && rect.bottom <= work.bottom);
            assert!(rect.left >= work.left && rect.right <= work.right);
            assert!(IsWindow(Some(edit)).as_bool());
            DestroyWindow(form).unwrap();
            assert!(metrics(form).is_none());
        }
    }
}
