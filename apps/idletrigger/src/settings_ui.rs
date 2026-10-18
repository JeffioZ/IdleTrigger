//! Settings window with five left navigation tabs, per-page content on the
//! right, and a footer with inline validation, Save, and Cancel.
//! Layout uses logical coordinates and adapts to DPI and text scaling.

use std::sync::atomic::{AtomicI32, AtomicIsize, AtomicU64, Ordering};

// Edit interior brushes (surface / disabled surface), live for the process.
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{HDC, HFONT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, IsWindowEnabled, SetFocus,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use crate::kit::controls::{label, push_button, tab_button};
use crate::settings_model::{Draft, draft_config, merge_settings};
use crate::settings_pages::*;
use crate::{t_pub, theme};

/// Appearance selection before opening an action footer (Browse/Install).
/// Cancelled actions must restore that value instead of saving "no change".
static APPEARANCE_PREVIOUS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<i32, String>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

// Owner-drawn checkbox states (BM_SETCHECK is inert on owner-draw buttons).
static CHECKS: std::sync::Mutex<Option<std::collections::HashMap<i32, bool>>> =
    std::sync::Mutex::new(None);

// Labels grayed through the palette tier instead of EnableWindow: a
// disabled STATIC renders with the system's disabled-static styling, which
// ignores our WM_CTLCOLORSTATIC text color and paints a bright GrayText in
// dark mode - reading as a washed layer over the card. An enabled static's
// colors are fully ours, so graying is a color-tier concern here.
static GRAYED_LABELS: std::sync::Mutex<Option<Vec<i32>>> = std::sync::Mutex::new(None);

fn grayed_labels() -> std::sync::MutexGuard<'static, Option<Vec<i32>>> {
    crate::runtime::lock(&GRAYED_LABELS)
}

fn is_grayed_label(id: i32) -> bool {
    grayed_labels()
        .as_ref()
        .is_some_and(|ids| ids.contains(&id))
}

/// Moves a label in or out of the palette-grayed set, repainting it when
/// membership changes.
fn set_label_grayed(hwnd: HWND, id: i32, grayed: bool) {
    let mut store = grayed_labels();
    let current = store.get_or_insert_with(Vec::new);
    let was = current.contains(&id);
    if was == grayed {
        return;
    }
    if grayed {
        current.push(id);
    } else {
        current.retain(|other| *other != id);
    }
    drop(store);
    unsafe {
        let control = get(hwnd, id);
        // Erase on repaint: without it the old text color can survive the
        // first invalidation at rounded corners.
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, true);
    }
}

fn checks() -> std::sync::MutexGuard<'static, Option<std::collections::HashMap<i32, bool>>> {
    crate::runtime::lock(&CHECKS)
}

static SETTINGS_HWND: AtomicIsize = AtomicIsize::new(0);
static RESTORE_REQUEST: AtomicU64 = AtomicU64::new(0);
static CLASSIC_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CLASSIC_RESULT: std::sync::Mutex<Option<Result<(), String>>> = std::sync::Mutex::new(None);
static CLASSIC_WORKER: std::sync::Mutex<Option<std::thread::JoinHandle<()>>> =
    std::sync::Mutex::new(None);

pub(crate) fn finish_classic_restart() {
    let worker = crate::runtime::lock(&CLASSIC_WORKER).take();
    if let Some(worker) = worker {
        crate::finish_worker_on_ui_thread(worker);
    }
    if let Some(Err(error)) = crate::runtime::lock(&CLASSIC_RESULT).take() {
        crate::log_line(&format!("Explorer restart failed during exit: {error}"));
    }
}

pub(crate) fn refresh_classic_restart() {
    let result = crate::runtime::lock(&CLASSIC_RESULT).take();
    let Some(result) = result else {
        return;
    };
    CLASSIC_BUSY.store(false, Ordering::SeqCst);
    crate::ctx_ui::refresh_config_state();
    if let Err(err) = result {
        crate::warn_dialog("", &err);
    }
}
static DRAFT_BASE: std::sync::Mutex<Option<idletrigger_core::config::Config>> =
    std::sync::Mutex::new(None);
static PAGE: AtomicI32 = AtomicI32::new(0);
/// Initial page is applied before creation to avoid stale first frames.
static INITIAL_PAGE: AtomicI32 = AtomicI32::new(0);
static AUTOSTART_BASE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static TOOLTIP_HWND: AtomicIsize = AtomicIsize::new(0);
/// Validation-line kind: true renders in the error color, false in normal
/// text (informational feedback such as a successful restore).
static VALIDATION_ERROR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Writes the shared validation line with its semantic kind.
fn set_validation(hwnd: HWND, text: &str, error: bool) {
    VALIDATION_ERROR.store(error, Ordering::SeqCst);
    set_text(hwnd, ID_VALIDATION, text);
}

use crate::wide;

fn current() -> HWND {
    HWND(SETTINGS_HWND.load(Ordering::SeqCst) as *mut _)
}

fn get(parent: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(parent), id).unwrap_or_default() }
}

fn set_text(parent: HWND, id: i32, text: &str) {
    unsafe {
        let target = get(parent, id);
        if !target.is_invalid() && control_text(parent, id) != text {
            let _ = SetWindowTextW(target, PCWSTR(wide(text).as_ptr()));
        }
    }
}

fn control_text(parent: HWND, id: i32) -> String {
    unsafe {
        let target = get(parent, id);
        if target.is_invalid() {
            return String::new();
        }
        let len = SendMessageW(target, WM_GETTEXTLENGTH, Some(WPARAM(0)), Some(LPARAM(0)))
            .0
            .max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let copied = SendMessageW(
            target,
            WM_GETTEXT,
            Some(WPARAM(buf.len())),
            Some(LPARAM(buf.as_mut_ptr() as isize)),
        )
        .0 as usize;
        String::from_utf16_lossy(&buf[..copied.min(len)])
    }
}

fn is_checked(_parent: HWND, id: i32) -> bool {
    checks()
        .as_ref()
        .and_then(|m| m.get(&id).copied())
        .unwrap_or(false)
}

/// Stores an owner-drawn checkbox state and repaints its control.
fn set_checked(parent: HWND, id: i32, value: bool) {
    checks()
        .get_or_insert_with(Default::default)
        .insert(id, value);
    unsafe {
        let control = get(parent, id);
        if !control.is_invalid() {
            crate::accessibility::check(control, value);
            // Erase before repaint: with erase=false, DPI-rounded corners can
            // retain stale pixels from the previous check state, making the
            // toggle appear to need two clicks.
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, true);
        }
    }
}

/// Toggles one owner-drawn checkbox.
fn toggle_checked(parent: HWND, id: i32) {
    let next = !is_checked(parent, id);
    set_checked(parent, id, next);
}

fn combo_sel(parent: HWND, id: i32) -> usize {
    crate::choice::selection(get(parent, id)).max(0) as usize
}

/// Open a settings destination before revealing its first frame.
pub fn show_appearance() {
    show_page(2);
}

/// Complete a panel enable request when no notification key has been selected.
pub fn configure_notification_keys() {
    show_page(3);
    let window = current();
    if window.is_invalid() {
        return;
    }
    let _dpi = crate::dpi::Scope::window(window);
    set_checked(window, ID_LOCK_KEYS, true);
    apply_dependent_states(window);
    set_validation(window, &t_pub("settings_error_notification_keys"), true);
    crate::present_layout(window);
    unsafe {
        let target = get(window, ID_LOCK_CAPS);
        let _ = SetFocus(Some(target));
        crate::viewport::reveal_control(window, target);
    }
}

fn show_page(page: i32) {
    let _dpi = crate::dpi::Scope::window(crate::hwnd(&crate::PANEL));
    if current().is_invalid() {
        INITIAL_PAGE.store(page, Ordering::SeqCst);
    } else {
        PAGE.store(page, Ordering::SeqCst);
        apply_dependent_states(current());
    }
    show();
}

pub fn show() {
    let _dpi = crate::dpi::Scope::window(crate::hwnd(&crate::PANEL));
    unsafe {
        if current().is_invalid() {
            create();
        }
        let hwnd = current();
        if hwnd.is_invalid() {
            return;
        }
        request_location_preview(hwnd);
        crate::hold_panel_modal(hwnd);
        // The radical fix for the invisible field wells: DWM's first
        // composition of this window's stacked owner-draw statics clips
        // the field surfaces, and no amount of invalidation repaints them.
        // Dragging the window off-screen and back rebuilds the redirection
        // surface - so do exactly that cycle programmatically, CLOAKED, so
        // the user never sees either state: show, present, hide, show,
        // present, then uncloak.
        let gate = crate::FirstFrameGate::begin(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);
        crate::present_frame(hwnd);
        let _ = ShowWindow(hwnd, SW_HIDE);
        let _ = ShowWindow(hwnd, SW_SHOW);
        crate::present_frame(hwnd);
        gate.reveal();
    }
}

/// Devtools capture and regression-test support: the settings window.
#[cfg(test)]
pub fn devtools_capture_hwnd() -> HWND {
    current()
}

pub fn default_button() -> HWND {
    get(current(), ID_SAVE)
}

/// Devtools capture support: switches the visible page.
#[cfg(feature = "devtools")]
pub fn devtools_select_page(page: i32) {
    let hwnd = current();
    if hwnd.is_invalid() || !(0..5).contains(&page) {
        return;
    }
    PAGE.store(page, Ordering::SeqCst);
    apply_dependent_states(hwnd);
    // Mirror the tab-click tail: visibility lands via SWP_NOREDRAW, so the
    // swap needs an explicit present before a capture reads the window DC.
    if unsafe { IsWindowVisible(hwnd).as_bool() } {
        crate::present_layout(hwnd);
    }
}

fn create() {
    unsafe {
        let body = crate::kit::fonts::body();
        let title_font = crate::kit::fonts::page_title();

        let form = crate::kit::form::FormWindow::create(crate::kit::form::FormSpec {
            class: windows::core::w!("IdleTriggerSettings"),
            kind: crate::kit::registry::Kind::Settings,
            title: t_pub("settings_title"),
            proc: Some(proc),
            client_w: CLIENT_W,
            client_h: CLIENT_H,
            style: crate::kit::form::secondary_style(),
            anchor: crate::hwnd(&crate::PANEL),
            bg_brush: theme::bg_brush(),
        });
        let hwnd = form.hwnd;
        SETTINGS_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        form.finish();

        build_controls(hwnd, body, title_font);
        populate(hwnd);
        PAGE.store(INITIAL_PAGE.swap(0, Ordering::SeqCst), Ordering::SeqCst);
        apply_dependent_states(hwnd);
        theme::retheme_children(hwnd);
        create_tooltip(hwnd);
        retheme_tooltip();
        // The power page is the tall one: size to it before the first
        // show, then force one full repaint - owner-draw statics stack
        // badly on the initial DWM frame (fields and card bottoms clip
        // until a full invalidate, which is what moving the window
        // off-screen and back used to fix by hand).
        let _ = windows::Win32::Graphics::Gdi::RedrawWindow(
            Some(hwnd),
            None,
            None,
            windows::Win32::Graphics::Gdi::RDW_INVALIDATE
                | windows::Win32::Graphics::Gdi::RDW_ERASE
                | windows::Win32::Graphics::Gdi::RDW_ALLCHILDREN
                | windows::Win32::Graphics::Gdi::RDW_FRAME,
        );
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
        crate::viewport::fit(hwnd);
    }
}

