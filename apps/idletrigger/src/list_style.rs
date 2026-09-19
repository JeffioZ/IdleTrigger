//! Native report-header interaction using the application palette, plus a
//! compact client scrollbar where Windows non-client colors are unreliable.
use std::cell::Cell;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{
    DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

const SUBCLASS: usize = 0x49544c53;
const HEADER_SUBCLASS: usize = 0x49544845;
#[derive(Default)]
struct HeaderState {
    hovered: Cell<Option<usize>>,
    pressed: Cell<Option<usize>>,
}

unsafe fn with_header_state<R>(hwnd: HWND, read: impl FnOnce(&HeaderState) -> R) -> Option<R> {
    unsafe {
        let mut data = 0;
        GetWindowSubclass(hwnd, Some(header_proc), HEADER_SUBCLASS, Some(&mut data))
            .as_bool()
            .then(|| read(&*(data as *const HeaderState)))
    }
}

unsafe fn header_column(hwnd: HWND, lp: LPARAM) -> Option<usize> {
    unsafe {
        let x = lp.0 as i16 as i32;
        let y = (lp.0 >> 16) as i16 as i32;
        let count = SendMessageW(hwnd, HDM_GETITEMCOUNT, None, None).0;
        for column in 0..count.max(0) as usize {
            let mut bounds = RECT::default();
            if SendMessageW(
                hwnd,
                HDM_GETITEMRECT,
                Some(WPARAM(column)),
                Some(LPARAM(&mut bounds as *mut _ as isize)),
            )
            .0 != 0
                && x >= bounds.left
                && x < bounds.right
                && y >= bounds.top
                && y < bounds.bottom
            {
                return Some(column);
            }
        }
        None
    }
}

unsafe fn update_header(hwnd: HWND, hovered: Option<usize>, pressed: Option<usize>) {
    unsafe {
        with_header_state(hwnd, |state| {
            let changed = (state.hovered.replace(hovered) != hovered)
                | (state.pressed.replace(pressed) != pressed);
            if changed {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        });
    }
}

unsafe extern "system" fn header_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        let state = &*(data as *const HeaderState);
        match msg {
            WM_MOUSEMOVE => {
                update_header(hwnd, header_column(hwnd, lp), state.pressed.get());
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    ..Default::default()
                };
                let _ = TrackMouseEvent(&mut tracking);
            }
            WM_LBUTTONDOWN => {
                let column = header_column(hwnd, lp);
                update_header(hwnd, column, column);
            }
            WM_LBUTTONUP => {
                // Deliver the native click/sort notification first, as in Go.
                let result = DefSubclassProc(hwnd, msg, wp, lp);
                update_header(hwnd, header_column(hwnd, lp), None);
                return result;
            }
            WM_MOUSELEAVE => update_header(hwnd, None, None),
            WM_CANCELMODE | WM_CAPTURECHANGED => update_header(hwnd, state.hovered.get(), None),
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(header_proc), id);
                drop(Box::from_raw(data as *mut HeaderState));
                return DefSubclassProc(hwnd, msg, wp, lp);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}
struct ListState {
    bar: HWND,
    syncing: Cell<bool>,
    wheel_delta: Cell<i32>,
}
struct BarState {
    list: HWND,
    drag: Cell<Option<i32>>,
    hover: Cell<bool>,
}
pub fn is_scrollbar(hwnd: HWND) -> bool {
    unsafe { GetWindowSubclass(hwnd, Some(bar_proc), SUBCLASS, None).as_bool() }
}
pub fn refresh(hwnd: HWND) {
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(list_proc), SUBCLASS, Some(&mut data)).as_bool() {
            sync(hwnd, &*(data as *const ListState));
        }
    }
}

