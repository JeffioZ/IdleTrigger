//! Signatures mirror the Go nativeform painters 1:1, argument count
//! included.
#![allow(clippy::too_many_arguments)]
//! GDI+ anti-aliased vector primitives plus the Go nativeform control
//! painters (controls.go / gdiplus_windows.go parity). Text always renders
//! through GDI DrawTextW; GDI+ only draws rounded surfaces, the check glyph,
//! and compact vector marks.

use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, FillRect, FrameRect,
    GetTextExtentPoint32W, HDC, HFONT, HGDIOBJ, LineTo, MoveToEx, PEN_STYLE, PS_SOLID, RoundRect,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::Graphics::GdiPlus::Point as GpPoint;
use windows::Win32::Graphics::GdiPlus::{
    FillModeWinding, GdipAddPathBezierI, GdipAddPathLineI, GdipClosePathFigure, GdipCreateFromHDC,
    GdipCreatePath, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath,
    GdipFillPath, GdipFillPolygonI, GdipSetPixelOffsetMode, GdipSetSmoothingMode,
    GdipStartPathFigure, GdiplusShutdown, GdiplusStartup, GdiplusStartupInput,
    GdiplusStartupOutput, GpBrush, GpGraphics, GpPath, GpSolidFill, PixelOffsetModeHalf,
    SmoothingMode,
};
use windows::core::BOOL;

use crate::theme::Palette;

/// Transient interaction state shared by every owner-drawn control
/// (Go nativeform.ControlState). Semantic state stays with the window.
#[derive(Clone, Copy, Default)]
pub struct ControlState {
    pub hovered: bool,
    pub pressed: bool,
    pub focused: bool,
    pub disabled: bool,
    pub active: bool,
    /// Choice-popup open state (reserved for the choice-popup batch).
    #[allow(dead_code)]
    pub open: bool,
}

/// Distinguishes a setup failure (GDI can still draw) from a final GDI+
/// call that may have changed pixels (Go gdiplus.DrawResult).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DrawResult {
    NotStarted,
    Completed,
    MayBeDirty,
}

// ---- GDI+ lifecycle --------------------------------------------------------

struct Session {
    /// Retained for GdiplusShutdown symmetry; the process runs to exit.
    #[allow(dead_code)]
    token: usize,
    #[allow(dead_code)]
    hook_token: usize,
    #[allow(dead_code)]
    unhook: isize,
}

static SESSION: OnceLock<Option<Session>> = OnceLock::new();
static DRAW_LOCK: Mutex<()> = Mutex::new(());

type HookProc = unsafe extern "system" fn(*mut usize) -> i32;

/// Boots GDI+ once with the notification-hook path so GDI+ never creates its
/// hidden background window (Go startup path). Returns false when GDI+ is
/// unavailable; callers then fall back to plain GDI shapes.
pub fn ensure_started() -> bool {
    SESSION
        .get_or_init(|| unsafe {
            let input = GdiplusStartupInput {
                GdiplusVersion: 1,
                DebugEventCallback: 0,
                SuppressBackgroundThread: BOOL::from(true),
                SuppressExternalCodecs: BOOL::from(false),
            };
            let mut token = 0usize;
            let mut output = GdiplusStartupOutput::default();
            if GdiplusStartup(&mut token, &input, &mut output).0 != 0 || token == 0 {
                return None;
            }
            let hook: HookProc = std::mem::transmute(output.NotificationHook);
            let mut hook_token = 0usize;
            if hook(&mut hook_token) != 0 {
                GdiplusShutdown(token);
                return None;
            }
            Some(Session {
                token,
                hook_token,
                unhook: output.NotificationUnhook,
            })
        })
        .is_some()
}

fn argb(color: u32) -> u32 {
    0xff00_0000
        | ((color & 0x0000_00ff) << 16)
        | (color & 0x0000_ff00)
        | ((color & 0x00ff_0000) >> 16)
}

struct GraphicsGuard(*mut GpGraphics);

