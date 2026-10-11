//! Context-menu manager with separate list, editor, and live menu options.
//! Geometry uses equal columns, 32px controls, and an 8/12/16px spacing scale.

use idletrigger_core::ctx_menu as core_ctx;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OPENFILENAMEW,
};
use windows::Win32::UI::Controls::WC_LISTBOX;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, IsWindowEnabled, SetFocus,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, CreateWindowExW, DefWindowProcW, GW_CHILD, GW_HWNDNEXT, GetClassNameW, GetDlgItem,
    GetWindow, GetWindowRect, GetWindowTextW, IDC_ARROW, IsWindowVisible, LBN_DBLCLK,
    LBN_SELCHANGE, LoadCursorW, MB_ICONQUESTION, MB_YESNO, MessageBoxW, RegisterClassW, SW_HIDE,
    SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow, SetWindowPos, ShowWindow,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_DRAWITEM, WM_ERASEBKGND, WM_NCHITTEST, WM_SETFONT, WNDCLASSW,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_TABSTOP, WS_VISIBLE,
};
use windows::Win32::UI::WindowsAndMessaging::{HMENU, IDYES, SendMessageW, SetWindowTextW};
use windows::core::PCWSTR;

use crate::theme;
use crate::wide;

const EM_SETMARGINS_RAW: u32 = 0x00D3;
const EN_SETFOCUS: usize = 0x0100;
const EN_KILLFOCUS: usize = 0x0200;
const EN_CHANGE: usize = 0x0300;

// ---- control ids -----------------------------------------------------------

const ID_TITLE: usize = 700; // header page title
const ID_MORE: usize = 701; // menu options tab
const ID_SURF: usize = 702; // list card surface
const ID_LIST: usize = 703;
const ID_ADD: usize = 704;
const ID_DELETE: usize = 705;
const ID_UP: usize = 706;
const ID_DOWN: usize = 707;
pub(crate) const ID_ADD_SEP: usize = 709;
const ID_EMPTY: usize = 708; // centered empty-state hint on the list card
const ID_FORM_TITLE: usize = 710; // "rule details" section title
const ID_VALID: usize = 711; // guidance / validation line (title row right)
const ID_FORM: usize = 712; // form card surface
// Column A fields.
const ID_TITLE_L: usize = 713;
const ID_TITLE_E: usize = 714;
const ID_PROG_L: usize = 715;
const ID_PROG_E: usize = 716;
const ID_PROG_B: usize = 717;
const ID_MATCH_L: usize = 718;
const ID_MATCH_E: usize = 719;
// Column B fields.
const ID_SCOPE_L: usize = 720;
const ID_SCOPE_C: usize = 721;
const ID_ARGS_L: usize = 722;
const ID_ARGS_E: usize = 723;
const ID_ICON_L: usize = 724;
const ID_ICON_E: usize = 725;
const ID_ICON_B: usize = 726;
const ID_CONSOLE: usize = 727; // hide-console switch row
const ID_DISCARD: usize = 728;
const ID_SAVE: usize = 729;
pub(crate) const ID_ENABLED: usize = 730;
const ID_WORK_L: usize = 731;
pub const FIELD_WORK: usize = 732;
const ID_PREVIEW: usize = 733;
const ID_STATUS: usize = 734;
pub const ID_WORK_B: usize = 735;
pub(crate) const ID_MASTER: usize = 737;
const ID_TAB_RULES: usize = 738;
const ID_SELECTION_L: usize = 739;
pub const FIELD_SELECTION: usize = 740;
const ID_EDIT: usize = 741;
const ID_BACK: usize = 742;
const ID_CLOSE: usize = 743;
const ID_ADVANCED: usize = 744;
const ID_LIST_HINT: usize = 745;
pub(crate) const ID_REPAIR: usize = 746;
const ID_OPTIONS_HINT: usize = 747;
const ID_LAYOUT_TITLE: usize = 748;
const ID_LAYOUT_CARD: usize = 749;
pub(crate) const ID_GROUPED: usize = 750;
const ID_LAYOUT_HINT: usize = 751;
pub(crate) const ID_CLASSIC: usize = 752;
const ID_COPY_TITLE: usize = 753;
const ID_COPY_CARD: usize = 754;
pub(crate) const ID_COPY: usize = 755;
const ID_QUOTE_L: usize = 756;
pub(crate) const ID_QUOTE: usize = 757;
const ID_SEPARATOR_L: usize = 758;
pub(crate) const ID_SEPARATOR: usize = 759;
const ID_OPTIONS_STATUS: usize = 760;
const ID_TOGGLE: usize = 761;
const ID_EDITOR_GUIDE: usize = 762;
const VIEW_LIST: usize = 0;
const VIEW_EDIT: usize = 1;
const VIEW_OPTIONS: usize = 2;
static VIEW: AtomicUsize = AtomicUsize::new(VIEW_LIST);
static ADVANCED: AtomicBool = AtomicBool::new(false);
static RULE_ENABLED: AtomicBool = AtomicBool::new(true);

/// Tooltip anchors for the manager's form fields.
pub const FIELD_ID_TITLE: usize = ID_TITLE_E;
pub const FIELD_ID_PROGRAM: usize = ID_PROG_E;
pub const FIELD_ID_ARGS: usize = ID_ARGS_E;
pub const FIELD_ID_ICON: usize = ID_ICON_E;
pub const FIELD_ID_MATCH: usize = ID_MATCH_E;
pub const FIELD_SCOPE: usize = ID_SCOPE_C;
pub const FIELD_CONSOLE: usize = ID_CONSOLE;

/// Every edit hosted on the form card (they get the card-painted well).
const FIELD_EDITS: [usize; 6] = [
    ID_TITLE_E, ID_PROG_E, ID_ARGS_E, ID_ICON_E, ID_MATCH_E, FIELD_WORK,
];
/// Statics whose background must erase with the card face.
const ON_CARD_STATICS: &[usize] = &[
    ID_TITLE_L,
    ID_PROG_L,
    ID_MATCH_L,
    ID_SCOPE_L,
    ID_ARGS_L,
    ID_ICON_L,
    ID_EMPTY,
    ID_WORK_L,
    ID_SELECTION_L,
    ID_LAYOUT_HINT,
    ID_QUOTE_L,
    ID_SEPARATOR_L,
];

// ---- geometry (logical px, scaled at creation) ------------------------------

const CLIENT_W: i32 = 680;
const CLIENT_H: i32 = 648;
const EDGE: i32 = 16;
const IN_X: i32 = EDGE + 12;
const COL_GAP: i32 = 16;
const COL_W: i32 = (CLIENT_W - 2 * IN_X - COL_GAP) / 2;
const COL_A: i32 = IN_X;
const COL_B: i32 = IN_X + COL_W + COL_GAP;

const HEADER_Y: i32 = 20;
const HEADER_H: i32 = 24;
const LIST_Y: i32 = 184;
const LIST_H: i32 = 360;
const ACTIONS_Y: i32 = 136;
const ACT_H: i32 = 32;
const ACT_W: i32 = 128;
const FORM_TITLE_Y: i32 = 142;
const FORM_Y: i32 = 184;

const LABEL_H: i32 = 20;
const LABEL_GAP: i32 = 4;
const FIELD_H: i32 = 32; // field slot; the edit itself is inset inside
const SLOT_GAP: i32 = 8;
const BROWSE_W: i32 = 80;
const FOOTER_Y: i32 = 600;

static MGR_HWND: AtomicIsize = AtomicIsize::new(0);
static LIST_HWND: AtomicIsize = AtomicIsize::new(0);
static LIST_ROW_COUNT: AtomicUsize = AtomicUsize::new(0);
/// The console switch is the form's one toggle; owner-draw buttons keep no
/// check state of their own.
static CONSOLE_HIDE: AtomicBool = AtomicBool::new(false);
/// Snapshot the list was rendered from; the edit base for stale detection.
static DISPLAYED: Mutex<Vec<core_ctx::CtxRule>> = Mutex::new(Vec::new());
/// The form's working rule (None = no selection / new rule not started).
static DRAFT: Mutex<Option<core_ctx::CtxRule>> = Mutex::new(None);

fn s(v: i32) -> i32 {
    crate::dpi::scale(v)
}

fn body_font() -> HFONT {
    crate::make_font_pub(14, 400)
}

fn label_font() -> HFONT {
    crate::make_font_pub(13, 400)
}

fn section_font() -> HFONT {
    crate::make_font_pub(14, 600)
}

fn mgr() -> HWND {
    HWND(MGR_HWND.load(Ordering::SeqCst) as *mut core::ffi::c_void)
}

/// Devtools capture support: the manager's top-level window.
#[cfg(any(feature = "devtools", test))]
pub fn devtools_hwnd() -> HWND {
    mgr()
}

fn list_hwnd() -> HWND {
    HWND(LIST_HWND.load(Ordering::SeqCst) as *mut core::ffi::c_void)
}

fn get_dlg_item(parent: HWND, id: usize) -> HWND {
    unsafe { GetDlgItem(Some(parent), id as i32).unwrap_or_default() }
}

fn text_of(parent: HWND, id: usize) -> String {
    let control = get_dlg_item(parent, id);
    if control.is_invalid() {
        return String::new();
    }
    let len = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW(control) };
    let mut buffer = vec![0u16; len.max(0) as usize + 1];
    let copied = unsafe { GetWindowTextW(control, &mut buffer) };
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

