#![allow(clippy::manual_dangling_ptr)]
//! Automation task UI: manager (rule list), editor (form), and process
//! picker — a three-level modal chain mirroring the Go UX. All owner-drawn
//! controls use the paint.rs system; geometry, defaults, and behavior follow
//! the Go automationpanel/processpicker sources.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};

use idletrigger_core::automation as auto;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use windows::Win32::Graphics::Gdi::HDC;

use crate::choice::ChoiceItem;
use crate::{t_pub, theme};

// ===== Control IDs ==========================================================

// Manager (Go idTitle 100, idList 102, idNext 103, idNew/Edit/Toggle/Delete).
const MGR_LIST: usize = 300;
const MGR_NEW: usize = 301;
const MGR_EDIT: usize = 302;
const MGR_DELETE: usize = 303;
const MGR_TOGGLE: usize = 304;
const MGR_LIST_SURFACE: usize = 310;
const MGR_TITLE: usize = 311;
const MGR_NEXT: usize = 312;
const MGR_EMPTY_TITLE: usize = 313;
const MGR_EMPTY_BODY: usize = 314;

// Editor (Go idName 200 … idFieldSurfaceBase 500).
const ED_NAME: usize = 311;
const ED_ACTION: usize = 313;
const ED_TRIGGER: usize = 315;
const ED_TIME: usize = 317;
const ED_SAVE: usize = 319;
const ED_CANCEL: usize = 320;
const ED_END_TIME: usize = 325;
const ED_WARNING: usize = 326;
const ED_IDLE_MIN: usize = 327;
const ED_DATE: usize = 328;
const ED_LOGIC: usize = 329;
const ED_MAX_WAIT: usize = 330;
const ED_KEEP_SCREEN: usize = 331;
const ED_DAYS_MON: usize = 332;
const ED_DAYS_TUE: usize = 333;
const ED_DAYS_WED: usize = 334;
const ED_DAYS_THU: usize = 335;
const ED_DAYS_FRI: usize = 336;
const ED_DAYS_SAT: usize = 337;
const ED_DAYS_SUN: usize = 338;
const ED_BASICS_TITLE: usize = 340;
const ED_TRIGGER_TITLE: usize = 341;
const ED_OPTIONS_TITLE: usize = 342;
const ED_NAME_LBL: usize = 343;
const ED_ACTION_LBL: usize = 344;
const ED_TRIGGER_LBL: usize = 345;
const ED_TIME_LBL: usize = 346;
const ED_DATE_LBL: usize = 347;
const ED_END_LBL: usize = 348;
const ED_LOGIC_LBL: usize = 349;
const ED_WARN_LBL: usize = 350;
const ED_IDLE_LBL: usize = 351;
const ED_MAX_LBL: usize = 352;
const ED_DAYS_LBL: usize = 353;
const ED_BLOCKED_LBL: usize = 354;
const ED_NAME_HINT: usize = 355;
const ED_NO_OPTIONS: usize = 356;
const ED_PROC_SUMMARY: usize = 357;
const ED_BLOCKED: usize = 358;
const ED_DAYS_WORKDAYS: usize = 359;
const ED_DAYS_EVERYDAY: usize = 360;
const ED_VALIDATION: usize = 361;
const ED_CHOOSE: usize = 362;
const ED_PROC_INFO: usize = 363;

/// Every editor control laid out by layout_editor (Go editorControlIDs).
const ED_LAYOUT_IDS: [usize; 44] = [
    ED_BASICS_TITLE,
    ED_NAME_LBL,
    ED_NAME,
    ED_NAME_HINT,
    ED_ACTION_LBL,
    ED_ACTION,
    ED_TRIGGER_LBL,
    ED_TRIGGER,
    ED_TRIGGER_TITLE,
    ED_TIME_LBL,
    ED_TIME,
    ED_DATE_LBL,
    ED_DATE,
    ED_END_LBL,
    ED_END_TIME,
    ED_DAYS_LBL,
    ED_DAYS_WORKDAYS,
    ED_DAYS_EVERYDAY,
    ED_DAYS_SUN,
    ED_DAYS_MON,
    ED_DAYS_TUE,
    ED_DAYS_WED,
    ED_DAYS_THU,
    ED_DAYS_FRI,
    ED_DAYS_SAT,
    ED_LOGIC_LBL,
    ED_LOGIC,
    ED_CHOOSE,
    ED_PROC_SUMMARY,
    ED_PROC_INFO,
    ED_OPTIONS_TITLE,
    ED_WARN_LBL,
    ED_WARNING,
    ED_IDLE_LBL,
    ED_IDLE_MIN,
    ED_BLOCKED_LBL,
    ED_BLOCKED,
    ED_MAX_LBL,
    ED_MAX_WAIT,
    ED_KEEP_SCREEN,
    ED_NO_OPTIONS,
    ED_VALIDATION,
    ED_SAVE,
    ED_CANCEL,
];

// Picker (Go idSearch 101 … idHelper 113).
const PK_SEARCH: usize = 400;
const PK_LIST: usize = 401;
const PK_CANCEL: usize = 402;
const PK_CONFIRM: usize = 403;
const PK_HEADING: usize = 404;
const PK_HELPER: usize = 405;
const PK_STATUS: usize = 406;
const PK_PRIVACY: usize = 407;
const PK_PREVIEW_TITLE: usize = 408;
const PK_PREVIEW: usize = 409;
const PK_REFRESH: usize = 412;
const PK_BROWSE: usize = 413;
const PK_EMPTY: usize = 344;
const PK_SEARCH_SURFACE: usize = 420;
const PK_LIST_SURFACE: usize = 421;
const PK_PREVIEW_SURFACE: usize = 422;

// ===== Window handles =======================================================

static MGR_HWND: AtomicIsize = AtomicIsize::new(0);
static MGR_LIST_HWND: AtomicIsize = AtomicIsize::new(0);
static EDIT_HWND: AtomicIsize = AtomicIsize::new(0);
static PICKER_HWND: AtomicIsize = AtomicIsize::new(0);
static PK_LIST_HWND: AtomicIsize = AtomicIsize::new(0);

/// Editor working state.
static EDIT_INDEX: AtomicI32 = AtomicI32::new(-1); // -1 = new
static EDIT_ERROR: AtomicBool = AtomicBool::new(false);
static EDIT_ORIG: Mutex<Option<auto::Rule>> = Mutex::new(None);

// Owner-draw checkboxes track state here (BS_OWNERDRAW absorbs the native
// check bits); keyed by control id, exactly like Go's p.checks map.
static EDIT_CHECKS: Mutex<Option<HashMap<usize, bool>>> = Mutex::new(None);

fn edit_checks() -> std::sync::MutexGuard<'static, Option<HashMap<usize, bool>>> {
    EDIT_CHECKS.lock().unwrap()
}

fn edit_is_checked(id: usize) -> bool {
    edit_checks()
        .as_ref()
        .and_then(|m| m.get(&id).copied())
        .unwrap_or(false)
}

fn edit_set_checked(ed: HWND, id: usize, value: bool) {
    edit_checks()
        .get_or_insert_with(Default::default)
        .insert(id, value);
    unsafe {
        let control = get_dlg_item(ed, id);
        if !control.is_invalid() {
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, true);
        }
    }
}

fn edit_toggle(ed: HWND, id: usize) {
    let next = !edit_is_checked(id);
    edit_set_checked(ed, id, next);
}

static EDIT_PROCS: Mutex<Vec<auto::ProcessTarget>> = Mutex::new(Vec::new());

// ===== Geometry (Go tokens) =================================================

// Manager: manager.go / automationpanel.go.
const MGR_PAD: i32 = 18;
const MGR_TITLE_Y: i32 = 16; // formEdgePadding
const MGR_TEXT_H: i32 = 18; // formTextHeight
const MGR_LIST_Y: i32 = MGR_TITLE_Y + MGR_TEXT_H + 12;
const MGR_LIST_H: i32 = 240;
const MGR_STATUS_Y: i32 = MGR_LIST_Y + MGR_LIST_H + 8;
const MGR_BUTTONS_Y: i32 = MGR_STATUS_Y + MGR_TEXT_H + 16;
const MGR_W: i32 = 600;
const MGR_H: i32 = MGR_BUTTONS_Y + BUTTON_H + 18;

// Editor: editor.go / nativeform metrics.go.
const ED_W: i32 = 680;
const ED_PAD: i32 = 18; // FormPadding
const ED_GAP: i32 = 8; // ControlGap
const ED_EDGE: i32 = 16; // formEdgePadding
const ED_LABEL_H: i32 = 18; // formTextHeight
const ED_LABEL_GAP: i32 = 4; // formLabelGap
const ED_CONTENT_GAP: i32 = 12; // formContentGap
const ED_RELATED_GAP: i32 = 8; // formRelatedGap
const ED_SECTION_GAP: i32 = 16; // formSectionGap
const ED_FIELD_H: i32 = 34; // FieldHeight
const ED_CHECK_H: i32 = 28; // checkboxRowHeight
const ED_SUMMARY_H: i32 = 22; // processSummaryRowH
const ED_DIALOG_W: i32 = 104; // DialogButtonWidth
const ED_CHECKBOX_SIZE: i32 = 16; // CheckboxSize

// Edit surfaces use the Go idFieldSurfaceBase offset from the edit id.
const FIELD_SURFACE_BASE: i32 = 500;

// Picker: processpicker.go / nativeform metrics.go.
const PK_W: i32 = 700;
const PK_PAD: i32 = 18;
const PK_TOP: i32 = 16;
const PK_TEXT_H: i32 = 18;
const PK_LABEL_GAP: i32 = 4;
const PK_RELATED_GAP: i32 = 8;
const PK_FIELD_H: i32 = 34;
const PK_LIST_H: i32 = 180;
const PK_PREVIEW_H: i32 = 60;
const PK_HEADING_Y: i32 = PK_TOP;
const PK_HELPER_Y: i32 = PK_HEADING_Y + PK_TEXT_H + PK_LABEL_GAP;
const PK_SEARCH_Y: i32 = PK_HELPER_Y + PK_TEXT_H + PK_RELATED_GAP;
const PK_LIST_Y: i32 = PK_SEARCH_Y + PK_FIELD_H + PK_RELATED_GAP;
const PK_STATUS_Y: i32 = PK_LIST_Y + PK_LIST_H + PK_RELATED_GAP;
const PK_PREVIEW_TITLE_Y: i32 = PK_STATUS_Y + PK_TEXT_H + PK_RELATED_GAP;
const PK_PREVIEW_Y: i32 = PK_PREVIEW_TITLE_Y + PK_TEXT_H + PK_LABEL_GAP;
// GDI STATIC reserves more descent below its glyphs; shift the footer up by
// 1px while keeping the combined footer gaps at 16px (Go optical bias).
const PK_PRIVACY_Y: i32 = PK_PREVIEW_Y + PK_PREVIEW_H + PK_RELATED_GAP - 1;
const PK_BUTTONS_Y: i32 = PK_PRIVACY_Y + PK_TEXT_H + PK_RELATED_GAP + 1;
const PK_H: i32 = PK_BUTTONS_Y + BUTTON_H + PK_TOP;
const PK_BUTTON_W: i32 = 104;

// Raw Win32 tokens that are missing or awkward in the windows crate surface.
const LVM_FIRST: u32 = 0x1000;
const LVM_SETBKCOLOR: u32 = LVM_FIRST + 1;
const LVM_SETIMAGELIST: u32 = LVM_FIRST + 3;
const LVM_DELETEALLITEMS: u32 = LVM_FIRST + 9;
const LVM_GETITEMSTATE: u32 = LVM_FIRST + 44;
const LVM_SETITEMSTATE: u32 = LVM_FIRST + 43;
const LVM_INSERTITEMW: u32 = LVM_FIRST + 77;
const LVM_SUBITEMHITTEST: u32 = LVM_FIRST + 57;
const LVM_SETITEMTEXTW: u32 = LVM_FIRST + 116;
const LVM_SETCOLUMNW: u32 = LVM_FIRST + 96;
const LVM_INSERTCOLUMNW: u32 = LVM_FIRST + 97;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = LVM_FIRST + 54;
const LVM_SETTEXTCOLOR: u32 = LVM_FIRST + 36;
const LVM_SETTEXTBKCOLOR: u32 = LVM_FIRST + 38;
const LVM_GETHEADER: u32 = LVM_FIRST + 31;
const LVSIL_STATE: usize = 2;
const LVS_EX_CHECKBOXES: u32 = 0x0000_0004;
const LVS_EX_FULLROWSELECT: u32 = 0x0000_0020;
const LVS_EX_DOUBLEBUFFER: u32 = 0x0001_0000;
const LBS_NOSEL: u32 = 0x4000;
const EN_CHANGE: u16 = 0x0300;
const EN_SETFOCUS: u16 = 0x0100;
const EN_KILLFOCUS: u16 = 0x0200;
const CBN_SELCHANGE: u16 = 1;
const LBN_SELCHANGE: u16 = 1;
const LBN_DBLCLK: u16 = 2;
const BN_CLICKED: u16 = 0;
const WM_APP_PICKER_DESC: u32 = 0x8000 + 1;
const EM_SETMARGINS_RAW: u32 = 0x00D3;
const EM_SETSEL_RAW: u32 = 0x00B1;
const EM_SETCUEBANNER_RAW: u32 = 0x1501;
const EM_GETSEL_RAW: u32 = 0x00B0;

const BUTTON_H: i32 = 36; // nativeform.ButtonHeight

// ===== Small helpers ========================================================

fn s(v: i32) -> i32 {
    crate::scale_pub(v)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn set_text(hwnd: HWND, text: &str) {
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide(text).as_ptr()));
    }
}

fn get_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        let mut buf = vec![0u16; (len + 1) as usize];
        let n = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

fn get_dlg_item(parent: HWND, id: usize) -> HWND {
    unsafe { GetDlgItem(Some(parent), id as i32).unwrap_or_default() }
}

fn window_text(control: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(control);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(control, &mut buf);
        String::from_utf16_lossy(&buf[..copied.max(0) as usize])
    }
}

fn is_chinese() -> bool {
    crate::i18n_is_chinese()
}

fn center_on_parent(w: i32, h: i32) -> (i32, i32) {
    unsafe {
        let parent = crate::hwnd(&crate::PANEL);
        let mut wr = RECT::default();
        if GetWindowRect(parent, &mut wr).is_ok() {
            let pw = wr.right - wr.left;
            let ph = wr.bottom - wr.top;
            return (wr.left + (pw - s(w)) / 2, wr.top + (ph - s(h)) / 2);
        }
        (s(200), s(200))
    }
}

pub fn section_font_cached() -> windows::Win32::Graphics::Gdi::HFONT {
    font_cache(600)
}

pub fn form_font_body() -> windows::Win32::Graphics::Gdi::HFONT {
    font_cache(400)
}

fn font_cache(weight: i32) -> windows::Win32::Graphics::Gdi::HFONT {
    #[derive(Clone, Copy)]
    struct SendFont(windows::Win32::Graphics::Gdi::HFONT);
    // Fonts are pointers into this process's GDI handle table; both values
    // are process-wide constants created once on the UI thread.
    unsafe impl Send for SendFont {}
    unsafe impl Sync for SendFont {}
    static BODY: std::sync::OnceLock<SendFont> = std::sync::OnceLock::new();
    static SECTION: std::sync::OnceLock<SendFont> = std::sync::OnceLock::new();
    let slot = if weight >= 600 { &SECTION } else { &BODY };
    slot.get_or_init(|| SendFont(crate::make_font_pub(14, weight)))
        .0
}

fn secondary_style() -> WINDOW_STYLE {
    WINDOW_STYLE(WS_OVERLAPPEDWINDOW.0 & !WS_MAXIMIZEBOX.0 & !WS_THICKFRAME.0 & !WS_MINIMIZEBOX.0)
}

fn confirm_dialog(parent: HWND, title: &str, body: &str) -> bool {
    unsafe {
        let result = MessageBoxW(
            Some(parent),
            PCWSTR(wide(body).as_ptr()),
            PCWSTR(wide(title).as_ptr()),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2, // Go mbYesNoWarningDefaultNo
        );
        result == IDYES
    }
}

fn info_dialog(parent: HWND, title: &str, body: &str) {
    unsafe {
        let _ = MessageBoxW(
            Some(parent),
            PCWSTR(wide(body).as_ptr()),
            PCWSTR(wide(title).as_ptr()),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

fn enable_control(parent: HWND, id: usize, enabled: bool) {
    unsafe {
        let control = get_dlg_item(parent, id);
        if control.is_invalid() {
            return;
        }
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(control, enabled);
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, false);
    }
}

/// Go cleanSingleLine: collapse line breaks and trim.
fn clean_single_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().to_string()
}

fn fill_template(template: &str, values: &[&str]) -> String {
    let mut out = template.to_string();
    for value in values {
        if let Some(pos) = out.find("%s") {
            out.replace_range(pos..pos + 2, value);
        } else if let Some(pos) = out.find("%d") {
            out.replace_range(pos..pos + 2, value);
        }
    }
    out
}

// ===== Label / localization mapping (Go actionKey, triggerKey) =============

fn action_label(value: &str) -> String {
    let key = match value {
        "stay_awake" => "automation_action_stay_awake",
        "pause_stay_awake" => "automation_action_pause_stay_awake",
        "enable_idle_monitor" => "automation_action_enable_idle",
        "pause_idle_monitor" => "automation_action_pause_idle",
        "lock" => "menu_lock",
        "sleep" => "menu_sleep",
        "hibernate" => "menu_hibernate",
        "shutdown" => "menu_shutdown",
        "restart" => "menu_restart",
        _ => return value.to_string(),
    };
    t_pub(key)
}

fn trigger_label(value: &str) -> String {
    let key = match value {
        "process_running" => "trigger_process_running",
        "process_started" => "trigger_process_started",
        "process_exited" => "trigger_process_exited",
        "time_window" => "trigger_time_window",
        "once" => "trigger_once",
        "daily" => "trigger_daily",
        "weekly" => "trigger_weekly",
        _ => return value.to_string(),
    };
    t_pub(key)
}

fn logic_label(value: &str) -> String {
    let key = match value {
        "any" => "automation_process_any",
        "all" => "automation_process_all",
        "none" => "automation_process_none",
        _ => return value.to_string(),
    };
    t_pub(key)
}

fn blocked_label(value: &str) -> String {
    t_pub(if value == "wait" {
        "automation_blocked_wait"
    } else {
        "automation_blocked_skip"
    })
}

fn action_keys() -> Vec<&'static str> {
    // Go actions[] order: state actions first, then system actions.
    vec![
        "stay_awake",
        "pause_stay_awake",
        "enable_idle_monitor",
        "pause_idle_monitor",
        "lock",
        "sleep",
        "hibernate",
        "shutdown",
        "restart",
    ]
}