impl GraphicsGuard {
    unsafe fn new(hdc: HDC) -> Option<Self> {
        unsafe {
            let mut graphics: *mut GpGraphics = std::ptr::null_mut();
            if GdipCreateFromHDC(hdc, &mut graphics).0 != 0 || graphics.is_null() {
                return None;
            }
            // SmoothingMode 5 = AntiAlias8x8 (Go smoothingModeAntiAlias8x8).
            if GdipSetSmoothingMode(graphics, SmoothingMode(5)).0 != 0
                || GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf).0 != 0
            {
                GdipDeleteGraphics(graphics);
                return None;
            }
            Some(Self(graphics))
        }
    }
}

impl Drop for GraphicsGuard {
    fn drop(&mut self) {
        unsafe {
            GdipDeleteGraphics(self.0);
        }
    }
}

struct BrushGuard(*mut GpBrush);

impl BrushGuard {
    unsafe fn new(color: u32) -> Option<Self> {
        unsafe {
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            if GdipCreateSolidFill(argb(color), &mut brush).0 != 0 || brush.is_null() {
                return None;
            }
            Some(Self(brush as *mut GpBrush))
        }
    }
}

impl Drop for BrushGuard {
    fn drop(&mut self) {
        unsafe {
            GdipDeleteBrush(self.0);
        }
    }
}

struct PathGuard(*mut GpPath);

impl PathGuard {
    /// Integer-bezier rounded rect (Go roundedRectPath).
    unsafe fn new_rounded(bounds: &RECT, mut radius: i32) -> Option<Self> {
        unsafe {
            let width = bounds.right - bounds.left;
            let height = bounds.bottom - bounds.top;
            if width <= 0 || height <= 0 {
                return None;
            }
            if radius < 0 {
                radius = 0;
            }
            if radius > width / 2 {
                radius = width / 2;
            }
            if radius > height / 2 {
                radius = height / 2;
            }
            let mut path: *mut GpPath = std::ptr::null_mut();
            if GdipCreatePath(FillModeWinding, &mut path).0 != 0 || path.is_null() {
                return None;
            }
            let guard = PathGuard(path);
            let (l, t, r, b) = (bounds.left, bounds.top, bounds.right, bounds.bottom);
            let ok = GdipStartPathFigure(path).0 == 0
                && if radius == 0 {
                    GdipAddPathLineI(path, l, t, r, t).0 == 0
                        && GdipAddPathLineI(path, r, t, r, b).0 == 0
                        && GdipAddPathLineI(path, r, b, l, b).0 == 0
                } else {
                    // 0.552 is the standard cubic approximation of a quarter
                    // circle, kept in integer device pixels (Go parity).
                    let o = (radius * 552 + 500) / 1000;
                    GdipAddPathLineI(path, l + radius, t, r - radius, t).0 == 0
                        && GdipAddPathBezierI(
                            path,
                            r - radius,
                            t,
                            r - radius + o,
                            t,
                            r,
                            t + radius - o,
                            r,
                            t + radius,
                        )
                        .0 == 0
                        && GdipAddPathLineI(path, r, t + radius, r, b - radius).0 == 0
                        && GdipAddPathBezierI(
                            path,
                            r,
                            b - radius,
                            r,
                            b - radius + o,
                            r - radius + o,
                            b,
                            r - radius,
                            b,
                        )
                        .0 == 0
                        && GdipAddPathLineI(path, r - radius, b, l + radius, b).0 == 0
                        && GdipAddPathBezierI(
                            path,
                            l + radius,
                            b,
                            l + radius - o,
                            b,
                            l,
                            b - radius + o,
                            l,
                            b - radius,
                        )
                        .0 == 0
                        && GdipAddPathLineI(path, l, b - radius, l, t + radius).0 == 0
                        && GdipAddPathBezierI(
                            path,
                            l,
                            t + radius,
                            l,
                            t + radius - o,
                            l + radius - o,
                            t,
                            l + radius,
                            t,
                        )
                        .0 == 0
                };
            if !ok || GdipClosePathFigure(path).0 != 0 {
                return None;
            }
            Some(guard)
        }
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        unsafe {
            GdipDeletePath(self.0);
        }
    }
}

