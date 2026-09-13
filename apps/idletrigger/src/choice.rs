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
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus};
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
    /// First visible row while the popup scrolls.
    first: i32,
    /// Open the popup above the anchor (Go PreferAbove for quick actions).
    prefer_above: bool,
}

static CHOICES: Mutex<Option<HashMap<isize, ChoiceData>>> = Mutex::new(None);
static OPEN_BUTTON: AtomicIsize = AtomicIsize::new(0);
static OPEN_OWNER: AtomicIsize = AtomicIsize::new(0);
static OPEN_BUTTON_ID: AtomicI32 = AtomicI32::new(0);

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
}

/// Full row model replacement (options + danger + headers).
pub fn set_rows(button: HWND, rows: &[ChoiceItem]) {
    let first_option = rows.iter().position(|r| !r.header).unwrap_or(0) as i32;
    choices().get_or_insert_with(Default::default).insert(
        button.0 as isize,
        ChoiceData {
            items: rows.to_vec(),
            selected: first_option,
            popup: 0,
            hover: -1,
            first: 0,
            prefer_above: false,
        },
    );
    repaint(button);
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
    if let Some(data) = choices()
        .get_or_insert_with(Default::default)
        .get_mut(&(button.0 as isize))
    {
        data.selected = index;
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
        let visible = visible_rows(count);
        let height = 2 * crate::scale_pub(INSET)
            + visible * crate::scale_pub(ROW_H)
            + (visible - 1) * crate::scale_pub(ROW_GAP);
        let mut anchor = RECT::default();
        let _ = GetWindowRect(button, &mut anchor);
        let work = crate::display::work_area_for(owner);
        let width = (anchor.right - anchor.left).min(work.right - work.left);
        // Below with a 1px gap; flip above when it would overflow.
        let prefer_above = choices()
            .as_ref()
            .and_then(|m| m.get(&(button.0 as isize)))
            .is_some_and(|d| d.prefer_above);
        let mut x = anchor.left;
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
        theme::apply_to_window(popup);
        if let Some(data) = choices()
            .get_or_insert_with(Default::default)
            .get_mut(&(button.0 as isize))
        {
            // Start the scroll window so the selected row is visible (Go
            // ensureVisible on open).
            let max_first = (count as i32 - visible).max(0);
            data.first = (selected - visible / 2).clamp(0, max_first);
            data.popup = popup.0 as isize;
            data.hover = selected;
        }
        OPEN_BUTTON.store(button.0 as isize, Ordering::SeqCst);
        OPEN_OWNER.store(owner.0 as isize, Ordering::SeqCst);
        OPEN_BUTTON_ID.store(id, Ordering::SeqCst);
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
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
            // WM_DESTROY detaches the registry entry; the OPEN_* slots are
            // already cleared, so the detach is a no-op there.
            let _ = DestroyWindow(HWND(popup as *mut _));
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
    let (items, selected, hover, first) = match choices().as_ref().and_then(|m| m.get(&button)) {
        Some(data) => (data.items.clone(), data.selected, data.hover, data.first),
        None => (Vec::new(), 0, -1, 0),
    };
    let font = font_for(HWND(button as *mut _));
    let capacity = row_count(client);
    let end = (first + capacity).min(items.len() as i32);
    for index in first..end {
        let bounds = row_rect(client, index - first);
        let item = &items[index as usize];
        if item.header {
            draw_popup_header(hdc, &bounds, font, &item.label, p);
            continue;
        }
        let state = paint::ControlState {
            hovered: index == hover,
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
    if y < client.top + inset {
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
        let visible = visible_rows(data.items.len());
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
        let visible = visible_rows(data.items.len());
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
                let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
                if delta > 0 {
                    scroll_rows(-3);
                } else if delta < 0 {
                    scroll_rows(3);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let _ = SetCapture(hwnd);
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
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let point = POINT {
                    x: (lparam.0 & 0xFFFF) as i16 as i32,
                    y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                };
                let chosen = if point.x < client.left || point.x >= client.right {
                    -1
                } else {
                    row_at(&client, point.y)
                };
                if chosen >= 0 {
                    let button = OPEN_BUTTON.load(Ordering::SeqCst);
                    select_index(HWND(button as *mut _), chosen);
                }
                close(chosen >= 0);
                LRESULT(0)
            }
            WM_KEYDOWN => match wparam.0 {
                0x1B => {
                    // Escape closes without committing.
                    close(false);
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
            WM_CANCELMODE | WM_KILLFOCUS => {
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
