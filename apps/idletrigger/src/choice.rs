//! Go nativeform choicepopup parity: owner-drawn choice buttons that open a
//! custom drop-down popup (rounded card rows with hover/press/selected
//! states) instead of a native combobox. Selection changes notify the owner
//! form via WM_COMMAND with CBN_SELCHANGE so existing handlers keep working.
//! Rows can be plain options, danger options, or non-selectable group
//! headers (Go ChoicePopupItem.Header); lists longer than the visible window
//! scroll with the wheel and arrow keys like the Go popup.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT};
use windows::Win32::UI::Input::KeyboardAndMouse::{SetCapture, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use crate::paint;
use crate::theme;

// Go popup metrics: row 34, gap 1, inset 4, radius 6, max 6 visible rows.
const ROW_H: i32 = 34;
const ROW_GAP: i32 = 1;
const INSET: i32 = 4;
const RADIUS: i32 = 6;
const MAX_VISIBLE: usize = 6;

/// One popup row. Headers are non-selectable group labels (Go header items
/// carry Value -1 and never match the selection).
#[derive(Clone)]
pub struct ChoiceItem {
    pub value: String,
    pub label: String,
    pub danger: bool,
    pub header: bool,
}

impl ChoiceItem {
    pub fn option(value: &str, label: &str) -> ChoiceItem {
        ChoiceItem {
            value: value.to_string(),
            label: label.to_string(),
            danger: false,
            header: false,
        }
    }

    #[allow(dead_code)]
    pub fn danger(value: &str, label: &str) -> ChoiceItem {
        ChoiceItem {
            value: value.to_string(),
            label: label.to_string(),
            danger: true,
            header: false,
        }
    }

    pub fn header(label: &str) -> ChoiceItem {
        ChoiceItem {
            value: String::new(),
            label: label.to_string(),
            danger: false,
            header: true,
        }
    }
}

struct ChoiceData {
    items: Vec<ChoiceItem>,
    /// Index into the full row list (headers included).
    selected: i32,
    /// Popup window handle while open (0 closed).
    popup: isize,
    /// Hovered row inside the open popup (full-list index; -1 none).
    hover: i32,
    pressed: i32,
    /// First visible row while the popup scrolls.
    first: i32,
    /// Open the popup above the anchor (Go PreferAbove for quick actions).
    prefer_above: bool,
    update_caption: bool,
}

static CHOICES: Mutex<Option<HashMap<isize, ChoiceData>>> = Mutex::new(None);
static OPEN_BUTTON: AtomicIsize = AtomicIsize::new(0);
static OPEN_OWNER: AtomicIsize = AtomicIsize::new(0);
static OPEN_BUTTON_ID: AtomicI32 = AtomicI32::new(0);
static BAR_DRAG: AtomicI32 = AtomicI32::new(-1);
static BAR_HOVER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static WHEEL_DELTA: AtomicI32 = AtomicI32::new(0);

fn choices() -> std::sync::MutexGuard<'static, Option<HashMap<isize, ChoiceData>>> {
    CHOICES.lock().unwrap()
}

fn scale() -> i32 {
    crate::scale_pub(96).max(96)
}

/// Creates an owner-drawn choice button and registers its items.
pub fn create(
    parent: HWND,
    id: i32,
    b: (i32, i32, i32, i32),
    items: &[(String, String)],
    font: windows::Win32::Graphics::Gdi::HFONT,
) -> HWND {
    let rows: Vec<ChoiceItem> = items
        .iter()
        .map(|(v, l)| ChoiceItem::option(v, l))
        .collect();
    create_rows(parent, id, b, &rows, font)
}

/// Choice creation with per-row danger flags.
#[allow(dead_code)]
pub fn create_danger(
    parent: HWND,
    id: i32,
    b: (i32, i32, i32, i32),
    items: &[(String, String, bool)],
    font: windows::Win32::Graphics::Gdi::HFONT,
) -> HWND {
    let rows: Vec<ChoiceItem> = items
        .iter()
        .map(|(v, l, d)| ChoiceItem {
            value: v.clone(),
            label: l.clone(),
            danger: *d,
            header: false,
        })
        .collect();
    create_rows(parent, id, b, &rows, font)
}

