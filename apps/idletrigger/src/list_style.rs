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
    host: HWND,
    bar: HWND,
    syncing: Cell<bool>,
    wheel_delta: Cell<i32>,
    painted: Cell<(i32, i32, i32, bool)>,
    lane: Cell<i32>,
    redraw: Cell<bool>,
}
struct BarState {
    list: HWND,
    drag: Cell<Option<i32>>,
    hover: Cell<bool>,
}

/// Painted-scrollbar lanes: a LISTBOX scrollbar is NOT a window. It is
/// painted into the card face that hosts the list (same grammar as the
/// dropdown flyout), and the form routes pointer input for the strip.
/// Painting and scrolling then share one pixel pipeline inside one message
/// dispatch - a separate scrollbar HWND has its own paint events, and on
/// some machines that window's repaints composite as visible flicker no
/// matter how tightly they are synchronized.
// HWND is not Send, so the cross-thread registries hold raw handles.
// (list, card, hovered)
static LANES: std::sync::Mutex<Vec<(isize, isize, bool)>> = std::sync::Mutex::new(Vec::new());
static LANE_DRAG: std::sync::Mutex<Option<(isize, i32)>> = std::sync::Mutex::new(None);

pub fn is_scrollbar(hwnd: HWND) -> bool {
    unsafe { GetWindowSubclass(hwnd, Some(bar_proc), SUBCLASS, None).as_bool() }
}
pub fn refresh(hwnd: HWND) {
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(list_proc), SUBCLASS, Some(&mut data)).as_bool() {
            let state = &*(data as *const ListState);
            // Force one repaint: external refreshes (theme flips) change the
            // palette while the scroll metrics may be identical.
            state.painted.set((i32::MIN, i32::MIN, i32::MIN, false));
            sync(hwnd, state);
        }
    }
}

/// Registers which card face paints a listbox's scrollbar lane. Called by
/// the host window code right after creating list + card; dead entries are
/// purged opportunistically.
pub fn set_lane_card(list: HWND, card: HWND) {
    unsafe {
        let mut lanes = crate::runtime::lock(&LANES);
        lanes.retain(|(list, card, _)| {
            IsWindow(Some(HWND(*list as *mut _))).as_bool()
                && IsWindow(Some(HWND(*card as *mut _))).as_bool()
        });
        let key = (list.0 as isize, card.0 as isize, false);
        if let Some(existing) = lanes.iter_mut().find(|(list, _, _)| *list == key.0) {
            existing.1 = key.1;
        } else {
            lanes.push(key);
        }
    }
}

/// The lane strip beside a list, in `host` CLIENT coordinates.
unsafe fn lane_rect(list: HWND, host: HWND) -> RECT {
    unsafe {
        let mut frame = RECT::default();
        let _ = GetWindowRect(list, &mut frame);
        let mut corners = [
            windows::Win32::Foundation::POINT {
                x: frame.left,
                y: frame.top,
            },
            windows::Win32::Foundation::POINT {
                x: frame.right,
                y: frame.bottom,
            },
        ];
        let _ = MapWindowPoints(None, Some(host), &mut corners);
        let width = px(list, 14);
        let gap = px(list, 3);
        RECT {
            left: corners[1].x + gap,
            top: corners[0].y,
            right: corners[1].x + gap + width,
            bottom: corners[1].y,
        }
    }
}

fn lane_card(list: HWND) -> Option<HWND> {
    crate::runtime::lock(&LANES)
        .iter()
        .find(|(l, _, _)| *l == list.0 as isize)
        .map(|(_, card, _)| HWND(*card as *mut _))
}

/// Synchronous repaint of one list's lane strip (pressed/hover feedback and
/// scroll updates share this path).
unsafe fn repaint_lane(list: HWND) {
    unsafe {
        let Some(card) = lane_card(list) else {
            return;
        };
        let mut size = RECT::default();
        if GetClientRect(card, &mut size).is_err() {
            return;
        }
        let strip = lane_strip_on_card(list, card, size.right, size.bottom);
        if strip.right > strip.left && strip.bottom > strip.top {
            let _ = RedrawWindow(
                Some(card),
                Some(&strip),
                None,
                REDRAW_WINDOW_FLAGS(RDW_INVALIDATE.0 | RDW_UPDATENOW.0),
            );
        }
    }
}