/// Go setTriggerOptions: triggers are filtered by the selected action type.
pub fn trigger_keys_for(action: &str) -> Vec<&'static str> {
    if auto::is_state_action(action) {
        vec!["process_running", "time_window"]
    } else {
        vec![
            "once",
            "daily",
            "weekly",
            "process_started",
            "process_exited",
        ]
    }
}

fn logic_keys() -> Vec<&'static str> {
    vec!["any", "all", "none"]
}

fn blocked_keys() -> Vec<&'static str> {
    vec!["skip", "wait"]
}

// ===== Choice wrappers ======================================================

fn choice_value(parent: HWND, id: usize) -> String {
    crate::choice::value(get_dlg_item(parent, id))
}

fn choice_select(parent: HWND, id: usize, value: &str) {
    crate::choice::select_value(get_dlg_item(parent, id), value);
}

/// Rebuilds the action choice with Go's two group headers.
fn fill_action_choice(parent: HWND) {
    let mut rows = vec![ChoiceItem::header(&t_pub("automation_action_group_state"))];
    for key in action_keys() {
        if key == "lock" {
            rows.push(ChoiceItem::header(&t_pub("automation_action_group_system")));
        }
        rows.push(ChoiceItem::option(key, &action_label(key)));
    }
    crate::choice::set_rows(get_dlg_item(parent, ED_ACTION), &rows);
}

fn fill_trigger_choice(parent: HWND, action: &str, desired: &str) {
    let keys = trigger_keys_for(action);
    let rows: Vec<ChoiceItem> = keys
        .iter()
        .map(|k| ChoiceItem::option(k, &trigger_label(k)))
        .collect();
    let button = get_dlg_item(parent, ED_TRIGGER);
    crate::choice::set_rows(button, &rows);
    if crate::choice::select_value(button, desired).is_none() {
        crate::choice::select_index(button, 0);
    }
}

fn fill_logic_choice(parent: HWND) {
    let rows: Vec<ChoiceItem> = logic_keys()
        .iter()
        .map(|k| ChoiceItem::option(k, &logic_label(k)))
        .collect();
    crate::choice::set_rows(get_dlg_item(parent, ED_LOGIC), &rows);
}

fn fill_blocked_choice(parent: HWND) {
    let rows: Vec<ChoiceItem> = blocked_keys()
        .iter()
        .map(|k| ChoiceItem::option(k, &blocked_label(k)))
        .collect();
    crate::choice::set_rows(get_dlg_item(parent, ED_BLOCKED), &rows);
}

// ===== Manager ==============================================================

pub fn ensure_created() {
    unsafe {
        if HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _).0 as usize != 0 {
            return;
        }
        let instance = GetModuleHandleW(None).expect("module handle");
        let font = form_font_body();
        let section_font = section_font_cached();

        register_mgr_class(instance);

        let style = secondary_style();
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: s(MGR_W),
            bottom: s(MGR_H),
        };
        let _ = AdjustWindowRectEx(&mut frame, style, false, WINDOW_EX_STYLE(0));
        let (x, y) = center_on_parent(frame.right - frame.left, frame.bottom - frame.top);

        let mgr = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("IdleTriggerAutoMgr"),
            PCWSTR(wide(&t_pub("automation_title")).as_ptr()),
            style,
            x,
            y,
            frame.right - frame.left,
            frame.bottom - frame.top,
            Some(crate::hwnd(&crate::PANEL)),
            None,
            Some(instance.into()),
            None,
        )
        .expect("manager window");
        MGR_HWND.store(mgr.0 as isize, Ordering::SeqCst);
        theme::apply_to_window(mgr);
        crate::set_window_icons_pub(mgr);

        let content_w = MGR_W - 2 * MGR_PAD;
        let mk_static = |id: usize,
                         text: &str,
                         font: windows::Win32::Graphics::Gdi::HFONT,
                         x: i32,
                         y: i32,
                         w: i32,
                         h: i32| {
            let wide_text: Vec<u16> = text.encode_utf16().chain([0]).collect();
            let hwnd_ = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                PCWSTR(wide_text.as_ptr()),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
                s(x),
                s(y),
                s(w),
                s(h),
                Some(mgr),
                Some(HMENU(id as *mut _)),
                Some(instance.into()),
                None,
            )
            .expect("mgr static");
            let _ = SendMessageW(
                hwnd_,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            hwnd_
        };

        let _ = mk_static(
            MGR_TITLE,
            &t_pub("automation_rules_title"),
            section_font,
            MGR_PAD,
            MGR_TITLE_Y,
            content_w,
            MGR_TEXT_H,
        );

        // Rounded surface behind the listbox (Go idListSurface), then the
        // list inset 2px inside it.
        let surface = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("STATIC"),
            windows::core::w!(""),
            // Go formSurfaceStyle: WS_CLIPSIBLINGS keeps sibling repaint
            // order sane (the list paints over the card, never under it).
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0 | 13), // SS_OWNERDRAW
            s(MGR_PAD),
            s(MGR_LIST_Y),
            s(content_w),
            s(MGR_LIST_H),
            Some(mgr),
            Some(HMENU(MGR_LIST_SURFACE as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("mgr list surface");
        let _ = SetWindowPos(
            surface,
            Some(windows::Win32::Foundation::HWND(1 as *mut _)), // HWND_BOTTOM
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        MGR_LIST_SURFACE_HWND.store(surface.0 as isize, Ordering::SeqCst);

        let list = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("LISTBOX"),
            windows::core::w!(""),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_TABSTOP.0
                    | WS_VSCROLL.0
                    | WS_CLIPSIBLINGS.0
                    | LBS_NOTIFY as u32
                    | windows::Win32::UI::WindowsAndMessaging::LBS_NOINTEGRALHEIGHT as u32,
            ),
            s(MGR_PAD + 2),
            s(MGR_LIST_Y + 2),
            s(content_w - 4),
            s(MGR_LIST_H - 4),
            Some(mgr),
            Some(HMENU(MGR_LIST as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("mgr list");
        let _ = SendMessageW(
            list,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        MGR_LIST_HWND.store(list.0 as isize, Ordering::SeqCst);

        // Empty-state overlay centered in the list card.
        let empty_y = MGR_LIST_Y + (MGR_LIST_H - (2 * MGR_TEXT_H + 12)) / 2;
        let _ = mk_static(
            MGR_EMPTY_TITLE,
            &t_pub("automation_empty_title"),
            section_font,
            MGR_PAD + 24,
            empty_y,
            content_w - 48,
            MGR_TEXT_H,
        );
        let _ = mk_static(
            MGR_EMPTY_BODY,
            &t_pub("automation_empty_body"),
            font,
            MGR_PAD + 24,
            empty_y + MGR_TEXT_H + 12,
            content_w - 48,
            MGR_TEXT_H,
        );

        // Status line under the list (Go idNext).
        let _ = mk_static(
            MGR_NEXT,
            "",
            font,
            MGR_PAD,
            MGR_STATUS_Y,
            content_w,
            MGR_TEXT_H,
        );

        // Button row: Go 116/116/116/192 grid (the wide one toggles).
        for (id, label_key, x, w) in [
            (MGR_NEW, "automation_new", MGR_PAD, 116),
            (MGR_EDIT, "automation_edit", MGR_PAD + 116 + 8, 116),
            (
                MGR_DELETE,
                "automation_delete",
                MGR_PAD + 2 * (116 + 8),
                116,
            ),
            (
                MGR_TOGGLE,
                "automation_toggle",
                MGR_PAD + 3 * (116 + 8),
                192,
            ),
        ] {
            let btn = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("BUTTON"),
                PCWSTR(wide(&t_pub(label_key)).as_ptr()),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
                s(x),
                s(MGR_BUTTONS_Y),
                s(w),
                s(BUTTON_H),
                Some(mgr),
                Some(HMENU(id as *mut _)),
                Some(instance.into()),
                None,
            )
            .expect("mgr button");
            let _ = SendMessageW(
                btn,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            crate::nativeform::track(btn);
        }
    }
}

static MGR_LIST_SURFACE_HWND: AtomicIsize = AtomicIsize::new(0);

pub fn show() {
    unsafe {
        ensure_created();
        let mgr = HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _);
        refresh_list();
        theme::retheme_children(mgr);
        present_control(HWND(MGR_LIST_HWND.load(Ordering::SeqCst) as *mut _));
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
            crate::hwnd(&crate::PANEL),
            false,
        );
        // Go BeginFirstFrame/Reveal: cloak, commit one full frame, uncloak.
        crate::FirstFrameGate::begin(mgr).reveal();
        let _ = SetForegroundWindow(mgr);
        // Go focuses the list (or New when empty) after showing the manager.
        let empty = crate::automation::RULES.lock().unwrap().is_empty();
        let target = if empty {
            get_dlg_item(mgr, MGR_NEW)
        } else {
            HWND(MGR_LIST_HWND.load(Ordering::SeqCst) as *mut _)
        };
        if !target.is_invalid() {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(target));
        }
    }
}

fn hide() {
    unsafe {
        let mgr = HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _);
        let _ = ShowWindow(mgr, SW_HIDE);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
            crate::hwnd(&crate::PANEL),
            true,
        );
        crate::refresh_status();
    }
}

fn refresh_list() {
    unsafe {
        let list = HWND(MGR_LIST_HWND.load(Ordering::SeqCst) as *mut _);

        // Keep the previous selection and top index across the reset (Go
        // populateRules remembers both before clearing).
        let rules = crate::automation::RULES.lock().unwrap().clone();
        let issues = crate::automation::ISSUES.lock().unwrap().clone();

        let had_selection = SendMessageW(list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0;
        let top = SendMessageW(list, LB_GETTOPINDEX, Some(WPARAM(0)), Some(LPARAM(0))).0;
        let _ = SendMessageW(list, LB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));

        let mut selected: isize = -1;
        for (index, rule) in rules.iter().enumerate() {
            // Go row text: "state  name — summary" with an invalid marker.
            let issue = issues
                .iter()
                .find(|i| i.index == index || (!i.rule_id.is_empty() && i.rule_id == rule.id));
            let (state, summary) = match &issue {
                Some(i) => (t_pub("automation_rule_invalid"), i.message.clone()),
                None => (
                    if rule.enabled {
                        t_pub("automation_rule_enabled")
                    } else {
                        t_pub("automation_rule_disabled")
                    },
                    rule_summary(rule),
                ),
            };
            let line = format!("{}  {} — {}", state, rule.name, summary);
            let _ = SendMessageW(
                list,
                LB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(wide(&line).as_ptr() as isize)),
            );
            if had_selection >= 0 && index as isize == had_selection {
                selected = index as isize;
            }
        }

        let empty = rules.is_empty();
        if !empty {
            if selected < 0 {
                selected = 0; // Go auto-selects the first row.
            }
            let _ = SendMessageW(
                list,
                LB_SETCURSEL,
                Some(WPARAM(selected as usize)),
                Some(LPARAM(0)),
            );
            if top >= 0 {
                let _ = SendMessageW(
                    list,
                    LB_SETTOPINDEX,
                    Some(WPARAM(top as usize)),
                    Some(LPARAM(0)),
                );
            }
        }

        present_control(list);

        update_manager_actions(mgr_from_list(), &rules, selected);

        // Status line mirrors the Go managerStatusText.
        let status = if empty {
            t_pub("automation_no_upcoming")
        } else {
            match crate::automation::next_scheduled() {
                Some(next) => t_pub("automation_next_format").replace("%s", &next),
                None => t_pub("automation_no_upcoming"),
            }
        };
        set_text(get_dlg_item(mgr_from_list(), MGR_NEXT), &status);
    }
}

fn mgr_from_list() -> HWND {
    HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _)
}

/// Go updateManagerActions: enable Edit/Delete/Toggle and relabel Toggle.
fn update_manager_actions(mgr: HWND, rules: &[auto::Rule], selected: isize) {
    let empty = rules.is_empty();
    let has = !empty && selected >= 0 && (selected as usize) < rules.len();
    for id in [MGR_EDIT, MGR_DELETE, MGR_TOGGLE] {
        enable_control(mgr, id, has);
    }
    let toggle_label = if has {
        if rules[selected as usize].enabled {
            "automation_disable"
        } else {
            "automation_enable"
        }
    } else {
        "automation_toggle"
    };
    let control = get_dlg_item(mgr, MGR_TOGGLE);
    set_text(control, &t_pub(toggle_label));
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, false);

        // Empty-state overlay swaps with the list (Go hides idList too).
        let list_control = HWND(MGR_LIST_HWND.load(Ordering::SeqCst) as *mut _);
        if !list_control.is_invalid() {
            let _ = ShowWindow(list_control, if empty { SW_HIDE } else { SW_SHOW });
        }
        for (id, visible) in [(MGR_EMPTY_TITLE, empty), (MGR_EMPTY_BODY, empty)] {
            let overlay = get_dlg_item(mgr, id);
            if !overlay.is_invalid() {
                let _ = ShowWindow(overlay, if visible { SW_SHOW } else { SW_HIDE });
            }
        }
    }
}

/// Go ruleSummary: localized "action · trigger detail" one-liner.
fn rule_summary(rule: &auto::Rule) -> String {
    let action = action_label(&rule.action);
    let count = rule.processes.len().to_string();
    match rule.trigger.as_str() {
        "process_running" => fill_template(
            &t_pub("automation_summary_process_running"),
            &[&action, &count],
        ),
        "process_started" => fill_template(
            &t_pub("automation_summary_process_started"),
            &[&action, &count],
        ),
        "process_exited" => fill_template(
            &t_pub("automation_summary_process_exited"),
            &[&action, &count],
        ),
        "time_window" => fill_template(
            &t_pub("automation_summary_time_window"),
            &[
                &action,
                &rule.time,
                &rule.end_time,
                &day_summary(&rule.days),
            ],
        ),
        "once" => fill_template(
            &t_pub("automation_summary_once"),
            &[&action, &rule.date, &rule.time],
        ),
        "daily" => fill_template(&t_pub("automation_summary_daily"), &[&action, &rule.time]),
        "weekly" => fill_template(
            &t_pub("automation_summary_weekly"),
            &[&action, &day_summary(&rule.days), &rule.time],
        ),
        _ => action,
    }
}

/// Go daySummary: 每天 / 工作日 / 周一、周二… (weekday order mon..sun).
fn day_summary(days: &[String]) -> String {
    let week = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let contains = |key: &str| days.iter().any(|d| d.trim().eq_ignore_ascii_case(key));

    if week.iter().all(|d| contains(d)) {
        return t_pub("automation_days_everyday");
    }
    let workdays = !contains("sat") && !contains("sun") && week[..5].iter().all(|d| contains(d));
    if workdays {
        return t_pub("automation_days_workdays");
    }

    let mut labels = Vec::new();
    for day in week {
        if !contains(day) {
            continue;
        }
        let mut label = t_pub(&format!("automation_day_{day}"));
        if is_chinese() {
            label = format!("周{label}");
        }
        labels.push(label);
    }
    labels.join(&t_pub("automation_days_separator"))
}