/// Re-themes the tooltip colors after a light/dark switch (Go ApplyTooltip).
pub(crate) fn retheme_tooltip() {
    unsafe {
        let tip = HWND(TOOLTIP_HWND.load(Ordering::SeqCst) as *mut _);
        if tip.is_invalid() {
            return;
        }
        theme::apply_control_theme(tip);
        let _ = SendMessageW(
            tip,
            windows::Win32::UI::Controls::TTM_SETTIPBKCOLOR,
            Some(WPARAM(theme::tooltip_bg_color() as usize)),
            Some(LPARAM(0)),
        );
        let _ = SendMessageW(
            tip,
            windows::Win32::UI::Controls::TTM_SETTIPTEXTCOLOR,
            Some(WPARAM(theme::tooltip_text_color() as usize)),
            Some(LPARAM(0)),
        );
    }
}

/// Full theme refresh: caption, control visual styles, tooltip, repaint
/// (Go applyTheme on WM_SETTINGCHANGE / WM_SYSCOLORCHANGE / WM_THEMECHANGED).
fn refresh_theme() {
    crate::request_theme_refresh();
}

unsafe fn build_controls(hwnd: HWND, font: HFONT, title_font: HFONT) {
    unsafe {
        // Header: the active page's nav label plus a one-line intro take the
        // old generic window-title/description spot; the visible pair swaps
        // with the page (see page_ids).
        for (title_id, sub_id, title_key, sub_key) in [
            (
                ID_PAGE_TITLE_POWER,
                ID_PAGE_SUB_POWER,
                "settings_tab_power",
                "settings_power_page_hint",
            ),
            (
                ID_PAGE_TITLE_THEME,
                ID_PAGE_SUB_THEME,
                "settings_tab_theme",
                "settings_theme_page_hint",
            ),
            (
                ID_PAGE_TITLE_APPEARANCE,
                ID_PAGE_SUB_APPEARANCE,
                "settings_tab_appearance",
                "settings_appearance_page_hint",
            ),
            (
                ID_PAGE_TITLE_NOTIFICATIONS,
                ID_PAGE_SUB_NOTIFICATIONS,
                "settings_tab_notifications",
                "settings_notifications_page_hint",
            ),
            (
                ID_PAGE_TITLE_APP,
                ID_PAGE_SUB_APP,
                "settings_tab_app",
                "settings_app_page_hint",
            ),
        ] {
            label(
                hwnd,
                title_id,
                &t_pub(title_key),
                title_font,
                (24, 16, 460, 24),
                false,
            );
            label(
                hwnd,
                sub_id,
                &t_pub(sub_key),
                font,
                (24, 42, 608, 20),
                false,
            );
        }
        // Header right edge: the version's right side rides CONTENT_RIGHT
        // (24-px page inset) on the page-title row (both bands end at y=40);
        // right-aligned so the longer "x (arch)" form grows leftward.
        label(
            hwnd,
            ID_VERSION,
            &crate::t_args("settings_version", &[&crate::version_with_arch()]),
            font,
            (484, 18, 168, 22),
            true,
        );

        // Left navigation tabs: ink-only column, 128×36 at x=20, 44px pitch.
        tab_button(
            hwnd,
            ID_TAB_POWER,
            &t_pub("settings_tab_power"),
            (20, 90, 128, BTN_H),
        );
        tab_button(
            hwnd,
            ID_TAB_THEME,
            &t_pub("settings_tab_theme"),
            (20, 134, 128, BTN_H),
        );
        tab_button(
            hwnd,
            ID_TAB_APPEARANCE,
            &t_pub("settings_tab_appearance"),
            (20, 178, 128, BTN_H),
        );
        tab_button(
            hwnd,
            ID_TAB_NOTIFICATIONS,
            &t_pub("settings_tab_notifications"),
            (20, 222, 128, BTN_H),
        );
        tab_button(
            hwnd,
            ID_TAB_APP,
            &t_pub("settings_tab_app"),
            (20, 266, 128, BTN_H),
        );

        // Pages ride the panel card grammar: the section title sits outside
        // above its card, rows live one CARD_PAD inside the card face, and
        // cards separate by CARD_GAP instead of bare whitespace.

        // Power and idle page.
        crate::kit::spec::apply(hwnd, &power_page_spec());

        // Day/night page (language-dependent geometry rides the relayout
        // spec flavor).
        crate::kit::spec::apply_relayout(hwnd, &theme_page_spec());

        // Appearance page: the wallpaper library dropdowns refresh by
        // value in their own path; the spec owns creation and captions.
        crate::kit::spec::apply(hwnd, &appearance_page_spec());

        // Application page: the project-home row is measured text, so the
        // page rides the relayout flavor; the update-check button's phase
        // caption is create-only.
        crate::kit::spec::apply_relayout(hwnd, &app_page_spec(hwnd));

        // Screen notifications page: the hint is one line in Chinese and
        // two in English, which grows the behavior card and moves the
        // preview button (relayout flavor).
        crate::kit::spec::apply_relayout(hwnd, &notifications_page_spec());
        // Footer: single-line validation above the buttons; long bilingual
        // messages ellipsize rather than claiming a second line.
        label(
            hwnd,
            ID_VALIDATION,
            "",
            font,
            (CONTENT_X, FOOTER_Y - VALIDATION_H - 6, 468, VALIDATION_H),
            false,
        );
        // Single-line now: long bilingual messages ellipsize instead of
        // needing a second line.
        {
            let control = get(hwnd, ID_VALIDATION);
            let _ = SetWindowLongPtrW(
                control,
                GWL_STYLE,
                (GetWindowLongPtrW(control, GWL_STYLE) | 0x00004000) as _, // SS_ENDELLIPSIS
            );
        }
        push_button(
            hwnd,
            ID_SAVE,
            &t_pub("common_save"),
            (CONTENT_RIGHT - DIALOG_BTN_W, FOOTER_Y, DIALOG_BTN_W, BTN_H),
        );
        push_button(
            hwnd,
            ID_CANCEL,
            &t_pub("common_cancel"),
            (
                CONTENT_RIGHT - 2 * DIALOG_BTN_W - 8,
                FOOTER_Y,
                DIALOG_BTN_W,
                BTN_H,
            ),
        );
    }
}

/// Whether a static sits inside a section card face: labels on cards must
/// erase with the card color or their text background punches a
/// window-colored patch into the card. Geometric detection keeps working
/// as controls move between cards without an id list to maintain.
unsafe fn sits_on_card(hwnd: HWND, child: HWND) -> bool {
    unsafe {
        let mut control = RECT::default();
        if GetWindowRect(child, &mut control).is_err() {
            return false;
        }
        for id in CARD_BASE..=CARD_BASE + 9 {
            let card = get(hwnd, id);
            if card.is_invalid() {
                continue;
            }
            // Hidden pages' cards still carry rects; without the
            // visibility check their geometry claims other pages'
            // first-section titles and paints them with the card face.
            if !IsWindowVisible(card).as_bool() {
                continue;
            }
            let mut rect = RECT::default();
            if GetWindowRect(card, &mut rect).is_ok()
                && control.left >= rect.left
                && control.right <= rect.right
                && control.top >= rect.top
                && control.bottom <= rect.bottom
            {
                return true;
            }
        }
        false
    }
}

/// Back color + erase brush for a child control: the card-face brush when
/// the child sits inside a section card, the window background otherwise.
/// Shared by every WM_CTLCOLOR* arm.
unsafe fn child_back_brush(hwnd: HWND, child: HWND) -> (u32, isize) {
    unsafe {
        if sits_on_card(hwnd, child) {
            let (light, dark) = theme::surface_brush_pairs();
            let pair = if theme::is_dark() { dark } else { light };
            (theme::palette().surface, pair.0.0 as isize)
        } else {
            (theme::bg_color(), theme::bg_brush().0 as isize)
        }
    }
}

/// Invalidates one field's well on its hosting card. Scoped, never the
/// whole card: a full-card repaint paints over the shared window surface
/// and erases the pixels of every sibling sitting on the card. The well
/// geometry itself lives in nativeform (shared with the editor and picker).
fn invalidate_well(hwnd: HWND, edit_id: i32) {
    let card = match edit_id {
        ID_BATTERY_THRESH => CARD_BASE,
        ID_IDLE_TIMEOUT | ID_WARNING_SECONDS => CARD_BASE + 1,
        ID_LIGHT_TIME | ID_DARK_TIME => CARD_BASE + 2,
        _ => return,
    };
    crate::nativeform::invalidate_field_well(get(hwnd, card), get(hwnd, edit_id));
}

/// Edits hosted on each section card (by card control id): the card paints
/// their wells.
fn card_field_ids(card_id: i32) -> &'static [i32] {
    match card_id - CARD_BASE {
        0 => &[ID_BATTERY_THRESH],
        1 => &[ID_IDLE_TIMEOUT, ID_WARNING_SECONDS],
        2 => &[ID_LIGHT_TIME, ID_DARK_TIME],
        _ => &[],
    }
}

/// Records a wallpaper as recently used: deduplicated, most recent last,
/// capped so the dropdown stays scannable.
fn remember_wallpaper(path: &str) {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return;
    }
    let mut library = crate::runtime::lock(&WALLPAPER_LIBRARY);
    library.retain(|p| !p.eq_ignore_ascii_case(trimmed));
    library.push(trimmed.to_string());
    let len = library.len();
    if len > 8 {
        library.drain(..len - 8);
    }
}

/// Replaces the rows of one choice, keeping the selection by value.
fn refresh_choice_rows(hwnd: HWND, id: i32, rows: &[crate::choice::ChoiceItem]) {
    let selected = crate::choice::value(get(hwnd, id));
    let index = rows
        .iter()
        .position(|r| !r.header && r.value == selected)
        .map(|i| i as i32)
        .unwrap_or(0);
    crate::choice::set_rows(get(hwnd, id), rows);
    crate::choice::select_index(get(hwnd, id), index);
}

/// Removes one wallpaper from the recent list; any side still pointing at
/// it falls back to No change.
fn remove_wallpaper(hwnd: HWND, path: &str) {
    crate::runtime::lock(&WALLPAPER_LIBRARY).retain(|p| !p.eq_ignore_ascii_case(path));
    for id in [ID_LIGHT_WALL, ID_DARK_WALL] {
        if crate::choice::value(get(hwnd, id)).eq_ignore_ascii_case(path) {
            crate::choice::select_index(get(hwnd, id), 0);
        }
    }
    refresh_wallpaper_choices(hwnd);
}

/// Reloads both cursor dropdowns, keeping current selections by value.
fn refresh_cursor_choices(hwnd: HWND) {
    for id in [ID_LIGHT_CURSOR, ID_DARK_CURSOR] {
        let current = crate::choice::value(get(hwnd, id));
        refresh_choice_rows(hwnd, id, &cursor_pick_items(&current));
    }
}

/// Reloads both wallpaper dropdowns from the recent list.
fn refresh_wallpaper_choices(hwnd: HWND) {
    for id in [ID_LIGHT_WALL, ID_DARK_WALL] {
        let current = crate::choice::value(get(hwnd, id));
        let picks = wallpaper_pick_items(&current);
        refresh_choice_rows(hwnd, id, &picks);
    }
}

