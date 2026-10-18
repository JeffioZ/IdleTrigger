//! Control creation primitives shared by every form window. Each helper
//! creates one native child control at a logical-pixel rect (scaled via
//! `scale_pub` under the caller's DPI scope), assigns the caller's
//! control id, sets the kit body font, and wires the shared interaction
//! plumbing (nativeform hover/press tracking, choice popups). Behavior is
//! lifted verbatim from the settings window's local helpers — control
//! ids, styles, and geometry are part of the observable contract.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::EM_SETLIMITTEXT;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use crate::kit::fonts;

/// The native EDIT occupies the text area inside the card-painted well.
pub fn edit_rect((x, y, w, h): (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    (x + 3, y + 7, w - 6, (h - 14).max(20))
}

unsafe fn child(
    parent: HWND,
    class: windows::core::PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    id: i32,
    font: windows::Win32::Graphics::Gdi::HFONT,
    b: (i32, i32, i32, i32),
) -> HWND {
    unsafe {
        let (x, y, w, h) = b;
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(crate::wide(text).as_ptr()),
            style,
            crate::scale_pub(x),
            crate::scale_pub(y),
            crate::scale_pub(w),
            crate::scale_pub(h),
            Some(parent),
            Some(HMENU(id as isize as *mut _)),
            Some(instance.into()),
            None,
        )
        .unwrap_or_default();
        if !hwnd.is_invalid() {
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            crate::dpi::remember_rect(hwnd, b);
        }
        hwnd
    }
}

/// Owner-drawn section card surface behind a section's rows (panel card
/// grammar): rounded face with the family hairline, painted by the parent's
/// WM_DRAWITEM dispatch. Created before the section's controls so it stays
/// beneath them in z-order; never interactive, so it gets no hover track.
pub unsafe fn section_card(
    parent: HWND,
    id: i32,
    font: windows::Win32::Graphics::Gdi::HFONT,
    b: (i32, i32, i32, i32),
) -> HWND {
    unsafe {
        let card = child(
            parent,
            windows::core::w!("STATIC"),
            "",
            // WS_CLIPSIBLINGS: the card repaints the field wells it hosts;
            // without it the card's DC does not exclude the edit sitting
            // above, and a well repaint paints OVER the edit's pixels -
            // toggling an unrelated switch made the field read as cleared.
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0 | 13), // SS_OWNERDRAW
            id,
            font,
            b,
        );
        // Pin beneath everything (manager list-surface parity): resize
        // passes reorder siblings, and a card floating above its rows
        // clips their owner-draw DCs to nothing - fields vanished this way.
        let _ = SetWindowPos(
            card,
            Some(windows::Win32::Foundation::HWND(
                std::ptr::NonNull::dangling().as_ptr(),
            )), // HWND_BOTTOM
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        card
    }
}

pub unsafe fn label(
    parent: HWND,
    id: i32,
    text: &str,
    font: windows::Win32::Graphics::Gdi::HFONT,
    b: (i32, i32, i32, i32),
    right: bool,
) -> HWND {
    // SS_RIGHT = 2 (Go labelRight uses the raw style value).
    let extra = 0x0080 | if right { 0x4002 } else { 0 }; // SS_NOPREFIX, SS_RIGHT | SS_ENDELLIPSIS
    unsafe {
        child(
            parent,
            windows::core::w!("STATIC"),
            text,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | extra),
            id,
            font,
            b,
        )
    }
}

/// Owner-drawn push button (Go bsOwnerDraw BUTTON + DrawButton).
pub unsafe fn push_button(parent: HWND, id: i32, text: &str, b: (i32, i32, i32, i32)) -> HWND {
    unsafe {
        let hwnd = child(
            parent,
            windows::core::w!("BUTTON"),
            text,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            id,
            fonts::body(),
            b,
        );
        crate::nativeform::track(hwnd);
        hwnd
    }
}

/// Navigation tab: an owner-drawn button latched active to the selected
/// page (Go tab buttons draw with the Active accent fill).
pub unsafe fn tab_button(parent: HWND, id: i32, text: &str, b: (i32, i32, i32, i32)) {
    unsafe { push_button(parent, id, text, b) };
}