/// Choice creation with the full row model (options, danger rows, headers).
pub fn create_rows(
    parent: HWND,
    id: i32,
    b: (i32, i32, i32, i32),
    rows: &[ChoiceItem],
    font: windows::Win32::Graphics::Gdi::HFONT,
) -> HWND {
    unsafe {
        let text = rows
            .iter()
            .find(|r| !r.header)
            .map(|r| r.label.as_str())
            .unwrap_or("");
        let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let (x, y, w, h) = b;
        let instance =
            windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("BUTTON"),
            PCWSTR(wide.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            crate::scale_pub(x),
            crate::scale_pub(y),
            crate::scale_pub(w),
            crate::scale_pub(h),
            Some(parent),
            Some(HMENU(id as isize as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("choice button");
        let _ = SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        crate::nativeform::track(hwnd);
        set_rows(hwnd, rows);
        hwnd
    }
}

/// Replaces the item list (Go fill + select flows).
#[allow(dead_code)]
pub fn set_items(button: HWND, items: &[(String, String)]) {
    let rows: Vec<ChoiceItem> = items
        .iter()
        .map(|(v, l)| ChoiceItem::option(v, l))
        .collect();
    set_rows(button, &rows);
}

/// Item list with per-row danger styling (Go quick-actions menu).
pub fn set_items_danger(button: HWND, items: &[(String, String, bool)]) {
    let mut caption = [0u16; 256];
    unsafe {
        GetWindowTextW(button, &mut caption);
    }
    let rows: Vec<ChoiceItem> = items
        .iter()
        .map(|(v, l, d)| ChoiceItem {
            value: v.clone(),
            label: l.clone(),
            danger: *d,
            header: false,
        })
        .collect();
    set_rows(button, &rows);
    if let Some(data) = choices()
        .as_mut()
        .and_then(|map| map.get_mut(&(button.0 as isize)))
    {
        data.update_caption = false;
    }
    unsafe {
        let _ = SetWindowTextW(button, PCWSTR(caption.as_ptr()));
    }
}

/// Full row model replacement (options + danger + headers).
pub fn set_rows(button: HWND, rows: &[ChoiceItem]) {
    unsafe {
        if !windows::Win32::UI::Shell::SetWindowSubclass(button, Some(button_proc), 0x49544348, 0)
            .as_bool()
        {
            return;
        }
    }
    if OPEN_BUTTON.load(Ordering::SeqCst) == button.0 as isize {
        close(false);
    }
    let first_option = rows.iter().position(|r| !r.header).unwrap_or(0) as i32;
    choices().get_or_insert_with(Default::default).insert(
        button.0 as isize,
        ChoiceData {
            items: rows.to_vec(),
            selected: first_option,
            popup: 0,
            hover: -1,
            pressed: -1,
            first: 0,
            prefer_above: false,
            update_caption: true,
        },
    );
    select_index(button, first_option);
}

unsafe extern "system" fn button_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_KEYDOWN && wparam.0 == 0x73 {
        unsafe {
            toggle(
                hwnd,
                GetParent(hwnd).unwrap_or_default(),
                GetDlgCtrlID(hwnd),
            );
        }
        return LRESULT(0);
    }
    if msg == WM_NCDESTROY {
        if OPEN_BUTTON.load(Ordering::SeqCst) == hwnd.0 as isize {
            close(false);
        }
        if let Some(map) = choices().as_mut() {
            map.remove(&(hwnd.0 as isize));
        }
        unsafe {
            let _ = windows::Win32::UI::Shell::RemoveWindowSubclass(
                hwnd,
                Some(button_proc),
                0x49544348,
            );
        }
    }
    unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Sets the popup open direction for the button's next open.
pub fn set_prefer_above(button: HWND, prefer: bool) {
    if let Some(data) = choices()
        .get_or_insert_with(Default::default)
        .get_mut(&(button.0 as isize))
    {
        data.prefer_above = prefer;
    }
}

/// Whether a control is a registered choice button.
pub fn is_choice(button: HWND) -> bool {
    choices()
        .as_ref()
        .is_some_and(|m| m.contains_key(&(button.0 as isize)))
}

/// Current selection index into the full row list (first option when none).
pub fn selection(button: HWND) -> i32 {
    choices()
        .as_ref()
        .and_then(|m| m.get(&(button.0 as isize)))
        .map(|d| d.selected)
        .unwrap_or(0)
}

/// Value of the current selection.
pub fn value(button: HWND) -> String {
    choices()
        .as_ref()
        .and_then(|m| m.get(&(button.0 as isize)))
        .and_then(|d| d.items.get(d.selected.max(0) as usize))
        .map(|item| item.value.clone())
        .unwrap_or_default()
}

/// Selects by value; returns the index (None when the value is absent).
pub fn select_value(button: HWND, value: &str) -> Option<i32> {
    let index = choices()
        .as_ref()
        .and_then(|m| m.get(&(button.0 as isize)))?
        .items
        .iter()
        .position(|item| !item.header && item.value == value)? as i32;
    select_index(button, index);
    Some(index)
}

/// Selects by index and repaints the button.
pub fn select_index(button: HWND, index: i32) {
    let mut label = String::new();
    let mut update_caption = false;
    if let Some(data) = choices()
        .get_or_insert_with(Default::default)
        .get_mut(&(button.0 as isize))
    {
        data.selected = index;
        update_caption = data.update_caption;
        label = data
            .items
            .get(index as usize)
            .map(|item| item.label.clone())
            .unwrap_or_default();
    }
    let wide: Vec<u16> = label.encode_utf16().chain([0]).collect();
    if update_caption {
        unsafe {
            let _ = SetWindowTextW(button, PCWSTR(wide.as_ptr()));
        }
    }
    repaint(button);
}

fn repaint(button: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(button), None, false);
    }
}

/// Draws a choice button's closed state (parent WM_DRAWITEM dispatch).
pub fn draw_button(
    button: HWND,
    dc: windows::Win32::Graphics::Gdi::HDC,
    bounds: &RECT,
    state: paint::ControlState,
) {
    let p = theme::palette();
    let label = choices()
        .as_ref()
        .and_then(|m| m.get(&(button.0 as isize)))
        .and_then(|d| d.items.get(d.selected.max(0) as usize))
        .map(|item| item.label.clone())
        .unwrap_or_default();
    let mut state = state;
    state.open = OPEN_BUTTON.load(Ordering::SeqCst) == button.0 as isize;
    let font = font_for(button);
    paint::draw_choice(
        dc,
        bounds,
        font,
        &label,
        p,
        p.window_bg,
        state,
        RADIUS,
        scale(),
    );
}

/// Cached form font for choice painting (the button's own WM_SETFONT font).
fn font_for(button: HWND) -> windows::Win32::Graphics::Gdi::HFONT {
    unsafe {
        let font = SendMessageW(button, WM_GETFONT, Some(WPARAM(0)), Some(LPARAM(0))).0;
        windows::Win32::Graphics::Gdi::HFONT(font as *mut _)
    }
}

/// Toggles the popup for a clicked choice button (owner form calls this from
/// its BN_CLICKED handler).
pub fn toggle(button: HWND, owner: HWND, id: i32) {
    if OPEN_BUTTON.load(Ordering::SeqCst) == button.0 as isize {
        close(false);
        return;
    }
    close(false);
    open(button, owner, id);
}

fn visible_rows(count: usize) -> i32 {
    count.min(MAX_VISIBLE) as i32
}

fn open(button: HWND, owner: HWND, id: i32) {
    let _dpi = crate::dpi::Scope::window(button);
    let (count, selected) = match choices().as_ref().and_then(|m| m.get(&(button.0 as isize))) {
        Some(data) => (data.items.len(), data.selected),
        None => return,
    };
    if count == 0 {
        return;
    }
    unsafe {
        register_class();
        let instance =
            windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        let popup = CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0),
            windows::core::w!("IdleTriggerChoicePopup"),
            windows::core::w!(""),
            WINDOW_STYLE(WS_POPUP.0),
            0,
            0,
            1,
            1,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        )
        .expect("choice popup");
        let work = crate::display::work_area_for(owner);
        let visible = visible_rows(count).min(row_count(&RECT {
            bottom: work.bottom - work.top,
            ..Default::default()
        }));
        let height = 2 * crate::scale_pub(INSET)
            + visible * crate::scale_pub(ROW_H)
            + (visible - 1) * crate::scale_pub(ROW_GAP);
        let mut anchor = RECT::default();
        let _ = GetWindowRect(button, &mut anchor);
        let width = (anchor.right - anchor.left).min(work.right - work.left);
        // Below with a 1px gap; flip above when it would overflow.
        let prefer_above = choices()
            .as_ref()
            .and_then(|m| m.get(&(button.0 as isize)))
            .is_some_and(|d| d.prefer_above);
        let mut x = anchor.left.max(work.left);
        let mut y = if prefer_above {
            anchor.top - height - crate::scale_pub(1)
        } else {
            anchor.bottom + crate::scale_pub(1)
        };
        if y + height > work.bottom {
            y = anchor.top - height - crate::scale_pub(1);
        }
        if x + width > work.right {
            x = work.right - width;
        }
        if y < work.top {
            y = work.top;
        }
        if let Some(data) = choices()
            .as_mut()
            .and_then(|m| m.get_mut(&(button.0 as isize)))
        {
            data.first = (selected - visible / 2).clamp(0, (count as i32 - visible).max(0));
            data.popup = popup.0 as isize;
            data.hover = selected;
            data.pressed = -1;
        }
        OPEN_BUTTON.store(button.0 as isize, Ordering::SeqCst);
        OPEN_OWNER.store(owner.0 as isize, Ordering::SeqCst);
        OPEN_BUTTON_ID.store(id, Ordering::SeqCst);
        BAR_DRAG.store(-1, Ordering::SeqCst);
        BAR_HOVER.store(false, Ordering::SeqCst);
        WHEEL_DELTA.store(0, Ordering::SeqCst);
        theme::apply_to_window(popup);
        let _ = SetWindowPos(
            popup,
            Some(HWND_TOPMOST),
            x,
            y,
            width,
            height,
            SWP_SHOWWINDOW,
        );
        // Own all mouse input while open so a click ANYWHERE outside commits
        // the dismissal (Go popup message filter behavior).
        let _ = SetFocus(Some(popup));
        let _ = SetCapture(popup);
        repaint(button);
    }
}

