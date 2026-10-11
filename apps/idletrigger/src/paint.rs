//! GDI+ anti-aliased vector primitives and owner-drawn control painters.
//! Text uses GDI DrawTextW; GDI+ draws rounded surfaces and vector marks.
//! Painters accept explicit geometry, colors, and state parameters.
#![allow(clippy::too_many_arguments)]
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DT_END_ELLIPSIS, DeleteObject, DrawTextW, ExcludeClipRect,
    FillRect, FrameRect, GetTextExtentPoint32W, HDC, HFONT, HGDIOBJ, LineTo, MoveToEx, PEN_STYLE,
    PS_SOLID, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::Graphics::GdiPlus::Point as GpPoint;
use windows::Win32::Graphics::GdiPlus::{
    CombineModeExclude, FillModeWinding, GdipAddPathBezierI, GdipAddPathLineI, GdipClosePathFigure,
    GdipCreateFromHDC, GdipCreatePath, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics,
    GdipDeletePath, GdipFillPath, GdipFillPolygonI, GdipSetClipRectI, GdipSetPixelOffsetMode,
    GdipSetSmoothingMode, GdipStartPathFigure, GdiplusShutdown, GdiplusStartup,
    GdiplusStartupInput, GdiplusStartupOutput, GpBrush, GpGraphics, GpPath, GpSolidFill,
    PixelOffsetModeHalf, SmoothingMode,
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
    /// Whether the choice popup is open.
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
            if output.NotificationHook == 0 {
                GdiplusShutdown(token);
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
    fill_rounded_rect_impl(hdc, bounds, radius, fill, border, None)
}

/// `fill_rounded_rect` with an optional clipped-out hole (card wells: the
/// hosted edit's rect). The owner-draw item DC carries no sibling clipping
/// and GDI+ ignores GDI clip regions, so wells exclude their edits on the
/// GDI+ side to keep a card repaint off the edit's pixels.
fn fill_rounded_rect_impl(
    hdc: HDC,
    bounds: &RECT,
    radius: i32,
    fill: u32,
    border: u32,
    exclude: Option<&RECT>,
) -> DrawResult {
    if !ensure_started() || bounds.right - bounds.left <= 2 || bounds.bottom - bounds.top <= 2 {
        return DrawResult::NotStarted;
    }
    let _guard = crate::runtime::lock(&DRAW_LOCK);
    unsafe {
        let Some(graphics) = GraphicsGuard::new(hdc) else {
            return DrawResult::NotStarted;
        };
        if let Some(ex) = exclude
            && GdipSetClipRectI(
                graphics.0,
                ex.left,
                ex.top,
                ex.right - ex.left,
                ex.bottom - ex.top,
                CombineModeExclude,
            )
            .0 != 0
        {
            return DrawResult::NotStarted;
        }
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
    let _guard = crate::runtime::lock(&DRAW_LOCK);
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

/// Keyboard-focus frame at two physical pixels: FrameRect alone stays 1px
/// at every DPI scale, which reads as a hairline on high-DPI screens
/// (WCAG 2.2 focus-appearance wants a clearly visible indicator).
pub fn draw_focus_frame(hdc: HDC, bounds: &RECT, color: u32) {
    frame_rect(hdc, bounds, color);
    let inner = RECT {
        left: bounds.left + 1,
        top: bounds.top + 1,
        right: bounds.right - 1,
        bottom: bounds.bottom - 1,
    };
    if inner.right > inner.left && inner.bottom > inner.top {
        frame_rect(hdc, &inner, color);
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
    ((logical as i64 * scale as i64 + 48) / 96) as i32
}

// ---- Control painters (Go nativeform/controls.go) --------------------------

use windows::Win32::Graphics::Gdi::{
    DT_CALCRECT, DT_CENTER, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK,
};

/// GDI fallback rounded shape: a solid fill with a 1px pen border. The
/// answer when the GDI+ path is unavailable (session start, GDI+ failure);
/// callers decide whether a dirty partial GDI+ fill must be cleared first.
fn round_rect_gdi(hdc: HDC, bounds: &RECT, fill: u32, border: u32, radius: i32) {
    unsafe {
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
    }
}

/// Rounded surface with a 1px border; falls back to GDI RoundRect when the
/// GDI+ path is unavailable (Go DrawSurface).
pub fn draw_surface(hdc: HDC, bounds: &RECT, background: u32, fill: u32, border: u32, radius: i32) {
    fill_rect(hdc, bounds, background);
    match fill_rounded_rect(hdc, bounds, radius, fill, border) {
        DrawResult::Completed => {}
        // Mid-fill failure may have written partial pixels: clear them, then
        // still draw the GDI fallback like Go instead of leaving a hole.
        DrawResult::MayBeDirty => {
            fill_rect(hdc, bounds, background);
            round_rect_gdi(hdc, bounds, fill, border, radius);
        }
        DrawResult::NotStarted => round_rect_gdi(hdc, bounds, fill, border, radius),
    }
}

/// Edit-field surface with focus/hover/disabled border states (Go DrawField).
/// `fill` is the field's resting interior: the raised surface on window
/// backgrounds, or the inset well color (window background) on card faces
/// where a same-color fill would erase the field down to its border.
pub fn draw_field(
    hdc: HDC,
    bounds: &RECT,
    p: &Palette,
    background: u32,
    fill: u32,
    state: ControlState,
    radius: i32,
    exclude: Option<&RECT>,
) {
    let (mut fill, mut border) = (fill, p.border);
    if state.disabled {
        fill = p.disabled_surface;
        border = p.subtle_border;
    } else if state.focused {
        border = p.focus;
    } else if state.hovered {
        border = p.accent;
    }
    // GDI side of the exclusion: fill_rect and the GDI fallback honor the
    // DC clip; the GDI+ rounded path gets its own SetClip below.
    if let Some(ex) = exclude {
        unsafe {
            let _ = ExcludeClipRect(hdc, ex.left, ex.top, ex.right, ex.bottom);
        }
    }
    fill_rect(hdc, bounds, background);
    match fill_rounded_rect_impl(hdc, bounds, radius, fill, border, exclude) {
        DrawResult::Completed => {}
        // Mid-fill failure may have written partial pixels: clear them, then
        // still draw the GDI fallback like Go instead of leaving a hole.
        DrawResult::MayBeDirty => {
            fill_rect(hdc, bounds, background);
            round_rect_gdi(hdc, bounds, fill, border, radius);
        }
        DrawResult::NotStarted => round_rect_gdi(hdc, bounds, fill, border, radius),
    }
}

/// Shared form corner radius in the same DPI/text scale as the controls.
/// Callers with a special shape (for example a circular info button) retain
/// their own radius. Drawing never changes the native control's bounds.
pub fn control_radius() -> i32 {
    crate::dpi::scale(6)
}

/// Fractional thumb position override for `draw_switch`: None snaps to the
/// state, Some(t) places the thumb animated between off (0) and on (1).
pub type SwitchProgress = Option<f32>;

/// Pill toggle switch (Win11-style) for label-left rows. Geometry derives
/// from the control's physical bounds and `scale`, never from integer
/// logical rounding, so the track and thumb stay smooth and correctly
/// proportioned at every per-monitor DPI. All colors come from palette
/// tokens, which also drives the high-contrast mapping.
pub fn draw_switch(
    hdc: HDC,
    bounds: &RECT,
    p: &Palette,
    background: u32,
    state: ControlState,
    scale: i32,
    progress: SwitchProgress,
) {
    fill_rect(hdc, bounds, background);
    let track_w = sp(40, scale);
    let track_h = sp(20, scale);
    let thumb_d = sp(16, scale);
    if track_w <= 0 || track_h <= 0 || thumb_d <= 0 {
        return;
    }
    let left = bounds.left + (bounds.right - bounds.left - track_w) / 2;
    let top = bounds.top + (bounds.bottom - bounds.top - track_h) / 2;
    let track = RECT {
        left,
        top,
        right: left + track_w,
        bottom: top + track_h,
    };
    // Thumb inset from the track edge and travel distance between states.
    // Pressing grows the thumb a touch around its own center (Win11 feel),
    // clamped so it never crosses the track border.
    let inset = ((track_h - thumb_d) / 2).max(1);
    let grow = if state.pressed && !state.disabled {
        sp(2, scale).min(inset.saturating_sub(1)).max(0)
    } else {
        0
    };
    let thumb_d = thumb_d + grow;
    let center_y = track.top + track_h / 2;
    // Animated runs place the thumb along the travel; the off/on ends stay
    // exactly where the snapped path puts them.
    let resting_center_x = match progress {
        Some(t) => {
            let t = t.clamp(0.0, 1.0);
            let left = track.left + inset + thumb_d / 2;
            let right = track.right - inset - thumb_d / 2;
            left + ((right - left) as f32 * t).round() as i32
        }
        None => {
            if state.active {
                track.right - inset - thumb_d / 2
            } else {
                track.left + inset + thumb_d / 2
            }
        }
    };
    let thumb = RECT {
        left: resting_center_x - thumb_d / 2,
        top: center_y - thumb_d / 2,
        right: resting_center_x + (thumb_d + 1) / 2,
        bottom: center_y + (thumb_d + 1) / 2,
    };
    // Track: solid pill in the state color. Hover must read at a glance:
    // the off track tints toward the accent, the on track steps to a
    // pressed-depth accent (a lighter step would wash out the white
    // thumb), and the off hover gains a 2px accent ring.
    let (fill, ring, ring_w) = if state.disabled {
        (p.disabled_surface, p.subtle_border, 1)
    } else if state.active {
        let color = if state.pressed {
            p.accent_pressed
        } else if state.hovered {
            p.switch_track_on_hover
        } else {
            p.accent
        };
        (color, color, 1)
    } else if state.pressed {
        (p.accent_pressed, p.accent_pressed, 1)
    } else if state.hovered {
        (p.switch_track_hover, p.accent_hover, 2)
    } else {
        (p.switch_track, p.switch_track, 1)
    };
    // Cards, buttons, chips, and segments share the 6px corner radius; the
    // switch keeps its fully-round control identity instead — a pill track
    // with a circular thumb (a rounded rect at half-size renders as a
    // bezier circle). draw_surface carries the GDI+ path plus the plain-GDI
    // fallback.
    let track_radius = (track_h / 2 - 1).max(1);
    draw_surface(hdc, &track, background, fill, ring, track_radius);
    if ring_w > 1 {
        // Second, inset fill draws the ring at 2 physical pixels.
        let inner = RECT {
            left: track.left + 1,
            top: track.top + 1,
            right: track.right - 1,
            bottom: track.bottom - 1,
        };
        if inner.right > inner.left && inner.bottom > inner.top {
            fill_rounded_rect(hdc, &inner, track_radius - 1, fill, fill);
        }
    }
    let ink = thumb_fill(p, state);
    // The thumb never pre-fills its bounding square: the square's corners
    // poke past the pill track's rounded ends and read as stray dots.
    // fill_rounded_rect paints only the shape; on GDI+ trouble the track
    // is redone and a GDI rounded shape takes over.
    let thumb_radius = (thumb_d / 2).max(1);
    match fill_rounded_rect(hdc, &thumb, thumb_radius, ink, ink) {
        DrawResult::Completed => {}
        DrawResult::MayBeDirty => {
            draw_surface(
                hdc,
                &track,
                background,
                fill,
                ring,
                (track_h / 2 - 1).max(1),
            );
            round_rect_gdi(hdc, &thumb, ink, ink, thumb_radius);
        }
        DrawResult::NotStarted => round_rect_gdi(hdc, &thumb, ink, ink, thumb_radius),
    }
    if state.focused && !state.disabled {
        let grow = sp(2, scale);
        let frame = RECT {
            left: track.left - grow,
            top: track.top - grow,
            right: track.right + grow,
            bottom: track.bottom + grow,
        };
        draw_focus_frame(hdc, &frame, p.focus);
    }
}

/// Thumb ink: light in both on/off states (Win11 style — the track color
/// change alone communicates the state); disabled uses the muted ink.
fn thumb_fill(p: &Palette, state: ControlState) -> u32 {
    if state.disabled {
        p.disabled_text
    } else {
        p.accent_text
    }
}

/// Plain row label for label-left rows (grouped form list style): body font,
/// primary text color, no surface of its own.
pub fn draw_row_label(
    hdc: HDC,
    bounds: &RECT,
    font: HFONT,
    label: &str,
    p: &Palette,
    background: u32,
) {
    fill_rect(hdc, bounds, background);
    draw_label(hdc, bounds, font, label, p.text, true, 0, 0);
}

/// Quiet navigation entry for page rails (settings nav): no button chrome.
/// The active page carries a small accent bar and heavier ink; hover chips
/// softly without gaining a border; focus draws the standard frame.
pub fn draw_nav_item(
    hdc: HDC,
    bounds: &RECT,
    font_active: HFONT,
    font_rest: HFONT,
    label: &str,
    p: &Palette,
    background: u32,
    state: ControlState,
    scale: i32,
) {
    fill_rect(hdc, bounds, background);
    let ink = if state.disabled {
        p.disabled_text
    } else if state.active {
        p.text
    } else if state.hovered || state.pressed {
        p.link
    } else {
        p.text2
    };
    // No hover chip: hover is a text-color change alone (link ink for
    // inactive entries), which sidesteps width measuring entirely.

    let bar_w = sp(3, scale);
    if state.active {
        let bar_h = sp(20, scale);
        let bar = RECT {
            left: bounds.left,
            top: bounds.top + (bounds.bottom - bounds.top - bar_h) / 2,
            right: bounds.left + bar_w,
            bottom: bounds.top + (bounds.bottom - bounds.top + bar_h) / 2,
        };
        match fill_rounded_rect(hdc, &bar, bar_w.max(1), p.accent, p.accent) {
            DrawResult::Completed | DrawResult::MayBeDirty => {}
            DrawResult::NotStarted => fill_rect(hdc, &bar, p.accent),
        }
    }
    let text = RECT {
        left: bounds.left + sp(14, scale),
        top: bounds.top,
        right: bounds.right - sp(4, scale),
        bottom: bounds.bottom,
    };
    let font = if state.active { font_active } else { font_rest };
    draw_label(hdc, &text, font, label, ink, true, 0, 0);
    if state.focused && !state.disabled {
        draw_focus_frame(hdc, bounds, p.focus);
    }
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

/// Danger push button, sharing the quiet-danger grammar with menu rows:
/// danger ink on the neutral surface at rest, committing to the filled
/// danger style only on hover/press. Destructive actions stay visually
/// calm until approached, then leave no doubt.
pub fn draw_button_danger(
    hdc: HDC,
    bounds: &RECT,
    font: HFONT,
    label: &str,
    p: &Palette,
    background: u32,
    state: ControlState,
    radius: i32,
) {
    let (mut fill, mut border, mut text) = (p.surface, p.subtle_border, p.danger_surface_text);
    if state.hovered {
        fill = p.danger_bg;
        border = p.danger_bg;
        text = p.danger_text;
    }
    if state.pressed {
        fill = p.danger_pressed;
        border = p.danger_pressed;
        text = p.danger_text;
    }
    if state.disabled {
        fill = p.disabled_surface;
        border = p.subtle_border;
        text = p.disabled_text;
    }
    if state.focused && !state.disabled {
        // Over the filled danger states the pale danger ink reads (it is
        // contrast-tested against danger_bg/pressed); at rest on the neutral
        // surface it would nearly vanish, so the standard focus ink takes
        // over (WCAG 2.2 focus-appearance — same rule as draw_exit_button).
        border = if state.hovered || state.pressed {
            p.danger_focus
        } else {
            p.focus
        };
    }
    draw_surface(hdc, bounds, background, fill, border, radius);
    draw_button_label(hdc, bounds, font, label, text, false, 10, 10);
}

/// Label-left row with a pill switch at the right edge — the panel's
/// toggle-row grammar for forms: the whole row is one hit target and the
/// pill carries the state. `background` is the surface the row sits on
/// (card face or window background).
pub fn draw_switch_row(
    hdc: HDC,
    bounds: &RECT,
    font: HFONT,
    label: &str,
    p: &Palette,
    background: u32,
    state: ControlState,
    scale: i32,
    progress: SwitchProgress,
) {
    fill_rect(hdc, bounds, background);
    let column = sp(crate::layout::SWITCH_HIT_W, scale);
    let pill = RECT {
        left: bounds.right - column,
        top: bounds.top,
        right: bounds.right,
        bottom: bounds.bottom,
    };
    let text = RECT {
        left: bounds.left,
        top: bounds.top,
        right: pill.left - sp(8, scale),
        bottom: bounds.bottom,
    };
    let ink = if state.disabled {
        p.disabled_text
    } else {
        p.text
    };
    unsafe {
        draw_row_text(hdc, &text, font, label, ink);
    }
    draw_switch(hdc, &pill, p, background, state, scale, progress);
}

/// Single-line, vertically centered, end-ellipsis label for compact rows:
/// long settings labels must truncate at the pill column instead of
/// wrapping underneath it.
unsafe fn draw_row_text(hdc: HDC, bounds: &RECT, font: HFONT, label: &str, color: u32) {
    if label.is_empty() {
        // Empty labels draw nothing (see draw_label for the AV reason).
        return;
    }
    unsafe {
        let mut text: Vec<u16> = label.encode_utf16().collect();
        SetTextColor(hdc, COLORREF(color));
        SetBkMode(hdc, TRANSPARENT);
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        let mut rect = *bounds;
        let _ = DrawTextW(
            hdc,
            &mut text,
            &mut rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        let _ = SelectObject(hdc, old);
    }
}

/// Ghost chip for secondary quick actions (preset strips): transparent fill,
/// hairline border, muted text at rest — one step below the primary buttons
/// in weight at every state, with an accent outline while armed.
///
/// `emphasis` marks an instant-action chip sharing a preset strip with
/// deferred actions: it uses the link ink so the one chip that acts
/// immediately reads as a verb, like the panel's action links.
pub fn draw_chip(
    hdc: HDC,
    bounds: &RECT,
    font: HFONT,
    label: &str,
    p: &Palette,
    background: u32,
    state: ControlState,
    radius: i32,
    emphasis: bool,
) {
    // Three chip languages, one visual per meaning:
    // - accent border + normal ink  -> armed preset (a state)
    // - link-colored ink            -> instant action (a verb, like links)
    // - subtle border + muted ink   -> ordinary preset
    let (mut fill, mut border, mut text) = (background, p.subtle_border, p.text2);
    if emphasis {
        text = p.link;
    }
    if state.active {
        border = p.accent;
    }
    // Hover swaps to the tinted fill and the normal ink: colored ink on the
    // hover tint does not separate well in dark mode. The fill and border
    // changes carry the hover state on their own.
    if state.hovered {
        fill = p.hover_surface;
        border = if state.active { p.accent } else { p.border };
        text = p.text;
    }
    if state.pressed {
        // Pressed steps one shade deeper than hover (the tint alone read as
        // a stuck hover); the quiet gray keeps both themes' ink legible.
        fill = p.subtle_border;
        border = if emphasis { p.accent_pressed } else { p.accent };
        text = p.text;
    }
    if state.disabled {
        border = background;
        text = p.disabled_text;
    }
    if state.focused && !state.disabled {
        border = p.focus;
    }
    draw_surface(hdc, bounds, background, fill, border, radius);
    draw_button_label(hdc, bounds, font, label, text, false, 8, 8);
}

/// Choice (combo) closed state with the drop arrow (Go DrawChoice).
/// Draws a choice button and its popup-state arrow.
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

/// Menu row for choice popups (Go DrawMenuOption).
/// Draws a selectable row in the custom choice popup.
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
    // Danger rows keep a quiet resting look (danger ink on the neutral
    // surface, like the panel's exit button) and commit to the filled
    // danger style only on hover/press.
    let (mut fill, mut border, mut text) = (p.surface, p.subtle_border, p.text);
    if danger {
        text = p.danger_surface_text;
        border = p.danger_border;
    }
    if state.hovered {
        if danger {
            fill = p.danger_bg;
            border = p.danger_bg;
            text = p.danger_text;
        } else {
            fill = p.hover_surface;
            border = p.border;
        }
    }
    if state.pressed {
        if danger {
            fill = p.danger_pressed;
            border = p.danger_pressed;
            text = p.danger_text;
        } else {
            fill = p.hover_surface;
            border = p.accent_pressed;
        }
    }
    if state.disabled {
        fill = p.disabled_surface;
        border = p.subtle_border;
        text = p.disabled_text;
    }
    if state.focused && !state.disabled {
        // Same quiet-danger focus rule as draw_button_danger: the pale
        // danger ring only reads over the filled hover/press states.
        border = if danger && (state.hovered || state.pressed) {
            p.danger_focus
        } else {
            p.focus
        };
    }
    draw_surface(hdc, bounds, background, fill, border, radius);
    let use_font = if selected && !selected_font.is_invalid() {
        selected_font
    } else {
        font
    };
    if selected {
        // Nav-rail selected grammar (paint::draw_nav_item): a slim
        // rounded accent bar, vertically centered - one selection
        // language across flyouts and the settings navigation. The bar
        // backs off the row's border by the nav's bar-to-text rhythm:
        // flyout rows carry card chrome the rail does not, so a bar on
        // the border line reads cramped.
        let bar_w = sp(3, scale);
        let bar_h = sp(20, scale);
        let bar_x = bounds.left + sp(4, scale);
        let marker = RECT {
            left: bar_x,
            top: bounds.top + (bounds.bottom - bounds.top - bar_h) / 2,
            right: bar_x + bar_w,
            bottom: bounds.top + (bounds.bottom - bounds.top + bar_h) / 2,
        };
        match fill_rounded_rect(hdc, &marker, bar_w.max(1), p.accent, p.accent) {
            DrawResult::Completed | DrawResult::MayBeDirty => {}
            DrawResult::NotStarted => fill_rect(hdc, &marker, p.accent),
        }
    }
    draw_label(
        hdc,
        bounds,
        use_font,
        label,
        text,
        true,
        sp(18, scale),
        sp(4, scale),
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
    let mut color = p.link;
    if state.hovered {
        color = p.link_hover;
    }
    if state.pressed {
        color = p.link_pressed;
    }
    if state.disabled {
        color = p.disabled_text;
    }
    if label.is_empty() {
        // Empty labels draw nothing (see draw_label for the AV reason).
        return;
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
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        let mut size = SIZE::default();
        // The underline appears only on approach: at rest the link reads
        // as colored text (Fluent link grammar).
        if (state.hovered || state.pressed)
            && GetTextExtentPoint32W(hdc, &text, &mut size).as_bool()
        {
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
    if state.focused && !state.disabled {
        draw_focus_frame(hdc, bounds, p.focus);
    }
}

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

pub fn draw_label(
    hdc: HDC,
    bounds: &RECT,
    font: HFONT,
    label: &str,
    color: u32,
    left: bool,
    left_inset: i32,
    right_inset: i32,
) {
    if label.is_empty() {
        // An empty Vec<u16>'s buffer pointer is the dangling aligned
        // pointer (0x2 for u16), and DrawTextW still probes the text
        // pointer - an access violation (the picker's blank description
        // column crashed here on first paint). Empty labels draw nothing.
        return;
    }
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
        let _ = DrawTextW(hdc, &mut text, &mut bounds, flags | DT_NOPREFIX);
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
    if label.is_empty() {
        // Empty labels draw nothing (see draw_label for the AV reason).
        return;
    }
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
        let has_measure = DrawTextW(
            hdc,
            &mut text,
            &mut measured,
            DT_WORDBREAK | DT_CALCRECT | DT_NOPREFIX,
        ) != 0;
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

        let _ = DrawTextW(
            hdc,
            &mut text,
            &mut bounds,
            DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
        );
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
