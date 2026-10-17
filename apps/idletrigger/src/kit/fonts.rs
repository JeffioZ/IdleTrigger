//! Role fonts for form windows. Every role rides the shared `dpi::font`
//! cache (via `make_font_pub`), so callers never hold their own caches
//! and a DPI rescale swaps fonts in one place.

use windows::Win32::Graphics::Gdi::HFONT;

/// Body text weight for labels, fields, and owner-draw controls.
pub fn body() -> HFONT {
    crate::make_font_pub(14, 400)
}

/// Compact body text for dense manager rows (ctx rules editor).
pub fn label() -> HFONT {
    crate::make_font_pub(13, 400)
}

/// Section group captions inside a page.
pub fn section() -> HFONT {
    crate::make_font_pub(14, 600)
}

/// Page header titles (settings header / manager titles).
pub fn page_title() -> HFONT {
    crate::make_font_pub(17, 600)
}