/// Anti-aliased rounded card: fills the outer path with `border`, then the
/// path inset by one physical pixel with `fill` (Go FillRoundedRect).
pub fn fill_rounded_rect(
    hdc: HDC,
    bounds: &RECT,
    radius: i32,
    fill: u32,
    border: u32,
) -> DrawResult {
    if !ensure_started() || bounds.right - bounds.left <= 2 || bounds.bottom - bounds.top <= 2 {
        return DrawResult::NotStarted;
    }
    let _guard = DRAW_LOCK.lock().unwrap();
    unsafe {
        let Some(graphics) = GraphicsGuard::new(hdc) else {
            return DrawResult::NotStarted;
        };
        let Some(border_brush) = BrushGuard::new(border) else {
            return DrawResult::NotStarted;
        };
        let Some(fill_brush) = BrushGuard::new(fill) else {
            return DrawResult::NotStarted;
        };
        let Some(outer) = PathGuard::new_rounded(bounds, radius) else {
            return DrawResult::NotStarted;
        };
        let inner = RECT {
            left: bounds.left + 1,
            top: bounds.top + 1,
            right: bounds.right - 1,
            bottom: bounds.bottom - 1,
        };
        if inner.right <= inner.left || inner.bottom <= inner.top {
            return DrawResult::NotStarted;
        }
        let Some(inner_path) = PathGuard::new_rounded(&inner, radius - 1) else {
            return DrawResult::NotStarted;
        };
        if GdipFillPath(graphics.0, border_brush.0, outer.0).0 != 0
            || GdipFillPath(graphics.0, fill_brush.0, inner_path.0).0 != 0
        {
            return DrawResult::MayBeDirty;
        }
        DrawResult::Completed
    }
}

/// Anti-aliased filled polygon (Go FillPolygon).
pub fn fill_polygon(hdc: HDC, points: &[GpPoint], color: u32) -> DrawResult {
    if !ensure_started() || points.len() < 3 {
        return DrawResult::NotStarted;
    }
    let _guard = DRAW_LOCK.lock().unwrap();
    unsafe {
        let Some(graphics) = GraphicsGuard::new(hdc) else {
            return DrawResult::NotStarted;
        };
        let Some(brush) = BrushGuard::new(color) else {
            return DrawResult::NotStarted;
        };
        if GdipFillPolygonI(
            graphics.0,
            brush.0,
            points.as_ptr(),
            points.len() as i32,
            FillModeWinding,
        )
        .0 != 0
        {
            return DrawResult::MayBeDirty;
        }
        DrawResult::Completed
    }
}

/// The checkbox's diagonal mark as a filled anti-aliased polygon (Go
/// DrawCheck).
pub fn draw_check(
    hdc: HDC,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    color: u32,
    width: i32,
) -> DrawResult {
    let (w, h) = (right - left, bottom - top);
    if w <= 0 || h <= 0 || width <= 0 {
        return DrawResult::NotStarted;
    }
    let (x1, y1) = (left + w * 24 / 100, top + h * 51 / 100);
    let (x2, y2) = (left + w * 43 / 100, top + h * 70 / 100);
    let (x3, y3) = (left + w * 77 / 100, top + h * 30 / 100);
    let pts = [
        GpPoint {
            X: x1 - width / 2,
            Y: y1,
        },
        GpPoint {
            X: x2,
            Y: y2 + width / 2,
        },
        GpPoint {
            X: x3 + width / 2,
            Y: y3,
        },
        GpPoint {
            X: x3,
            Y: y3 - width / 2,
        },
        GpPoint {
            X: x2,
            Y: y2 - width / 2,
        },
        GpPoint {
            X: x1 + width / 2,
            Y: y1 - width / 2,
        },
    ];
    fill_polygon(hdc, &pts, color)
}

// ---- GDI helpers -----------------------------------------------------------