fn selected_index() -> Option<usize> {
    unsafe {
        let list = HWND(MGR_LIST_HWND.load(Ordering::SeqCst) as *mut _);
        let sel = SendMessageW(list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0;
        if sel < 0 { None } else { Some(sel as usize) }
    }
}

fn edit_selected() {
    if let Some(idx) = selected_index() {
        let rules = crate::automation::RULES.lock().unwrap();
        if let Some(rule) = rules.get(idx) {
            *EDIT_PROCS.lock().unwrap() = rule.processes.clone();
            drop(rules);
            EDIT_INDEX.store(idx as i32, Ordering::SeqCst);
            show_editor();
        }
    }
}

/// Go idToggle: flip the selected rule's enabled flag and republish.
fn toggle_selected(mgr: HWND) {
    let Some(idx) = selected_index() else {
        return;
    };
    let mut rules = crate::automation::RULES.lock().unwrap();
    if idx >= rules.len() {
        return;
    }
    rules[idx].enabled = !rules[idx].enabled;
    drop(rules);
    save_rules_to_config();
    let _ = mgr;
}

/// Go idDelete: confirm, then remove the selected rule.
fn delete_selected(mgr: HWND) {
    let Some(idx) = selected_index() else {
        return;
    };
    let rules = crate::automation::RULES.lock().unwrap();
    let Some(rule) = rules.get(idx).cloned() else {
        return;
    };
    drop(rules);

    let body = t_pub("automation_delete_confirm").replace("%s", &rule.name);
    if !confirm_dialog(mgr, &t_pub("automation_delete_title"), &body) {
        return;
    }
    let mut rules = crate::automation::RULES.lock().unwrap();
    if idx < rules.len() {
        rules.remove(idx);
    }
    drop(rules);
    save_rules_to_config();
    crate::log_line(&format!("automation: rule {} deleted", rule.id));
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
                let code = wparam.0 & 0xFFFF;
                let hi = ((wparam.0 >> 16) & 0xFFFF) as u16;
                match code {
                    MGR_NEW => {
                        EDIT_INDEX.store(-1, Ordering::SeqCst);
                        *EDIT_PROCS.lock().unwrap() = Vec::new();
                        crate::log_line("automation: new rule editor");
                        show_editor();
                    }
                    MGR_EDIT if hi == BN_CLICKED => edit_selected(),
                    MGR_DELETE if hi == BN_CLICKED => delete_selected(hwnd),
                    MGR_TOGGLE if hi == BN_CLICKED => toggle_selected(hwnd),
                    MGR_LIST if hi == LBN_DBLCLK => edit_selected(),
                    MGR_LIST if hi == LBN_SELCHANGE => {
                        // Go updates the action row on every selection change.
                        let rules = crate::automation::RULES.lock().unwrap().clone();
                        let sel = selected_index().map(|s| s as isize).unwrap_or(-1);
                        update_manager_actions(hwnd, &rules, sel);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                hide();
                LRESULT(0)
            }
            WM_DRAWITEM => {
                if let Some(item) = crate::nativeform::draw_item(lparam) {
                    draw_manager_item(&item);
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
                // Secondary labels (Go isSecondaryLabel) get SecondaryText;
                // section titles keep PrimaryText. The empty-state overlay
                // paints on the Surface card.
                let palette = theme::palette();
                let id = GetWindowLongPtrW(HWND(lparam.0 as *mut _), GWL_ID) as usize;
                let secondary = matches!(id, MGR_EMPTY_BODY | MGR_NEXT);
                let on_surface = matches!(id, MGR_EMPTY_TITLE | MGR_EMPTY_BODY);
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                    hdc,
                    COLORREF(if secondary {
                        palette.text2
                    } else {
                        palette.text
                    }),
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
            WM_CTLCOLORLISTBOX => {
                // Listbox on the surface card: PrimaryText on Surface.
                let p = theme::palette();
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                let _ = windows::Win32::Graphics::Gdi::SetTextColor(hdc, COLORREF(p.text));
                let _ = windows::Win32::Graphics::Gdi::SetBkColor(hdc, COLORREF(p.surface));
                let (light, dark) = theme::surface_brush_pairs();
                let pair = if theme::is_dark() { dark } else { light };
                LRESULT(pair.0.0 as isize)
            }
            WM_DESTROY => LRESULT(0),
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn register_mgr_class(instance: windows::Win32::Foundation::HMODULE) {
    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(mgr_proc),
            hInstance: instance.into(),
            lpszClassName: windows::core::w!("IdleTriggerAutoMgr"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1usize as *const u16))
                .unwrap_or_default(),
            hbrBackground: windows::Win32::Graphics::Gdi::GetSysColorBrush(
                windows::Win32::Graphics::Gdi::COLOR_WINDOW,
            ),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
}

/// WM_DRAWITEM dispatcher for the manager (Go owner-draw parity).
fn draw_manager_item(item: &crate::nativeform::DrawItem) {
    unsafe {
        if crate::nativeform::draw_buffered(item.dc, &item.bounds, |dc, bounds| {
            draw_manager_item_impl(item, dc, bounds);
        }) {
            return;
        }
        draw_manager_item_impl(item, item.dc, &item.bounds);
    }
}

fn draw_manager_item_impl(item: &crate::nativeform::DrawItem, dc: HDC, bounds: &RECT) {
    let p = theme::palette();
    let id = item.control_id as usize;
    if id == MGR_LIST_SURFACE {
        // Card surface behind the listbox: elevated fill + border.
        crate::paint::draw_surface(dc, bounds, p.window_bg, p.surface, p.border, 6);
    } else {
        let label = window_text(item.control);
        let state = crate::nativeform::control_state(item.control, item.state);
        crate::paint::draw_button(
            dc,
            bounds,
            form_font_body(),
            &label,
            p,
            p.window_bg,
            state,
            6,
        );
    }
}

// ===== Time edit subclass (Go time_edit.go) ================================

/// Original wndprocs for the subclassed time edits, keyed by hwnd.
static TIME_EDIT_PROCS: Mutex<Option<HashMap<isize, isize>>> = Mutex::new(None);

/// Installs the time-grammar subclass on a start/end time edit (Go
/// timeEdit): digits and one separator only, auto-inserted colon, hour pad
/// on blur.
fn install_time_edit(edit: HWND) {
    unsafe {
        // WNDPROC roundtrip through GWLP_WNDPROC is the documented contract
        // (same pattern as nativeform::track).
        #[cfg(target_arch = "x86")]
        #[allow(renamed_and_removed_lints, function_casts_as_integer)]
        let old = SetWindowLongPtrW(edit, GWLP_WNDPROC, time_edit_proc as usize as i32) as isize;
        #[cfg(not(target_arch = "x86"))]
        #[allow(renamed_and_removed_lints, function_casts_as_integer)]
        let old = SetWindowLongPtrW(edit, GWLP_WNDPROC, time_edit_proc as usize as isize);
        if old == 0 {
            return;
        }
        TIME_EDIT_PROCS
            .lock()
            .unwrap()
            .get_or_insert_with(Default::default)
            .insert(edit.0 as isize, old);
    }
}

fn call_time_edit_old(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let old = TIME_EDIT_PROCS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(&(hwnd.0 as isize)).copied())
        .unwrap_or(0);
    type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;
    unsafe {
        let proc: WndProc = std::mem::transmute(old);
        CallWindowProcW(Some(proc), hwnd, msg, wparam, lparam)
    }
}

/// Go normalizeTimeEditRune: ASCII/full-width digits and colons map to the
/// small editing grammar; everything else is rejected.
fn time_edit_rune(unit: u32) -> Option<char> {
    match unit {
        0x30..=0x39 => Some(char::from_u32(unit).unwrap()),
        0xFF10..=0xFF19 => char::from_u32(unit - 0xFF10 + 0x30),
        0x3A | 0xFF1A => Some(':'),
        _ => None,
    }
}

/// Go normalizeTimeEditText: up to two hour digits, one separator (inserted
/// automatically on the third digit) and two minute digits; blur pads a
/// one-digit hour. Returns the normalized text and caret.
fn normalize_time_edit_text(value: &str, caret: usize, final_: bool) -> (String, usize) {
    let mut out = String::with_capacity(5);
    let (mut hour_digits, mut minute_digits, mut separator) = (0, 0, false);
    let mut input_offset = 0usize;
    let mut normalized_caret = 0usize;
    for value_char in value.chars() {
        let rune_units = value_char.len_utf16();
        if let Some(mapped) = time_edit_rune(value_char as u32) {
            let m = mapped as u32;
            if (0x30..=0x39).contains(&m) && !separator && hour_digits < 2 {
                out.push(mapped);
                hour_digits += 1;
            } else if (0x30..=0x39).contains(&m) && !separator && minute_digits < 2 {
                out.push(':');
                out.push(mapped);
                separator = true;
                minute_digits += 1;
            } else if (0x30..=0x39).contains(&m) && minute_digits < 2 {
                out.push(mapped);
                minute_digits += 1;
            } else if mapped == ':' && !separator && hour_digits > 0 {
                out.push(mapped);
                separator = true;
            }
        }
        input_offset += rune_units;
        if input_offset <= caret {
            normalized_caret = out.chars().count();
        }
    }
    if final_ && separator && hour_digits == 1 {
        out.insert(0, '0');
        if caret > 0 {
            normalized_caret += 1;
        }
    }
    if normalized_caret > out.chars().count() {
        normalized_caret = out.chars().count();
    }
    (out, normalized_caret)
}

/// Go timeEditNormalizeWindow: reformat the field and restore the caret.
fn time_edit_normalize(hwnd: HWND, final_: bool) {
    let value = get_text(hwnd);
    let caret = unsafe {
        let mut start: u32 = 0;
        let mut end: u32 = 0;
        let _ = SendMessageW(
            hwnd,
            EM_GETSEL_RAW,
            Some(WPARAM(&mut start as *mut u32 as usize)),
            Some(LPARAM(&mut end as *mut u32 as usize as isize)),
        );
        start as usize
    };
    let (normalized, caret) = normalize_time_edit_text(&value, caret, final_);
    if normalized == value {
        return;
    }
    set_text(hwnd, &normalized);
    unsafe {
        let _ = SendMessageW(
            hwnd,
            EM_SETSEL_RAW,
            Some(WPARAM(caret)),
            Some(LPARAM(caret as isize)),
        );
    }
}

unsafe extern "system" fn time_edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        0x0102 if wparam.0 >= 0x20 => {
            // WM_CHAR: block anything outside the time grammar.
            if time_edit_rune(wparam.0 as u32).is_none() {
                return LRESULT(0);
            }
            let result = call_time_edit_old(hwnd, msg, wparam, lparam);
            time_edit_normalize(hwnd, false);
            result
        }
        0x0302 => {
            // WM_PASTE: normalize whatever landed.
            let result = call_time_edit_old(hwnd, msg, wparam, lparam);
            time_edit_normalize(hwnd, false);
            result
        }
        0x0008 => {
            // WM_KILLFOCUS: pad the hour and settle the format.
            time_edit_normalize(hwnd, true);
            call_time_edit_old(hwnd, msg, wparam, lparam)
        }
        _ => call_time_edit_old(hwnd, msg, wparam, lparam),
    }
}

// ===== Rules persistence ====================================================

fn save_rules_to_config() {
    let rules = crate::automation::RULES.lock().unwrap().clone();
    let text = toml_edit::ser::to_string(&rules).unwrap_or_default();
    if let Ok(item) = text.parse::<toml_edit::Item>() {
        let mut doc = crate::CONFIG_DOC.lock().unwrap();
        if let Some(doc) = doc.as_mut() {
            doc["automation_rules"] = item;
        }
    }
    crate::persist_config();
    crate::automation::reload_rules();
    refresh_list();
}

// ===== Editor ===============================================================

fn show_editor() {
    unsafe {
        let mgr = HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _);
        if HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _).0 as usize == 0 {
            create_editor();
        }
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        populate_editor();
        theme::retheme_children(ed);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(mgr, false);
        // Go BeginFirstFrame/Reveal: cloak, commit one full frame, uncloak.
        crate::FirstFrameGate::begin(ed).reveal();
        let _ = SetForegroundWindow(ed);
        // Go focuses the name field when the editor opens.
        let name = get_dlg_item(ed, ED_NAME);
        if !name.is_invalid() {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(name));
        }
    }
}

fn hide_editor() {
    unsafe {
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        let mgr = HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _);
        let _ = ShowWindow(ed, SW_HIDE);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(mgr, true);
        refresh_list();
        // Go focuses the manager list after save/cancel (keyboard stays live).
        let list = HWND(MGR_LIST_HWND.load(Ordering::SeqCst) as *mut _);
        if !list.is_invalid() {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(list));
        }
    }
}

fn create_editor() {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module handle");
        let font = form_font_body();
        let section_font = section_font_cached();

        register_editor_class(instance);

        let style = secondary_style();
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: s(ED_W),
            bottom: s(ED_EDGE * 2 + 600),
        };
        let _ = AdjustWindowRectEx(&mut frame, style, false, WINDOW_EX_STYLE(0));
        let (x, y) = center_on_parent(frame.right - frame.left, frame.bottom - frame.top);

        let ed = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("IdleTriggerAutoEdit"),
            PCWSTR(wide(&t_pub("automation_new_title")).as_ptr()),
            style,
            x,
            y,
            frame.right - frame.left,
            frame.bottom - frame.top,
            Some(HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _)),
            None,
            Some(instance.into()),
            None,
        )
        .expect("editor window");
        EDIT_HWND.store(ed.0 as isize, Ordering::SeqCst);
        theme::apply_to_window(ed);
        crate::set_window_icons_pub(ed);

        let mk_label = |id: usize, text: &str, font: windows::Win32::Graphics::Gdi::HFONT| {
            let wide_text: Vec<u16> = text.encode_utf16().chain([0]).collect();
            let hwnd_ = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                PCWSTR(wide_text.as_ptr()),
                WINDOW_STYLE(WS_CHILD.0),
                0,
                0,
                1,
                1,
                Some(ed),
                Some(HMENU(id as *mut _)),
                Some(instance.into()),
                None,
            )
            .expect("ed label");
            let _ = SendMessageW(
                hwnd_,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        };

        let mk_edit = |id: usize, numeric: bool| {
            // Surface static behind the edit, matching settings (Go editWithStyle).
            let surface = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WINDOW_STYLE(WS_CHILD.0 | WS_CLIPSIBLINGS.0 | 13), // SS_OWNERDRAW
                0,
                0,
                1,
                1,
                Some(ed),
                Some(HMENU((FIELD_SURFACE_BASE + id as i32) as *mut _)),
                Some(instance.into()),
                None,
            )
            .expect("ed field surface");
            let _ = SendMessageW(
                surface,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            let extra = if numeric { ES_NUMBER as u32 } else { 0 };
            let hwnd_ = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("EDIT"),
                windows::core::w!(""),
                WINDOW_STYLE(
                    WS_CHILD.0 | WS_TABSTOP.0 | WS_CLIPSIBLINGS.0 | ES_AUTOHSCROLL as u32 | extra,
                ),
                0,
                0,
                1,
                1,
                Some(ed),
                Some(HMENU(id as *mut _)),
                Some(instance.into()),
                None,
            )
            .expect("ed edit");
            let _ = SendMessageW(
                hwnd_,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            // Go editWithStyle: 6px inner margins (EM_SETMargins).
            let margin = s(6) as usize;
            let _ = SendMessageW(
                hwnd_,
                EM_SETMARGINS_RAW,
                Some(WPARAM(3)),
                Some(LPARAM((margin | (margin << 16)) as isize)),
            );
            hwnd_
        };

        // Choice buttons: owner-drawn BUTTONs with custom popups (Go combo()).
        let mk_button = |id: usize, text: &str, height: i32| {
            let wide_text: Vec<u16> = text.encode_utf16().chain([0]).collect();
            let hwnd_ = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("BUTTON"),
                PCWSTR(wide_text.as_ptr()),
                WINDOW_STYLE(WS_CHILD.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
                0,
                0,
                1,
                s(height),
                Some(ed),
                Some(HMENU(id as *mut _)),
                Some(instance.into()),
                None,
            )
            .expect("ed button");
            let _ = SendMessageW(
                hwnd_,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            crate::nativeform::track(hwnd_);
            hwnd_
        };

        // Section titles + labels (Go editor.go build order).
        mk_label(ED_BASICS_TITLE, &t_pub("automation_basics"), section_font);
        mk_label(
            ED_TRIGGER_TITLE,
            &t_pub("automation_trigger_conditions"),
            section_font,
        );
        mk_label(
            ED_OPTIONS_TITLE,
            &t_pub("automation_action_options"),
            section_font,
        );
        mk_label(ED_NAME_LBL, &t_pub("automation_name"), font);
        mk_label(ED_ACTION_LBL, &t_pub("automation_action"), font);
        mk_label(ED_TRIGGER_LBL, &t_pub("automation_trigger"), font);
        mk_label(ED_TIME_LBL, &t_pub("automation_time"), font);
        mk_label(ED_DATE_LBL, &t_pub("automation_date"), font);
        mk_label(ED_END_LBL, &t_pub("automation_end_time"), font);
        mk_label(ED_LOGIC_LBL, &t_pub("automation_process_logic"), font);
        mk_label(ED_WARN_LBL, &t_pub("automation_warning_seconds"), font);
        mk_label(ED_IDLE_LBL, &t_pub("automation_idle_minutes"), font);
        mk_label(ED_MAX_LBL, &t_pub("automation_max_wait"), font);
        mk_label(ED_DAYS_LBL, &t_pub("automation_days"), font);
        mk_label(ED_BLOCKED_LBL, &t_pub("automation_blocked_policy"), font);
        mk_label(ED_NAME_HINT, &t_pub("automation_name_hint"), font);
        mk_label(ED_NO_OPTIONS, &t_pub("automation_no_action_options"), font);
        mk_label(ED_VALIDATION, &t_pub("automation_runtime_note"), font);

        // Fields (Go: date/time edits, numeric edits).
        let name_edit = mk_edit(ED_NAME, false);
        let placeholder = wide(&t_pub("automation_name_placeholder"));
        let _ = SendMessageW(
            name_edit,
            EM_SETCUEBANNER_RAW,
            Some(WPARAM(1)), // show even when focused (Go cue behavior)
            Some(LPARAM(placeholder.as_ptr() as isize)),
        );
        install_time_edit(mk_edit(ED_TIME, false));
        install_time_edit(mk_edit(ED_END_TIME, false));
        mk_edit(ED_DATE, false);
        mk_edit(ED_WARNING, true);
        mk_edit(ED_IDLE_MIN, true);
        mk_edit(ED_MAX_WAIT, true);

        // Choice fields (Go combo: owner-draw BUTTON + popup).
        let _ = crate::choice::create_rows(
            ed,
            ED_ACTION as i32,
            (0, 0, 1, ED_FIELD_H),
            &[ChoiceItem::option("", "")],
            font,
        );
        let _ = crate::choice::create_rows(
            ed,
            ED_TRIGGER as i32,
            (0, 0, 1, ED_FIELD_H),
            &[ChoiceItem::option("", "")],
            font,
        );
        let _ = crate::choice::create_rows(
            ed,
            ED_LOGIC as i32,
            (0, 0, 1, ED_FIELD_H),
            &[ChoiceItem::option("", "")],
            font,
        );
        let _ = crate::choice::create_rows(
            ed,
            ED_BLOCKED as i32,
            (0, 0, 1, ED_FIELD_H),
            &[ChoiceItem::option("", "")],
            font,
        );

        mk_button(ED_CHOOSE, &t_pub("automation_choose_processes"), ED_FIELD_H);
        mk_button(ED_DAYS_WORKDAYS, &t_pub("automation_days_workdays"), 24);
        mk_button(ED_DAYS_EVERYDAY, &t_pub("automation_days_everyday"), 24);
        mk_button(ED_KEEP_SCREEN, &t_pub("automation_keep_screen"), ED_CHECK_H);
        mk_button(ED_PROC_INFO, "i", ED_SUMMARY_H);

        for (id, key) in [
            (ED_DAYS_MON, "automation_day_mon"),
            (ED_DAYS_TUE, "automation_day_tue"),
            (ED_DAYS_WED, "automation_day_wed"),
            (ED_DAYS_THU, "automation_day_thu"),
            (ED_DAYS_FRI, "automation_day_fri"),
            (ED_DAYS_SAT, "automation_day_sat"),
            (ED_DAYS_SUN, "automation_day_sun"),
        ] {
            mk_button(id, &t_pub(key), ED_FIELD_H);
        }

        // Footer: Save (accent) + Cancel, owner-drawn.
        mk_button(ED_SAVE, &t_pub("automation_save"), BUTTON_H);
        mk_button(ED_CANCEL, &t_pub("automation_cancel"), BUTTON_H);
    }
}