/// The Browse footer row was picked on one side: open the picker, remember
/// the file, and select it on that side.
fn browse_wallpaper_for_side(hwnd: HWND, id: i32) {
    let Some(path) = browse_wallpaper_file(hwnd) else {
        let previous = crate::runtime::lock(&APPEARANCE_PREVIOUS)
            .get(&id)
            .cloned()
            .unwrap_or_default();
        let rows = wallpaper_pick_items(&previous);
        let index = rows
            .iter()
            .position(|row| row.value == previous)
            .unwrap_or(0);
        crate::choice::set_rows(get(hwnd, id), &rows);
        crate::choice::select_index(get(hwnd, id), index as i32);
        return;
    };
    remember_wallpaper(&path);
    // Reinstall this side's rows built around the new value: they carry the
    // Remove action for the freshly picked image, which a generic refresh
    // (built from a no-change selection) does not include.
    let rows = wallpaper_pick_items(&path);
    let index = rows
        .iter()
        .position(|r| r.value.eq_ignore_ascii_case(&path))
        .map(|i| i as i32)
        .unwrap_or(0);
    crate::choice::select_index(get(hwnd, id), 0);
    crate::choice::set_rows(get(hwnd, id), &rows);
    crate::choice::select_index(get(hwnd, id), index);
    // The other side's rows change too (the recent list grew).
    let other = if id == ID_LIGHT_WALL {
        ID_DARK_WALL
    } else {
        ID_LIGHT_WALL
    };
    let other_current = crate::choice::value(get(hwnd, other));
    let other_rows = wallpaper_pick_items(&other_current);
    refresh_choice_rows(hwnd, other, &other_rows);
}

/// Installs a pointer-scheme .inf through the system installer (the
/// context-menu "Install" verb); we never parse or execute inf content
/// ourselves.
fn install_cursor_inf(owner: HWND) {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OPENFILENAMEW};
    unsafe {
        let mut filter: Vec<u16> = t_pub("settings_inf_filter").encode_utf16().collect();
        filter.push(0);
        filter.extend("*.inf".encode_utf16());
        filter.push(0);
        filter.push(0);
        let mut file = vec![0u16; 32768];
        let title = wide(&t_pub("settings_inf_browse_title"));
        let mut dialog = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: windows::core::PCWSTR(filter.as_ptr()),
            nFilterIndex: 1,
            lpstrFile: windows::core::PWSTR(file.as_mut_ptr()),
            nMaxFile: file.len() as u32,
            lpstrTitle: windows::core::PCWSTR(title.as_ptr()),
            Flags: windows::Win32::UI::Controls::Dialogs::OPEN_FILENAME_FLAGS(
                0x0000_0004 // OFN_HIDEREADONLY
                    | 0x0000_0008 // OFN_NOCHANGEDIR
                    | 0x0000_0800 // OFN_PATHMUSTEXIST
                    | 0x0000_1000 // OFN_FILEMUSTEXIST
                    | 0x0008_0000 // OFN_EXPLORER
                    | 0x0200_0000, // OFN_DONTADDTORECENT
            ),
            ..Default::default()
        };
        if !GetOpenFileNameW(&mut dialog).as_bool() {
            return;
        }
        let end = file.iter().position(|c| *c == 0).unwrap_or(file.len());
        let path = String::from_utf16_lossy(&file[..end]);
        // The system handles elevation prompts and file copy itself.
        let wide_path = wide(&path);
        let _ = windows::Win32::UI::Shell::ShellExecuteW(
            Some(owner),
            windows::core::w!("install"),
            PCWSTR(wide_path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// The Install footer row was picked on one side: run the .inf installer,
/// then restore the selection that was in place before the dropdown opened
/// — the action itself is not a scheme. The install verb is asynchronous,
/// so a freshly installed scheme shows up when the dropdown next opens (its
/// rows refresh right before the popup appears).
fn run_cursor_install(hwnd: HWND, id: i32) {
    let previous = crate::runtime::lock(&APPEARANCE_PREVIOUS)
        .get(&id)
        .cloned()
        .unwrap_or_default();
    install_cursor_inf(hwnd);
    refresh_cursor_choices(hwnd);
    let rows = cursor_pick_items(&previous);
    let index = rows
        .iter()
        .position(|r| !r.header && r.value == previous)
        .map_or(0, |i| i as i32);
    crate::choice::set_rows(get(hwnd, id), &rows);
    crate::choice::select_index(get(hwnd, id), index);
}

/// Wallpaper picker: the formats Windows accepts as desktop backgrounds,
/// plus an all-files fallback. Returns the chosen path.
fn browse_wallpaper_file(owner: HWND) -> Option<String> {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OPENFILENAMEW};
    unsafe {
        let mut filter: Vec<u16> = t_pub("settings_wallpaper_filter").encode_utf16().collect();
        filter.push(0);
        filter.extend("*.jpg;*.jpeg;*.png;*.bmp;*.gif;*.tif;*.tiff;*.webp;*.heic".encode_utf16());
        filter.push(0);
        let all = t_pub("settings_all_files");
        filter.extend(all.encode_utf16());
        filter.push(0);
        filter.extend("*.*".encode_utf16());
        filter.push(0);
        filter.push(0);
        let mut file = vec![0u16; 32768];
        let title = wide(&t_pub("settings_wallpaper_browse_title"));
        let mut dialog = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: windows::core::PCWSTR(filter.as_ptr()),
            nFilterIndex: 1,
            lpstrFile: windows::core::PWSTR(file.as_mut_ptr()),
            nMaxFile: file.len() as u32,
            lpstrTitle: windows::core::PCWSTR(title.as_ptr()),
            Flags: windows::Win32::UI::Controls::Dialogs::OPEN_FILENAME_FLAGS(
                0x0000_0004 // OFN_HIDEREADONLY
                    | 0x0000_0008 // OFN_NOCHANGEDIR
                    | 0x0000_0800 // OFN_PATHMUSTEXIST
                    | 0x0000_1000 // OFN_FILEMUSTEXIST
                    | 0x0008_0000 // OFN_EXPLORER
                    | 0x0200_0000, // OFN_DONTADDTORECENT
            ),
            ..Default::default()
        };
        if !GetOpenFileNameW(&mut dialog).as_bool() {
            return None;
        }
        let end = file.iter().position(|c| *c == 0).unwrap_or(file.len());
        Some(String::from_utf16_lossy(&file[..end]))
    }
}

fn populate(hwnd: HWND) {
    let (
        keep_screen,
        battery_allowed,
        pause_on_lock,
        battery_threshold,
        idle_timeout,
        idle_action,
        warning,
        idle_enhanced,
        theme_mode,
        light_time,
        dark_time,
        ip_enabled,
        theme_battery,
        theme_fullscreen,
        light_wallpaper,
        dark_wallpaper,
        light_cursor,
        dark_cursor,
        language,
        lock_keys,
        caps,
        num,
        scroll,
        lock_fullscreen,
        hotkeys,
        autostart,
        logging,
        update_auto,
    ) = crate::cfg_map(|c| {
        *crate::runtime::lock(&DRAFT_BASE) = Some(c.clone());
        (
            c.keep_screen_on,
            c.nosleep_on_battery,
            c.nosleep_pause_on_lock,
            c.nosleep_battery_threshold,
            c.idle_timeout_minutes,
            c.idle_action.clone(),
            c.idle_warning_seconds,
            c.idle_enhanced_monitor,
            c.theme_mode.clone(),
            c.theme_light_time.clone(),
            c.theme_dark_time.clone(),
            c.theme_ip_location_enabled,
            c.theme_dark_on_battery,
            c.theme_skip_fullscreen,
            c.theme_light_wallpaper.clone(),
            c.theme_dark_wallpaper.clone(),
            c.theme_light_cursor_scheme.clone(),
            c.theme_dark_cursor_scheme.clone(),
            c.language.clone(),
            c.lock_keys_enabled,
            c.lock_keys_caps_enabled,
            c.lock_keys_num_enabled,
            c.lock_keys_scroll_enabled,
            c.lock_keys_skip_fullscreen,
            c.hotkeys_enabled,
            crate::system::autostart_is_enabled(),
            c.logging_enabled,
            c.update_check_enabled,
        )
    });

    AUTOSTART_BASE.store(autostart, Ordering::SeqCst);
    for (id, value) in [
        (ID_KEEP_SCREEN, keep_screen),
        (ID_BATTERY_ALLOWED, battery_allowed),
        (ID_PAUSE_ON_LOCK, pause_on_lock),
        (ID_IDLE_ENHANCED, idle_enhanced),
        (ID_THEME_BATTERY, theme_battery),
        (ID_THEME_FULLSCREEN, theme_fullscreen),
        (ID_LOCK_KEYS, lock_keys),
        (ID_LOCK_CAPS, caps),
        (ID_LOCK_NUM, num),
        (ID_LOCK_SCROLL, scroll),
        (ID_LOCK_FULLSCREEN, lock_fullscreen),
        (ID_HOTKEYS, hotkeys),
        (ID_AUTOSTART, autostart),
        (ID_LOGGING, logging),
        (ID_APP_UPDATE_AUTO, update_auto),
    ] {
        set_checked(hwnd, id, value);
    }

    set_text(hwnd, ID_BATTERY_THRESH, &battery_threshold.to_string());
    set_text(hwnd, ID_IDLE_TIMEOUT, &idle_timeout.to_string());
    set_text(hwnd, ID_WARNING_SECONDS, &warning.to_string());
    set_text(hwnd, ID_LIGHT_TIME, &light_time);
    set_text(hwnd, ID_DARK_TIME, &dark_time);
    *crate::runtime::lock(&WALLPAPER_LIBRARY) = crate::cfg_map(|c| c.theme_wallpapers.clone());
    refresh_wallpaper_choices(hwnd);
    for (id, path) in [
        (ID_LIGHT_WALL, &light_wallpaper),
        (ID_DARK_WALL, &dark_wallpaper),
    ] {
        let rows = wallpaper_pick_items(path);
        let index = rows
            .iter()
            .position(|r| r.value == *path)
            .map(|i| i as i32)
            .unwrap_or(0);
        crate::choice::set_rows(get(hwnd, id), &rows);
        crate::choice::select_index(get(hwnd, id), index);
    }
    for (id, scheme) in [
        (ID_LIGHT_CURSOR, &light_cursor),
        (ID_DARK_CURSOR, &dark_cursor),
    ] {
        let rows = cursor_pick_items(scheme);
        let index = rows
            .iter()
            .position(|r| r.value == *scheme)
            .map_or(0, |i| i as i32);
        crate::choice::set_rows(get(hwnd, id), &rows);
        crate::choice::select_index(get(hwnd, id), index);
    }

    let mode_idx = if theme_mode == "sunrise" { 1 } else { 0 };
    crate::choice::select_index(get(hwnd, ID_THEME_MODE), mode_idx);
    crate::choice::select_index(
        get(hwnd, ID_LOCATION_SOURCE),
        if ip_enabled { 1 } else { 0 },
    );
    let action_idx = IDLE_ACTIONS
        .iter()
        .position(|a| *a == idle_action)
        .unwrap_or(0);
    crate::choice::select_index(get(hwnd, ID_IDLE_ACTION), action_idx as i32);
    let lang_idx = match language.as_str() {
        "en" => 1,
        "zh-CN" => 2,
        _ => 0,
    };
    crate::choice::select_index(get(hwnd, ID_LANGUAGE), lang_idx);

    set_text(
        hwnd,
        ID_THEME_LOCATION_STATUS,
        &location_status_text(ip_enabled),
    );
    refresh_update_status();
}