pub fn fill_rect(hdc: HDC, bounds: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(COLORREF(color));
        if brush.is_invalid() {
            return;
        }
        FillRect(hdc, bounds, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

pub fn frame_rect(hdc: HDC, bounds: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(COLORREF(color));
        if brush.is_invalid() {
            return;
        }
        let _ = FrameRect(hdc, bounds, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

/// Logical→physical pixels via the live DPI scale (Go scaledPixels).
pub fn sp(logical: i32, scale: i32) -> i32 {
    if scale <= 96 {
        if scale <= 0 {
            return logical;
        }
        return logical * scale / 96;
    }
    (logical as i64 * scale as i64 + 48) as i32 / 96
}

// ---- Control painters (Go nativeform/controls.go) --------------------------

use windows::Win32::Graphics::Gdi::{
    DT_CALCRECT, DT_CENTER, DT_LEFT, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK,
};

/// Rounded surface with a 1px border; falls back to GDI RoundRect when the
/// GDI+ path is unavailable (Go DrawSurface).
pub fn draw_surface(hdc: HDC, bounds: &RECT, background: u32, fill: u32, border: u32, radius: i32) {
    fill_rect(hdc, bounds, background);
    match fill_rounded_rect(hdc, bounds, radius, fill, border) {
        DrawResult::Completed => {}
        DrawResult::MayBeDirty => fill_rect(hdc, bounds, background),
        DrawResult::NotStarted => unsafe {
            let brush = CreateSolidBrush(COLORREF(fill));
            let pen = CreatePen(PEN_STYLE(PS_SOLID.0), 1, COLORREF(border));
            if brush.is_invalid() || pen.is_invalid() {
                if !brush.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(brush.0));
                }
                if !pen.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(pen.0));
                }
                return;
            }
            let old_brush = SelectObject(hdc, HGDIOBJ(brush.0));
            let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
            let _ = RoundRect(
                hdc,
                bounds.left,
                bounds.top,
                bounds.right,
                bounds.bottom,
                (radius * 2).max(2),
                (radius * 2).max(2),
            );
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_brush);
            let _ = DeleteObject(HGDIOBJ(pen.0));
            let _ = DeleteObject(HGDIOBJ(brush.0));
        },
    }
}

/// Edit-field surface with focus/hover/disabled border states (Go DrawField).
pub fn draw_field(
    hdc: HDC,
    bounds: &RECT,
    p: &Palette,
    background: u32,
    state: ControlState,
    radius: i32,
) {
    let (mut fill, mut border) = (p.surface, p.border);
    if state.disabled {
        fill = p.disabled_surface;
        border = p.subtle_border;
    } else if state.focused {
        border = p.focus;
    } else if state.hovered {
        border = p.accent;
    }
    draw_surface(hdc, bounds, background, fill, border, radius);
}

fn button_visual(p: &Palette, state: ControlState) -> (u32, u32, u32) {
    let (mut fill, mut border, mut text) = (p.surface, p.border, p.text);
    if state.hovered {
        fill = p.hover_surface;
        border = p.accent;
    }
    if state.active {
        fill = p.selected;
        border = p.selected;
        text = p.accent_text;
        if state.hovered {
            fill = p.selected_hover;
            border = p.selected_hover;
        }
    }
    // Pressed is the strongest transient state for both roles.
    if state.pressed {
        fill = p.accent_pressed;
        border = p.accent_pressed;
        text = p.accent_text;
    }
    if state.disabled {
        fill = p.disabled_surface;
        border = p.subtle_border;
        text = p.disabled_text;
    }
    if state.focused && !state.disabled {
        border = p.focus;
    }
    (fill, border, text)
}

/// Owner-drawn push button / toggle (Go DrawButton). `state.active` paints
/// the accent-filled "on" look used by toggles and selected tabs.
pub fn draw_button(
    hdc: HDC,
    bounds: &RECT,
    font: HFONT,
    label: &str,
    p: &Palette,
    background: u32,
    state: ControlState,
    radius: i32,
) {
    let (fill, border, text) = button_visual(p, state);
    draw_surface(hdc, bounds, background, fill, border, radius);
    draw_button_label(hdc, bounds, font, label, text, false, 10, 10);
}