/// Closes any open popup; notifies the owner when a selection was committed.
// Guard against re-entry: WM_KILLFOCUS fires during DestroyWindow and
// would call close() again, attempting to destroy the same popup twice.
static CLOSING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn close(notify: bool) {
    if CLOSING.swap(true, Ordering::SeqCst) {
        return; // Already closing — prevent re-entrant DestroyWindow.
    }
    let button = OPEN_BUTTON.swap(0, Ordering::SeqCst);
    let owner = OPEN_OWNER.swap(0, Ordering::SeqCst);
    let id = OPEN_BUTTON_ID.swap(0, Ordering::SeqCst);
    if button == 0 {
        CLOSING.store(false, Ordering::SeqCst);
        return;
    }
    let popup = choices()
        .get_or_insert_with(Default::default)
        .get_mut(&button)
        .map(|data| {
            let handle = data.popup;
            data.popup = 0;
            data.hover = -1;
            data.first = 0;
            handle
        })
        .unwrap_or(0);
    unsafe {
        if popup != 0 {
            let restore_focus =
                windows::Win32::UI::Input::KeyboardAndMouse::GetFocus().0 as isize == popup;
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
            // WM_DESTROY detaches the registry entry; the OPEN_* slots are
            // already cleared, so the detach is a no-op there.
            let _ = DestroyWindow(HWND(popup as *mut _));
            if restore_focus && IsWindow(Some(HWND(button as *mut _))).as_bool() {
                let _ = SetFocus(Some(HWND(button as *mut _)));
            }
        }
        let _ = InvalidateRect(Some(HWND(button as *mut _)), None, false);
        if notify {
            let _ = PostMessageW(
                Some(HWND(owner as *mut _)),
                WM_COMMAND,
                WPARAM(((id as usize) & 0xFFFF) | ((CBN_SELCHANGE as usize) << 16)),
                LPARAM(button),
            );
        }
    }
    CLOSING.store(false, Ordering::SeqCst);
}