unsafe fn register_editor_class(instance: windows::Win32::Foundation::HMODULE) {
    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(ed_proc),
            hInstance: instance.into(),
            lpszClassName: windows::core::w!("IdleTriggerAutoEdit"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1usize as *const u16))
                .unwrap_or_default(),
            hbrBackground: windows::Win32::Graphics::Gdi::GetSysColorBrush(
                windows::Win32::Graphics::Gdi::COLOR_WINDOW,
            ),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
}

/// Go defaultRule(): stay_awake + process_running + sensible times.
fn default_rule() -> auto::Rule {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let date = format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay);
    let next_hour = format!("{:02}:{:02}", (st.wHour as i32 + 1) % 24, st.wMinute);
    let two_hours = format!("{:02}:{:02}", (st.wHour as i32 + 2) % 24, st.wMinute);
    auto::Rule {
        id: format!("rule-{now:x}"),
        name: String::new(),
        enabled: true,
        action: "stay_awake".into(),
        trigger: "process_running".into(),
        time: next_hour,
        end_time: two_hours,
        date,
        days: vec![
            "mon".into(),
            "tue".into(),
            "wed".into(),
            "thu".into(),
            "fri".into(),
        ],
        process_logic: "any".into(),
        processes: Vec::new(),
        keep_screen_on: false,
        idle_minutes: auto::DEFAULT_IDLE_MINUTES,
        warning_seconds: auto::DEFAULT_WARNING_SECONDS,
        blocked_policy: "skip".into(),
        max_wait_minutes: 60,
    }
}

fn populate_editor() {
    unsafe {
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        let idx = EDIT_INDEX.load(Ordering::SeqCst);
        let rules = crate::automation::RULES.lock().unwrap();
        let rule = if idx >= 0 && (idx as usize) < rules.len() {
            Some(rules[idx as usize].clone())
        } else {
            None
        };
        drop(rules);
        let rule = rule.unwrap_or_else(default_rule);
        *EDIT_ORIG.lock().unwrap() = Some(rule.clone());

        // Title follows new/edit mode (Go setCaption).
        let title = if idx >= 0 {
            t_pub("automation_edit_title")
        } else {
            t_pub("automation_new_title")
        };
        let _ = SetWindowTextW(ed, PCWSTR(wide(&title).as_ptr()));

        set_text(get_dlg_item(ed, ED_NAME), &rule.name);
        set_text(get_dlg_item(ed, ED_DATE), &rule.date);
        set_text(get_dlg_item(ed, ED_TIME), &rule.time);
        set_text(get_dlg_item(ed, ED_END_TIME), &rule.end_time);
        set_text(
            get_dlg_item(ed, ED_IDLE_MIN),
            &rule.idle_minutes.to_string(),
        );
        set_text(
            get_dlg_item(ed, ED_WARNING),
            &rule.warning_seconds.to_string(),
        );
        set_text(
            get_dlg_item(ed, ED_MAX_WAIT),
            &rule.max_wait_minutes.to_string(),
        );

        // Validation row: runtime note, or the rule's issue (Go issueForRule).
        let issues = crate::automation::ISSUES.lock().unwrap().clone();
        let issue = issues
            .iter()
            .find(|i| i.index as i32 == idx || (!i.rule_id.is_empty() && i.rule_id == rule.id));
        EDIT_ERROR.store(issue.is_some(), Ordering::SeqCst);
        set_text(
            get_dlg_item(ed, ED_VALIDATION),
            &match &issue {
                Some(i) => i.message.clone(),
                None => t_pub("automation_runtime_note"),
            },
        );

        // Day checkboxes reflect rule.days; keepScreen reflects the draft.
        let day_ids = [
            (ED_DAYS_SUN, "sun"),
            (ED_DAYS_MON, "mon"),
            (ED_DAYS_TUE, "tue"),
            (ED_DAYS_WED, "wed"),
            (ED_DAYS_THU, "thu"),
            (ED_DAYS_FRI, "fri"),
            (ED_DAYS_SAT, "sat"),
        ];
        for (id, key) in day_ids {
            let on = rule.days.iter().any(|d| d == key);
            edit_set_checked(ed, id, on);
        }
        edit_set_checked(ed, ED_KEEP_SCREEN, rule.keep_screen_on);

        fill_action_choice(ed);
        choice_select(ed, ED_ACTION, &rule.action);
        fill_trigger_choice(ed, &rule.action, &rule.trigger);
        fill_logic_choice(ed);
        choice_select(ed, ED_LOGIC, &rule.process_logic);
        fill_blocked_choice(ed);
        choice_select(ed, ED_BLOCKED, &rule.blocked_policy);

        update_proc_summary();
        layout_editor();
    }
}

/// Reads the editor controls into a Rule (Go syncDraft).
fn read_draft(ed: HWND, base: &auto::Rule) -> auto::Rule {
    let mut draft = base.clone();
    draft.name = clean_single_line(&get_text(get_dlg_item(ed, ED_NAME)));
    draft.action = choice_value(ed, ED_ACTION);
    draft.trigger = choice_value(ed, ED_TRIGGER);
    draft.process_logic = choice_value(ed, ED_LOGIC);
    draft.blocked_policy = choice_value(ed, ED_BLOCKED);
    draft.date = clean_single_line(&get_text(get_dlg_item(ed, ED_DATE)));
    draft.time = clean_single_line(&get_text(get_dlg_item(ed, ED_TIME)));
    draft.end_time = clean_single_line(&get_text(get_dlg_item(ed, ED_END_TIME)));
    let day_ids = [
        (ED_DAYS_MON, "mon"),
        (ED_DAYS_TUE, "tue"),
        (ED_DAYS_WED, "wed"),
        (ED_DAYS_THU, "thu"),
        (ED_DAYS_FRI, "fri"),
        (ED_DAYS_SAT, "sat"),
        (ED_DAYS_SUN, "sun"),
    ];
    draft.days = day_ids
        .iter()
        .filter(|(id, _)| edit_is_checked(*id))
        .map(|(_, key)| key.to_string())
        .collect();
    draft.keep_screen_on = edit_is_checked(ED_KEEP_SCREEN);
    draft.processes = EDIT_PROCS.lock().unwrap().clone();
    draft.idle_minutes = get_text(get_dlg_item(ed, ED_IDLE_MIN))
        .trim()
        .parse()
        .unwrap_or(0);
    draft.warning_seconds = get_text(get_dlg_item(ed, ED_WARNING))
        .trim()
        .parse()
        .unwrap_or(0);
    draft.max_wait_minutes = get_text(get_dlg_item(ed, ED_MAX_WAIT))
        .trim()
        .parse()
        .unwrap_or(0);
    draft
}

/// Go validateDraft: returns the first error message (with its control).
fn validate_draft(ed: HWND, draft: &auto::Rule) -> Option<(usize, String)> {
    let t = |key: &str| t_pub(key);
    let trigger = draft.trigger.as_str();
    let needs_process = matches!(
        trigger,
        "process_running" | "process_started" | "process_exited"
    );
    if needs_process && draft.processes.is_empty() {
        return Some((ED_CHOOSE, t("automation_error_process_required")));
    }
    if trigger == "once" && !parse_date(&draft.date) {
        return Some((ED_DATE, t("automation_error_date")));
    }
    if matches!(trigger, "once" | "daily" | "weekly" | "time_window") && !parse_time(&draft.time) {
        return Some((ED_TIME, t("automation_error_time")));
    }
    if trigger == "time_window" && !parse_time(&draft.end_time) {
        return Some((ED_END_TIME, t("automation_error_end_time")));
    }
    if matches!(trigger, "weekly" | "time_window") && draft.days.is_empty() {
        return Some((ED_DAYS_MON, t("automation_error_days")));
    }
    if draft.action == "enable_idle_monitor"
        && (draft.idle_minutes <= 0 || draft.idle_minutes > 7 * 24 * 60)
    {
        return Some((ED_IDLE_MIN, t("automation_error_idle_minutes")));
    }
    if auto::is_event_action(&draft.action)
        && (draft.warning_seconds < auto::MIN_WARNING_SECONDS || draft.warning_seconds > 3600)
    {
        return Some((ED_WARNING, t("automation_error_warning")));
    }
    if auto::is_event_action(&draft.action)
        && !draft.processes.is_empty()
        && trigger != "process_started"
        && trigger != "process_exited"
        && draft.blocked_policy == "wait"
        && (draft.max_wait_minutes <= 0 || draft.max_wait_minutes > 7 * 24 * 60)
    {
        return Some((ED_MAX_WAIT, t("automation_error_max_wait")));
    }
    let _ = ed;
    None
}

fn parse_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    fn num(bytes: &[u8], mut range: std::ops::Range<usize>) -> bool {
        range.all(|i| bytes[i].is_ascii_digit())
    }
    let (y, m, d) = (
        value[..4].parse::<u32>().unwrap_or(0),
        value[5..7].parse::<u32>().unwrap_or(0),
        value[8..10].parse::<u32>().unwrap_or(0),
    );
    if !(num(bytes, 0..4) && num(bytes, 5..7) && num(bytes, 8..10)) || y == 0 {
        return false;
    }
    // Go time.Parse rejects impossible dates (e.g. 2026-02-31).
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let max_day = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    (1..=12).contains(&m) && (1..=max_day).contains(&d) && y > 0
}

fn parse_time(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return false;
    }
    let all_digits =
        (0..2).all(|i| bytes[i].is_ascii_digit()) && (3..5).all(|i| bytes[i].is_ascii_digit());
    let (h, m) = (
        value[..2].parse::<u32>().unwrap_or(99),
        value[3..5].parse::<u32>().unwrap_or(99),
    );
    all_digits && h < 24 && m < 60
}

/// Go setEditorError: show the message in the validation row and focus the field.
fn set_editor_error(ed: HWND, id: usize, message: &str) {
    EDIT_ERROR.store(true, Ordering::SeqCst);
    set_text(get_dlg_item(ed, ED_VALIDATION), message);
    let control = get_dlg_item(ed, id);
    if !control.is_invalid() {
        unsafe {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(control));
        }
    }
}

fn clear_editor_error(ed: HWND) {
    if !EDIT_ERROR.load(Ordering::SeqCst) {
        return;
    }
    EDIT_ERROR.store(false, Ordering::SeqCst);
    set_text(
        get_dlg_item(ed, ED_VALIDATION),
        &t_pub("automation_runtime_note"),
    );
}

/// Go saveEditor: validate, default the name, persist, close on success.
fn save_rule(ed: HWND) {
    let orig = EDIT_ORIG
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(default_rule);
    let draft = read_draft(ed, &orig);

    if let Some((id, message)) = validate_draft(ed, &draft) {
        set_editor_error(ed, id, &message);
        return;
    }

    let mut draft = draft;
    if draft.name.is_empty() {
        draft.name = rule_summary(&draft);
    }

    let idx = EDIT_INDEX.load(Ordering::SeqCst);
    let mut candidate = crate::automation::RULES.lock().unwrap().clone();
    if idx >= 0 && (idx as usize) < candidate.len() {
        candidate[idx as usize] = draft.clone();
    } else {
        candidate.push(draft.clone());
    }
    let (_, issues) = auto::prepare_rules(&candidate);
    if !issues.is_empty() {
        set_editor_error(ed, ED_SAVE, &issues[0].message);
        return;
    }

    // Go saves PrepareRules' normalized output, not the raw candidate.
    let (normalized, _) = auto::prepare_rules(&candidate);
    *crate::automation::RULES.lock().unwrap() = normalized;
    save_rules_to_config();
    hide_editor();
}

/// Go cancelEditor: confirm when unsaved work would be lost.
fn cancel_editor(ed: HWND) {
    let orig = EDIT_ORIG.lock().unwrap().clone();
    if let Some(orig) = orig {
        let current = read_draft(ed, &orig);
        if editor_needs_discard_confirm(&current, &orig) {
            let title = t_pub("automation_discard_title");
            let body = t_pub("automation_discard_confirm");
            if !confirm_dialog(ed, &title, &body) {
                return;
            }
        }
    }
    hide_editor();
}

/// Go editorChangesRequireConfirmation: an edit always confirms when
/// changed; a NEW rule confirms only when the newRuleIntent projection
/// (identity/action/trigger/schedule/days/process targets) differs, so
/// tweaking runtime-only options never nags.
fn editor_needs_discard_confirm(current: &auto::Rule, orig: &auto::Rule) -> bool {
    if current == orig {
        return false;
    }
    if EDIT_INDEX.load(Ordering::SeqCst) >= 0 {
        return true;
    }
    let intent = |r: &auto::Rule| {
        (
            r.name.clone(),
            r.action.clone(),
            r.trigger.clone(),
            r.time.clone(),
            r.end_time.clone(),
            r.date.clone(),
            r.days.clone(),
            r.process_logic.clone(),
            r.processes.clone(),
        )
    };
    intent(current) != intent(orig)
}