fn set_text(id: usize, text: &str) {
    let control = get_dlg_item(mgr(), id);
    if !control.is_invalid() && text_of(mgr(), id) != text {
        unsafe {
            let _ = SetWindowTextW(control, PCWSTR(wide(text).as_ptr()));
        }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn mk_control(
    parent: HWND,
    class: windows::core::PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    font: HFONT,
) -> HWND {
    unsafe {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(wide(text).as_ptr()),
            style,
            s(x),
            s(y),
            s(w),
            s(h),
            Some(parent),
            Some(HMENU(id as *mut _)),
            None,
            None,
        )
        .expect("ctx manager control");
        let _ = SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        hwnd
    }
}

/// Owner-draw static card surface pinned beneath every hosted row (settings
/// section-card grammar): the parent's WM_DRAWITEM paints its face.
unsafe fn surface_card(parent: HWND, id: usize, x: i32, y: i32, w: i32, h: i32) -> HWND {
    unsafe {
        let card = mk_control(
            parent,
            windows::core::w!("STATIC"),
            "",
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0 | 13), // SS_OWNERDRAW
            id,
            x,
            y,
            w,
            h,
            body_font(),
        );
        // Pin beneath everything: z-order passes must never float the card
        // above its rows (their owner-draw DCs would clip to nothing).
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

fn scope_items() -> Vec<crate::choice::ChoiceItem> {
    [
        ("ctx_scope_file", core_ctx::SCOPE_FILE),
        ("ctx_scope_dir", core_ctx::SCOPE_DIR),
        ("ctx_scope_file_dir", core_ctx::SCOPE_FILE_DIR),
        ("ctx_scope_background", core_ctx::SCOPE_BACKGROUND),
        ("ctx_scope_all", core_ctx::SCOPE_ALL),
    ]
    .into_iter()
    .map(|(key, value)| crate::choice::ChoiceItem::option(value, &crate::t_pub(key)))
    .collect()
}

fn scope_pairs() -> Vec<(String, String)> {
    scope_items()
        .into_iter()
        .map(|item| (item.value, item.label))
        .collect()
}

const SELECTION_MODES: [core_ctx::SelectionMode; 3] = [
    core_ctx::SelectionMode::Single,
    core_ctx::SelectionMode::Each,
    core_ctx::SelectionMode::Together,
];
fn selection_pairs() -> Vec<(String, String)> {
    [
        "ctx_selection_single",
        "ctx_selection_each",
        "ctx_selection_together",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, key)| (index.to_string(), crate::t_pub(key)))
    .collect()
}
fn selection_index(mode: core_ctx::SelectionMode) -> i32 {
    SELECTION_MODES
        .iter()
        .position(|value| *value == mode)
        .unwrap_or(1) as i32
}

fn scope_row_index(scope: &str) -> i32 {
    [
        core_ctx::SCOPE_FILE,
        core_ctx::SCOPE_DIR,
        core_ctx::SCOPE_FILE_DIR,
        core_ctx::SCOPE_BACKGROUND,
        core_ctx::SCOPE_ALL,
    ]
    .iter()
    .position(|value| *value == scope)
    .unwrap_or(0) as i32
}

pub fn theme_hwnd() -> HWND {
    mgr()
}

pub fn default_button(window: HWND) -> Option<HWND> {
    (window == mgr() && !window.is_invalid()).then(|| {
        get_dlg_item(
            window,
            match VIEW.load(Ordering::SeqCst) {
                VIEW_EDIT => ID_SAVE,
                VIEW_OPTIONS => ID_CLOSE,
                _ if selected_index().is_some_and(|index| {
                    crate::runtime::lock(&DISPLAYED)
                        .get(index)
                        .is_some_and(|rule| !rule.separator)
                }) =>
                {
                    ID_EDIT
                }
                _ => ID_ADD,
            },
        )
    })
}

pub(crate) fn handle_key(window: HWND, key: usize) -> bool {
    if window != mgr() || window.is_invalid() {
        return false;
    }
    let view = VIEW.load(Ordering::SeqCst);
    if key == 0x1b && view == VIEW_EDIT {
        if leave_draft() {
            finish_edit(false);
        }
        return true;
    }
    if key == 0x53
        && view == VIEW_EDIT
        && unsafe {
            windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(
                windows::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL.0 as i32,
            ) < 0
        }
    {
        if unsafe { IsWindowEnabled(get_dlg_item(mgr(), ID_SAVE)).as_bool() } && apply_form() {
            finish_edit(true);
        }
        return true;
    }
    if view == VIEW_LIST && unsafe { GetFocus() } == list_hwnd() {
        match key {
            0x71 => {
                edit_selected();
                return true;
            } // F2
            0x2e => {
                delete_selected();
                return true;
            } // Delete
            _ => (),
        }
    }
    false
}

fn quote_pairs() -> Vec<(String, String)> {
    [
        ("auto", "ctx_quote_auto"),
        ("always", "ctx_quote_always"),
        ("none", "ctx_quote_none"),
    ]
    .into_iter()
    .map(|(value, key)| (value.into(), crate::t_pub(key)))
    .collect()
}

fn separator_pairs() -> Vec<(String, String)> {
    [("newline", "ctx_sep_newline"), ("space", "ctx_sep_space")]
        .into_iter()
        .map(|(value, key)| (value.into(), crate::t_pub(key)))
        .collect()
}

fn visible(id: usize, show: bool) {
    let control = get_dlg_item(mgr(), id);
    if !control.is_invalid() {
        unsafe {
            let _ = ShowWindow(control, if show { SW_SHOW } else { SW_HIDE });
        }
    }
}

fn refresh_editor_visibility() {
    let editing = VIEW.load(Ordering::SeqCst) == VIEW_EDIT;
    let advanced = editing && ADVANCED.load(Ordering::SeqCst);
    visible(ID_EDITOR_GUIDE, editing && !advanced);
    for id in [
        ID_WORK_L, FIELD_WORK, ID_WORK_B, ID_ICON_L, ID_ICON_E, ID_ICON_B, ID_CONSOLE,
    ] {
        visible(id, advanced);
    }
    let selection = editing
        && crate::choice::value(get_dlg_item(mgr(), ID_SCOPE_C)) != core_ctx::SCOPE_BACKGROUND;
    for id in [ID_SELECTION_L, FIELD_SELECTION] {
        visible(id, selection);
    }
    set_text(
        ID_ADVANCED,
        &crate::t_pub(if advanced {
            "ctx_advanced_hide"
        } else {
            "ctx_advanced_show"
        }),
    );
    crate::accessibility::check(get_dlg_item(mgr(), ID_ADVANCED), advanced);
    let card = get_dlg_item(mgr(), ID_FORM);
    let mut bounds = RECT::default();
    let height = s(if advanced { 368 } else { 256 });
    unsafe {
        if GetWindowRect(card, &mut bounds).is_ok() && bounds.bottom - bounds.top != height {
            let _ = SetWindowPos(
                card,
                None,
                0,
                0,
                bounds.right - bounds.left,
                height,
                SWP_NOMOVE | SWP_NOACTIVATE | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
            );
        }
    }
}

fn apply_view() {
    let view = VIEW.load(Ordering::SeqCst);
    for id in [
        ID_SURF,
        ID_LIST,
        ID_ADD,
        ID_EDIT,
        ID_ADD_SEP,
        ID_UP,
        ID_DOWN,
        ID_DELETE,
        ID_LIST_HINT,
        ID_TOGGLE,
    ] {
        visible(id, view == VIEW_LIST);
    }
    visible(
        ID_EMPTY,
        view == VIEW_LIST && LIST_ROW_COUNT.load(Ordering::SeqCst) == 0,
    );
    for id in [
        ID_BACK,
        ID_FORM_TITLE,
        ID_ENABLED,
        ID_FORM,
        ID_TITLE_L,
        ID_TITLE_E,
        ID_SCOPE_L,
        ID_SCOPE_C,
        ID_PROG_L,
        ID_PROG_E,
        ID_PROG_B,
        ID_ARGS_L,
        ID_ARGS_E,
        ID_MATCH_L,
        ID_MATCH_E,
        ID_ADVANCED,
        ID_VALID,
        ID_PREVIEW,
        ID_DISCARD,
        ID_SAVE,
    ] {
        visible(id, view == VIEW_EDIT);
    }
    for id in [
        ID_OPTIONS_HINT,
        ID_LAYOUT_TITLE,
        ID_LAYOUT_CARD,
        ID_GROUPED,
        ID_LAYOUT_HINT,
        ID_COPY_TITLE,
        ID_COPY_CARD,
        ID_COPY,
        ID_QUOTE_L,
        ID_QUOTE,
        ID_SEPARATOR_L,
        ID_SEPARATOR,
        ID_OPTIONS_STATUS,
    ] {
        visible(id, view == VIEW_OPTIONS);
    }
    visible(
        ID_CLASSIC,
        view == VIEW_OPTIONS && crate::system::is_windows_11(),
    );
    visible(ID_CLOSE, view != VIEW_EDIT);
    refresh_editor_visibility();
    for (id, active) in [
        (ID_TAB_RULES, view != VIEW_OPTIONS),
        (ID_MORE, view == VIEW_OPTIONS),
    ] {
        crate::accessibility::check(get_dlg_item(mgr(), id), active);
    }
    unsafe {
        lower_cards(mgr());
        let _ = windows::Win32::Graphics::Gdi::RedrawWindow(
            Some(mgr()),
            None,
            None,
            windows::Win32::Graphics::Gdi::RDW_INVALIDATE
                | windows::Win32::Graphics::Gdi::RDW_ALLCHILDREN,
        );
    }
}

fn select_view(view: usize) {
    crate::choice::close(false);
    crate::tooltips::dismiss_focus_tip(unsafe { GetFocus() });
    VIEW.store(view, Ordering::SeqCst);
    refresh_options();
    apply_view();
    let target = match view {
        VIEW_OPTIONS => ID_GROUPED,
        VIEW_EDIT => ID_PROG_E,
        _ if selected_index().is_some() => ID_LIST,
        _ => ID_ADD,
    };
    unsafe {
        let control = get_dlg_item(mgr(), target);
        let _ = SetFocus(Some(control));
        crate::viewport::reveal_control(mgr(), control);
    }
}

fn finish_edit(saved: bool) {
    let id = crate::runtime::lock(&DRAFT)
        .as_ref()
        .map(|rule| rule.id.clone());
    *crate::runtime::lock(&DRAFT) = None;
    refresh_list();
    let index = crate::runtime::lock(&DISPLAYED)
        .iter()
        .position(|rule| Some(&rule.id) == id.as_ref());
    select_index(index.map_or(-1, |index| index as i32));
    refresh_form_buttons();
    select_view(VIEW_LIST);
    set_text(
        ID_LIST_HINT,
        &crate::t_pub(if !saved {
            "ctx_list_hint"
        } else if crate::cfg_map(|c| c.ctx_menu_enabled) {
            "ctx_rule_saved"
        } else {
            "ctx_rule_saved_off"
        }),
    );
}

fn edit_selected() {
    let Some(rule) =
        selected_index().and_then(|index| crate::runtime::lock(&DISPLAYED).get(index).cloned())
    else {
        return;
    };
    if rule.separator {
        return;
    }
    ADVANCED.store(
        !rule.working_dir.is_empty() || !rule.icon.is_empty() || rule.hidden_console,
        Ordering::SeqCst,
    );
    *crate::runtime::lock(&DRAFT) = Some(rule);
    load_form();
    select_view(VIEW_EDIT);
}

fn refresh_options() {
    if mgr().is_invalid() {
        return;
    }
    let config = crate::cfg_map(Clone::clone);
    set_text(
        ID_LAYOUT_HINT,
        &crate::t_pub(if config.ctx_menu_grouped {
            "ctx_layout_hint"
        } else {
            "ctx_layout_hint_flat"
        }),
    );
    let classic = matches!(
        crate::ctxmenu::classic_state(),
        crate::ctxmenu::ClassicState::ByUs
            | crate::ctxmenu::ClassicState::ByOther
            | crate::ctxmenu::ClassicState::Legacy
    );
    for (id, checked) in [
        (ID_GROUPED, config.ctx_menu_grouped),
        (ID_COPY, config.ctx_copy_enabled),
        (ID_CLASSIC, classic),
    ] {
        let control = get_dlg_item(mgr(), id);
        crate::accessibility::check(control, checked);
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, true);
        }
    }
    for (id, values, selected) in [
        (ID_QUOTE, quote_pairs(), config.ctx_copy_quote),
        (ID_SEPARATOR, separator_pairs(), config.ctx_copy_separator),
    ] {
        let index = values
            .iter()
            .position(|(value, _)| *value == selected)
            .unwrap_or(0);
        crate::choice::select_index(get_dlg_item(mgr(), id), index as i32);
        unsafe {
            let _ = EnableWindow(get_dlg_item(mgr(), id), config.ctx_copy_enabled);
        }
    }
    unsafe {
        let _ = EnableWindow(
            get_dlg_item(mgr(), ID_CLASSIC),
            !crate::settings_ui::classic_busy(),
        );
    }
    set_text(
        ID_OPTIONS_STATUS,
        &if crate::settings_ui::classic_busy() {
            crate::t_pub("ctx_classic_busy")
        } else {
            crate::settings_ui::conflict_text()
        },
    );
}

fn change_option(id: usize) {
    let value = crate::choice::value(get_dlg_item(mgr(), id));
    let before = crate::cfg_map(|config| match id {
        ID_GROUPED => config.ctx_menu_grouped,
        _ => config.ctx_copy_enabled,
    });
    let result = crate::edit_config(|config| match id {
        ID_GROUPED => config.ctx_menu_grouped = !config.ctx_menu_grouped,
        ID_COPY => config.ctx_copy_enabled = !config.ctx_copy_enabled,
        ID_QUOTE if ["auto", "always", "none"].contains(&value.as_str()) => {
            config.ctx_copy_quote = value
        }
        ID_SEPARATOR if ["newline", "space"].contains(&value.as_str()) => {
            config.ctx_copy_separator = value
        }
        _ => (),
    });
    if let Err(error) = result {
        crate::warn_dialog("", &error);
    } else {
        crate::ctxmenu::sync("context-options");
        crate::settings_ui::sync_ctx_options();
        if matches!(id, ID_GROUPED | ID_COPY) {
            crate::start_switch_animation(get_dlg_item(mgr(), id), before, !before);
        }
    }
    refresh_options();
    refresh_manager_status();
}

pub fn show() {
    let _dpi = crate::dpi::Scope::window(crate::hwnd(&crate::PANEL));
    unsafe {
        ensure_created();
        let window = mgr();
        if crate::runtime::lock(&DRAFT).is_none() {
            let selected = selected_index();
            refresh_list();
            select_index(selected.map_or(0, |index| index as i32));
            load_form();
        }
        refresh_options();
        apply_view();
        theme::retheme_children(window);
        refresh_manager_status();
        crate::tooltips::refresh_all(window);
        crate::hold_panel_modal(window);
        if crate::viewport::metrics(window).is_none() {
            crate::viewport::fit(window);
        }
        // Pin the cards beneath their rows again right before reveal: a card
        // floating above its fields clips their owner-draw DCs to nothing —
        // the blank-manager failure the automation windows guard against with
        // lower_surfaces at every show/layout pass.
        lower_cards(window);
        crate::FirstFrameGate::begin(window).reveal();
        let _ = SetForegroundWindow(window);
        let target = if VIEW.load(Ordering::SeqCst) == VIEW_OPTIONS {
            get_dlg_item(window, ID_GROUPED)
        } else if VIEW.load(Ordering::SeqCst) == VIEW_EDIT {
            get_dlg_item(window, ID_PROG_E)
        } else if LIST_ROW_COUNT.load(Ordering::SeqCst) == 0 {
            get_dlg_item(window, ID_ADD)
        } else {
            list_hwnd()
        };
        if !target.is_invalid() {
            let _ = SetFocus(Some(target));
        }
    }
}

fn hide() {
    if !leave_draft() {
        return;
    }
    *crate::runtime::lock(&DRAFT) = None;
    VIEW.store(VIEW_LIST, Ordering::SeqCst);
    unsafe {
        let _ = ShowWindow(mgr(), SW_HIDE);
        crate::release_panel_modal(mgr());
        // Rules may have changed behind the panel's status line.
        crate::refresh_status();
    }
}