/// Choice (combo) closed state with the drop arrow (Go DrawChoice).
/// Reserved for the custom choice-popup batch.
#[allow(dead_code)]
pub fn draw_choice(
    hdc: HDC,
    bounds: &RECT,
    font: HFONT,
    label: &str,
    p: &Palette,
    background: u32,
    state: ControlState,
    radius: i32,
    scale: i32,
) {
    let (mut fill, mut border) = (p.surface, p.border);
    let (mut text, mut arrow) = (p.text, p.text2);
    if state.hovered || state.open {
        fill = p.hover_surface;
        border = p.accent;
        arrow = p.accent;
    }
    if state.pressed {
        fill = p.accent_pressed;
        border = p.accent_pressed;
        text = p.accent_text;
        arrow = p.accent_text;
    }
    if state.disabled {
        fill = p.disabled_surface;
        border = p.subtle_border;
        text = p.disabled_text;
        arrow = p.disabled_text;
    }
    if state.focused && !state.disabled {
        border = p.focus;
    }
    draw_surface(hdc, bounds, background, fill, border, radius);
    let mut text_bounds = *bounds;
    text_bounds.right -= sp(30, scale);
    draw_label(hdc, &text_bounds, font, label, text, true, 10, 4);
    draw_arrow(
        hdc,
        bounds.right - sp(18, scale),
        (bounds.top + bounds.bottom) / 2,
        state.open,
        arrow,
        scale,
    );
}

/// Checkbox with label (Go DrawCheckbox). `state.active` = checked.
pub fn draw_checkbox(
    hdc: HDC,
    bounds: &RECT,
    font: HFONT,
    label: &str,
    p: &Palette,
    background: u32,
    state: ControlState,
    scale: i32,
    checkbox_size: i32,
) {
    fill_rect(hdc, bounds, background);
    let size = sp(checkbox_size, scale);
    let left = bounds.left + sp(2, scale);
    let top = bounds.top + (bounds.bottom - bounds.top - size) / 2;
    let box_rect = RECT {
        left,
        top,
        right: left + size,
        bottom: top + size,
    };
    draw_checkbox_box(hdc, &box_rect, p, background, state, scale);
    if state.focused && !state.disabled {
        let inset = sp(1, scale);
        frame_rect(
            hdc,
            &RECT {
                left: bounds.left + inset,
                top: bounds.top + inset,
                right: bounds.right - inset,
                bottom: bounds.bottom - inset,
            },
            p.focus,
        );
    }
    let mut text_bounds = *bounds;
    text_bounds.left = box_rect.right + sp(8, scale);
    let color = if state.disabled {
        p.disabled_text
    } else {
        p.text
    };
    draw_label(hdc, &text_bounds, font, label, color, true, 0, 4);
}

fn draw_checkbox_box(
    hdc: HDC,
    box_rect: &RECT,
    p: &Palette,
    background: u32,
    state: ControlState,
    scale: i32,
) {
    let (mut fill, mut border) = (p.surface, p.border);
    if state.active {
        fill = p.accent;
        border = p.accent;
    }
    if state.hovered {
        border = p.accent_hover;
        if state.active {
            fill = p.accent_hover;
        }
    }
    if state.pressed {
        fill = p.accent_pressed;
        border = p.accent_pressed;
    }
    if state.disabled {
        fill = p.disabled_surface;
        border = p.subtle_border;
    }
    draw_surface(hdc, box_rect, background, fill, border, sp(2, scale));
    if state.active {
        let mut check_color = p.accent_text;
        if state.disabled {
            check_color = p.muted;
        }
        if draw_check(
            hdc,
            box_rect.left,
            box_rect.top,
            box_rect.right,
            box_rect.bottom,
            check_color,
            sp(2, scale).max(1),
        ) != DrawResult::Completed
        {
            draw_check_fallback(hdc, box_rect, check_color, scale);
        }
    }
}