/// The lane strip of `list` in `card`'s client coordinate space, clamped
/// to (0,0,width,height). Empty rect when it falls outside.
unsafe fn lane_strip_on_card(list: HWND, card: HWND, width: i32, height: i32) -> RECT {
    unsafe {
        let host = GetParent(list).unwrap_or(list);
        let lane = lane_rect(list, host);
        let mut frame = RECT::default();
        if GetWindowRect(card, &mut frame).is_err() {
            return RECT::default();
        }
        let mut corners = [
            windows::Win32::Foundation::POINT {
                x: frame.left,
                y: frame.top,
            },
            windows::Win32::Foundation::POINT {
                x: frame.right,
                y: frame.bottom,
            },
        ];
        let _ = MapWindowPoints(None, Some(host), &mut corners);
        RECT {
            left: (lane.left - corners[0].x).max(0),
            top: (lane.top - corners[0].y).max(0),
            right: (lane.right - corners[0].x).min(width),
            bottom: (lane.bottom - corners[0].y).min(height),
        }
    }
}

/// Paints the scrollbar lane of the list hosted on `card`, in the card's
/// own coordinate space (0,0,width,height). No-op when the card hosts no
/// registered lane or the list does not overflow.
pub unsafe fn paint_lane(dc: HDC, card: HWND, width: i32, height: i32) {
    unsafe {
        let (list, hovered) = {
            let lanes = crate::runtime::lock(&LANES);
            match lanes
                .iter()
                .find(|(l, c, _)| {
                    *c == card.0 as isize && IsWindow(Some(HWND(*l as *mut _))).as_bool()
                })
                .map(|(l, _, h)| (HWND(*l as *mut _), *h))
            {
                Some(found) => found,
                None => return,
            }
        };
        let (total, page, position) = metrics(list);
        if total <= page || page <= 0 {
            return;
        }
        let strip = lane_strip_on_card(list, card, width, height);
        if strip.right <= strip.left || strip.bottom <= strip.top {
            return;
        }
        let inset = px(list, 2);
        let track = RECT {
            left: strip.left + inset,
            top: strip.top + inset,
            right: strip.right - inset,
            bottom: strip.bottom - inset,
        };
        let (top, bottom) = thumb(
            total,
            page,
            position,
            track.bottom - track.top,
            px(list, 24),
        );
        let thumb_rect = RECT {
            top: track.top + top,
            bottom: track.top + bottom,
            ..track
        };
        let pressed = crate::runtime::lock(&LANE_DRAG).is_some_and(|(l, _)| l == list.0 as isize);
        draw_scrollbar(dc, &track, &thumb_rect, px(list, 4), hovered, pressed);
    }
}

/// NCHITTEST support: true when the screen point lands on a live lane strip
/// that no other window covers - the form then reports HTCLIENT so mouse
/// messages flow to `lane_pointer`.
pub unsafe fn lane_hit(form: HWND, lp: LPARAM) -> bool {
    unsafe {
        // NCHITTEST delivers SCREEN coordinates; the lane rects live in the
        // form's client space. No WindowFromPoint coverage probe here - its
        // HTTRANSPARENT pass-through SENDS WM_NCHITTEST, re-entering this
        // handler in an infinite recursion (the blank-drag lesson). The
        // probe is redundant anyway: if this handler runs for a point, no
        // opaque child claimed it, so the form owns the pixel.
        let mut local = windows::Win32::Foundation::POINT {
            x: lp.0 as i16 as i32,
            y: (lp.0 >> 16) as i16 as i32,
        };
        let _ = ScreenToClient(form, &mut local);
        // Collect the candidate first, evaluate after the lock is released:
        // metrics() sends messages that can re-enter the registry.
        let candidate = crate::runtime::lock(&LANES)
            .iter()
            .filter(|(list, _, _)| {
                let list = HWND(*list as *mut _);
                IsWindow(Some(list)).as_bool() && GetParent(list) == Ok(form)
            })
            .map(|(list, _, _)| HWND(*list as *mut _))
            .find(|list| {
                let rect = lane_rect(*list, form);
                local.x >= rect.left
                    && local.x < rect.right
                    && local.y >= rect.top
                    && local.y < rect.bottom
            });
        match candidate {
            Some(list) => {
                let (total, page, _) = metrics(list);
                total > page && page > 0
            }
            None => false,
        }
    }
}