pub fn install(list: HWND) {
    unsafe {
        if GetWindowSubclass(list, Some(list_proc), SUBCLASS, None).as_bool() {
            return;
        }
        SetWindowLongW(
            list,
            GWL_STYLE,
            GetWindowLongW(list, GWL_STYLE) | WS_CLIPCHILDREN.0 as i32,
        );
        let Ok(bar) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_CHILD | WS_CLIPSIBLINGS | WINDOW_STYLE(0x0100), // SS_NOTIFY
            0,
            0,
            1,
            1,
            Some(list),
            None,
            None,
            None,
        ) else {
            return;
        };
        let bar_state = Box::into_raw(Box::new(BarState {
            list,
            drag: Cell::new(None),
            hover: Cell::new(false),
        }));
        if !SetWindowSubclass(bar, Some(bar_proc), SUBCLASS, bar_state as usize).as_bool() {
            drop(Box::from_raw(bar_state));
            let _ = DestroyWindow(bar);
            return;
        }
        let state = Box::into_raw(Box::new(ListState {
            bar,
            syncing: Cell::new(false),
            wheel_delta: Cell::new(0),
        }));
        if !SetWindowSubclass(list, Some(list_proc), SUBCLASS, state as usize).as_bool() {
            drop(Box::from_raw(state));
            let _ = DestroyWindow(bar);
            return;
        }
        let header = list_header(list);
        if !header.is_invalid() {
            let tracking = Box::into_raw(Box::new(HeaderState::default()));
            if !SetWindowSubclass(
                header,
                Some(header_proc),
                HEADER_SUBCLASS,
                tracking as usize,
            )
            .as_bool()
            {
                drop(Box::from_raw(tracking));
            }
        }
        sync(list, &*state);
    }
}

fn px(list: HWND, value: i32) -> i32 {
    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(list) }.max(96);
    ((value as i64 * dpi as i64 * crate::dpi::text_percent() as i64 / 9600) as i32).max(1)
}

unsafe fn metrics(list: HWND) -> (i32, i32, i32) {
    unsafe {
        if let Some(metrics) = crate::viewport::metrics(list) {
            return metrics;
        }
        if is_listbox(list) {
            let mut client = RECT::default();
            let _ = GetClientRect(list, &mut client);
            let row = SendMessageW(list, LB_GETITEMHEIGHT, Some(WPARAM(0)), None)
                .0
                .max(1) as i32;
            return (
                SendMessageW(list, LB_GETCOUNT, None, None).0.max(0) as i32,
                (client.bottom / row).max(1),
                SendMessageW(list, LB_GETTOPINDEX, None, None).0.max(0) as i32,
            );
        }
        (
            SendMessageW(list, LVM_GETITEMCOUNT, None, None).0 as i32,
            SendMessageW(list, LVM_GETCOUNTPERPAGE, None, None).0 as i32,
            SendMessageW(list, LVM_GETTOPINDEX, None, None).0 as i32,
        )
    }
}

unsafe fn is_listbox(list: HWND) -> bool {
    let mut name = [0u16; 32];
    let len = unsafe { GetClassNameW(list, &mut name) };
    String::from_utf16_lossy(&name[..len.max(0) as usize]).eq_ignore_ascii_case("ListBox")
}

unsafe fn list_header(list: HWND) -> HWND {
    unsafe {
        if is_listbox(list) || crate::viewport::metrics(list).is_some() {
            HWND::default()
        } else {
            HWND(SendMessageW(list, LVM_GETHEADER, None, None).0 as *mut _)
        }
    }
}

unsafe fn sync(list: HWND, state: &ListState) {
    unsafe {
        if state.syncing.replace(true) {
            return;
        }
        let _ = ShowScrollBar(list, SB_VERT, false);
        let mut client = RECT::default();
        let _ = GetClientRect(list, &mut client);
        let header = list_header(list);
        let mut bounds = RECT::default();
        let _ = GetWindowRect(header, &mut bounds);
        let height = bounds.bottom - bounds.top;
        let width = px(list, 14);
        let _ = SetWindowPos(
            state.bar,
            Some(HWND::default()),
            client.right - width,
            height,
            width,
            (client.bottom - height).max(1),
            SWP_NOACTIVATE,
        );
        let (total, page, _) = metrics(list);
        let _ = ShowWindow(
            state.bar,
            if total > page && page > 0 {
                SW_SHOWNA
            } else {
                SW_HIDE
            },
        );
        let _ = InvalidateRect(Some(state.bar), None, false);
        state.syncing.set(false);
    }
}

