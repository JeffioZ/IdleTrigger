//! Process picker: the modal window over the automation manager for
//! choosing process targets - search, filter, enrichment, preview, and
//! the executable browse. Extracted from automation_ui; the confirm seam
//! writes the editor's process selection via set_edit_procs.

use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};

use idletrigger_core::automation as auto;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::UpdateWindow;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use crate::automation_ui::{
    BUTTON_H, ED_CHOOSE, ED_GAP, ED_TRIGGER, WM_APP_PICKER_DESC, caption_key, choice_value,
    clear_editor_error, dpi_changed, draw_form_item, edit_procs, enable_control, layout_editor,
    lower_surfaces, present_control, refresh_tooltips, set_edit_procs, update_proc_summary,
};
use crate::{t_pub, theme, wide, window_text};

fn s(v: i32) -> i32 {
    crate::scale_pub(v)
}

fn set_text(hwnd: HWND, text: &str) {
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide(text).as_ptr()));
    }
}

fn get_dlg_item(parent: HWND, id: usize) -> HWND {
    unsafe { GetDlgItem(Some(parent), id as i32).unwrap_or_default() }
}

// Raw Win32 tokens that are missing or awkward in the windows crate
// surface (automation_ui's compat set).
const LBS_NOSEL: u32 = 0x4000;
const EN_CHANGE: u16 = 0x0300;
const EN_SETFOCUS: u16 = 0x0100;
const EN_KILLFOCUS: u16 = 0x0200;
const BN_CLICKED: u16 = 0;

fn fill_template(template: &str, values: &[&str]) -> String {
    idletrigger_core::i18n::format(template, values)
}

// Picker (Go idSearch 101 … idHelper 113).
pub(crate) const PK_SEARCH: usize = 400;
pub(crate) const PK_LIST: usize = 401;
pub(crate) const PK_CANCEL: usize = 402;
pub(crate) const PK_CONFIRM: usize = 403;
const PK_HEADING: usize = 404;
const PK_HELPER: usize = 405;
const PK_STATUS: usize = 406;
const PK_PRIVACY: usize = 407;
const PK_PREVIEW_TITLE: usize = 408;
const PK_PREVIEW: usize = 409;
pub(crate) const PK_REFRESH: usize = 412;
pub(crate) const PK_BROWSE: usize = 413;
/// Owner-draw column sort buttons (the listview header's replacement).
pub(crate) const PK_COL_BASE: usize = 414; // +0 name, +1 description, +2 instances
const PK_EMPTY: usize = 344;
/// Window-scoped debounce timer id for the search box filter.
const PK_FILTER_TIMER: usize = 1;
pub(crate) const PK_SEARCH_SURFACE: usize = 420;
pub(crate) const PK_LIST_SURFACE: usize = 421;
pub(crate) const PK_PREVIEW_SURFACE: usize = 422;
static PICKER_HWND: AtomicIsize = AtomicIsize::new(0);
static PK_LIST_HWND: AtomicIsize = AtomicIsize::new(0);
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
const PICKER_TEXTS: &[(usize, &str)] = &[
    (PK_HEADING, "process_picker_heading"),
    (PK_HELPER, "process_picker_helper"),
    (PK_PRIVACY, "process_picker_privacy"),
    (PK_REFRESH, "process_picker_refresh"),
    (PK_BROWSE, "process_picker_browse"),
    (PK_CONFIRM, "process_picker_confirm"),
    (PK_CANCEL, "common_cancel"),
];

// ===== Process picker =======================================================

#[derive(Clone)]
pub(crate) struct PickItem {
    pub(crate) target: auto::ProcessTarget,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) count: u32,
    search: String,
}

static PK_ITEMS: Mutex<Vec<PickItem>> = Mutex::new(Vec::new());
pub(crate) static PK_VISIBLE: Mutex<Vec<PickItem>> = Mutex::new(Vec::new());
pub(crate) static PK_SELECTED: Mutex<Vec<auto::ProcessTarget>> = Mutex::new(Vec::new());
pub(crate) static PK_SORT: Mutex<(i32, bool)> = Mutex::new((0, true));