/// Settings-side manual check button: same one-click flow as the panel link.
/// Idle starts a (manual) check; a known update opens the shared confirm;
/// in-flight phases ignore clicks.
fn on_update_check_button() {
    match crate::selfupdate::phase() {
        crate::selfupdate::Phase::Checking | crate::selfupdate::Phase::Downloading { .. } => {}
        crate::selfupdate::Phase::Available(_) | crate::selfupdate::Phase::Ready(_) => {
            crate::update_prompt_and_begin();
        }
        _ => crate::selfupdate::manual_check_prompting(),
    }
    refresh_update_status();
}

/// Keep unrelated Settings edits valid after a live context-menu change.
pub(crate) fn sync_ctx_options() {
    let config = crate::cfg_map(Clone::clone);
    if let Some(base) = crate::runtime::lock(&DRAFT_BASE).as_mut() {
        base.ctx_menu_enabled = config.ctx_menu_enabled;
        base.ctx_copy_enabled = config.ctx_copy_enabled;
        base.ctx_menu_grouped = config.ctx_menu_grouped;
        base.ctx_copy_quote = config.ctx_copy_quote;
        base.ctx_copy_separator = config.ctx_copy_separator;
        base.ctx_win11_classic = config.ctx_win11_classic;
    }
}

pub(crate) fn classic_busy() -> bool {
    CLASSIC_BUSY.load(Ordering::SeqCst)
}

/// One-line coexistence warning naming detected third-party menu managers;
/// empty when none was found (no notice is shown at all then).
pub(crate) fn conflict_text() -> String {
    let hits = crate::ctxmenu::detect_conflicts();
    if hits.is_empty() {
        return String::new();
    }
    let names = hits
        .iter()
        .map(|hit| match hit.as_str() {
            "nilesoft" => t_pub("ctx_conflict_nilesoft"),
            "openxx" => t_pub("ctx_conflict_openxx"),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    crate::t_args("ctx_conflict_warning", &[&names])
}

/// Instant action (not staged): the rollback switch applies or reverts the
/// Windows 11 classic menu after an explicit confirmation, then restarts
/// Explorer. The switch mirrors the registry, so it re-reads the state
/// afterwards — a declined confirm snaps back visually.
pub(crate) fn on_win11_classic_switch(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONWARNING, MB_YESNO, MessageBoxW};
    if CLASSIC_BUSY.load(Ordering::SeqCst) || !crate::system::is_windows_11() {
        return;
    }
    let state = crate::ctxmenu::classic_state();
    if state == crate::ctxmenu::ClassicState::Unavailable {
        crate::warn_dialog("", &t_pub("ctx_registry_failed"));
        sync_ctx_options();
        crate::ctx_ui::refresh_config_state();
        return;
    }
    let enable = state == crate::ctxmenu::ClassicState::Off;
    let prompt = if enable {
        t_pub("ctx_win11_confirm_on")
    } else {
        t_pub("ctx_win11_confirm_off")
    };
    let answer = unsafe {
        MessageBoxW(
            Some(hwnd),
            PCWSTR(crate::wide(&prompt).as_ptr()),
            PCWSTR(crate::wide(&t_pub("settings_tab_ctx")).as_ptr()),
            MB_YESNO | MB_ICONWARNING | windows::Win32::UI::WindowsAndMessaging::MB_DEFBUTTON2,
        )
    };
    if answer != IDYES {
        sync_ctx_options();
        crate::ctx_ui::refresh_config_state();
        return;
    }
    if let Err(err) = crate::ctxmenu::set_classic(enable) {
        crate::warn_dialog("", &crate::t_args("ctx_classic_failed", &[&err]));
        sync_ctx_options();
        crate::ctx_ui::refresh_config_state();
        return;
    }
    if let Err(err) = crate::edit_config(|c| c.ctx_win11_classic = enable) {
        // This is an action record, not a startup self-heal preference.
        crate::log_line(&format!("ctx win11 flag save failed: {err}"));
    }
    if let Some(base) = crate::runtime::lock(&DRAFT_BASE).as_mut() {
        base.ctx_win11_classic = crate::cfg_map(|c| c.ctx_win11_classic);
    }
    CLASSIC_BUSY.store(true, Ordering::SeqCst);
    let worker = std::thread::Builder::new()
        .name("explorer-restart".into())
        .spawn(|| {
            let result = std::panic::catch_unwind(crate::ctxmenu::restart_explorer)
                .unwrap_or_else(|_| Err(t_pub("ctx_restart_failed")));
            *crate::runtime::lock(&CLASSIC_RESULT) = Some(result);
            unsafe {
                let _ = PostMessageW(
                    Some(crate::hwnd(&crate::HIDDEN)),
                    crate::WM_REFRESH_UI,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        });
    match worker {
        Ok(worker) => *crate::runtime::lock(&CLASSIC_WORKER) = Some(worker),
        Err(_) => {
            CLASSIC_BUSY.store(false, Ordering::SeqCst);
            crate::warn_dialog("", &t_pub("ctx_restart_failed"));
        }
    }
    sync_ctx_options();
    crate::ctx_ui::refresh_config_state();
}

/// Runtime refresh hook (WM_REFRESH_UI + language refresh): the check
/// button's caption follows the updater phase; no-op when closed.
pub fn refresh_update_status() {
    let hwnd = current();
    if hwnd.is_invalid() {
        return;
    }
    set_text(hwnd, ID_APP_UPDATE_CHECK, &crate::update_action_caption());
    unsafe {
        let control = get(hwnd, ID_APP_UPDATE_CHECK);
        if !control.is_invalid() {
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, false);
        }
    }
}

/// Go theme_location.go status line for the day/night page.
fn location_status_text(ip_enabled: bool) -> String {
    if ip_enabled {
        let key = match crate::iplocate::status() {
            crate::iplocate::Status::Resolved(label) => {
                return crate::t_args("settings_location_ip_resolved", &[&label]);
            }
            crate::iplocate::Status::Querying => "settings_location_ip_pending",
            crate::iplocate::Status::Failed => "settings_location_ip_failed",
            crate::iplocate::Status::NotRequested => "settings_location_ip_not_requested",
        };
        return crate::t_args(key, &[&t_pub(crate::theme_engine::location(false).2)]);
    }
    crate::t_args(
        "settings_location_auto_status",
        &[&t_pub(crate::theme_engine::location(false).2)],
    )
}

/// Preview the selected source without saving the settings draft.
fn request_location_preview(hwnd: HWND) {
    if combo_sel(hwnd, ID_LOCATION_SOURCE) == 1 {
        crate::iplocate::request();
    }
    refresh_location_status();
}

/// Refresh only the runtime label; never repopulate an open settings draft.
pub fn refresh_location_status() {
    let hwnd = current();
    if !hwnd.is_invalid() {
        set_text(
            hwnd,
            ID_THEME_LOCATION_STATUS,
            &location_status_text(combo_sel(hwnd, ID_LOCATION_SOURCE) == 1),
        );
    }
}

/// Page membership — Go pageControlIDs().
fn page_ids(page: i32) -> &'static [i32] {
    match page {
        0 => &[
            ID_PAGE_TITLE_POWER,
            ID_PAGE_SUB_POWER,
            CARD_BASE,
            CARD_BASE + 1,
            ID_POWER_TITLE,
            ID_KEEP_SCREEN,
            ID_BATTERY_ALLOWED,
            ID_PAUSE_ON_LOCK,
            ID_BATTERY_LBL,
            ID_BATTERY_THRESH,
            ID_IDLE_TITLE,
            ID_IDLE_TIMEOUT_LBL,
            ID_IDLE_TIMEOUT,
            ID_IDLE_ACTION_LBL,
            ID_IDLE_ACTION,
            ID_WARNING_LBL,
            ID_WARNING_SECONDS,
            ID_IDLE_ENHANCED,
        ],
        1 => &[
            ID_PAGE_TITLE_THEME,
            ID_PAGE_SUB_THEME,
            CARD_BASE + 2,
            CARD_BASE + 3,
            ID_THEME_SCHEDULE_TITLE,
            ID_THEME_BEHAVIOR_TITLE,
            ID_THEME_MODE_LBL,
            ID_THEME_MODE,
            ID_LIGHT_TIME_LBL,
            ID_LIGHT_TIME,
            ID_DARK_TIME_LBL,
            ID_DARK_TIME,
            ID_LOCATION_LBL,
            ID_LOCATION_SOURCE,
            ID_THEME_LOCATION_STATUS,
            ID_THEME_BATTERY,
            ID_THEME_FULLSCREEN,
            ID_THEME_HINT,
        ],

        2 => &[
            ID_PAGE_TITLE_APPEARANCE,
            ID_PAGE_SUB_APPEARANCE,
            CARD_BASE + 4,
            ID_COL_LIGHT,
            ID_COL_DARK,
            ID_ROW_WALL_LBL,
            ID_LIGHT_WALL,
            ID_DARK_WALL,
            ID_ROW_CURSOR_LBL,
            ID_LIGHT_CURSOR,
            ID_DARK_CURSOR,
            ID_RESTORE_PREV,
            ID_RESTORE_DEFAULT,
            ID_RESTORE_HINT,
        ],
        4 => &[
            ID_PAGE_TITLE_APP,
            ID_PAGE_SUB_APP,
            CARD_BASE + 7,
            ID_APP_GENERAL_TITLE,
            ID_APP_ABOUT_TITLE,
            ID_LANGUAGE_LBL,
            ID_LANGUAGE,
            ID_HOTKEYS,
            ID_AUTOSTART,
            ID_LOGGING,
            ID_PROJECT_HOME_LBL,
            ID_PROJECT_HOME,
            ID_APP_UPDATE_AUTO,
            ID_APP_UPDATE_CHECK,
        ],
        _ => &[
            ID_PAGE_TITLE_NOTIFICATIONS,
            ID_PAGE_SUB_NOTIFICATIONS,
            CARD_BASE + 5,
            CARD_BASE + 6,
            ID_NOTIFICATIONS_TITLE,
            ID_LOCK_KEYS,
            ID_LOCK_CAPS,
            ID_LOCK_NUM,
            ID_LOCK_SCROLL,
            ID_NOTIFICATIONS_BEHAVIOR,
            ID_LOCK_FULLSCREEN,
            ID_NOTIFICATIONS_HINT,
            ID_LOCK_PREVIEW,
        ],
    }
}

/// Visibility + enable cascades — Go applyDependentStates().
fn apply_dependent_states(hwnd: HWND) {
    unsafe {
        let mut visibility = std::collections::BTreeMap::new();
        let mut show = |id, visible| {
            visibility.insert(id, visible);
        };
        let page = PAGE.load(Ordering::SeqCst);
        for p in 0..5 {
            for id in page_ids(p) {
                show(*id, p == page);
            }
        }
        let battery_allowed = is_checked(hwnd, ID_BATTERY_ALLOWED);
        // The label grays through the palette tier (see GRAYED_LABELS);
        // only the edit goes through EnableWindow.
        set_label_grayed(hwnd, ID_BATTERY_LBL, !battery_allowed);
        let edit = get(hwnd, ID_BATTERY_THRESH);
        if IsWindowEnabled(edit).as_bool() != battery_allowed {
            let _ = EnableWindow(edit, battery_allowed);
            // EnableWindow alone can repaint without erasing the old edit
            // background. Refresh its interior and only its own card well,
            // preserving neighboring controls' text.
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(edit), None, true);
            invalidate_well(hwnd, ID_BATTERY_THRESH);
        }
        if page == 2 {
            // The pre-change restore needs a captured snapshot; the muted
            // caption under it explains when one appears.
            let snapshot = crate::appearance::has_restore_snapshot();
            let busy = crate::theme_engine::restore_running();
            let _ = EnableWindow(get(hwnd, ID_RESTORE_PREV), snapshot && !busy);
            let _ = EnableWindow(get(hwnd, ID_RESTORE_DEFAULT), !busy);
            show(ID_RESTORE_HINT, !snapshot);
        }
        if page == 1 {
            let sunrise = combo_sel(hwnd, ID_THEME_MODE) == 1;
            for id in [
                ID_LIGHT_TIME_LBL,
                ID_LIGHT_TIME,
                ID_DARK_TIME_LBL,
                ID_DARK_TIME,
            ] {
                show(id, !sunrise);
            }
            for id in [ID_LOCATION_LBL, ID_LOCATION_SOURCE, ID_THEME_HINT] {
                show(id, sunrise);
            }
            // The status line always shows: the location source in sunrise
            // mode, a one-line fixed-schedule note otherwise - hiding it
            // left the card's lower half hollow.
            show(ID_THEME_LOCATION_STATUS, true);
            set_text(
                hwnd,
                ID_THEME_LOCATION_STATUS,
                &if sunrise {
                    location_status_text(combo_sel(hwnd, ID_LOCATION_SOURCE) == 1)
                } else {
                    t_pub("settings_theme_fixed_note")
                },
            );
        }
        for (id, visible) in visibility {
            crate::nativeform::set_visible_deferred(get(hwnd, id), visible);
        }
        let lock_keys = is_checked(hwnd, ID_LOCK_KEYS);
        for id in [
            ID_LOCK_CAPS,
            ID_LOCK_NUM,
            ID_LOCK_SCROLL,
            ID_LOCK_FULLSCREEN,
        ] {
            let _ = EnableWindow(get(hwnd, id), lock_keys);
        }
    }
}