unsafe fn scroll_to(list: HWND, position: i32) {
    unsafe {
        if crate::viewport::scroll_to(list, position) {
            return;
        }
        let (total, page, current) = metrics(list);
        let position = position.clamp(0, (total - page).max(0));
        if is_listbox(list) {
            let _ = SendMessageW(list, LB_SETTOPINDEX, Some(WPARAM(position as usize)), None);
            return;
        }
        let mut row = RECT::default(); // LVIR_BOUNDS = 0 in left.
        if SendMessageW(
            list,
            LVM_GETITEMRECT,
            Some(WPARAM(0)),
            Some(LPARAM(&mut row as *mut _ as isize)),
        )
        .0 != 0
        {
            let delta = (position - current) * (row.bottom - row.top).max(1);
            let _ = SendMessageW(
                list,
                LVM_SCROLL,
                Some(WPARAM(0)),
                Some(LPARAM(delta as isize)),
            );
        }
    }
}

pub fn thumb(total: i32, page: i32, position: i32, height: i32, minimum: i32) -> (i32, i32) {
    let height = height.max(0);
    if total <= page || page <= 0 {
        return (0, height);
    }
    let size = ((height as i64 * page as i64 / total as i64) as i32)
        .max(minimum)
        .min(height);
    let top = ((height - size) as i64 * position.clamp(0, total - page) as i64
        / (total - page) as i64) as i32;
    (top, top + size)
}

unsafe fn bar_geometry(bar: HWND, list: HWND) -> (RECT, RECT) {
    unsafe {
        let mut track = RECT::default();
        let _ = GetClientRect(bar, &mut track);
        let inset = px(list, 2);
        track.left += inset;
        track.right -= inset;
        track.top += inset;
        track.bottom -= inset;
        let (total, page, position) = metrics(list);
        let (top, bottom) = thumb(
            total,
            page,
            position,
            track.bottom - track.top,
            px(list, 24),
        );
        (
            track,
            RECT {
                top: track.top + top,
                bottom: track.top + bottom,
                ..track
            },
        )
    }
}

unsafe fn paint_bar(bar: HWND, dc: HDC, state: &BarState) {
    unsafe {
        let p = crate::theme::palette();
        let mut bounds = RECT::default();
        let _ = GetClientRect(bar, &mut bounds);
        crate::paint::fill_rect(dc, &bounds, p.surface);
        let (track, thumb) = bar_geometry(bar, state.list);
        draw_scrollbar(
            dc,
            &track,
            &thumb,
            px(state.list, 4),
            state.hover.get(),
            state.drag.get().is_some(),
        );
    }
}

pub fn draw_scrollbar(
    dc: HDC,
    track: &RECT,
    thumb: &RECT,
    radius: i32,
    hovered: bool,
    pressed: bool,
) {
    let p = crate::theme::palette();
    let track_color = if hovered {
        p.hover_surface
    } else {
        p.disabled_surface
    };
    let color = if pressed {
        p.accent_pressed
    } else if hovered {
        p.text2
    } else {
        p.border
    };
    let _ = crate::paint::fill_rounded_rect(dc, track, radius, track_color, track_color);
    let _ = crate::paint::fill_rounded_rect(dc, thumb, radius, color, color);
}