/// Repositions and shows/hides editor controls per the current trigger and
/// action selections, then resizes the window (Go layoutEditorContent).
pub fn layout_editor() {
    unsafe {
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        if ed.is_invalid() {
            return;
        }
        let content_w = ED_W - 2 * ED_PAD;
        let column_w = (content_w - ED_GAP) / 2;
        let trigger = choice_value(ed, ED_TRIGGER);
        let action = choice_value(ed, ED_ACTION);

        // Everything starts hidden; each section places what it needs.
        for id in ED_LAYOUT_IDS {
            let _ = ShowWindow(get_dlg_item(ed, id), SW_HIDE);
            let surface = field_surface_of(id);
            if let Some(surface) = surface {
                let _ = ShowWindow(get_dlg_item(ed, surface), SW_HIDE);
            }
        }

        let mut place = |id: usize, x: i32, y: i32, w: i32, h: i32| {
            let control = get_dlg_item(ed, id);
            if control.is_invalid() {
                return;
            }
            // Field edits sit inset 2px inside their surface (Go place()).
            if let Some(surface_id) = field_surface_of(id) {
                let surface = get_dlg_item(ed, surface_id);
                if !surface.is_invalid() {
                    let _ = SetWindowPos(
                        surface,
                        None,
                        s(x),
                        s(y),
                        s(w),
                        s(h),
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    let _ = ShowWindow(surface, SW_SHOW);
                    let inner_h = (h - 4).min(20);
                    let _ = SetWindowPos(
                        control,
                        None,
                        s(x + 2),
                        s(y + (h - inner_h) / 2),
                        s(w - 4),
                        s(inner_h),
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            } else {
                let _ = SetWindowPos(
                    control,
                    None,
                    s(x),
                    s(y),
                    s(w),
                    s(h),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            let _ = ShowWindow(control, SW_SHOW);
        };

        let label_row2 = |place: &mut dyn FnMut(usize, i32, i32, i32, i32),
                          left: usize,
                          right: usize,
                          y: i32| {
            place(left, ED_PAD, y, column_w, ED_LABEL_H);
            place(right, ED_PAD + column_w + ED_GAP, y, column_w, ED_LABEL_H);
        };

        // 基本设置.
        let mut y = ED_EDGE;
        place(ED_BASICS_TITLE, ED_PAD, y, content_w, ED_LABEL_H);
        y += ED_LABEL_H + ED_CONTENT_GAP;
        place(ED_NAME_LBL, ED_PAD, y, content_w, ED_LABEL_H);
        y += ED_LABEL_H + ED_LABEL_GAP;
        place(ED_NAME, ED_PAD, y, content_w, ED_FIELD_H);
        y += ED_FIELD_H + ED_LABEL_GAP;
        place(ED_NAME_HINT, ED_PAD, y, content_w, ED_LABEL_H);
        y += ED_LABEL_H + ED_SECTION_GAP;
        label_row2(&mut place, ED_ACTION_LBL, ED_TRIGGER_LBL, y);
        y += ED_LABEL_H + ED_LABEL_GAP;
        place(ED_ACTION, ED_PAD, y, column_w, ED_FIELD_H);
        place(
            ED_TRIGGER,
            ED_PAD + column_w + ED_GAP,
            y,
            column_w,
            ED_FIELD_H,
        );
        y += ED_FIELD_H + ED_SECTION_GAP;

        // 触发条件.
        place(ED_TRIGGER_TITLE, ED_PAD, y, content_w, ED_LABEL_H);
        y += ED_LABEL_H + ED_CONTENT_GAP;
        set_text(get_dlg_item(ed, ED_TIME_LBL), &time_label_text(&trigger));

        let row_fields = |place: &mut dyn FnMut(usize, i32, i32, i32, i32),
                          llbl: usize,
                          lctl: usize,
                          rlbl: usize,
                          rctl: usize,
                          y: &mut i32| {
            label_row2(place, llbl, rlbl, *y);
            *y += ED_LABEL_H + ED_LABEL_GAP;
            place(lctl, ED_PAD, *y, column_w, ED_FIELD_H);
            place(rctl, ED_PAD + column_w + ED_GAP, *y, column_w, ED_FIELD_H);
            *y += ED_FIELD_H + ED_RELATED_GAP;
        };

        match trigger.as_str() {
            "once" => row_fields(
                &mut place,
                ED_DATE_LBL,
                ED_DATE,
                ED_TIME_LBL,
                ED_TIME,
                &mut y,
            ),
            "daily" => {
                place(ED_TIME_LBL, ED_PAD, y, column_w, ED_LABEL_H);
                y += ED_LABEL_H + ED_LABEL_GAP;
                place(ED_TIME, ED_PAD, y, column_w, ED_FIELD_H);
                y += ED_FIELD_H + ED_RELATED_GAP;
            }
            "weekly" => {
                place(ED_TIME_LBL, ED_PAD, y, column_w, ED_LABEL_H);
                y += ED_LABEL_H + ED_LABEL_GAP;
                place(ED_TIME, ED_PAD, y, column_w, ED_FIELD_H);
                y += ED_FIELD_H + ED_RELATED_GAP;
                y = layout_weekdays(&mut place, y, content_w);
            }
            "time_window" => {
                row_fields(
                    &mut place,
                    ED_TIME_LBL,
                    ED_TIME,
                    ED_END_LBL,
                    ED_END_TIME,
                    &mut y,
                );
                y = layout_weekdays(&mut place, y, content_w);
            }
            _ => {}
        }

        // Process condition row.
        let process_required = matches!(
            trigger.as_str(),
            "process_running" | "process_started" | "process_exited"
        );
        let logic_key = if process_required {
            "automation_process_condition"
        } else {
            "automation_optional_process"
        };
        set_text(get_dlg_item(ed, ED_LOGIC_LBL), &t_pub(logic_key));
        place(ED_LOGIC_LBL, ED_PAD, y, content_w, ED_LABEL_H);
        y += ED_LABEL_H + ED_LABEL_GAP;
        match trigger.as_str() {
            "process_exited" => {
                choice_select(ed, ED_LOGIC, "none");
                place(ED_CHOOSE, ED_PAD, y, content_w, ED_FIELD_H);
            }
            "process_started" => {
                choice_select(ed, ED_LOGIC, "any");
                place(ED_CHOOSE, ED_PAD, y, content_w, ED_FIELD_H);
            }
            _ => {
                place(ED_LOGIC, ED_PAD, y, column_w, ED_FIELD_H);
                place(
                    ED_CHOOSE,
                    ED_PAD + column_w + ED_GAP,
                    y,
                    column_w,
                    ED_FIELD_H,
                );
            }
        }
        y += ED_FIELD_H + ED_RELATED_GAP;

        // Summary row: text shrinks by 30 when the info glyph is visible.
        let has_procs = !EDIT_PROCS.lock().unwrap().is_empty();
        update_proc_summary();
        if has_procs {
            place(ED_PROC_SUMMARY, ED_PAD, y, content_w - 30, ED_SUMMARY_H);
            place(
                ED_PROC_INFO,
                ED_PAD + content_w - ED_SUMMARY_H,
                y,
                ED_SUMMARY_H,
                ED_SUMMARY_H,
            );
        } else {
            place(ED_PROC_SUMMARY, ED_PAD, y, content_w, ED_SUMMARY_H);
        }
        y += ED_SUMMARY_H + ED_CONTENT_GAP;

        // 执行选项.
        place(ED_OPTIONS_TITLE, ED_PAD, y, content_w, ED_LABEL_H);
        y += ED_LABEL_H + ED_CONTENT_GAP;
        match action.as_str() {
            "stay_awake" => {
                // Only as wide as the checkbox + label (Go CheckboxHitWidth).
                let label = window_text(get_dlg_item(ed, ED_KEEP_SCREEN));
                let check_w = checkbox_hit_width(ed, &label).unwrap_or(content_w);
                place(
                    ED_KEEP_SCREEN,
                    ED_PAD,
                    y,
                    check_w.min(content_w),
                    ED_CHECK_H,
                );
                y += ED_CHECK_H;
            }
            "enable_idle_monitor" => {
                place(ED_IDLE_LBL, ED_PAD, y, column_w, ED_LABEL_H);
                y += ED_LABEL_H + ED_LABEL_GAP;
                place(ED_IDLE_MIN, ED_PAD, y, column_w, ED_FIELD_H);
                y += ED_FIELD_H;
            }
            "pause_stay_awake" | "pause_idle_monitor" => {
                place(ED_NO_OPTIONS, ED_PAD, y, content_w, ED_LABEL_H);
                y += ED_LABEL_H;
            }
            _ => {
                place(ED_WARN_LBL, ED_PAD, y, column_w, ED_LABEL_H);
                y += ED_LABEL_H + ED_LABEL_GAP;
                place(ED_WARNING, ED_PAD, y, column_w, ED_FIELD_H);
                y += ED_FIELD_H;
                if has_procs && trigger != "process_started" && trigger != "process_exited" {
                    y += ED_RELATED_GAP;
                    place(ED_BLOCKED_LBL, ED_PAD, y, column_w, ED_LABEL_H);
                    let waiting = choice_value(ed, ED_BLOCKED) == "wait";
                    if waiting {
                        place(
                            ED_MAX_LBL,
                            ED_PAD + column_w + ED_GAP,
                            y,
                            column_w,
                            ED_LABEL_H,
                        );
                    }
                    y += ED_LABEL_H + ED_LABEL_GAP;
                    place(ED_BLOCKED, ED_PAD, y, column_w, ED_FIELD_H);
                    if waiting {
                        place(
                            ED_MAX_WAIT,
                            ED_PAD + column_w + ED_GAP,
                            y,
                            column_w,
                            ED_FIELD_H,
                        );
                    }
                    y += ED_FIELD_H;
                }
            }
        }

        // Status row + footer (stable bounds in both states).
        y += ED_RELATED_GAP;
        place(ED_VALIDATION, ED_PAD, y, content_w, ED_LABEL_H);
        y += ED_LABEL_H + ED_SECTION_GAP;
        place(
            ED_SAVE,
            ED_PAD + content_w - ED_DIALOG_W,
            y,
            ED_DIALOG_W,
            BUTTON_H,
        );
        place(
            ED_CANCEL,
            ED_PAD + content_w - 2 * ED_DIALOG_W - ED_GAP,
            y,
            ED_DIALOG_W,
            BUTTON_H,
        );
        y += BUTTON_H + ED_EDGE;

        // Resize the window to the computed client height, keeping the
        // top-left corner (Go resize).
        let style = secondary_style();
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: s(ED_W),
            bottom: s(y),
        };
        let _ = AdjustWindowRectEx(&mut frame, style, false, WINDOW_EX_STYLE(0));
        let mut wr = RECT::default();
        let _ = GetWindowRect(ed, &mut wr);
        let _ = SetWindowPos(
            ed,
            None,
            wr.left,
            wr.top,
            frame.right - frame.left,
            frame.bottom - frame.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        // Native EDIT fields can keep a validated region after the hide/
        // place/show cycle; present each visible one (Go PresentFrame over
        // frameControls at the end of a layout pass).
        for id in ED_LAYOUT_IDS {
            let control = get_dlg_item(ed, id);
            if control.is_invalid() || !IsWindowVisible(control).as_bool() {
                continue;
            }
            present_control(control);
            if let Some(surface_id) = field_surface_of(id) {
                let surface = get_dlg_item(ed, surface_id);
                if !surface.is_invalid() && IsWindowVisible(surface).as_bool() {
                    present_control(surface);
                }
            }
        }
    }
}

/// Weekday row: label + 工作日/每天 quick buttons, then 7 equal buttons
/// (Go layoutWeekdays).
fn layout_weekdays(
    place: &mut dyn FnMut(usize, i32, i32, i32, i32),
    mut y: i32,
    content_w: i32,
) -> i32 {
    const QUICK_W: i32 = 88;
    const QUICK_H: i32 = 24;
    let gap = ED_GAP;
    place(
        ED_DAYS_LBL,
        ED_PAD,
        y + (QUICK_H - ED_LABEL_H) / 2,
        content_w - 2 * (QUICK_W + gap),
        ED_LABEL_H,
    );
    place(
        ED_DAYS_WORKDAYS,
        ED_PAD + content_w - 2 * QUICK_W - gap,
        y,
        QUICK_W,
        QUICK_H,
    );
    place(
        ED_DAYS_EVERYDAY,
        ED_PAD + content_w - QUICK_W,
        y,
        QUICK_W,
        QUICK_H,
    );
    y += QUICK_H + ED_RELATED_GAP;
    let days = [
        ED_DAYS_MON,
        ED_DAYS_TUE,
        ED_DAYS_WED,
        ED_DAYS_THU,
        ED_DAYS_FRI,
        ED_DAYS_SAT,
        ED_DAYS_SUN,
    ];
    let button_w = (content_w - gap * (days.len() as i32 - 1)) / days.len() as i32;
    for (index, id) in days.iter().enumerate() {
        place(
            *id,
            ED_PAD + index as i32 * (button_w + gap),
            y,
            button_w,
            ED_FIELD_H,
        );
    }
    y + ED_FIELD_H + ED_RELATED_GAP
}

/// Time label text per trigger (Go automationTimeLabelKey).
fn time_label_text(trigger: &str) -> String {
    let key = if trigger == "time_window" {
        "automation_time"
    } else {
        "automation_execution_time"
    };
    t_pub(key)
}

/// Refreshes the selected-processes summary line (Go idProcessSummary).
fn update_proc_summary() {
    let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
    let count = EDIT_PROCS.lock().unwrap().len();
    let text = if count == 0 {
        t_pub("automation_no_processes")
    } else {
        t_pub("automation_process_count").replace("%d", &count.to_string())
    };
    set_text(get_dlg_item(ed, ED_PROC_SUMMARY), &text);
}

/// Checks exactly the given weekday keys (Go quick buttons).
fn set_weekdays(ed: HWND, keys: &[&str]) {
    let day_ids = [
        (ED_DAYS_SUN, "sun"),
        (ED_DAYS_MON, "mon"),
        (ED_DAYS_TUE, "tue"),
        (ED_DAYS_WED, "wed"),
        (ED_DAYS_THU, "thu"),
        (ED_DAYS_FRI, "fri"),
        (ED_DAYS_SAT, "sat"),
    ];
    for (id, key) in day_ids {
        let on = keys.contains(&key);
        edit_set_checked(ed, id, on);
    }
}

/// Checkbox + label width in logical pixels (Go CheckboxHitWidth).
fn checkbox_hit_width(ed: HWND, label: &str) -> Option<i32> {
    unsafe {
        let font = SendMessageW(
            get_dlg_item(ed, ED_KEEP_SCREEN),
            WM_GETFONT,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        )
        .0;
        let hdc = windows::Win32::Graphics::Gdi::GetDC(Some(ed));
        let old = windows::Win32::Graphics::Gdi::SelectObject(
            hdc,
            windows::Win32::Graphics::Gdi::HGDIOBJ(font as *mut _),
        );
        let mut size = windows::Win32::Foundation::SIZE::default();
        let wide_label = label.encode_utf16().collect::<Vec<u16>>();
        let ok = windows::Win32::Graphics::Gdi::GetTextExtentPoint32W(hdc, &wide_label, &mut size)
            .as_bool();
        windows::Win32::Graphics::Gdi::SelectObject(hdc, old);
        windows::Win32::Graphics::Gdi::ReleaseDC(Some(ed), hdc);
        if !ok {
            return None;
        }
        let scale96 = crate::scale_pub(96).max(96) as f32 / 96.0;
        let width = size.cx as f32 / scale96.max(1.0);
        Some(2 + ED_CHECKBOX_SIZE + 8 + width.ceil() as i32 + 2)
    }
}

/// Maps an edit id to its field-surface id (Go idFieldSurfaceBase scheme).
fn field_surface_of(edit_id: usize) -> Option<usize> {
    if matches!(
        edit_id,
        ED_NAME | ED_TIME | ED_END_TIME | ED_DATE | ED_WARNING | ED_IDLE_MIN | ED_MAX_WAIT
    ) {
        Some((FIELD_SURFACE_BASE + edit_id as i32) as usize)
    } else {
        None
    }
}

/// Process details text for the info glyph (Go processDetails).
fn process_details() -> String {
    let targets = EDIT_PROCS.lock().unwrap().clone();
    let mut lines = Vec::new();
    for target in &targets {
        let mut name = target.executable.clone();
        if !target.path.is_empty() && target.kind == "path" {
            lines.push(fill_template(
                &t_pub("automation_process_detail_path"),
                &[&name, &target.path],
            ));
            continue;
        }
        let _ = &mut name;
        lines.push(fill_template(
            &t_pub("automation_process_detail_name"),
            &[&target.executable],
        ));
    }
    lines.join("\n")
}

unsafe extern "system" fn ed_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                let code = wparam.0 & 0xFFFF;
                let hi = ((wparam.0 >> 16) & 0xFFFF) as u16;
                match code {
                    ED_SAVE if hi == BN_CLICKED => save_rule(hwnd),
                    ED_CANCEL if hi == BN_CLICKED => cancel_editor(hwnd),
                    ED_CHOOSE if hi == BN_CLICKED => show_picker(hwnd),

                    // Checkbox toggles (owner-draw; state in EDIT_CHECKS).
                    ED_KEEP_SCREEN if hi == BN_CLICKED => {
                        edit_toggle(hwnd, ED_KEEP_SCREEN);
                        clear_editor_error(hwnd);
                    }
                    ED_DAYS_MON | ED_DAYS_TUE | ED_DAYS_WED | ED_DAYS_THU | ED_DAYS_FRI
                    | ED_DAYS_SAT | ED_DAYS_SUN
                        if hi == BN_CLICKED =>
                    {
                        edit_toggle(hwnd, code);
                        clear_editor_error(hwnd);
                    }
                    ED_DAYS_WORKDAYS if hi == BN_CLICKED => {
                        set_weekdays(hwnd, &["mon", "tue", "wed", "thu", "fri"]);
                        clear_editor_error(hwnd);
                    }
                    ED_DAYS_EVERYDAY if hi == BN_CLICKED => {
                        set_weekdays(hwnd, &["sun", "mon", "tue", "wed", "thu", "fri", "sat"]);
                        clear_editor_error(hwnd);
                    }
                    ED_PROC_INFO if hi == BN_CLICKED => {
                        if !EDIT_PROCS.lock().unwrap().is_empty() {
                            info_dialog(
                                hwnd,
                                &t_pub("automation_process_details_title"),
                                &process_details(),
                            );
                        }
                    }

                    // Numeric edits: filter digits and clear errors (Go
                    // sanitizeNumericEdit).
                    ED_WARNING | ED_IDLE_MIN | ED_MAX_WAIT if hi == EN_CHANGE => {
                        sanitize_numeric_edit(hwnd, code);
                        clear_editor_error(hwnd);
                    }
                    ED_NAME | ED_DATE | ED_TIME | ED_END_TIME if hi == EN_CHANGE => {
                        clear_editor_error(hwnd);
                    }

                    // Field focus repaints the surface border (Go surfaces).
                    code if (hi == EN_SETFOCUS || hi == EN_KILLFOCUS)
                        && field_surface_of(code).is_some() =>
                    {
                        if let Some(surface) = field_surface_of(code) {
                            let control = get_dlg_item(hwnd, surface);
                            if !control.is_invalid() {
                                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(
                                    Some(control),
                                    None,
                                    true,
                                );
                            }
                        }
                    }

                    // Choice selections drive the dynamic Go layout; the
                    // trigger list itself depends on the selected action.
                    ED_ACTION | ED_TRIGGER | ED_LOGIC | ED_BLOCKED if hi == CBN_SELCHANGE => {
                        if code == ED_ACTION {
                            // Keep the current trigger when still valid for the
                            // new action (Go setTriggerOptions desired).
                            let previous = choice_value(hwnd, ED_TRIGGER);
                            let action = choice_value(hwnd, ED_ACTION);
                            fill_trigger_choice(hwnd, &action, &previous);
                        }
                        layout_editor();
                        clear_editor_error(hwnd);
                    }

                    // Choice buttons open their popup on click.
                    ED_ACTION | ED_TRIGGER | ED_LOGIC | ED_BLOCKED if hi == BN_CLICKED => {
                        crate::choice::toggle(get_dlg_item(hwnd, code), hwnd, code as i32);
                    }

                    _ => {}
                }
                LRESULT(0)
            }
            WM_DRAWITEM => {
                if let Some(item) = crate::nativeform::draw_item(lparam) {
                    draw_form_item(&item);
                    LRESULT(1)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_CLOSE => {
                cancel_editor(hwnd);
                LRESULT(0)
            }
            WM_ERASEBKGND => {
                crate::popups::erase_theme_bg(hwnd, wparam);
                LRESULT(1)
            }
            WM_CTLCOLORSTATIC => {
                // Go 3-tier labels: field labels SecondaryText, hints muted,
                // titles PrimaryText, validation muted or danger-on-error.
                let palette = theme::palette();
                let id = GetWindowLongPtrW(HWND(lparam.0 as *mut _), GWL_ID) as usize;
                let secondary = matches!(
                    id,
                    ED_NAME_LBL
                        | ED_ACTION_LBL
                        | ED_TRIGGER_LBL
                        | ED_DATE_LBL
                        | ED_TIME_LBL
                        | ED_END_LBL
                        | ED_DAYS_LBL
                        | ED_LOGIC_LBL
                        | ED_IDLE_LBL
                        | ED_WARN_LBL
                        | ED_BLOCKED_LBL
                        | ED_MAX_LBL
                        | ED_PROC_SUMMARY
                );
                let muted = matches!(id, ED_NAME_HINT | ED_NO_OPTIONS);
                let color = if id == ED_VALIDATION {
                    if EDIT_ERROR.load(Ordering::SeqCst) {
                        if theme::is_dark() {
                            palette.danger_border
                        } else {
                            palette.danger_bg
                        }
                    } else {
                        palette.muted
                    }
                } else if secondary {
                    palette.text2
                } else if muted {
                    palette.muted
                } else {
                    palette.text
                };
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                let _ = windows::Win32::Graphics::Gdi::SetTextColor(hdc, COLORREF(color));
                let _ = windows::Win32::Graphics::Gdi::SetBkColor(hdc, COLORREF(theme::bg_color()));
                LRESULT(theme::bg_brush().0 as isize)
            }
            WM_CTLCOLOREDIT => {
                // Edit interior: PrimaryText on Surface (Go surfaces).
                let p = theme::palette();
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                let control = HWND(lparam.0 as *mut _);
                let disabled = !IsWindowEnabled(control).as_bool();
                if disabled {
                    let _ =
                        windows::Win32::Graphics::Gdi::SetTextColor(hdc, COLORREF(p.disabled_text));
                    let _ = windows::Win32::Graphics::Gdi::SetBkColor(
                        hdc,
                        COLORREF(p.disabled_surface),
                    );
                } else {
                    let _ = windows::Win32::Graphics::Gdi::SetTextColor(hdc, COLORREF(p.text));
                    let _ = windows::Win32::Graphics::Gdi::SetBkColor(hdc, COLORREF(p.surface));
                }
                let (light, dark) = theme::surface_brush_pairs();
                let pair = if theme::is_dark() { dark } else { light };
                LRESULT(pair.0.0 as isize)
            }
            WM_DESTROY => LRESULT(0),
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Strips non-digits from a numeric edit and restores the caret (Go).
fn sanitize_numeric_edit(ed: HWND, id: usize) {
    let control = get_dlg_item(ed, id);
    if control.is_invalid() {
        return;
    }
    let value = get_text(control);
    let filtered: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
    if filtered == value {
        return;
    }
    set_text(control, &filtered);
    unsafe {
        let _ = SendMessageW(
            control,
            EM_SETSEL_RAW,
            Some(WPARAM(usize::MAX)),
            Some(LPARAM(usize::MAX as isize)),
        );
    }
}

// ===== Shared owner-draw ====================================================

/// Shared WM_DRAWITEM painter for the editor and picker forms.
fn draw_form_item(item: &crate::nativeform::DrawItem) {
    unsafe {
        if crate::nativeform::draw_buffered(item.dc, &item.bounds, |dc, bounds| {
            draw_form_item_impl(item, dc, bounds);
        }) {
            return;
        }
        draw_form_item_impl(item, item.dc, &item.bounds);
    }
}

fn draw_form_item_impl(item: &crate::nativeform::DrawItem, dc: HDC, bounds: &RECT) {
    let p = theme::palette();
    let scale = crate::scale_pub(96);
    let font = form_font_body();
    unsafe {
        // Picker card surfaces (Go DrawSurface): elevated fill + border,
        // no label — never the button path.
        if matches!(
            item.control_id as usize,
            PK_SEARCH_SURFACE | PK_LIST_SURFACE | PK_PREVIEW_SURFACE
        ) {
            crate::paint::draw_surface(dc, bounds, p.window_bg, p.surface, p.border, 6);
            return;
        }

        // Choice buttons render their closed state via the choice module.
        if crate::choice::is_choice(item.control) {
            let state = crate::nativeform::control_state(item.control, item.state);
            crate::choice::draw_button(item.control, dc, bounds, state);
            return;
        }

        if item.control_id >= FIELD_SURFACE_BASE {
            // Edit field surface: focus/disabled border via draw_field.
            let parent = GetParent(item.control).unwrap_or_default();
            let edit = get_dlg_item(parent, (item.control_id - FIELD_SURFACE_BASE) as usize);
            let mut buffer = [0u16; 8];
            let len = GetClassNameW(edit, &mut buffer);
            let edit_class =
                String::from_utf16_lossy(&buffer[..len.max(0) as usize]).to_uppercase();
            if edit_class != "EDIT" {
                crate::paint::draw_surface(dc, bounds, p.window_bg, p.surface, p.border, 6);
                return;
            }
            let mut state = crate::nativeform::control_state(item.control, item.state);
            state.focused = GetFocus() == edit;
            state.disabled = !IsWindowEnabled(edit).as_bool();
            crate::paint::draw_field(dc, bounds, p, p.window_bg, state, 6);
            return;
        }

        let label = window_text(item.control);
        let weekday_ids: &[i32] = &[
            ED_DAYS_MON as i32,
            ED_DAYS_TUE as i32,
            ED_DAYS_WED as i32,
            ED_DAYS_THU as i32,
            ED_DAYS_FRI as i32,
            ED_DAYS_SAT as i32,
            ED_DAYS_SUN as i32,
        ];
        if item.control_id == ED_KEEP_SCREEN as i32 {
            let mut state = crate::nativeform::control_state(item.control, item.state);
            state.active = edit_is_checked(ED_KEEP_SCREEN);
            crate::paint::draw_checkbox(
                dc,
                bounds,
                font,
                &label,
                p,
                p.window_bg,
                state,
                scale,
                ED_CHECKBOX_SIZE,
            );
        } else if item.control_id == ED_PROC_INFO as i32 {
            // Round info glyph (Go draws it as a circle button).
            let state = crate::nativeform::control_state(item.control, item.state);
            crate::paint::draw_button(
                dc,
                bounds,
                font,
                &label,
                p,
                p.window_bg,
                state,
                (bounds.bottom - bounds.top) / 2,
            );
        } else if weekday_ids.contains(&item.control_id) {
            let mut state = crate::nativeform::control_state(item.control, item.state);
            state.active = edit_is_checked(item.control_id as usize);
            crate::paint::draw_button(dc, bounds, font, &label, p, p.window_bg, state, 6);
        } else {
            let state = crate::nativeform::control_state(item.control, item.state);
            crate::paint::draw_button(dc, bounds, font, &label, p, p.window_bg, state, 6);
        }
    }
}

/// Go nativeform.PresentControl parity: force one synchronous on-screen
/// paint of a native content control after its data changed. Without this,
/// freshly-filled ListView/EDIT controls can keep a validated update region
/// and never composite their text to the screen.
fn present_control(control: HWND) {
    use windows::Win32::Graphics::Gdi::{
        RDW_ERASE, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW, REDRAW_WINDOW_FLAGS, RedrawWindow,
    };
    unsafe {
        let _ = RedrawWindow(
            Some(control),
            None,
            None,
            REDRAW_WINDOW_FLAGS(RDW_INVALIDATE.0 | RDW_ERASE.0 | RDW_UPDATENOW.0 | RDW_FRAME.0),
        );
    }
}

// ===== Process picker =======================================================

#[derive(Clone)]
struct PickItem {
    target: auto::ProcessTarget,
    name: String,
    description: String,
    count: u32,
    search: String,
}

static PK_ITEMS: Mutex<Vec<PickItem>> = Mutex::new(Vec::new());
static PK_VISIBLE: Mutex<Vec<PickItem>> = Mutex::new(Vec::new());
static PK_SELECTED: Mutex<Vec<auto::ProcessTarget>> = Mutex::new(Vec::new());
static PK_SORT: Mutex<(i32, bool)> = Mutex::new((0, true));

/// True while the ListView is being refilled (Go p.populating): item-state
/// notifications during population must not recompute the selection, or
/// not-yet-inserted rows would drop out of it.
static PK_POPULATING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn show_picker(owner: HWND) {
    unsafe {
        if HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _).0 as usize == 0 {
            create_picker(owner);
        }
        let pk = HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _);

        // Seed the selection from the editor draft (Go Show Selected).
        *PK_SELECTED.lock().unwrap() = EDIT_PROCS.lock().unwrap().clone();
        *PK_SORT.lock().unwrap() = (0, true);
        set_text(get_dlg_item(pk, PK_SEARCH), "");

        picker_load();
        apply_filter();

        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(owner, false);
        // The snapshot, rows, preview and status commit as one visible
        // frame (Go picker firstFrame.Reveal).
        crate::FirstFrameGate::begin(pk).reveal();
        // The uncloak can leave freshly-filled native controls with a
        // validated region; force their first on-screen paint explicitly.
        present_control(get_dlg_item(pk, PK_SEARCH));
        present_control(HWND(PK_LIST_HWND.load(Ordering::SeqCst) as *mut _));
        present_control(get_dlg_item(pk, PK_PREVIEW));
        let _ = SetForegroundWindow(pk);
        let search = get_dlg_item(pk, PK_SEARCH);
        if !search.is_invalid() {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(search));
        }
    }
}

fn hide_picker() {
    unsafe {
        let pk = HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _);
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        let _ = ShowWindow(pk, SW_HIDE);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(ed, true);
        let _ = SetForegroundWindow(ed);
        // Go returns focus to the editor's Choose button.
        let choose = get_dlg_item(ed, ED_CHOOSE);
        if !choose.is_invalid() {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(choose));
        }
    }
}

fn create_picker(owner: HWND) {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module handle");
        let font = form_font_body();

        register_picker_class(instance);

        let style = secondary_style();
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: s(PK_W),
            bottom: s(PK_H),
        };
        let _ = AdjustWindowRectEx(&mut frame, style, false, WINDOW_EX_STYLE(0));
        let (x, y) = center_on_parent(frame.right - frame.left, frame.bottom - frame.top);

        let pk = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("IdleTriggerProcPicker"),
            PCWSTR(wide(&t_pub("process_picker_title")).as_ptr()),
            style,
            x,
            y,
            frame.right - frame.left,
            frame.bottom - frame.top,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        )
        .expect("picker window");
        PICKER_HWND.store(pk.0 as isize, Ordering::SeqCst);
        theme::apply_to_window(pk);
        crate::set_window_icons_pub(pk);

        let content_w = PK_W - 2 * PK_PAD;

        let mk_static =
            |id: usize, text: &str, font: windows::Win32::Graphics::Gdi::HFONT, y: i32| {
                let wide_text: Vec<u16> = text.encode_utf16().chain([0]).collect();
                let hwnd_ = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    windows::core::w!("STATIC"),
                    PCWSTR(wide_text.as_ptr()),
                    WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
                    s(PK_PAD),
                    s(y),
                    s(content_w),
                    s(PK_TEXT_H),
                    Some(pk),
                    Some(HMENU(id as *mut _)),
                    Some(instance.into()),
                    None,
                )
                .expect("picker static");
                let _ = SendMessageW(
                    hwnd_,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            };

        // Section-title header via the shared owner-draw path.
        mk_static(
            PK_HEADING,
            &t_pub("process_picker_heading"),
            font,
            PK_HEADING_Y,
        );
        mk_static(
            PK_HELPER,
            &t_pub("process_picker_helper"),
            font,
            PK_HELPER_Y,
        );

        // Search field surface + edit (Go searchSurface 370 wide).
        let surface = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("STATIC"),
            windows::core::w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0 | 13), // SS_OWNERDRAW
            s(PK_PAD),
            s(PK_SEARCH_Y),
            s(370),
            s(PK_FIELD_H),
            Some(pk),
            Some(HMENU(PK_SEARCH_SURFACE as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("picker search surface");
        let _ = SetWindowPos(
            surface,
            Some(windows::Win32::Foundation::HWND(1 as *mut _)), // HWND_BOTTOM
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        let _ = SendMessageW(
            surface,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );

        let search = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("EDIT"),
            windows::core::w!(""),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_TABSTOP.0
                    | WS_CLIPSIBLINGS.0
                    | ES_AUTOHSCROLL as u32,
            ),
            s(PK_PAD + 2),
            s(PK_SEARCH_Y + 7),
            s(370 - 4),
            s(20),
            Some(pk),
            Some(HMENU(PK_SEARCH as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("picker search");
        let _ = SendMessageW(
            search,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        // Cue banner hint (Go NewCueBanner).
        let hint = wide(&t_pub("process_picker_search_hint"));
        let _ = SendMessageW(
            search,
            EM_SETCUEBANNER_RAW,
            Some(WPARAM(1)),
            Some(LPARAM(hint.as_ptr() as isize)),
        );

        // Refresh + Browse buttons (Go 132 / 146 wide).
        for (id, key, x, w) in [
            (
                PK_REFRESH,
                "process_picker_refresh",
                PK_PAD + 370 + ED_GAP,
                132,
            ),
            (
                PK_BROWSE,
                "process_picker_browse",
                PK_PAD + 370 + ED_GAP + 132 + ED_GAP,
                146,
            ),
        ] {
            let wide_text: Vec<u16> = t_pub(key).encode_utf16().chain([0]).collect();
            let btn = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("BUTTON"),
                PCWSTR(wide_text.as_ptr()),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
                s(x),
                s(PK_SEARCH_Y),
                s(w),
                s(PK_FIELD_H),
                Some(pk),
                Some(HMENU(id as *mut _)),
                Some(instance.into()),
                None,
            )
            .expect("picker button");
            let _ = SendMessageW(
                btn,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            crate::nativeform::track(btn);
        }

        // List surface card + ListView (Go SysListView32 report + checkboxes).
        let list_surface = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("STATIC"),
            windows::core::w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0 | 13), // SS_OWNERDRAW
            s(PK_PAD),
            s(PK_LIST_Y),
            s(content_w),
            s(PK_LIST_H),
            Some(pk),
            Some(HMENU(PK_LIST_SURFACE as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("picker list surface");
        let _ = SetWindowPos(
            list_surface,
            Some(windows::Win32::Foundation::HWND(1 as *mut _)), // HWND_BOTTOM
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        let _ = SendMessageW(
            list_surface,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );

        let list = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("SysListView32"),
            windows::core::w!(""),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_TABSTOP.0
                    | WS_CLIPCHILDREN.0
                    | WS_CLIPSIBLINGS.0
                    | 0x0001 // LVS_REPORT
                    | 0x0004 // LVS_SINGLESEL
                    | 0x0008, // LVS_SHOWSELALWAYS
            ),
            s(PK_PAD + 2),
            s(PK_LIST_Y + 2),
            s(content_w - 4),
            s(PK_LIST_H - 4),
            Some(pk),
            Some(HMENU(PK_LIST as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("picker list");
        let _ = SendMessageW(
            list,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        PK_LIST_HWND.store(list.0 as isize, Ordering::SeqCst);

        let _ = SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            Some(WPARAM(0)),
            Some(LPARAM(
                (LVS_EX_CHECKBOXES | LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
            )),
        );
        picker_create_columns(list);
        apply_list_theme(list);
        apply_state_images(list);

        // Loading overlay inside the list card (Go idEmpty, hidden until the
        // list turns empty).
        mk_static(
            PK_EMPTY,
            &t_pub("process_picker_loading"),
            font,
            PK_LIST_Y + (PK_LIST_H - PK_TEXT_H) / 2,
        );
        let _ = ShowWindow(get_dlg_item(pk, PK_EMPTY), SW_HIDE);

        mk_static(
            PK_STATUS,
            &t_pub("process_picker_loading"),
            font,
            PK_STATUS_Y,
        );
        mk_static(
            PK_PREVIEW_TITLE,
            &fill_template(&t_pub("process_picker_selection_title"), &["0"]),
            font,
            PK_PREVIEW_TITLE_Y,
        );

        // Preview card surface + no-selection listbox (Go idPreviewSurface).
        let preview_surface = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("STATIC"),
            windows::core::w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0 | 13), // SS_OWNERDRAW
            s(PK_PAD),
            s(PK_PREVIEW_Y),
            s(content_w),
            s(PK_PREVIEW_H),
            Some(pk),
            Some(HMENU(PK_PREVIEW_SURFACE as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("picker preview surface");
        let _ = SetWindowPos(
            preview_surface,
            Some(windows::Win32::Foundation::HWND(1 as *mut _)), // HWND_BOTTOM
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        let _ = SendMessageW(
            preview_surface,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        let preview = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("LISTBOX"),
            windows::core::w!(""),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_VSCROLL.0
                    | WS_CLIPSIBLINGS.0
                    | LBS_NOSEL
                    | LBS_NOINTEGRALHEIGHT as u32,
            ),
            s(PK_PAD + 2),
            s(PK_PREVIEW_Y + 2),
            s(content_w - 4),
            s(PK_PREVIEW_H - 4),
            Some(pk),
            Some(HMENU(PK_PREVIEW as *mut _)),
            Some(instance.into()),
            None,
        )
        .expect("picker preview");
        let _ = SendMessageW(
            preview,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );

        mk_static(
            PK_PRIVACY,
            &t_pub("process_picker_privacy"),
            font,
            PK_PRIVACY_Y,
        );

        // Footer: Confirm right, Cancel left of it (Go placement).
        let confirm_x = PK_W - PK_PAD - PK_BUTTON_W;
        let cancel_x = confirm_x - ED_GAP - PK_BUTTON_W;
        for (id, key, x) in [
            (PK_CANCEL, "automation_cancel", cancel_x),
            (PK_CONFIRM, "process_picker_confirm", confirm_x),
        ] {
            let wide_text: Vec<u16> = t_pub(key).encode_utf16().chain([0]).collect();
            let btn = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("BUTTON"),
                PCWSTR(wide_text.as_ptr()),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32),
                s(x),
                s(PK_BUTTONS_Y),
                s(PK_BUTTON_W),
                s(BUTTON_H),
                Some(pk),
                Some(HMENU(id as *mut _)),
                Some(instance.into()),
                None,
            )
            .expect("picker button");
            let _ = SendMessageW(
                btn,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            crate::nativeform::track(btn);
        }
    }
}

unsafe fn register_picker_class(instance: windows::Win32::Foundation::HMODULE) {
    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(picker_proc),
            hInstance: instance.into(),
            lpszClassName: windows::core::w!("IdleTriggerProcPicker"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1usize as *const u16))
                .unwrap_or_default(),
            hbrBackground: windows::Win32::Graphics::Gdi::GetSysColorBrush(
                windows::Win32::Graphics::Gdi::COLOR_WINDOW,
            ),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
}

unsafe fn picker_create_columns(list: HWND) {
    unsafe {
        let widths = [250, 310, 82];
        let keys = [
            "process_picker_column_process",
            "process_picker_column_description",
            "process_picker_column_instances",
        ];
        use windows::Win32::UI::Controls::{
            LVCF_FMT, LVCF_SUBITEM, LVCF_TEXT, LVCF_WIDTH, LVCFMT_LEFT, LVCOLUMNW,
        };
        for (index, key) in keys.iter().enumerate() {
            let mut text = wide(&t_pub(key));
            let mut column = LVCOLUMNW {
                mask: LVCF_FMT | LVCF_WIDTH | LVCF_TEXT | LVCF_SUBITEM,
                fmt: LVCFMT_LEFT,
                cx: s(widths[index]),
                pszText: windows::core::PWSTR(text.as_mut_ptr()),
                cchTextMax: 0,
                iSubItem: index as i32,
                iImage: 0,
                iOrder: index as i32,
                cxMin: 0,
                ..Default::default()
            };
            let _ = SendMessageW(
                list,
                LVM_INSERTCOLUMNW,
                Some(WPARAM(index)),
                Some(LPARAM(&mut column as *mut _ as isize)),
            );
        }
    }
}

unsafe fn apply_list_theme(list: HWND) {
    unsafe {
        let p = theme::palette();
        let _ = SendMessageW(
            list,
            LVM_SETBKCOLOR,
            Some(WPARAM(0)),
            Some(LPARAM(p.surface as isize)),
        );
        let _ = SendMessageW(
            list,
            LVM_SETTEXTCOLOR,
            Some(WPARAM(0)),
            Some(LPARAM(p.text as isize)),
        );
        let _ = SendMessageW(
            list,
            LVM_SETTEXTBKCOLOR,
            Some(WPARAM(0)),
            Some(LPARAM(p.surface as isize)),
        );
        // The embedded header follows the window dark mode.
        let header = SendMessageW(list, LVM_GETHEADER, Some(WPARAM(0)), Some(LPARAM(0))).0;
        if header != 0 {
            theme::apply_to_window(HWND(header as *mut _));
        }
    }
}

/// Custom checkbox state images (Go applyStateImages): the same glyph style
/// as form checkboxes, drawn into an image list.
unsafe fn apply_state_images(list: HWND) {
    unsafe {
        use windows::Win32::UI::Controls::{ImageList_Add, ImageList_Create};
        let p = theme::palette();
        let size = s(22).max(22);
        let images = ImageList_Create(
            size,
            size,
            windows::Win32::UI::Controls::IMAGELIST_CREATION_FLAGS(0x0000_0020), // ILC_COLOR32
            2,
            0,
        );
        if images.is_invalid() {
            return;
        }
        for checked in [false, true] {
            let hdc = windows::Win32::Graphics::Gdi::GetDC(Some(list));
            let memory = windows::Win32::Graphics::Gdi::CreateCompatibleDC(Some(hdc));
            let bitmap = windows::Win32::Graphics::Gdi::CreateCompatibleBitmap(hdc, size, size);
            let old = windows::Win32::Graphics::Gdi::SelectObject(
                memory,
                windows::Win32::Graphics::Gdi::HGDIOBJ(bitmap.0),
            );
            let cell = RECT {
                left: 0,
                top: 0,
                right: size,
                bottom: size,
            };
            crate::paint::fill_rect(memory, &cell, p.surface);
            let box_size = s(ED_CHECKBOX_SIZE);
            let box_rect = RECT {
                left: (size - box_size) / 2,
                top: (size - box_size) / 2,
                right: (size + box_size) / 2,
                bottom: (size + box_size) / 2,
            };
            let (fill, border) = if checked {
                (p.accent, p.accent)
            } else {
                (p.surface, p.border)
            };
            let _ = crate::paint::fill_rounded_rect(
                memory,
                &box_rect,
                crate::paint::sp(2, crate::scale_pub(96)),
                fill,
                border,
            );
            if checked {
                let _ = crate::paint::draw_check(
                    memory,
                    box_rect.left,
                    box_rect.top,
                    box_rect.right,
                    box_rect.bottom,
                    p.accent_text,
                    crate::paint::sp(2, crate::scale_pub(96)).max(1),
                );
            }
            windows::Win32::Graphics::Gdi::SelectObject(memory, old);
            ImageList_Add(images, bitmap, None);
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(
                windows::Win32::Graphics::Gdi::HGDIOBJ(bitmap.0),
            );
            let _ = windows::Win32::Graphics::Gdi::DeleteDC(memory);
            let _ = windows::Win32::Graphics::Gdi::ReleaseDC(Some(list), hdc);
        }
        let _ = SendMessageW(
            list,
            LVM_SETIMAGELIST,
            Some(WPARAM(LVSIL_STATE)),
            Some(LPARAM(images.0 as isize)),
        );
    }
}

/// One process snapshot pass, grouped into picker rows (Go startLoad →
/// finishLoad → buildItems). Descriptions fill in asynchronously.
fn picker_load() {
    let snapshot = crate::automation::Snapshot::take();
    let self_pid = std::process::id();
    let selected = PK_SELECTED.lock().unwrap().clone();

    // Description paths come from this single snapshot; the worker only
    // reads version resources afterwards (Go reuses its scan result too).
    let mut desc_paths: Vec<String> = Vec::new();
    let mut items: Vec<PickItem> = Vec::new();
    if let Some(snapshot) = snapshot {
        let mut counts: HashMap<String, u32> = HashMap::new();
        for (name, pid) in snapshot.pid_names() {
            if *pid == self_pid {
                continue;
            }
            *counts.entry(name.clone()).or_insert(0) += 1;
        }
        let mut names: Vec<String> = counts.keys().cloned().collect();
        names.sort();
        for name in &names {
            if let Some(path) = snapshot.path_of(name) {
                desc_paths.push(path);
            }
        }
        for name in names {
            let count = counts[&name];
            let target = auto::ProcessTarget {
                kind: "name".into(),
                executable: name.clone(),
                path: String::new(),
            };
            let search = name.to_lowercase();
            items.push(PickItem {
                target,
                name: name.clone(),
                description: String::new(),
                count,
                search,
            });
        }
    }

    // Selected name targets that are not running stay visible (Go
    // process_picker_not_running).
    let mut missing: Vec<auto::ProcessTarget> = selected
        .iter()
        .filter(|t| t.kind != "path" && !items.iter().any(|i| i.target.key() == t.key()))
        .cloned()
        .collect();
    missing.sort_by_key(|t| t.executable.to_lowercase());
    for target in missing {
        let description = t_pub("process_picker_not_running");
        let search = format!("{} {}", target.executable, description).to_lowercase();
        items.push(PickItem {
            name: target.executable.clone(),
            description,
            count: 0,
            search,
            target,
        });
    }

    *PK_ITEMS.lock().unwrap() = items;
    spawn_description_load(desc_paths);
}

/// Resolves executable descriptions off the UI thread, then posts a message
/// so the picker repaints the description column (Go enrich worker).
fn spawn_description_load(paths: Vec<String>) {
    let pk = HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _);
    if pk.is_invalid() || paths.is_empty() {
        return;
    }
    std::thread::Builder::new()
        .name("picker-desc".into())
        .spawn(move || {
            let mut found: HashMap<String, String> = HashMap::new();
            for path in paths {
                if let Some(description) = file_description(&path) {
                    found.insert(path, description);
                }
            }
            if found.is_empty() {
                return;
            }
            {
                // Map path → executable name for the item rows.
                let mut items = PK_ITEMS.lock().unwrap();
                for item in items.iter_mut() {
                    if !item.description.is_empty() {
                        continue;
                    }
                    // Match by file stem against the executable name.
                    let stem = std::path::Path::new(&item.name)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    for (path, description) in &found {
                        let path_stem = std::path::Path::new(path)
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_lowercase())
                            .unwrap_or_default();
                        if path_stem == stem {
                            item.description = description.clone();
                            item.search = format!("{} {}", item.name, description).to_lowercase();
                            break;
                        }
                    }
                }
            }
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _)),
                    WM_APP_PICKER_DESC,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        })
        .ok();
}

/// Reads the FileDescription string from a PE version resource.
fn file_description(path: &str) -> Option<String> {
    use windows::core::PCWSTR;
    unsafe {
        let wide_path = wide(path);
        let size = windows::Win32::Storage::FileSystem::GetFileVersionInfoSizeW(
            PCWSTR(wide_path.as_ptr()),
            None,
        );
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if !windows::Win32::Storage::FileSystem::GetFileVersionInfoW(
            PCWSTR(wide_path.as_ptr()),
            None,
            size,
            data.as_mut_ptr().cast(),
        )
        .is_ok()
        {
            return None;
        }
        // Read the translation table, then the first language's description.
        let mut block: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len: u32 = 0;
        let query = windows::core::w!("\\VarFileInfo\\Translation");
        if !windows::Win32::Storage::FileSystem::VerQueryValueW(
            data.as_ptr().cast(),
            query,
            &mut block,
            &mut len,
        )
        .as_bool()
            || len < 4
        {
            return None;
        }
        let lang = *(block as *const u16);
        let code_page = *((block as *const u16).add(1));
        let sub = format!(
            "\\StringFileInfo\\{:04x}{:04x}\\FileDescription",
            lang, code_page
        );
        let sub_wide = wide(&sub);
        let mut text: *mut core::ffi::c_void = std::ptr::null_mut();
        if !windows::Win32::Storage::FileSystem::VerQueryValueW(
            data.as_ptr().cast(),
            PCWSTR(sub_wide.as_ptr()),
            &mut text,
            &mut len,
        )
        .as_bool()
            || text.is_null()
        {
            return None;
        }
        let chars = std::slice::from_raw_parts(text as *const u16, len as usize);
        let end = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
        Some(String::from_utf16_lossy(&chars[..end]))
    }
}

/// Filters and sorts the item rows, then repopulates the ListView (Go
/// applyFilter + reconcileVisible).
fn apply_filter() {
    unsafe {
        let pk = HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _);
        let list = HWND(PK_LIST_HWND.load(Ordering::SeqCst) as *mut _);
        let filter = get_text(get_dlg_item(pk, PK_SEARCH)).trim().to_lowercase();
        let (column, ascending) = *PK_SORT.lock().unwrap();

        let mut items = PK_ITEMS.lock().unwrap().clone();
        items.retain(|i| filter.is_empty() || i.search.contains(&filter));
        items.sort_by(|a, b| {
            let key = |item: &PickItem| match column {
                1 => item.description.to_lowercase(),
                2 => format!("{:08}", item.count),
                _ => item.name.to_lowercase(),
            };
            if ascending {
                key(a).cmp(&key(b))
            } else {
                key(b).cmp(&key(a))
            }
        });
        *PK_VISIBLE.lock().unwrap() = items.clone();

        let _ = SendMessageW(list, WM_SETREDRAW, Some(WPARAM(0)), Some(LPARAM(0)));
        PK_POPULATING.store(true, Ordering::SeqCst);
        let _ = SendMessageW(list, LVM_DELETEALLITEMS, Some(WPARAM(0)), Some(LPARAM(0)));
        for (index, item) in items.iter().enumerate() {
            lv_insert(list, index, item);
        }
        PK_POPULATING.store(false, Ordering::SeqCst);
        let _ = SendMessageW(list, WM_SETREDRAW, Some(WPARAM(1)), Some(LPARAM(0)));
        present_control(list);

        // Empty overlay + status row.
        let empty = items.is_empty();
        let message = if !filter.is_empty() {
            t_pub("process_picker_no_results")
        } else {
            t_pub("process_picker_empty")
        };
        set_text(get_dlg_item(pk, PK_EMPTY), &message);
        let overlay = get_dlg_item(pk, PK_EMPTY);
        if !overlay.is_invalid() {
            let _ = ShowWindow(overlay, if empty { SW_SHOW } else { SW_HIDE });
        }
        update_selection_status(pk);
        update_preview(pk);
        update_header_captions(pk);
    }
}

unsafe fn lv_insert(list: HWND, index: usize, item: &PickItem) {
    unsafe {
        use windows::Win32::UI::Controls::{LVIF_TEXT, LVITEMW};
        let mut name = wide(&item.name);
        let mut entry = LVITEMW {
            mask: LVIF_TEXT,
            iItem: index as i32,
            pszText: windows::core::PWSTR(name.as_mut_ptr()),
            cchTextMax: 0,
            ..Default::default()
        };
        let _ = SendMessageW(
            list,
            LVM_INSERTITEMW,
            Some(WPARAM(0)),
            Some(LPARAM(&mut entry as *mut _ as isize)),
        );
        for (column, value) in [(1i32, &item.description), (2, &item.count.to_string())] {
            let mut wide_value = wide(value);
            let mut cell = LVITEMW {
                iItem: index as i32,
                iSubItem: column,
                pszText: windows::core::PWSTR(wide_value.as_mut_ptr()),
                ..Default::default()
            };
            let _ = SendMessageW(
                list,
                LVM_SETITEMTEXTW,
                Some(WPARAM(index)),
                Some(LPARAM(&mut cell as *mut _ as isize)),
            );
        }
        let selected = PK_SELECTED.lock().unwrap();
        let checked = selected.iter().any(|t| t.key() == item.target.key());
        lv_set_check(list, index, checked);
    }
}

unsafe fn lv_set_check(list: HWND, index: usize, checked: bool) {
    unsafe {
        use windows::Win32::UI::Controls::{
            LIST_VIEW_ITEM_STATE_FLAGS, LVIS_STATEIMAGEMASK, LVITEMW,
        };
        let state: u32 = if checked { 2 << 12 } else { 1 << 12 };
        let mut entry = LVITEMW {
            state: LIST_VIEW_ITEM_STATE_FLAGS(state),
            stateMask: LVIS_STATEIMAGEMASK,
            ..Default::default()
        };
        let _ = SendMessageW(
            list,
            LVM_SETITEMSTATE,
            Some(WPARAM(index)),
            Some(LPARAM(&mut entry as *mut _ as isize)),
        );
    }
}

unsafe fn lv_is_checked(list: HWND, index: usize) -> bool {
    unsafe {
        let state = SendMessageW(
            list,
            LVM_GETITEMSTATE,
            Some(WPARAM(index)),
            Some(LPARAM(
                windows::Win32::UI::Controls::LVIS_STATEIMAGEMASK.0 as isize,
            )),
        )
        .0 as u32;
        state & 0x0000_F000 == 2 << 12
    }
}

/// Reads checkbox states back into the selection (Go captureSelection).
unsafe fn capture_selection() {
    unsafe {
        let list = HWND(PK_LIST_HWND.load(Ordering::SeqCst) as *mut _);
        let visible = PK_VISIBLE.lock().unwrap().clone();
        let mut selected: Vec<auto::ProcessTarget> = PK_SELECTED
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.kind == "path")
            .cloned()
            .collect();
        for (index, item) in visible.iter().enumerate() {
            if lv_is_checked(list, index) {
                selected.push(item.target.clone());
            }
        }
        *PK_SELECTED.lock().unwrap() = selected;
    }
}