/// Registry-only cleanup from the popup's own WM_DESTROY.
fn detach_popup(popup: HWND) {
    let button = OPEN_BUTTON.swap(0, Ordering::SeqCst);
    OPEN_OWNER.store(0, Ordering::SeqCst);
    OPEN_BUTTON_ID.store(0, Ordering::SeqCst);
    if button != 0 {
        if let Some(data) = choices()
            .get_or_insert_with(Default::default)
            .get_mut(&button)
            && data.popup == popup.0 as isize
        {
            data.popup = 0;
            data.hover = -1;
            data.first = 0;
        }
        unsafe {
            let _ = InvalidateRect(Some(HWND(button as *mut _)), None, false);
        }
    }
}

/// The popup window currently open, if any.
#[allow(dead_code)]
pub fn open_popup() -> HWND {
    HWND(
        choices()
            .as_ref()
            .and_then(|m| m.values().find(|d| d.popup != 0))
            .map(|d| d.popup as *mut _)
            .unwrap_or(std::ptr::null_mut()),
    )
}

fn register_class() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| unsafe {
        let instance =
            windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(popup_proc),
            hInstance: instance.into(),
            lpszClassName: windows::core::w!("IdleTriggerChoicePopup"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: theme::bg_brush(),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);
    });
}

fn row_rect(client: &RECT, index: i32) -> RECT {
    let inset = crate::scale_pub(INSET);
    let row = crate::scale_pub(ROW_H);
    let gap = crate::scale_pub(ROW_GAP);
    let top = client.top + inset + index * (row + gap);
    RECT {
        left: client.left + inset,
        top,
        right: client.right - inset,
        bottom: top + row,
    }
}

/// Renders the popup rows into a memory DC and blits once.
unsafe fn paint_popup_buffered(hwnd: HWND, hdc: windows::Win32::Graphics::Gdi::HDC) -> bool {
    unsafe {
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        crate::nativeform::draw_buffered(hdc, &client, |mem, local| {
            paint_popup(mem, local);
        })
    }
}