// ---- Draft collection, validation, save, cancel --------------------------

fn collect_draft(hwnd: HWND) -> Draft {
    let battery_text = control_text(hwnd, ID_BATTERY_THRESH);
    let mut battery_threshold = battery_text.trim().parse::<i32>().ok();
    // A disabled field cannot be corrected. Preserve its saved value if
    // the unfinished edit is invalid; keep valid edits for later use.
    if !is_checked(hwnd, ID_BATTERY_ALLOWED)
        && !battery_threshold.is_some_and(|value| (0..=100).contains(&value))
    {
        battery_threshold = Some(
            crate::runtime::lock(&DRAFT_BASE)
                .as_ref()
                .map(|base| base.nosleep_battery_threshold)
                .unwrap_or_else(|| crate::cfg_map(|c| c.nosleep_battery_threshold)),
        );
    }
    // The row value already is the scheme name ("" = no linkage); the
    // Install footer is an action, never a saved selection.
    let scheme_at = |id: i32| -> String {
        let value = crate::choice::value(get(hwnd, id));
        if value.is_empty() || value == "__install__" {
            String::new()
        } else {
            value
        }
    };
    // The row value already is the wallpaper path ("" = no change).
    let wall_at = |id: i32| -> String {
        let value = crate::choice::value(get(hwnd, id));
        if value == "__browse__" || value.starts_with("__remove__") {
            String::new()
        } else {
            value
        }
    };
    Draft {
        keep_screen: is_checked(hwnd, ID_KEEP_SCREEN),
        battery_allowed: is_checked(hwnd, ID_BATTERY_ALLOWED),
        pause_on_lock: is_checked(hwnd, ID_PAUSE_ON_LOCK),
        battery_threshold,
        idle_timeout: control_text(hwnd, ID_IDLE_TIMEOUT).trim().parse().ok(),
        warning: control_text(hwnd, ID_WARNING_SECONDS).trim().parse().ok(),
        idle_action_idx: combo_sel(hwnd, ID_IDLE_ACTION).min(IDLE_ACTIONS.len() - 1),
        idle_enhanced: is_checked(hwnd, ID_IDLE_ENHANCED),
        theme_mode_idx: combo_sel(hwnd, ID_THEME_MODE),
        light_time: control_text(hwnd, ID_LIGHT_TIME),
        dark_time: control_text(hwnd, ID_DARK_TIME),
        location_idx: combo_sel(hwnd, ID_LOCATION_SOURCE),
        theme_battery: is_checked(hwnd, ID_THEME_BATTERY),
        theme_fullscreen: is_checked(hwnd, ID_THEME_FULLSCREEN),
        light_wallpaper: wall_at(ID_LIGHT_WALL),
        dark_wallpaper: wall_at(ID_DARK_WALL),
        wallpapers: crate::runtime::lock(&WALLPAPER_LIBRARY).clone(),
        // Map the selected row's value back to a scheme name ("" = off).
        light_cursor: scheme_at(ID_LIGHT_CURSOR),
        dark_cursor: scheme_at(ID_DARK_CURSOR),
        language_idx: combo_sel(hwnd, ID_LANGUAGE).min(2),
        lock_keys: is_checked(hwnd, ID_LOCK_KEYS),
        caps: is_checked(hwnd, ID_LOCK_CAPS),
        num: is_checked(hwnd, ID_LOCK_NUM),
        scroll: is_checked(hwnd, ID_LOCK_SCROLL),
        lock_fullscreen: is_checked(hwnd, ID_LOCK_FULLSCREEN),
        hotkeys: is_checked(hwnd, ID_HOTKEYS),
        autostart: is_checked(hwnd, ID_AUTOSTART),
        logging: is_checked(hwnd, ID_LOGGING),
        update_auto: is_checked(hwnd, ID_APP_UPDATE_AUTO),
    }
}

fn validate_draft(draft: &Draft) -> Option<(i32, i32, &'static str)> {
    use crate::settings_model::ValidationField;
    crate::settings_model::validate_draft(draft).map(|(field, key)| {
        let (page, id) = match field {
            ValidationField::NotificationKeys => (3, ID_LOCK_CAPS),
            ValidationField::BatteryThreshold => (0, ID_BATTERY_THRESH),
            ValidationField::IdleTimeout => (0, ID_IDLE_TIMEOUT),
            ValidationField::Warning => (0, ID_WARNING_SECONDS),
            ValidationField::LightTime => (1, ID_LIGHT_TIME),
            ValidationField::DarkTime => (1, ID_DARK_TIME),
        };
        (page, id, key)
    })
}

fn draft_differs(draft: &Draft) -> bool {
    let base = crate::runtime::lock(&DRAFT_BASE)
        .clone()
        .unwrap_or_else(|| crate::cfg_map(Clone::clone));
    crate::settings_model::draft_differs(draft, &base, AUTOSTART_BASE.load(Ordering::SeqCst))
}

fn save() {
    unsafe {
        let hwnd = current();
        let draft = collect_draft(hwnd);
        if let Some((page, id, key)) = validate_draft(&draft) {
            set_validation(hwnd, &t_pub(key), true);
            PAGE.store(page, Ordering::SeqCst);
            apply_dependent_states(hwnd);
            crate::present_layout(hwnd);
            let target = get(hwnd, id);
            if !target.is_invalid() {
                let _ = SetFocus(Some(target));
                crate::viewport::reveal_control(hwnd, target);
            }
            return;
        }

        let autostart_edited = draft.autostart != AUTOSTART_BASE.load(Ordering::SeqCst);
        let Some(base) = crate::runtime::lock(&DRAFT_BASE).clone() else {
            return;
        };
        let edited = draft_config(&draft, &base);
        let mut approved = None;
        let mut changes = crate::settings_model::Changes::default();
        loop {
            let mut conflict = None;
            let result = crate::commit_config(|c, _| {
                let current = c.clone();
                let labels = merge_settings(&base, &edited, c, approved.as_ref());
                if !labels.is_empty() {
                    conflict = Some((current, labels));
                    return Err(t_pub("settings_draft_retained"));
                }
                changes = crate::settings_model::Changes::between(&current, c);
                Ok(())
            });
            let Err(err) = result else {
                break;
            };
            set_validation(hwnd, &err, true);
            let Some((current, labels)) = conflict else {
                return;
            };
            let labels = labels
                .into_iter()
                .map(t_pub)
                .collect::<Vec<_>>()
                .join("\n• ");
            // The transaction has ended before entering the message loop.
            // No configuration lock is held while the user decides.
            if MessageBoxW(
                Some(hwnd),
                PCWSTR(wide(&crate::t_args("settings_conflict_confirm", &[&labels])).as_ptr()),
                PCWSTR(wide(&t_pub("settings_title")).as_ptr()),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            ) != IDYES
            {
                return;
            }
            approved = Some(current);
        }
        let mut failures = Vec::new();
        crate::log_line("settings changed");
        // Apply only effects introduced by this successful merge, including
        // the individual wallpaper/cursor side. Unrelated saves keep restores.
        crate::theme_engine::apply_current_side(changes.appearance);
        crate::apply_stay_awake();
        if changes.language {
            crate::apply_language(&crate::cfg_map(|c| c.language.clone()));
        }
        crate::refresh_checkboxes();
        crate::refresh_status();
        // The worker refreshes restore availability when the queued
        // appearance operation completes, if this window is still open.
        // Settings do not directly change the active palette. Theme changes
        // arrive through the normal notification path; retheming here can
        // cloak/reveal this newly created dialog just before we destroy it.

        if changes.hotkeys || crate::system::hotkeys_need_retry() {
            crate::system::unregister_all();
            if crate::cfg_map(|c| c.hotkeys_enabled) {
                let failed = crate::system::register_all();
                if !failed.is_empty() {
                    failures.push(format!("{}: {}", t_pub("menu_hotkeys"), failed.join(", ")));
                }
            }
        }
        if autostart_edited && draft.autostart != crate::system::autostart_is_enabled() {
            if draft.autostart {
                match std::env::current_exe() {
                    Ok(exe) if crate::system::autostart_enable(&exe.to_string_lossy()) => {
                        crate::log_line("autostart enabled from settings");
                    }
                    Ok(_) => failures.push(t_pub("menu_autostart")),
                    Err(error) => failures.push(format!("{}: {error}", t_pub("menu_autostart"))),
                }
            } else if crate::system::autostart_disable() {
                crate::log_line("autostart disabled from settings");
            } else {
                failures.push(t_pub("menu_autostart"));
            }
        }
        if !failures.is_empty() {
            let details = failures.join(", ");
            crate::log_line(&format!("settings system integration failed: {details}"));
            // Config has been saved, including merged external changes.
            // Refresh the controls/baseline together; retain only the failed
            // autostart request so Save can retry it without undoing the merge.
            populate(hwnd);
            if autostart_edited {
                set_checked(hwnd, ID_AUTOSTART, draft.autostart);
            }
            apply_dependent_states(hwnd);
            set_validation(
                hwnd,
                &crate::t_args("settings_system_apply_failed", &[&details]),
                true,
            );
            return;
        }
        let _ = DestroyWindow(hwnd);
    }
}