fn selected_count() -> usize {
    PK_SELECTED.lock().unwrap().len()
}

/// Go updateSelectionStatus: limit warning or "shown N · selected M".
fn update_selection_status(pk: HWND) {
    let visible = PK_VISIBLE.lock().unwrap().len();
    let selected = selected_count();
    let status = if selected > auto::MAX_PROCESSES_PER_RULE {
        fill_template(
            &t_pub("process_picker_limit"),
            &[&auto::MAX_PROCESSES_PER_RULE.to_string()],
        )
    } else {
        fill_template(
            &t_pub("process_picker_status_results"),
            &[&visible.to_string(), &selected.to_string()],
        )
    };
    set_text(get_dlg_item(pk, PK_STATUS), &status);
    enable_control(
        pk,
        PK_CONFIRM,
        selected > 0 && selected <= auto::MAX_PROCESSES_PER_RULE,
    );
}

/// Rebuilds the preview card and its title (Go updatePreview).
fn update_preview(pk: HWND) {
    unsafe {
        let preview = get_dlg_item(pk, PK_PREVIEW);
        let selected = PK_SELECTED.lock().unwrap().clone();
        let _ = SendMessageW(preview, LB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
        if selected.is_empty() {
            let text = wide(&t_pub("process_picker_preview_empty"));
            let _ = SendMessageW(
                preview,
                LB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(text.as_ptr() as isize)),
            );
        } else {
            for target in &selected {
                let name = &target.executable;
                let label = if target.kind == "path" {
                    fill_template(&t_pub("process_picker_preview_path"), &[name, &target.path])
                } else {
                    fill_template(&t_pub("process_picker_preview_name"), &[name])
                };
                let text = wide(&label);
                let _ = SendMessageW(
                    preview,
                    LB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(text.as_ptr() as isize)),
                );
            }
        }
        set_text(
            get_dlg_item(pk, PK_PREVIEW_TITLE),
            &fill_template(
                &t_pub("process_picker_selection_title"),
                &[&selected.len().to_string()],
            ),
        );
        present_control(preview);
    }
}