/// Pointer input for painted lanes; the form forwards WM_LBUTTONDOWN /
/// WM_MOUSEMOVE / WM_LBUTTONUP (and WM_CANCELMODE) here first. Returns true
/// when the message was a lane interaction and is fully handled.
pub unsafe fn lane_pointer(form: HWND, msg: u32, _wp: WPARAM, lp: LPARAM) -> bool {
    unsafe {
        let x = lp.0 as i16 as i32;
        let y = (lp.0 >> 16) as i16 as i32;
        match msg {
            WM_LBUTTONDOWN => {
                // Collect the candidate list BEFORE any scrolling: the
                // registry lock must be released before scroll_to - the
                // scroll's sync path re-locks the registry (lane_card), and
                // a std Mutex is not reentrant.
                let candidate = {
                    let lanes = crate::runtime::lock(&LANES);
                    let mut found = None;
                    for (list, _) in lanes.iter().map(|(l, c, _)| (l, c)) {
                        let lane_list = HWND(*list as *mut _);
                        if !IsWindow(Some(lane_list)).as_bool() || GetParent(lane_list) != Ok(form)
                        {
                            continue;
                        }
                        let rect = lane_rect(lane_list, form);
                        if x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom {
                            found = Some(lane_list);
                            break;
                        }
                    }
                    found
                };
                let Some(lane_list) = candidate else {
                    return false;
                };
                let (total, page, position) = metrics(lane_list);
                if total <= page || page <= 0 {
                    return false;
                }
                let _ = SetFocus(Some(lane_list));
                let rect = lane_rect(lane_list, form);
                let inset = px(lane_list, 2);
                let track_top = rect.top + inset;
                let track_bottom = rect.bottom - inset;
                let (top, bottom) = thumb(
                    total,
                    page,
                    position,
                    track_bottom - track_top,
                    px(lane_list, 24),
                );
                let thumb_top = track_top + top;
                let thumb_bottom = track_top + bottom;
                if y >= thumb_top && y < thumb_bottom {
                    *crate::runtime::lock(&LANE_DRAG) = Some((lane_list.0 as isize, y - thumb_top));
                } else {
                    scroll_to(
                        lane_list,
                        position + if y < thumb_top { -page } else { page },
                    );
                }
                let _ = SetCapture(form);
                // Immediate pressed feedback: without this a thumb press
                // gives no visual response until the first move.
                repaint_lane(lane_list);
                true
            }
            WM_MOUSEMOVE => {
                let Some((list, offset)) = *crate::runtime::lock(&LANE_DRAG) else {
                    // Not dragging: maintain the lane hover tint. Leaving
                    // the form entirely is handled by WM_MOUSELEAVE.
                    update_lane_hover(form, x, y);
                    return false;
                };
                let list = HWND(list as *mut _);
                if !IsWindow(Some(list)).as_bool() {
                    *crate::runtime::lock(&LANE_DRAG) = None;
                    return false;
                }
                let rect = lane_rect(list, form);
                let inset = px(list, 2);
                let track_top = rect.top + inset;
                let track_bottom = rect.bottom - inset;
                let (total, page, _) = metrics(list);
                let travel = track_bottom
                    - track_top
                    - thumb(total, page, 0, track_bottom - track_top, px(list, 24)).1;
                if travel > 0 {
                    scroll_to(
                        list,
                        drag_position(y - offset, track_top, travel, total - page),
                    );
                }
                true
            }
            WM_LBUTTONUP | WM_CANCELMODE | WM_CAPTURECHANGED => {
                let had = crate::runtime::lock(&LANE_DRAG).is_some();
                if had {
                    let (list, _) = crate::runtime::lock(&LANE_DRAG).unwrap();
                    *crate::runtime::lock(&LANE_DRAG) = None;
                    if msg != WM_CAPTURECHANGED {
                        let _ = ReleaseCapture();
                    }
                    if IsWindow(Some(HWND(list as *mut _))).as_bool() {
                        repaint_lane(HWND(list as *mut _));
                    }
                }
                had
            }
            WM_MOUSELEAVE => {
                clear_lane_hover(form);
                false
            }
            _ => false,
        }
    }
}

