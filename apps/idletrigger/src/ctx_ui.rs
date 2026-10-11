//! Context-menu rule manager: the automation-manager family grammar —
//! list card with a horizontal action row, then a persistent edit form
//! (Open++ flow) hosted on its own section card with card-painted field
//! wells (settings field grammar).

use idletrigger_core::ctx_menu as core_ctx;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
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
const ID_MORE: usize = 701; // "more settings" link
const ID_SURF: usize = 702; // list card surface
const ID_LIST: usize = 703;
const ID_ADD: usize = 704;
const ID_DELETE: usize = 705;
const ID_UP: usize = 706;
const ID_DOWN: usize = 707;
const ID_ADD_SEP: usize = 709;
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

/// Tooltip anchors for the manager's form fields.
pub const FIELD_ID_TITLE: usize = ID_TITLE_E;
pub const FIELD_ID_PROGRAM: usize = ID_PROG_E;
pub const FIELD_ID_ARGS: usize = ID_ARGS_E;
pub const FIELD_ID_ICON: usize = ID_ICON_E;
pub const FIELD_ID_MATCH: usize = ID_MATCH_E;
pub const FIELD_SCOPE: usize = ID_SCOPE_C;
pub const FIELD_CONSOLE: usize = ID_CONSOLE;

/// Every edit hosted on the form card (they get the card-painted well).
const FIELD_EDITS: [usize; 5] = [ID_TITLE_E, ID_PROG_E, ID_ARGS_E, ID_ICON_E, ID_MATCH_E];
/// Statics whose background must erase with the card face.
const ON_CARD_STATICS: [usize; 7] = [
    ID_TITLE_L, ID_PROG_L, ID_MATCH_L, ID_SCOPE_L, ID_ARGS_L, ID_ICON_L, ID_EMPTY,
];

// ---- geometry (logical px, scaled at creation) ------------------------------

const CLIENT_W: i32 = 680;
const CLIENT_H: i32 = 546;
const EDGE: i32 = 16;
const IN_X: i32 = EDGE + 12;
const COL_GAP: i32 = 8;
const COL_W: i32 = (CLIENT_W - 2 * IN_X - COL_GAP) / 2;
const COL_A: i32 = IN_X;
const COL_B: i32 = IN_X + COL_W + COL_GAP;

const HEADER_Y: i32 = 16;
const HEADER_H: i32 = 24;
const LIST_Y: i32 = 44;
const LIST_H: i32 = 168;
const ACTIONS_Y: i32 = 224;
const ACT_H: i32 = 30;
const ACT_W: i32 = 104;
const FORM_TITLE_Y: i32 = 268;
const FORM_Y: i32 = 288;

const LABEL_H: i32 = 18;
const LABEL_GAP: i32 = 4;
const FIELD_H: i32 = 34; // field slot; the edit itself is inset inside
const SLOT_GAP: i32 = 10;
const BROWSE_W: i32 = 80;
const BTN_H: i32 = 28;

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
    static FONT: OnceLock<isize> = OnceLock::new();
    HFONT(*FONT.get_or_init(|| crate::make_font_pub(14, 400).0 as isize) as *mut core::ffi::c_void)
}

fn label_font() -> HFONT {
    static FONT: OnceLock<isize> = OnceLock::new();
    HFONT(*FONT.get_or_init(|| crate::make_font_pub(13, 400).0 as isize) as *mut core::ffi::c_void)
}