/// Column captions with sort arrows (Go headerCaption).
fn update_header_captions(pk: HWND) {
    unsafe {
        let list = HWND(PK_LIST_HWND.load(Ordering::SeqCst) as *mut _);
        let (column, ascending) = *PK_SORT.lock().unwrap();
        let keys = [
            "process_picker_column_process",
            "process_picker_column_description",
            "process_picker_column_instances",
        ];
        for (index, key) in keys.iter().enumerate() {
            let mut caption = t_pub(key);
            if column == index as i32 {
                caption += if ascending { "  ↑" } else { "  ↓" };
            }
            let mut text = wide(&caption);
            let mut column_data = windows::Win32::UI::Controls::LVCOLUMNW {
                mask: windows::Win32::UI::Controls::LVCF_TEXT,
                pszText: windows::core::PWSTR(text.as_mut_ptr()),
                ..Default::default()
            };
            let _ = SendMessageW(
                list,
                LVM_SETCOLUMNW,
                Some(WPARAM(index)),
                Some(LPARAM(&mut column_data as *mut _ as isize)),
            );
        }
        let _ = pk;
    }
}

/// Go confirm: publish the checked targets back to the editor draft.
fn picker_confirm() {
    unsafe {
        capture_selection();
        let selected = PK_SELECTED.lock().unwrap().clone();
        if selected.len() > auto::MAX_PROCESSES_PER_RULE {
            let pk = HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _);
            update_selection_status(pk);
            return;
        }
        *EDIT_PROCS.lock().unwrap() = selected;
        update_proc_summary();
        layout_editor();
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        clear_editor_error(ed);
        hide_picker();
    }
}