/// Hover bookkeeping for painted lanes: flips the hovered flag of this
/// form's lanes on enter/leave and repaints the affected strip. Arm
/// leave-tracking once so the tint clears when the cursor exits the form.
unsafe fn update_lane_hover(form: HWND, x: i32, y: i32) {
    unsafe {
        let mut entered_any = false;
        let mut changed = Vec::new();
        {
            let mut lanes = crate::runtime::lock(&LANES);
            for (list, _, hovered) in lanes.iter_mut() {
                let list_hwnd = HWND(*list as *mut _);
                if !IsWindow(Some(list_hwnd)).as_bool() || GetParent(list_hwnd) != Ok(form) {
                    continue;
                }
                let rect = lane_rect(list_hwnd, form);
                let inside = x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
                let (total, page, _) = metrics(list_hwnd);
                let inside = inside && total > page && page > 0;
                if inside {
                    entered_any = true;
                }
                if *hovered != inside {
                    *hovered = inside;
                    changed.push(list_hwnd);
                }
            }
        }
        // Lock released: repaint outside the registry (RedrawWindow can
        // re-enter paint_lane, which locks it again).
        for list in changed {
            repaint_lane(list);
        }
        if entered_any {
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: form,
                ..Default::default()
            };
            let _ = TrackMouseEvent(&mut tracking);
        }
    }
}

/// Clear the hover tint of every lane on this form (cursor left the form).
unsafe fn clear_lane_hover(form: HWND) {
    unsafe {
        let mut changed = Vec::new();
        {
            let mut lanes = crate::runtime::lock(&LANES);
            for (list, _, hovered) in lanes.iter_mut() {
                if !*hovered {
                    continue;
                }
                let list_hwnd = HWND(*list as *mut _);
                if IsWindow(Some(list_hwnd)).as_bool() && GetParent(list_hwnd) == Ok(form) {
                    *hovered = false;
                    changed.push(list_hwnd);
                }
            }
        }
        for list in changed {
            repaint_lane(list);
        }
    }
}