fn draw_check_fallback(hdc: HDC, box_rect: &RECT, color: u32, scale: i32) {
    unsafe {
        let pen = CreatePen(PEN_STYLE(PS_SOLID.0), sp(2, scale).max(1), COLORREF(color));
        if pen.is_invalid() {
            return;
        }
        let old = SelectObject(hdc, HGDIOBJ(pen.0));
        let _ = MoveToEx(
            hdc,
            box_rect.left + sp(4, scale),
            box_rect.top + sp(9, scale),
            None,
        );
        let _ = LineTo(
            hdc,
            box_rect.left + sp(8, scale),
            box_rect.top + sp(13, scale),
        );
        let _ = LineTo(
            hdc,
            box_rect.left + sp(15, scale),
            box_rect.top + sp(5, scale),
        );
        SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
}

/// Menu row for choice popups (Go DrawMenuOption).
/// Reserved for the custom choice-popup batch.
#[allow(dead_code)]
pub fn draw_menu_option(
    hdc: HDC,
    bounds: &RECT,
    font: HFONT,
    selected_font: HFONT,
    label: &str,
    p: &Palette,
    background: u32,
    state: ControlState,
    selected: bool,
    danger: bool,
    radius: i32,
    scale: i32,
) {
    let (mut fill, mut border, mut text) = (p.surface, p.border, p.text);
    if state.hovered {
        fill = p.hover_surface;
    }
    if danger {
        fill = p.danger_bg;
        border = p.danger_border;
        text = p.danger_text;
        if state.hovered {
            fill = p.danger_hover;
            border = p.danger_hover_border;
        }
    }
    if state.pressed {
        fill = p.elevated;
        border = p.accent_pressed;
        if danger {
            fill = p.danger_pressed;
            border = p.danger_pressed_border;
            text = p.danger_text;
        }
    }
    if state.disabled {
        fill = p.disabled_surface;
        border = p.subtle_border;
        text = p.disabled_text;
    }
    if state.focused && !state.disabled {
        border = if danger { p.danger_focus } else { p.focus };
    }
    draw_surface(hdc, bounds, background, fill, border, radius);
    if selected {
        // Go: marker.Left += MenuSurfaceInset(4) * scale, width 3 * scale.
        let inset = sp(4, scale);
        let marker = RECT {
            left: bounds.left + inset,
            right: bounds.left + inset + sp(3, scale),
            top: bounds.top + sp(6, scale),
            bottom: bounds.bottom - sp(6, scale),
        };
        if marker.left < marker.right && marker.top < marker.bottom {
            fill_rect(hdc, &marker, p.accent);
        }
    }
    let use_font = if selected && !selected_font.is_invalid() {
        selected_font
    } else {
        font
    };
    draw_label(
        hdc,
        bounds,
        use_font,
        label,
        text,
        true,
        sp(10, scale),
        sp(8, scale),
    );
}

/// Underlined accent-colored hyperlink (Go DrawTextLink).
pub fn draw_text_link(
    hdc: HDC,
    bounds: &RECT,
    font: HFONT,
    label: &str,
    p: &Palette,
    background: u32,
    state: ControlState,
    scale: i32,
) {
    fill_rect(hdc, bounds, background);
    let mut color = p.accent;
    if state.hovered {
        color = p.accent_hover;
    }
    if state.pressed {
        color = p.accent_pressed;
    }
    unsafe {
        let mut text: Vec<u16> = label.encode_utf16().collect();
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        SetTextColor(hdc, COLORREF(color));
        SetBkMode(hdc, TRANSPARENT);
        let mut text_bounds = *bounds;
        let _ = DrawTextW(
            hdc,
            &mut text,
            &mut text_bounds,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        let chars: Vec<u16> = label.encode_utf16().collect();
        let mut size = SIZE::default();
        if GetTextExtentPoint32W(hdc, &chars, &mut size).as_bool() {
            let underline_y = bounds.top + (bounds.bottom - bounds.top + size.cy) / 2;
            let pen = CreatePen(PEN_STYLE(PS_SOLID.0), sp(1, scale).max(1), COLORREF(color));
            if !pen.is_invalid() {
                let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
                let _ = MoveToEx(hdc, bounds.left, underline_y, None);
                let _ = LineTo(hdc, bounds.right.min(bounds.left + size.cx), underline_y);
                SelectObject(hdc, old_pen);
                let _ = DeleteObject(HGDIOBJ(pen.0));
            }
        }
        SelectObject(hdc, old);
    }
    if state.focused {
        let frame = RECT {
            bottom: bounds.bottom - 1,
            ..*bounds
        };
        frame_rect(hdc, &frame, p.focus);
    }
}

#[allow(dead_code)]
fn draw_arrow(hdc: HDC, x: i32, y: i32, up: bool, color: u32, scale: i32) {
    unsafe {
        let pen = CreatePen(PEN_STYLE(PS_SOLID.0), sp(1, scale).max(1), COLORREF(color));
        if pen.is_invalid() {
            return;
        }
        let old = SelectObject(hdc, HGDIOBJ(pen.0));
        let (half_w, half_h) = (sp(4, scale), sp(2, scale));
        if up {
            let _ = MoveToEx(hdc, x - half_w, y + half_h, None);
            let _ = LineTo(hdc, x, y - half_h);
            let _ = LineTo(hdc, x + half_w, y + half_h);
        } else {
            let _ = MoveToEx(hdc, x - half_w, y - half_h, None);
            let _ = LineTo(hdc, x, y + half_h);
            let _ = LineTo(hdc, x + half_w, y - half_h);
        }
        SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
}

fn draw_label(
    hdc: HDC,
    bounds: &RECT,
    font: HFONT,
    label: &str,
    color: u32,
    left: bool,
    left_inset: i32,
    right_inset: i32,
) {
    unsafe {
        // No trailing NUL: windows-rs passes the slice length as cchText, and
        // an explicitly counted NUL widens DT_CALCRECT results.
        let mut text: Vec<u16> = label.encode_utf16().collect();
        let mut bounds = RECT {
            left: bounds.left + left_inset,
            right: bounds.right - right_inset,
            ..*bounds
        };
        SetTextColor(hdc, COLORREF(color));
        SetBkMode(hdc, TRANSPARENT);
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        let flags = if left {
            DT_LEFT | DT_VCENTER | DT_SINGLELINE
        } else {
            DT_CENTER | DT_VCENTER | DT_SINGLELINE
        };
        let _ = DrawTextW(hdc, &mut text, &mut bounds, flags);
        SelectObject(hdc, old);
    }
}

/// Measures the text block first, centers it, and applies the CJK optical
/// lift so Han glyphs sit on the optical row center (Go drawButtonLabel).
pub fn draw_button_label(
    hdc: HDC,
    bounds: &RECT,
    font: HFONT,
    label: &str,
    color: u32,
    left: bool,
    left_inset: i32,
    right_inset: i32,
) {
    unsafe {
        // No trailing NUL (cchText counts it and inflates the measure).
        let mut text: Vec<u16> = label.encode_utf16().collect();
        let mut bounds = RECT {
            left: bounds.left + left_inset,
            right: bounds.right - right_inset,
            ..*bounds
        };
        SetTextColor(hdc, COLORREF(color));
        SetBkMode(hdc, TRANSPARENT);
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        // Measure first, then place the text block at exact integer centers:
        // DT_CENTER rounds asymmetrically at fractional DPI scales, which read
        // as a persistent left bias (Go measures and positions manually).
        let original_top = bounds.top;
        let mut measured = bounds;
        let has_measure = DrawTextW(hdc, &mut text, &mut measured, DT_WORDBREAK | DT_CALCRECT) != 0;
        if has_measure {
            let text_w = measured.right - measured.left;
            let text_h = measured.bottom - measured.top;
            if !left {
                let avail = bounds.right - bounds.left;
                // Round half up: plain floor rounding reads as a persistent
                // left bias at fractional DPI scales (Go lands on exact
                // centers).
                bounds.left += (avail - text_w + 1).max(0) / 2;
                bounds.right = bounds.left + text_w;
            }
            bounds.top = centered_text_top(bounds.top, bounds.bottom, text_h);
            if bounds.top > original_top && needs_han_lift(label) {
                // Microsoft YaHei UI's Han mass sits one pixel below the
                // measured center at 100/150/200% DPI (Go parity).
                bounds.top -= 1;
                bounds.bottom -= 1;
            }
        }

        let _ = DrawTextW(hdc, &mut text, &mut bounds, DT_LEFT | DT_WORDBREAK);
        SelectObject(hdc, old);
    }
}

fn centered_text_top(top: i32, bottom: i32, text_height: i32) -> i32 {
    let available = bottom - top;
    if text_height <= 0 || text_height >= available {
        return top;
    }
    top + (available - text_height) / 2
}

fn needs_han_lift(label: &str) -> bool {
    label.chars().any(|c| {
        let v = c as u32;
        (0x4E00..=0x9FFF).contains(&v)
            || (0x3400..=0x4DBF).contains(&v)
            || (0xF900..=0xFAFF).contains(&v)
    })
}