/// Re-pins the card surfaces to the bottom of the sibling z-order. Native
/// creation/layout behavior can float a card above the rows it hosts, and a
/// floating card blanks every hosted field (automation lower_surfaces
/// parity); idempotent, so every show and size pass may call it freely.
unsafe fn lower_cards(window: HWND) {
    unsafe {
        for id in [ID_SURF, ID_FORM, ID_LAYOUT_CARD, ID_COPY_CARD] {
            let card = get_dlg_item(window, id);
            if !card.is_invalid() {
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
            }
        }
    }
}

/// Refreshes captions after a language swap (refresh_language hook).
pub fn refresh_language() {
    let window = mgr();
    if window.is_invalid() {
        return;
    }
    let has_draft = crate::runtime::lock(&DRAFT).is_some();
    let edited = has_draft.then(draft_from_form);
    let selected = selected_index();
    for (id, key) in [
        (ID_ENABLED, "ctx_rule_enabled"),
        (ID_WORK_L, "ctx_working_dir"),
        (ID_PREVIEW, "ctx_preview"),
        (ID_FORM_TITLE, "ctx_form_title"),
        (ID_TITLE_L, "ctx_field_title"),
        (ID_SCOPE_L, "ctx_field_scope"),
        (ID_SELECTION_L, "ctx_selection_mode"),
        (ID_PROG_L, "ctx_field_program"),
        (ID_ARGS_L, "ctx_field_args"),
        (ID_ICON_L, "ctx_field_icon"),
        (ID_MATCH_L, "ctx_field_match"),
        (ID_CONSOLE, "ctx_console_label"),
        (ID_ADD, "ctx_add"),
        (ID_ADD_SEP, "ctx_add_separator"),
        (ID_DELETE, "ctx_delete"),
        (ID_UP, "ctx_up"),
        (ID_DOWN, "ctx_down"),
        (ID_SAVE, "ctx_save_rule"),
        (ID_DISCARD, "common_cancel"),
        (ID_MORE, "ctx_tab_options"),
        (ID_TAB_RULES, "ctx_tab_rules"),
        (ID_EDIT, "ctx_edit"),
        (ID_BACK, "ctx_back"),
        (ID_CLOSE, "ctx_close"),
        (ID_REPAIR, "ctx_repair_action"),
        (ID_LIST_HINT, "ctx_list_hint"),
        (ID_OPTIONS_HINT, "ctx_options_hint"),
        (ID_LAYOUT_TITLE, "ctx_layout_title"),
        (ID_GROUPED, "ctx_submenu_label"),
        (ID_LAYOUT_HINT, "ctx_layout_hint"),
        (ID_CLASSIC, "ctx_win11_switch"),
        (ID_COPY_TITLE, "ctx_copy_section"),
        (ID_COPY, "settings_ctx_copy"),
        (ID_QUOTE_L, "ctx_quote_label"),
        (ID_SEPARATOR_L, "ctx_sep_label"),
        (ID_MASTER, "ctx_master_live"),
        (ID_EDITOR_GUIDE, "ctx_editor_guide"),
    ] {
        set_text(id, &crate::t_pub(key));
    }
    set_text(ID_PROG_B, &crate::t_pub("ctx_field_browse"));
    set_text(ID_ICON_B, &crate::t_pub("ctx_field_browse"));
    set_text(ID_WORK_B, &crate::t_pub("ctx_field_browse"));
    let title = crate::t_pub("ctx_manager_title");
    unsafe {
        let _ = SetWindowTextW(window, PCWSTR(wide(&title).as_ptr()));
        let _ = SetWindowTextW(
            get_dlg_item(window, ID_TITLE),
            PCWSTR(wide(&title).as_ptr()),
        );
    }
    crate::choice::set_rows(get_dlg_item(window, ID_SCOPE_C), &scope_items());
    for (id, pairs) in [(ID_QUOTE, quote_pairs()), (ID_SEPARATOR, separator_pairs())] {
        crate::choice::set_rows(
            get_dlg_item(window, id),
            &pairs
                .iter()
                .map(|(value, label)| crate::choice::ChoiceItem::option(value, label))
                .collect::<Vec<_>>(),
        );
    }
    crate::choice::set_rows(
        get_dlg_item(window, FIELD_SELECTION),
        &selection_pairs()
            .iter()
            .map(|(value, label)| crate::choice::ChoiceItem::option(value, label))
            .collect::<Vec<_>>(),
    );
    // Repaint captions from the existing snapshot; language changes must
    // neither adopt a new conflict baseline nor discard typed fields.
    render_list();
    if let Some(index) = selected {
        select_index(index as i32);
    }
    if let Some(edited) = edited {
        crate::choice::select_index(
            get_dlg_item(window, FIELD_SELECTION),
            selection_index(edited.selection_mode),
        );
        crate::choice::select_index(
            get_dlg_item(window, ID_SCOPE_C),
            scope_row_index(&edited.scope),
        );
    }
    refresh_manager_status();
    crate::tooltips::refresh_all(window);
    refresh_form_buttons();
    refresh_options();
    apply_view();
}

fn refresh_manager_status() {
    let control = get_dlg_item(mgr(), ID_MASTER);
    crate::accessibility::check(control, crate::cfg_map(|c| c.ctx_menu_enabled));
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, true);
    }
    let key = if crate::ctxmenu::registration_blocked() {
        "ctx_status_config_error"
    } else if !crate::cfg_map(|c| c.ctx_menu_enabled) {
        "ctx_manager_disabled"
    } else if !crate::ctxmenu::registration_healthy() {
        "ctx_manager_drift"
    } else if crate::system::is_windows_11()
        && crate::ctxmenu::classic_state() == crate::ctxmenu::ClassicState::Off
    {
        "ctx_manager_ready_more"
    } else {
        "ctx_manager_ready"
    };
    set_text(ID_STATUS, &crate::t_pub(key));
    visible(ID_REPAIR, key == "ctx_manager_drift");
}

/// Refresh live state without replacing the user's draft or conflict baseline.
pub fn refresh_config_state() {
    let window = mgr();
    if window.is_invalid() {
        return;
    }
    let _dpi = crate::dpi::Scope::window(window);
    if crate::runtime::lock(&DRAFT).is_none()
        && *crate::runtime::lock(&DISPLAYED) != *crate::runtime::lock(&crate::ctxexec::CTX_RULES)
    {
        let selected = selected_index().and_then(|index| {
            crate::runtime::lock(&DISPLAYED)
                .get(index)
                .map(|rule| rule.id.clone())
        });
        refresh_list();
        let index = crate::runtime::lock(&DISPLAYED)
            .iter()
            .position(|rule| Some(&rule.id) == selected.as_ref());
        select_index(index.map_or(-1, |index| index as i32));
    }
    refresh_form_buttons();
    refresh_manager_status();
    refresh_options();
}

/// Centers the manager over the control panel (settings parity).
fn center_on_panel(win_w: i32, win_h: i32) -> (i32, i32) {
    let panel = crate::hwnd(&crate::PANEL);
    let mut rect = RECT::default();
    let placed =
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(panel, &mut rect) }.is_ok();
    let (cx, cy) = if placed {
        ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
    } else {
        (
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                    windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN,
                )
            } / 2,
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                    windows::Win32::UI::WindowsAndMessaging::SM_CYSCREEN,
                )
            } / 2,
        )
    };
    (cx - win_w / 2, cy - win_h / 2)
}

// ---- Window ---------------------------------------------------------------

unsafe fn ensure_created() {
    // MAKEINTRESOURCE(1): the app icon resource (automation-manager parity).
    #[allow(clippy::manual_dangling_ptr)]
    let icon_resource = PCWSTR(1usize as *const u16);
    unsafe {
        if !mgr().is_invalid() {
            return;
        }
        let instance =
            windows::Win32::System::LibraryLoader::GetModuleHandleW(None).expect("module handle");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(mgr_proc),
            hInstance: instance.into(),
            lpszClassName: windows::core::w!("IdleTriggerCtxMgr"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: windows::Win32::UI::WindowsAndMessaging::LoadIconW(
                Some(instance.into()),
                icon_resource,
            )
            .unwrap_or_default(),
            hbrBackground: windows::Win32::Graphics::Gdi::GetSysColorBrush(
                windows::Win32::Graphics::Gdi::COLOR_WINDOW,
            ),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let style = WINDOW_STYLE(
            (windows::Win32::UI::WindowsAndMessaging::WS_OVERLAPPEDWINDOW.0
                & !windows::Win32::UI::WindowsAndMessaging::WS_MAXIMIZEBOX.0
                & !windows::Win32::UI::WindowsAndMessaging::WS_THICKFRAME.0
                & !windows::Win32::UI::WindowsAndMessaging::WS_MINIMIZEBOX.0)
                | WS_CLIPCHILDREN.0,
        );
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: s(CLIENT_W),
            bottom: s(CLIENT_H),
        };
        let _ = windows::Win32::UI::WindowsAndMessaging::AdjustWindowRectEx(
            &mut frame,
            style,
            false,
            WINDOW_EX_STYLE(0),
        );
        let win_w = frame.right - frame.left;
        let win_h = frame.bottom - frame.top;
        let (x, y) = center_on_panel(win_w, win_h);

        let title = crate::t_pub("ctx_manager_title");
        let window = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("IdleTriggerCtxMgr"),
            PCWSTR(wide(&title).as_ptr()),
            style,
            x,
            y,
            win_w,
            win_h,
            Some(crate::hwnd(&crate::PANEL)),
            None,
            Some(instance.into()),
            None,
        )
        .expect("ctx manager window");
        MGR_HWND.store(window.0 as isize, Ordering::SeqCst);
        crate::dpi::install(window);
        theme::apply_to_window(window);
        crate::set_window_icons_pub(window);
        build_controls(window, instance);
        crate::tooltips::register_controls(window);
        theme::retheme_children(window);
        let _ = windows::Win32::Graphics::Gdi::RedrawWindow(
            Some(window),
            None,
            None,
            windows::Win32::Graphics::Gdi::RDW_INVALIDATE
                | windows::Win32::Graphics::Gdi::RDW_ERASE
                | windows::Win32::Graphics::Gdi::RDW_ALLCHILDREN
                | windows::Win32::Graphics::Gdi::RDW_FRAME,
        );
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(window);
    }
}

unsafe fn owner_button(window: HWND, id: usize, key: &str, x: i32, y: i32, w: i32, h: i32) -> HWND {
    unsafe {
        let btn = mk_control(
            window,
            windows::core::w!("BUTTON"),
            &crate::t_pub(key),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | 0x0000000B), // BS_OWNERDRAW
            id,
            x,
            y,
            w,
            h,
            body_font(),
        );
        crate::nativeform::track(btn);
        btn
    }
}

/// Field label above its slot (editor label grammar).
unsafe fn form_label(window: HWND, id: usize, key: &str, x: i32, y: i32, w: i32) {
    unsafe {
        mk_control(
            window,
            windows::core::w!("STATIC"),
            &crate::t_pub(key),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0x0080), // SS_NOPREFIX
            id,
            x,
            y,
            w,
            LABEL_H,
            label_font(),
        );
    }
}

/// Bare inset edit: the hosting card paints the surrounding well, so the
/// control itself stays borderless and inset inside the slot (settings
/// field grammar: x+3, y+7, w-6, h-14).
unsafe fn form_edit(window: HWND, id: usize, x: i32, y: i32, w: i32) {
    unsafe {
        let edit = mk_control(
            window,
            windows::core::w!("EDIT"),
            "",
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | WS_CLIPSIBLINGS.0 | 0x0080), // ES_AUTOHSCROLL
            id,
            x + 3,
            y + 7,
            w - 6,
            FIELD_H - 14,
            body_font(),
        );
        // Go editWithStyle: 6px inner margins.
        let margin = s(6) as usize;
        let _ = SendMessageW(
            edit,
            EM_SETMARGINS_RAW,
            Some(WPARAM(3)),
            Some(LPARAM((margin | (margin << 16)) as isize)),
        );
    }
}