pub fn install(list: HWND) {
    unsafe {
        if GetWindowSubclass(list, Some(list_proc), SUBCLASS, None).as_bool() {
            return;
        }
        // LISTBOXES get the painted lane (see `Lane` above); ListViews and
        // top-level viewport forms keep the overlay bar window below - the
        // comctl listview and form scrollbars never showed the flicker.
        let painted = is_listbox(list);
        // Top-level "lists" (forms scrolled as viewports) must host the bar
        // on themselves: their GetParent is an OWNER, not a host surface.
        let host = if GetWindowLongW(list, GWL_STYLE) as u32 & WS_CHILD.0 != 0 {
            GetParent(list).unwrap_or(list)
        } else {
            list
        };
        let bar = if painted {
            HWND::default()
        } else {
            match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_CHILD | WS_CLIPSIBLINGS | WINDOW_STYLE(0x0100), // SS_NOTIFY
                0,
                0,
                1,
                1,
                Some(host),
                None,
                None,
                None,
            ) {
                Ok(bar) => bar,
                Err(_) => return,
            }
        };
        if !painted && host != list {
            // These forms stack freshly created children at the BOTTOM of
            // the sibling z-order, which would bury the bar under the very
            // list it overlays. Raise it above the stack once - but ONLY
            // for bars riding a child list's parent. A top-level viewport
            // form's own bar stays buried by design: raising it put a
            // phantom strip over the form's right-edge controls (the panel
            // audit caught it covering the exit/settings buttons).
            let _ = SetWindowPos(
                bar,
                Some(HWND(std::ptr::null_mut())), // HWND_TOP
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        if !painted {
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
        }
        let state = Box::into_raw(Box::new(ListState {
            host,
            bar,
            syncing: Cell::new(false),
            wheel_delta: Cell::new(0),
            painted: Cell::new((0, 0, 0, false)),
            lane: Cell::new(0),
            redraw: Cell::new(true),
        }));
        if !SetWindowSubclass(list, Some(list_proc), SUBCLASS, state as usize).as_bool() {
            drop(Box::from_raw(state));
            if !bar.is_invalid() {
                let _ = DestroyWindow(bar);
            }
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
        let width = px(list, 14);
        // Child LISTBOXES get a DEDICATED LANE beside the list: the list is
        // narrowed so the painted scrollbar strip never overlaps its client
        // at all. ListViews and top-level viewport forms keep the overlay
        // bar window (a comctl listview regrows its native scrollbar when
        // resized, and a form cannot be narrowed).
        let painted = state.bar.is_invalid();
        if painted {
            let mut client = RECT::default();
            let _ = GetClientRect(list, &mut client);
            let gap = px(list, 3);
            let target = client.right - width - gap;
            // A width we did not set ourselves is a fresh layout by the
            // host (creation, DPI change): reserve the lane beside it.
            // Comparing against the width WE set (not the recomputed
            // target) is what keeps the shrink from collapsing on itself.
            if target >= 1 && client.right != state.lane.get() {
                state.lane.set(target);
                let mut frame = RECT::default();
                if GetWindowRect(list, &mut frame).is_ok() {
                    let mut origin = windows::Win32::Foundation::POINT {
                        x: frame.left,
                        y: frame.top,
                    };
                    let _ = ScreenToClient(state.host, &mut origin);
                    let _ = SetWindowPos(
                        list,
                        None,
                        origin.x,
                        origin.y,
                        target,
                        frame.bottom - frame.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            // Stretch the row height within ~1px of the design rhythm so
            // WHOLE rows exactly fill the client - a fractional last row
            // reads as an empty line once scrolled to the bottom. Applied
            // here via LB_SETITEMHEIGHT (sizes have settled); WM_MEASUREITEM
            // alone fires too early, while the client rect is still empty.
            let design = px(list, 30);
            if client.bottom >= design {
                let rows = ((client.bottom as f64) / design as f64).round().max(1.0) as i32;
                let fill = (client.bottom / rows).clamp(design - 4, design + 4);
                let current = SendMessageW(list, LB_GETITEMHEIGHT, Some(WPARAM(0)), None).0 as i32;
                if fill != current && fill >= 1 {
                    let _ = SendMessageW(
                        list,
                        LB_SETITEMHEIGHT,
                        Some(WPARAM(0)),
                        Some(LPARAM(fill as isize)),
                    );
                }
            }
        }
        if !painted {
            let mut client = RECT::default();
            let _ = GetClientRect(list, &mut client);
            let header = list_header(list);
            let mut bounds = RECT::default();
            let _ = GetWindowRect(header, &mut bounds);
            let height = bounds.bottom - bounds.top;
            // Desired bar rect in the host's coordinate space (the bar is a
            // sibling of the list, not a child). Reposition only when the
            // target moved (resize/DPI/theme): scrolling syncs several times
            // per drag step and a same-rect SetWindowPos is pure repaint
            // churn on the bar.
            let mut corners = [
                windows::Win32::Foundation::POINT {
                    x: client.right - width,
                    y: height,
                },
                windows::Win32::Foundation::POINT {
                    x: client.right,
                    y: height + (client.bottom - height).max(1),
                },
            ];
            let _ = MapWindowPoints(Some(list), Some(state.host), &mut corners);
            let mut screen = corners;
            let _ = MapWindowPoints(Some(state.host), None, &mut screen);
            let mut current = RECT::default();
            let _ = GetWindowRect(state.bar, &mut current);
            if current.left != screen[0].x
                || current.top != screen[0].y
                || current.right != screen[1].x
                || current.bottom != screen[1].y
            {
                let _ = SetWindowPos(
                    state.bar,
                    None,
                    corners[0].x,
                    corners[0].y,
                    corners[1].x - corners[0].x,
                    corners[1].y - corners[0].y,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
        }
        let (total, page, position) = metrics(list);
        let shown = total > page && page > 0;
        // Repaint the scrollbar SYNCHRONOUSLY, and only when something
        // actually moved: a merely invalidated surface waits for the idle
        // WM_PAINT that fast input bursts (thumb drags) starve, and the
        // stale pixels then survive across composed frames. For painted
        // lanes the repaint is a scoped card invalidation; for overlay bars
        // it is a full bar repaint. Both complete inside this dispatch.
        let stamp = (total, page, position, shown);
        // While the host rebuilds the list behind WM_SETREDRAW(0) (filter
        // passes, reloads), intermediate metrics would paint the scrollbar
        // mid-rebuild - the thumb visibly jumping during loads. Record the
        // stamp but skip the repaint; unfreezing repaints the final state.
        if state.painted.replace(stamp) != stamp && state.redraw.get() {
            if painted {
                if let Some(card) = lane_card(list) {
                    let mut size = RECT::default();
                    if GetClientRect(card, &mut size).is_ok() {
                        let strip = lane_strip_on_card(list, card, size.right, size.bottom);
                        if strip.right > strip.left && strip.bottom > strip.top {
                            let _ = RedrawWindow(
                                Some(card),
                                Some(&strip),
                                None,
                                REDRAW_WINDOW_FLAGS(RDW_INVALIDATE.0 | RDW_UPDATENOW.0),
                            );
                        }
                    }
                }
            } else {
                let _ = ShowWindow(state.bar, if shown { SW_SHOWNA } else { SW_HIDE });
                let _ = RedrawWindow(
                    Some(state.bar),
                    None,
                    None,
                    REDRAW_WINDOW_FLAGS(RDW_INVALIDATE.0 | RDW_UPDATENOW.0),
                );
            }
        }
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
            // Paint the exposed strip inside this dispatch. Thumb drags
            // stream mouse messages continuously, which starves the list's
            // WM_PAINT - without this the freshly exposed rows show the
            // smeared pre-scroll pixels across composed frames.
            let _ = UpdateWindow(list);
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

/// Thumb-drag position mapping shared by the choice flyout, the lane strip,
/// and the overlay bar: the pointer offset (already minus the grab offset)
/// within the track becomes a scroll position, clamped so a drag past either
/// end stops at that end. `max` is the last scrollable position (total-page).
pub fn drag_position(pointer: i32, track_top: i32, travel: i32, max: i32) -> i32 {
    (((pointer - track_top).clamp(0, travel) as i64 * max.max(0) as i64) / travel as i64) as i32
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
    // The track merges into the list face (surface) at rest - a tinted
    // rail read as a mismatched background stripe next to the list.
    let track_color = if hovered { p.hover_surface } else { p.surface };
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
                    if travel > 0 {
                        let (total, page, _) = metrics(state.list);
                        scroll_to(
                            state.list,
                            drag_position(y - offset, track.top, travel, total - page),
                        );
                    }
                }
            }
            WM_LBUTTONUP | WM_CANCELMODE => {
                state.drag.set(None);
                // Release with the cursor wherever it is: clear hover and
                // let the next move retint if the pointer is still over.
                state.hover.set(false);
                let _ = ReleaseCapture();
            }
            WM_CAPTURECHANGED => state.drag.set(None),
            // Real drags wander in and out of the narrow strip constantly;
            // letting the track tint toggle on every crossing is the
            // flicker. Freeze hover while the drag owns the capture.
            WM_MOUSELEAVE => {
                if state.drag.get().is_none() {
                    state.hover.set(false);
                }
            }
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
        // ListViews grow a native WS_VSCROLL (arrow buttons and all) the
        // moment content overflows, even without the style at creation, and
        // ListBoxes re-assert one on internal recalcs. The lists are created
        // without it and every re-add is stripped before it lands: the family
        // bar below is the only scrollbar they ever show, so the native one
        // can never flash through mid-scroll or stack as a second bar.
        if msg == WM_STYLECHANGING && wp.0 == GWL_STYLE.0 as usize && lp.0 != 0 {
            let styles = &mut *(lp.0 as *mut STYLESTRUCT);
            if (styles.styleOld & WS_VSCROLL.0) == 0 && (styles.styleNew & WS_VSCROLL.0) != 0 {
                styles.styleNew &= !WS_VSCROLL.0;
            }
        }
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
        if msg == WM_SETREDRAW {
            state.redraw.set(wp.0 != 0);
        }
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        if msg == WM_SETREDRAW && wp.0 != 0 {
            // Unfreeze: force the scrollbar to its final state in one shot.
            state.painted.set((i32::MIN, i32::MIN, i32::MIN, false));
        }
        if matches!(
            msg,
            WM_SIZE
                | WM_PAINT
                | WM_VSCROLL
                | WM_MOUSEWHEEL
                | WM_KEYDOWN
                | WM_THEMECHANGED
                | WM_SETREDRAW
                | WM_SETFONT
                | WM_STYLECHANGED
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
            // The bar rides the parent as a sibling overlay above the list.
            let mut bar = GetWindow(parent, GW_CHILD).unwrap_or_default();
            while !bar.is_invalid() && !is_scrollbar(bar) {
                bar = GetWindow(bar, GW_HWNDNEXT).unwrap_or_default();
            }
            assert!(is_scrollbar(bar));
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
            let card = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                400,
                400,
                Some(parent),
                None,
                None,
                None,
            )
            .unwrap();
            let list = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("LISTBOX"),
                w!(""),
                WS_CHILD | WS_VISIBLE,
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
            set_lane_card(list, card);
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
            // Painted-lane input: press the thumb (resting at the track top),
            // drag to the far end, release - all through form-level messages.
            scroll_to(list, 0);
            let (count, page, _) = metrics(list);
            let narrowed = 300 - 21 - 4; // width minus bar+gap reserved by sync
            let at = |x: i32, y: i32| {
                LPARAM(((x as u16 as usize) | ((y as u16 as usize) << 16)) as isize)
            };
            assert!(lane_pointer(
                parent,
                WM_LBUTTONDOWN,
                WPARAM(1),
                at(narrowed + 4 + 10, 12)
            ));
            assert!(lane_pointer(
                parent,
                WM_MOUSEMOVE,
                WPARAM(1),
                at(narrowed + 4 + 10, 170)
            ));
            let (_, _, top) = metrics(list);
            assert_eq!(top, count - page);
            assert_eq!(SendMessageW(list, LB_GETCURSEL, None, None).0, 50);
            assert!(lane_pointer(
                parent,
                WM_LBUTTONUP,
                WPARAM(0),
                at(narrowed + 4 + 10, 170)
            ));
            assert!(!lane_pointer(
                parent,
                WM_LBUTTONUP,
                WPARAM(0),
                at(narrowed + 4 + 10, 170)
            ));
            // Track click page jump: this path scrolls from inside the
            // press handler, which once deadlocked on the lane registry
            // (scroll_to's sync re-locks it). A hang here fails the run.
            scroll_to(list, 0);
            let (_, page, _) = metrics(list);
            assert!(lane_pointer(
                parent,
                WM_LBUTTONDOWN,
                WPARAM(1),
                at(narrowed + 4 + 10, 100)
            ));
            let (_, _, top) = metrics(list);
            assert_eq!(top, page);
            // A track click never arms the drag, so its button-up is not a
            // lane interaction (it falls through to DefWindowProc, a no-op).
            assert!(!lane_pointer(
                parent,
                WM_LBUTTONUP,
                WPARAM(0),
                at(narrowed + 4 + 10, 100)
            ));
            // The NCHITTEST arm is deterministic now that it no longer
            // probes the screen: map a point through the popup's screen
            // origin and check lane vs list areas.
            let mut origin = windows::Win32::Foundation::POINT { x: 0, y: 0 };
            let _ = ClientToScreen(parent, &mut origin);
            let screen_at = |x: i32, y: i32| {
                LPARAM(
                    ((((origin.y + y) as u16 as usize) << 16) | ((origin.x + x) as u16 as usize))
                        as isize,
                )
            };
            assert!(lane_hit(parent, screen_at(narrowed + 4 + 10, 12)));
            assert!(!lane_hit(parent, screen_at(50, 12)));
            DestroyWindow(parent).unwrap();
        }
    }
}