unsafe extern "system" fn bar_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        let state = &*(data as *const BarState);
        let y = (lp.0 >> 16) as i16 as i32;
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                paint_bar(hwnd, dc, state);
                let _ = EndPaint(hwnd, &ps);
                return LRESULT(0);
            }
            WM_PRINTCLIENT => {
                paint_bar(hwnd, HDC(wp.0 as *mut _), state);
                return LRESULT(0);
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_LBUTTONDOWN => {
                let (track, thumb) = bar_geometry(hwnd, state.list);
                let (_, page, position) = metrics(state.list);
                let _ = SetFocus(Some(state.list));
                if y >= thumb.top && y < thumb.bottom {
                    state.drag.set(Some(y - thumb.top));
                    SetCapture(hwnd);
                } else if y >= track.top && y < track.bottom {
                    scroll_to(
                        state.list,
                        position + if y < thumb.top { -page } else { page },
                    );
                }
            }
            WM_MOUSEMOVE => {
                state.hover.set(true);
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    ..Default::default()
                };
                let _ = TrackMouseEvent(&mut tracking);
                if let Some(offset) = state.drag.get() {
                    let (track, thumb) = bar_geometry(hwnd, state.list);
                    let travel = track.bottom - track.top - (thumb.bottom - thumb.top);
                    let (total, page, _) = metrics(state.list);
                    if travel > 0 {
                        scroll_to(
                            state.list,
                            ((y - offset - track.top).clamp(0, travel) as i64
                                * (total - page).max(0) as i64
                                / travel as i64) as i32,
                        );
                    }
                }
            }
            WM_LBUTTONUP | WM_CANCELMODE => {
                state.drag.set(None);
                let _ = ReleaseCapture();
            }
            WM_CAPTURECHANGED => state.drag.set(None),
            WM_MOUSELEAVE => state.hover.set(false),
            WM_MOUSEWHEEL => return SendMessageW(state.list, msg, Some(wp), Some(lp)),
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(bar_proc), id);
                drop(Box::from_raw(data as *mut BarState));
                return DefSubclassProc(hwnd, msg, wp, lp);
            }
            _ => return DefSubclassProc(hwnd, msg, wp, lp),
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
        LRESULT(0)
    }
}

