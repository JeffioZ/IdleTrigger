#![allow(clippy::manual_dangling_ptr)]
//! Automatic-task manager window hosting the rule list and an embedded
//! rule-editor pane that swaps in for editing; the process picker remains
//! a separate popup, modal to the manager. Owner-drawn controls share the
//! painting, theme, DPI, and viewport helpers used by the other native forms.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use idletrigger_core::automation as auto;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use windows::Win32::Graphics::Gdi::{HDC, UpdateWindow};

use crate::choice::ChoiceItem;
use crate::{t_pub, theme};

// ===== Control IDs ==========================================================

// Manager (Go idTitle 100, idList 102, idNext 103, idNew/Edit/Toggle/Delete).
const MGR_LIST: usize = 300;
const MGR_NEW: usize = 301;
const MGR_EDIT: usize = 302;
const MGR_DELETE: usize = 303;
const MGR_TOGGLE: usize = 304;
/// Posted by the New-button flyout commit: opens the editor with the
/// chosen template outside the popup teardown chain.
const WM_MGR_TEMPLATE_OPEN: u32 = 0x800B;
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
const ED_BATTERY: usize = 364;
const ED_BATTERY_LBL: usize = 365;
const ED_MODE_TITLE: usize = 366;
/// Section cards (owner-draw statics pinned beneath their rows) and the
/// basic-info section title (settings card grammar: titles ride outside).
const ED_CARD_BASIC: usize = 368;
const ED_CARD_TRIGGER: usize = 369;
const ED_CARD_OPTIONS: usize = 370;
const ED_BASIC_TITLE: usize = 371;

/// Every editor control laid out by layout_editor (Go editorControlIDs).
const ED_LAYOUT_IDS: [usize; 50] = [
    ED_MODE_TITLE,
    ED_CARD_BASIC,
    ED_CARD_TRIGGER,
    ED_CARD_OPTIONS,
    ED_BASIC_TITLE,
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
    ED_BATTERY_LBL,
    ED_BATTERY,
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
/// Owner-draw column sort buttons (the listview header's replacement).
const PK_COL_BASE: usize = 414; // +0 name, +1 description, +2 instances
const PK_EMPTY: usize = 344;
/// Window-scoped debounce timer id for the search box filter.
const PK_FILTER_TIMER: usize = 1;
const PK_SEARCH_SURFACE: usize = 420;
const PK_LIST_SURFACE: usize = 421;
const PK_PREVIEW_SURFACE: usize = 422;

// ===== Window handles =======================================================

static MGR_HWND: AtomicIsize = AtomicIsize::new(0);
static MGR_LIST_HWND: AtomicIsize = AtomicIsize::new(0);
static EDIT_HWND: AtomicIsize = AtomicIsize::new(0);
static PICKER_HWND: AtomicIsize = AtomicIsize::new(0);
static PK_LIST_HWND: AtomicIsize = AtomicIsize::new(0);

/// Editor working state. EDIT_ERROR is a UI flag; everything describing the
/// session itself lives in one struct behind one lock so index, snapshot,
/// original rule and process selection can never be observed torn.
static EDIT_ERROR: AtomicBool = AtomicBool::new(false);
static MGR_DISPLAYED_RULES: Mutex<Vec<auto::Rule>> = Mutex::new(Vec::new());

struct EditorSession {
    index: i32, // -1 = new rule
    orig: Option<auto::Rule>,
    base_rules: Vec<auto::Rule>,
    procs: Vec<auto::ProcessTarget>,
}

impl Default for EditorSession {
    fn default() -> Self {
        Self {
            index: -1,
            orig: None,
            base_rules: Vec::new(),
            procs: Vec::new(),
        }
    }
}

static EDIT_SESSION: Mutex<Option<EditorSession>> = Mutex::new(None);

/// Opens a session when a rule is chosen for editing; the snapshot and
/// original rule are filled in later by complete_edit at populate time.
fn begin_edit(index: i32, procs: Vec<auto::ProcessTarget>) {
    *crate::runtime::lock(&EDIT_SESSION) = Some(EditorSession {
        index,
        procs,
        ..Default::default()
    });
}

fn complete_edit(index: i32, base_rules: Vec<auto::Rule>, orig: auto::Rule) {
    let mut guard = crate::runtime::lock(&EDIT_SESSION);
    let session = guard.get_or_insert_with(Default::default);
    session.index = index;
    session.base_rules = base_rules;
    session.orig = Some(orig);
}

fn end_edit() {
    *crate::runtime::lock(&EDIT_SESSION) = None;
}

fn edit_index() -> i32 {
    crate::runtime::lock(&EDIT_SESSION)
        .as_ref()
        .map_or(-1, |s| s.index)
}

fn edit_orig() -> Option<auto::Rule> {
    crate::runtime::lock(&EDIT_SESSION)
        .as_ref()
        .and_then(|s| s.orig.clone())
}

fn edit_procs() -> Vec<auto::ProcessTarget> {
    crate::runtime::lock(&EDIT_SESSION)
        .as_ref()
        .map(|s| s.procs.clone())
        .unwrap_or_default()
}

fn set_edit_procs(procs: Vec<auto::ProcessTarget>) {
    crate::runtime::lock(&EDIT_SESSION)
        .get_or_insert_with(Default::default)
        .procs = procs;
}

// Owner-draw checkboxes track state here (BS_OWNERDRAW absorbs the native
// check bits); keyed by control id, exactly like Go's p.checks map.
static EDIT_CHECKS: Mutex<Option<HashMap<usize, bool>>> = Mutex::new(None);

fn edit_checks() -> std::sync::MutexGuard<'static, Option<HashMap<usize, bool>>> {
    crate::runtime::lock(&EDIT_CHECKS)
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
            crate::accessibility::check(control, value);
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(control), None, true);
        }
    }
}

fn edit_toggle(ed: HWND, id: usize) {
    let next = !edit_is_checked(id);
    edit_set_checked(ed, id, next);
}

// ===== Geometry (Go tokens) =================================================

// Manager: one window, two views. The rule list view is compact (the Go
// manager size); opening the editor swaps the whole client area to the
// editor pane and grows the window to the form's content height. The
// process picker remains a modal popup.
const MGR_PAD: i32 = 18;
const MGR_TITLE_Y: i32 = 16; // formEdgePadding
const MGR_TEXT_H: i32 = 18;
const MGR_TITLE_H: i32 = 24; // page-title row (settings header parity)
const MGR_LIST_Y: i32 = MGR_TITLE_Y + MGR_TITLE_H + 12;
const MGR_LIST_H: i32 = 240;
const MGR_STATUS_Y: i32 = MGR_LIST_Y + MGR_LIST_H + 8;
const MGR_BUTTONS_Y: i32 = MGR_STATUS_Y + MGR_TEXT_H + 16;
// The editor pane spans the full client width (the old editor's width).
const ED_PANE_W: i32 = 680;
const MGR_W: i32 = ED_PANE_W;
const MGR_H: i32 = MGR_BUTTONS_Y + BUTTON_H + 18;

// Editor: editor.go / nativeform metrics.go.
const ED_W: i32 = ED_PANE_W;
// Card-inner row padding: cards ride at ED_EDGE with the settings
// CARD_PAD_X rhythm inside (18 -> ED_EDGE + 12).
const ED_PAD: i32 = ED_EDGE + 12;
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
const EM_GETSEL_RAW: u32 = 0x00B0;

const BUTTON_H: i32 = 36; // nativeform.ButtonHeight

// ===== Small helpers ========================================================

fn s(v: i32) -> i32 {
    crate::scale_pub(v)
}

use crate::wide;
use crate::window_text;

fn set_text(hwnd: HWND, text: &str) {
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide(text).as_ptr()));
    }
}

fn get_dlg_item(parent: HWND, id: usize) -> HWND {
    unsafe { GetDlgItem(Some(parent), id as i32).unwrap_or_default() }
}

fn is_chinese() -> bool {
    crate::i18n_is_chinese()
}

/// Centers on the window the new dialog is modal to (Go centers on the
/// actual owner, not the panel behind it).
fn center_on_parent(owner: HWND, w: i32, h: i32) -> (i32, i32) {
    unsafe {
        let parent = if owner.is_invalid() {
            crate::hwnd(&crate::PANEL)
        } else {
            owner
        };
        let mut wr = RECT::default();
        if GetWindowRect(parent, &mut wr).is_ok() {
            let pw = wr.right - wr.left;
            let ph = wr.bottom - wr.top;
            let work = crate::display::work_area_for(parent);
            return (
                (wr.left + (pw - w) / 2).clamp(work.left, (work.right - w).max(work.left)),
                (wr.top + (ph - h) / 2).clamp(work.top, (work.bottom - h).max(work.top)),
            );
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
    crate::make_font_pub(14, weight)
}

fn secondary_style() -> WINDOW_STYLE {
    // WS_CLIPCHILDREN: these forms host overlay scrollbar siblings above
    // their lists - without the clip the form's background erase would blank
    // the bar between its repaints.
    WINDOW_STYLE(
        (WS_OVERLAPPEDWINDOW.0 & !WS_MAXIMIZEBOX.0 & !WS_THICKFRAME.0 & !WS_MINIMIZEBOX.0)
            | WS_CLIPCHILDREN.0,
    )
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
    idletrigger_core::i18n::format(template, values)
}

// ===== Label / localization mapping (Go actionKey, triggerKey) =============

fn action_label(value: &str) -> String {
    let key = match value {
        auto::ACTION_STAY_AWAKE => "automation_action_stay_awake",
        auto::ACTION_PAUSE_STAY_AWAKE => "automation_action_pause_stay_awake",
        auto::ACTION_ENABLE_IDLE => "automation_action_enable_idle",
        auto::ACTION_PAUSE_IDLE => "automation_action_pause_idle",
        auto::ACTION_LOCK => "menu_lock",
        auto::ACTION_SLEEP => "menu_sleep",
        auto::ACTION_HIBERNATE => "menu_hibernate",
        auto::ACTION_SHUTDOWN => "menu_shutdown",
        auto::ACTION_RESTART => "menu_restart",
        auto::ACTION_SCREEN_OFF => "menu_action_screen_off",
        auto::ACTION_LOGOFF => "menu_action_logoff",
        _ => return value.to_string(),
    };
    t_pub(key)
}

fn trigger_label(value: &str) -> String {
    let key = match value {
        auto::TRIGGER_PROCESS_RUNNING => "trigger_process_running",
        auto::TRIGGER_PROCESS_STARTED => "trigger_process_started",
        auto::TRIGGER_PROCESS_EXITED => "trigger_process_exited",
        auto::TRIGGER_TIME_WINDOW => "trigger_time_window",
        auto::TRIGGER_ONCE => "trigger_once",
        auto::TRIGGER_DAILY => "trigger_daily",
        auto::TRIGGER_WEEKLY => "trigger_weekly",
        auto::TRIGGER_SESSION_LOCKED => "trigger_session_locked",
        auto::TRIGGER_SESSION_UNLOCKED => "trigger_session_unlocked",
        auto::TRIGGER_ON_RESUME => "trigger_on_resume",
        auto::TRIGGER_BATTERY_BELOW => "trigger_battery_below",
        _ => return value.to_string(),
    };
    t_pub(key)
}

fn logic_label(value: &str) -> String {
    let key = match value {
        auto::LOGIC_ANY => "automation_process_any",
        auto::LOGIC_ALL => "automation_process_all",
        auto::LOGIC_NONE => "automation_process_none",
        _ => return value.to_string(),
    };
    t_pub(key)
}

fn blocked_label(value: &str) -> String {
    t_pub(if value == auto::BLOCKED_WAIT {
        "automation_blocked_wait"
    } else {
        "automation_blocked_skip"
    })
}

fn action_keys() -> Vec<&'static str> {
    // Go actions[] order: state actions first, then system actions.
    vec![
        auto::ACTION_STAY_AWAKE,
        auto::ACTION_PAUSE_STAY_AWAKE,
        auto::ACTION_ENABLE_IDLE,
        auto::ACTION_PAUSE_IDLE,
        auto::ACTION_LOCK,
        auto::ACTION_SLEEP,
        auto::ACTION_HIBERNATE,
        auto::ACTION_SHUTDOWN,
        auto::ACTION_RESTART,
        auto::ACTION_SCREEN_OFF,
        auto::ACTION_LOGOFF,
    ]
}

/// Go setTriggerOptions: triggers are filtered by the selected action type.
pub fn trigger_keys_for(action: &str) -> Vec<&'static str> {
    if auto::is_state_action(action) {
        vec![auto::TRIGGER_PROCESS_RUNNING, auto::TRIGGER_TIME_WINDOW]
    } else {
        vec![
            auto::TRIGGER_ONCE,
            auto::TRIGGER_DAILY,
            auto::TRIGGER_WEEKLY,
            auto::TRIGGER_PROCESS_STARTED,
            auto::TRIGGER_PROCESS_EXITED,
            auto::TRIGGER_SESSION_LOCKED,
            auto::TRIGGER_SESSION_UNLOCKED,
            auto::TRIGGER_ON_RESUME,
            auto::TRIGGER_BATTERY_BELOW,
        ]
    }
}

fn logic_keys() -> Vec<&'static str> {
    vec![auto::LOGIC_ANY, auto::LOGIC_ALL, auto::LOGIC_NONE]
}