unsafe fn build_controls(window: HWND, instance: windows::Win32::Foundation::HMODULE) {
    unsafe {
        mk_control(
            window,
            windows::core::w!("STATIC"),
            &crate::t_pub("ctx_manager_title"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0x0080),
            ID_TITLE,
            EDGE,
            HEADER_Y,
            280,
            HEADER_H,
            crate::make_font_pub(18, 600),
        );
        let master = owner_button(window, ID_MASTER, "ctx_master_live", 360, 16, 304, ACT_H);
        crate::nativeform::set_hover_column(master, s(crate::layout::SWITCH_HIT_W));
        form_label(window, ID_STATUS, "ctx_manager_disabled", EDGE, 56, 516);
        owner_button(window, ID_REPAIR, "ctx_repair_action", 552, 48, 112, ACT_H);
        owner_button(window, ID_TAB_RULES, "ctx_tab_rules", EDGE, 88, 144, ACT_H);
        owner_button(window, ID_MORE, "ctx_tab_options", 168, 88, 144, ACT_H);
        for (id, key, x, width) in [
            (ID_ADD, "ctx_add", EDGE, ACT_W),
            (ID_EDIT, "ctx_edit", 152, ACT_W),
            (ID_ADD_SEP, "ctx_add_separator", 288, ACT_W),
        ] {
            owner_button(window, id, key, x, ACTIONS_Y, width, ACT_H);
        }
        // List card, then the list above it.
        let surf = surface_card(window, ID_SURF, EDGE, LIST_Y, CLIENT_W - 2 * EDGE, LIST_H);
        let list = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WC_LISTBOX,
            windows::core::w!(""),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_TABSTOP.0
                    | WS_CLIPSIBLINGS.0
                    | 0x0001 // LBS_NOTIFY
                    | 0x0010 // LBS_OWNERDRAWFIXED
                    | 0x0040 // LBS_HASSTRINGS
                    | 0x0100, // LBS_NOINTEGRALHEIGHT
            ),
            s(EDGE + 2),
            s(LIST_Y + 2),
            s(CLIENT_W - 2 * EDGE - 4),
            s(LIST_H - 4),
            Some(window),
            Some(HMENU(ID_LIST as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("ctx rule list");
        let _ = SendMessageW(
            list,
            WM_SETFONT,
            Some(WPARAM(body_font().0 as usize)),
            Some(LPARAM(1)),
        );
        LIST_HWND.store(list.0 as isize, Ordering::SeqCst);
        crate::list_style::install(list);
        crate::list_style::set_row_height(list, 52);
        crate::list_style::set_lane_card(list, surf);
        mk_control(
            window,
            windows::core::w!("STATIC"),
            &crate::t_pub("ctx_manager_empty"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0x0080 | 0x0001), // SS_NOPREFIX | SS_CENTER
            ID_EMPTY,
            EDGE + 2,
            LIST_Y + (LIST_H - 48) / 2,
            CLIENT_W - 2 * EDGE - 4,
            48,
            label_font(),
        );

        form_label(
            window,
            ID_LIST_HINT,
            "ctx_list_hint",
            EDGE,
            560,
            CLIENT_W - 2 * EDGE,
        );
        owner_button(
            window,
            ID_TOGGLE,
            "ctx_disable_rule",
            EDGE,
            FOOTER_Y,
            112,
            32,
        );

        for (id, key, x, width) in [
            (ID_UP, "ctx_up", 136, 112),
            (ID_DOWN, "ctx_down", 256, 112),
            (ID_DELETE, "ctx_delete", 376, 112),
        ] {
            owner_button(window, id, key, x, FOOTER_Y, width, 32);
        }

        owner_button(window, ID_BACK, "ctx_back", EDGE, ACTIONS_Y, 104, ACT_H);
        mk_control(
            window,
            windows::core::w!("STATIC"),
            "",
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0x0080),
            ID_FORM_TITLE,
            136,
            FORM_TITLE_Y,
            252,
            22,
            section_font(),
        );
        let enabled = owner_button(
            window,
            ID_ENABLED,
            "ctx_rule_enabled",
            420,
            ACTIONS_Y,
            244,
            ACT_H,
        );
        crate::nativeform::set_hover_column(enabled, s(crate::layout::SWITCH_HIT_W));
        surface_card(window, ID_FORM, EDGE, FORM_Y, CLIENT_W - 2 * EDGE, 368);

        let field1 = FORM_Y + 12 + LABEL_H + LABEL_GAP;
        form_label(
            window,
            ID_PROG_L,
            "ctx_field_program",
            COL_A,
            field1 - LABEL_H - LABEL_GAP,
            COL_W,
        );
        form_label(
            window,
            ID_ARGS_L,
            "ctx_field_args",
            COL_B,
            field1 - LABEL_H - LABEL_GAP,
            COL_W,
        );
        form_edit(window, ID_PROG_E, COL_A, field1, COL_W - BROWSE_W - 8);
        owner_button(
            window,
            ID_PROG_B,
            "ctx_field_browse",
            COL_A + COL_W - BROWSE_W,
            field1,
            BROWSE_W,
            FIELD_H,
        );
        form_edit(window, ID_ARGS_E, COL_B, field1, COL_W);

        let field2 = field1 + FIELD_H + SLOT_GAP + LABEL_H + LABEL_GAP;
        form_label(
            window,
            ID_TITLE_L,
            "ctx_field_title",
            COL_A,
            field2 - LABEL_H - LABEL_GAP,
            COL_W,
        );
        form_label(
            window,
            ID_SCOPE_L,
            "ctx_field_scope",
            COL_B,
            field2 - LABEL_H - LABEL_GAP,
            COL_W,
        );
        form_edit(window, ID_TITLE_E, COL_A, field2, COL_W);
        crate::choice::create(
            window,
            ID_SCOPE_C as i32,
            (COL_B, field2, COL_W, FIELD_H),
            &scope_pairs(),
            body_font(),
        );

        let field3 = field2 + FIELD_H + SLOT_GAP + LABEL_H + LABEL_GAP;
        form_label(
            window,
            ID_MATCH_L,
            "ctx_field_match",
            COL_A,
            field3 - LABEL_H - LABEL_GAP,
            COL_W,
        );
        form_label(
            window,
            ID_SELECTION_L,
            "ctx_selection_mode",
            COL_B,
            field3 - LABEL_H - LABEL_GAP,
            COL_W,
        );
        form_edit(window, ID_MATCH_E, COL_A, field3, COL_W);
        crate::choice::create(
            window,
            FIELD_SELECTION as i32,
            (COL_B, field3, COL_W, FIELD_H),
            &selection_pairs(),
            body_font(),
        );

        owner_button(
            window,
            ID_ADVANCED,
            "ctx_advanced_show",
            COL_A,
            396,
            176,
            32,
        );
        form_label(window, ID_WORK_L, "ctx_working_dir", COL_A, 440, COL_W);
        form_label(window, ID_ICON_L, "ctx_field_icon", COL_B, 440, COL_W);
        form_edit(window, FIELD_WORK, COL_A, 464, COL_W - BROWSE_W - 8);
        form_edit(window, ID_ICON_E, COL_B, 464, COL_W - BROWSE_W - 8);
        owner_button(
            window,
            ID_WORK_B,
            "ctx_field_browse",
            COL_A + COL_W - BROWSE_W,
            464,
            BROWSE_W,
            FIELD_H,
        );
        owner_button(
            window,
            ID_ICON_B,
            "ctx_field_browse",
            COL_B + COL_W - BROWSE_W,
            464,
            BROWSE_W,
            FIELD_H,
        );
        let console = owner_button(
            window,
            ID_CONSOLE,
            "ctx_console_label",
            COL_A,
            508,
            COL_W,
            ACT_H,
        );
        crate::nativeform::set_hover_column(console, s(crate::layout::SWITCH_HIT_W));

        mk_control(
            window,
            windows::core::w!("STATIC"),
            "",
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0x0080),
            ID_VALID,
            EDGE,
            560,
            648,
            32,
            label_font(),
        );
        owner_button(window, ID_PREVIEW, "ctx_preview", EDGE, FOOTER_Y, 112, 32);
        owner_button(window, ID_DISCARD, "common_cancel", 432, FOOTER_Y, 112, 32);
        owner_button(window, ID_SAVE, "ctx_save_rule", 552, FOOTER_Y, 112, 32);

        mk_control(
            window,
            windows::core::w!("STATIC"),
            &crate::t_pub("ctx_editor_guide"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0x0080),
            ID_EDITOR_GUIDE,
            COL_A,
            456,
            624,
            64,
            label_font(),
        );

        form_label(
            window,
            ID_OPTIONS_HINT,
            "ctx_options_hint",
            EDGE,
            ACTIONS_Y,
            648,
        );
        form_label(window, ID_LAYOUT_TITLE, "ctx_layout_title", EDGE, 176, 648);
        let layout_h = if crate::system::is_windows_11() {
            128
        } else {
            84
        };
        surface_card(window, ID_LAYOUT_CARD, EDGE, 208, 648, layout_h);
        let grouped = owner_button(
            window,
            ID_GROUPED,
            "ctx_submenu_label",
            IN_X,
            220,
            624,
            ACT_H,
        );
        crate::nativeform::set_hover_column(grouped, s(crate::layout::SWITCH_HIT_W));
        form_label(window, ID_LAYOUT_HINT, "ctx_layout_hint", IN_X, 260, 624);
        let classic = owner_button(
            window,
            ID_CLASSIC,
            "ctx_win11_switch",
            IN_X,
            292,
            624,
            ACT_H,
        );
        crate::nativeform::set_hover_column(classic, s(crate::layout::SWITCH_HIT_W));
        let copy_y = 208 + layout_h + 16;
        form_label(window, ID_COPY_TITLE, "ctx_copy_section", EDGE, copy_y, 648);
        surface_card(window, ID_COPY_CARD, EDGE, copy_y + 28, 648, 148);
        let copy = owner_button(
            window,
            ID_COPY,
            "settings_ctx_copy",
            IN_X,
            copy_y + 40,
            624,
            32,
        );
        crate::nativeform::set_hover_column(copy, s(crate::layout::SWITCH_HIT_W));
        form_label(
            window,
            ID_QUOTE_L,
            "ctx_quote_label",
            IN_X,
            copy_y + 94,
            224,
        );
        form_label(
            window,
            ID_SEPARATOR_L,
            "ctx_sep_label",
            IN_X,
            copy_y + 138,
            224,
        );
        for (id, y, choices) in [
            (ID_QUOTE, copy_y + 88, quote_pairs()),
            (ID_SEPARATOR, copy_y + 132, separator_pairs()),
        ] {
            crate::choice::create(
                window,
                id as i32,
                (COL_B, y, COL_W, FIELD_H),
                &choices,
                body_font(),
            );
        }
        form_label(window, ID_OPTIONS_STATUS, "", EDGE, 560, 648);
        owner_button(window, ID_CLOSE, "ctx_close", 552, FOOTER_Y, 112, 32);
        apply_view();
    }
}