/// Owner-drawn switch row: on/off state lives in the window's checkbox
/// state store (BM_SETCHECK has no effect on owner-draw buttons), painted
/// via paint::draw_switch_row.
pub unsafe fn checkbox(parent: HWND, id: i32, text: &str, b: (i32, i32, i32, i32)) {
    unsafe {
        let hwnd = child(
            parent,
            windows::core::w!("BUTTON"),
            text,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            id,
            fonts::body(),
            b,
        );
        crate::nativeform::track(hwnd);
        // Toggle rows hover on the pill column alone (panel parity).
        crate::nativeform::set_hover_column(hwnd, crate::kit::layout::SWITCH_HIT_W);
    }
}

/// Text hyperlink rendered as an owner-drawn button (Go DrawTextLink keeps
/// native keyboard focus and pressed feedback).
pub unsafe fn link(parent: HWND, id: i32, text: &str, b: (i32, i32, i32, i32)) {
    unsafe {
        let hwnd = child(
            parent,
            windows::core::w!("BUTTON"),
            text,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            id,
            fonts::body(),
            b,
        );
        crate::nativeform::track(hwnd);
    }
}

/// Edit field: a borderless EDIT inset in the card face, with the
/// surrounding well and hairline painted by the hosting card (see the
/// CARD arm of the draw dispatch), so nothing stacks against the card
/// (Go editWithStyle + ControlSurface parity).
pub unsafe fn edit(parent: HWND, id: i32, b: (i32, i32, i32, i32), numeric: bool) -> HWND {
    unsafe {
        // The edit is the inner area; the surrounding well/ring paints on
        // the hosting card's face (see the CARD arm in the draw dispatch),
        // so the field is card pixels + one inset child - no stacking, no
        // non-client tricks.
        let extra = if numeric { ES_NUMBER as u32 } else { 0 };
        let edit = child(
            parent,
            windows::core::w!("EDIT"),
            "",
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32 | extra),
            id,
            fonts::body(),
            edit_rect(b),
        );
        // Cap input length: 5 digits for numbers, 5 chars for HH:MM.
        let _ = SendMessageW(edit, EM_SETLIMITTEXT, Some(WPARAM(5)), Some(LPARAM(0)));
        edit
    }
}

/// Go choice control: an owner-drawn button with a custom drop-down popup
/// (nativeform choicepopup parity).
pub fn combo(parent: HWND, id: i32, b: (i32, i32, i32, i32), items: &[String]) {
    let pairs: Vec<(String, String)> = items
        .iter()
        .map(|label| (label.clone(), label.clone()))
        .collect();
    crate::choice::create(parent, id, b, &pairs, fonts::body());
}

/// Choice creation from full row items (headers allowed).
pub fn combo_items(
    parent: HWND,
    id: i32,
    b: (i32, i32, i32, i32),
    rows: &[crate::choice::ChoiceItem],
) {
    crate::choice::create_rows(parent, id, b, rows, fonts::body());
}

/// Physical text width converted back to logical pixels (Go
/// logicalTextWidth).
pub fn logical_text_width(
    parent: HWND,
    font: windows::Win32::Graphics::Gdi::HFONT,
    text: &str,
    fallback: i32,
) -> i32 {
    use windows::Win32::Graphics::Gdi::{GetDC, GetTextExtentPoint32W, ReleaseDC, SelectObject};
    unsafe {
        let hdc = GetDC(Some(parent));
        if hdc.is_invalid() {
            return fallback;
        }
        let old = SelectObject(hdc, windows::Win32::Graphics::Gdi::HGDIOBJ(font.0));
        let chars: Vec<u16> = text.encode_utf16().collect();
        let mut size = windows::Win32::Foundation::SIZE::default();
        let measured = GetTextExtentPoint32W(hdc, &chars, &mut size).as_bool();
        SelectObject(hdc, old);
        let _ = ReleaseDC(Some(parent), hdc);
        if !measured || size.cx <= 0 {
            return fallback;
        }
        let dpi = crate::scale_pub(96);
        if dpi <= 0 {
            return fallback;
        }
        (size.cx as i64 * 96 + (dpi / 2) as i64) as i32 / dpi
    }
}