fn close_request() {
    unsafe {
        let hwnd = current();
        if draft_differs(&collect_draft(hwnd)) {
            // Go settingspanel confirm: Yes/No + warning + default No.
            let choice = MessageBoxW(
                Some(hwnd),
                PCWSTR(wide(&t_pub("settings_discard_confirm")).as_ptr()),
                PCWSTR(wide(&t_pub("settings_discard_title")).as_ptr()),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            );
            if choice != IDYES {
                return;
            }
        }
        let _ = DestroyWindow(hwnd);
    }
}

unsafe fn open_project_home(hwnd: HWND) {
    unsafe {
        let _ = windows::Win32::UI::Shell::ShellExecuteW(
            Some(hwnd),
            windows::core::w!("open"),
            PCWSTR(wide(PROJECT_URL).as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

unsafe fn create_tooltip(hwnd: HWND) {
    // (control id, i18n key) pairs — Go settingsTooltipBindings().
    static TOOLS: &[(i32, &str)] = &[
        (ID_KEEP_SCREEN, "tip_keep_screen"),
        (ID_BATTERY_ALLOWED, "tip_nosleep_battery"),
        (ID_PAUSE_ON_LOCK, "tip_pause_on_lock"),
        (ID_BATTERY_LBL, "tip_nosleep_battery_threshold"),
        (ID_BATTERY_THRESH, "tip_nosleep_battery_threshold"),
        (ID_IDLE_TIMEOUT_LBL, "tip_idle_timeout"),
        (ID_IDLE_TIMEOUT, "tip_idle_timeout"),
        (ID_IDLE_ACTION_LBL, "tip_idle_action"),
        (ID_IDLE_ACTION, "tip_idle_action"),
        (ID_WARNING_LBL, "tip_idle_warning_seconds"),
        (ID_WARNING_SECONDS, "tip_idle_warning_seconds"),
        (ID_IDLE_ENHANCED, "tip_idle_enhanced"),
        (ID_THEME_MODE_LBL, "tip_theme_mode"),
        (ID_THEME_MODE, "tip_theme_mode"),
        (ID_LIGHT_TIME_LBL, "tip_theme_light_time"),
        (ID_LIGHT_TIME, "tip_theme_light_time"),
        (ID_DARK_TIME_LBL, "tip_theme_dark_time"),
        (ID_DARK_TIME, "tip_theme_dark_time"),
        (ID_LOCATION_LBL, "tip_theme_location_source"),
        (ID_LOCATION_SOURCE, "tip_theme_location_source"),
        (ID_THEME_LOCATION_STATUS, "tip_theme_location_status"),
        (ID_THEME_BATTERY, "tip_battery_theme"),
        (ID_THEME_FULLSCREEN, "tip_fullscreen"),
        (ID_ROW_WALL_LBL, "tip_theme_wallpaper"),
        (ID_LIGHT_WALL, "tip_theme_wallpaper"),
        (ID_DARK_WALL, "tip_theme_wallpaper"),
        (ID_ROW_CURSOR_LBL, "tip_theme_cursor"),
        (ID_LIGHT_CURSOR, "tip_theme_cursor"),
        (ID_DARK_CURSOR, "tip_theme_cursor"),
        (ID_RESTORE_PREV, "tip_theme_restore_prev"),
        (ID_RESTORE_DEFAULT, "tip_theme_restore_default"),
        (ID_LANGUAGE_LBL, "tip_language"),
        (ID_LANGUAGE, "tip_language"),
        (ID_LOCK_KEYS, "tip_lock_keys"),
        (ID_LOCK_CAPS, "tip_lock_key_selection"),
        (ID_LOCK_NUM, "tip_lock_key_selection"),
        (ID_LOCK_SCROLL, "tip_lock_key_selection"),
        (ID_LOCK_FULLSCREEN, "tip_notification_fullscreen"),
        (ID_LOCK_PREVIEW, "tip_notification_preview"),
        (ID_HOTKEYS, "tip_hotkeys"),
        (ID_AUTOSTART, "tip_autostart"),
        (ID_LOGGING, "tip_logging"),
        (ID_APP_UPDATE_AUTO, "tip_update_auto"),
        (ID_PROJECT_HOME, "tip_project_home"),
        (ID_CANCEL, "tip_settings_cancel"),
        (ID_SAVE, "tip_settings_save"),
    ];
    let bindings: Vec<(usize, &str)> = TOOLS.iter().map(|(id, key)| (*id as usize, *key)).collect();
    crate::nativeform::form_tooltips(hwnd, &bindings);
    // Keep the module handle in sync for retheme_tooltip; form_tooltips
    // stores the tooltip as a window property.
    TOOLTIP_HWND.store(
        unsafe { GetPropW(hwnd, windows::core::w!("IdleTriggerFormTooltip")) }.0 as isize,
        Ordering::SeqCst,
    );
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    crate::guarded_proc("settings", hwnd, msg, wparam, lparam, move || unsafe {
        match msg {
            WM_COMMAND => {
                let idc = (wparam.0 & 0xFFFF) as i32;
                let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                match code {
                    EN_CHANGE => {
                        set_text(hwnd, ID_VALIDATION, "");
                        LRESULT(0)
                    }
                    // The well follows focus: repaint just that well, not
                    // the whole card (full-card paints erase siblings).
                    EN_SETFOCUS | EN_KILLFOCUS => {
                        invalidate_well(hwnd, idc);
                        LRESULT(0)
                    }
                    CBN_SELCHANGE if idc == ID_THEME_MODE || idc == ID_LOCATION_SOURCE => {
                        request_location_preview(hwnd);
                        apply_dependent_states(hwnd);
                        LRESULT(0)
                    }
                    CBN_SELCHANGE if idc == ID_LIGHT_WALL || idc == ID_DARK_WALL => {
                        // The Browse footer opens the picker and selects the
                        // chosen file on this side.
                        let value = crate::choice::value(get(hwnd, idc));
                        if value == "__browse__" {
                            browse_wallpaper_for_side(hwnd, idc);
                            set_text(hwnd, ID_VALIDATION, "");
                        } else if let Some(path) = value.strip_prefix("__remove__") {
                            remove_wallpaper(hwnd, path);
                            set_text(hwnd, ID_VALIDATION, "");
                        }
                        LRESULT(0)
                    }
                    CBN_SELCHANGE if idc == ID_LIGHT_CURSOR || idc == ID_DARK_CURSOR => {
                        // The Install footer runs the .inf installer and then
                        // restores the pre-open selection on this side.
                        if crate::choice::value(get(hwnd, idc)) == "__install__" {
                            run_cursor_install(hwnd, idc);
                            set_text(hwnd, ID_VALIDATION, "");
                        }
                        LRESULT(0)
                    }
                    BN_CLICKED => {
                        handle_click(hwnd, idc);
                        LRESULT(0)
                    }
                    _ => DefWindowProcW(hwnd, msg, wparam, lparam),
                }
            }
            WM_DRAWITEM => {
                if let Some(item) = crate::nativeform::draw_item(lparam) {
                    draw_settings_item(hwnd, &item);
                    LRESULT(1)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_CLOSE => {
                close_request();
                LRESULT(0)
            }
            WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
                refresh_theme();
                LRESULT(0)
            }
            WM_SETCURSOR => {
                // Hand cursor over the project-home link (Go wmSetCursor).
                let target = HWND(wparam.0 as *mut _);
                if (target == get(hwnd, ID_PROJECT_HOME))
                    && let Ok(hand) = LoadCursorW(None, IDC_HAND)
                {
                    let _ = SetCursor(Some(hand));
                    return LRESULT(1);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_ERASEBKGND => {
                crate::popups::erase_theme_bg(hwnd, wparam);
                LRESULT(1)
            }
            WM_CTLCOLOREDIT => {
                // Edit interiors paint on the palette surface, not the
                // default white (Go surfaceBrush path).
                let p = theme::palette();
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                let child = HWND(lparam.0 as *mut _);
                let disabled = !IsWindowEnabled(child).as_bool();
                // The erase BRUSH must match the card-face field wells
                // (draw_field inset): the brush does the actual erasing,
                // a surface brush here painted a white band over the well.
                let (fill, color) = if disabled {
                    (p.disabled_surface, p.disabled_text)
                } else {
                    (p.surface, p.text)
                };
                let (light, dark) = theme::surface_brush_pairs();
                let pair = if theme::is_dark() { dark } else { light };
                let brush = if disabled { pair.1 } else { pair.0 };
                let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                    hdc,
                    windows::Win32::Foundation::COLORREF(color),
                );
                let _ = windows::Win32::Graphics::Gdi::SetBkColor(
                    hdc,
                    windows::Win32::Foundation::COLORREF(fill),
                );
                LRESULT(brush.0 as isize)
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                let child_hwnd = HWND(lparam.0 as *mut _);
                // Go wmCtlColorStatic: section titles get PrimaryText, muted
                // labels (version/hints/status) get MutedText, everything
                // else gets SecondaryText; disabled controls get DisabledText.
                if msg == WM_CTLCOLORSTATIC {
                    // Disabled edit interiors paint on the disabled surface,
                    // not the window background — this check MUST come before
                    // the text tier logic (Go disabledBrush path).
                    if !IsWindowEnabled(child_hwnd).as_bool() {
                        let mut buffer = [0u16; 8];
                        let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameW(
                            child_hwnd,
                            &mut buffer,
                        );
                        let class =
                            String::from_utf16_lossy(&buffer[..len.max(0) as usize]).to_uppercase();
                        if class == "EDIT" {
                            let palette = theme::palette();
                            let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                                hdc,
                                windows::Win32::Foundation::COLORREF(palette.disabled_text),
                            );
                            let _ = windows::Win32::Graphics::Gdi::SetBkColor(
                                hdc,
                                windows::Win32::Foundation::COLORREF(palette.disabled_surface),
                            );
                            let (light, dark) = theme::surface_brush_pairs();
                            let pair = if theme::is_dark() { dark } else { light };
                            return LRESULT(pair.1.0 as isize);
                        }
                    }
                    let palette = theme::palette();
                    let id = GetWindowLongPtrW(child_hwnd, GWL_ID) as i32;
                    let is_muted = matches!(
                        id,
                        ID_PAGE_SUB_POWER
                            | ID_PAGE_SUB_THEME
                            | ID_PAGE_SUB_APPEARANCE
                            | ID_PAGE_SUB_NOTIFICATIONS
                            | ID_PAGE_SUB_APP
                            | ID_VERSION
                            | ID_THEME_HINT
                            | ID_THEME_LOCATION_STATUS
                            | ID_VALIDATION
                            | ID_NOTIFICATIONS_HINT
                            | ID_RESTORE_HINT
                    );
                    let disabled = !IsWindowEnabled(child_hwnd).as_bool() || is_grayed_label(id);
                    let text = if id == ID_VALIDATION
                        && GetWindowTextLengthW(child_hwnd) > 0
                        && VALIDATION_ERROR.load(Ordering::SeqCst)
                    {
                        palette.danger_surface_text
                    } else if disabled {
                        palette.disabled_text
                    } else if is_muted {
                        palette.muted
                    } else {
                        // Two ink tiers only: primary for titles AND labels,
                        // muted for real secondary prose. A third body gray
                        // read as noise across the cards.
                        palette.text
                    };
                    let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                        hdc,
                        windows::Win32::Foundation::COLORREF(text),
                    );
                    let (bk, brush) = child_back_brush(hwnd, child_hwnd);
                    let _ = windows::Win32::Graphics::Gdi::SetBkColor(
                        hdc,
                        windows::Win32::Foundation::COLORREF(bk),
                    );
                    return LRESULT(brush);
                }
                let validation = get(hwnd, ID_VALIDATION);
                let is_error = child_hwnd == validation
                    && GetWindowTextLengthW(validation) > 0
                    && VALIDATION_ERROR.load(Ordering::SeqCst);
                let color = if is_error {
                    // Validation messages render in the error color.
                    theme::palette().danger_surface_text
                } else {
                    theme::text_color()
                };
                let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                    hdc,
                    windows::Win32::Foundation::COLORREF(color),
                );
                let (bk, brush) = child_back_brush(hwnd, child_hwnd);
                let _ = windows::Win32::Graphics::Gdi::SetBkColor(
                    hdc,
                    windows::Win32::Foundation::COLORREF(bk),
                );
                LRESULT(brush)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_NCHITTEST => {
                // Shared blank drag (see nativeform::blank_drag_hit): a
                // HTCLIENT point no interactive child claims drags the
                // window. Never WindowFromPoint here - its probe re-enters
                // this handler once transparent children sit above and
                // recurses (the switch-row blank crash).
                crate::nativeform::blank_drag_hit(hwnd, msg, wparam, lparam)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_NCRBUTTONUP => LRESULT(0),
            WM_DESTROY => {
                RESTORE_REQUEST.fetch_add(1, Ordering::SeqCst);
                SETTINGS_HWND.store(0, Ordering::SeqCst);
                TOOLTIP_HWND.store(0, Ordering::SeqCst);
                PAGE.store(0, Ordering::SeqCst);
                // Drop owner-draw checkbox states so a recreated window does
                // not resurrect check marks on reused control ids.
                *checks() = None;
                crate::release_panel_modal(hwnd);
                crate::refresh_status();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    })
}

/// WM_DRAWITEM dispatcher (Go settingspanel window.go drawItem).
fn draw_settings_item(hwnd: HWND, item: &crate::nativeform::DrawItem) {
    // Buffered blit first (Go drawItemBuffered); fall back to direct paint.
    unsafe {
        // Card repaints end in ONE SRCCOPY of the whole card rect back
        // onto the target DC - an unclipped blit that overwrites the
        // pixels of every edit hosted on the card (their text survives
        // only if they happen to repaint afterwards). Cut the hosted
        // edits' rects out of the TARGET DC first; BitBlt honors the DC
        // clip, so neither the blit nor the direct fallback can touch
        // the edits, no matter who invalidated the card or how widely.
        if (CARD_BASE..=CARD_BASE + 9).contains(&item.control_id) {
            let mut card_rect = RECT::default();
            if GetWindowRect(item.control, &mut card_rect).is_ok() {
                for edit_id in card_field_ids(item.control_id) {
                    let edit = get(hwnd, *edit_id);
                    if edit.is_invalid() || !IsWindowVisible(edit).as_bool() {
                        continue;
                    }
                    let mut edit_rect = RECT::default();
                    if GetWindowRect(edit, &mut edit_rect).is_ok() {
                        let _ = windows::Win32::Graphics::Gdi::ExcludeClipRect(
                            item.dc,
                            edit_rect.left - card_rect.left,
                            edit_rect.top - card_rect.top,
                            edit_rect.right - card_rect.left,
                            edit_rect.bottom - card_rect.top,
                        );
                    }
                }
            }
        }
        if crate::nativeform::draw_buffered(item.dc, &item.bounds, |dc, bounds| {
            draw_settings_item_impl(hwnd, item, dc, bounds);
        }) {
            return;
        }
        draw_settings_item_impl(hwnd, item, item.dc, &item.bounds);
    }
}

unsafe fn draw_settings_item_impl(
    hwnd: HWND,
    item: &crate::nativeform::DrawItem,
    dc: HDC,
    bounds: &RECT,
) {
    let p = theme::palette();
    let scale = crate::scale_pub(96);
    let id = item.control_id;
    let label = crate::window_text(item.control);
    if (CARD_BASE..=CARD_BASE + 9).contains(&id) {
        // Section card face: rounded surface with the family hairline,
        // same grammar as the panel's cards.
        crate::paint::draw_surface(
            dc,
            bounds,
            p.window_bg,
            p.surface,
            p.border,
            crate::paint::control_radius(),
        );
        // Field wells ride the card face: each hosted edit is inset, and
        // the well + hairline around it are card pixels. The card never
        // paints INSIDE an edit: draw_settings_item clips the edits out
        // of the target DC (buffered-blit protection), and draw_field
        // excludes them on the GDI+ side here.
        for edit_id in card_field_ids(id) {
            let edit = get(hwnd, *edit_id);
            if edit.is_invalid() || !unsafe { IsWindowVisible(edit).as_bool() } {
                // Hidden fields (fixed/sunrise mode swap) paint no well: a
                // blank box where an inactive field would sit reads as a
                // ghost control.
                continue;
            }
            unsafe {
                let mut edit_rect = RECT::default();
                if GetWindowRect(edit, &mut edit_rect).is_err() {
                    continue;
                }
                let mut card_rect = RECT::default();
                if GetWindowRect(item.control, &mut card_rect).is_err() {
                    continue;
                }
                let well = crate::nativeform::field_well_rect(&card_rect, &edit_rect);
                if well.right <= well.left || well.bottom <= well.top {
                    continue;
                }
                // The owner-draw item DC carries NO sibling clipping
                // (WS_CLIPSIBLINGS does not reach it), so an unclipped
                // well fill lands on the edit's own pixels and blanks its
                // text until the edit happens to repaint. Cut the edit's
                // rect out on both the GDI and GDI+ sides: the card can
                // then only ever paint the ring and margin around it.
                let edit_area = RECT {
                    left: edit_rect.left - card_rect.left,
                    top: edit_rect.top - card_rect.top,
                    right: edit_rect.right - card_rect.left,
                    bottom: edit_rect.bottom - card_rect.top,
                };
                let state = crate::paint::ControlState {
                    focused: GetFocus() == edit,
                    disabled: !IsWindowEnabled(edit).as_bool(),
                    ..Default::default()
                };
                crate::paint::draw_field(
                    dc,
                    &well,
                    p,
                    p.surface,
                    p.surface,
                    state,
                    crate::paint::control_radius(),
                    Some(&edit_area),
                );
            }
        }
    } else if id == ID_PROJECT_HOME {
        let state = crate::nativeform::control_state(item.control, item.state);
        crate::paint::draw_text_link(
            dc,
            bounds,
            crate::kit::fonts::body(),
            &label,
            p,
            // The fix link sits below the card on the window background;
            // the project-home link rides the app page card face... both
            // read fine on the window color, and CLIPC targets cards only.
            p.window_bg,
            state,
            scale,
        );
    } else if crate::choice::is_choice(item.control) {
        let state = crate::nativeform::control_state(item.control, item.state);
        crate::choice::draw_button(item.control, dc, bounds, state, p.surface);
    } else if matches!(
        id,
        ID_TAB_POWER | ID_TAB_THEME | ID_TAB_APP | ID_TAB_NOTIFICATIONS | ID_TAB_APPEARANCE
    ) {
        let mut state = crate::nativeform::control_state(item.control, item.state);
        let page = PAGE.load(Ordering::SeqCst);
        state.active = id
            == [
                ID_TAB_POWER,
                ID_TAB_THEME,
                ID_TAB_APPEARANCE,
                ID_TAB_NOTIFICATIONS,
                ID_TAB_APP,
            ][page as usize];
        crate::paint::draw_nav_item(
            dc,
            bounds,
            crate::kit::fonts::section(),
            crate::kit::fonts::body(),
            &label,
            p,
            p.window_bg,
            state,
            scale,
        );
    } else if id == ID_SAVE {
        // Save is the form's default action: accent fill (Go state.Active).
        let mut state = crate::nativeform::control_state(item.control, item.state);
        state.active = true;
        crate::paint::draw_button(
            dc,
            bounds,
            crate::kit::fonts::body(),
            &label,
            p,
            p.window_bg,
            state,
            crate::paint::control_radius(),
        );
    } else if checks().as_ref().is_some_and(|m| m.contains_key(&id)) {
        // Toggle rows use the panel's switch grammar: the whole row is
        // the hit target, the pill on the right carries the state. The
        // checked value stays a DRAFT until Save, exactly like before.
        let mut state = crate::nativeform::control_state(item.control, item.state);
        state.active = is_checked(hwnd, id);
        let progress = crate::switch_animation_progress(item.control);
        crate::paint::draw_switch_row(
            dc,
            bounds,
            crate::kit::fonts::body(),
            &label,
            p,
            p.surface,
            state,
            scale,
            progress,
        );
    } else if matches!(id, ID_RESTORE_PREV | ID_LOCK_PREVIEW | ID_APP_UPDATE_CHECK) {
        // Buttons that sit on a card face must erase with the card
        // color, or their background punch-through shows window ink.
        let state = crate::nativeform::control_state(item.control, item.state);
        crate::paint::draw_button(
            dc,
            bounds,
            crate::kit::fonts::body(),
            &label,
            p,
            p.surface,
            state,
            crate::paint::control_radius(),
        );
    } else {
        let state = crate::nativeform::control_state(item.control, item.state);
        crate::paint::draw_button(
            dc,
            bounds,
            crate::kit::fonts::body(),
            &label,
            p,
            p.window_bg,
            state,
            crate::paint::control_radius(),
        );
    }
}

fn handle_click(hwnd: HWND, idc: i32) {
    unsafe {
        match idc {
            ID_TAB_POWER | ID_TAB_THEME | ID_TAB_APP | ID_TAB_NOTIFICATIONS | ID_TAB_APPEARANCE => {
                let page = match idc {
                    ID_TAB_POWER => 0,
                    ID_TAB_THEME => 1,
                    ID_TAB_APPEARANCE => 2,
                    ID_TAB_NOTIFICATIONS => 3,
                    _ => 4,
                };
                PAGE.store(page, Ordering::SeqCst);
                apply_dependent_states(hwnd);
                // Page-change visuals live here, NOT in
                // apply_dependent_states: that also runs on every checkbox
                // toggle, and a tab sweep plus a full layout present per
                // toggle made the rest of the page flash.
                for id in [
                    ID_TAB_POWER,
                    ID_TAB_THEME,
                    ID_TAB_APP,
                    ID_TAB_NOTIFICATIONS,
                    ID_TAB_APPEARANCE,
                ] {
                    let control = get(hwnd, id);
                    if !control.is_invalid() {
                        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(
                            Some(control),
                            None,
                            false,
                        );
                    }
                }
                if IsWindowVisible(hwnd).as_bool() {
                    crate::present_layout(hwnd);
                }
            }
            ID_KEEP_SCREEN | ID_BATTERY_ALLOWED | ID_PAUSE_ON_LOCK | ID_IDLE_ENHANCED
            | ID_THEME_BATTERY | ID_THEME_FULLSCREEN | ID_HOTKEYS | ID_AUTOSTART | ID_LOGGING
            | ID_APP_UPDATE_AUTO | ID_LOCK_KEYS | ID_LOCK_CAPS | ID_LOCK_NUM | ID_LOCK_SCROLL
            | ID_LOCK_FULLSCREEN => {
                // Owner-drawn checkboxes keep state in CHECKS. The flight
                // animates the flip with the same glide the panel uses.
                let before = is_checked(hwnd, idc);
                toggle_checked(hwnd, idc);
                let control = get(hwnd, idc);
                if !control.is_invalid() {
                    crate::start_switch_animation(control, before, is_checked(hwnd, idc));
                }
                apply_dependent_states(hwnd);
                set_text(hwnd, ID_VALIDATION, "");
            }
            ID_THEME_MODE | ID_LOCATION_SOURCE | ID_IDLE_ACTION | ID_LANGUAGE | ID_LIGHT_CURSOR
            | ID_DARK_CURSOR | ID_LIGHT_WALL | ID_DARK_WALL => {
                // Refresh scheme/wallpaper rows right before opening so an
                // install that just finished or a library edit shows up.
                if idc == ID_LIGHT_CURSOR || idc == ID_DARK_CURSOR {
                    refresh_cursor_choices(hwnd);
                } else if idc == ID_LIGHT_WALL || idc == ID_DARK_WALL {
                    refresh_wallpaper_choices(hwnd);
                }
                if matches!(
                    idc,
                    ID_LIGHT_CURSOR | ID_DARK_CURSOR | ID_LIGHT_WALL | ID_DARK_WALL
                ) {
                    let value = crate::choice::value(get(hwnd, idc));
                    crate::runtime::lock(&APPEARANCE_PREVIOUS).insert(idc, value);
                }
                crate::choice::toggle(get(hwnd, idc), hwnd, idc);
            }
            ID_LOCK_PREVIEW => crate::popups::show(crate::popups::VK_CAPITAL, true),
            ID_RESTORE_PREV => {
                start_appearance_restore(hwnd, false);
            }
            ID_RESTORE_DEFAULT => {
                start_appearance_restore(hwnd, true);
            }
            ID_PROJECT_HOME => open_project_home(hwnd),
            ID_APP_UPDATE_CHECK => on_update_check_button(),
            ID_SAVE => save(),
            ID_CANCEL => close_request(),
            _ => {}
        }
    }
}

fn start_appearance_restore(hwnd: HWND, defaults: bool) {
    if crate::theme_engine::restore_running() {
        return;
    }
    let id = RESTORE_REQUEST.fetch_add(1, Ordering::SeqCst) + 1;
    if crate::theme_engine::restore_appearance(defaults, id) {
        set_validation(hwnd, &t_pub("settings_restore_started"), false);
        apply_dependent_states(hwnd);
    }
}

pub fn refresh_appearance_state() {
    let hwnd = current();
    if !hwnd.is_invalid() {
        let _dpi = crate::dpi::Scope::window(hwnd);
        apply_dependent_states(hwnd);
    }
}

pub fn finish_appearance_restore(id: u64, sides: crate::appearance::RestoreSides) {
    refresh_appearance_state();
    let hwnd = current();
    let (message, failed) = restore_feedback(sides);
    if !hwnd.is_invalid() && RESTORE_REQUEST.load(Ordering::SeqCst) == id {
        set_validation(hwnd, &message, failed);
    } else {
        // A completion from a closed dialog must not write into a new
        // settings draft, even if Windows reused the same HWND.
        if failed {
            crate::warn_dialog("", &message);
        } else {
            crate::log_line(&message);
        }
    }
}

/// Report only attempted sides; a wallpaper-only snapshot must not claim
/// that cursors were restored too. Keep successful sides visible on failure.
fn restore_feedback(sides: crate::appearance::RestoreSides) -> (String, bool) {
    let mut restored = Vec::new();
    let mut errors = Vec::new();
    for (label, result) in [
        ("settings_row_wallpaper", sides.0),
        ("settings_row_cursor", sides.1),
    ] {
        match result {
            Some(Ok(())) => restored.push(t_pub(label)),
            Some(Err(error)) => errors.push(format!("{}: {error}", t_pub(label))),
            None => {}
        }
    }
    if errors.is_empty() && restored.is_empty() {
        (t_pub("settings_restore_prev_missing"), false)
    } else if errors.is_empty() {
        (
            crate::t_args("settings_restore_done", &[&restored.join(", ")]),
            false,
        )
    } else if restored.is_empty() {
        (
            crate::t_args("settings_restore_failed", &[&errors.join("; ")]),
            true,
        )
    } else {
        (
            crate::t_args(
                "settings_restore_partial",
                &[&restored.join(", "), &errors.join("; ")],
            ),
            true,
        )
    }
}

/// Center on the owner, then clamp into its monitor work area (Go
/// PlaceWindow: CenteredRect + ConstrainRect).
/// Refresh captions and choice labels while retaining every draft value.
pub fn refresh_language() {
    let hwnd = current();
    if hwnd.is_invalid() {
        return;
    }
    let _dpi = crate::dpi::Scope::window(hwnd);
    let _frame = crate::FrameTransition::begin(hwnd);
    crate::viewport::begin_layout(hwnd);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide(&t_pub("settings_title")).as_ptr()));
    }
    for (id, key) in [
        // Chrome only: page headers, nav tabs, footer buttons. Page
        // content rides the declarative specs below.
        (ID_PAGE_TITLE_POWER, "settings_tab_power"),
        (ID_PAGE_SUB_POWER, "settings_power_page_hint"),
        (ID_PAGE_TITLE_THEME, "settings_tab_theme"),
        (ID_PAGE_SUB_THEME, "settings_theme_page_hint"),
        (ID_PAGE_TITLE_APPEARANCE, "settings_tab_appearance"),
        (ID_PAGE_SUB_APPEARANCE, "settings_appearance_page_hint"),
        (ID_PAGE_TITLE_NOTIFICATIONS, "settings_tab_notifications"),
        (
            ID_PAGE_SUB_NOTIFICATIONS,
            "settings_notifications_page_hint",
        ),
        (ID_PAGE_TITLE_APP, "settings_tab_app"),
        (ID_PAGE_SUB_APP, "settings_app_page_hint"),
        (ID_TAB_POWER, "settings_tab_power"),
        (ID_TAB_THEME, "settings_tab_theme"),
        (ID_TAB_NOTIFICATIONS, "settings_tab_notifications"),
        (ID_TAB_APP, "settings_tab_app"),
        (ID_TAB_APPEARANCE, "settings_tab_appearance"),
        (ID_SAVE, "common_save"),
        (ID_CANCEL, "common_cancel"),
    ] {
        set_text(hwnd, id, &t_pub(key));
    }
    set_text(
        hwnd,
        ID_VERSION,
        &crate::t_args("settings_version", &[&crate::version_with_arch()]),
    );
    // The five pages ride the declarative specs: relabel, combo item
    // rebuild (index-preserving), and - for the language-geometry pages -
    // the reposition, all from the same specs creation uses.
    unsafe { crate::kit::spec::apply(hwnd, &power_page_spec()) };
    unsafe { crate::kit::spec::apply(hwnd, &appearance_page_spec()) };
    unsafe { crate::kit::spec::apply_relayout(hwnd, &theme_page_spec()) };
    unsafe { crate::kit::spec::apply_relayout(hwnd, &app_page_spec(hwnd)) };
    unsafe {
        crate::kit::spec::apply_relayout(hwnd, &notifications_page_spec());
    }
    // The check button's caption is phase-formatted; re-derive it in the
    // new language.
    refresh_update_status();
    // The wallpaper and cursor rows carry localized action entries ("No
    // change", "Browse…", "Install…"), so they need the same language
    // rebuild; the file-name and scheme-name entries keep their labels via
    // the value-preserving refresh.
    refresh_wallpaper_choices(hwnd);
    refresh_cursor_choices(hwnd);
    set_text(
        hwnd,
        ID_THEME_LOCATION_STATUS,
        &location_status_text(combo_sel(hwnd, ID_LOCATION_SOURCE) == 1),
    );
    unsafe {
        create_tooltip(hwnd);
    }
    crate::viewport::end_layout(hwnd);
    refresh_theme();
}

#[cfg(test)]
mod locale_tests {
    use super::*;
    #[test]
    fn relabeling_preserves_native_edits_choice_and_save_baseline() {
        let _guard = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        let old_config = crate::runtime::lock(&crate::CONFIG).replace(Default::default());
        let old_locale = crate::I18N
            .write()
            .unwrap()
            .replace(idletrigger_core::i18n::I18n::load("en"));
        create();
        let window = current();
        let edit = get(window, ID_IDLE_TIMEOUT);
        set_text(window, ID_IDLE_TIMEOUT, "073");
        crate::choice::select_index(get(window, ID_LANGUAGE), 2);
        let baseline = crate::runtime::lock(&DRAFT_BASE).clone();
        *crate::I18N.write().unwrap() = Some(idletrigger_core::i18n::I18n::load("zh-CN"));
        refresh_language();
        assert_eq!(get(window, ID_IDLE_TIMEOUT), edit);
        assert_eq!(control_text(window, ID_IDLE_TIMEOUT), "073");
        assert_eq!(control_text(window, ID_PAGE_TITLE_POWER), "电源管理");
        assert_eq!(combo_sel(window, ID_LANGUAGE), 2);
        assert_eq!(*crate::runtime::lock(&DRAFT_BASE), baseline);
        set_checked(window, ID_BATTERY_ALLOWED, true);
        set_text(window, ID_BATTERY_THRESH, "101");
        assert_eq!(
            validate_draft(&collect_draft(window)).unwrap().1,
            ID_BATTERY_THRESH
        );
        set_checked(window, ID_BATTERY_ALLOWED, false);
        apply_dependent_states(window);
        assert!(!unsafe { IsWindowEnabled(get(window, ID_BATTERY_THRESH)).as_bool() });
        let disabled_draft = collect_draft(window);
        assert_eq!(
            disabled_draft.battery_threshold,
            baseline.as_ref().map(|base| base.nosleep_battery_threshold)
        );
        assert!(validate_draft(&disabled_draft).is_none());
        assert_eq!(control_text(window, ID_BATTERY_THRESH), "101");
        set_checked(window, ID_BATTERY_ALLOWED, true);
        apply_dependent_states(window);
        assert_eq!(
            validate_draft(&collect_draft(window)).unwrap().1,
            ID_BATTERY_THRESH
        );
        set_text(window, ID_BATTERY_THRESH, "42");
        set_checked(window, ID_BATTERY_ALLOWED, false);
        assert_eq!(collect_draft(window).battery_threshold, Some(42));
        PAGE.store(1, Ordering::SeqCst);
        crate::choice::select_index(get(window, ID_THEME_MODE), 1);
        crate::choice::select_index(get(window, ID_LOCATION_SOURCE), 1);
        apply_dependent_states(window);
        refresh_location_status();
        assert!(control_text(window, ID_THEME_LOCATION_STATUS).starts_with("IP 定位"));
        let visible =
            |id| unsafe { GetWindowLongW(get(window, id), GWL_STYLE) as u32 & WS_VISIBLE.0 != 0 };
        assert!(visible(ID_LOCATION_SOURCE));
        assert!(!visible(ID_LIGHT_TIME));
        assert!(!visible(ID_IDLE_TIMEOUT));
        crate::choice::select_index(get(window, ID_THEME_MODE), 0);
        apply_dependent_states(window);
        assert!(!visible(ID_LOCATION_SOURCE));
        assert!(visible(ID_LIGHT_TIME));
        assert_eq!(control_text(window, ID_IDLE_TIMEOUT), "073");
        unsafe {
            DestroyWindow(window).unwrap();
        }
        *crate::runtime::lock(&crate::CONFIG) = old_config;
        *crate::I18N.write().unwrap() = old_locale;
    }
}