unsafe extern "system" fn mgr_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    crate::guarded_proc(
        "context manager",
        hwnd,
        msg,
        wparam,
        lparam,
        move || unsafe {
            let _dpi = crate::dpi::Scope::window(hwnd);
            match msg {
                WM_COMMAND => {
                    let id = wparam.0 & 0xFFFF;
                    let code = wparam.0 >> 16;
                    match id {
                        ID_MASTER if code == BN_CLICKED as usize => {
                            let before = crate::cfg_map(|c| c.ctx_menu_enabled);
                            if let Err(err) =
                                crate::edit_config(|c| c.ctx_menu_enabled = !c.ctx_menu_enabled)
                            {
                                crate::warn_dialog("", &err);
                            } else {
                                crate::ctxmenu::sync("rule-manager");
                                crate::settings_ui::sync_ctx_options();
                                crate::start_switch_animation(
                                    get_dlg_item(hwnd, ID_MASTER),
                                    before,
                                    !before,
                                );
                            }
                            refresh_manager_status();
                            LRESULT(0)
                        }
                        ID_MORE if code == BN_CLICKED as usize => {
                            select_view(VIEW_OPTIONS);
                            LRESULT(0)
                        }
                        ID_TAB_RULES if code == BN_CLICKED as usize => {
                            let has_draft = crate::runtime::lock(&DRAFT).is_some();
                            select_view(if has_draft { VIEW_EDIT } else { VIEW_LIST });
                            LRESULT(0)
                        }
                        ID_EDIT if code == BN_CLICKED as usize => {
                            edit_selected();
                            LRESULT(0)
                        }
                        ID_TOGGLE if code == BN_CLICKED as usize => {
                            toggle_selected();
                            LRESULT(0)
                        }
                        ID_BACK if code == BN_CLICKED as usize => {
                            if leave_draft() {
                                finish_edit(false);
                            }
                            LRESULT(0)
                        }
                        ID_CLOSE if code == BN_CLICKED as usize => {
                            hide();
                            LRESULT(0)
                        }
                        ID_ADVANCED if code == BN_CLICKED as usize => {
                            ADVANCED.fetch_xor(true, Ordering::SeqCst);
                            refresh_editor_visibility();
                            let _ = windows::Win32::Graphics::Gdi::RedrawWindow(
                                Some(hwnd),
                                None,
                                None,
                                windows::Win32::Graphics::Gdi::RDW_INVALIDATE
                                    | windows::Win32::Graphics::Gdi::RDW_ALLCHILDREN,
                            );
                            LRESULT(0)
                        }
                        ID_GROUPED | ID_COPY if code == BN_CLICKED as usize => {
                            change_option(id);
                            LRESULT(0)
                        }
                        ID_QUOTE | ID_SEPARATOR if code == BN_CLICKED as usize => {
                            crate::choice::toggle(get_dlg_item(hwnd, id), hwnd, id as i32);
                            LRESULT(0)
                        }
                        ID_QUOTE | ID_SEPARATOR
                            if code
                                == windows::Win32::UI::WindowsAndMessaging::CBN_SELCHANGE
                                    as usize =>
                        {
                            change_option(id);
                            LRESULT(0)
                        }
                        ID_CLASSIC if code == BN_CLICKED as usize => {
                            crate::settings_ui::on_win11_classic_switch(hwnd);
                            refresh_options();
                            refresh_manager_status();
                            LRESULT(0)
                        }
                        ID_REPAIR if code == BN_CLICKED as usize => {
                            crate::ctxmenu::sync("repair");
                            if !crate::ctxmenu::registration_healthy() {
                                crate::warn_dialog("", &crate::t_pub("ctx_registry_failed"));
                            }
                            refresh_manager_status();
                            LRESULT(0)
                        }
                        ID_ADD if code == BN_CLICKED as usize => {
                            begin_new_rule();
                            LRESULT(0)
                        }
                        ID_ADD_SEP if code == BN_CLICKED as usize => {
                            begin_new_separator();
                            LRESULT(0)
                        }
                        ID_DELETE if code == BN_CLICKED as usize => {
                            delete_selected();
                            LRESULT(0)
                        }
                        ID_UP if code == BN_CLICKED as usize => {
                            move_selected(-1);
                            LRESULT(0)
                        }
                        ID_DOWN if code == BN_CLICKED as usize => {
                            move_selected(1);
                            LRESULT(0)
                        }
                        ID_SAVE if code == BN_CLICKED as usize => {
                            if apply_form() {
                                finish_edit(true);
                            }
                            LRESULT(0)
                        }
                        ID_DISCARD if code == BN_CLICKED as usize => {
                            finish_edit(false);
                            LRESULT(0)
                        }
                        ID_PREVIEW if code == BN_CLICKED as usize => {
                            let rule = draft_from_form();
                            let sample = if rule.scope == core_ctx::SCOPE_DIR
                                || rule.scope == core_ctx::SCOPE_BACKGROUND
                            {
                                r"C:\Example\Folder"
                            } else {
                                r"C:\Example\Sample.txt"
                            };
                            let mut paths = vec![sample.to_owned()];
                            if rule.selection_mode == core_ctx::SelectionMode::Together
                                && rule.scope != core_ctx::SCOPE_BACKGROUND
                            {
                                paths.push(
                                    if rule.scope == core_ctx::SCOPE_DIR {
                                        r"C:\Example\Second Folder"
                                    } else {
                                        r"C:\Example\Second File.txt"
                                    }
                                    .to_owned(),
                                );
                            }
                            let Some(args) = core_ctx::rule_selection_args(&rule, &paths) else {
                                set_valid(&crate::t_pub("ctx_issue_args"));
                                let field = get_dlg_item(hwnd, ID_ARGS_E);
                                let _ = SetFocus(Some(field));
                                crate::viewport::reveal_control(hwnd, field);
                                return LRESULT(0);
                            };
                            let directory = if !rule.working_dir.is_empty() {
                                rule.working_dir.clone()
                            } else if matches!(
                                rule.scope.as_str(),
                                core_ctx::SCOPE_DIR | core_ctx::SCOPE_BACKGROUND
                            ) {
                                sample.to_owned()
                            } else {
                                r"C:\Example".into()
                            };
                            let text = crate::t_args(
                                "ctx_preview_body",
                                &[&paths.join("\n"), &rule.command, &args, &directory],
                            );
                            let _ = MessageBoxW(
                                Some(hwnd),
                                PCWSTR(wide(&text).as_ptr()),
                                PCWSTR(wide(&crate::t_pub("ctx_preview")).as_ptr()),
                                windows::Win32::UI::WindowsAndMessaging::MB_OK
                                    | windows::Win32::UI::WindowsAndMessaging::MB_ICONINFORMATION,
                            );
                            LRESULT(0)
                        }
                        ID_WORK_B if code == BN_CLICKED as usize => {
                            browse_for_folder();
                            refresh_form_buttons();
                            LRESULT(0)
                        }
                        ID_ENABLED if code == BN_CLICKED as usize => {
                            let before = RULE_ENABLED.load(Ordering::SeqCst);
                            RULE_ENABLED.store(!before, Ordering::SeqCst);
                            let control = get_dlg_item(hwnd, ID_ENABLED);
                            crate::accessibility::check(control, !before);
                            crate::start_switch_animation(control, before, !before);
                            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(
                                Some(control),
                                None,
                                true,
                            );
                            refresh_form_buttons();
                            LRESULT(0)
                        }
                        ID_CONSOLE if code == BN_CLICKED as usize => {
                            let before = CONSOLE_HIDE.load(Ordering::SeqCst);
                            let next = !before;
                            CONSOLE_HIDE.store(next, Ordering::SeqCst);
                            let control = get_dlg_item(hwnd, ID_CONSOLE);
                            crate::accessibility::check(control, next);
                            // Same glide as the panel/settings switches.
                            crate::start_switch_animation(control, before, next);
                            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(
                                Some(control),
                                None,
                                true,
                            );
                            refresh_form_buttons();
                            LRESULT(0)
                        }
                        ID_PROG_B if code == BN_CLICKED as usize => {
                            browse_for_file(ID_PROG_E, "ctx_filter_programs");
                            LRESULT(0)
                        }
                        ID_ICON_B if code == BN_CLICKED as usize => {
                            browse_for_file(ID_ICON_E, "ctx_filter_icons");
                            LRESULT(0)
                        }
                        ID_LIST if code == LBN_DBLCLK as usize => {
                            edit_selected();
                            LRESULT(0)
                        }
                        ID_LIST if code == LBN_SELCHANGE as usize => {
                            selection_changed();
                            LRESULT(0)
                        }
                        _ if code == EN_CHANGE && FIELD_EDITS.contains(&id) => {
                            refresh_form_buttons();
                            DefWindowProcW(hwnd, msg, wparam, lparam)
                        }
                        ID_SCOPE_C | FIELD_SELECTION
                            if code
                                == windows::Win32::UI::WindowsAndMessaging::CBN_SELCHANGE
                                    as usize =>
                        {
                            if id == FIELD_SELECTION
                                && crate::choice::value(get_dlg_item(hwnd, FIELD_SELECTION)) == "2"
                                && matches!(
                                    text_of(hwnd, ID_ARGS_E).as_str(),
                                    "\"%1\"" | "\"%L\"" | "\"%V\""
                                )
                            {
                                set_text(ID_ARGS_E, "%*");
                            }
                            refresh_form_buttons();
                            LRESULT(0)
                        }
                        ID_SCOPE_C | FIELD_SELECTION if code == BN_CLICKED as usize => {
                            // The choice button opens its popup only when the
                            // owner toggles it (settings combo parity); the
                            // selection read-back doubles as the dirty check.
                            crate::choice::toggle(get_dlg_item(hwnd, id), hwnd, id as i32);
                            refresh_form_buttons();
                            DefWindowProcW(hwnd, msg, wparam, lparam)
                        }
                        _ if (code == EN_SETFOCUS || code == EN_KILLFOCUS)
                            && FIELD_EDITS.contains(&id) =>
                        {
                            // The well's focus ring is card pixels: repaint just
                            // that well, never the whole card face.
                            let edit = HWND(lparam.0 as *mut core::ffi::c_void);
                            if code == EN_KILLFOCUS {
                                crate::tooltips::dismiss_focus_tip(edit);
                            }
                            let _ = crate::nativeform::invalidate_field_well(
                                get_dlg_item(hwnd, ID_FORM),
                                edit,
                            );
                            DefWindowProcW(hwnd, msg, wparam, lparam)
                        }
                        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
                    }
                }
                WM_NCHITTEST => {
                    if crate::list_style::lane_hit(hwnd, lparam) {
                        return LRESULT(1);
                    }
                    crate::nativeform::blank_drag_hit(hwnd, msg, wparam, lparam)
                }
                windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONDOWN
                | windows::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE
                | windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP
                | windows::Win32::UI::WindowsAndMessaging::WM_CANCELMODE
                | windows::Win32::UI::WindowsAndMessaging::WM_CAPTURECHANGED
                | windows::Win32::UI::Controls::WM_MOUSELEAVE => {
                    if crate::list_style::lane_pointer(hwnd, msg, wparam, lparam) {
                        LRESULT(0)
                    } else {
                        DefWindowProcW(hwnd, msg, wparam, lparam)
                    }
                }
                windows::Win32::UI::WindowsAndMessaging::WM_SHOWWINDOW => {
                    if wparam.0 != 0 {
                        refresh_config_state();
                    }
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
                WM_CLOSE => {
                    hide();
                    LRESULT(0)
                }
                windows::Win32::UI::WindowsAndMessaging::WM_SIZE => {
                    // Any size pass (viewport fit, DPI rescale) may reorder
                    // siblings; re-lower the hosted cards before the next paint.
                    lower_cards(hwnd);
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
                WM_DRAWITEM => {
                    if let Some(item) = crate::nativeform::draw_item(lparam) {
                        draw_item(hwnd, &item);
                        LRESULT(1)
                    } else {
                        DefWindowProcW(hwnd, msg, wparam, lparam)
                    }
                }
                WM_ERASEBKGND => {
                    crate::popups::erase_theme_bg(hwnd, wparam);
                    LRESULT(1)
                }
                WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC
                    if FIELD_EDITS.contains(
                        &(windows::Win32::UI::WindowsAndMessaging::GetDlgCtrlID(HWND(
                            lparam.0 as *mut _,
                        )) as usize),
                    ) =>
                {
                    // Disabled edits send STATIC instead of EDIT. Both must
                    // use the same fill and erase brush as the card's well.
                    let p = theme::palette();
                    let control = HWND(lparam.0 as *mut _);
                    let disabled = !IsWindowEnabled(control).as_bool();
                    let (fill, color) = if disabled {
                        (p.disabled_surface, p.disabled_text)
                    } else {
                        (p.surface, p.text)
                    };
                    let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                    let _ = windows::Win32::Graphics::Gdi::SetTextColor(hdc, COLORREF(color));
                    let _ = windows::Win32::Graphics::Gdi::SetBkColor(hdc, COLORREF(fill));
                    let (light, dark) = theme::surface_brush_pairs();
                    let pair = if theme::is_dark() { dark } else { light };
                    LRESULT(if disabled { pair.1.0 } else { pair.0.0 } as isize)
                }
                WM_CTLCOLORSTATIC => {
                    let palette = theme::palette();
                    let control = HWND(lparam.0 as *mut core::ffi::c_void);
                    let id = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
                        control,
                        windows::Win32::UI::WindowsAndMessaging::GWL_ID,
                    ) as usize;
                    let on_surface = ON_CARD_STATICS.contains(&id);
                    let hdc =
                        windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut core::ffi::c_void);
                    let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                        hdc,
                        COLORREF(
                            if control == get_dlg_item(hwnd, ID_VALID)
                                || control == get_dlg_item(hwnd, ID_STATUS)
                                || control == get_dlg_item(hwnd, ID_EMPTY)
                                || control == get_dlg_item(hwnd, ID_EDITOR_GUIDE)
                            {
                                palette.muted
                            } else {
                                palette.text
                            },
                        ),
                    );
                    let _ = windows::Win32::Graphics::Gdi::SetBkColor(
                        hdc,
                        COLORREF(if on_surface {
                            palette.surface
                        } else {
                            theme::bg_color()
                        }),
                    );
                    if on_surface {
                        let (light, dark) = theme::surface_brush_pairs();
                        let pair = if theme::is_dark() { dark } else { light };
                        LRESULT(pair.0.0 as isize)
                    } else {
                        LRESULT(theme::bg_brush().0 as isize)
                    }
                }
                WM_CTLCOLORLISTBOX | WM_CTLCOLOREDIT => {
                    let palette = theme::palette();
                    let hdc =
                        windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut core::ffi::c_void);
                    let _ =
                        windows::Win32::Graphics::Gdi::SetTextColor(hdc, COLORREF(palette.text));
                    let _ =
                        windows::Win32::Graphics::Gdi::SetBkColor(hdc, COLORREF(palette.surface));
                    let (light, dark) = theme::surface_brush_pairs();
                    let pair = if theme::is_dark() { dark } else { light };
                    LRESULT(pair.0.0 as isize)
                }
                WM_DESTROY => {
                    MGR_HWND.store(0, Ordering::SeqCst);
                    LIST_HWND.store(0, Ordering::SeqCst);
                    crate::runtime::lock(&DISPLAYED).clear();
                    *crate::runtime::lock(&DRAFT) = None;
                    crate::release_panel_modal(hwnd);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        },
    )
}

