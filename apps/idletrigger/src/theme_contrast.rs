//! Immutable, process-lifetime palettes keyed by the user's system colors.
use crate::theme::Palette;
use std::sync::{
    LazyLock, Mutex,
    atomic::{AtomicPtr, Ordering},
};
static ACTIVE: AtomicPtr<Palette> = AtomicPtr::new(std::ptr::null_mut());
static CACHE: LazyLock<Mutex<std::collections::HashMap<[u32; 9], &'static Palette>>> =
    LazyLock::new(Default::default);
pub fn palette() -> Option<&'static Palette> {
    unsafe { ACTIVE.load(Ordering::SeqCst).as_ref() }
}
pub fn refresh() -> bool {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows::Win32::UI::WindowsAndMessaging::*;
    let next = unsafe {
        let mut state = HIGHCONTRASTW {
            cbSize: size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        if SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            state.cbSize,
            Some((&mut state as *mut HIGHCONTRASTW).cast()),
            Default::default(),
        )
        .is_ok()
            && state.dwFlags & HCF_HIGHCONTRASTON != Default::default()
        {
            let colors = [
                COLOR_WINDOW,
                COLOR_WINDOWTEXT,
                COLOR_HIGHLIGHT,
                COLOR_HIGHLIGHTTEXT,
                COLOR_BTNFACE,
                COLOR_GRAYTEXT,
                COLOR_WINDOWFRAME,
                COLOR_INFOBK,
                COLOR_INFOTEXT,
            ]
            .map(|index| GetSysColor(index));
            let mut cache = crate::runtime::lock(&CACHE);
            let value = cache
                .entry(colors)
                .or_insert_with(|| Box::leak(Box::new(from_colors(colors))));
            *value as *const Palette as *mut Palette
        } else {
            std::ptr::null_mut()
        }
    };
    ACTIVE.swap(next, Ordering::SeqCst) != next
}
fn from_colors(
    [
        bg,
        text,
        selected,
        selected_text,
        disabled_bg,
        disabled_text,
        frame,
        tip_bg,
        tip_text,
    ]: [u32; 9],
) -> Palette {
    Palette {
        window_bg: bg,
        surface: bg,
        elevated: bg,
        hover_surface: bg,
        border: text,
        subtle_border: frame,
        text,
        text2: text,
        muted: text,
        disabled_text,
        disabled_surface: disabled_bg,
        accent: selected,
        accent_hover: selected,
        accent_pressed: selected,
        selected,
        selected_hover: selected,
        accent_text: selected_text,
        link: text,
        link_hover: text,
        link_pressed: text,
        focus: text,
        danger_bg: selected,
        danger_hover: selected,
        danger_pressed: selected,
        danger_border: text,
        danger_hover_border: text,
        danger_pressed_border: text,
        danger_text: selected_text,
        danger_surface_text: text,
        danger_focus: selected_text,
        tooltip_bg: tip_bg,
        tooltip_text: tip_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hover_keeps_window_text_and_active_states_use_system_pairs() {
        let p = from_colors([1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!((p.hover_surface, p.text), (1, 2));
        assert_eq!((p.accent_pressed, p.accent_text), (3, 4));
        assert_eq!((p.disabled_surface, p.disabled_text), (5, 6));
        assert_eq!((p.tooltip_bg, p.tooltip_text), (8, 9));
        assert_eq!([p.link, p.link_hover, p.link_pressed], [2; 3]);
        assert_eq!((p.danger_hover, p.danger_focus), (3, 4));
    }
}