/// True while the ListView is being refilled (Go p.populating): item-state
/// notifications during population must not recompute the selection, or
/// not-yet-inserted rows would drop out of it.
static PK_POPULATING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn show_picker(owner: HWND) {
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
        let ed = crate::automation_ui::editor_hwnd();
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
        let font = crate::kit::fonts::body();

        let form = crate::kit::form::FormWindow::create(crate::kit::form::FormSpec {
            class: windows::core::w!("IdleTriggerProcPicker"),
            kind: crate::kit::registry::Kind::ProcessPicker,
            title: t_pub("process_picker_title"),
            proc: Some(picker_proc),
            client_w: PK_W,
            client_h: PK_H,
            style: crate::kit::form::secondary_style(),
            // Center on the manager root: the picker's modal target is the
            // embedded editor pane, but a popup parented to a child window has
            // unreliable activation, so the OS-level parent is the root window.
            anchor: {
                let root = GetAncestor(owner, GA_ROOT);
                if root.is_invalid() { owner } else { root }
            },
            bg_brush: windows::Win32::Graphics::Gdi::GetSysColorBrush(
                windows::Win32::Graphics::Gdi::COLOR_WINDOW,
            ),
        });
        let pk = form.hwnd;
        PICKER_HWND.store(pk.0 as isize, Ordering::SeqCst);
        form.finish();

        let content_w = PK_W - 2 * PK_PAD;

        let mk_static =
            |id: usize, text: &str, font: windows::Win32::Graphics::Gdi::HFONT, y: i32| {
                crate::kit::controls::static_text(
                    pk,
                    id,
                    text,
                    font,
                    (PK_PAD, y, content_w, PK_TEXT_H),
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
        crate::kit::controls::section_card(
            pk,
            PK_SEARCH_SURFACE as i32,
            font,
            (PK_PAD, PK_SEARCH_Y, 370, PK_FIELD_H),
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
            crate::kit::controls::EM_SETMARGINS_RAW,
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
            crate::kit::controls::push_button(
                pk,
                id as i32,
                &t_pub(key),
                (x, PK_SEARCH_Y, w, PK_FIELD_H),
            );
        }

        // List surface card + ListView (Go SysListView32 report + checkboxes).
        crate::kit::controls::section_card(
            pk,
            PK_LIST_SURFACE as i32,
            font,
            (PK_PAD, PK_LIST_Y, content_w, PK_LIST_H),
        );

        // Column sort strip (the old listview header's job), quiet text
        // buttons in the family grammar riding the list card face.
        let col_widths = [250, 310, 82];
        let mut col_x = PK_PAD + 2;
        for (index, width) in col_widths.iter().enumerate() {
            // Empty captions: the owner-draw arm paints the localized
            // column titles itself.
            crate::kit::controls::push_button(
                pk,
                (PK_COL_BASE + index) as i32,
                "",
                (col_x, PK_LIST_Y + 2, *width, 24),
            );
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
        crate::kit::controls::section_card(
            pk,
            PK_PREVIEW_SURFACE as i32,
            font,
            (PK_PAD, PK_PREVIEW_Y, content_w, PK_PREVIEW_H),
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
            crate::kit::controls::push_button(
                pk,
                id as i32,
                &t_pub(key),
                (x, PK_BUTTONS_Y, PK_BUTTON_W, BUTTON_H),
            );
        }
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
                        let description =
                            crate::runtime::lock(&crate::automation_rules::DESCRIPTIONS)
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
                let mut descriptions = crate::runtime::lock(&crate::automation_rules::DESCRIPTIONS);
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
        choice_value(crate::automation_ui::editor_hwnd(), ED_TRIGGER).as_str(),
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
                let name = &crate::automation_rules::described_target(target);
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
    let ed = crate::automation_ui::editor_hwnd();
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
                    crate::runtime::lock(&crate::automation_rules::DESCRIPTIONS).clear();
                    *crate::runtime::lock(&PK_RESULT) = None;
                    *crate::runtime::lock(&PK_UPDATED) = None;
                    // The modal disable targeted the manager ROOT, not the
                    // embedded editor pane — re-enable the same window.
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
                        crate::automation_ui::mgr_hwnd(),
                        true,
                    );
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        },
    )
}

/// The picker half of the automation language refresh: relabels captions,
/// rebuilds the filter overlay text, and re-syncs the status lines.
pub fn refresh_language() {
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
        crate::automation_ui::dpi_changed(pk);
        update_selection_status(pk);
        update_preview(pk);
    }
}

/// The picker window handle: the automation family accessor's third slot.
pub fn picker_hwnd() -> HWND {
    HWND(PICKER_HWND.load(Ordering::SeqCst) as *mut _)
}