unsafe fn draw_item(hwnd: HWND, item: &crate::nativeform::DrawItem) {
    unsafe {
        // Card repaints end in one SRCCOPY of the whole card rect: cut the
        // hosted children's rects out of the target DC first so the blit can
        // never paint over their pixels (settings/edit card parity).
        if [ID_FORM, ID_LAYOUT_CARD, ID_COPY_CARD].contains(&(item.control_id as usize)) {
            let mut card_rect = RECT::default();
            if GetWindowRect(item.control, &mut card_rect).is_ok() {
                let mut child = GetWindow(hwnd, GW_CHILD).unwrap_or_default();
                let mut guard = 0;
                while !child.is_invalid() && guard < 128 {
                    let mut rect = RECT::default();
                    if child != item.control
                        && IsWindowVisible(child).as_bool()
                        && GetWindowRect(child, &mut rect).is_ok()
                        && rect.left < card_rect.right
                        && rect.right > card_rect.left
                        && rect.top < card_rect.bottom
                        && rect.bottom > card_rect.top
                    {
                        let _ = windows::Win32::Graphics::Gdi::ExcludeClipRect(
                            item.dc,
                            rect.left - card_rect.left,
                            rect.top - card_rect.top,
                            rect.right - card_rect.left,
                            rect.bottom - card_rect.top,
                        );
                    }
                    child = GetWindow(child, GW_HWNDNEXT).unwrap_or_default();
                    guard += 1;
                }
            }
        }
        if crate::nativeform::draw_buffered(item.dc, &item.bounds, |dc, bounds| {
            draw_item_impl(item, dc, bounds);
        }) {
            return;
        }
        draw_item_impl(item, item.dc, &item.bounds);
    }
}

unsafe fn draw_item_impl(
    item: &crate::nativeform::DrawItem,
    dc: windows::Win32::Graphics::Gdi::HDC,
    bounds: &RECT,
) {
    let p = theme::palette();
    let scale = crate::scale_pub(96);
    let id = item.control_id as usize;
    unsafe {
        if [ID_SURF, ID_LAYOUT_CARD, ID_COPY_CARD].contains(&id) {
            crate::paint::draw_surface(
                dc,
                bounds,
                p.window_bg,
                p.surface,
                p.border,
                crate::paint::control_radius(),
            );
            if id == ID_SURF {
                crate::list_style::paint_lane(
                    dc,
                    item.control,
                    bounds.right - bounds.left,
                    bounds.bottom - bounds.top,
                );
            }
        } else if id == ID_TAB_RULES || id == ID_MORE {
            let mut state = crate::nativeform::control_state(item.control, item.state);
            state.active = (VIEW.load(Ordering::SeqCst) == VIEW_OPTIONS) == (id == ID_MORE);
            crate::paint::draw_chip(
                dc,
                bounds,
                body_font(),
                &text_of(mgr(), id),
                p,
                p.window_bg,
                state,
                crate::paint::control_radius(),
                false,
            );
        } else if id == ID_FORM {
            // Section card face + the settings field wells: every visible
            // EDIT hosted on the card gets its ring painted as card pixels.
            crate::paint::draw_surface(
                dc,
                bounds,
                p.window_bg,
                p.surface,
                p.border,
                crate::paint::control_radius(),
            );
            let mut card_rect = RECT::default();
            if GetWindowRect(item.control, &mut card_rect).is_err() {
                return;
            }
            let mut child = GetWindow(mgr(), GW_CHILD).unwrap_or_default();
            let mut guard = 0;
            while !child.is_invalid() && guard < 128 {
                let mut edit_rect = RECT::default();
                if IsWindowVisible(child).as_bool()
                    && GetWindowRect(child, &mut edit_rect).is_ok()
                    && edit_rect.left >= card_rect.left
                    && edit_rect.right <= card_rect.right
                    && edit_rect.top >= card_rect.top
                    && edit_rect.bottom <= card_rect.bottom
                {
                    let mut class = [0u16; 8];
                    let len = GetClassNameW(child, &mut class);
                    if String::from_utf16_lossy(&class[..len.max(0) as usize])
                        .eq_ignore_ascii_case("EDIT")
                    {
                        let local = RECT {
                            left: 0,
                            top: 0,
                            right: bounds.right - bounds.left,
                            bottom: bounds.bottom - bounds.top,
                        };
                        let control = RECT {
                            left: edit_rect.left - card_rect.left,
                            top: edit_rect.top - card_rect.top,
                            right: edit_rect.right - card_rect.left,
                            bottom: edit_rect.bottom - card_rect.top,
                        };
                        let well = crate::nativeform::field_well_rect(&local, &control);
                        let hole = control;
                        if well.right > well.left && well.bottom > well.top {
                            let state = crate::paint::ControlState {
                                focused: GetFocus() == child,
                                disabled: !IsWindowEnabled(child).as_bool(),
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
                                Some(&hole),
                            );
                        }
                    }
                }
                child = GetWindow(child, GW_HWNDNEXT).unwrap_or_default();
                guard += 1;
            }
        } else if id == ID_LIST {
            // Owner-draw rule rows; family selection grammar from the
            // automation manager (accent bar + heavier ink when selected).
            let selected = item.state & crate::nativeform::ODS_SELECTED != 0;
            let focused = item.state & crate::nativeform::ODS_FOCUS != 0;
            let len = SendMessageW(
                item.control,
                windows::Win32::UI::WindowsAndMessaging::LB_GETTEXTLEN,
                Some(WPARAM(item.item_id as usize)),
                None,
            )
            .0
            .max(0) as usize;
            let mut text = String::new();
            if len > 0 {
                let mut buffer = vec![0u16; len + 1];
                let copied = SendMessageW(
                    item.control,
                    windows::Win32::UI::WindowsAndMessaging::LB_GETTEXT,
                    Some(WPARAM(item.item_id as usize)),
                    Some(LPARAM(buffer.as_mut_ptr() as isize)),
                )
                .0
                .max(0) as usize;
                text = String::from_utf16_lossy(&buffer[..copied.min(len)]);
            }
            crate::paint::fill_rect(dc, bounds, p.surface);
            if selected {
                let bar_w = crate::scale_pub(3);
                let bar_h = crate::scale_pub(20);
                let marker = RECT {
                    left: bounds.left,
                    top: bounds.top + (bounds.bottom - bounds.top - bar_h) / 2,
                    right: bounds.left + bar_w,
                    bottom: bounds.top + (bounds.bottom - bounds.top + bar_h) / 2,
                };
                match crate::paint::fill_rounded_rect(dc, &marker, bar_w.max(1), p.accent, p.accent)
                {
                    crate::paint::DrawResult::Completed | crate::paint::DrawResult::MayBeDirty => {}
                    crate::paint::DrawResult::NotStarted => {
                        crate::paint::fill_rect(dc, &marker, p.accent);
                    }
                }
            }
            let mut row = *bounds;
            row.left += crate::scale_pub(12);
            row.right -= crate::scale_pub(8);
            if let Some((title, summary)) = text.split_once('\n') {
                let middle = (row.top + row.bottom) / 2;
                let title_box = RECT {
                    top: row.top + s(3),
                    bottom: middle,
                    ..row
                };
                let summary_box = RECT {
                    top: middle,
                    bottom: row.bottom - s(3),
                    ..row
                };
                crate::paint::draw_label(dc, &title_box, body_font(), title, p.text, true, 0, 0);
                crate::paint::draw_label(
                    dc,
                    &summary_box,
                    label_font(),
                    summary,
                    p.text2,
                    true,
                    0,
                    0,
                );
            } else {
                crate::paint::draw_label(dc, &row, body_font(), &text, p.text2, true, 0, 0);
            }
            if focused {
                crate::paint::draw_focus_frame(dc, bounds, p.focus);
            }
        } else if crate::choice::is_choice(item.control) {
            let state = crate::nativeform::control_state(item.control, item.state);
            crate::choice::draw_button(item.control, dc, bounds, state, p.surface);
        } else if matches!(id, ID_PREVIEW | ID_ADVANCED | ID_REPAIR | ID_BACK) {
            let label = text_of(mgr(), id);
            let background = if id == ID_ADVANCED {
                p.surface
            } else {
                p.window_bg
            };
            crate::paint::draw_text_link(
                dc,
                bounds,
                body_font(),
                &label,
                p,
                background,
                crate::nativeform::control_state(item.control, item.state),
                scale,
            );
        } else if matches!(
            id,
            ID_CONSOLE | ID_ENABLED | ID_MASTER | ID_GROUPED | ID_COPY | ID_CLASSIC
        ) {
            // Panel switch grammar on the card face.
            let label = text_of(mgr(), id);
            let mut state = crate::nativeform::control_state(item.control, item.state);
            state.active = if id == ID_CONSOLE {
                CONSOLE_HIDE.load(Ordering::SeqCst)
            } else if id == ID_MASTER {
                crate::cfg_map(|c| c.ctx_menu_enabled)
            } else if id == ID_GROUPED {
                crate::cfg_map(|c| c.ctx_menu_grouped)
            } else if id == ID_COPY {
                crate::cfg_map(|c| c.ctx_copy_enabled)
            } else if id == ID_CLASSIC {
                matches!(
                    crate::ctxmenu::classic_state(),
                    crate::ctxmenu::ClassicState::ByUs
                        | crate::ctxmenu::ClassicState::ByOther
                        | crate::ctxmenu::ClassicState::Legacy
                )
            } else {
                RULE_ENABLED.load(Ordering::SeqCst)
            };
            let progress = crate::switch_animation_progress(item.control);
            let draw_switch = if matches!(id, ID_MASTER | ID_ENABLED) {
                crate::paint::draw_compact_switch_row
            } else {
                crate::paint::draw_switch_row
            };
            draw_switch(
                dc,
                bounds,
                body_font(),
                &label,
                p,
                if matches!(id, ID_MASTER | ID_ENABLED) {
                    p.window_bg
                } else {
                    p.surface
                },
                state,
                scale,
                progress,
            );
        } else {
            let label = text_of(mgr(), id);
            // Buttons riding a card face erase with the card color.
            let on_card = id == ID_PROG_B || id == ID_ICON_B || id == ID_WORK_B;
            let background = if on_card { p.surface } else { p.window_bg };
            // Save carries the accent fill of a default action (settings and
            // editor footer parity).
            let mut state = crate::nativeform::control_state(item.control, item.state);
            if id == ID_SAVE || id == ID_ADD {
                state.active = true;
            }
            if id == ID_DELETE {
                crate::paint::draw_button_danger(
                    dc,
                    bounds,
                    body_font(),
                    &label,
                    p,
                    background,
                    state,
                    crate::paint::control_radius(),
                );
            } else {
                crate::paint::draw_button(
                    dc,
                    bounds,
                    body_font(),
                    &label,
                    p,
                    background,
                    state,
                    crate::paint::control_radius(),
                );
            }
        }
    }
}

// ---- Data flow -------------------------------------------------------------

fn row_text(rule: &core_ctx::CtxRule) -> String {
    if rule.separator {
        let label = crate::t_pub("ctx_separator_row");
        return if rule.enabled {
            label
        } else {
            format!("{} ({})", label, crate::t_pub("ctx_disabled_short"))
        };
    }
    let (_, issues) = core_ctx::prepare_rules(vec![rule.clone()]);
    let matched = if !issues.is_empty() {
        crate::t_pub("ctx_invalid_short")
    } else if rule.patterns.is_empty() {
        crate::t_pub("ctx_match_all_short")
    } else {
        rule.patterns.join(";")
    };
    let program = std::path::Path::new(&rule.command)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| rule.command.clone());
    format!(
        "{}\n{}  ·  {}  ·  {}",
        if rule.enabled {
            rule.title.clone()
        } else {
            format!("{} ({})", rule.title, crate::t_pub("ctx_disabled_short"))
        },
        crate::t_pub(match rule.scope.as_str() {
            core_ctx::SCOPE_DIR => "ctx_scope_dir",
            core_ctx::SCOPE_FILE_DIR => "ctx_scope_file_dir",
            core_ctx::SCOPE_BACKGROUND => "ctx_scope_background",
            core_ctx::SCOPE_ALL => "ctx_scope_all",
            _ => "ctx_scope_file",
        }),
        matched,
        program
    )
}

fn refresh_list() {
    *crate::runtime::lock(&DISPLAYED) = crate::runtime::lock(&crate::ctxexec::CTX_RULES).clone();
    render_list();
}

