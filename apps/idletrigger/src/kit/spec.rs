//! Declarative control specs: a window describes a page as data, and
//! `apply` either creates each control (first run) or relabels the
//! existing HWND by control id (language refresh). Geometry is explicit
//! logical rects and creation is its single authority — apply never
//! repositions, which keeps reused windows pixel-identical across DPI
//! ratio rescales (reflow-on-apply lands with the first window that
//! needs it). Value state (checkbox stores, edit contents, combo
//! selections) is never touched: captions and items only, with combo
//! selections restored by index, mirroring the hand-rolled refresh paths
//! this replaces.

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetDlgItem, GetWindowLongPtrW, GetWindowRect, SWP_NOACTIVATE, SWP_NOZORDER,
    SendMessageW, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, WM_GETTEXT, WM_GETTEXTLENGTH,
};
use windows::core::PCWSTR;

use crate::kit::{controls, fonts};

/// Caption source, resolved at apply time so a language refresh re-runs
/// the spec and re-labels.
pub enum Text {
    /// i18n key resolved via `t_pub`.
    Key(&'static str),
    /// Literal text ("Caps Lock", empty card faces).
    Literal(&'static str),
    /// Caption only set at creation; later applies never touch it (phase
    /// captions owned by the window, e.g. the update-check button).
    Init(&'static str),
    /// No caption management at all — the window fully owns the text
    /// (dynamic status lines).
    None,
}

/// Which kit font role a control's caption renders in.
#[derive(Clone, Copy)]
pub enum FontRole {
    Body,
    Section,
    /// Page header titles — constructed once the header pages migrate.
    #[allow(dead_code)]
    PageTitle,
}

impl FontRole {
    fn resolve(self) -> HFONT {
        match self {
            FontRole::Body => fonts::body(),
            FontRole::Section => fonts::section(),
            FontRole::PageTitle => fonts::page_title(),
        }
    }
}

pub enum ControlKind {
    Label {
        right: bool,
        notify: bool,
    },
    CheckBox,
    /// Owner-drawn push button.
    Button,
    /// Text hyperlink rendered as an owner-drawn button.
    Link,
    /// Owner-drawn section card surface pinned beneath its rows.
    SectionCard,
    /// Borderless field edit inset in a card well.
    Edit {
        numeric: bool,
    },
    /// Choice dropdown with translated label pairs; items re-run per
    /// refresh (index-preserving).
    Combo {
        items: fn() -> Vec<String>,
    },
    /// Choice dropdown from full row items (headers/actions allowed).
    /// `rebuild` picks the refresh policy: true re-runs `rows`
    /// index-preserving; false leaves the rows to the window's own
    /// value-preserving refresh (library dropdowns whose entries are
    /// file names, not translations).
    ComboRows {
        rows: fn() -> Vec<crate::choice::ChoiceItem>,
        rebuild: bool,
    },
}

pub struct ControlSpec {
    pub id: i32,
    pub kind: ControlKind,
    /// Logical rect in the parent's client space; fixed at creation.
    pub rect: (i32, i32, i32, i32),
    pub text: Text,
    pub font: FontRole,
}

impl ControlSpec {
    pub fn label(id: i32, text: Text, font: FontRole, rect: (i32, i32, i32, i32)) -> Self {
        Self {
            id,
            kind: ControlKind::Label {
                right: false,
                notify: false,
            },
            rect,
            text,
            font,
        }
    }

    /// Body label that also receives mouse hits for its hover tooltip.
    pub fn notify_label(id: i32, key: &'static str, rect: (i32, i32, i32, i32)) -> Self {
        Self {
            id,
            kind: ControlKind::Label {
                right: false,
                notify: true,
            },
            rect,
            text: Text::Key(key),
            font: FontRole::Body,
        }
    }

    pub fn checkbox(id: i32, key: &'static str, rect: (i32, i32, i32, i32)) -> Self {
        Self::checkbox_text(id, Text::Key(key), rect)
    }

    /// Checkbox with an explicit text source (literals like "Caps Lock").
    pub fn checkbox_text(id: i32, text: Text, rect: (i32, i32, i32, i32)) -> Self {
        Self {
            id,
            kind: ControlKind::CheckBox,
            rect,
            text,
            font: FontRole::Body,
        }
    }

    pub fn edit(id: i32, rect: (i32, i32, i32, i32), numeric: bool) -> Self {
        Self {
            id,
            kind: ControlKind::Edit { numeric },
            rect,
            text: Text::Literal(""),
            font: FontRole::Body,
        }
    }

    pub fn combo(id: i32, rect: (i32, i32, i32, i32), items: fn() -> Vec<String>) -> Self {
        Self {
            id,
            kind: ControlKind::Combo { items },
            rect,
            text: Text::Literal(""),
            font: FontRole::Body,
        }
    }

    /// Choice from full row items (headers/actions), whose entries the
    /// window refreshes itself by value (`rebuild: false`).
    pub fn combo_rows(
        id: i32,
        rect: (i32, i32, i32, i32),
        rows: fn() -> Vec<crate::choice::ChoiceItem>,
    ) -> Self {
        Self {
            id,
            kind: ControlKind::ComboRows {
                rows,
                rebuild: false,
            },
            rect,
            text: Text::Literal(""),
            font: FontRole::Body,
        }
    }

    pub fn button(id: i32, key: &'static str, rect: (i32, i32, i32, i32)) -> Self {
        Self {
            id,
            kind: ControlKind::Button,
            rect,
            text: Text::Key(key),
            font: FontRole::Body,
        }
    }

    /// Hyperlink with create-only text (phase captions own themselves).
    pub fn button_init(id: i32, key: &'static str, rect: (i32, i32, i32, i32)) -> Self {
        Self {
            id,
            kind: ControlKind::Button,
            rect,
            text: Text::Init(key),
            font: FontRole::Body,
        }
    }

    pub fn link(id: i32, text: &'static str, rect: (i32, i32, i32, i32)) -> Self {
        Self {
            id,
            kind: ControlKind::Link,
            rect,
            text: Text::Literal(text),
            font: FontRole::Body,
        }
    }

    pub fn card(id: i32, rect: (i32, i32, i32, i32)) -> Self {
        Self {
            id,
            kind: ControlKind::SectionCard,
            rect,
            text: Text::Literal(""),
            font: FontRole::Body,
        }
    }

    fn caption(&self) -> String {
        match &self.text {
            Text::Key(key) => crate::t_pub(key),
            Text::Literal(text) | Text::Init(text) => (*text).to_string(),
            Text::None => String::new(),
        }
    }

    /// Whether apply re-resolves and compares the caption on reuse.
    fn manages_caption(&self) -> bool {
        !matches!(self.text, Text::None | Text::Init(_))
    }
}

/// Reads a control's current caption for compare-first relabels.
fn control_text(control: HWND) -> String {
    unsafe {
        let len = SendMessageW(control, WM_GETTEXTLENGTH, Some(WPARAM(0)), Some(LPARAM(0)))
            .0
            .max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let copied = SendMessageW(
            control,
            WM_GETTEXT,
            Some(WPARAM(buf.len())),
            Some(LPARAM(buf.as_mut_ptr() as isize)),
        )
        .0 as usize;
        String::from_utf16_lossy(&buf[..copied.min(len)])
    }
}

unsafe fn create(parent: HWND, spec: &ControlSpec, text: &str) {
    unsafe {
        match &spec.kind {
            ControlKind::Label { right, .. } => {
                let label = controls::label(
                    parent,
                    spec.id,
                    text,
                    spec.font.resolve(),
                    spec.rect,
                    *right,
                );
                if matches!(spec.kind, ControlKind::Label { notify: true, .. }) {
                    // SS_NOTIFY makes the static hit-testable so its hover
                    // tooltip can fire (plain statics are mouse-transparent).
                    let _ = SetWindowLongPtrW(
                        label,
                        GWL_STYLE,
                        (GetWindowLongPtrW(label, GWL_STYLE) | 0x00000100) as _, // SS_NOTIFY
                    );
                }
            }
            ControlKind::CheckBox => {
                controls::checkbox(parent, spec.id, text, spec.rect);
            }
            ControlKind::Button => {
                controls::push_button(parent, spec.id, text, spec.rect);
            }
            ControlKind::Link => {
                controls::link(parent, spec.id, text, spec.rect);
            }
            ControlKind::SectionCard => {
                controls::section_card(parent, spec.id, spec.font.resolve(), spec.rect);
            }
            ControlKind::Edit { numeric } => {
                controls::edit(parent, spec.id, spec.rect, *numeric);
            }
            ControlKind::Combo { items } => {
                let labels = items();
                controls::combo(parent, spec.id, spec.rect, &labels);
            }
            ControlKind::ComboRows { rows, .. } => {
                let rows = rows();
                controls::combo_items(parent, spec.id, spec.rect, &rows);
            }
        }
    }
}

/// Creates missing controls and relabels existing ones (by control id).
/// Idempotent; run under the caller's DPI scope.
pub unsafe fn apply(parent: HWND, specs: &[ControlSpec]) {
    unsafe {
        for spec in specs {
            let existing = GetDlgItem(Some(parent), spec.id).unwrap_or_default();
            if existing.is_invalid() {
                let text = spec.caption();
                create(parent, spec, &text);
                continue;
            }
            match &spec.kind {
                ControlKind::Label { .. }
                | ControlKind::CheckBox
                | ControlKind::Button
                | ControlKind::Link => {
                    if spec.manages_caption() {
                        let text = spec.caption();
                        if control_text(existing) != text {
                            let _ = SetWindowTextW(existing, PCWSTR(crate::wide(&text).as_ptr()));
                        }
                    }
                }
                ControlKind::Combo { items } => {
                    // Index-preserving item rebuild (refresh parity).
                    let selected = crate::choice::selection(existing);
                    let labels = items();
                    let pairs: Vec<(String, String)> = labels
                        .into_iter()
                        .map(|label| (label.clone(), label))
                        .collect();
                    crate::choice::set_items(existing, &pairs);
                    crate::choice::select_index(existing, selected);
                }
                ControlKind::ComboRows { rebuild, rows } => {
                    if *rebuild {
                        let selected = crate::choice::selection(existing);
                        let fresh = rows();
                        crate::choice::set_rows(existing, &fresh);
                        crate::choice::select_index(existing, selected);
                    }
                }
                // Card faces carry no caption; edit contents are value
                // state and belong to the populate/draft layer.
                ControlKind::SectionCard | ControlKind::Edit { .. } => {}
            }
        }
    }
}

/// The relayout flavor of [`apply`] for pages whose geometry depends on
/// language: existing controls also move to the spec rect when it changed
/// (compare-first, so unchanged rows take no redundant SetWindowPos). Run
/// under the caller's DPI scope; the rect must be computed with the same
/// language state as the captions.
pub unsafe fn apply_relayout(parent: HWND, specs: &[ControlSpec]) {
    unsafe {
        for spec in specs {
            let existing = GetDlgItem(Some(parent), spec.id).unwrap_or_default();
            if existing.is_invalid() {
                let text = spec.caption();
                create(parent, spec, &text);
                continue;
            }
            if rect_changed(parent, existing, spec.rect) {
                let (x, y, w, h) = spec.rect;
                let _ = SetWindowPos(
                    existing,
                    None,
                    crate::scale_pub(x),
                    crate::scale_pub(y),
                    crate::scale_pub(w),
                    crate::scale_pub(h),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
        apply(parent, specs);
    }
}

/// Whether a control's physical client rect differs from the logical spec
/// rect at the current scale.
fn rect_changed(parent: HWND, control: HWND, rect: (i32, i32, i32, i32)) -> bool {
    unsafe {
        let mut wr = RECT::default();
        if GetWindowRect(control, &mut wr).is_err() {
            return false;
        }
        let mut origin = windows::Win32::Foundation::POINT {
            x: wr.left,
            y: wr.top,
        };
        let _ = windows::Win32::Graphics::Gdi::ScreenToClient(parent, &mut origin);
        let (x, y, w, h) = rect;
        origin.x != crate::scale_pub(x)
            || origin.y != crate::scale_pub(y)
            || (wr.right - wr.left) != crate::scale_pub(w)
            || (wr.bottom - wr.top) != crate::scale_pub(h)
    }
}