/// Direct popup row painting (shared by the buffered path).
unsafe fn paint_popup(hdc: windows::Win32::Graphics::Gdi::HDC, client: &RECT) {
    let p = theme::palette();
    // Card background fills the whole popup (Go popup surface).
    paint::fill_rect(hdc, client, p.elevated);
    let button = OPEN_BUTTON.load(Ordering::SeqCst);
    let (items, selected, hover, first, pressed) =
        match choices().as_ref().and_then(|m| m.get(&button)) {
            Some(data) => (
                data.items.clone(),
                data.selected,
                data.hover,
                data.first,
                data.pressed,
            ),
            None => (Vec::new(), 0, -1, 0, -1),
        };
    let font = font_for(HWND(button as *mut _));
    let capacity = row_count(client);
    let end = (first + capacity).min(items.len() as i32);
    for index in first..end {
        let mut bounds = row_rect(client, index - first);
        if items.len() as i32 > capacity {
            bounds.right -= crate::scale_pub(14);
        }
        let item = &items[index as usize];
        if item.header {
            draw_popup_header(hdc, &bounds, font, &item.label, p);
            continue;
        }
        let state = paint::ControlState {
            hovered: index == hover,
            pressed: index == pressed && index == hover,
            ..Default::default()
        };
        let selected_font = crate::automation_ui::section_font_cached();
        paint::draw_menu_option(
            hdc,
            &bounds,
            font,
            selected_font,
            &item.label,
            p,
            p.elevated,
            state,
            index == selected,
            item.danger,
            RADIUS,
            scale(),
        );
    }
    if let Some((track, thumb, _, _)) = scroll_geometry(client) {
        crate::list_style::draw_scrollbar(
            hdc,
            &track,
            &thumb,
            crate::scale_pub(4),
            BAR_HOVER.load(Ordering::SeqCst),
            BAR_DRAG.load(Ordering::SeqCst) >= 0,
        );
    }
}

fn scroll_geometry(client: &RECT) -> Option<(RECT, RECT, i32, i32)> {
    let button = OPEN_BUTTON.load(Ordering::SeqCst);
    let (count, first) = choices()
        .as_ref()?
        .get(&button)
        .map(|d| (d.items.len() as i32, d.first))?;
    let page = row_count(client);
    if count <= page {
        return None;
    }
    let track = RECT {
        left: client.right - crate::scale_pub(14),
        right: client.right - crate::scale_pub(4),
        top: crate::scale_pub(INSET),
        bottom: client.bottom - crate::scale_pub(INSET),
    };
    let (top, bottom) = crate::list_style::thumb(
        count,
        page,
        first,
        track.bottom - track.top,
        crate::scale_pub(24),
    );
    Some((
        track,
        RECT {
            top: track.top + top,
            bottom: track.top + bottom,
            ..track
        },
        page,
        count - page,
    ))
}