fn render_list() {
    unsafe {
        let rules = crate::runtime::lock(&DISPLAYED).clone();
        LIST_ROW_COUNT.store(rules.len(), Ordering::SeqCst);
        let list = list_hwnd();
        if list.is_invalid() {
            return;
        }
        let _ = SendMessageW(
            list,
            windows::Win32::UI::WindowsAndMessaging::WM_SETREDRAW,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
        let _ = SendMessageW(
            list,
            windows::Win32::UI::WindowsAndMessaging::LB_RESETCONTENT,
            None,
            None,
        );
        for rule in &rules {
            let text = row_text(rule);
            let _ = SendMessageW(
                list,
                windows::Win32::UI::WindowsAndMessaging::LB_ADDSTRING,
                None,
                Some(LPARAM(wide(&text).as_ptr() as isize)),
            );
        }
        let _ = SendMessageW(
            list,
            windows::Win32::UI::WindowsAndMessaging::WM_SETREDRAW,
            Some(WPARAM(1)),
            Some(LPARAM(0)),
        );
        let empty = rules.is_empty();
        let _ = ShowWindow(
            get_dlg_item(mgr(), ID_EMPTY),
            if empty && VIEW.load(Ordering::SeqCst) == VIEW_LIST {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(list), None, true);
        refresh_action_states();
    }
}

/// Row buttons follow the selection: delete/reorder need a selected rule.
fn refresh_action_states() {
    let selected = selected_index();
    let selected_rule =
        selected.and_then(|index| crate::runtime::lock(&DISPLAYED).get(index).cloned());
    set_text(
        ID_TOGGLE,
        &crate::t_pub(if selected_rule.as_ref().is_some_and(|rule| rule.enabled) {
            "ctx_disable_rule"
        } else {
            "ctx_enable_rule"
        }),
    );
    let (count, can_append_separator) = {
        let rules = crate::runtime::lock(&DISPLAYED);
        (rules.len(), can_append_separator(&rules))
    };
    for (id, enabled) in [
        (ID_DELETE, selected.is_some()),
        (
            ID_EDIT,
            selected_rule.as_ref().is_some_and(|rule| !rule.separator),
        ),
        (ID_TOGGLE, selected.is_some()),
        (ID_UP, selected.is_some_and(|index| index > 0)),
        (ID_DOWN, selected.is_some_and(|index| index + 1 < count)),
        (ID_ADD, count < core_ctx::MAX_CTX_RULES),
        (ID_ADD_SEP, can_append_separator),
    ] {
        unsafe {
            let _ = EnableWindow(get_dlg_item(mgr(), id), enabled);
        }
    }
}

fn can_append_separator(rules: &[core_ctx::CtxRule]) -> bool {
    rules.len() < core_ctx::MAX_CTX_RULES && rules.last().is_some_and(|rule| !rule.separator)
}

fn selected_index() -> Option<usize> {
    unsafe {
        let sel = SendMessageW(
            list_hwnd(),
            windows::Win32::UI::WindowsAndMessaging::LB_GETCURSEL,
            None,
            None,
        )
        .0;
        if sel < 0 || sel as usize >= crate::runtime::lock(&DISPLAYED).len() {
            None
        } else {
            Some(sel as usize)
        }
    }
}

fn select_index(index: i32) {
    unsafe {
        let _ = SendMessageW(
            list_hwnd(),
            windows::Win32::UI::WindowsAndMessaging::LB_SETCURSEL,
            Some(WPARAM(index as usize)),
            None,
        );
    }
    refresh_action_states();
}

/// Guidance when no draft is open; blank while editing.
fn set_valid(text: &str) {
    set_text(ID_VALID, text);
    let control = get_dlg_item(mgr(), ID_VALID);
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, true);
    }
}

/// Localized one-liner for a prepare_rules issue (core messages stay English
/// for the log).
fn issue_text(issue: &core_ctx::CtxIssue) -> String {
    match issue.code {
        core_ctx::IssueCode::TitleRequired => crate::t_pub("ctx_issue_title"),
        core_ctx::IssueCode::PatternInvalid => crate::t_pub("ctx_issue_pattern"),
        core_ctx::IssueCode::ArgumentsInvalid => crate::t_pub("ctx_issue_args"),
        core_ctx::IssueCode::CommandRequired => crate::t_pub("ctx_issue_program"),
        core_ctx::IssueCode::TooMany => {
            crate::t_args("ctx_issue_limit", &[&core_ctx::MAX_CTX_RULES.to_string()])
        }
        core_ctx::IssueCode::Parse => crate::t_args("ctx_parse_failed", &[&issue.message]),
    }
}

/// Reads the form into the working draft.
fn draft_from_form() -> core_ctx::CtxRule {
    let window = mgr();
    let mut draft = crate::runtime::lock(&DRAFT).clone().unwrap_or_default();
    draft.enabled = RULE_ENABLED.load(Ordering::SeqCst);
    draft.working_dir = text_of(window, FIELD_WORK).trim().to_string();
    draft.title = text_of(window, ID_TITLE_E).trim().to_string();
    draft.command = text_of(window, ID_PROG_E).trim().to_string();
    draft.args = text_of(window, ID_ARGS_E).trim().to_string();
    draft.icon = text_of(window, ID_ICON_E).trim().to_string();
    if draft.separator {
        // Separator drafts carry no fields; keep whatever id they had.
        return draft;
    }
    draft.patterns = core_ctx::patterns_from_text(&text_of(window, ID_MATCH_E));
    let scope = crate::choice::value(get_dlg_item(window, ID_SCOPE_C));
    if core_ctx::valid_scope(&scope) {
        draft.scope = scope;
    }
    if let Ok(index) = crate::choice::value(get_dlg_item(window, FIELD_SELECTION)).parse::<usize>()
        && let Some(mode) = SELECTION_MODES.get(index)
    {
        draft.selection_mode = *mode;
    }
    draft.hidden_console = CONSOLE_HIDE.load(Ordering::SeqCst);
    draft
}

fn load_form() {
    let window = mgr();
    let draft = crate::runtime::lock(&DRAFT).clone();
    let editable = draft.as_ref().is_some_and(|rule| !rule.separator);
    for id in FIELD_EDITS.into_iter().chain([
        ID_SCOPE_C,
        FIELD_SELECTION,
        ID_PROG_B,
        ID_ICON_B,
        ID_WORK_B,
        ID_CONSOLE,
        ID_PREVIEW,
    ]) {
        unsafe {
            let _ = EnableWindow(get_dlg_item(window, id), editable);
        }
        if FIELD_EDITS.contains(&id) {
            // EnableWindow repaints the EDIT, but its rounded border belongs
            // to a different HWND. Repaint that well on state changes too.
            crate::nativeform::invalidate_field_well(
                get_dlg_item(window, ID_FORM),
                get_dlg_item(window, id),
            );
        }
    }
    unsafe {
        let _ = EnableWindow(get_dlg_item(window, ID_ENABLED), draft.is_some());
    }
    let Some(rule) = draft else {
        set_text(ID_TITLE_E, "");
        set_text(ID_PROG_E, "");
        set_text(ID_ARGS_E, "");
        set_text(ID_ICON_E, "");
        set_text(ID_MATCH_E, "");
        set_text(FIELD_WORK, "");
        crate::choice::select_index(
            get_dlg_item(window, ID_SCOPE_C),
            scope_row_index(core_ctx::SCOPE_FILE),
        );
        set_console(false);
        set_valid(&crate::t_pub("ctx_form_pick_hint"));
        refresh_form_buttons();
        return;
    };
    RULE_ENABLED.store(rule.enabled, Ordering::SeqCst);
    crate::accessibility::check(get_dlg_item(window, ID_ENABLED), rule.enabled);
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(
            Some(get_dlg_item(window, ID_ENABLED)),
            None,
            true,
        );
    }
    set_text(FIELD_WORK, &rule.working_dir);
    crate::choice::select_index(
        get_dlg_item(window, FIELD_SELECTION),
        selection_index(rule.selection_mode),
    );
    set_text(ID_TITLE_E, &rule.title);
    set_text(ID_PROG_E, &rule.command);
    set_text(ID_ARGS_E, &rule.args);
    set_text(ID_ICON_E, &rule.icon);
    set_text(ID_MATCH_E, &rule.patterns.join(";"));
    crate::choice::select_index(
        get_dlg_item(window, ID_SCOPE_C),
        scope_row_index(&rule.scope),
    );
    set_console(rule.hidden_console);
    let separator = rule.separator;
    let (_, issues) = core_ctx::prepare_rules(vec![rule]);
    set_valid(&issues.first().map(issue_text).unwrap_or_else(|| {
        if separator {
            crate::t_pub("ctx_form_sep_hint")
        } else {
            String::new()
        }
    }));
    refresh_form_buttons();
}

/// Keep the editor's visible state and actions in sync with its saved baseline.
fn refresh_form_buttons() {
    let draft = crate::runtime::lock(&DRAFT).clone();
    let is_new = draft.as_ref().is_some_and(|rule| rule.id.is_empty());
    let dirty = match draft.as_ref() {
        None => false,
        Some(rule) => is_new || draft_from_form() != *rule,
    };
    let stale =
        *crate::runtime::lock(&DISPLAYED) != *crate::runtime::lock(&crate::ctxexec::CTX_RULES);
    let title_key = if stale {
        "ctx_form_stale_title"
    } else if is_new {
        "ctx_form_new_title"
    } else if dirty {
        "ctx_form_dirty_title"
    } else if draft.is_some() {
        "ctx_form_saved_title"
    } else {
        "ctx_form_title"
    };
    set_text(ID_FORM_TITLE, &crate::t_pub(title_key));
    set_text(
        ID_TAB_RULES,
        &crate::t_pub(if dirty {
            "ctx_tab_rules_dirty"
        } else {
            "ctx_tab_rules"
        }),
    );
    set_text(ID_DISCARD, &crate::t_pub("common_cancel"));
    refresh_editor_visibility();
    unsafe {
        let _ = EnableWindow(get_dlg_item(mgr(), ID_SAVE), dirty);
        let _ = EnableWindow(get_dlg_item(mgr(), ID_DISCARD), true);
    }
}

fn set_console(value: bool) {
    CONSOLE_HIDE.store(value, Ordering::SeqCst);
    let control = get_dlg_item(mgr(), ID_CONSOLE);
    if !control.is_invalid() {
        crate::accessibility::check(control, value);
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, true);
        }
    }
}

/// Saves the form. New drafts get a generated id (prepare_rules assigns one).
fn apply_form() -> bool {
    if crate::runtime::lock(&DRAFT).is_none() {
        return true;
    }
    if VIEW.load(Ordering::SeqCst) != VIEW_EDIT {
        select_view(VIEW_EDIT);
    }
    let draft = draft_from_form();
    let base = crate::runtime::lock(&DISPLAYED).clone();
    let mut rules = base.clone();
    let position = rules
        .iter()
        .position(|item| !draft.id.is_empty() && item.id == draft.id)
        .unwrap_or(rules.len());
    if position == rules.len() {
        rules.push(draft);
    } else {
        rules[position] = draft;
    }
    let (prepared, issues) = core_ctx::prepare_rules(rules);
    if let Some(issue) = issues
        .iter()
        .find(|issue| issue.index == position && issue.code != core_ctx::IssueCode::TooMany)
    {
        let message = issue_text(issue);
        set_valid(&message);
        crate::warn_dialog("", &message);
        let field = match issue.code {
            core_ctx::IssueCode::TitleRequired => ID_TITLE_E,
            core_ctx::IssueCode::CommandRequired => ID_PROG_E,
            core_ctx::IssueCode::ArgumentsInvalid => ID_ARGS_E,
            _ => ID_MATCH_E,
        };
        unsafe {
            let control = get_dlg_item(mgr(), field);
            let _ = SetFocus(Some(control));
            crate::viewport::reveal_control(mgr(), control);
        }
        return false;
    }
    if let Err(err) = crate::save_ctx_rules(&base, &prepared) {
        set_valid(&err);
        crate::warn_dialog("", &err);
        return false;
    }
    *crate::runtime::lock(&DRAFT) = Some(prepared[position].clone());
    refresh_list();
    select_index(position as i32);
    load_form();
    refresh_manager_status();
    true
}

/// Explicit save/cancel semantics at every navigation boundary.
fn leave_draft() -> bool {
    let original = crate::runtime::lock(&DRAFT).clone();
    let Some(original) = original else {
        return true;
    };
    if draft_from_form() == original {
        return true;
    }
    let answer = unsafe {
        MessageBoxW(
            Some(mgr()),
            PCWSTR(wide(&crate::t_pub("ctx_unsaved_prompt")).as_ptr()),
            PCWSTR(wide(&crate::t_pub("ctx_manager_title")).as_ptr()),
            windows::Win32::UI::WindowsAndMessaging::MB_YESNOCANCEL | MB_ICONQUESTION,
        )
    };
    match answer {
        IDYES => apply_form(),
        windows::Win32::UI::WindowsAndMessaging::IDNO => {
            load_form();
            true
        }
        _ => false,
    }
}