/// Go browseExecutable: file dialog restricted to real executables.
fn browse_executable(pk: HWND) {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OPENFILENAMEW};
    unsafe {
        let filter_text = t_pub("process_picker_exe_filter");
        let mut filter: Vec<u16> = filter_text.encode_utf16().collect();
        filter.push(0);
        filter.extend("*.exe".encode_utf16());
        filter.push(0);
        filter.push(0);
        let mut file = vec![0u16; 32768];
        let title = wide(&t_pub("process_picker_browse_title"));
        let default_extension = wide("exe");
        let mut dialog = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: pk,
            lpstrFilter: windows::core::PCWSTR(filter.as_ptr()),
            nFilterIndex: 1,
            lpstrFile: windows::core::PWSTR(file.as_mut_ptr()),
            nMaxFile: file.len() as u32,
            lpstrTitle: windows::core::PCWSTR(title.as_ptr()),
            lpstrDefExt: windows::core::PCWSTR(default_extension.as_ptr()),
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
        let path = path.trim().to_string();
        let extension = std::path::Path::new(&path)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if extension != "exe" {
            set_text(
                get_dlg_item(pk, PK_STATUS),
                &t_pub("process_picker_invalid_exe"),
            );
            return;
        }
        let wide_path = wide(&path);
        let mut binary_type: u32 = 0;
        let valid = windows::Win32::Storage::FileSystem::GetBinaryTypeW(
            windows::core::PCWSTR(wide_path.as_ptr()),
            &mut binary_type,
        )
        .is_ok();
        if !valid {
            set_text(
                get_dlg_item(pk, PK_STATUS),
                &t_pub("process_picker_invalid_exe"),
            );
            return;
        }
        let executable = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let target = auto::ProcessTarget {
            kind: "path".into(),
            executable,
            path: path.clone(),
        };
        // A path target replaces the equivalent name target (Go parity).
        let mut selected: Vec<auto::ProcessTarget> = PK_SELECTED
            .lock()
            .unwrap()
            .iter()
            .filter(|t| {
                !(t.kind == "name" && t.executable.eq_ignore_ascii_case(&target.executable))
            })
            .cloned()
            .collect();
        if selected.len() + 1 > auto::MAX_PROCESSES_PER_RULE
            && !selected.iter().any(|t| t.key() == target.key())
        {
            set_text(
                get_dlg_item(pk, PK_STATUS),
                &fill_template(
                    &t_pub("process_picker_limit"),
                    &[&auto::MAX_PROCESSES_PER_RULE.to_string()],
                ),
            );
            return;
        }
        selected.retain(|t| t.key() != target.key());
        selected.push(target);
        *PK_SELECTED.lock().unwrap() = selected;
        sync_check_states();
        update_preview(pk);
        update_selection_status(pk);

        // Resolve the description off-thread (Go browse worker).
        let pk_addr = pk.0 as isize;
        std::thread::Builder::new()
            .name("picker-browse".into())
            .spawn(move || {
                let description = file_description(&path);
                if let Some(description) = description {
                    // Preview shows the raw executable; nothing to merge into
                    // items for a path target beyond the preview itself.
                    let _ = description;
                }
                let _ = PostMessageW(
                    Some(HWND(pk_addr as *mut _)),
                    WM_APP_PICKER_DESC,
                    WPARAM(0),
                    LPARAM(0),
                );
            })
            .ok();
    }
}

/// Pushes the selection back onto the visible checkboxes (Go syncCheckStates).
unsafe fn sync_check_states() {
    unsafe {
        PK_POPULATING.store(true, Ordering::SeqCst);
        let list = HWND(PK_LIST_HWND.load(Ordering::SeqCst) as *mut _);
        let visible = PK_VISIBLE.lock().unwrap().clone();
        let selected = PK_SELECTED.lock().unwrap().clone();
        for (index, item) in visible.iter().enumerate() {
            let checked = selected.iter().any(|t| t.key() == item.target.key());
            lv_set_check(list, index, checked);
        }
        PK_POPULATING.store(false, Ordering::SeqCst);
    }
}

unsafe extern "system" fn picker_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                let code = wparam.0 & 0xFFFF;
                let hi = ((wparam.0 >> 16) & 0xFFFF) as u16;
                match code {
                    PK_CONFIRM if hi == BN_CLICKED => picker_confirm(),
                    PK_CANCEL if hi == BN_CLICKED => hide_picker(),
                    PK_REFRESH if hi == BN_CLICKED => {
                        capture_selection();
                        picker_load();
                        apply_filter();
                    }
                    PK_BROWSE if hi == BN_CLICKED => browse_executable(hwnd),
                    PK_SEARCH if hi == EN_CHANGE => {
                        // Go debounces 120ms; in-memory filtering is fast
                        // enough to run directly.
                        apply_filter();
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_NOTIFY => {
                handle_picker_notify(hwnd, lparam);
                LRESULT(0)
            }
            WM_APP_PICKER_DESC => {
                // Description worker finished: refresh rows and status.
                apply_filter();
                LRESULT(0)
            }
            WM_DRAWITEM => {
                if let Some(item) = crate::nativeform::draw_item(lparam) {
                    draw_form_item(&item);
                    LRESULT(1)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_CLOSE => {
                hide_picker();
                LRESULT(0)
            }
            WM_ERASEBKGND => {
                crate::popups::erase_theme_bg(hwnd, wparam);
                LRESULT(1)
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX => {
                // Helper/status/privacy/preview-title SecondaryText; heading
                // and titles PrimaryText; overlays paint on the Surface card.
                let palette = theme::palette();
                let id = GetWindowLongPtrW(HWND(lparam.0 as *mut _), GWL_ID) as usize;
                let secondary = matches!(id, PK_HELPER | PK_STATUS | PK_PRIVACY | PK_PREVIEW_TITLE);
                let on_surface = matches!(id, PK_EMPTY | PK_PREVIEW);
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                    hdc,
                    COLORREF(if secondary {
                        palette.text2
                    } else {
                        palette.text
                    }),
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
            WM_CTLCOLOREDIT => {
                // Search interior: PrimaryText on Surface (Go picker).
                let p = theme::palette();
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                let _ = windows::Win32::Graphics::Gdi::SetTextColor(hdc, COLORREF(p.text));
                let _ = windows::Win32::Graphics::Gdi::SetBkColor(hdc, COLORREF(p.surface));
                let (light, dark) = theme::surface_brush_pairs();
                let pair = if theme::is_dark() { dark } else { light };
                LRESULT(pair.0.0 as isize)
            }
            WM_DESTROY => LRESULT(0),
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Go handleNotify: checkbox changes, column sorting, label-click toggles.
unsafe fn handle_picker_notify(hwnd: HWND, lparam: LPARAM) {
    unsafe {
        use windows::Win32::UI::Controls::{NMHDR, NMITEMACTIVATE, NMLISTVIEW};
        if lparam.0 == 0 {
            return;
        }
        let header = &*(lparam.0 as *const NMHDR);
        if header.idFrom != PK_LIST
            && header.hwndFrom.0 as isize != PK_LIST_HWND.load(Ordering::SeqCst)
        {
            return;
        }
        let code = header.code;
        const LVN_ITEMCHANGED: u32 = 0xFFFF_FF9B; // LVN_FIRST(-100)-1
        const LVN_COLUMNCLICK: u32 = 0xFFFF_FF94; // LVN_FIRST-8
        const NM_CLICK: u32 = 0xFFFF_FFFE; // -2

        let list = HWND(PK_LIST_HWND.load(Ordering::SeqCst) as *mut _);

        if code == LVN_ITEMCHANGED {
            if PK_POPULATING.load(Ordering::SeqCst) {
                return;
            }
            let notification = &*(lparam.0 as *const NMLISTVIEW);
            if (notification.uChanged.0 & 0x0000_0008) == 0 || notification.iItem < 0 {
                return;
            }
            if notification.uNewState & 0x0000_F000 != notification.uOldState & 0x0000_F000 {
                // Go rejects over-limit checks here (canAddSelection) and
                // resets the row to unchecked with the limit message.
                let visible = PK_VISIBLE.lock().unwrap().clone();
                let index = notification.iItem as usize;
                let over_limit =
                    notification.uNewState & 0x0000_F000 == 2 << 12 && index < visible.len() && {
                        let selected = PK_SELECTED.lock().unwrap().clone();
                        let target = &visible[index].target;
                        !selected.iter().any(|t| t.key() == target.key())
                            && selected_count() >= auto::MAX_PROCESSES_PER_RULE
                    };
                if over_limit {
                    // Revert the row quietly: the state write re-enters
                    // ITEMCHANGED, so the populating guard stays held.
                    PK_POPULATING.store(true, Ordering::SeqCst);
                    lv_set_check(list, index, false);
                    PK_POPULATING.store(false, Ordering::SeqCst);
                    set_text(
                        get_dlg_item(hwnd, PK_STATUS),
                        &fill_template(
                            &t_pub("process_picker_limit"),
                            &[&auto::MAX_PROCESSES_PER_RULE.to_string()],
                        ),
                    );
                    return;
                }
                capture_selection();
                update_selection_status(hwnd);
                update_preview(hwnd);
            }
        } else if code == LVN_COLUMNCLICK {
            let notification = &*(lparam.0 as *const NMLISTVIEW);
            let column = notification.iSubItem;
            let mut sort = PK_SORT.lock().unwrap();
            if sort.0 == column {
                sort.1 = !sort.1;
            } else {
                *sort = (column, true);
            }
            drop(sort);
            apply_filter();
        } else if code == NM_CLICK {
            let notification = &*(lparam.0 as *const NMITEMACTIVATE);
            // Go nmClick: run a sub-item hit test at the click point and
            // toggle manually only for plain label hits; state-icon and
            // subitem hits belong to the native checkbox machinery.
            const LVHT_ONITEMSTATEICON: u32 = 0x0008;
            const LVHT_ONITEMLABEL: u32 = 0x0004;
            if notification.iItem < 0 {
                return;
            }
            let mut hit = windows::Win32::UI::Controls::LVHITTESTINFO {
                pt: notification.ptAction,
                iItem: -1,
                iSubItem: -1,
                ..Default::default()
            };
            let _ = SendMessageW(
                list,
                LVM_SUBITEMHITTEST,
                Some(WPARAM(0)),
                Some(LPARAM(&mut hit as *mut _ as isize)),
            );
            if hit.iItem != notification.iItem
                || hit.iSubItem != 0
                || hit.flags.0 & LVHT_ONITEMSTATEICON != 0
                || hit.flags.0 & LVHT_ONITEMLABEL == 0
            {
                return;
            }
            // Toggling by clicking the label keeps the checkbox affordance
            // discoverable (Go nmClick handler).
            let index = notification.iItem as usize;
            let visible = PK_VISIBLE.lock().unwrap().clone();
            if index >= visible.len() {
                return;
            }
            let checked = lv_is_checked(list, index);
            let target = &visible[index].target;
            let can_add = checked
                || PK_SELECTED
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|t| t.key() == target.key())
                || selected_count() < auto::MAX_PROCESSES_PER_RULE;
            if !can_add {
                set_text(
                    get_dlg_item(hwnd, PK_STATUS),
                    &fill_template(
                        &t_pub("process_picker_limit"),
                        &[&auto::MAX_PROCESSES_PER_RULE.to_string()],
                    ),
                );
                return;
            }
            lv_set_check(list, index, !checked);
            capture_selection();
            update_selection_status(hwnd);
            update_preview(hwnd);
        }
    }
}

// ===== Devtools + theme hooks ===============================================

/// Devtools capture support: open the editor in new-rule mode.
#[cfg(feature = "devtools")]
pub fn devtools_show_editor() {
    EDIT_INDEX.store(-1, Ordering::SeqCst);
    *EDIT_PROCS.lock().unwrap() = Vec::new();
    show_editor();
}

/// Devtools capture support: open the process picker from the editor.
#[cfg(feature = "devtools")]
pub fn devtools_show_picker() {
    let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
    show_picker(ed);
}

/// Devtools capture support: open the trigger choice popup on the editor.
#[cfg(feature = "devtools")]
pub fn devtools_open_trigger_choice() {
    let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
    crate::choice::toggle(get_dlg_item(ed, ED_TRIGGER), ed, ED_TRIGGER as i32);
}

/// Devtools capture support: inject one visible rule so list interactions
/// can be exercised (IDLETRIGGER_DEVTOOLS_CLICK_LIST flow).
#[cfg(feature = "devtools")]
pub fn devtools_seed_demo_rule() {
    let rule = auto::Rule {
        id: "devtools-demo".into(),
        name: "演示任务".into(),
        enabled: true,
        action: "lock".into(),
        trigger: "daily".into(),
        time: "18:30".into(),
        end_time: String::new(),
        date: String::new(),
        days: Vec::new(),
        process_logic: String::new(),
        processes: Vec::new(),
        keep_screen_on: false,
        idle_minutes: 0,
        warning_seconds: 0,
        blocked_policy: String::new(),
        max_wait_minutes: 0,
    };
    let mut rules = crate::automation::RULES.lock().unwrap();
    rules.push(rule);
    rules.push(auto::Rule {
        id: "devtools-demo2".into(),
        name: "第二个任务".into(),
        enabled: false,
        action: "shutdown".into(),
        trigger: "once".into(),
        date: "2026-09-20".into(),
        time: "23:00".into(),
        end_time: String::new(),
        days: Vec::new(),
        process_logic: String::new(),
        processes: Vec::new(),
        keep_screen_on: false,
        idle_minutes: 0,
        warning_seconds: 0,
        blocked_policy: String::new(),
        max_wait_minutes: 0,
    });
    drop(rules);
    refresh_list();
}

/// Devtools capture support: post a mouse click on the first rule row.
#[cfg(feature = "devtools")]
pub fn devtools_click_list() {
    unsafe {
        let list = HWND(MGR_LIST_HWND.load(Ordering::SeqCst) as *mut _);
        // Row 1, then the empty area below all rows, then row 2.
        for y in [10, 300, 42] {
            let at = LPARAM(((s(30) as isize) & 0xFFFF) | ((s(y) as isize) << 16));
            let _ = SendMessageW(list, WM_LBUTTONDOWN, Some(WPARAM(1)), Some(at));
            let _ = SendMessageW(list, WM_LBUTTONUP, Some(WPARAM(0)), Some(at));
            let _ = windows::Win32::Graphics::Gdi::UpdateWindow(list);
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
    }
}

/// Theme refresh support: open automation top-level windows.
pub fn theme_hwnds() -> [HWND; 3] {
    [
        HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _),
        HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _),
        HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _),
    ]
}