unsafe fn draw_header(draw: &NMCUSTOMDRAW) -> LRESULT {
    unsafe {
        let p = crate::theme::palette();
        let mut bounds = draw.rc;
        if draw.dwDrawStage == CDDS_PREPAINT {
            let _ = GetClientRect(draw.hdr.hwndFrom, &mut bounds);
            crate::paint::fill_rect(draw.hdc, &bounds, p.elevated);
            return LRESULT((CDRF_NOTIFYITEMDRAW | CDRF_NOTIFYPOSTPAINT) as isize);
        }
        if draw.dwDrawStage == CDDS_POSTPAINT {
            let header = draw.hdr.hwndFrom;
            let count = SendMessageW(header, HDM_GETITEMCOUNT, None, None).0;
            if count > 0 {
                let mut last = RECT::default();
                let _ = GetClientRect(header, &mut bounds);
                let _ = SendMessageW(
                    header,
                    HDM_GETITEMRECT,
                    Some(WPARAM(count as usize - 1)),
                    Some(LPARAM(&mut last as *mut _ as isize)),
                );
                bounds.left = last.right;
                if bounds.left < bounds.right {
                    crate::paint::fill_rect(draw.hdc, &bounds, p.elevated);
                }
            }
            return LRESULT(CDRF_DODEFAULT as isize);
        }
        if draw.dwDrawStage != CDDS_ITEMPREPAINT {
            return LRESULT(CDRF_DODEFAULT as isize);
        }
        // Native custom-draw flags do not reliably identify hot/pressed
        // header columns. Go tracks these explicitly on the Header HWND.
        let (hovered, pressed) = with_header_state(draw.hdr.hwndFrom, |s| {
            (
                s.hovered.get() == Some(draw.dwItemSpec),
                s.pressed.get() == Some(draw.dwItemSpec),
            )
        })
        .unwrap_or_default();
        let (fill, color, divider) = if pressed {
            (p.accent_pressed, p.accent_text, p.accent_pressed)
        } else if hovered {
            (p.hover_surface, p.text, p.accent)
        } else {
            (p.elevated, p.text, p.subtle_border)
        };
        crate::paint::fill_rect(draw.hdc, &bounds, fill);
        let line = px(draw.hdr.hwndFrom, 1);
        crate::paint::fill_rect(
            draw.hdc,
            &RECT {
                top: bounds.bottom - line,
                ..bounds
            },
            divider,
        );
        crate::paint::fill_rect(
            draw.hdc,
            &RECT {
                left: bounds.right - line,
                ..bounds
            },
            p.subtle_border,
        );
        let mut text = [0u16; 512];
        let mut item = HDITEMW {
            mask: HDI_TEXT,
            pszText: windows::core::PWSTR(text.as_mut_ptr()),
            cchTextMax: text.len() as i32,
            ..Default::default()
        };
        let _ = SendMessageW(
            draw.hdr.hwndFrom,
            HDM_GETITEMW,
            Some(WPARAM(draw.dwItemSpec)),
            Some(LPARAM(&mut item as *mut _ as isize)),
        );
        bounds.left += px(draw.hdr.hwndFrom, 8);
        bounds.right -= px(draw.hdr.hwndFrom, 6);
        let saved = SaveDC(draw.hdc);
        let font = SendMessageW(draw.hdr.hwndFrom, WM_GETFONT, None, None).0;
        if font != 0 {
            SelectObject(draw.hdc, HGDIOBJ(font as *mut _));
        }
        SetTextColor(draw.hdc, COLORREF(color));
        SetBkMode(draw.hdc, TRANSPARENT);
        let mut length = text.iter().position(|v| *v == 0).unwrap_or(text.len());
        if length > 0 && matches!(text[length - 1], 0x2191 | 0x2193) {
            let mut arrow = [text[length - 1]];
            length -= 1;
            while length > 0 && text[length - 1] == b' ' as u16 {
                length -= 1;
            }
            let width = px(draw.hdr.hwndFrom, 14);
            let gap = px(draw.hdr.hwndFrom, 6);
            let mut measured = bounds;
            DrawTextW(
                draw.hdc,
                &mut text[..length],
                &mut measured,
                DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
            );
            let mut arrow_bounds = RECT {
                left: (measured.right + gap)
                    .min(bounds.right - width)
                    .max(bounds.left),
                ..bounds
            };
            bounds.right = (arrow_bounds.left - gap).max(bounds.left);
            DrawTextW(
                draw.hdc,
                &mut arrow,
                &mut arrow_bounds,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
        DrawTextW(
            draw.hdc,
            &mut text[..length],
            &mut bounds,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        let _ = RestoreDC(draw.hdc, saved);
        LRESULT(CDRF_SKIPDEFAULT as isize)
    }
}

unsafe extern "system" fn list_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        let state = &*(data as *const ListState);
        if matches!(msg, WM_MOUSEWHEEL | WM_MOUSEHWHEEL)
            && crate::viewport::horizontal_wheel(hwnd, msg, wp)
        {
            return LRESULT(0);
        }
        if msg == WM_MOUSEWHEEL {
            let delta = state.wheel_delta.get() + (wp.0 >> 16) as i16 as i32;
            state.wheel_delta.set(delta % 120);
            let (_, page, position) = metrics(hwnd);
            let mut lines = 3u32;
            let _ = SystemParametersInfoW(
                SPI_GETWHEELSCROLLLINES,
                0,
                Some((&mut lines as *mut u32).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            let rows = if lines == u32::MAX {
                page
            } else {
                (lines.min(i32::MAX as u32) as i32).saturating_mul(
                    if crate::viewport::metrics(hwnd).is_some() {
                        px(hwnd, 24)
                    } else {
                        1
                    },
                )
            };
            scroll_to(
                hwnd,
                position.saturating_sub((delta / 120).saturating_mul(rows)),
            );
            return LRESULT(0);
        }
        if msg == WM_NOTIFY && lp.0 != 0 {
            let hdr = &*(lp.0 as *const NMHDR);
            if hdr.code == NM_CUSTOMDRAW
                && hdr.hwndFrom.0 as isize == SendMessageW(hwnd, LVM_GETHEADER, None, None).0
            {
                return draw_header(&*(lp.0 as *const NMCUSTOMDRAW));
            }
        }
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(list_proc), id);
            drop(Box::from_raw(data as *mut ListState));
            return DefSubclassProc(hwnd, msg, wp, lp);
        }
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        if matches!(
            msg,
            WM_SIZE
                | WM_PAINT
                | WM_VSCROLL
                | WM_MOUSEWHEEL
                | WM_KEYDOWN
                | WM_THEMECHANGED
                | WM_SETREDRAW
                | LVM_INSERTITEMW
                | LVM_DELETEALLITEMS
                | LVM_DELETEITEM
                | LVM_SCROLL
                | LVM_ENSUREVISIBLE
                | LB_ADDSTRING
                | LB_INSERTSTRING
                | LB_DELETESTRING
                | LB_RESETCONTENT
                | LB_SETTOPINDEX
                | LB_SETCURSEL
                | LB_SETITEMHEIGHT
        ) {
            sync(hwnd, state);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_list_scrolls_with_client_bar_and_releases_subclasses() {
        let _guard = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        unsafe {
            InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            })
            .unwrap();
            let parent = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                0,
                0,
                320,
                240,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let list = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("SysListView32"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WINDOW_STYLE(LVS_REPORT),
                0,
                0,
                300,
                200,
                Some(parent),
                None,
                None,
                None,
            )
            .unwrap();
            let column = LVCOLUMNW {
                mask: LVCF_WIDTH,
                cx: 220,
                ..Default::default()
            };
            SendMessageW(
                list,
                LVM_INSERTCOLUMNW,
                Some(WPARAM(0)),
                Some(LPARAM(&column as *const _ as isize)),
            );
            for i in 0..100 {
                let item = LVITEMW {
                    iItem: i,
                    ..Default::default()
                };
                assert!(
                    SendMessageW(
                        list,
                        LVM_INSERTITEMW,
                        None,
                        Some(LPARAM(&item as *const _ as isize))
                    )
                    .0 >= 0
                );
            }
            install(list);
            let header = HWND(SendMessageW(list, LVM_GETHEADER, None, None).0 as *mut _);
            let point_in_header = LPARAM((5 << 16) | 10);
            let palette = crate::theme::palette();
            let assert_colors = |fill: u32, divider: u32| {
                let dc = GetDC(Some(header));
                let memory = CreateCompatibleDC(Some(dc));
                let bitmap = CreateCompatibleBitmap(dc, 100, 40);
                let old = SelectObject(memory, HGDIOBJ(bitmap.0));
                let draw = NMCUSTOMDRAW {
                    hdr: NMHDR {
                        hwndFrom: header,
                        code: NM_CUSTOMDRAW,
                        ..Default::default()
                    },
                    dwDrawStage: CDDS_ITEMPREPAINT,
                    hdc: memory,
                    rc: RECT {
                        left: 0,
                        top: 0,
                        right: 100,
                        bottom: 40,
                    },
                    dwItemSpec: 0,
                    ..Default::default() // No native CDIS_HOT/SELECTED flags.
                };
                assert_eq!(
                    SendMessageW(
                        list,
                        WM_NOTIFY,
                        None,
                        Some(LPARAM(&draw as *const _ as isize))
                    )
                    .0,
                    CDRF_SKIPDEFAULT as isize
                );
                let actual_fill = GetPixel(memory, 2, 2).0;
                let actual_divider = GetPixel(memory, 20, 39).0;
                SelectObject(memory, old);
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                let _ = DeleteDC(memory);
                ReleaseDC(Some(header), dc);
                assert_eq!(actual_fill, fill);
                assert_eq!(actual_divider, divider);
            };
            assert_colors(palette.elevated, palette.subtle_border);
            SendMessageW(header, WM_MOUSEMOVE, None, Some(point_in_header));
            assert_colors(palette.hover_surface, palette.accent);
            SendMessageW(
                header,
                WM_LBUTTONDOWN,
                Some(WPARAM(1)),
                Some(point_in_header),
            );
            assert_colors(palette.accent_pressed, palette.accent_pressed);
            SendMessageW(header, WM_LBUTTONUP, None, Some(point_in_header));
            assert_colors(palette.hover_surface, palette.accent);
            SendMessageW(
                header,
                WM_LBUTTONDOWN,
                Some(WPARAM(1)),
                Some(point_in_header),
            );
            SendMessageW(header, WM_CANCELMODE, None, None);
            assert_colors(palette.hover_surface, palette.accent);
            SendMessageW(header, WM_MOUSELEAVE, None, None);
            assert_colors(palette.elevated, palette.subtle_border);
            let bar = GetWindow(list, GW_CHILD).unwrap();
            let (total, page, _) = metrics(list);
            assert_eq!(total, 100);
            assert!(page > 0 && page < total);
            assert_eq!(GetWindowLongW(list, GWL_STYLE) as u32 & WS_VSCROLL.0, 0);
            scroll_to(list, total - page);
            assert_eq!(metrics(list).2, total - page);
            scroll_to(list, 0);
            let (_, thumb) = bar_geometry(bar, list);
            let point = |y: i32| LPARAM(((y as u32) << 16 | 5) as isize);
            SendMessageW(bar, WM_LBUTTONDOWN, None, Some(point(thumb.top + 1)));
            SendMessageW(bar, WM_MOUSEMOVE, Some(WPARAM(1)), Some(point(1000)));
            SendMessageW(bar, WM_LBUTTONUP, None, Some(point(1000)));
            assert_eq!(metrics(list).2, total - page);
            DestroyWindow(parent).unwrap();
            assert!(!IsWindow(Some(bar)).as_bool());
        }
    }
    #[test]
    fn thumb_reaches_both_ends_and_handles_tiny_tracks() {
        assert_eq!(thumb(100, 10, 0, 100, 24), (0, 24));
        assert_eq!(thumb(100, 10, 90, 100, 24), (76, 100));
        assert_eq!(thumb(100, 10, 999, 10, 24), (0, 10));
        assert_eq!(thumb(0, 0, 0, 100, 24), (0, 100));
    }

    #[test]
    fn listbox_uses_the_same_scrollbar_without_losing_selection() {
        let _guard = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        unsafe {
            let parent = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
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
            let list = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("LISTBOX"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_VSCROLL,
                0,
                0,
                300,
                180,
                Some(parent),
                None,
                None,
                None,
            )
            .unwrap();
            install(list);
            install(list); // Installing twice must not replace/leak the subclass.
            for _ in 0..100 {
                SendMessageW(
                    list,
                    LB_ADDSTRING,
                    None,
                    Some(LPARAM(w!("row").as_ptr() as isize)),
                );
            }
            SendMessageW(list, LB_SETCURSEL, Some(WPARAM(50)), None);
            scroll_to(list, 90);
            let (count, page, top) = metrics(list);
            assert_eq!(count, 100);
            assert_eq!(top, 90.min(count - page));
            assert_eq!(SendMessageW(list, LB_GETCURSEL, None, None).0, 50);
            assert_eq!(GetWindowLongW(list, GWL_STYLE) as u32 & WS_VSCROLL.0, 0);
            DestroyWindow(parent).unwrap();
        }
    }
}