/// Public generic creator for windows that compose type variants on top
/// of the shared engine: scaled rect, explicit class/style/font, usize
/// control id (the automation/ctx convention).
pub unsafe fn raw(
    parent: HWND,
    class: windows::core::PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    id: usize,
    font: windows::Win32::Graphics::Gdi::HFONT,
    b: (i32, i32, i32, i32),
) -> HWND {
    unsafe { child(parent, class, text, style, id as i32, font, b) }
}

/// Creator for surfaces that position in physical pixels (the panel and
/// other runtime-placed controls): like `raw`, but the rect is used
/// as-is, never scaled.
pub unsafe fn raw_physical(
    parent: HWND,
    class: windows::core::PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    id: usize,
    font: windows::Win32::Graphics::Gdi::HFONT,
    at: (i32, i32, i32, i32),
) -> HWND {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(crate::wide(text).as_ptr()),
            style,
            at.0,
            at.1,
            at.2,
            at.3,
            Some(parent),
            Some(HMENU(id as isize as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("child control");
        let _ = SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        hwnd
    }
}

/// Plain visible static at a logical rect — no SS_NOPREFIX (manager and
/// picker captions never show menu mnemonics).
pub unsafe fn static_text(
    parent: HWND,
    id: usize,
    text: &str,
    font: windows::Win32::Graphics::Gdi::HFONT,
    b: (i32, i32, i32, i32),
) -> HWND {
    unsafe {
        child(
            parent,
            windows::core::w!("STATIC"),
            text,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            id as i32,
            font,
            b,
        )
    }
}

/// Raw creator for flow-placed controls: hidden, at a literal placeholder
/// rect (never scaled — the control is invisible until a kit::flow pass
/// positions it). Mirrors the automation editor's creation idiom.
unsafe fn hidden_child(
    parent: HWND,
    class: windows::core::PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    id: usize,
    font: windows::Win32::Graphics::Gdi::HFONT,
    placeholder: (i32, i32),
) -> HWND {
    unsafe {
        raw_physical(
            parent,
            class,
            text,
            style,
            id,
            font,
            (0, 0, placeholder.0, placeholder.1),
        )
    }
}

/// Flow-placed label: hidden at a 1×1 placeholder; a flow pass positions
/// it and applies visibility.
pub unsafe fn hidden_label(
    parent: HWND,
    id: usize,
    text: &str,
    font: windows::Win32::Graphics::Gdi::HFONT,
) {
    unsafe {
        hidden_child(
            parent,
            windows::core::w!("STATIC"),
            text,
            WINDOW_STYLE(WS_CHILD.0),
            id,
            font,
            (1, 1),
        );
    }
}

/// Flow-placed field edit: a bare inset EDIT (the ring/margin is painted
/// by the hosting section card), with Go editWithStyle's 6px inner
/// margins.
pub unsafe fn hidden_edit(parent: HWND, id: usize, numeric: bool) -> HWND {
    unsafe {
        let extra = if numeric { ES_NUMBER as u32 } else { 0 };
        let edit = hidden_child(
            parent,
            windows::core::w!("EDIT"),
            "",
            WINDOW_STYLE(
                WS_CHILD.0 | WS_TABSTOP.0 | WS_CLIPSIBLINGS.0 | ES_AUTOHSCROLL as u32 | extra,
            ),
            id,
            fonts::body(),
            (1, 1),
        );
        crate::dpi::set_edit_margins(edit, 6);
        edit
    }
}

/// Flow-placed owner-drawn button with hover/press tracking; created at
/// a 1×s(height) placeholder.
pub unsafe fn hidden_button(parent: HWND, id: usize, text: &str, height: i32) -> HWND {
    unsafe {
        let button = hidden_child(
            parent,
            windows::core::w!("BUTTON"),
            text,
            WINDOW_STYLE(WS_CHILD.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
            id,
            fonts::body(),
            (1, crate::scale_pub(height)),
        );
        crate::nativeform::track(button);
        button
    }
}