fn blocked_keys() -> Vec<&'static str> {
    vec![auto::BLOCKED_SKIP, auto::BLOCKED_WAIT]
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
        if key == auto::ACTION_LOCK {
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
        let (x, y) = center_on_parent(
            crate::hwnd(&crate::PANEL),
            frame.right - frame.left,
            frame.bottom - frame.top,
        );

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
        crate::dpi::install(mgr);
        theme::apply_to_window(mgr);
        crate::set_window_icons_pub(mgr);

        // The list view spans the window; MGR_COL_W only sizes the editor
        // grid (the pane's column layout).
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

        // Page-title parity with the settings header: 17/600 on a 24px row.
        let _ = mk_static(
            MGR_TITLE,
            &t_pub(caption_key(MANAGER_TEXTS, MGR_TITLE)),
            crate::make_font_pub(17, 600),
            MGR_PAD,
            MGR_TITLE_Y,
            content_w,
            MGR_TITLE_H,
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

        let list = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("LISTBOX"),
            windows::core::w!(""),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_TABSTOP.0
                    | WS_CLIPSIBLINGS.0
                    | LBS_NOTIFY as u32
                    | LBS_OWNERDRAWFIXED as u32
                    | LBS_HASSTRINGS as u32
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
        crate::list_style::install(list);
        // The list's scrollbar is painted into its card face.
        crate::list_style::set_lane_card(list, get_dlg_item(mgr, MGR_LIST_SURFACE));

        // Empty-state overlay centered in the list card.
        let empty_y = MGR_LIST_Y + (MGR_LIST_H - (2 * MGR_TEXT_H + 12)) / 2;
        let _ = mk_static(
            MGR_EMPTY_TITLE,
            &t_pub(caption_key(MANAGER_TEXTS, MGR_EMPTY_TITLE)),
            section_font,
            MGR_PAD + 24,
            empty_y,
            content_w - 48,
            MGR_TEXT_H,
        );
        let _ = mk_static(
            MGR_EMPTY_BODY,
            &t_pub(caption_key(MANAGER_TEXTS, MGR_EMPTY_BODY)),
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

        // Button row: MGR_TOGGLE's caption is state-managed (enable/disable
        // wording); the registry covers the static captions.
        // The row spans the content width exactly (3 x 130 + 230 + 3 x
        // 8 = 644): no dead strip to the right of the actions.
        for (index, (id, label_key)) in [
            (MGR_NEW, caption_key(MANAGER_TEXTS, MGR_NEW)),
            (MGR_EDIT, caption_key(MANAGER_TEXTS, MGR_EDIT)),
            (MGR_DELETE, caption_key(MANAGER_TEXTS, MGR_DELETE)),
            (MGR_TOGGLE, "automation_toggle"),
        ]
        .into_iter()
        .enumerate()
        {
            let (x, w) = if id == MGR_TOGGLE {
                (MGR_PAD + 3 * (130 + 8), 230)
            } else {
                (MGR_PAD + index as i32 * (130 + 8), 130)
            };
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

pub fn show() {
    let _dpi = crate::dpi::Scope::window(crate::hwnd(&crate::PANEL));
    unsafe {
        ensure_created();
        let mgr = HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _);
        lower_surfaces(mgr);
        refresh_tooltips();
        refresh_list();
        theme::retheme_children(mgr);
        present_control(HWND(MGR_LIST_HWND.load(Ordering::SeqCst) as *mut _));
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
            crate::hwnd(&crate::PANEL),
            false,
        );
        // Go BeginFirstFrame/Reveal: cloak, commit one full frame, uncloak.
        if crate::viewport::metrics(mgr).is_none() {
            crate::viewport::fit(mgr);
        }
        crate::FirstFrameGate::begin(mgr).reveal();
        let _ = SetForegroundWindow(mgr);
        // Go focuses the list (or New when empty) after showing the manager.
        let empty = crate::runtime::lock(&crate::automation::RULES).is_empty();
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

pub fn default_button(window: HWND) -> Option<HWND> {
    // The editor pane is a child of the manager: keyboard focus inside it
    // reports the manager as GA_ROOT, so an open pane takes precedence.
    let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
    unsafe {
        if !ed.is_invalid()
            && window.0 as isize == MGR_HWND.load(Ordering::SeqCst)
            && IsWindowVisible(ed).as_bool()
        {
            return Some(get_dlg_item(ed, ED_SAVE));
        }
    }
    let id = if window.0 as isize == PICKER_HWND.load(Ordering::SeqCst) {
        PK_CONFIRM
    } else if window.0 as isize == MGR_HWND.load(Ordering::SeqCst) {
        MGR_EDIT
    } else if window == ed {
        ED_SAVE
    } else {
        return None;
    };
    Some(get_dlg_item(window, id))
}

/// Whether `control` is the pane or one of its descendants.
fn inside_editor_pane(control: HWND) -> bool {
    let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
    if ed.is_invalid() {
        return false;
    }
    unsafe {
        if !IsWindowVisible(ed).as_bool() {
            return false;
        }
        let mut current = control;
        while !current.is_invalid() {
            if current == ed {
                return true;
            }
            current = match GetParent(current) {
                Ok(parent) => parent,
                Err(_) => break,
            };
        }
        false
    }
}

pub fn weekday_key(window: HWND, key: usize) -> bool {
    if !matches!(key, 0x25 | 0x27 | 0x24 | 0x23) {
        return false;
    }
    unsafe {
        // The pane is embedded in the manager, so accept either the pane
        // itself or the manager root while a weekday checkbox has focus.
        if !inside_editor_pane(GetFocus()) && window.0 as isize != EDIT_HWND.load(Ordering::SeqCst)
        {
            return false;
        }
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        let id = GetDlgCtrlID(GetFocus()) as usize;
        if !(ED_DAYS_MON..=ED_DAYS_SUN).contains(&id) {
            return false;
        }
        let index = id - ED_DAYS_MON;
        let next = match key {
            0x25 => (index + 6) % 7,
            0x27 => (index + 1) % 7,
            0x24 => 0,
            _ => 6,
        };
        crate::nativeform::keyboard_navigation();
        let target = get_dlg_item(ed, ED_DAYS_MON + next);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(target));
        crate::viewport::reveal_control(ed, target);
        true
    }
}

fn refresh_list() {
    unsafe {
        let list = HWND(MGR_LIST_HWND.load(Ordering::SeqCst) as *mut _);

        // Keep the previous selection and top index across the reset (Go
        // populateRules remembers both before clearing).
        let rules = crate::runtime::lock(&crate::automation::RULES).clone();
        let issues = crate::runtime::lock(&crate::automation::ISSUES).clone();

        let had_selection = SendMessageW(list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0;
        let selected_id = usize::try_from(had_selection).ok().and_then(|index| {
            crate::runtime::lock(&MGR_DISPLAYED_RULES)
                .get(index)
                .map(|r| r.id.clone())
        });
        *crate::runtime::lock(&MGR_DISPLAYED_RULES) = rules.clone();
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
            if selected_id.as_ref() == Some(&rule.id) {
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
                Some(next) => crate::t_args("automation_next_format", &[&next]),
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
        auto::TRIGGER_PROCESS_RUNNING => fill_template(
            &t_pub("automation_summary_process_running"),
            &[&action, &count],
        ),
        auto::TRIGGER_PROCESS_STARTED => fill_template(
            &t_pub("automation_summary_process_started"),
            &[&action, &count],
        ),
        auto::TRIGGER_PROCESS_EXITED => fill_template(
            &t_pub("automation_summary_process_exited"),
            &[&action, &count],
        ),
        auto::TRIGGER_TIME_WINDOW => fill_template(
            &t_pub("automation_summary_time_window"),
            &[
                &action,
                &rule.time,
                &rule.end_time,
                &day_summary(&rule.days),
            ],
        ),
        auto::TRIGGER_ONCE => fill_template(
            &t_pub("automation_summary_once"),
            &[&action, &rule.date, &rule.time],
        ),
        auto::TRIGGER_DAILY => {
            fill_template(&t_pub("automation_summary_daily"), &[&action, &rule.time])
        }
        auto::TRIGGER_WEEKLY => fill_template(
            &t_pub("automation_summary_weekly"),
            &[&action, &day_summary(&rule.days), &rule.time],
        ),
        auto::TRIGGER_SESSION_LOCKED => {
            fill_template(&t_pub("automation_summary_session"), &[&action])
        }
        auto::TRIGGER_SESSION_UNLOCKED => {
            fill_template(&t_pub("automation_summary_session_unlocked"), &[&action])
        }
        auto::TRIGGER_ON_RESUME => {
            fill_template(&t_pub("automation_summary_on_resume"), &[&action])
        }
        auto::TRIGGER_BATTERY_BELOW => fill_template(
            &t_pub("automation_summary_battery"),
            &[&action, &rule.battery_level.to_string()],
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
        let rules = crate::runtime::lock(&MGR_DISPLAYED_RULES);
        if let Some(rule) = rules.get(idx) {
            let procs = rule.processes.clone();
            drop(rules);
            begin_edit(idx as i32, procs);
            show_editor();
        }
    }
}

/// Go idToggle: flip the selected rule's enabled flag and republish.
fn toggle_selected() {
    let Some(idx) = selected_index() else {
        return;
    };
    let base = crate::runtime::lock(&MGR_DISPLAYED_RULES).clone();
    let mut rules = base.clone();
    if idx >= rules.len() {
        return;
    }
    rules[idx].enabled = !rules[idx].enabled;
    if let Err(err) = save_rules_to_config(&base, rules) {
        crate::warn_dialog("", &err);
    }
}

/// Go idDelete: confirm, then remove the selected rule.
fn delete_selected(mgr: HWND) {
    let Some(idx) = selected_index() else {
        return;
    };
    let base = crate::runtime::lock(&MGR_DISPLAYED_RULES).clone();
    let rules = &base;
    let Some(rule) = rules.get(idx).cloned() else {
        return;
    };

    let body = crate::t_args("automation_delete_confirm", &[&rule.name]);
    if !confirm_dialog(mgr, &t_pub("automation_delete_title"), &body) {
        return;
    }
    let mut rules = base.clone();
    if idx < rules.len() {
        rules.remove(idx);
    }
    if let Err(err) = save_rules_to_config(&base, rules) {
        crate::warn_dialog("", &err);
        return;
    }
    crate::log_line(&format!("automation: rule {} deleted", rule.id));
}

unsafe extern "system" fn mgr_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    crate::guarded_proc(
        "automation-manager",
        hwnd,
        msg,
        wparam,
        lparam,
        move || unsafe {
            match msg {
                WM_MEASUREITEM => {
                    // Owner-draw rule rows: settings-row height rhythm.
                    let measure =
                        &mut *(lparam.0 as *mut windows::Win32::UI::Controls::MEASUREITEMSTRUCT);
                    if measure.CtlID == MGR_LIST as u32 {
                        measure.itemHeight = s(30) as u32;
                        return LRESULT(1);
                    }
                    LRESULT(0)
                }
                WM_MGR_TEMPLATE_OPEN => {
                    // Posted by the New flyout commit: opens the editor
                    // outside the popup teardown chain.
                    *crate::runtime::lock(&PENDING_TEMPLATE) = wparam.0;
                    begin_edit(-1, Vec::new());
                    show_editor();
                    LRESULT(0)
                }
                WM_COMMAND => {
                    let code = wparam.0 & 0xFFFF;
                    let hi = ((wparam.0 >> 16) & 0xFFFF) as u16;
                    match code {
                        MGR_NEW if hi == BN_CLICKED => {
                            crate::log_line("automation: new rule menu");
                            show_new_menu(hwnd);
                        }
                        // The template flyout commits through the choice
                        // protocol; open the editor from a posted message so
                        // it never runs inside the popup's teardown chain.
                        MGR_NEW if hi == CBN_SELCHANGE => {
                            let button = get_dlg_item(hwnd, MGR_NEW);
                            if let Ok(index) = crate::choice::value(button).parse::<usize>() {
                                let _ = PostMessageW(
                                    Some(hwnd),
                                    WM_MGR_TEMPLATE_OPEN,
                                    WPARAM(index),
                                    LPARAM(0),
                                );
                            }
                        }
                        MGR_EDIT if hi == BN_CLICKED => edit_selected(),
                        MGR_DELETE if hi == BN_CLICKED => delete_selected(hwnd),
                        MGR_TOGGLE if hi == BN_CLICKED => toggle_selected(),
                        MGR_LIST if hi == LBN_DBLCLK => edit_selected(),
                        MGR_LIST if hi == LBN_SELCHANGE => {
                            let rules = crate::runtime::lock(&MGR_DISPLAYED_RULES).clone();
                            let sel = selected_index().map(|s| s as isize).unwrap_or(-1);
                            update_manager_actions(hwnd, &rules, sel);
                        }
                        _ => {}
                    }
                    LRESULT(0)
                }
                windows::Win32::UI::WindowsAndMessaging::WM_NCHITTEST => {
                    // Painted scrollbar lanes take pointer input on the form:
                    // report HTCLIENT so mouse messages reach lane_pointer.
                    if crate::list_style::lane_hit(hwnd, lparam) {
                        return LRESULT(1); // HTCLIENT
                    }
                    // Shared blank drag (nativeform::blank_drag_hit): a
                    // HTCLIENT point no interactive child claims drags the
                    // window; never WindowFromPoint from inside a hit-test
                    // (its probe re-enters the caller and recurses).
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
                windows::Win32::UI::WindowsAndMessaging::WM_NCRBUTTONUP => LRESULT(0),
                WM_CLOSE => {
                    // Esc targets the root window; with a draft open it must
                    // cancel the pane, not close the whole manager.
                    let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
                    if !ed.is_invalid() && IsWindowVisible(ed).as_bool() {
                        let _ = SendMessageW(ed, WM_CLOSE, None, None);
                    } else {
                        hide();
                    }
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
                    // Two ink tiers (settings parity): titles primary, status
                    // and empty-state prose muted. The empty-state overlay
                    // paints on the Surface card.
                    let palette = theme::palette();
                    let id = GetWindowLongPtrW(HWND(lparam.0 as *mut _), GWL_ID) as usize;
                    let muted = matches!(id, MGR_EMPTY_BODY | MGR_NEXT);
                    let on_surface = matches!(id, MGR_EMPTY_TITLE | MGR_EMPTY_BODY);
                    let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                    let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                        hdc,
                        COLORREF(if muted { palette.muted } else { palette.text }),
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
                WM_DESTROY => {
                    MGR_HWND.store(0, Ordering::SeqCst);
                    MGR_LIST_HWND.store(0, Ordering::SeqCst);
                    // The pane is a child window and dies with the manager;
                    // clear its slot so lazy creation can run again.
                    EDIT_HWND.store(0, Ordering::SeqCst);
                    crate::runtime::lock(&MGR_DISPLAYED_RULES).clear();
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
                        crate::hwnd(&crate::PANEL),
                        true,
                    );
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        },
    )
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
        // Card surface behind the listbox: elevated fill + border, plus the
        // painted scrollbar lane beside the list (no bar window - the lane
        // shares this card's paint pass with the scroll that drives it).
        crate::paint::draw_surface(
            dc,
            bounds,
            p.window_bg,
            p.surface,
            p.border,
            crate::paint::control_radius(),
        );
        unsafe {
            crate::list_style::paint_lane(
                dc,
                item.control,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
            );
        }
    } else if id == MGR_LIST {
        // Owner-draw rule rows, family selection grammar: the selected row
        // carries the nav accent bar and heavier ink (the system highlight
        // was the last non-family color in the app).
        let selected = item.state & crate::nativeform::ODS_SELECTED != 0;
        let focused = item.state & crate::nativeform::ODS_FOCUS != 0;
        let len = unsafe {
            SendMessageW(
                item.control,
                windows::Win32::UI::WindowsAndMessaging::LB_GETTEXTLEN,
                Some(WPARAM(item.item_id as usize)),
                None,
            )
        }
        .0
        .max(0) as usize;
        let mut text = String::new();
        if len > 0 {
            let mut buffer = vec![0u16; len + 1];
            let copied = unsafe {
                SendMessageW(
                    item.control,
                    windows::Win32::UI::WindowsAndMessaging::LB_GETTEXT,
                    Some(WPARAM(item.item_id as usize)),
                    Some(LPARAM(buffer.as_mut_ptr() as isize)),
                )
            }
            .0
            .max(0) as usize;
            text = String::from_utf16_lossy(&buffer[..copied.min(len)]);
        }
        // The buffered blit rewrites the whole row rect from an uninitialized
        // memory bitmap: seed the card face first or unpainted pixels leak as
        // black bands (same rule as the picker rows).
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
            match crate::paint::fill_rounded_rect(dc, &marker, bar_w.max(1), p.accent, p.accent) {
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
            if selected {
                section_font_cached()
            } else {
                form_font_body()
            },
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
        // The New button hosts the template flyout and renders its closed
        // state like every other dropdown.
        let state = crate::nativeform::control_state(item.control, item.state);
        crate::choice::draw_button(item.control, dc, bounds, state, p.window_bg);
    } else {
        let label = window_text(item.control);
        let state = crate::nativeform::control_state(item.control, item.state);
        // Deleting a rule is the manager's one destructive action: quiet
        // danger ink at rest, the filled danger style on approach.
        if id == MGR_DELETE {
            crate::paint::draw_button_danger(
                dc,
                bounds,
                form_font_body(),
                &label,
                p,
                p.window_bg,
                state,
                crate::paint::control_radius(),
            );
        } else {
            crate::paint::draw_button(
                dc,
                bounds,
                form_font_body(),
                &label,
                p,
                p.window_bg,
                state,
                crate::paint::control_radius(),
            );
        }
    }
}

// ===== Time edit subclass (Go time_edit.go) ================================

const TIME_EDIT_SUBCLASS: usize = 0x49545445;
fn install_time_edit(edit: HWND) {
    unsafe {
        let _ = windows::Win32::UI::Shell::SetWindowSubclass(
            edit,
            Some(time_edit_proc),
            TIME_EDIT_SUBCLASS,
            0,
        );
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
    let value = window_text(hwnd);
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
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_NCDESTROY => {
            unsafe {
                let _ = windows::Win32::UI::Shell::RemoveWindowSubclass(
                    hwnd,
                    Some(time_edit_proc),
                    TIME_EDIT_SUBCLASS,
                );
            }
            unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        0x0102 if wparam.0 >= 0x20 => {
            // WM_CHAR: block anything outside the time grammar.
            if time_edit_rune(wparam.0 as u32).is_none() {
                return LRESULT(0);
            }
            let result =
                unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam) };
            time_edit_normalize(hwnd, false);
            result
        }
        0x0302 => {
            // WM_PASTE: normalize whatever landed.
            let result =
                unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam) };
            time_edit_normalize(hwnd, false);
            result
        }
        0x0008 => {
            // WM_KILLFOCUS: pad the hour and settle the format.
            time_edit_normalize(hwnd, true);
            unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        _ => unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}

// ===== Rules persistence ====================================================

fn save_rules_to_config(base: &[auto::Rule], rules: Vec<auto::Rule>) -> Result<(), String> {
    crate::save_automation_rules(base, &rules)?;
    refresh_list();
    Ok(())
}
// ===== Editor ===============================================================

// ---- New-task templates ---------------------------------------------------

/// New-task recipes as prefilled partial rules. Data-driven on purpose: the
/// planned single-window editor reuses these verbatim. The blank rule stays
/// last, below a separator, as the escape hatch.
const TEMPLATE_KEYS: [&str; 5] = [
    "automation_template_process_awake",
    "automation_template_night_shutdown",
    "automation_template_work_awake",
    "automation_template_low_battery",
    "automation_template_blank",
];
const TEMPLATE_BLANK: usize = 4;
static PENDING_TEMPLATE: Mutex<usize> = Mutex::new(TEMPLATE_BLANK);

fn template_rule(index: usize) -> auto::Rule {
    let mut rule = default_rule();
    match index {
        0 => {
            rule.name = t_pub("automation_template_process_awake");
            rule.action = auto::ACTION_STAY_AWAKE.into();
            rule.trigger = auto::TRIGGER_PROCESS_RUNNING.into();
        }
        1 => {
            rule.name = t_pub("automation_template_night_shutdown");
            rule.action = auto::ACTION_SHUTDOWN.into();
            rule.trigger = auto::TRIGGER_DAILY.into();
            rule.time = "23:30".into();
            rule.warning_seconds = 60;
        }
        2 => {
            rule.name = t_pub("automation_template_work_awake");
            rule.action = auto::ACTION_STAY_AWAKE.into();
            rule.trigger = auto::TRIGGER_TIME_WINDOW.into();
            rule.time = "09:00".into();
            rule.end_time = "18:00".into();
            rule.days = ["mon", "tue", "wed", "thu", "fri"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        }
        3 => {
            rule.name = t_pub("automation_template_low_battery");
            rule.action = auto::ACTION_HIBERNATE.into();
            rule.trigger = auto::TRIGGER_BATTERY_BELOW.into();
            rule.battery_level = 20;
            rule.warning_seconds = 60;
        }
        _ => {}
    }
    rule
}

/// The New button opens the template list in the same choice flyout the
/// dropdowns and quick menu use (panel grammar, no native menu chrome).
fn show_new_menu(owner: HWND) {
    {
        let button = get_dlg_item(owner, MGR_NEW);
        if button.is_invalid() {
            *crate::runtime::lock(&PENDING_TEMPLATE) = TEMPLATE_BLANK;
            begin_edit(-1, Vec::new());
            show_editor();
            return;
        }
        let items: Vec<(String, String, bool)> = TEMPLATE_KEYS
            .iter()
            .enumerate()
            .map(|(index, key)| (index.to_string(), t_pub(key), false))
            .collect();
        // Atomic menu registration: caption kept, caret suppressed, no
        // intermediate repaint with the dropdown look.
        crate::choice::set_menu_items(button, &items);
        crate::choice::set_prefer_above(button, true);
        crate::choice::select_index(button, -1);
        crate::choice::toggle(button, owner, MGR_NEW as i32);
    }
}

/// One window, two views: the rule list hides while a draft is open and the
/// editor pane covers the client area (and vice versa).
fn set_list_view_visible(visible: bool) {
    unsafe {
        let mgr = HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _);
        for id in [
            MGR_TITLE,
            MGR_LIST_SURFACE,
            MGR_LIST,
            MGR_EMPTY_TITLE,
            MGR_EMPTY_BODY,
            MGR_NEXT,
            MGR_NEW,
            MGR_EDIT,
            MGR_DELETE,
            MGR_TOGGLE,
        ] {
            let control = get_dlg_item(mgr, id);
            if !control.is_invalid() {
                let _ = ShowWindow(control, if visible { SW_SHOW } else { SW_HIDE });
            }
        }
    }
}

/// Resizes the manager to a client height in logical pixels, keeping the
/// window centered on its panel owner and clamped to the work area.
unsafe fn resize_manager(client_h: i32) {
    unsafe {
        let mgr = HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _);
        if mgr.is_invalid() {
            return;
        }
        let style = secondary_style();
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: s(MGR_W),
            bottom: s(client_h),
        };
        let _ = AdjustWindowRectEx(&mut frame, style, false, WINDOW_EX_STYLE(0));
        let width = frame.right - frame.left;
        let height = frame.bottom - frame.top;
        let (x, y) = center_on_parent(crate::hwnd(&crate::PANEL), width, height);
        let _ = SetWindowPos(
            mgr,
            None,
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        if !ed.is_invalid() {
            // The pane tracks the client area at the new scale.
            let mut client = RECT::default();
            if GetClientRect(mgr, &mut client).is_ok() {
                let _ = SetWindowPos(
                    ed,
                    None,
                    0,
                    0,
                    client.right,
                    client.bottom,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }
}

/// Latest editor content height (logical), set by layout_editor.
static EDITOR_CONTENT_H: AtomicIsize = AtomicIsize::new(0);

/// After a layout pass, grow the manager window so the open pane shows its
/// full footer; the viewport still scrolls anything taller than the work
/// area allows.
fn sync_manager_height() {
    let content = EDITOR_CONTENT_H.load(Ordering::SeqCst) as i32;
    if content <= 0 {
        return;
    }
    unsafe {
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        if ed.is_invalid() || !IsWindowVisible(ed).as_bool() {
            return;
        }
        resize_manager(content.max(MGR_H));
    }
}

fn show_editor() {
    let _dpi = crate::dpi::Scope::window(HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _));
    unsafe {
        if HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _).0 as usize == 0 {
            create_editor();
        }
        refresh_tooltips();
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        populate_editor();
        theme::retheme_children(ed);
        set_list_view_visible(false);
        // Swap views and size the window to the form in one pass.
        let _ = ShowWindow(ed, SW_SHOW);
        sync_manager_height();
        let _ = UpdateWindow(ed);
        // No default focus into the name field: the empty-field hint only
        // shows while the edit is unfocused, and opening straight into it
        // hid the hint until the user clicked elsewhere. The pane itself
        // takes focus so keyboard state is clean (the list behind it is
        // hidden at this point).
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(ed));
    }
}

fn hide_editor() {
    unsafe {
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        let _ = ShowWindow(ed, SW_HIDE);
        set_list_view_visible(true);
        refresh_list();
        // Back to the compact list view.
        resize_manager(MGR_H);
        let mgr = HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _);
        // Go focuses the manager list after save/cancel (keyboard stays
        // live). An empty rule list hides the listbox; focus New instead of
        // silently dropping the focus onto a hidden window.
        let list = HWND(MGR_LIST_HWND.load(Ordering::SeqCst) as *mut _);
        if !list.is_invalid() && IsWindowVisible(list).as_bool() {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(list));
        } else {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(get_dlg_item(
                mgr, MGR_NEW,
            )));
        }
    }
}

fn create_editor() {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module handle");
        let font = form_font_body();
        let section_font = section_font_cached();

        register_editor_class(instance);

        // Overlay pane: a full-client child of the manager shown while a
        // draft is open (the list view hides underneath). It keeps its own
        // class and proc, so all editor message handling stays unchanged.
        let ed = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("IdleTriggerAutoEdit"),
            PCWSTR(wide(&t_pub("automation_new_title")).as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_CLIPSIBLINGS.0),
            0,
            0,
            s(ED_PANE_W),
            s(MGR_H),
            Some(HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _)),
            None,
            Some(instance.into()),
            None,
        )
        .expect("editor pane");
        EDIT_HWND.store(ed.0 as isize, Ordering::SeqCst);
        theme::apply_to_window(ed);

        // Section cards first (settings card grammar): owner-draw statics
        // pinned beneath every row they host; layout_editor sizes them per
        // trigger/action mode. Blank-surface hit transparency keeps the
        // pane's blank drag alive over them.
        for card_id in [ED_CARD_BASIC, ED_CARD_TRIGGER, ED_CARD_OPTIONS] {
            let card = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0 | 13), // SS_OWNERDRAW
                0,
                0,
                1,
                1,
                Some(ed),
                Some(HMENU(card_id as *mut _)),
                Some(instance.into()),
                None,
            )
            .expect("editor section card");
            let _ = SetWindowPos(
                card,
                Some(windows::Win32::Foundation::HWND(1 as *mut _)), // HWND_BOTTOM
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }

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
            // Settings field-well architecture: a bare inset EDIT; the
            // ring/margin is painted by the hosting section card (see the
            // editor card arm), so nothing stacks against the card.
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
        mk_label(ED_MODE_TITLE, &t_pub("automation_new_title"), section_font);
        mk_label(
            ED_BASIC_TITLE,
            &t_pub(caption_key(EDITOR_TEXTS, ED_BASIC_TITLE)),
            section_font,
        );
        mk_label(
            ED_TRIGGER_TITLE,
            &t_pub(caption_key(EDITOR_TEXTS, ED_TRIGGER_TITLE)),
            section_font,
        );
        mk_label(
            ED_OPTIONS_TITLE,
            &t_pub(caption_key(EDITOR_TEXTS, ED_OPTIONS_TITLE)),
            section_font,
        );
        mk_label(
            ED_NAME_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_NAME_LBL)),
            font,
        );
        mk_label(
            ED_ACTION_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_ACTION_LBL)),
            font,
        );
        mk_label(
            ED_TRIGGER_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_TRIGGER_LBL)),
            font,
        );
        mk_label(
            ED_TIME_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_TIME_LBL)),
            font,
        );
        mk_label(
            ED_DATE_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_DATE_LBL)),
            font,
        );
        mk_label(
            ED_END_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_END_LBL)),
            font,
        );
        mk_label(
            ED_LOGIC_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_LOGIC_LBL)),
            font,
        );
        mk_label(
            ED_WARN_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_WARN_LBL)),
            font,
        );
        mk_label(
            ED_IDLE_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_IDLE_LBL)),
            font,
        );
        mk_label(
            ED_BATTERY_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_BATTERY_LBL)),
            font,
        );
        mk_label(
            ED_MAX_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_MAX_LBL)),
            font,
        );
        mk_label(
            ED_DAYS_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_DAYS_LBL)),
            font,
        );
        mk_label(
            ED_BLOCKED_LBL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_BLOCKED_LBL)),
            font,
        );
        mk_label(
            ED_NAME_HINT,
            &t_pub(caption_key(EDITOR_TEXTS, ED_NAME_HINT)),
            font,
        );
        mk_label(ED_PROC_SUMMARY, "", font);
        mk_label(
            ED_NO_OPTIONS,
            &t_pub(caption_key(EDITOR_TEXTS, ED_NO_OPTIONS)),
            font,
        );
        mk_label(ED_VALIDATION, &t_pub("automation_runtime_note"), font);

        // Fields (Go: date/time edits, numeric edits).
        let name_edit = mk_edit(ED_NAME, false);
        crate::nativeform::cue_banner(name_edit, "automation_name_placeholder");
        install_time_edit(mk_edit(ED_TIME, false));
        install_time_edit(mk_edit(ED_END_TIME, false));
        mk_edit(ED_DATE, false);
        mk_edit(ED_WARNING, true);
        mk_edit(ED_IDLE_MIN, true);
        mk_edit(ED_MAX_WAIT, true);
        mk_edit(ED_BATTERY, true);

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

        mk_button(
            ED_CHOOSE,
            &t_pub(caption_key(EDITOR_TEXTS, ED_CHOOSE)),
            ED_FIELD_H,
        );
        mk_button(
            ED_DAYS_WORKDAYS,
            &t_pub(caption_key(EDITOR_TEXTS, ED_DAYS_WORKDAYS)),
            24,
        );
        mk_button(
            ED_DAYS_EVERYDAY,
            &t_pub(caption_key(EDITOR_TEXTS, ED_DAYS_EVERYDAY)),
            24,
        );
        let keep_screen = mk_button(
            ED_KEEP_SCREEN,
            &t_pub(caption_key(EDITOR_TEXTS, ED_KEEP_SCREEN)),
            ED_CHECK_H,
        );
        // Toggle rows hover on the pill column alone (panel parity).
        crate::nativeform::set_hover_column(keep_screen, s(crate::layout::SWITCH_HIT_W));
        mk_button(
            ED_PROC_INFO,
            &t_pub(caption_key(EDITOR_TEXTS, ED_PROC_INFO)),
            ED_SUMMARY_H,
        );

        for id in [
            ED_DAYS_MON,
            ED_DAYS_TUE,
            ED_DAYS_WED,
            ED_DAYS_THU,
            ED_DAYS_FRI,
            ED_DAYS_SAT,
            ED_DAYS_SUN,
        ] {
            mk_button(id, &t_pub(caption_key(EDITOR_TEXTS, id)), ED_FIELD_H);
        }

        // Footer: Save (accent) + Cancel, owner-drawn.
        mk_button(
            ED_SAVE,
            &t_pub(caption_key(EDITOR_TEXTS, ED_SAVE)),
            BUTTON_H,
        );
        mk_button(
            ED_CANCEL,
            &t_pub(caption_key(EDITOR_TEXTS, ED_CANCEL)),
            BUTTON_H,
        );
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
        action: auto::ACTION_STAY_AWAKE.into(),
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
        process_logic: auto::LOGIC_ANY.into(),
        processes: Vec::new(),
        keep_screen_on: false,
        idle_minutes: auto::DEFAULT_IDLE_MINUTES,
        warning_seconds: auto::DEFAULT_WARNING_SECONDS,
        blocked_policy: auto::BLOCKED_SKIP.into(),
        max_wait_minutes: 60,
        battery_level: 0,
    }
}