unsafe fn scrollbar_pointer(hwnd: HWND, msg: u32, lp: LPARAM) -> bool {
    unsafe {
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let Some((track, thumb, page, max)) = scroll_geometry(&client) else {
            return false;
        };
        let x = lp.0 as i16 as i32;
        let y = (lp.0 >> 16) as i16 as i32;
        let hover = x >= track.left && x < client.right && y >= 0 && y < client.bottom;
        let drag = BAR_DRAG.load(Ordering::SeqCst);
        let changed = BAR_HOVER.swap(hover, Ordering::SeqCst) != hover;
        if changed {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        match msg {
            WM_LBUTTONDOWN if hover => {
                if y >= thumb.top && y < thumb.bottom {
                    BAR_DRAG.store(y - thumb.top, Ordering::SeqCst);
                } else {
                    scroll_rows(if y < thumb.top { -page } else { page });
                }
            }
            WM_MOUSEMOVE if drag >= 0 => {
                let travel = track.bottom - track.top - (thumb.bottom - thumb.top);
                if travel > 0 {
                    let next = ((y - drag - track.top).clamp(0, travel) as i64 * max as i64
                        / travel as i64) as i32;
                    let current = choices()
                        .as_ref()
                        .and_then(|m| m.get(&OPEN_BUTTON.load(Ordering::SeqCst)))
                        .map(|d| d.first)
                        .unwrap_or(0);
                    scroll_rows(next - current);
                }
            }
            WM_LBUTTONUP if drag >= 0 || hover => {
                BAR_DRAG.store(-1, Ordering::SeqCst);
            }
            WM_MOUSEMOVE if hover => {}
            _ => return false,
        }
        if let Some(data) = choices()
            .as_mut()
            .and_then(|map| map.get_mut(&OPEN_BUTTON.load(Ordering::SeqCst)))
        {
            data.hover = -1;
            if msg == WM_LBUTTONUP {
                data.pressed = -1;
            }
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
        true
    }
}

/// Go DrawPopupHeader: quiet secondary label with a left inset.
fn draw_popup_header(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    bounds: &RECT,
    font: windows::Win32::Graphics::Gdi::HFONT,
    label: &str,
    p: &crate::theme::Palette,
) {
    let scale = scale();
    paint::fill_rect(hdc, bounds, p.elevated);
    let mut text = label.encode_utf16().collect::<Vec<u16>>();
    let mut text_bounds = RECT {
        left: bounds.left + crate::paint::sp(10, scale),
        top: bounds.top,
        right: bounds.right - crate::paint::sp(6, scale),
        bottom: bounds.bottom,
    };
    unsafe {
        let old = windows::Win32::Graphics::Gdi::SelectObject(
            hdc,
            windows::Win32::Graphics::Gdi::HGDIOBJ(font.0),
        );
        windows::Win32::Graphics::Gdi::SetTextColor(
            hdc,
            windows::Win32::Foundation::COLORREF(p.text2),
        );
        windows::Win32::Graphics::Gdi::SetBkMode(hdc, windows::Win32::Graphics::Gdi::TRANSPARENT);
        let _ = windows::Win32::Graphics::Gdi::DrawTextW(
            hdc,
            &mut text,
            &mut text_bounds,
            windows::Win32::Graphics::Gdi::DT_LEFT
                | windows::Win32::Graphics::Gdi::DT_VCENTER
                | windows::Win32::Graphics::Gdi::DT_SINGLELINE
                | windows::Win32::Graphics::Gdi::DT_NOPREFIX
                | windows::Win32::Graphics::Gdi::DT_END_ELLIPSIS,
        );
        windows::Win32::Graphics::Gdi::SelectObject(hdc, old);
    }
}

fn row_count(client: &RECT) -> i32 {
    let usable =
        client.bottom - client.top - 2 * crate::scale_pub(INSET) + crate::scale_pub(ROW_GAP);
    (usable / (crate::scale_pub(ROW_H) + crate::scale_pub(ROW_GAP))).max(1)
}

/// Maps a client Y to a full-list row index, or -1 (gaps and headers).
unsafe fn row_at(client: &RECT, y: i32) -> i32 {
    let button = OPEN_BUTTON.load(Ordering::SeqCst);
    let (count, first) = match choices().as_ref().and_then(|m| m.get(&button)) {
        Some(data) => (data.items.len() as i32, data.first),
        None => return -1,
    };
    let inset = crate::scale_pub(INSET);
    if y < client.top + inset || y >= client.bottom - inset {
        return -1;
    }
    let stride = crate::scale_pub(ROW_H) + crate::scale_pub(ROW_GAP);
    if stride <= 0 {
        return -1;
    }
    let offset = y - client.top - inset;
    if offset % stride >= crate::scale_pub(ROW_H) {
        return -1; // Inter-row gap.
    }
    let row = first + offset / stride;
    if row < first || row >= count {
        return -1;
    }
    if choices()
        .as_ref()
        .and_then(|m| m.get(&button))
        .is_some_and(|d| d.items[row as usize].header)
    {
        return -1;
    }
    row
}

/// Scrolls the open popup by whole rows (mouse wheel, Go scroll(±1)).
fn scroll_rows(delta: i32) {
    let button = OPEN_BUTTON.load(Ordering::SeqCst);
    if button == 0 {
        return;
    }
    if let Some(data) = choices()
        .get_or_insert_with(Default::default)
        .get_mut(&button)
    {
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(HWND(data.popup as *mut _), &mut client);
        }
        let visible = row_count(&client);
        let max_first = (data.items.len() as i32 - visible).max(0);
        let next = (data.first + delta).clamp(0, max_first);
        if next != data.first {
            data.first = next;
            unsafe {
                if data.popup != 0 {
                    let _ = InvalidateRect(Some(HWND(data.popup as *mut _)), None, false);
                }
            }
        }
    }
}

/// Next selectable row from `start` moving by `delta` (Go nextSelectable).
fn next_selectable(start: i32, delta: i32) -> i32 {
    let button = OPEN_BUTTON.load(Ordering::SeqCst);
    let count = choices()
        .as_ref()
        .and_then(|m| m.get(&button))
        .map(|d| d.items.len() as i32)
        .unwrap_or(0);
    let mut index = start + delta;
    while index >= 0 && index < count {
        let selectable = choices()
            .as_ref()
            .and_then(|m| m.get(&button))
            .is_some_and(|d| !d.items[index as usize].header);
        if selectable {
            return index;
        }
        index += delta;
    }
    start
}

/// Moves hover/focus to a row and keeps it inside the scroll window.
fn move_focus(row: i32) {
    let button = OPEN_BUTTON.load(Ordering::SeqCst);
    if button == 0 {
        return;
    }
    if let Some(data) = choices()
        .get_or_insert_with(Default::default)
        .get_mut(&button)
    {
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(HWND(data.popup as *mut _), &mut client);
        }
        let visible = row_count(&client);
        data.hover = row;
        if row < data.first {
            data.first = row;
        } else if row >= data.first + visible {
            data.first = row - visible + 1;
        }
        unsafe {
            if data.popup != 0 {
                let _ = InvalidateRect(Some(HWND(data.popup as *mut _)), None, false);
            }
        }
    }
}