fn section_font() -> HFONT {
    static FONT: OnceLock<isize> = OnceLock::new();
    HFONT(*FONT.get_or_init(|| crate::make_font_pub(14, 600).0 as isize) as *mut core::ffi::c_void)
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
    let mut buffer = [0u16; 1024];
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

pub fn show() {
    let _dpi = crate::dpi::Scope::window(crate::hwnd(&crate::PANEL));
    unsafe {
        ensure_created();
        let window = mgr();
        refresh_list();
        load_form();
        theme::retheme_children(window);
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
        let target = if LIST_ROW_COUNT.load(Ordering::SeqCst) == 0 {
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
        for id in [ID_SURF, ID_FORM] {
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
    for (id, key) in [
        (ID_FORM_TITLE, "ctx_form_title"),
        (ID_TITLE_L, "ctx_field_title"),
        (ID_SCOPE_L, "ctx_field_scope"),
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
        (ID_SAVE, "common_save"),
        (ID_DISCARD, "common_cancel"),
        (ID_MORE, "ctx_more_settings"),
    ] {
        set_text(id, &crate::t_pub(key));
    }
    set_text(ID_PROG_B, &crate::t_pub("ctx_field_browse"));
    set_text(ID_ICON_B, &crate::t_pub("ctx_field_browse"));
    let title = crate::t_pub("ctx_manager_title");
    unsafe {
        let _ = SetWindowTextW(window, PCWSTR(wide(&title).as_ptr()));
        let _ = SetWindowTextW(
            get_dlg_item(window, ID_TITLE),
            PCWSTR(wide(&title).as_ptr()),
        );
    }
    crate::choice::set_rows(get_dlg_item(window, ID_SCOPE_C), &scope_items());
    refresh_list();
    load_form();
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
            let _ = ShowWindow(mgr(), SW_SHOW);
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
            (FIELD_H - 14).max(20),
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
        // Header: page title left, "more settings" right (manager parity).
        mk_control(
            window,
            windows::core::w!("STATIC"),
            &crate::t_pub("ctx_manager_title"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0x0080),
            ID_TITLE,
            EDGE,
            HEADER_Y,
            320,
            HEADER_H,
            section_font(),
        );
        owner_button(
            window,
            ID_MORE,
            "ctx_more_settings",
            CLIENT_W - EDGE - 120,
            HEADER_Y - 2,
            120,
            BTN_H,
        );

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
        crate::list_style::set_lane_card(list, surf);
        mk_control(
            window,
            windows::core::w!("STATIC"),
            &crate::t_pub("ctx_manager_empty"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0x0080 | 0x0001), // SS_NOPREFIX | SS_CENTER
            ID_EMPTY,
            EDGE + 2,
            LIST_Y + (LIST_H - 20) / 2,
            CLIENT_W - 2 * EDGE - 4,
            20,
            label_font(),
        );

        // Horizontal action row under the list (manager parity).
        let mut x = EDGE;
        for (id, key) in [
            (ID_ADD, "ctx_add"),
            (ID_ADD_SEP, "ctx_add_separator"),
            (ID_DELETE, "ctx_delete"),
            (ID_UP, "ctx_up"),
            (ID_DOWN, "ctx_down"),
        ] {
            owner_button(window, id, key, x, ACTIONS_Y, ACT_W, ACT_H);
            x += ACT_W + COL_GAP;
        }

        // Form section title with the guidance/validation line at its right.
        mk_control(
            window,
            windows::core::w!("STATIC"),
            &crate::t_pub("ctx_form_title"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0x0080),
            ID_FORM_TITLE,
            EDGE,
            FORM_TITLE_Y,
            200,
            LABEL_H + 2,
            section_font(),
        );
        mk_control(
            window,
            windows::core::w!("STATIC"),
            "",
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0x0080 | 0x0002 | 0x4000), // SS_NOPREFIX | SS_RIGHT | SS_ENDELLIPSIS
            ID_VALID,
            CLIENT_W - EDGE - 420,
            FORM_TITLE_Y + 2,
            420,
            LABEL_H,
            label_font(),
        );

        // Form card hosts every field row below.
        // Card closes 8px under the action row.
        let form_card_h = CLIENT_H - EDGE - FORM_Y;
        surface_card(
            window,
            ID_FORM,
            EDGE,
            FORM_Y,
            CLIENT_W - 2 * EDGE,
            form_card_h,
        );

        // Two-column label-above-field grid (editor grammar).
        let f1 = FORM_Y + 12;
        let field1 = f1 + LABEL_H + LABEL_GAP;
        form_label(window, ID_TITLE_L, "ctx_field_title", COL_A, f1, COL_W);
        form_label(window, ID_SCOPE_L, "ctx_field_scope", COL_B, f1, COL_W);
        form_edit(window, ID_TITLE_E, COL_A, field1, COL_W);
        // choice::create scales its geometry internally: pass LOGICAL
        // values, pre-scaling here put the dropdown double-scaled into the
        // window's bottom-right corner.
        crate::choice::create(
            window,
            ID_SCOPE_C as i32,
            (COL_B, field1, COL_W, FIELD_H),
            &scope_pairs(),
            body_font(),
        );

        // Row 2: program (+browse) | arguments.
        let f2 = field1 + FIELD_H + SLOT_GAP - 2;
        let field2 = f2 + LABEL_H + LABEL_GAP;
        form_label(window, ID_PROG_L, "ctx_field_program", COL_A, f2, COL_W);
        form_label(window, ID_ARGS_L, "ctx_field_args", COL_B, f2, COL_W);
        form_edit(window, ID_PROG_E, COL_A, field2, COL_W - BROWSE_W - COL_GAP);
        owner_button(
            window,
            ID_PROG_B,
            "ctx_field_browse",
            COL_A + COL_W - BROWSE_W,
            field2,
            BROWSE_W,
            FIELD_H,
        );
        form_edit(window, ID_ARGS_E, COL_B, field2, COL_W);

        // Row 3: file types | icon (+browse).
        let f3 = field2 + FIELD_H + SLOT_GAP - 2;
        let field3 = f3 + LABEL_H + LABEL_GAP;
        form_label(window, ID_MATCH_L, "ctx_field_match", COL_A, f3, COL_W);
        form_label(window, ID_ICON_L, "ctx_field_icon", COL_B, f3, COL_W);
        form_edit(window, ID_MATCH_E, COL_A, field3, COL_W);
        form_edit(window, ID_ICON_E, COL_B, field3, COL_W - BROWSE_W - COL_GAP);
        owner_button(
            window,
            ID_ICON_B,
            "ctx_field_browse",
            COL_B + COL_W - BROWSE_W,
            field3,
            BROWSE_W,
            FIELD_H,
        );

        // Bottom row: console switch left, discard/save flush right.
        let right = COL_B + COL_W;
        let bottom = field3 + FIELD_H + SLOT_GAP;
        let console = mk_control(
            window,
            windows::core::w!("BUTTON"),
            &crate::t_pub("ctx_console_label"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | 0x0000000B), // BS_OWNERDRAW
            ID_CONSOLE,
            COL_A,
            bottom,
            260,
            BTN_H,
            body_font(),
        );
        crate::nativeform::track(console);
        // Switch rows hover on the pill column alone (panel parity).
        crate::nativeform::set_hover_column(console, s(crate::layout::SWITCH_HIT_W));
        owner_button(
            window,
            ID_DISCARD,
            "common_cancel",
            right - 2 * 96 - COL_GAP,
            bottom,
            96,
            BTN_H,
        );
        owner_button(
            window,
            ID_SAVE,
            "common_save",
            right - 96,
            bottom,
            96,
            BTN_H,
        );
    }
}

unsafe extern "system" fn mgr_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                let id = wparam.0 & 0xFFFF;
                let code = wparam.0 >> 16;
                match id {
                    ID_MORE if code == BN_CLICKED as usize => {
                        // Modal to the manager: settings must not leave this
                        // window interactive behind it.
                        crate::settings_ui::open_ctx_page_modal_to(hwnd);
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
                        apply_form();
                        LRESULT(0)
                    }
                    ID_DISCARD if code == BN_CLICKED as usize => {
                        discard_form();
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
                        let _ = SetFocus(Some(get_dlg_item(hwnd, ID_TITLE_E)));
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
                    ID_SCOPE_C if code == BN_CLICKED as usize => {
                        // The choice button opens its popup only when the
                        // owner toggles it (settings combo parity); the
                        // selection read-back doubles as the dirty check.
                        crate::choice::toggle(
                            get_dlg_item(hwnd, ID_SCOPE_C),
                            hwnd,
                            ID_SCOPE_C as i32,
                        );
                        refresh_form_buttons();
                        DefWindowProcW(hwnd, msg, wparam, lparam)
                    }
                    _ if (code == EN_SETFOCUS || code == EN_KILLFOCUS)
                        && FIELD_EDITS.contains(&id) =>
                    {
                        // The well's focus ring is card pixels: repaint just
                        // that well, never the whole card face.
                        let edit = HWND(lparam.0 as *mut core::ffi::c_void);
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
            WM_CTLCOLORSTATIC => {
                let palette = theme::palette();
                let control = HWND(lparam.0 as *mut core::ffi::c_void);
                let id = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
                    control,
                    windows::Win32::UI::WindowsAndMessaging::GWL_ID,
                ) as usize;
                let on_surface = ON_CARD_STATICS.contains(&id);
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut core::ffi::c_void);
                let _ = windows::Win32::Graphics::Gdi::SetTextColor(hdc, COLORREF(palette.muted));
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
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut core::ffi::c_void);
                let _ = windows::Win32::Graphics::Gdi::SetTextColor(hdc, COLORREF(palette.text));
                let _ = windows::Win32::Graphics::Gdi::SetBkColor(hdc, COLORREF(palette.surface));
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
    }
}

unsafe fn draw_item(hwnd: HWND, item: &crate::nativeform::DrawItem) {
    unsafe {
        // Card repaints end in one SRCCOPY of the whole card rect: cut the
        // hosted children's rects out of the target DC first so the blit can
        // never paint over their pixels (settings/edit card parity).
        if item.control_id as usize == ID_FORM {
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
        if id == ID_SURF {
            crate::paint::draw_surface(
                dc,
                bounds,
                p.window_bg,
                p.surface,
                p.border,
                crate::paint::control_radius(),
            );
            crate::list_style::paint_lane(
                dc,
                item.control,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
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
            crate::paint::draw_label(
                dc,
                &row,
                body_font(),
                text.as_str(),
                if selected { p.text } else { p.text2 },
                true,
                0,
                0,
            );
            if focused {
                crate::paint::draw_focus_frame(dc, bounds, p.focus);
            }
        } else if crate::choice::is_choice(item.control) {
            let state = crate::nativeform::control_state(item.control, item.state);
            crate::choice::draw_button(item.control, dc, bounds, state, p.surface);
        } else if id == ID_CONSOLE {
            // Panel switch grammar on the card face.
            let label = text_of(mgr(), ID_CONSOLE);
            let mut state = crate::nativeform::control_state(item.control, item.state);
            state.active = CONSOLE_HIDE.load(Ordering::SeqCst);
            let progress = crate::switch_animation_progress(item.control);
            crate::paint::draw_switch_row(
                dc,
                bounds,
                body_font(),
                &label,
                p,
                p.surface,
                state,
                scale,
                progress,
            );
        } else {
            let label = text_of(mgr(), id);
            // Buttons riding a card face erase with the card color.
            let on_card = id == ID_PROG_B || id == ID_ICON_B || id == ID_SAVE || id == ID_DISCARD;
            let background = if on_card { p.surface } else { p.window_bg };
            // Save carries the accent fill of a default action (settings and
            // editor footer parity).
            let mut state = crate::nativeform::control_state(item.control, item.state);
            if id == ID_SAVE {
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
        return crate::t_pub("ctx_separator_row");
    }
    let matched = if rule.patterns.is_empty() {
        crate::t_pub("ctx_match_all_short")
    } else {
        rule.patterns.join(";")
    };
    let program = std::path::Path::new(&rule.command)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| rule.command.clone());
    format!("{}  ·  {}  ·  {}", rule.title, matched, program)
}

fn refresh_list() {
    unsafe {
        let rules = crate::runtime::lock(&crate::ctxexec::CTX_RULES).clone();
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
            if empty { SW_SHOW } else { SW_HIDE },
        );
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(list), None, true);
        *crate::runtime::lock(&DISPLAYED) = rules;
        refresh_action_states();
    }
}

/// Row buttons follow the selection: delete/reorder need a selected rule.
fn refresh_action_states() {
    let has_selection = selected_index().is_some();
    for id in [ID_DELETE, ID_UP, ID_DOWN] {
        unsafe {
            let _ = EnableWindow(get_dlg_item(mgr(), id), has_selection);
        }
    }
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
            Some(WPARAM(index.max(0) as usize)),
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
        core_ctx::IssueCode::CommandRequired => crate::t_pub("ctx_issue_program"),
        core_ctx::IssueCode::TooMany => {
            crate::t_args("ctx_issue_limit", &[&core_ctx::MAX_CTX_RULES.to_string()])
        }
        core_ctx::IssueCode::Parse => issue.message.clone(),
    }
}

/// Reads the form into the working draft.
fn draft_from_form() -> core_ctx::CtxRule {
    let window = mgr();
    let mut draft = crate::runtime::lock(&DRAFT).clone().unwrap_or_default();
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
    draft.hidden_console = CONSOLE_HIDE.load(Ordering::SeqCst);
    draft
}

fn load_form() {
    let window = mgr();
    let draft = crate::runtime::lock(&DRAFT).clone();
    let Some(rule) = draft else {
        set_text(ID_TITLE_E, "");
        set_text(ID_PROG_E, "");
        set_text(ID_ARGS_E, "");
        set_text(ID_ICON_E, "");
        set_text(ID_MATCH_E, "");
        crate::choice::select_index(
            get_dlg_item(window, ID_SCOPE_C),
            scope_row_index(core_ctx::SCOPE_FILE),
        );
        set_console(false);
        set_valid(&crate::t_pub("ctx_form_pick_hint"));
        refresh_form_buttons();
        return;
    };
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
    set_valid("");
    refresh_form_buttons();
}

/// Save/Cancel follow the draft: both stay disabled until the form differs
/// from the loaded rule (no selection parks them; an untouched rule too —
/// Cancel used to read as a dead button).
fn refresh_form_buttons() {
    let draft = crate::runtime::lock(&DRAFT).clone();
    let dirty = match draft {
        None => false,
        Some(rule) => draft_from_form() != rule,
    };
    for id in [ID_SAVE, ID_DISCARD] {
        unsafe {
            let _ = EnableWindow(get_dlg_item(mgr(), id), dirty);
        }
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
fn apply_form() {
    let draft = draft_from_form();
    let mut rules = crate::runtime::lock(&DISPLAYED).clone();
    let base = crate::runtime::lock(&crate::ctxexec::CTX_RULES).clone();
    let index = crate::runtime::lock(&DRAFT)
        .as_ref()
        .and_then(|rule| rules.iter().position(|item| item.id == rule.id));
    match index {
        Some(position) => rules[position] = draft.clone(),
        None => rules.push(draft.clone()),
    }
    let (prepared, issues) = core_ctx::prepare_rules(rules);
    if let Some(issue) = issues.first() {
        set_valid(&issue_text(issue));
        return;
    }
    if let Err(err) = crate::save_ctx_rules(&base, &prepared) {
        set_valid(&err);
        return;
    }
    let saved = prepared
        .iter()
        .find(|rule| rule.title == draft.title)
        .cloned()
        .unwrap_or(draft);
    let saved_id = saved.id.clone();
    *crate::runtime::lock(&DRAFT) = Some(saved);
    refresh_list();
    let position = crate::runtime::lock(&DISPLAYED)
        .iter()
        .position(|rule| rule.id == saved_id)
        .unwrap_or(0);
    select_index(position as i32);
    load_form();
}

fn begin_new_rule() {
    *crate::runtime::lock(&DRAFT) = Some(core_ctx::CtxRule::default());
    load_form();
    // A blank draft looks identical to the empty selection state; the
    // example hint is what makes the click read as "started something".
    set_valid(&crate::t_pub("ctx_form_new_hint"));
    let _ = unsafe { SetFocus(Some(get_dlg_item(mgr(), ID_TITLE_E))) };
}

fn begin_new_separator() {
    let rule = core_ctx::CtxRule {
        separator: true,
        ..Default::default()
    };
    *crate::runtime::lock(&DRAFT) = Some(rule);
    load_form();
    set_valid(&crate::t_pub("ctx_form_sep_hint"));
    let _ = unsafe { SetFocus(Some(get_dlg_item(mgr(), ID_ADD))) };
}

/// Discard: reload the selected rule; a brand-new draft just cancels out.
fn discard_form() {
    let is_new = crate::runtime::lock(&DRAFT)
        .as_ref()
        .is_some_and(|rule| rule.id.is_empty());
    if is_new {
        *crate::runtime::lock(&DRAFT) = None;
    }
    load_form();
}

fn selection_changed() {
    // Switching rows commits a valid draft (Open++-like flow); an invalid
    // one is discarded — the validation line explains what was missing.
    let draft = draft_from_form();
    let dirty = crate::runtime::lock(&DRAFT).as_ref() != Some(&draft);
    if dirty && !draft.title.is_empty() && !draft.command.is_empty() {
        apply_form();
    }
    let Some(index) = selected_index() else {
        *crate::runtime::lock(&DRAFT) = None;
        load_form();
        return;
    };
    let rule = crate::runtime::lock(&DISPLAYED)[index].clone();
    *crate::runtime::lock(&DRAFT) = Some(rule);
    load_form();
}

fn delete_selected() {
    let Some(index) = selected_index() else {
        return;
    };
    let base = crate::runtime::lock(&crate::ctxexec::CTX_RULES).clone();
    let mut rules = base.clone();
    if index >= rules.len() {
        return;
    }
    let title = rules[index].title.clone();
    let message = crate::t_args("ctx_delete_confirm", &[&title]);
    let result = unsafe {
        MessageBoxW(
            Some(mgr()),
            PCWSTR(wide(&message).as_ptr()),
            PCWSTR(wide(&crate::t_pub("ctx_manager_title")).as_ptr()),
            MB_YESNO | MB_ICONQUESTION,
        )
    };
    if result != IDYES {
        return;
    }
    rules.remove(index);
    if let Err(err) = crate::save_ctx_rules(&base, &rules) {
        set_valid(&err);
        return;
    }
    *crate::runtime::lock(&DRAFT) = None;
    refresh_list();
    let count = crate::runtime::lock(&DISPLAYED).len();
    select_index(index.min(count.saturating_sub(1)) as i32);
    selection_changed();
}

fn move_selected(delta: i32) {
    let Some(index) = selected_index() else {
        return;
    };
    let target = index as i32 + delta;
    let len = crate::runtime::lock(&DISPLAYED).len() as i32;
    if target < 0 || target >= len {
        return;
    }
    let base = crate::runtime::lock(&crate::ctxexec::CTX_RULES).clone();
    let mut rules = base.clone();
    if index >= rules.len() {
        return;
    }
    rules.swap(index, target as usize);
    if let Err(err) = crate::save_ctx_rules(&base, &rules) {
        set_valid(&err);
        return;
    }
    refresh_list();
    select_index(target);
    selection_changed();
}

fn browse_for_file(edit_id: usize, filter_key: &str) {
    let filters = crate::t_pub(filter_key);
    let mut filter_wide: Vec<u16> = filters
        .replace('|', "\0")
        .encode_utf16()
        .chain([0, 0])
        .collect();
    let initial = text_of(mgr(), edit_id);
    let mut file = [0u16; 1024];
    let prefix_len = initial.encode_utf16().count().min(1000);
    file[..prefix_len].copy_from_slice(&initial.encode_utf16().collect::<Vec<u16>>());
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: mgr(),
        lpstrFilter: PCWSTR(filter_wide.as_mut_ptr()),
        lpstrFile: windows::core::PWSTR(file.as_mut_ptr()),
        nMaxFile: file.len() as u32,
        lpstrTitle: PCWSTR(wide(&crate::t_pub("ctx_browse_title")).as_ptr()),
        Flags: OFN_FILEMUSTEXIST | OFN_HIDEREADONLY,
        ..Default::default()
    };
    let picked = unsafe { GetOpenFileNameW(&mut ofn) };
    if picked.as_bool() {
        let len = file.iter().position(|ch| *ch == 0).unwrap_or(file.len());
        let path = String::from_utf16_lossy(&file[..len]);
        set_text(edit_id, &path);
    }
}