fn populate_editor() {
    let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
    {
        let idx = edit_index();
        let rules = if idx >= 0 {
            crate::runtime::lock(&MGR_DISPLAYED_RULES).clone()
        } else {
            crate::runtime::lock(&crate::automation::RULES).clone()
        };
        let rule = if idx >= 0 && (idx as usize) < rules.len() {
            Some(rules[idx as usize].clone())
        } else {
            None
        };
        let rule = rule.unwrap_or_else(|| {
            let template = std::mem::replace(
                &mut *crate::runtime::lock(&PENDING_TEMPLATE),
                TEMPLATE_BLANK,
            );
            template_rule(template)
        });
        complete_edit(idx, rules, rule.clone());

        // The pane has no caption bar: the mode title row labels new/edit.
        let title = if idx >= 0 {
            t_pub("automation_edit_title")
        } else {
            t_pub("automation_new_title")
        };
        set_text(get_dlg_item(ed, ED_MODE_TITLE), &title);

        set_text(get_dlg_item(ed, ED_NAME), &rule.name);
        set_text(get_dlg_item(ed, ED_DATE), &rule.date);
        set_text(get_dlg_item(ed, ED_TIME), &rule.time);
        set_text(get_dlg_item(ed, ED_END_TIME), &rule.end_time);
        set_text(
            get_dlg_item(ed, ED_IDLE_MIN),
            &rule.idle_minutes.to_string(),
        );
        set_text(
            get_dlg_item(ed, ED_BATTERY),
            &rule.battery_level.to_string(),
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
        let issues = crate::runtime::lock(&crate::automation::ISSUES).clone();
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
    draft.name = clean_single_line(&window_text(get_dlg_item(ed, ED_NAME)));
    draft.action = choice_value(ed, ED_ACTION);
    draft.trigger = choice_value(ed, ED_TRIGGER);
    draft.process_logic = choice_value(ed, ED_LOGIC);
    draft.blocked_policy = choice_value(ed, ED_BLOCKED);
    draft.date = clean_single_line(&window_text(get_dlg_item(ed, ED_DATE)));
    draft.time = clean_single_line(&window_text(get_dlg_item(ed, ED_TIME)));
    draft.end_time = clean_single_line(&window_text(get_dlg_item(ed, ED_END_TIME)));
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
    draft.processes = edit_procs();
    draft.idle_minutes = window_text(get_dlg_item(ed, ED_IDLE_MIN))
        .trim()
        .parse()
        .unwrap_or(0);
    draft.warning_seconds = window_text(get_dlg_item(ed, ED_WARNING))
        .trim()
        .parse()
        .unwrap_or(0);
    draft.max_wait_minutes = window_text(get_dlg_item(ed, ED_MAX_WAIT))
        .trim()
        .parse()
        .unwrap_or(0);
    draft.battery_level = window_text(get_dlg_item(ed, ED_BATTERY))
        .trim()
        .parse()
        .unwrap_or(0);
    draft
}

/// Go validateDraft: returns the first error message (with its control).
fn validate_draft(draft: &auto::Rule) -> Option<(usize, String)> {
    let t = |key: &str| t_pub(key);
    let trigger = draft.trigger.as_str();
    let needs_process = matches!(
        trigger,
        "process_running" | "process_started" | "process_exited"
    );
    if needs_process && draft.processes.is_empty() {
        return Some((ED_CHOOSE, t("automation_error_process_required")));
    }
    if trigger == auto::TRIGGER_ONCE && !parse_date(&draft.date) {
        return Some((ED_DATE, t("automation_error_date")));
    }
    if matches!(
        trigger,
        auto::TRIGGER_ONCE | auto::TRIGGER_DAILY | auto::TRIGGER_WEEKLY | auto::TRIGGER_TIME_WINDOW
    ) && !parse_time(&draft.time)
    {
        return Some((ED_TIME, t("automation_error_time")));
    }
    if trigger == auto::TRIGGER_TIME_WINDOW && !parse_time(&draft.end_time) {
        return Some((ED_END_TIME, t("automation_error_end_time")));
    }
    if matches!(trigger, auto::TRIGGER_WEEKLY | auto::TRIGGER_TIME_WINDOW) && draft.days.is_empty()
    {
        return Some((ED_DAYS_MON, t("automation_error_days")));
    }
    if draft.action == "enable_idle_monitor"
        && (draft.idle_minutes <= 0 || draft.idle_minutes > auto::MAX_IDLE_MINUTES)
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
        && draft.blocked_policy == auto::BLOCKED_WAIT
        && (draft.max_wait_minutes <= 0 || draft.max_wait_minutes > auto::MAX_IDLE_MINUTES)
    {
        return Some((ED_MAX_WAIT, t("automation_error_max_wait")));
    }
    None
}

fn parse_date(value: &str) -> bool {
    auto::valid_date(value)
}

fn parse_time(value: &str) -> bool {
    auto::valid_hhmm(value)
}

/// Go setEditorError: show the message in the validation row and focus the field.
fn set_editor_error(ed: HWND, id: usize, message: &str) {
    EDIT_ERROR.store(true, Ordering::SeqCst);
    set_text(get_dlg_item(ed, ED_VALIDATION), message);
    let control = get_dlg_item(ed, id);
    if !control.is_invalid() {
        unsafe {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(control));
            crate::viewport::reveal_control(ed, control);
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
    let orig = edit_orig().unwrap_or_else(default_rule);
    let draft = read_draft(ed, &orig);

    if let Some((id, message)) = validate_draft(&draft) {
        set_editor_error(ed, id, &message);
        return;
    }

    let mut draft = draft;
    if draft.name.is_empty() {
        draft.name = rule_summary(&draft);
    }

    let (mut normalized, issues) = auto::prepare_rules(std::slice::from_ref(&draft));
    if let Some(issue) = issues.first() {
        set_editor_error(ed, ED_SAVE, &issue.message);
        return;
    }
    let draft = normalized.remove(0);

    // One lock: index and snapshot must describe the same session.
    let (idx, base) = crate::runtime::lock(&EDIT_SESSION)
        .as_ref()
        .map_or((-1, Vec::new()), |s| (s.index, s.base_rules.clone()));
    let mut candidate = base.clone();
    if idx >= 0 && (idx as usize) < candidate.len() {
        candidate[idx as usize] = draft.clone();
    } else {
        candidate.push(draft.clone());
    }
    if let Err(err) = save_rules_to_config(&base, candidate) {
        set_editor_error(ed, ED_SAVE, &err);
        return;
    }
    hide_editor();
}

/// Go cancelEditor: confirm when unsaved work would be lost.
fn cancel_editor(ed: HWND) {
    let orig = edit_orig();
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
    let mut current = current.clone();
    let mut orig = orig.clone();
    current.days = auto::normalize_days(&current.days);
    orig.days = auto::normalize_days(&orig.days);
    current.processes = auto::normalize_targets(current.processes);
    orig.processes = auto::normalize_targets(orig.processes);
    if current == orig {
        return false;
    }
    if edit_index() >= 0 {
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
    intent(&current) != intent(&orig)
}

/// Repositions and shows/hides editor controls per the current trigger and
/// action selections, then resizes the window (Go layoutEditorContent).
pub fn layout_editor() {
    unsafe {
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        if ed.is_invalid() {
            return;
        }
        let _dpi = crate::dpi::Scope::window(ed);
        crate::viewport::begin_layout(ed);
        lower_surfaces(ed);
        let content_w = ED_W - 2 * ED_PAD;
        let column_w = (content_w - ED_GAP) / 2;
        let trigger = choice_value(ed, ED_TRIGGER);
        let action = choice_value(ed, ED_ACTION);

        let mut visible = Vec::new();

        let mut place = |id: usize, x: i32, y: i32, w: i32, h: i32| {
            let control = get_dlg_item(ed, id);
            if control.is_invalid() {
                return;
            }
            // Field edits sit inset inside their card-painted well
            // (settings parity: x+3, y+7, w-6, h-14).
            if is_field_edit(id) {
                let _ = SetWindowPos(
                    control,
                    None,
                    s(x + 3),
                    s(y + 7),
                    s(w - 6),
                    s(h - 14).max(20),
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
                );
            } else {
                let _ = SetWindowPos(
                    control,
                    None,
                    s(x),
                    s(y),
                    s(w),
                    s(h),
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
                );
            }
            visible.push(control);
        };

        let label_row2 = |place: &mut dyn FnMut(usize, i32, i32, i32, i32),
                          left: usize,
                          right: usize,
                          y: i32| {
            place(left, ED_PAD, y, column_w, ED_LABEL_H);
            place(right, ED_PAD + column_w + ED_GAP, y, column_w, ED_LABEL_H);
        };

        // Mode title spans the pane top; the form starts directly with the
        // name row (a second section header right below read as duplication).
        // Settings card grammar: page title, then title-outside-card
        // sections whose cards span [ED_EDGE, ED_EDGE + card_w] with rows
        // at the ED_PAD inner rhythm. Card heights close over whatever
        // rows the current trigger/action pair produced.
        let card_w = ED_W - 2 * ED_EDGE;
        let mut y = ED_EDGE;
        place(ED_MODE_TITLE, ED_EDGE, y, card_w, ED_LABEL_H);
        y += ED_LABEL_H + ED_SECTION_GAP;

        // 基本信息.
        place(ED_BASIC_TITLE, ED_EDGE, y, card_w, ED_LABEL_H);
        y += ED_LABEL_H + crate::layout::TITLE_GAP;
        let basic_top = y;
        y += crate::layout::CARD_PAD_Y;
        place(ED_NAME_LBL, ED_PAD, y, content_w, ED_LABEL_H);
        y += ED_LABEL_H + ED_LABEL_GAP;
        place(ED_NAME, ED_PAD, y, content_w, ED_FIELD_H);
        y += ED_FIELD_H + ED_LABEL_GAP;
        place(ED_NAME_HINT, ED_PAD, y, content_w, ED_LABEL_H);
        y += ED_LABEL_H + ED_CONTENT_GAP;
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
        y += ED_FIELD_H + crate::layout::CARD_PAD_Y;
        place(ED_CARD_BASIC, ED_EDGE, basic_top, card_w, y - basic_top);
        y += crate::layout::CARD_GAP;

        // 触发条件.
        place(ED_TRIGGER_TITLE, ED_EDGE, y, card_w, ED_LABEL_H);
        y += ED_LABEL_H + crate::layout::TITLE_GAP;
        let trigger_top = y;
        y += crate::layout::CARD_PAD_Y;
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
            auto::TRIGGER_ONCE => row_fields(
                &mut place,
                ED_DATE_LBL,
                ED_DATE,
                ED_TIME_LBL,
                ED_TIME,
                &mut y,
            ),
            auto::TRIGGER_DAILY => {
                place(ED_TIME_LBL, ED_PAD, y, column_w, ED_LABEL_H);
                y += ED_LABEL_H + ED_LABEL_GAP;
                place(ED_TIME, ED_PAD, y, column_w, ED_FIELD_H);
                y += ED_FIELD_H + ED_RELATED_GAP;
            }
            auto::TRIGGER_WEEKLY => {
                place(ED_TIME_LBL, ED_PAD, y, column_w, ED_LABEL_H);
                y += ED_LABEL_H + ED_LABEL_GAP;
                place(ED_TIME, ED_PAD, y, column_w, ED_FIELD_H);
                y += ED_FIELD_H + ED_RELATED_GAP;
                y = layout_weekdays(&mut place, y, content_w);
            }
            auto::TRIGGER_TIME_WINDOW => {
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
            auto::TRIGGER_BATTERY_BELOW => {
                set_text(
                    get_dlg_item(ed, ED_BATTERY_LBL),
                    &t_pub("automation_battery_level"),
                );
                place(ED_BATTERY_LBL, ED_PAD, y, column_w, ED_LABEL_H);
                y += ED_LABEL_H + ED_LABEL_GAP;
                place(ED_BATTERY, ED_PAD, y, column_w, ED_FIELD_H);
                y += ED_FIELD_H + ED_RELATED_GAP;
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
                choice_select(ed, ED_LOGIC, auto::LOGIC_NONE);
                place(ED_CHOOSE, ED_PAD, y, content_w, ED_FIELD_H);
            }
            "process_started" => {
                choice_select(ed, ED_LOGIC, auto::LOGIC_ANY);
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
        let has_procs = !edit_procs().is_empty();
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
        y += ED_SUMMARY_H + crate::layout::CARD_PAD_Y;
        place(
            ED_CARD_TRIGGER,
            ED_EDGE,
            trigger_top,
            card_w,
            y - trigger_top,
        );
        y += crate::layout::CARD_GAP;

        // 执行选项.
        place(ED_OPTIONS_TITLE, ED_EDGE, y, card_w, ED_LABEL_H);
        y += ED_LABEL_H + crate::layout::TITLE_GAP;
        let options_top = y;
        y += crate::layout::CARD_PAD_Y;
        match action.as_str() {
            auto::ACTION_STAY_AWAKE => {
                // Full row width: the switch row parks its pill at the right
                // edge, so a checkbox-width control would slide the pill
                // over its own label.
                place(ED_KEEP_SCREEN, ED_PAD, y, content_w, ED_CHECK_H);
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
                    let waiting = choice_value(ed, ED_BLOCKED) == auto::BLOCKED_WAIT;
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

        // Status row + footer follow the flow: the manager window grows to
        // the content height, and anything the work area cannot fit scrolls
        // via fit_content.
        y += crate::layout::CARD_PAD_Y;
        place(
            ED_CARD_OPTIONS,
            ED_EDGE,
            options_top,
            card_w,
            y - options_top,
        );
        y += ED_SECTION_GAP;
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
        let content_bottom = y + BUTTON_H + ED_EDGE;

        // Apply only final visibility; existing fields never hide and reappear.
        for id in ED_LAYOUT_IDS {
            let control = get_dlg_item(ed, id);
            crate::nativeform::set_visible_deferred(control, visible.contains(&control));
        }
        // Grow the window first, then record the content extent against the
        // final client size: recording against the old (short) pane would
        // arm scrollbars for a size that no longer exists. The width keeps a
        // scrollbar margin so a vertical bar cannot phantom-trigger the
        // horizontal one.
        EDITOR_CONTENT_H.store(content_bottom as isize, Ordering::SeqCst);
        sync_manager_height();
        crate::viewport::fit_content(ed, ED_W - ED_EDGE, content_bottom);
        if IsWindowVisible(ed).as_bool() {
            crate::present_layout(ed);
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
    const QUICK_H: i32 = 24;
    let gap = ED_GAP;
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
    // The quick pair rides the SAT/SUN columns of the day row below: same
    // width, same x, so the two rows read as one grid.
    let quick_sat_x = ED_PAD + 5 * (button_w + gap);
    let quick_sun_x = ED_PAD + 6 * (button_w + gap);
    place(
        ED_DAYS_LBL,
        ED_PAD,
        y + (QUICK_H - ED_LABEL_H) / 2,
        quick_sat_x - gap - ED_PAD,
        ED_LABEL_H,
    );
    place(ED_DAYS_WORKDAYS, quick_sat_x, y, button_w, QUICK_H);
    place(ED_DAYS_EVERYDAY, quick_sun_x, y, button_w, QUICK_H);
    y += QUICK_H + ED_RELATED_GAP;
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
    let key = if trigger == auto::TRIGGER_TIME_WINDOW {
        "automation_time"
    } else {
        "automation_execution_time"
    };
    t_pub(key)
}

/// Refreshes the selected-processes summary line (Go idProcessSummary).
fn update_proc_summary() {
    let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
    let count = edit_procs().len();
    let text = if count == 0 {
        t_pub("automation_no_processes")
    } else {
        crate::t_args("automation_process_count", &[&count.to_string()])
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

/// Whether an id is one of the editor's numeric/text EDIT fields (they get
/// the card-painted well treatment; settings field grammar).
fn is_field_edit(edit_id: usize) -> bool {
    matches!(
        edit_id,
        ED_NAME
            | ED_TIME
            | ED_END_TIME
            | ED_DATE
            | ED_WARNING
            | ED_IDLE_MIN
            | ED_MAX_WAIT
            | ED_BATTERY
    )
}

/// Scoped invalidation of one field's card-painted well (settings parity):
/// full-card repaints erase sibling pixels, so only the well rect goes.
fn invalidate_editor_well(ed: HWND, edit_id: usize) {
    let edit = get_dlg_item(ed, edit_id);
    if edit.is_invalid() {
        return;
    }
    for card_id in [ED_CARD_BASIC, ED_CARD_TRIGGER, ED_CARD_OPTIONS] {
        let card = get_dlg_item(ed, card_id);
        if !card.is_invalid() && crate::nativeform::invalidate_field_well(card, edit) {
            return;
        }
    }
}

/// Process details text for the info glyph (Go processDetails).
fn process_details() -> String {
    let targets = edit_procs();
    let mut lines = Vec::new();
    for target in &targets {
        let name = described_target(target);
        if !target.path.is_empty() && target.kind == auto::MATCH_PATH {
            lines.push(fill_template(
                &t_pub("automation_process_detail_path"),
                &[&name, &target.path],
            ));
            continue;
        }
        lines.push(fill_template(
            &t_pub("automation_process_detail_name"),
            &[&name],
        ));
    }
    lines.join("\n")
}

#[cfg(all(test, feature = "devtools"))]
pub(crate) fn test_process_details_fixture() -> String {
    set_edit_procs(vec![auto::ProcessTarget {
        kind: auto::MATCH_NAME.into(),
        executable: "IdleTrigger-audit.exe".into(),
        path: String::new(),
    }]);
    process_details()
}

unsafe extern "system" fn ed_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    crate::guarded_proc(
        "automation-editor",
        hwnd,
        msg,
        wparam,
        lparam,
        move || unsafe {
            match msg {
                windows::Win32::UI::WindowsAndMessaging::WM_NCHITTEST => {
                    // The editor pane is a full-client CHILD of the manager:
                    // its own blank surface must not claim the mouse (the
                    // manager's blank drag would die while a draft is open),
                    // while its controls keep answering for themselves.
                    // Latched probe (nativeform::pane_blank_transparent):
                    // no re-entrant recursion.
                    crate::nativeform::pane_blank_transparent(hwnd, msg, wparam, lparam)
                }
                WM_COMMAND => {
                    let code = wparam.0 & 0xFFFF;
                    let hi = ((wparam.0 >> 16) & 0xFFFF) as u16;
                    match code {
                        ED_SAVE if hi == BN_CLICKED => save_rule(hwnd),
                        ED_CANCEL if hi == BN_CLICKED => cancel_editor(hwnd),
                        ED_CHOOSE if hi == BN_CLICKED => show_picker(hwnd),

                        // Checkbox toggles (owner-draw; state in EDIT_CHECKS).
                        ED_KEEP_SCREEN if hi == BN_CLICKED => {
                            let before = edit_is_checked(ED_KEEP_SCREEN);
                            edit_toggle(hwnd, ED_KEEP_SCREEN);
                            let control = get_dlg_item(hwnd, ED_KEEP_SCREEN);
                            if !control.is_invalid() {
                                crate::start_switch_animation(
                                    control,
                                    before,
                                    edit_is_checked(ED_KEEP_SCREEN),
                                );
                            }
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
                            if !edit_procs().is_empty() {
                                info_dialog(
                                    hwnd,
                                    &t_pub("automation_process_details_title"),
                                    &process_details(),
                                );
                            }
                        }

                        // Numeric edits: filter digits and clear errors (Go
                        // sanitizeNumericEdit).
                        ED_WARNING | ED_IDLE_MIN | ED_MAX_WAIT | ED_BATTERY if hi == EN_CHANGE => {
                            sanitize_numeric_edit(hwnd, code);
                            clear_editor_error(hwnd);
                        }
                        ED_NAME | ED_DATE | ED_TIME | ED_END_TIME if hi == EN_CHANGE => {
                            clear_editor_error(hwnd);
                        }

                        // Field focus repaints the card-painted well border.
                        code if (hi == EN_SETFOCUS || hi == EN_KILLFOCUS)
                            && is_field_edit(code) =>
                        {
                            invalidate_editor_well(hwnd, code);
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
                    } else if muted {
                        palette.muted
                    } else {
                        // Two ink tiers everywhere (settings parity): titles
                        // AND field labels share the primary ink; muted is
                        // reserved for real secondary prose.
                        palette.text
                    };
                    let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                    let _ = windows::Win32::Graphics::Gdi::SetTextColor(hdc, COLORREF(color));
                    // Card grammar: labels riding a section card erase with
                    // the card face; the page/section titles and the
                    // validation row sit on the pane background.
                    let on_card = !matches!(
                        id,
                        ED_MODE_TITLE
                            | ED_BASIC_TITLE
                            | ED_TRIGGER_TITLE
                            | ED_OPTIONS_TITLE
                            | ED_VALIDATION
                    );
                    let (light, dark) = theme::surface_brush_pairs();
                    let p = theme::palette();
                    let (fill, brush) = if on_card {
                        (p.surface, if theme::is_dark() { dark.0 } else { light.0 })
                    } else {
                        (theme::bg_color(), theme::bg_brush())
                    };
                    let _ = windows::Win32::Graphics::Gdi::SetBkColor(hdc, COLORREF(fill));
                    LRESULT(brush.0 as isize)
                }
                WM_CTLCOLOREDIT => {
                    // Edit interior: PrimaryText on Surface (Go surfaces).
                    let p = theme::palette();
                    let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
                    let control = HWND(lparam.0 as *mut _);
                    let disabled = !IsWindowEnabled(control).as_bool();
                    if disabled {
                        let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                            hdc,
                            COLORREF(p.disabled_text),
                        );
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
                WM_DESTROY => {
                    EDIT_HWND.store(0, Ordering::SeqCst);
                    // Hidden-reuse never reaches here; a real destroy must drop
                    // editor state so a recreated window cannot inherit stale
                    // checkbox marks on reused control ids.
                    EDIT_ERROR.store(false, Ordering::SeqCst);
                    *crate::runtime::lock(&EDIT_CHECKS) = None;
                    end_edit();
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
                        HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _),
                        true,
                    );
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        },
    )
}

/// Strips non-digits from a numeric edit and restores the caret (Go).
fn sanitize_numeric_edit(ed: HWND, id: usize) {
    let control = get_dlg_item(ed, id);
    if control.is_invalid() {
        return;
    }
    let value = window_text(control);
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
        // Section-card repaints end in ONE SRCCOPY of the whole card rect
        // with no sibling clipping (the settings blanked-field lesson):
        // cut every visible child that overlaps the card out of the TARGET
        // dc first, so the blit can only ever paint the card's own blank
        // face - labels, field surfaces and edits keep their pixels.
        if ((ED_CARD_BASIC as i32..=ED_CARD_OPTIONS as i32).contains(&item.control_id)
            || item.control_id == PK_SEARCH_SURFACE as i32)
            && let Ok(parent) = GetParent(item.control)
        {
            let mut card_rect = RECT::default();
            if GetWindowRect(item.control, &mut card_rect).is_ok() {
                let mut child = GetWindow(parent, GW_CHILD).unwrap_or_default();
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
        // Editor section cards: the settings card face (surface + family
        // hairline) plus the settings field wells - every visible EDIT
        // hosted on the card gets its ring painted as card pixels; the
        // clip guard in draw_form_item keeps this paint out of the edits.
        if (ED_CARD_BASIC as i32..=ED_CARD_OPTIONS as i32).contains(&item.control_id) {
            crate::paint::draw_surface(
                dc,
                bounds,
                p.window_bg,
                p.surface,
                p.border,
                crate::paint::control_radius(),
            );
            let Ok(parent) = GetParent(item.control) else {
                return;
            };
            let mut card_rect = RECT::default();
            if GetWindowRect(item.control, &mut card_rect).is_err() {
                return;
            }
            let mut child = GetWindow(parent, GW_CHILD).unwrap_or_default();
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
            return;
        }

        // The search field is one plain settings-style field well: a
        // single draw_field pass over the window background - the SAME
        // function, radius and stroke every other input in the app uses.
        // (A card face underneath would double the border and show the two
        // rounded edges cutting into each other as white fringing.)
        if item.control_id as usize == PK_SEARCH_SURFACE {
            let edit = GetParent(item.control)
                .ok()
                .map(|parent| get_dlg_item(parent, PK_SEARCH))
                .filter(|edit| !edit.is_invalid())
                .unwrap_or_default();
            let state = crate::paint::ControlState {
                focused: GetFocus() == edit,
                disabled: !IsWindowEnabled(edit).as_bool(),
                ..Default::default()
            };
            crate::paint::draw_field(
                dc,
                bounds,
                p,
                p.window_bg,
                p.surface,
                state,
                crate::paint::control_radius(),
                None,
            );
            return;
        }

        // Picker card surfaces (Go DrawSurface): elevated fill + border,
        // no label — never the button path.
        if matches!(
            item.control_id as usize,
            PK_LIST_SURFACE | PK_PREVIEW_SURFACE
        ) {
            crate::paint::draw_surface(
                dc,
                bounds,
                p.window_bg,
                p.surface,
                p.border,
                crate::paint::control_radius(),
            );
            // The painted scrollbar lanes ride the list and preview card
            // faces (no bar window - the lane shares this card's paint pass
            // with the scroll that drives it).
            crate::list_style::paint_lane(
                dc,
                item.control,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
            );
            return;
        }

        if item.control_id == PK_LIST as i32 {
            // Picker process rows (manager listbox grammar): family checkbox,
            // three text columns, accent-bar selection cursor. Checked state
            // lives in PK_SELECTED; the row is painted, not state-imaged.
            let selected = item.state & crate::nativeform::ODS_SELECTED != 0;
            let visible = crate::runtime::lock(&PK_VISIBLE);
            let mut name = String::new();
            let mut description = String::new();
            let mut count = String::new();
            let mut checked = false;
            if let Some(row) = visible.get(item.item_id as usize) {
                name = row.name.clone();
                description = row.description.clone();
                count = row.count.to_string();
                let selected_now = crate::runtime::lock(&PK_SELECTED);
                checked = selected_now.iter().any(|t| t.key() == row.target.key());
            }
            drop(visible);
            // Seed the row with the card face: draw_buffered blits an
            // uninitialized memory bitmap over the whole row, so any pixel
            // not painted here would come back black.
            crate::paint::fill_rect(dc, bounds, p.surface);
            // Quiet column rules at the column gaps: without them the three
            // text columns read as one run-on line (the old listview header
            // drew dividers; rows need the same hint).
            for gap in [crate::scale_pub(256), crate::scale_pub(564)] {
                let rule = RECT {
                    left: bounds.left + gap,
                    top: bounds.top,
                    right: bounds.left + gap + crate::scale_pub(1),
                    bottom: bounds.bottom,
                };
                crate::paint::fill_rect(dc, &rule, p.subtle_border);
            }
            let box_size = crate::scale_pub(ED_CHECKBOX_SIZE);
            let box_y = bounds.top + (bounds.bottom - bounds.top - box_size) / 2;
            let box_rect = RECT {
                left: bounds.left + crate::scale_pub(14),
                top: box_y,
                right: bounds.left + crate::scale_pub(14) + box_size,
                bottom: box_y + box_size,
            };
            let (fill, border) = if checked {
                (p.accent, p.accent)
            } else {
                (p.surface, p.border)
            };
            let scale96 = crate::scale_pub(96);
            let radius = crate::paint::sp(2, scale96);
            match crate::paint::fill_rounded_rect(dc, &box_rect, radius, fill, border) {
                crate::paint::DrawResult::Completed | crate::paint::DrawResult::MayBeDirty => {}
                crate::paint::DrawResult::NotStarted => {
                    crate::paint::fill_rect(dc, &box_rect, fill);
                }
            }
            if checked {
                crate::paint::draw_check(
                    dc,
                    box_rect.left,
                    box_rect.top,
                    box_rect.right,
                    box_rect.bottom,
                    p.accent_text,
                    radius.max(1),
                );
            }
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
                    // Plain-GDI fallback, manager-row parity: without it the
                    // cursor row loses its bar entirely when GDI+ is down.
                    crate::paint::DrawResult::NotStarted => {
                        crate::paint::fill_rect(dc, &marker, p.accent)
                    }
                }
            }
            let columns = [
                (
                    crate::scale_pub(40),
                    crate::scale_pub(212),
                    name.as_str(),
                    true,
                ),
                (
                    crate::scale_pub(260),
                    crate::scale_pub(300),
                    description.as_str(),
                    false,
                ),
                (
                    crate::scale_pub(568),
                    crate::scale_pub(72),
                    count.as_str(),
                    false,
                ),
            ];
            for (x, width, text, primary) in columns {
                let cell = RECT {
                    left: bounds.left + x,
                    top: bounds.top,
                    right: bounds.left + x + width,
                    bottom: bounds.bottom,
                };
                crate::paint::draw_label(
                    dc,
                    &cell,
                    if primary && selected {
                        section_font_cached()
                    } else {
                        form_font_body()
                    },
                    text,
                    if primary { p.text } else { p.muted },
                    true,
                    0,
                    0,
                );
            }
            // No row-wide focus frame here: it crossed the accent bar at the
            // left edge and ran under the overlay scrollbar at the right; the
            // accent bar already tracks the cursor row (mouse and keyboard).
            return;
        }
        // Choice buttons render their closed state via the choice module;
        // every editor choice rides a section card, so their faces erase
        // with the card color.
        if crate::choice::is_choice(item.control) {
            let state = crate::nativeform::control_state(item.control, item.state);
            crate::choice::draw_button(item.control, dc, bounds, state, p.surface);
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
            // The editor's one toggle rides the panel's switch-row grammar:
            // label left, pill right, whole row is the hit target. Hover
            // belongs to the pill column alone (panel parity).
            let mut state = crate::nativeform::control_state(item.control, item.state);
            state.active = edit_is_checked(ED_KEEP_SCREEN);
            let progress = crate::switch_animation_progress(item.control);
            crate::paint::draw_switch_row(
                dc, bounds, font, &label, p, p.surface, state, scale, progress,
            );
        } else if item.control_id == ED_PROC_INFO as i32 {
            // Round info glyph (Go draws it as a circle button).
            let state = crate::nativeform::control_state(item.control, item.state);
            crate::paint::draw_button(
                dc,
                bounds,
                font,
                "i",
                p,
                p.surface,
                state,
                (bounds.bottom - bounds.top) / 2,
            );
        } else if weekday_ids.contains(&item.control_id) {
            // Day chips: quiet when off, accent-filled when on - chip
            // grammar rather than button chrome, pairing with the quick
            // pair below. Both ride the trigger card face.
            let mut state = crate::nativeform::control_state(item.control, item.state);
            let selected = edit_is_checked(item.control_id as usize);
            state.active = selected;
            crate::paint::draw_chip(
                dc,
                bounds,
                font,
                &label,
                p,
                p.surface,
                state,
                crate::paint::control_radius(),
                selected,
            );
        } else if matches!(
            item.control_id as usize,
            ED_DAYS_WORKDAYS | ED_DAYS_EVERYDAY
        ) {
            // The weekday quick pair uses the panel's instant-action chip
            // grammar (same as the timed keep-awake presets): a quiet verb
            // chip, not a button. Rides the trigger card face.
            let state = crate::nativeform::control_state(item.control, item.state);
            crate::paint::draw_chip(
                dc,
                bounds,
                font,
                &label,
                p,
                p.surface,
                state,
                crate::paint::control_radius(),
                false,
            );
        } else if (PK_COL_BASE as i32..=(PK_COL_BASE + 2) as i32).contains(&item.control_id) {
            // Sort strip: quiet ink captions on the list card (nav-item
            // grammar - text color alone carries hover), the active column
            // heavier with its arrow.
            let state = crate::nativeform::control_state(item.control, item.state);
            let (column, _) = *crate::runtime::lock(&PK_SORT);
            let active = column == item.control_id - PK_COL_BASE as i32;
            let ink = if state.pressed {
                p.link_pressed
            } else if state.hovered || active {
                p.link
            } else {
                p.text2
            };
            crate::paint::fill_rect(dc, bounds, p.surface);
            let mut text_bounds = *bounds;
            text_bounds.left += crate::scale_pub(2);
            crate::paint::draw_label(
                dc,
                &text_bounds,
                if active {
                    section_font_cached()
                } else {
                    form_font_body()
                },
                &label,
                ink,
                true,
                0,
                0,
            );
        } else {
            let mut state = crate::nativeform::control_state(item.control, item.state);
            // Footer parity with the settings form: Save (and the picker's
            // Confirm) carry the accent fill of a default action. The
            // editor's process picker rides the trigger card, the footer
            // buttons ride the pane background.
            if matches!(item.control_id as usize, ED_SAVE | PK_CONFIRM) {
                state.active = true;
            }
            let background = if item.control_id as usize == ED_CHOOSE {
                p.surface
            } else {
                p.window_bg
            };
            crate::paint::draw_button(
                dc,
                bounds,
                font,
                &label,
                p,
                background,
                state,
                crate::paint::control_radius(),
            );
        }
    }
}

/// Go nativeform.PresentControl parity: force one synchronous on-screen
/// paint of a native content control after its data changed. Without this,
/// freshly-filled ListView/EDIT controls can keep a validated update region
/// and never composite their text to the screen.
fn present_control(control: HWND) {
    use windows::Win32::Graphics::Gdi::{
        RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW, REDRAW_WINDOW_FLAGS,
        RedrawWindow,
    };
    unsafe {
        let _ = RedrawWindow(
            Some(control),
            None,
            None,
            REDRAW_WINDOW_FLAGS(
                RDW_INVALIDATE.0 | RDW_ERASE.0 | RDW_UPDATENOW.0 | RDW_FRAME.0 | RDW_ALLCHILDREN.0,
            ),
        );
    }
}

/// Child controls are inserted below existing siblings. Move cards behind
/// content only after all children exist, not before creating the EDIT/list.
unsafe fn lower_surfaces(parent: HWND) {
    unsafe {
        let ids = [
            MGR_LIST_SURFACE,
            PK_SEARCH_SURFACE,
            PK_LIST_SURFACE,
            PK_PREVIEW_SURFACE,
        ];
        for id in ids
            .into_iter()
            .chain([ED_CARD_BASIC, ED_CARD_TRIGGER, ED_CARD_OPTIONS])
        {
            let surface = get_dlg_item(parent, id);
            if !surface.is_invalid() {
                let _ = SetWindowPos(
                    surface,
                    Some(HWND(1 as *mut _)),
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
        lower_surfaces(pk);
        refresh_tooltips();

        // Seed the selection from the editor draft (Go Show Selected).
        *crate::runtime::lock(&PK_SELECTED) = edit_procs();
        *crate::runtime::lock(&PK_SORT) = (0, true);
        set_text(get_dlg_item(pk, PK_SEARCH), "");

        picker_load();
        apply_filter();

        // Modal target is the manager ROOT: disabling only the embedded
        // pane left the manager's caption draggable/clickable underneath.
        // The picker is an owned popup, so the standard owner-disable modal
        // pattern applies.
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
            GetAncestor(owner, GA_ROOT),
            false,
        );
        // The snapshot, rows, preview and status commit as one visible
        // frame (Go picker firstFrame.Reveal).
        if crate::viewport::metrics(pk).is_none() {
            crate::viewport::fit(pk);
        }
        crate::FirstFrameGate::begin(pk).reveal();
        // The uncloak can leave freshly-filled native controls with a
        // validated region; force their first on-screen paint explicitly.
        present_control(get_dlg_item(pk, PK_SEARCH));
        present_control(HWND(PK_LIST_HWND.load(Ordering::SeqCst) as *mut _));
        present_control(get_dlg_item(pk, PK_PREVIEW));
        let _ = SetForegroundWindow(pk);
        // No default focus into the search edit - same reasoning as the
        // editor's name field: the hint must be visible on arrival. The
        // form itself takes focus.
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(pk));
    }
}

fn hide_picker() {
    PK_GENERATION.fetch_add(1, Ordering::SeqCst);
    PK_LOADING.store(false, Ordering::SeqCst);
    unsafe {
        let pk = HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _);
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        let _ = ShowWindow(pk, SW_HIDE);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
            GetAncestor(ed, GA_ROOT),
            true,
        );
        // The editor is an embedded pane: foreground belongs to its root.
        let _ = SetForegroundWindow(GetAncestor(ed, GA_ROOT));
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
        // Center on the manager root: the picker's modal target is the
        // embedded editor pane, but a popup parented to a child window has
        // unreliable activation, so the OS-level parent is the root window.
        let root = GetAncestor(owner, GA_ROOT);
        let host = if root.is_invalid() { owner } else { root };
        let (x, y) = center_on_parent(host, frame.right - frame.left, frame.bottom - frame.top);

        let pk = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("IdleTriggerProcPicker"),
            PCWSTR(wide(&t_pub("process_picker_title")).as_ptr()),
            style,
            x,
            y,
            frame.right - frame.left,
            frame.bottom - frame.top,
            Some(host),
            None,
            Some(instance.into()),
            None,
        )
        .expect("picker window");
        PICKER_HWND.store(pk.0 as isize, Ordering::SeqCst);
        crate::dpi::install(pk);
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
            &t_pub(caption_key(PICKER_TEXTS, PK_HEADING)),
            font,
            PK_HEADING_Y,
        );
        mk_static(
            PK_HELPER,
            &t_pub(caption_key(PICKER_TEXTS, PK_HELPER)),
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
            // The settings field grammar (place()): the 3px inset puts the
            // well ring exactly on the card edge, like every other input.
            s(PK_PAD + 3),
            s(PK_SEARCH_Y + 7),
            s(370 - 6),
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
        // Same inner margins as every mk_edit field (Go editWithStyle).
        // AFTER the font assignment, in mk_edit's order: a font change
        // resets an edit's margins to the font-derived defaults, which
        // silently wiped a before-font setting here.
        let margin = s(6) as usize;
        let _ = SendMessageW(
            search,
            EM_SETMARGINS_RAW,
            Some(WPARAM(3)),
            Some(LPARAM((margin | (margin << 16)) as isize)),
        );
        // Cue banner hint (Go NewCueBanner).
        crate::nativeform::cue_banner(search, "process_picker_search_hint");

        // Refresh + Browse buttons (Go 132 / 146 wide).
        for (id, key, x, w) in [
            (
                PK_REFRESH,
                caption_key(PICKER_TEXTS, PK_REFRESH),
                PK_PAD + 370 + ED_GAP,
                132,
            ),
            (
                PK_BROWSE,
                caption_key(PICKER_TEXTS, PK_BROWSE),
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

        // Column sort strip (the old listview header's job), quiet text
        // buttons in the family grammar riding the list card face.
        let col_widths = [250, 310, 82];
        let mut col_x = PK_PAD + 2;
        for (index, width) in col_widths.iter().enumerate() {
            let btn = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("BUTTON"),
                windows::core::w!(""),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | 0x0B), // BS_OWNERDRAW
                s(col_x),
                s(PK_LIST_Y + 2),
                s(*width),
                s(24),
                Some(pk),
                Some(HMENU((PK_COL_BASE + index) as *mut _)),
                Some(instance.into()),
                None,
            )
            .expect("picker column button");
            let _ = SendMessageW(
                btn,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            crate::nativeform::track(btn);
            col_x += width + 8;
        }

        // The process list itself: the manager's owner-draw LISTBOX
        // (family rows, family scrollbar) - SysListView32's self-managed
        // scrollbar and themed chrome were an endless mismatch source.
        let list = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("LISTBOX"),
            windows::core::w!(""),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_TABSTOP.0
                    | WS_CLIPSIBLINGS.0
                    | LBS_NOTIFY as u32
                    | LBS_OWNERDRAWFIXED as u32
                    | LBS_HASSTRINGS as u32
                    | windows::Win32::UI::WindowsAndMessaging::LBS_NOINTEGRALHEIGHT as u32,
            ),
            s(PK_PAD + 2),
            s(PK_LIST_Y + 30),
            s(content_w - 4),
            s(PK_LIST_H - 32),
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
        {
            let proc: unsafe extern "system" fn(
                HWND,
                u32,
                WPARAM,
                LPARAM,
                usize,
                usize,
            ) -> LRESULT = picker_list_proc;
            windows::Win32::UI::Shell::SetWindowSubclass(list, Some(proc), 0x5150, 0)
                .expect("picker list subclass");
        }
        dpi_changed(pk);
        crate::list_style::install(list);
        // The list's scrollbar is painted into its card face.
        crate::list_style::set_lane_card(list, get_dlg_item(pk, PK_LIST_SURFACE));

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
        // Install after the font: the family bar's visibility math reads the
        // item height, which the final font sets.
        crate::list_style::install(preview);
        crate::list_style::set_lane_card(preview, get_dlg_item(pk, PK_PREVIEW_SURFACE));

        mk_static(
            PK_PRIVACY,
            &t_pub(caption_key(PICKER_TEXTS, PK_PRIVACY)),
            font,
            PK_PRIVACY_Y,
        );

        // Footer: Confirm right, Cancel left of it (Go placement).
        let confirm_x = PK_W - PK_PAD - PK_BUTTON_W;
        let cancel_x = confirm_x - ED_GAP - PK_BUTTON_W;
        for (id, key, x) in [
            (PK_CANCEL, caption_key(PICKER_TEXTS, PK_CANCEL), cancel_x),
            (PK_CONFIRM, caption_key(PICKER_TEXTS, PK_CONFIRM), confirm_x),
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

/// Each refresh owns its result: workers never mutate the visible model,
/// and a generation bump in invalidate_picker discards every in-flight load.
static PK_GENERATION: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
type PickerResult = (usize, bool, Result<Vec<PickItem>, String>);
static PK_RESULT: Mutex<Option<PickerResult>> = Mutex::new(None);
static PK_LOADING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PK_ENRICHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PK_UPDATED: Mutex<Option<std::time::Instant>> = Mutex::new(None);
static DESCRIPTIONS: std::sync::LazyLock<Mutex<std::collections::HashMap<String, String>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

fn described_target(target: &auto::ProcessTarget) -> String {
    let descriptions = crate::runtime::lock(&DESCRIPTIONS);
    match descriptions
        .get(&target.key())
        .filter(|value| !value.is_empty())
    {
        Some(description) => format!("{} ({description})", target.executable),
        None => target.executable.clone(),
    }
}

fn publish_picker(
    pk: isize,
    generation: usize,
    complete: bool,
    result: Result<Vec<PickItem>, String>,
) {
    let mut slot = crate::runtime::lock(&PK_RESULT);
    if PK_GENERATION.load(Ordering::SeqCst) == generation {
        *slot = Some((generation, complete, result));
        unsafe {
            let _ = PostMessageW(
                Some(HWND(pk as *mut _)),
                WM_APP_PICKER_DESC,
                WPARAM(generation),
                LPARAM(0),
            );
        }
    }
}

fn picker_load() {
    let generation = PK_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    PK_LOADING.store(true, Ordering::SeqCst);
    PK_ENRICHING.store(false, Ordering::SeqCst);
    let selected = crate::runtime::lock(&PK_SELECTED).clone();
    let pk = PICKER_HWND.load(Ordering::SeqCst);
    let spawn = std::thread::Builder::new()
        .name("picker-load".into())
        .spawn(move || {
            // On a panic the default error still reaches publish_picker, so
            // the picker cannot get stuck on "loading" forever.
            let mut result = Err("picker load failed".to_string());
            crate::runtime::catch_and_log("picker-load", || {
                result = (|| {
                    let snapshot = crate::automation::Snapshot::take()
                        .ok_or_else(|| "snapshot unavailable".to_string())?;
                    let mut targets: Vec<auto::ProcessTarget> = snapshot
                        .names()
                        .iter()
                        .filter(|name| {
                            // Bracketed entries are kernel pseudo-processes
                            // ([System Process] and friends): Task Manager
                            // hides them, and no rule usefully targets them.
                            !name.starts_with('[')
                                && snapshot
                                    .pid_names()
                                    .iter()
                                    .any(|(n, pid)| n == *name && *pid != std::process::id())
                        })
                        .map(|name| auto::ProcessTarget {
                            kind: auto::MATCH_NAME.into(),
                            executable: snapshot.display_of(name).to_string(),
                            path: String::new(),
                        })
                        .collect();
                    for target in selected {
                        if !targets.iter().any(|t| t.key() == target.key()) {
                            targets.push(target);
                        }
                    }
                    let mut items = Vec::new();
                    for target in targets {
                        if PK_GENERATION.load(Ordering::SeqCst) != generation {
                            return Ok(items);
                        }
                        let count = snapshot.count_target(&target);
                        let description = crate::runtime::lock(&DESCRIPTIONS)
                            .get(&target.key())
                            .cloned()
                            .unwrap_or_default();
                        let name = if target.kind == auto::MATCH_PATH {
                            target.path.clone()
                        } else {
                            target.executable.clone()
                        };
                        let search = format!("{name} {description}").to_lowercase();
                        items.push(PickItem {
                            target,
                            name,
                            description,
                            count,
                            search,
                        });
                    }
                    publish_picker(pk, generation, false, Ok(items.clone()));
                    for item in &mut items {
                        if PK_GENERATION.load(Ordering::SeqCst) != generation {
                            return Ok(items);
                        }
                        let path = if item.target.kind == auto::MATCH_PATH {
                            Some(item.target.path.clone())
                        } else {
                            snapshot.path_of(&item.target.executable.to_lowercase())
                        };
                        item.description = path
                            .as_deref()
                            .and_then(file_description)
                            .unwrap_or_default();
                        item.search = format!("{} {}", item.name, item.description).to_lowercase();
                    }
                    Ok(items)
                })();
            });
            publish_picker(pk, generation, true, result);
        });
    if let Err(error) = spawn {
        PK_LOADING.store(false, Ordering::SeqCst);
        update_selection_status(HWND(pk as *mut _));
        set_text(
            get_dlg_item(HWND(pk as *mut _), PK_STATUS),
            &fill_template(&t_pub("process_picker_error"), &[&error.to_string()]),
        );
    }
}

fn finish_picker_load(pk: HWND) {
    let result = crate::runtime::lock(&PK_RESULT).take();
    let Some((generation, complete, result)) = result else {
        return;
    };
    if generation != PK_GENERATION.load(Ordering::SeqCst) {
        return;
    }
    PK_LOADING.store(false, Ordering::SeqCst);
    PK_ENRICHING.store(!complete, Ordering::SeqCst);
    match result {
        Ok(items) => {
            if complete {
                *crate::runtime::lock(&PK_UPDATED) = Some(std::time::Instant::now());
                let mut descriptions = crate::runtime::lock(&DESCRIPTIONS);
                if descriptions.len() > 2048 {
                    descriptions.clear();
                }
                for item in &items {
                    descriptions.insert(item.target.key(), item.description.clone());
                }
            }
            *crate::runtime::lock(&PK_ITEMS) = items;
            apply_filter();
        }
        Err(error) => {
            update_selection_status(pk);
            set_text(
                get_dlg_item(pk, PK_STATUS),
                &fill_template(&t_pub("process_picker_error"), &[&error]),
            );
        }
    }
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
        if size == 0 || size > 16 * 1024 * 1024 {
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
        // Read the translation table, then each language's description in
        // turn: some binaries only carry FileDescription under a secondary
        // translation, and Task Manager walks them all before giving up.
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
        let start = data.as_ptr() as usize;
        let end = start + data.len();
        if block.is_null()
            || (block as usize) < start
            || (block as usize)
                .checked_add(len as usize)
                .is_none_or(|p| p > end)
        {
            return None;
        }
        for index in 0..(len / 4) as usize {
            let lang = (block as *const u16).add(index * 2).read_unaligned();
            let code_page = (block as *const u16).add(index * 2 + 1).read_unaligned();
            let sub = format!(
                "\\StringFileInfo\\{:04x}{:04x}\\FileDescription",
                lang, code_page
            );
            let sub_wide = wide(&sub);
            let mut text: *mut core::ffi::c_void = std::ptr::null_mut();
            let mut text_len: u32 = 0;
            if !windows::Win32::Storage::FileSystem::VerQueryValueW(
                data.as_ptr().cast(),
                PCWSTR(sub_wide.as_ptr()),
                &mut text,
                &mut text_len,
            )
            .as_bool()
                || text.is_null()
            {
                continue;
            }
            if (text as usize) < start
                || (text as usize)
                    .checked_add(text_len as usize * 2)
                    .is_none_or(|p| p > end)
            {
                continue;
            }
            let chars: Vec<u16> = (0..text_len as usize)
                .map(|i| (text as *const u16).add(i).read_unaligned())
                .collect();
            let terminator = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
            let description = String::from_utf16_lossy(&chars[..terminator])
                .trim()
                .to_string();
            if !description.is_empty() {
                return Some(description);
            }
        }
        None
    }
}

/// Filters and sorts the item rows, then repopulates the ListView (Go
/// applyFilter + reconcileVisible).
fn apply_filter() {
    unsafe {
        let pk = HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _);
        let list = HWND(PK_LIST_HWND.load(Ordering::SeqCst) as *mut _);
        let filter = window_text(get_dlg_item(pk, PK_SEARCH))
            .trim()
            .to_lowercase();
        let (column, ascending) = *crate::runtime::lock(&PK_SORT);
        let old_visible = crate::runtime::lock(&PK_VISIBLE).clone();
        let old_top = SendMessageW(
            list,
            windows::Win32::UI::WindowsAndMessaging::LB_GETTOPINDEX,
            None,
            None,
        )
        .0;
        let old_focus = SendMessageW(
            list,
            windows::Win32::UI::WindowsAndMessaging::LB_GETCURSEL,
            None,
            None,
        )
        .0;
        let key_at = |index: isize| {
            old_visible
                .get(index as usize)
                .map(|item| item.target.key())
        };
        let top_key = key_at(old_top);
        let focus_key = key_at(old_focus);

        let mut items = crate::runtime::lock(&PK_ITEMS).clone();
        for target in crate::runtime::lock(&PK_SELECTED).iter() {
            if !items.iter().any(|item| item.target.key() == target.key()) {
                let name = if target.kind == auto::MATCH_PATH {
                    target.path.clone()
                } else {
                    target.executable.clone()
                };
                items.push(PickItem {
                    target: target.clone(),
                    search: name.to_lowercase(),
                    name,
                    description: String::new(),
                    count: 0,
                });
            }
        }
        items.retain(|i| filter.is_empty() || i.search.contains(&filter));
        items.sort_by(|a, b| {
            let key = |item: &PickItem| match column {
                1 => item.description.to_lowercase(),
                2 => format!("{:08}", item.count),
                _ => item.name.to_lowercase(),
            };
            let group = a
                .target
                .executable
                .to_lowercase()
                .cmp(&b.target.executable.to_lowercase());
            let ordering = if column == 0 {
                group
                    .then_with(|| a.target.kind.cmp(&b.target.kind))
                    .then_with(|| a.target.key().cmp(&b.target.key()))
            } else {
                key(a)
                    .cmp(&key(b))
                    .then(group)
                    .then_with(|| a.target.key().cmp(&b.target.key()))
            };
            if ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });
        *crate::runtime::lock(&PK_VISIBLE) = items.clone();

        let _ = SendMessageW(list, WM_SETREDRAW, Some(WPARAM(0)), Some(LPARAM(0)));
        PK_POPULATING.store(true, Ordering::SeqCst);
        let _ = SendMessageW(
            list,
            windows::Win32::UI::WindowsAndMessaging::LB_RESETCONTENT,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
        for item in items.iter() {
            let text = wide(&item.name);
            let _ = SendMessageW(
                list,
                windows::Win32::UI::WindowsAndMessaging::LB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(text.as_ptr() as isize)),
            );
        }
        if let Some(index) = items
            .iter()
            .position(|item| focus_key.as_ref() == Some(&item.target.key()))
        {
            let _ = SendMessageW(
                list,
                windows::Win32::UI::WindowsAndMessaging::LB_SETCURSEL,
                Some(WPARAM(index)),
                Some(LPARAM(0)),
            );
        }
        if let Some(index) = items
            .iter()
            .position(|item| top_key.as_ref() == Some(&item.target.key()))
        {
            let _ = SendMessageW(
                list,
                windows::Win32::UI::WindowsAndMessaging::LB_SETTOPINDEX,
                Some(WPARAM(index)),
                Some(LPARAM(0)),
            );
        } else {
            // First population (loading): no remembered position - pin the
            // top so the scrollbar never opens mid-list.
            let _ = SendMessageW(
                list,
                windows::Win32::UI::WindowsAndMessaging::LB_SETTOPINDEX,
                Some(WPARAM(0)),
                Some(LPARAM(0)),
            );
        }
        PK_POPULATING.store(false, Ordering::SeqCst);
        let _ = SendMessageW(list, WM_SETREDRAW, Some(WPARAM(1)), Some(LPARAM(0)));
        present_control(list);

        // Empty overlay + status row.
        let empty = items.is_empty();
        let message = if PK_LOADING.load(Ordering::SeqCst) {
            t_pub("process_picker_loading")
        } else if !filter.is_empty() {
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

/// Reads checkbox states back into the selection (Go captureSelection).
/// Toggles one visible row's check in PK_SELECTED (the single source of
/// truth now that rows are painted, not state-imaged), with the per-rule
/// process limit enforced on add.
unsafe fn toggle_picker_row(pk: HWND, index: usize) {
    unsafe {
        let visible = crate::runtime::lock(&PK_VISIBLE).clone();
        let Some(item) = visible.get(index) else {
            return;
        };
        let target = item.target.clone();
        let mut selected = crate::runtime::lock(&PK_SELECTED);
        if let Some(pos) = selected.iter().position(|t| t.key() == target.key()) {
            selected.remove(pos);
        } else if selected.len() < auto::MAX_PROCESSES_PER_RULE {
            selected.push(target);
        } else {
            drop(selected);
            set_text(
                get_dlg_item(pk, PK_STATUS),
                &fill_template(
                    &t_pub("process_picker_limit"),
                    &[&auto::MAX_PROCESSES_PER_RULE.to_string()],
                ),
            );
            return;
        }
        drop(selected);
        update_selection_status(pk);
        update_preview(pk);
        let list = HWND(PK_LIST_HWND.load(Ordering::SeqCst) as *mut _);
        let mut row = RECT::default();
        if SendMessageW(
            list,
            windows::Win32::UI::WindowsAndMessaging::LB_GETITEMRECT,
            Some(WPARAM(index)),
            Some(LPARAM(&mut row as *mut _ as isize)),
        )
        .0 != 0
        {
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(list), Some(&row), false);
        }
    }
}

/// Picker listbox subclass: the whole row is the check toggle (the picker
/// is a checklist), Space toggles the cursored row, and plain clicks keep
/// the native cursor behavior on top.
unsafe extern "system" fn picker_list_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        let pk = HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _);
        match msg {
            windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONDOWN
            | windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONDBLCLK => {
                let result = windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam);
                let point = ((lparam.0 & 0xFFFF) as u16) as usize
                    | (((lparam.0 >> 16) & 0xFFFF) as usize) << 16;
                let hit = SendMessageW(
                    hwnd,
                    windows::Win32::UI::WindowsAndMessaging::LB_ITEMFROMPOINT,
                    Some(WPARAM(0)),
                    Some(LPARAM(point as isize)),
                )
                .0 as u32;
                if hit & 0xFFFF_0000 == 0 {
                    toggle_picker_row(pk, (hit & 0xFFFF) as usize);
                }
                return result;
            }
            windows::Win32::UI::WindowsAndMessaging::WM_KEYDOWN
                if wparam.0 == windows::Win32::UI::Input::KeyboardAndMouse::VK_SPACE.0 as usize =>
            {
                let index = SendMessageW(
                    hwnd,
                    windows::Win32::UI::WindowsAndMessaging::LB_GETCURSEL,
                    None,
                    None,
                )
                .0;
                if index >= 0 {
                    toggle_picker_row(pk, index as usize);
                    return LRESULT(0);
                }
            }
            _ => {}
        }
        windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

fn selected_count() -> usize {
    crate::runtime::lock(&PK_SELECTED).len()
}

fn picker_requires_process() -> bool {
    matches!(
        choice_value(HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _), ED_TRIGGER).as_str(),
        "process_running" | "process_started" | "process_exited"
    )
}

/// Go updateSelectionStatus: limit warning or "shown N · selected M".
fn update_selection_status(pk: HWND) {
    let visible = crate::runtime::lock(&PK_VISIBLE).len();
    let selected = selected_count();
    let status = if PK_LOADING.load(Ordering::SeqCst) {
        t_pub("process_picker_loading")
    } else if PK_ENRICHING.load(Ordering::SeqCst) {
        t_pub("process_picker_loading_descriptions")
    } else if selected > auto::MAX_PROCESSES_PER_RULE {
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
        PK_REFRESH,
        !PK_LOADING.load(Ordering::SeqCst) && !PK_ENRICHING.load(Ordering::SeqCst),
    );
    enable_control(
        pk,
        PK_CONFIRM,
        (selected > 0 || !picker_requires_process()) && selected <= auto::MAX_PROCESSES_PER_RULE,
    );
}

/// Rebuilds the preview card and its title (Go updatePreview).
fn update_preview(pk: HWND) {
    unsafe {
        let preview = get_dlg_item(pk, PK_PREVIEW);
        let selected = crate::runtime::lock(&PK_SELECTED).clone();
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
                let name = &described_target(target);
                let label = if target.kind == auto::MATCH_PATH {
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

/// Column captions with sort arrows (Go headerCaption), on the sort-button
/// strip that replaced the listview header.
fn update_header_captions(pk: HWND) {
    unsafe {
        let (column, ascending) = *crate::runtime::lock(&PK_SORT);
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
            let button = get_dlg_item(pk, PK_COL_BASE + index);
            if !button.is_invalid() {
                let text = wide(&caption);
                let _ = SetWindowTextW(button, windows::core::PCWSTR(text.as_ptr()));
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(button), None, false);
            }
        }
    }
}

/// Go confirm: publish the checked targets back to the editor draft.
fn picker_confirm() {
    let selected = crate::runtime::lock(&PK_SELECTED).clone();
    if selected.len() > auto::MAX_PROCESSES_PER_RULE
        || (selected.is_empty() && picker_requires_process())
    {
        let pk = HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _);
        update_selection_status(pk);
        return;
    }
    set_edit_procs(selected);
    update_proc_summary();
    layout_editor();
    let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
    clear_editor_error(ed);
    hide_picker();
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
            kind: auto::MATCH_PATH.into(),
            executable,
            path: path.clone(),
        };
        // A path target replaces the equivalent name target (Go parity).
        let mut selected: Vec<auto::ProcessTarget> = crate::runtime::lock(&PK_SELECTED)
            .iter()
            .filter(|t| {
                !(t.kind == auto::MATCH_NAME
                    && t.executable.eq_ignore_ascii_case(&target.executable))
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
        *crate::runtime::lock(&PK_SELECTED) = selected;
        picker_load();
        apply_filter();
    }
}

unsafe extern "system" fn picker_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    crate::guarded_proc(
        "automation-picker",
        hwnd,
        msg,
        wparam,
        lparam,
        move || unsafe {
            match msg {
                windows::Win32::UI::WindowsAndMessaging::WM_NCHITTEST => {
                    // Painted scrollbar lanes take pointer input on the form:
                    // report HTCLIENT so mouse messages reach lane_pointer.
                    if crate::list_style::lane_hit(hwnd, lparam) {
                        return LRESULT(1); // HTCLIENT
                    }
                    // Shared blank drag (nativeform::blank_drag_hit): a
                    // HTCLIENT point no interactive child claims drags the
                    // window; never WindowFromPoint from inside a hit-test
                    // (its probe re-enters the caller and recurses).
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
                windows::Win32::UI::WindowsAndMessaging::WM_NCRBUTTONUP => LRESULT(0),
                WM_ACTIVATE if wparam.0 & 0xffff != 0 => {
                    if !PK_LOADING.load(Ordering::SeqCst)
                        && !PK_ENRICHING.load(Ordering::SeqCst)
                        && crate::runtime::lock(&PK_UPDATED).is_some_and(|last| {
                            last.elapsed() >= std::time::Duration::from_secs(30)
                        })
                    {
                        picker_load();
                        update_selection_status(hwnd);
                    }
                    LRESULT(0)
                }
                WM_COMMAND => {
                    let code = wparam.0 & 0xFFFF;
                    let hi = ((wparam.0 >> 16) & 0xFFFF) as u16;
                    match code {
                        PK_CONFIRM if hi == BN_CLICKED => picker_confirm(),
                        PK_CANCEL if hi == BN_CLICKED => hide_picker(),
                        PK_REFRESH if hi == BN_CLICKED => {
                            picker_load();
                            apply_filter();
                        }
                        PK_BROWSE if hi == BN_CLICKED => browse_executable(hwnd),
                        code if hi == BN_CLICKED
                            && (PK_COL_BASE..=PK_COL_BASE + 2).contains(&code) =>
                        {
                            let column = (code - PK_COL_BASE) as i32;
                            let mut sort = crate::runtime::lock(&PK_SORT);
                            if sort.0 == column {
                                sort.1 = !sort.1;
                            } else {
                                *sort = (column, true);
                            }
                            drop(sort);
                            apply_filter();
                        }
                        PK_SEARCH if hi == EN_CHANGE => {
                            // Go debounces the search box by 120ms; IME
                            // composition fires EN_CHANGE per candidate, and
                            // each filter rebuilds the whole list view.
                            let _ = KillTimer(Some(hwnd), PK_FILTER_TIMER);
                            let _ = SetTimer(Some(hwnd), PK_FILTER_TIMER, 120, None);
                        }
                        PK_SEARCH if hi == EN_SETFOCUS || hi == EN_KILLFOCUS => {
                            // Focus tint swap for the card-painted well. The
                            // scoped well rect lands a pixel short of the
                            // ring's right/bottom edge here (the ring rides
                            // the card edge, unlike editor wells), leaving a
                            // stale-color seam - repaint the whole card,
                            // synchronously, instead.
                            let card = get_dlg_item(hwnd, PK_SEARCH_SURFACE);
                            if !card.is_invalid() {
                                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(
                                    Some(card),
                                    None,
                                    false,
                                );
                                let _ = UpdateWindow(card);
                            }
                        }
                        _ => {}
                    }
                    LRESULT(0)
                }
                WM_MEASUREITEM => {
                    // Owner-draw process rows: settings-row height rhythm.
                    let measure =
                        &mut *(lparam.0 as *mut windows::Win32::UI::Controls::MEASUREITEMSTRUCT);
                    if measure.CtlID == PK_LIST as u32 {
                        measure.itemHeight = s(30) as u32;
                        return LRESULT(1);
                    }
                    LRESULT(0)
                }
                WM_TIMER if wparam.0 == PK_FILTER_TIMER => {
                    let _ = KillTimer(Some(hwnd), PK_FILTER_TIMER);
                    apply_filter();
                    LRESULT(0)
                }
                WM_APP_PICKER_DESC => {
                    finish_picker_load(hwnd);
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
                    let secondary =
                        matches!(id, PK_HELPER | PK_STATUS | PK_PRIVACY | PK_PREVIEW_TITLE);
                    let on_surface = matches!(id, PK_EMPTY | PK_PREVIEW | PK_LIST);
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
                WM_DESTROY => {
                    let _ = KillTimer(Some(hwnd), PK_FILTER_TIMER);
                    PK_GENERATION.fetch_add(1, Ordering::SeqCst);
                    PICKER_HWND.store(0, Ordering::SeqCst);
                    PK_LIST_HWND.store(0, Ordering::SeqCst);
                    // Same rule as the editor: drop picker caches on a real
                    // destroy so recreation starts clean. PK_GENERATION above
                    // already invalidates any in-flight background load.
                    crate::runtime::lock(&PK_ITEMS).clear();
                    crate::runtime::lock(&PK_VISIBLE).clear();
                    crate::runtime::lock(&PK_SELECTED).clear();
                    *crate::runtime::lock(&PK_SORT) = (0, true);
                    crate::runtime::lock(&DESCRIPTIONS).clear();
                    *crate::runtime::lock(&PK_RESULT) = None;
                    *crate::runtime::lock(&PK_UPDATED) = None;
                    // The modal disable targeted the manager ROOT, not the
                    // embedded editor pane — re-enable the same window.
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
                        HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _),
                        true,
                    );
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        },
    )
}

// ===== Devtools + theme hooks ===============================================

pub fn refresh_theme() {
    refresh_tooltips();
    // Picker list colors flow through WM_CTLCOLORLISTBOX and the owner-draw
    // row painter - nothing theme-specific to push into the control.
}

fn refresh_tooltips() {
    for (window, bindings) in [
        (
            HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _),
            &[
                (MGR_LIST, "tip_automation_list"),
                (MGR_NEW, "tip_automation_new"),
                (MGR_EDIT, "tip_automation_edit"),
                (MGR_TOGGLE, "tip_automation_toggle"),
                (MGR_DELETE, "tip_automation_delete"),
            ][..],
        ),
        (
            HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _),
            &[
                (ED_NAME, "tip_automation_name"),
                (ED_ACTION, "tip_automation_action"),
                (ED_TRIGGER, "tip_automation_trigger"),
                (ED_TIME, "tip_automation_time"),
                (ED_DATE, "tip_automation_date"),
                (ED_END_TIME, "tip_automation_end_time"),
                (ED_LOGIC, "tip_process_logic"),
                (ED_WARNING, "tip_warning_seconds"),
                (ED_IDLE_MIN, "tip_idle_minutes"),
                (ED_MAX_WAIT, "tip_max_wait"),
                (ED_KEEP_SCREEN, "tip_keep_screen"),
                (ED_BLOCKED, "tip_blocked_policy"),
                (ED_CHOOSE, "tip_choose_processes"),
                (ED_PROC_INFO, "automation_process_info_accessible"),
                (ED_DAYS_MON, "tip_automation_days"),
                (ED_DAYS_TUE, "tip_automation_days"),
                (ED_DAYS_WED, "tip_automation_days"),
                (ED_DAYS_THU, "tip_automation_days"),
                (ED_DAYS_FRI, "tip_automation_days"),
                (ED_DAYS_SAT, "tip_automation_days"),
                (ED_DAYS_SUN, "tip_automation_days"),
                (ED_DAYS_WORKDAYS, "tip_automation_days_workdays"),
                (ED_DAYS_EVERYDAY, "tip_automation_days_everyday"),
                (ED_SAVE, "tip_automation_save"),
                (ED_CANCEL, "tip_automation_cancel"),
            ][..],
        ),
        (
            HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _),
            &[
                (PK_SEARCH, "tip_process_search"),
                (PK_REFRESH, "tip_process_refresh"),
                (PK_BROWSE, "tip_process_browse"),
                (PK_CONFIRM, "tip_process_confirm"),
                (PK_CANCEL, "tip_process_cancel"),
            ][..],
        ),
    ] {
        if !window.is_invalid() {
            let _dpi = crate::dpi::Scope::window(window);
            crate::nativeform::form_tooltips(window, bindings);
        }
    }
}

pub fn dpi_changed(hwnd: HWND) {
    let _dpi = crate::dpi::Scope::window(hwnd);
    // The editor pane is a child window and never sees WM_DPICHANGED; the
    // manager's hook stretches it over the client area and re-lays-out its
    // controls (which also resizes the window to the new content height).
    if hwnd.0 as isize == MGR_HWND.load(Ordering::SeqCst) {
        unsafe {
            let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
            if !ed.is_invalid() {
                let mut client = RECT::default();
                if GetClientRect(hwnd, &mut client).is_ok() {
                    let _ = SetWindowPos(
                        ed,
                        None,
                        0,
                        0,
                        client.right,
                        client.bottom,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                if IsWindowVisible(ed).as_bool() {
                    layout_editor();
                }
            }
        }
    }
}

/// Devtools capture support: open the editor in new-rule mode.
#[cfg(feature = "devtools")]
pub fn devtools_show_editor() {
    begin_edit(-1, Vec::new());
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
        action: auto::ACTION_LOCK.into(),
        trigger: auto::TRIGGER_TIME_WINDOW.into(),
        time: "09:00".into(),
        end_time: "18:00".into(),
        date: String::new(),
        days: vec!["mon".into(), "tue".into(), "wed".into()],
        process_logic: String::new(),
        processes: Vec::new(),
        keep_screen_on: false,
        idle_minutes: 0,
        warning_seconds: 0,
        blocked_policy: String::new(),
        max_wait_minutes: 0,
        battery_level: 0,
    };
    let mut rules = crate::runtime::lock(&crate::automation::RULES);
    rules.push(rule);
    rules.push(auto::Rule {
        id: "devtools-demo2".into(),
        name: "第二个任务".into(),
        enabled: false,
        action: auto::ACTION_SHUTDOWN.into(),
        trigger: auto::TRIGGER_ONCE.into(),
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
        battery_level: 0,
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

/// Relabel existing forms without repopulating edits or changing their save baseline.
/// Single (control id -> caption key) registries: window creation and
/// refresh_language must render the same captions - before these existed
/// the picker's Cancel button was created from `automation_cancel` but
/// refreshed from `common_cancel`. State-managed captions (MGR_TOGGLE,
/// ED_VALIDATION, ED_PROC_SUMMARY) deliberately stay out.
const MANAGER_TEXTS: &[(usize, &str)] = &[
    (MGR_TITLE, "automation_rules_title"),
    (MGR_EMPTY_TITLE, "automation_empty_title"),
    (MGR_EMPTY_BODY, "automation_empty_body"),
    (MGR_NEW, "automation_new"),
    (MGR_EDIT, "automation_edit"),
    (MGR_DELETE, "automation_delete"),
];

const EDITOR_TEXTS: &[(usize, &str)] = &[
    (ED_BASIC_TITLE, "automation_basics"),
    (ED_TRIGGER_TITLE, "automation_trigger_conditions"),
    (ED_OPTIONS_TITLE, "automation_action_options"),
    (ED_NAME_LBL, "automation_name"),
    (ED_ACTION_LBL, "automation_action"),
    (ED_TRIGGER_LBL, "automation_trigger"),
    (ED_TIME_LBL, "automation_time"),
    (ED_DATE_LBL, "automation_date"),
    (ED_END_LBL, "automation_end_time"),
    (ED_LOGIC_LBL, "automation_process_logic"),
    (ED_WARN_LBL, "automation_warning_seconds"),
    (ED_IDLE_LBL, "automation_idle_minutes"),
    (ED_BATTERY_LBL, "automation_battery_level"),
    (ED_MAX_LBL, "automation_max_wait"),
    (ED_DAYS_LBL, "automation_days"),
    (ED_BLOCKED_LBL, "automation_blocked_policy"),
    (ED_NAME_HINT, "automation_name_hint"),
    (ED_NO_OPTIONS, "automation_no_action_options"),
    (ED_CHOOSE, "automation_choose_processes"),
    (ED_PROC_INFO, "automation_process_info_accessible"),
    (ED_DAYS_WORKDAYS, "automation_days_workdays"),
    (ED_DAYS_EVERYDAY, "automation_days_everyday"),
    (ED_KEEP_SCREEN, "automation_keep_screen"),
    (ED_SAVE, "automation_save"),
    (ED_CANCEL, "automation_cancel"),
    (ED_DAYS_MON, "automation_day_mon"),
    (ED_DAYS_TUE, "automation_day_tue"),
    (ED_DAYS_WED, "automation_day_wed"),
    (ED_DAYS_THU, "automation_day_thu"),
    (ED_DAYS_FRI, "automation_day_fri"),
    (ED_DAYS_SAT, "automation_day_sat"),
    (ED_DAYS_SUN, "automation_day_sun"),
];

const PICKER_TEXTS: &[(usize, &str)] = &[
    (PK_HEADING, "process_picker_heading"),
    (PK_HELPER, "process_picker_helper"),
    (PK_PRIVACY, "process_picker_privacy"),
    (PK_REFRESH, "process_picker_refresh"),
    (PK_BROWSE, "process_picker_browse"),
    (PK_CONFIRM, "process_picker_confirm"),
    (PK_CANCEL, "common_cancel"),
];

fn caption_key(registry: &'static [(usize, &'static str)], id: usize) -> &'static str {
    registry
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .map(|(_, key)| *key)
        .unwrap_or("")
}

pub fn refresh_language() {
    refresh_tooltips();
    let mgr = HWND(MGR_HWND.load(Ordering::SeqCst) as *mut _);
    if !mgr.is_invalid() {
        set_text(mgr, &t_pub("automation_title"));
        for &(id, key) in MANAGER_TEXTS {
            set_text(get_dlg_item(mgr, id), &t_pub(key));
        }
        refresh_list();
    }
    let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
    if !ed.is_invalid() {
        let _dpi = crate::dpi::Scope::window(ed);
        crate::nativeform::cue_banner(get_dlg_item(ed, ED_NAME), "automation_name_placeholder");
        set_text(
            ed,
            &t_pub(if edit_index() >= 0 {
                "automation_edit_title"
            } else {
                "automation_new_title"
            }),
        );
        for &(id, key) in EDITOR_TEXTS {
            set_text(get_dlg_item(ed, id), &t_pub(key));
        }
        let action = choice_value(ed, ED_ACTION);
        let trigger = choice_value(ed, ED_TRIGGER);
        let logic = choice_value(ed, ED_LOGIC);
        let blocked = choice_value(ed, ED_BLOCKED);
        fill_action_choice(ed);
        choice_select(ed, ED_ACTION, &action);
        fill_trigger_choice(ed, &action, &trigger);
        fill_logic_choice(ed);
        choice_select(ed, ED_LOGIC, &logic);
        fill_blocked_choice(ed);
        choice_select(ed, ED_BLOCKED, &blocked);
        update_proc_summary();
        layout_editor();
    }
    let pk = HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _);
    if !pk.is_invalid() {
        set_text(pk, &t_pub("process_picker_title"));
        crate::nativeform::cue_banner(get_dlg_item(pk, PK_SEARCH), "process_picker_search_hint");
        for &(id, key) in PICKER_TEXTS {
            set_text(get_dlg_item(pk, id), &t_pub(key));
        }
        // Re-derive the empty-overlay caption for the new language; its
        // text normally only refreshes from apply_filter.
        let filter = window_text(get_dlg_item(pk, PK_SEARCH));
        let overlay_text = if PK_LOADING.load(Ordering::SeqCst) {
            t_pub("process_picker_loading")
        } else if !filter.trim().is_empty() {
            t_pub("process_picker_no_results")
        } else {
            t_pub("process_picker_empty")
        };
        set_text(get_dlg_item(pk, PK_EMPTY), &overlay_text);
        let overlay = get_dlg_item(pk, PK_EMPTY);
        let list_empty = crate::runtime::lock(&PK_VISIBLE).is_empty();
        if !overlay.is_invalid() {
            unsafe {
                let _ = ShowWindow(overlay, if list_empty { SW_SHOW } else { SW_HIDE });
            }
        }
        update_header_captions(pk);
        dpi_changed(pk);
        update_selection_status(pk);
        update_preview(pk);
    }
}

#[cfg(test)]
mod surface_tests {
    use super::*;
    #[test]
    fn worst_case_editor_layout_fits_the_pane_without_scrolling() {
        let _guard = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        let previous = crate::runtime::lock(&crate::CONFIG).replace(Default::default());
        let host = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                400,
                400,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let previous_mgr = MGR_HWND.swap(host.0 as isize, Ordering::SeqCst);
        let previous_edit = EDIT_HWND.load(Ordering::SeqCst);
        let previous_session =
            crate::runtime::lock(&EDIT_SESSION).replace(EditorSession::default());
        create_editor();
        populate_editor();
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        // Worst composite: a time window with weekdays, a process condition,
        // an event action with a cancellable countdown, and wait policy.
        *crate::runtime::lock(&EDIT_SESSION).as_mut().unwrap() = EditorSession {
            procs: vec![auto::ProcessTarget {
                kind: auto::MATCH_NAME.into(),
                executable: "app.exe".into(),
                path: String::new(),
            }],
            ..EditorSession::default()
        };
        choice_select(ed, ED_ACTION, auto::ACTION_LOCK);
        fill_trigger_choice(ed, auto::ACTION_LOCK, auto::TRIGGER_TIME_WINDOW);
        choice_select(ed, ED_TRIGGER, auto::TRIGGER_TIME_WINDOW);
        choice_select(ed, ED_BLOCKED, auto::BLOCKED_WAIT);
        set_text(get_dlg_item(ed, ED_MAX_WAIT), "10");
        // Laying out without showing: the full visible path (window resize
        // plus synchronous repaint) is verified by the devtools capture
        // walk each round — inside the whole test binary that chain trips a
        // cross-test resource-accumulation crash unrelated to this layout.
        layout_editor();
        let content = EDITOR_CONTENT_H.load(Ordering::SeqCst) as i32;
        assert!(
            content > MGR_H && content <= 2 * MGR_H,
            "worst-case layout height {content} outside the sane pane range"
        );
        unsafe {
            DestroyWindow(ed).unwrap();
            DestroyWindow(host).unwrap();
        }
        *crate::runtime::lock(&EDIT_SESSION) = previous_session;
        EDIT_HWND.store(previous_edit, Ordering::SeqCst);
        MGR_HWND.store(previous_mgr, Ordering::SeqCst);
        *crate::runtime::lock(&crate::CONFIG) = previous;
    }
    #[test]
    fn editor_layout_keeps_common_fields_visible_and_preserves_drafts() {
        use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
        unsafe extern "system" fn track(
            hwnd: HWND,
            msg: u32,
            wp: WPARAM,
            lp: LPARAM,
            _: usize,
            data: usize,
        ) -> LRESULT {
            unsafe {
                if msg == WM_WINDOWPOSCHANGING {
                    let position = &*(lp.0 as *const WINDOWPOS);
                    if position.flags.contains(SWP_HIDEWINDOW) {
                        let count = &*(data as *const std::cell::Cell<u32>);
                        count.set(count.get() + 1);
                    }
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
        }
        let _guard = crate::runtime::lock(&crate::CONFIG_TEST_LOCK);
        let previous = crate::runtime::lock(&crate::CONFIG).replace(Default::default());
        // The editor pane needs the manager as its parent; stand one up.
        let host = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                400,
                400,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let previous_mgr = MGR_HWND.swap(host.0 as isize, Ordering::SeqCst);
        let previous_edit = EDIT_HWND.load(Ordering::SeqCst);
        create_editor();
        populate_editor();
        let ed = HWND(EDIT_HWND.load(Ordering::SeqCst) as *mut _);
        let name = get_dlg_item(ed, ED_NAME);
        set_text(name, "unsaved draft");
        let hidden = std::cell::Cell::new(0u32);
        unsafe {
            assert!(
                SetWindowSubclass(name, Some(track), 992, &hidden as *const _ as usize).as_bool()
            );
        }
        for (action, trigger) in [
            (auto::ACTION_STAY_AWAKE, auto::TRIGGER_TIME_WINDOW),
            (auto::ACTION_STAY_AWAKE, auto::TRIGGER_PROCESS_RUNNING),
            (auto::ACTION_LOCK, auto::TRIGGER_ONCE),
            (auto::ACTION_LOCK, auto::TRIGGER_WEEKLY),
            (auto::ACTION_LOCK, auto::TRIGGER_PROCESS_EXITED),
        ] {
            choice_select(ed, ED_ACTION, action);
            fill_trigger_choice(ed, action, trigger);
            layout_editor();
            assert_eq!(crate::window_text(name), "unsaved draft");
            assert_ne!(
                unsafe { GetWindowLongW(name, GWL_STYLE) as u32 & WS_VISIBLE.0 },
                0
            );
        }
        assert_eq!(
            hidden.get(),
            0,
            "common fields must not hide between layouts"
        );
        unsafe {
            let _ = RemoveWindowSubclass(name, Some(track), 992);
            DestroyWindow(ed).unwrap();
            DestroyWindow(host).unwrap();
        }
        EDIT_HWND.store(previous_edit, Ordering::SeqCst);
        MGR_HWND.store(previous_mgr, Ordering::SeqCst);
        *crate::runtime::lock(&crate::CONFIG) = previous;
    }
    #[test]
    fn background_is_lowered_after_content_children_are_created() {
        unsafe {
            let parent = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                100,
                100,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let surface = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WS_CHILD,
                0,
                0,
                100,
                100,
                Some(parent),
                Some(HMENU(MGR_LIST_SURFACE as *mut _)),
                None,
                None,
            )
            .unwrap();
            let list = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("LISTBOX"),
                windows::core::w!(""),
                WS_CHILD,
                2,
                2,
                96,
                96,
                Some(parent),
                Some(HMENU(MGR_LIST as *mut _)),
                None,
                None,
            )
            .unwrap();
            use windows::Win32::UI::WindowsAndMessaging::{DestroyWindow, GW_CHILD, GetWindow};
            // Establish the native creation behavior that caused the blank UI.
            assert_eq!(GetWindow(parent, GW_CHILD).unwrap(), surface);
            lower_surfaces(parent);
            assert_eq!(GetWindow(parent, GW_CHILD).unwrap(), list);
            DestroyWindow(parent).unwrap();
        }
    }
}