unsafe extern "system" fn popup_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let _dpi = crate::dpi::Scope::window(hwnd);
        if matches!(msg, WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP)
            && scrollbar_pointer(hwnd, msg, lparam)
        {
            return LRESULT(0);
        }
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                // Buffered row painting: hover repaints never flash the card
                // background (Go popup draws through its own presenter).
                if paint_popup_buffered(hwnd, hdc) {
                    let _ = EndPaint(hwnd, &ps);
                    return LRESULT(0);
                }
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                paint_popup(hdc, &client);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let point = POINT {
                    x: (lparam.0 & 0xFFFF) as i16 as i32,
                    y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                };
                let mut row = row_at(&client, point.y);
                if point.x < client.left || point.x >= client.right {
                    row = -1;
                }
                let button = OPEN_BUTTON.load(Ordering::SeqCst);
                if let Some(data) = choices()
                    .get_or_insert_with(Default::default)
                    .get_mut(&button)
                    && data.hover != row
                {
                    data.hover = row;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = WHEEL_DELTA.load(Ordering::SeqCst) + ((wparam.0 >> 16) as i16 as i32);
                WHEEL_DELTA.store(delta % 120, Ordering::SeqCst);
                scroll_rows(-(delta / 120));
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if windows::Win32::UI::Input::KeyboardAndMouse::GetCapture() != hwnd {
                    let _ = SetCapture(hwnd);
                }
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let point = POINT {
                    x: (lparam.0 & 0xFFFF) as i16 as i32,
                    y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                };
                let outside = point.x < client.left
                    || point.x >= client.right
                    || point.y < client.top
                    || point.y >= client.bottom;
                if outside {
                    close(false);
                } else {
                    let row = row_at(&client, point.y);
                    if let Some(data) = choices()
                        .as_mut()
                        .and_then(|m| m.get_mut(&OPEN_BUTTON.load(Ordering::SeqCst)))
                    {
                        data.pressed = row;
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let point = POINT {
                    x: (lparam.0 & 0xFFFF) as i16 as i32,
                    y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                };
                let mut chosen = if point.x < client.left || point.x >= client.right {
                    -1
                } else {
                    row_at(&client, point.y)
                };
                let pressed = choices()
                    .as_ref()
                    .and_then(|m| m.get(&OPEN_BUTTON.load(Ordering::SeqCst)))
                    .map(|d| d.pressed)
                    .unwrap_or(-1);
                if chosen != pressed {
                    chosen = -1;
                }
                if chosen >= 0 {
                    let button = OPEN_BUTTON.load(Ordering::SeqCst);
                    select_index(HWND(button as *mut _), chosen);
                }
                close(chosen >= 0);
                LRESULT(0)
            }
            WM_KEYDOWN => match wparam.0 {
                0x1B | 0x73 => {
                    // Escape closes without committing.
                    close(false);
                    LRESULT(0)
                }
                0x09 => {
                    let owner = HWND(OPEN_OWNER.load(Ordering::SeqCst) as *mut _);
                    let button = HWND(OPEN_BUTTON.load(Ordering::SeqCst) as *mut _);
                    close(false);
                    let previous =
                        windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x10) < 0;
                    if let Ok(next) = GetNextDlgTabItem(owner, Some(button), previous) {
                        let _ = SetFocus(Some(next));
                    }
                    LRESULT(0)
                }
                0x25 | 0x27 | 0x48 | 0x4B | 0x4D | 0x50 | 0x57 => LRESULT(0), // swallow
                0x26 => {
                    // Up: previous selectable row.
                    let hover = choices()
                        .as_ref()
                        .and_then(|m| m.get(&OPEN_BUTTON.load(Ordering::SeqCst)))
                        .map(|d| d.hover)
                        .unwrap_or(-1);
                    let next = next_selectable(hover.max(0), -1);
                    if next != hover {
                        move_focus(next);
                    }
                    LRESULT(0)
                }
                0x28 => {
                    // Down: next selectable row.
                    let hover = choices()
                        .as_ref()
                        .and_then(|m| m.get(&OPEN_BUTTON.load(Ordering::SeqCst)))
                        .map(|d| d.hover)
                        .unwrap_or(-1);
                    let next = next_selectable(hover.max(-1), 1);
                    if next != hover {
                        move_focus(next);
                    }
                    LRESULT(0)
                }
                0x24 => {
                    // Home.
                    let next = next_selectable(-1, 1);
                    move_focus(next);
                    LRESULT(0)
                }
                0x23 => {
                    // End.
                    let count = choices()
                        .as_ref()
                        .and_then(|m| m.get(&OPEN_BUTTON.load(Ordering::SeqCst)))
                        .map(|d| d.items.len() as i32)
                        .unwrap_or(0);
                    let next = next_selectable(count, -1);
                    move_focus(next);
                    LRESULT(0)
                }
                0x0D | 0x20 => {
                    // Enter/Space commits the hovered row.
                    let hover = choices()
                        .as_ref()
                        .and_then(|m| m.get(&OPEN_BUTTON.load(Ordering::SeqCst)))
                        .map(|d| d.hover)
                        .unwrap_or(-1);
                    if hover >= 0 {
                        let button = OPEN_BUTTON.load(Ordering::SeqCst);
                        select_index(HWND(button as *mut _), hover);
                        close(true);
                    } else {
                        close(false);
                    }
                    LRESULT(0)
                }
                _ => LRESULT(0),
            },
            WM_KILLFOCUS => {
                // An anchor click can transfer focus through the owner before
                // its deferred command arrives. Let that command toggle the popup.
                let next = HWND(wparam.0 as *mut _);
                let owner = HWND(OPEN_OWNER.load(Ordering::SeqCst) as *mut _);
                let anchor = HWND(OPEN_BUTTON.load(Ordering::SeqCst) as *mut _);
                if next.is_invalid()
                    || (next != owner && next != anchor && !IsChild(owner, next).as_bool())
                {
                    close(false);
                }
                LRESULT(0)
            }
            WM_CANCELMODE | WM_CAPTURECHANGED => {
                close(false);
                LRESULT(0)
            }
            WM_ERASEBKGND => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                FillRect(hdc, &client, theme::bg_brush());
                LRESULT(1)
            }
            WM_DESTROY => {
                detach_popup(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_pointer_cannot_select_hidden_rows_and_button_destruction_cleans_registry() {
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
            let rows: Vec<_> = (0..10)
                .map(|i| ChoiceItem::option(&i.to_string(), &format!("Item {i}")))
                .collect();
            let _dpi = crate::dpi::Scope::window(owner);
            let font = windows::Win32::Graphics::Gdi::HFONT(
                windows::Win32::Graphics::Gdi::GetStockObject(
                    windows::Win32::Graphics::Gdi::DEFAULT_GUI_FONT,
                )
                .0,
            );
            let button = create_rows(owner, 100, (0, 0, 200, 32), &rows, font);
            open(button, owner, 100);
            let popup = open_popup();
            for next in [owner, button] {
                SendMessageW(popup, WM_KILLFOCUS, Some(WPARAM(next.0 as usize)), None);
                assert_eq!(open_popup(), popup);
            }
            let mut client = RECT::default();
            GetClientRect(popup, &mut client).unwrap();
            let (track, thumb, _, max) = scroll_geometry(&client).unwrap();
            let point_at =
                |x: i32, y: i32| LPARAM(((y as u32) << 16 | (x as u32 & 0xffff)) as isize);
            SendMessageW(
                popup,
                WM_LBUTTONDOWN,
                Some(WPARAM(1)),
                Some(point_at(track.left + 1, thumb.top + 1)),
            );
            SendMessageW(
                popup,
                WM_MOUSEMOVE,
                Some(WPARAM(1)),
                Some(point_at(track.left + 1, client.bottom + 100)),
            );
            SendMessageW(
                popup,
                WM_LBUTTONUP,
                None,
                Some(point_at(track.left + 1, client.bottom + 100)),
            );
            assert_eq!(open_popup(), popup);
            assert_eq!(selection(button), 0);
            assert_eq!(
                choices()
                    .as_ref()
                    .unwrap()
                    .get(&(button.0 as isize))
                    .unwrap()
                    .first,
                max
            );
            scroll_rows(-max);
            assert_eq!(row_at(&client, client.bottom + crate::scale_pub(ROW_H)), -1);
            let point =
                LPARAM(((crate::scale_pub(INSET + ROW_H + ROW_GAP + 5) as isize) << 16) | 10);
            SendMessageW(popup, WM_LBUTTONDOWN, Some(WPARAM(1)), Some(point));
            SendMessageW(popup, WM_LBUTTONUP, Some(WPARAM(0)), Some(point));
            assert_eq!(selection(button), 1);
            let mut label = [0u16; 32];
            let len = GetWindowTextW(button, &mut label);
            assert_eq!(String::from_utf16_lossy(&label[..len as usize]), "Item 1");
            open(button, owner, 100);
            set_rows(button, &rows[..2]);
            assert!(open_popup().is_invalid());
            open(button, owner, 100);
            SendMessageW(open_popup(), WM_KILLFOCUS, Some(WPARAM(0)), None);
            assert!(open_popup().is_invalid());
            DestroyWindow(owner).unwrap();
            assert!(!is_choice(button));
        }
    }
}