fn begin_new_rule() {
    if !leave_draft() || crate::runtime::lock(&DISPLAYED).len() >= core_ctx::MAX_CTX_RULES {
        return;
    }
    select_index(-1);
    *crate::runtime::lock(&DRAFT) = Some(core_ctx::CtxRule {
        args: "\"%1\"".into(),
        ..Default::default()
    });
    ADVANCED.store(false, Ordering::SeqCst);
    load_form();
    select_view(VIEW_EDIT);
    set_valid("");
    let _ = unsafe { SetFocus(Some(get_dlg_item(mgr(), ID_PROG_E))) };
}

fn begin_new_separator() {
    if !can_append_separator(&crate::runtime::lock(&DISPLAYED)) || !leave_draft() {
        return;
    }
    // Saving a pending draft can change the list; recheck the append boundary.
    let base = crate::runtime::lock(&DISPLAYED).clone();
    if !can_append_separator(&base) {
        return;
    }
    let mut rules = base.clone();
    rules.push(core_ctx::CtxRule {
        separator: true,
        ..Default::default()
    });
    let (prepared, _) = core_ctx::prepare_rules(rules);
    if let Err(err) = crate::save_ctx_rules(&base, &prepared) {
        set_valid(&err);
        crate::warn_dialog("", &err);
        return;
    }
    let position = prepared.len() - 1;
    *crate::runtime::lock(&DRAFT) = None;
    refresh_list();
    select_index(position as i32);
    set_text(ID_LIST_HINT, &crate::t_pub("ctx_separator_saved"));
    refresh_manager_status();
    let _ = unsafe { SetFocus(Some(list_hwnd())) };
}

fn selection_changed() {
    refresh_action_states();
    set_text(ID_LIST_HINT, &crate::t_pub("ctx_list_hint"));
}

fn toggle_selected() {
    let Some(index) = selected_index() else {
        return;
    };
    let base = crate::runtime::lock(&DISPLAYED).clone();
    let mut next = base.clone();
    let Some(rule) = next.get_mut(index) else {
        return;
    };
    rule.enabled = !rule.enabled;
    let enabled = rule.enabled;
    let (_, issues) = core_ctx::prepare_rules(next.clone());
    if enabled && let Some(issue) = issues.iter().find(|issue| issue.index == index) {
        crate::warn_dialog("", &issue_text(issue));
        return;
    }
    if let Err(error) = crate::save_ctx_rules(&base, &next) {
        crate::warn_dialog("", &error);
        return;
    }
    refresh_list();
    select_index(index as i32);
    refresh_manager_status();
}

fn delete_selected() {
    let Some(index) = selected_index() else {
        return;
    };
    let base = crate::runtime::lock(&DISPLAYED).clone();
    let mut rules = base.clone();
    if index >= rules.len() {
        return;
    }
    let message = if rules[index].separator {
        crate::t_pub("ctx_delete_separator_confirm")
    } else {
        crate::t_args("ctx_delete_confirm", &[&rules[index].title])
    };
    let result = unsafe {
        MessageBoxW(
            Some(mgr()),
            PCWSTR(wide(&message).as_ptr()),
            PCWSTR(wide(&crate::t_pub("ctx_manager_title")).as_ptr()),
            MB_YESNO | MB_ICONQUESTION | windows::Win32::UI::WindowsAndMessaging::MB_DEFBUTTON2,
        )
    };
    if result != IDYES {
        return;
    }
    rules.remove(index);
    if let Err(err) = crate::save_ctx_rules(&base, &rules) {
        set_valid(&err);
        crate::warn_dialog("", &err);
        return;
    }
    *crate::runtime::lock(&DRAFT) = None;
    refresh_list();
    let count = crate::runtime::lock(&DISPLAYED).len();
    select_index(index.min(count.saturating_sub(1)) as i32);
    selection_changed();
}

fn move_selected(delta: i32) {
    if !leave_draft() {
        return;
    }
    let Some(index) = selected_index() else {
        return;
    };
    let target = index as i32 + delta;
    let len = crate::runtime::lock(&DISPLAYED).len() as i32;
    if target < 0 || target >= len {
        return;
    }
    let base = crate::runtime::lock(&DISPLAYED).clone();
    let mut rules = base.clone();
    if index >= rules.len() {
        return;
    }
    rules.swap(index, target as usize);
    if let Err(err) = crate::save_ctx_rules(&base, &rules) {
        set_valid(&err);
        crate::warn_dialog("", &err);
        return;
    }
    *crate::runtime::lock(&DRAFT) = None;
    refresh_list();
    select_index(target);
    selection_changed();
}

fn browse_for_folder() {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog,
        IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    let picked = (|| -> windows::core::Result<Option<String>> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            let _apartment = Apartment;
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetOptions(
                dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST,
            )?;
            dialog.SetTitle(PCWSTR(
                wide(&crate::t_pub("ctx_browse_work_title")).as_ptr(),
            ))?;
            let initial = text_of(mgr(), FIELD_WORK);
            if !initial.is_empty() {
                let folder: windows::core::Result<IShellItem> =
                    SHCreateItemFromParsingName(PCWSTR(wide(&initial).as_ptr()), None);
                if let Ok(folder) = folder {
                    let _ = dialog.SetFolder(&folder);
                }
            }
            if let Err(err) = dialog.Show(Some(mgr())) {
                if err.code()
                    == windows::core::HRESULT::from_win32(
                        windows::Win32::Foundation::ERROR_CANCELLED.0,
                    )
                {
                    return Ok(None);
                }
                return Err(err);
            }
            let path = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let text = path.to_string();
            CoTaskMemFree(Some(path.0.cast()));
            Ok(Some(text?))
        }
    })();
    match picked {
        Ok(Some(path)) => set_text(FIELD_WORK, &path),
        Ok(None) => {}
        Err(err) => set_valid(&crate::t_args("ctx_folder_failed", &[&err.to_string()])),
    }
}

fn browse_for_file(edit_id: usize, filter_key: &str) {
    let filters = crate::t_pub(filter_key);
    let mut filter_wide: Vec<u16> = filters
        .replace('|', "\0")
        .encode_utf16()
        .chain([0, 0])
        .collect();
    let initial = text_of(mgr(), edit_id);
    let mut file = [0u16; 32768];
    let initial: Vec<u16> = initial.encode_utf16().take(file.len() - 1).collect();
    file[..initial.len()].copy_from_slice(&initial);
    let title = wide(&crate::t_pub(if edit_id == ID_ICON_E {
        "ctx_browse_icon_title"
    } else {
        "ctx_browse_title"
    }));
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: mgr(),
        lpstrFilter: PCWSTR(filter_wide.as_mut_ptr()),
        lpstrFile: windows::core::PWSTR(file.as_mut_ptr()),
        nMaxFile: file.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        Flags: OFN_FILEMUSTEXIST | OFN_HIDEREADONLY,
        ..Default::default()
    };
    let picked = unsafe { GetOpenFileNameW(&mut ofn) };
    if picked.as_bool() {
        let len = file.iter().position(|ch| *ch == 0).unwrap_or(file.len());
        let path = String::from_utf16_lossy(&file[..len]);
        set_text(edit_id, &path);
        if edit_id == ID_PROG_E
            && text_of(mgr(), ID_TITLE_E).trim().is_empty()
            && let Some(name) = std::path::Path::new(&path).file_stem()
        {
            set_text(
                ID_TITLE_E,
                &crate::t_args("ctx_default_rule_title", &[&name.to_string_lossy()]),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_dirty_state_navigation_and_text_are_lossless() {
        const CHILD: &str = "IDLETRIGGER_CTX_FORM_TEST";
        if std::env::var_os(CHILD).is_none() {
            let _ui_test = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "ctx_ui::tests::manager_dirty_state_navigation_and_text_are_lossless",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success());
                    return;
                }
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("context manager test timed out");
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        *crate::runtime::lock(&crate::CONFIG) = Some(Default::default());
        *crate::I18N.write().unwrap() = Some(idletrigger_core::i18n::I18n::load("en"));
        crate::create_windows();
        unsafe {
            ensure_created();
        }
        let is_shown = |id| unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetWindowLongW(
                get_dlg_item(mgr(), id),
                windows::Win32::UI::WindowsAndMessaging::GWL_STYLE,
            ) as u32
                & WS_VISIBLE.0
                != 0
        };
        let rule = core_ctx::CtxRule {
            id: "a".into(),
            title: "A".into(),
            command: "tool.exe".into(),
            args: "\"%1\"".into(),
            ..Default::default()
        };
        *crate::runtime::lock(&crate::ctxexec::CTX_RULES) = vec![rule.clone()];
        refresh_list();
        select_index(0);
        select_view(VIEW_LIST);
        assert!(is_shown(ID_LIST));
        assert!(!is_shown(ID_FORM));
        assert_eq!(default_button(mgr()), Some(get_dlg_item(mgr(), ID_EDIT)));
        edit_selected();
        assert_eq!(VIEW.load(Ordering::SeqCst), VIEW_EDIT);
        assert!(!is_shown(ID_LIST));
        assert!(is_shown(ID_TITLE_E));
        assert!(!is_shown(FIELD_WORK));
        assert!(!unsafe { IsWindowEnabled(get_dlg_item(mgr(), ID_SAVE)).as_bool() });
        set_text(ID_ARGS_E, "unsaved argument");
        assert!(unsafe { IsWindowEnabled(get_dlg_item(mgr(), ID_SAVE)).as_bool() });
        select_view(VIEW_OPTIONS);
        assert!(is_shown(ID_GROUPED));
        assert!(!is_shown(ID_SAVE));
        assert_eq!(is_shown(ID_CLASSIC), crate::system::is_windows_11());
        refresh_language();
        assert_eq!(text_of(mgr(), ID_ARGS_E), "unsaved argument");
        assert_eq!(*crate::runtime::lock(&DRAFT), Some(rule.clone()));
        select_view(VIEW_EDIT);
        ADVANCED.store(true, Ordering::SeqCst);
        refresh_editor_visibility();
        assert!(is_shown(FIELD_WORK));
        set_text(FIELD_WORK, r"C:\Example");
        ADVANCED.store(false, Ordering::SeqCst);
        refresh_editor_visibility();
        assert_eq!(draft_from_form().working_dir, r"C:\Example");
        let long = "x".repeat(4096);
        set_text(ID_ARGS_E, &long);
        refresh_language();
        assert_eq!(text_of(mgr(), ID_ARGS_E), long);
        set_text(ID_ARGS_E, "\"%1\"");
        crate::choice::select_index(get_dlg_item(mgr(), FIELD_SELECTION), 2);
        unsafe {
            SendMessageW(
                mgr(),
                WM_COMMAND,
                Some(WPARAM(
                    FIELD_SELECTION
                        | ((windows::Win32::UI::WindowsAndMessaging::CBN_SELCHANGE as usize) << 16),
                )),
                None,
            );
        }
        assert_eq!(text_of(mgr(), ID_ARGS_E), "%*");
        assert_eq!(
            draft_from_form().selection_mode,
            core_ctx::SelectionMode::Together
        );
        crate::choice::select_index(
            get_dlg_item(mgr(), ID_SCOPE_C),
            scope_row_index(core_ctx::SCOPE_BACKGROUND),
        );
        refresh_form_buttons();
        assert!(!is_shown(FIELD_SELECTION));
        crate::runtime::lock(&crate::ctxexec::CTX_RULES)[0].title = "External edit".into();
        refresh_config_state();
        assert_eq!(text_of(mgr(), ID_TITLE_E), "A");
        assert_eq!(*crate::runtime::lock(&DRAFT), Some(rule));
        assert_eq!(
            text_of(mgr(), ID_FORM_TITLE),
            crate::t_pub("ctx_form_stale_title")
        );
        finish_edit(false);
        assert!(crate::runtime::lock(&DRAFT).is_none());
        assert_eq!(VIEW.load(Ordering::SeqCst), VIEW_LIST);
        assert!(!is_shown(ID_TITLE_E));
        edit_selected();
        assert_eq!(text_of(mgr(), ID_TITLE_E), "External edit");
        assert_eq!(default_button(mgr()), Some(get_dlg_item(mgr(), ID_SAVE)));
        set_valid("A long validation message");
        assert!(is_shown(ID_VALID));
        select_view(VIEW_OPTIONS);
        assert!(!is_shown(ID_VALID));
        select_view(VIEW_EDIT);
        assert!(is_shown(ID_VALID));
        finish_edit(false);
        let separator = core_ctx::CtxRule {
            id: "sep".into(),
            separator: true,
            ..Default::default()
        };
        crate::runtime::lock(&crate::ctxexec::CTX_RULES).push(separator);
        refresh_list();
        select_index(1);
        assert_eq!(default_button(mgr()), Some(get_dlg_item(mgr(), ID_ADD)));
        begin_new_rule();
        assert!(crate::runtime::lock(&DRAFT).as_ref().unwrap().id.is_empty());
        finish_edit(false);
        assert_eq!(VIEW.load(Ordering::SeqCst), VIEW_LIST);
    }
}
