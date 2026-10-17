//! Panel control construction: the physical-rect spec family the control
//! panel builds its rows from. The panel positions children at runtime in
//! physical pixels, so these ride kit::controls::raw_physical; every other
//! window's controls go through the kit's scaled idioms instead.

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::kit::layout::{CARD_PAD_X, PAD, SECTION_H, SUBTITLE_H};
use crate::nativeform;

use crate::{IDC_FOOTER_VERSION, PANEL_CLIENT_WIDTH};

fn scale(v: i32) -> i32 {
    crate::dpi::scale(v)
}

/// Logical pixel width of a single-line label at the given font (Go
/// GetTextExtent-based measuring).
pub(crate) fn measured_text_width(parent: HWND, font: HFONT, label: &str) -> i32 {
    unsafe {
        let hdc = windows::Win32::Graphics::Gdi::GetDC(Some(parent));
        if hdc.is_invalid() {
            return 0;
        }
        let old = windows::Win32::Graphics::Gdi::SelectObject(
            hdc,
            windows::Win32::Graphics::Gdi::HGDIOBJ(font.0),
        );
        let mut text: Vec<u16> = label.encode_utf16().collect();
        let mut bounds = RECT::default();
        let height = windows::Win32::Graphics::Gdi::DrawTextW(
            hdc,
            &mut text,
            &mut bounds,
            windows::Win32::Graphics::Gdi::DT_LEFT
                | windows::Win32::Graphics::Gdi::DT_SINGLELINE
                | windows::Win32::Graphics::Gdi::DT_CALCRECT,
        );
        windows::Win32::Graphics::Gdi::SelectObject(hdc, old);
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(Some(parent), hdc);
        if height == 0 || bounds.right <= bounds.left {
            return 0;
        }
        let dpi = scale(96).max(96);
        (bounds.right - bounds.left) * 96 / dpi
    }
}

/// Right-aligned muted version text on the More card. Owner-draw like every
/// panel static, so the themed muted ink rides the normal draw dispatch.
pub(crate) unsafe fn mk_footer_version(panel: HWND, text: &str, font: HFONT, y: i32) -> HWND {
    {
        let card_w = PANEL_CLIENT_WIDTH - 2 * PAD;
        let row_right = PAD + CARD_PAD_X + (card_w - 2 * CARD_PAD_X);
        let width = scale(measured_text_width(panel, font, text).max(8));
        owner_static(
            &ControlSpec {
                parent: panel,
                label: text.to_string(),
                // ControlSpec coordinates are PHYSICAL like every other
                // creation site — the raw logical values placed the label
                // at the panel's top-left corner.
                x: scale(row_right) - width,
                y: scale(y),
                width,
                id: IDC_FOOTER_VERSION,
                font,
            },
            scale(SUBTITLE_H),
        )
    }
}

pub(crate) fn section_header(
    parent: HWND,
    label: &str,
    y: i32,
    section_id: usize,
    font: HFONT,
) -> HWND {
    owner_static(
        &ControlSpec {
            parent,
            label: label.to_string(),
            x: scale(PAD),
            y: scale(y),
            width: scale(PANEL_CLIENT_WIDTH - 2 * PAD),
            id: section_id,
            font,
        },
        scale(SECTION_H),
    )
}

pub(crate) fn set_control_text(control: HWND, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = SetWindowTextW(control, PCWSTR(wide.as_ptr()));
    }
}

/// Shared creation parameters for standard child controls.
pub(crate) struct ControlSpec {
    pub(crate) parent: HWND,
    pub(crate) label: String,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) id: usize,
    pub(crate) font: HFONT,
}

pub(crate) unsafe fn create_control(
    spec: &ControlSpec,
    class: PCWSTR,
    extra_style: u32,
    height: i32,
) -> HWND {
    // The panel positions children in physical pixels at runtime, so the
    // kit's physical-rect creator serves it directly.
    unsafe {
        crate::kit::controls::raw_physical(
            spec.parent,
            class,
            &spec.label,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | extra_style),
            spec.id,
            spec.font,
            (spec.x, spec.y, spec.width, height),
        )
    }
}

/// Owner-drawn toggle/command button (Go: bsOwnerDraw BUTTON). Visuals come
/// from paint::draw_button / paint::draw_switch_row in the parent's WM_DRAWITEM.
pub(crate) fn owner_button(spec: &ControlSpec, height: i32) -> HWND {
    unsafe {
        let hwnd_ = create_control(
            spec,
            w!("BUTTON"),
            BS_OWNERDRAW as u32 | WS_TABSTOP.0,
            height,
        );
        nativeform::track(hwnd_);
        hwnd_
    }
}

/// Owner-drawn static (Go: ssOwnerDraw STATIC) painted by the parent.
pub(crate) fn owner_static(spec: &ControlSpec, height: i32) -> HWND {
    // SS_OWNERDRAW = 13 (Go raw style value).
    unsafe { create_control(spec, w!("STATIC"), 13, height) }
}

pub(crate) fn static_text(spec: &ControlSpec) -> HWND {
    unsafe { create_control(spec, w!("STATIC"), 0, scale(20)) }
}

pub(crate) fn set_control_font(control: HWND, font: HFONT) -> bool {
    unsafe {
        SendMessageW(
            control,
            0x0030,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        )
        .0 != 0
    }
}
