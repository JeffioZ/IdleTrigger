//! Flow layout pass: the shared mechanics of a mode-driven layout run —
//! DPI scope, batched placement (SWP_NOREDRAW), deferred visibility —
//! lifted from the automation editor's hand-rolled pass. Windows keep
//! their flow descriptions (the y-cursor walks); this owns the machinery.
//! The DPI scope lives until the Flow is dropped: the caller's tail work
//! (window grow, viewport fit, present) must run under the same scope as
//! the placements, so `finish` borrows instead of consuming.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetDlgItem, SWP_NOACTIVATE, SWP_NOREDRAW, SWP_NOZORDER, SetWindowPos,
};

pub struct Flow {
    parent: HWND,
    _dpi: crate::dpi::Scope,
    placed: Vec<HWND>,
}

impl Flow {
    /// Enters the parent's DPI scope and marks the viewport layout begun.
    /// Window-specific surface lowering runs after this, before places.
    pub unsafe fn begin(parent: HWND) -> Flow {
        let dpi = crate::dpi::Scope::window(parent);
        crate::viewport::begin_layout(parent);
        Flow {
            parent,
            _dpi: dpi,
            placed: Vec::new(),
        }
    }

    unsafe fn position(&mut self, id: usize, x: i32, y: i32, w: i32, h: i32) {
        unsafe {
            let control = GetDlgItem(Some(self.parent), id as i32).unwrap_or_default();
            if control.is_invalid() {
                return;
            }
            let _ = SetWindowPos(
                control,
                None,
                crate::scale_pub(x),
                crate::scale_pub(y),
                crate::scale_pub(w),
                crate::scale_pub(h),
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
            );
            crate::dpi::remember_rect(control, (x, y, w, h));
            self.placed.push(control);
        }
    }

    /// Places a control at a logical rect.
    pub unsafe fn place(&mut self, id: usize, x: i32, y: i32, w: i32, h: i32) {
        unsafe { self.position(id, x, y, w, h) }
    }

    /// Places a field edit inset inside its card-painted well (settings
    /// parity: x+3, y+7, w-6, h-14, floored at 20).
    pub unsafe fn place_field(&mut self, id: usize, x: i32, y: i32, w: i32, h: i32) {
        let (x, y, w, h) = crate::kit::controls::edit_rect((x, y, w, h));
        unsafe { self.position(id, x, y, w, h) }
    }

    /// Applies deferred visibility: controls in `layout_ids` that were
    /// placed this pass become visible, the rest hide. Existing fields
    /// never hide and reappear within one pass (set_visible_deferred
    /// batches the swap into one present).
    pub unsafe fn finish(&mut self, layout_ids: &[usize]) {
        unsafe {
            for id in layout_ids {
                let control = GetDlgItem(Some(self.parent), *id as i32).unwrap_or_default();
                if control.is_invalid() {
                    continue;
                }
                crate::nativeform::set_visible_deferred(control, self.placed.contains(&control));
            }
        }
    }
}
