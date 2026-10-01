//! Regression coverage for retained windows whose theme changes while hidden.
use super::*;
use windows::Win32::Graphics::Gdi::*;
#[cfg(target_pointer_width = "32")]
use windows::Win32::UI::WindowsAndMessaging::GetClassLongW as GetClassLongPtrW;
use windows::Win32::UI::WindowsAndMessaging::*;

#[cfg(feature = "devtools")]
#[path = "popup_tests.rs"]
mod popup_tests;

fn pump(milliseconds: u64) {
    let until = std::time::Instant::now() + Duration::from_millis(milliseconds);
    while std::time::Instant::now() < until {
        let mut msg = msg_default();
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(feature = "devtools")]
fn capture_states(control: HWND, folder: &std::path::Path, prefix: &str) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
    unsafe {
        nativeform::keyboard_navigation();
        for (name, message, value) in [
            ("focus", WM_MOUSELEAVE_AUDIT, 0),
            ("focus-hover", WM_MOUSEMOVE, 0),
            ("focus-pressed", BM_SETSTATE, 1),
            ("hover", WM_MOUSEMOVE, 0),
            ("pressed", BM_SETSTATE, 1),
            ("normal", BM_SETSTATE, 0),
        ] {
            SendMessageW(control, BM_SETSTATE, Some(WPARAM(0)), None);
            SendMessageW(control, WM_MOUSELEAVE_AUDIT, None, None);
            let _ = SetFocus(if name.starts_with("focus") {
                Some(control)
            } else {
                None
            });
            SendMessageW(
                control,
                message,
                Some(WPARAM(value)),
                Some(LPARAM(0x00050005)),
            );
            if name.starts_with("focus") {
                // Mouse movement hides keyboard cues. Model keyboard focus
                // arriving after the pointer is already over the control.
                nativeform::keyboard_navigation();
            }
            present_layout(control);
            capture::capture_client_bmp(control, &folder.join(format!("{prefix}-{name}.bmp")))
                .unwrap();
        }
        let _ = EnableWindow(control, false);
        present_layout(control);
        capture::capture_client_bmp(control, &folder.join(format!("{prefix}-disabled.bmp")))
            .unwrap();
        let _ = EnableWindow(control, true);
        SendMessageW(control, WM_MOUSELEAVE_AUDIT, None, None);
        let _ = SetFocus(None);
    }
}
#[cfg(feature = "devtools")]
const WM_MOUSELEAVE_AUDIT: u32 = windows::Win32::UI::Controls::WM_MOUSELEAVE;

#[cfg(feature = "devtools")]
fn capture_native_popup(owner: HWND, path: std::path::PathBuf, menu: bool, open: impl FnOnce()) {
    let owner = owner.0 as isize;
    let capture = std::thread::spawn(move || {
        let class = if menu { w!("#32768") } else { w!("#32770") };
        let mut result = Err("native popup did not appear".to_string());
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(20));
            unsafe {
                let mut previous = None;
                while let Ok(window) = FindWindowExW(None, previous, class, None) {
                    previous = Some(window);
                    let mut pid = 0;
                    GetWindowThreadProcessId(window, Some(&mut pid));
                    if pid != std::process::id() || !IsWindowVisible(window).as_bool() {
                        continue;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    result =
                        capture::capture_print_client_bmp(window, &path).map_err(|e| e.to_string());
                    let _ = PostMessageW(
                        Some(window),
                        if menu { WM_CANCELMODE } else { WM_CLOSE },
                        WPARAM(0),
                        LPARAM(0),
                    );
                    break;
                }
            }
            if result.is_ok() {
                break;
            }
        }
        unsafe {
            let _ = PostMessageW(
                Some(HWND(owner as *mut _)),
                WM_CANCELMODE,
                WPARAM(0),
                LPARAM(0),
            );
        }
        result
    });
    open();
    if let Err(error) = capture.join().unwrap() {
        eprintln!("capture limitation: {error}");
    }
}

fn child_geometry(panel: HWND) -> Vec<(isize, RECT)> {
    unsafe extern "system" fn collect(control: HWND, data: LPARAM) -> windows::core::BOOL {
        unsafe {
            let entries = &mut *(data.0 as *mut Vec<(isize, RECT)>);
            let mut rect = RECT::default();
            GetWindowRect(control, &mut rect).unwrap();
            entries.push((control.0 as isize, rect));
        }
        windows::core::BOOL(1)
    }
    let mut entries = Vec::new();
    unsafe {
        let _ = EnumChildWindows(
            Some(panel),
            Some(collect),
            LPARAM(&mut entries as *mut _ as isize),
        );
    }
    entries
}

fn assert_static_erase_is_deferred(control: HWND) {
    unsafe {
        // An erase request must not expose a separate flat fill before the
        // parent's buffered WM_DRAWITEM supplies background and text together.
        let screen = GetDC(Some(control));
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, 2, 2);
        assert!(!dc.is_invalid() && !bitmap.is_invalid());
        let old = SelectObject(dc, HGDIOBJ(bitmap.0));
        let sentinel = windows::Win32::Foundation::COLORREF(0xFE01FE);
        SetPixel(dc, 0, 0, sentinel);
        let result = SendMessageW(
            control,
            WM_ERASEBKGND,
            Some(WPARAM(dc.0 as usize)),
            Some(LPARAM(0)),
        );
        let pixel = GetPixel(dc, 0, 0);
        let _ = SelectObject(dc, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(dc);
        let _ = ReleaseDC(Some(control), screen);
        assert_eq!(result, LRESULT(1));
        assert_eq!(pixel, sentinel, "static erased before its complete frame");
    }
}

#[cfg(feature = "devtools")]
fn assert_form_surfaces_are_buffered(window: HWND) {
    let mut checked = 0;
    for (control, _) in child_geometry(window) {
        let control = HWND(control as *mut _);
        let mut class = [0u16; 32];
        let len = unsafe { GetClassNameW(control, &mut class) };
        if String::from_utf16_lossy(&class[..len as usize]).eq_ignore_ascii_case("STATIC")
            && unsafe { GetWindowLongW(control, GWL_STYLE) } as u32 & 0x1f == 13
        {
            checked += 1;
            assert_static_erase_is_deferred(control);
        }
    }
    assert!(checked > 0, "no owner-drawn form surfaces checked");
}

#[test]
fn hidden_theme_reopen_preserves_background_and_geometry() {
    const CHILD: &str = "IDLETRIGGER_TEST_HIDDEN_THEME_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // Process isolation protects globals, not desktop focus. Serialize
        // the complete child lifetime with other native UI tests.
        let _ui_test = crate::runtime::lock(&CONFIG_TEST_LOCK);
        for language in ["en", "zh-CN"] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "render_tests::hidden_theme_reopen_preserves_background_and_geometry",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("IDLETRIGGER_TEST_LANGUAGE", language)
                .spawn()
                .unwrap();
            // Native modal dialogs and cross-thread WM_PRINT can wait on
            // USER32. Bound the isolated child, including capture sessions.
            let deadline = std::time::Instant::now() + Duration::from_secs(120);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("hidden-theme child timed out for {language}");
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            assert!(status.success(), "hidden-theme child failed: {status}");
        }
        return;
    }
    let record = std::env::var_os("IDLETRIGGER_TEST_RECORD_FRAMES").is_some();
    let language = std::env::var("IDLETRIGGER_TEST_LANGUAGE").unwrap_or_else(|_| "en".into());
    *crate::runtime::lock(&CONFIG) = Some(config::Config {
        language: language.clone(),
        ..Default::default()
    });
    *I18N.write().unwrap() = Some(I18n::load(&language));
    theme::force_dark(false);
    create_windows();
    let panel = hwnd(&PANEL);
    let button = unsafe { GetDlgItem(Some(panel), IDC_NOSLEEP as i32) }.unwrap();
    let native_brush = unsafe { GetClassLongPtrW(button, GCLP_HBRBACKGROUND) };
    let mut stale_brushes = 0;
    let mut geometry = None;
    let mut controls = None;
    for (index, dark) in [false, true, false, true].into_iter().enumerate() {
        unsafe {
            let _ = ShowWindow(panel, SW_HIDE);
        }
        pump(if record { 150 } else { 5 });
        theme::force_dark(dark);
        theme::apply_to_all();
        refresh_status();
        assert_eq!(
            unsafe { GetClassLongPtrW(button, GCLP_HBRBACKGROUND) },
            native_brush,
            "theme refresh changed the native button class background"
        );
        // USER32's fallback background must agree with WM_ERASEBKGND even
        // when there has been no visible paint in the new theme yet.
        let brush =
            unsafe { GetClassLongPtrW(panel, GCLP_HBRBACKGROUND) } as *mut core::ffi::c_void;
        if brush != theme::bg_brush().0 {
            stale_brushes += 1;
        }
        eprintln!(
            "hidden-theme phase {index}: dark={dark}, stale-brush={}",
            brush != theme::bg_brush().0
        );
        show_panel();
        pump(if record { 900 } else { 10 });
        #[cfg(feature = "devtools")]
        if let Some(folder) = std::env::var_os("IDLETRIGGER_TEST_CAPTURE_DIR") {
            capture::capture_client_bmp(
                panel,
                &std::path::PathBuf::from(folder).join(format!("{language}-{index}.bmp")),
            )
            .unwrap();
        }
        let mut rect = RECT::default();
        unsafe {
            GetClientRect(panel, &mut rect).unwrap();
        }
        if let Some(original) = geometry {
            assert_eq!(rect, original);
        } else {
            geometry = Some(rect);
        }
        let current = child_geometry(panel);
        if let Some(original) = &controls {
            assert_eq!(
                &current, original,
                "theme change moved or replaced controls"
            );
        } else {
            controls = Some(current);
        }
        // Final rendered background of the actual controls and static text
        // must use the new palette; sample a margin outside their glyphs.
        // Card rows sit on the section card face; the section title lives
        // outside its card on the window background.
        for (id, expected) in [
            (IDC_NOSLEEP, theme::palette().surface),
            (STATIC_SUBTITLE_BASE, theme::palette().surface),
            (STATIC_SECTION_BASE, theme::bg_color()),
        ] {
            let control = unsafe { GetDlgItem(Some(panel), id as i32) }.unwrap();
            unsafe {
                let dc = GetDC(Some(control));
                let pixel = GetPixel(dc, 0, 0);
                let _ = ReleaseDC(Some(control), dc);
                assert_eq!(pixel.0, expected, "control {id} has a stale background");
            }
        }
        for id in [STATIC_SECTION_BASE, STATIC_SUBTITLE_BASE] {
            let control = unsafe { GetDlgItem(Some(panel), id as i32) }.unwrap();
            assert_eq!(
                unsafe { GetWindowLongW(control, GWL_STYLE) } as u32 & WS_TABSTOP.0,
                0
            );
            assert_static_erase_is_deferred(control);
        }
    }
    #[cfg(feature = "devtools")]
    if let Some(folder) = std::env::var_os("IDLETRIGGER_TEST_CAPTURE_DIR") {
        let folder = std::path::PathBuf::from(folder);
        settings_ui::show();
        let settings = settings_ui::devtools_capture_hwnd();
        assert_form_surfaces_are_buffered(settings);
        for dark in [false, true] {
            theme::force_dark(dark);
            theme::apply_to_all();
            for page in 0..4 {
                settings_ui::devtools_select_page(page);
                present_layout(settings);
                pump(20);
                popup_tests::tooltips(
                    settings,
                    &folder,
                    &format!("{language}-settings-{dark}-{page}"),
                );
                capture::capture_client_bmp(
                    settings,
                    &folder.join(format!("{language}-settings-{dark}-{page}.bmp")),
                )
                .unwrap();
                if page == 2 {
                    for id in [157, 163] {
                        capture_states(
                            unsafe { GetDlgItem(Some(settings), id) }.unwrap(),
                            &folder,
                            &format!("{language}-{dark}-settings-control-{id}"),
                        );
                    }
                }
            }
        }
        popup_tests::discard(
            settings,
            119,
            "999",
            "settings_discard_title",
            "settings_discard_confirm",
            &folder,
            &language,
        );
        for dark in [false, true] {
            theme::force_dark(dark);
            theme::apply_to_all();
            popup_tests::tooltips(panel, &folder, &format!("{language}-panel-{dark}"));
            for id in [IDC_NOSLEEP, IDC_EXIT_BUTTON, IDC_SETTINGS_BUTTON] {
                capture_states(
                    unsafe { GetDlgItem(Some(panel), id as i32) }.unwrap(),
                    &folder,
                    &format!("{language}-{dark}-control-{id}"),
                );
            }
            unsafe {
                show_system_controls_menu(panel);
            }
            pump(20);
            capture::capture_client_bmp(
                choice::open_popup(),
                &folder.join(format!("{language}-system-menu-{dark}.bmp")),
            )
            .unwrap();
            choice::close(false);
            capture_native_popup(
                panel,
                folder.join(format!("{language}-tray-{dark}.bmp")),
                true,
                || crate::tray::show_context_menu(panel),
            );
            automation_ui::show();
            let manager = automation_ui::theme_hwnds()[0];
            assert_form_surfaces_are_buffered(manager);
            pump(20);
            popup_tests::tooltips(manager, &folder, &format!("{language}-manager-{dark}"));
            capture::capture_client_bmp(
                manager,
                &folder.join(format!("{language}-tasks-{dark}.bmp")),
            )
            .unwrap();
            automation_ui::devtools_show_editor();
            let editor = automation_ui::theme_hwnds()[1];
            assert_form_surfaces_are_buffered(editor);
            pump(20);
            popup_tests::tooltips(editor, &folder, &format!("{language}-editor-{dark}"));
            let detail = automation_ui::test_process_details_fixture();
            popup_tests::dialog(
                &t("automation_process_details_title"),
                &detail,
                IDOK,
                IDOK,
                folder.join(format!("{language}-process-info-{dark}.bmp")),
                || unsafe {
                    SendMessageW(editor, WM_COMMAND, Some(WPARAM(363)), None);
                },
            );
            capture::capture_client_bmp(
                editor,
                &folder.join(format!("{language}-editor-{dark}.bmp")),
            )
            .unwrap();
            automation_ui::devtools_open_trigger_choice();
            pump(20);
            capture::capture_client_bmp(
                choice::open_popup(),
                &folder.join(format!("{language}-trigger-menu-{dark}.bmp")),
            )
            .unwrap();
            choice::close(false);
            automation_ui::devtools_show_picker();
            let picker = automation_ui::theme_hwnds()[2];
            assert_form_surfaces_are_buffered(picker);
            pump(100);
            popup_tests::tooltips(picker, &folder, &format!("{language}-picker-{dark}"));
            capture::capture_client_bmp(
                picker,
                &folder.join(format!("{language}-picker-{dark}.bmp")),
            )
            .unwrap();
            // Search through the native edit; no process is selected or acted on.
            capture_native_popup(
                picker,
                folder.join(format!("{language}-file-dialog-{dark}.bmp")),
                false,
                || unsafe {
                    SendMessageW(picker, WM_COMMAND, Some(WPARAM(413)), None);
                },
            );
            unsafe {
                SetWindowTextW(
                    GetDlgItem(Some(picker), 400).unwrap(),
                    w!("__no_matching_process__"),
                )
                .unwrap();
            }
            pump(400);
            capture::capture_client_bmp(
                picker,
                &folder.join(format!("{language}-picker-empty-{dark}.bmp")),
            )
            .unwrap();
            unsafe {
                DestroyWindow(picker).unwrap();
            }
            for (action, trigger) in [
                ("stay_awake", "time_window"),
                ("lock", "once"),
                ("lock", "daily"),
                ("lock", "weekly"),
                ("lock", "process_started"),
                ("lock", "process_exited"),
            ] {
                for (id, value) in [(313, action), (315, trigger)] {
                    let control = unsafe { GetDlgItem(Some(editor), id) }.unwrap();
                    choice::select_value(control, value).unwrap();
                    unsafe {
                        SendMessageW(
                            editor,
                            WM_COMMAND,
                            Some(WPARAM((CBN_SELCHANGE as usize) << 16 | id as usize)),
                            Some(LPARAM(control.0 as isize)),
                        );
                    }
                }
                present_layout(editor);
                pump(20);
                capture::capture_client_bmp(
                    editor,
                    &folder.join(format!("{language}-editor-{trigger}-{dark}.bmp")),
                )
                .unwrap();
            }
            popup_tests::discard(
                editor,
                311,
                "audit draft",
                "automation_discard_title",
                "automation_discard_confirm",
                &folder,
                &format!("{language}-editor-{dark}"),
            );
            automation_ui::devtools_seed_demo_rule();
            let baseline = crate::runtime::lock(&automation::RULES).clone();
            unsafe {
                SendMessageW(
                    GetDlgItem(Some(manager), 300).unwrap(),
                    LB_SETCURSEL,
                    Some(WPARAM(0)),
                    None,
                );
            }
            popup_tests::dialog(
                &t("automation_delete_title"),
                &t("automation_delete_confirm").replace("%s", &baseline[0].name),
                IDNO,
                IDNO,
                folder.join(format!("{language}-delete-no-{dark}.bmp")),
                || unsafe {
                    SendMessageW(manager, WM_COMMAND, Some(WPARAM(303)), None);
                },
            );
            assert_eq!(*crate::runtime::lock(&automation::RULES), baseline);
            crate::runtime::lock(&automation::RULES).clear();
            unsafe {
                DestroyWindow(manager).unwrap();
            }
            devtools::WARNING_PREVIEW.store(true, Ordering::SeqCst);
            popups_show_warning_preview();
            pump(20);
            capture::capture_client_bmp(
                hwnd(&WARNING),
                &folder.join(format!("{language}-idle-warning-{dark}.bmp")),
            )
            .unwrap();
            hide_warning("audit preview");
            popups::create();
            *crate::runtime::lock(&automation::PENDING_ACTION) = Some(automation::PendingAction {
                rule: None,
                action: "restart".into(),
                seconds: 600,
                rule_id: "audit-preview".into(),
                once_date: None,
            });
            popups::show_pending();
            pump(20);
            capture::capture_client_bmp(
                hwnd(&ACTION_WARN_HWND),
                &folder.join(format!("{language}-action-warning-{dark}.bmp")),
            )
            .unwrap();
            unsafe {
                SendMessageW(hwnd(&ACTION_WARN_HWND), WM_CLOSE, None, None);
                DestroyWindow(hwnd(&ACTION_WARN_HWND)).unwrap();
            }
            for vk in [0x14, 0x90, 0x91] {
                for on in [false, true] {
                    for dpi in [96, 144, 192] {
                        popups::capture_lock_preview(
                            dpi,
                            vk,
                            on,
                            &folder.join(format!("{language}-lock-{vk}-{on}-{dark}-{dpi}.bmp")),
                        )
                        .unwrap();
                    }
                }
            }
            let body = format!(
                "{}\n\nAudit: simulated failure",
                t("automation_overview_empty")
            );
            popup_tests::dialog(
                &t("app_title"),
                &body,
                IDOK,
                IDOK,
                folder.join(format!("{language}-native-warning-{dark}.bmp")),
                || warn_dialog("", &body),
            );
        }
    }
    unsafe {
        DestroyWindow(panel).unwrap();
        DestroyWindow(hwnd(&HIDDEN)).unwrap();
    }
    assert_eq!(
        stale_brushes, 0,
        "hidden theme changes left the original class brush installed"
    );
}

/// The pill switch must render smooth (anti-aliased), correctly proportioned,
/// and palette-driven at every DPI scale and in both themes — the three
/// failure modes of the abandoned first attempt. Renders off a screen into a
/// memory bitmap and asserts geometry, colors, and edge gradients directly.
#[test]
fn switch_paints_smooth_and_aligned_at_every_dpi_and_theme() {
    let _ui_test = crate::runtime::lock(&CONFIG_TEST_LOCK);
    unsafe {
        for dark in [false, true] {
            theme::force_dark(dark);
            let p = theme::palette();
            for scale in [96, 120, 144, 192] {
                let width = paint::sp(SWITCH_HIT_W, scale);
                let height = paint::sp(BUTTON_H, scale);
                let screen = GetDC(None);
                let dc = CreateCompatibleDC(Some(screen));
                let bitmap = CreateCompatibleBitmap(screen, width, height);
                assert!(!dc.is_invalid() && !bitmap.is_invalid());
                let old = SelectObject(dc, HGDIOBJ(bitmap.0));
                let bounds = RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                };
                // Same metrics as paint::draw_switch.
                let track_w = paint::sp(40, scale);
                let track_h = paint::sp(20, scale);
                let thumb_d = paint::sp(16, scale);
                let inset = ((track_h - thumb_d) / 2).max(1);
                let track_left = (width - track_w) / 2;
                let center_y = height / 2;

                let render = |active: bool| {
                    // Sentinel fill proves the painter covers its bounds.
                    let sentinel = windows::Win32::Foundation::COLORREF(0x00FF00FF);
                    let brush = CreateSolidBrush(sentinel);
                    FillRect(dc, &bounds, brush);
                    let _ = DeleteObject(HGDIOBJ(brush.0));
                    paint::draw_switch(
                        dc,
                        &bounds,
                        p,
                        p.window_bg,
                        paint::ControlState {
                            active,
                            ..Default::default()
                        },
                        scale,
                    );
                };
                let px = |x: i32, y: i32| GetPixel(dc, x, y).0;

                render(false);
                // Corners outside the pill keep the themed background: no
                // bleed outside the track bounds.
                assert_eq!(px(0, 0), p.window_bg, "off: corner bleed at {scale}");
                let off_thumb_cx = track_left + inset + thumb_d / 2;
                let track_cx = track_left + track_w - inset - thumb_d / 2;
                assert_eq!(
                    px(off_thumb_cx, center_y),
                    p.accent_text,
                    "off: thumb ink at {scale}/{dark}"
                );
                // No thumb ink outside the circle: a square pre-fill (or
                // its corner dots poking past the pill track) leaves ink
                // farther from the thumb center than its radius.
                let track_top = (height - track_h) / 2;
                let cx = track_left + inset + thumb_d / 2;
                let cy = track_top + inset + thumb_d / 2;
                let radius = thumb_d / 2;
                for y in (track_top + inset - 1)..=(track_top + inset + thumb_d + 1) {
                    for x in (track_left + inset - 1)..=(track_left + inset + thumb_d + 1) {
                        let dx = x - cx;
                        let dy = y - cy;
                        assert!(
                            !(px(x, y) == p.accent_text
                                && dx * dx + dy * dy > (radius + 1) * (radius + 1)),
                            "off: thumb ink outside the circle at ({x},{y}) {scale}/{dark}"
                        );
                    }
                }
                assert_eq!(
                    px(track_cx, center_y),
                    p.switch_track,
                    "off: track ink at {scale}/{dark}"
                );
                // Off-hover: the track tints toward the accent.
                paint::draw_switch(
                    dc,
                    &bounds,
                    p,
                    p.surface,
                    paint::ControlState {
                        hovered: true,
                        ..Default::default()
                    },
                    scale,
                );
                assert_eq!(
                    px(track_cx, center_y),
                    p.switch_track_hover,
                    "off-hover: track ink at {scale}/{dark}"
                );
                paint::draw_switch(
                    dc,
                    &bounds,
                    p,
                    p.surface,
                    paint::ControlState {
                        active: true,
                        ..Default::default()
                    },
                    scale,
                );
                assert_eq!(
                    px(track_left + inset + thumb_d / 2, center_y),
                    p.accent,
                    "on: track ink again at {scale}/{dark}"
                );
                paint::draw_switch(
                    dc,
                    &bounds,
                    p,
                    p.surface,
                    paint::ControlState {
                        active: true,
                        hovered: true,
                        ..Default::default()
                    },
                    scale,
                );
                assert_eq!(
                    px(track_left + inset + thumb_d / 2, center_y),
                    p.switch_track_on_hover,
                    "on-hover: track ink at {scale}/{dark}"
                );
                paint::draw_switch(
                    dc,
                    &bounds,
                    p,
                    p.surface,
                    paint::ControlState::default(),
                    scale,
                );
                // Anti-aliasing: the pill and thumb edges must pass through
                // blended pixels — a hard-edged GDI fallback has none.
                // Sample three rows around the center and the two columns
                // beyond each track end, where curvature guarantees
                // fractional coverage somewhere.
                let known = |c: u32| c == p.window_bg || c == p.switch_track || c == p.accent_text;
                let blends = |known: &dyn Fn(u32) -> bool| -> i32 {
                    let mut count = 0;
                    // A band deep into the corner arcs: the straight edges
                    // may land on integer pixels and blend nothing even
                    // while anti-aliasing is on, but the arcs always do.
                    let band = (thumb_d / 4).max(2);
                    for y in (center_y - band)..=(center_y + band) {
                        for x in (track_left - 2)..(track_left + track_w + 2) {
                            if !known(px(x, y)) {
                                count += 1;
                            }
                        }
                    }
                    count
                };
                let blends_off = blends(&known);
                assert!(
                    blends_off >= 3,
                    "off: no anti-aliased edges at {scale}/{dark}"
                );

                render(true);
                assert_eq!(px(0, 0), p.window_bg, "on: corner bleed at {scale}");
                let on_thumb_cx = track_left + track_w - inset - thumb_d / 2;
                assert_eq!(
                    px(on_thumb_cx, center_y),
                    p.accent_text,
                    "on: thumb ink at {scale}/{dark}"
                );
                let track_top = (height - track_h) / 2;
                let cx = track_left + track_w - inset - thumb_d / 2;
                let cy = track_top + inset + thumb_d / 2;
                let radius = thumb_d / 2;
                for y in (track_top + inset - 1)..=(track_top + inset + thumb_d + 1) {
                    for x in (track_left + track_w - inset - thumb_d - 1)
                        ..=(track_left + track_w - inset + 1)
                    {
                        let dx = x - cx;
                        let dy = y - cy;
                        assert!(
                            !(px(x, y) == p.accent_text
                                && dx * dx + dy * dy > (radius + 1) * (radius + 1)),
                            "on: thumb ink outside the circle at ({x},{y}) {scale}/{dark}"
                        );
                    }
                }
                assert_eq!(
                    px(track_left + inset + thumb_d / 2, center_y),
                    p.accent,
                    "on: track ink at {scale}/{dark}"
                );
                let known_on = |c: u32| c == p.window_bg || c == p.accent || c == p.accent_text;
                let blends_on = blends(&known_on);
                assert!(
                    blends_on >= 3,
                    "on: no anti-aliased edges at {scale}/{dark}"
                );

                // The thumb travels the full track: find its left edge in the
                // off render and its right edge in the on render by scanning
                // for the thumb ink just outside its known core.
                let ink_left_edge = |from: i32, ink: u32| -> i32 {
                    let mut x = from;
                    while x < track_left + track_w && px(x, center_y) != ink {
                        x += 1;
                    }
                    x
                };
                render(false);
                let off_left = ink_left_edge(track_left, p.accent_text);
                render(true);
                // On-state thumb ink is accent_text; the leftmost accent_text
                // pixel marks where the thumb begins its travel home.
                let on_left = ink_left_edge(track_left, p.accent_text);
                let travel = track_w - thumb_d - 2 * inset;
                let measured = on_left - off_left;
                assert!(
                    (measured - travel).abs() <= 2,
                    "thumb travel {measured} != {travel} at {scale}"
                );

                SelectObject(dc, old);
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                let _ = DeleteDC(dc);
                let _ = ReleaseDC(None, screen);
            }
        }
    }
}

/// Regression: header action links must be painted in the panel's first
/// visible frame. An early SetWindowPos z-order raise on never-painted
/// owner-draw buttons ate their initial WM_DRAWITEM, leaving 管理/修复
/// invisible until a hover invalidated them.
#[test]
fn header_links_paint_in_the_first_visible_frame() {
    const CHILD: &str = "IDLETRIGGER_TEST_HEADER_LINK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let _ui_test = crate::runtime::lock(&CONFIG_TEST_LOCK);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "render_tests::header_links_paint_in_the_first_visible_frame",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("header-link child timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(status.success(), "header-link child failed: {status}");
        return;
    }
    let _ui_test = crate::runtime::lock(&CONFIG_TEST_LOCK);
    *crate::runtime::lock(&CONFIG) = Some(config::Config::default());
    *I18N.write().unwrap() = Some(I18n::load("zh-CN"));
    theme::force_dark(false);
    create_windows();
    let panel = hwnd(&PANEL);
    show_panel();
    pump(80);
    unsafe {
        let mut visible = 0;
        for id in [IDC_MANAGE_BUTTON, IDC_THEME_REPAIR] {
            let link = GetDlgItem(Some(panel), id as i32).unwrap();
            let dc = GetDC(Some(link));
            let mut rect = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(link, &mut rect);
            let bg = theme::bg_color();
            for x in rect.left..rect.right {
                for y in rect.top..rect.bottom {
                    if GetPixel(dc, x, y).0 != bg {
                        visible += 1;
                    }
                }
            }
            let _ = ReleaseDC(Some(link), dc);
        }
        // Link-styled controls show the hand cursor: forwarding a child's
        // WM_SETCURSOR to the panel must install IDC_HAND.
        let manage = GetDlgItem(Some(panel), IDC_MANAGE_BUTTON as i32).unwrap();
        let hand = LoadCursorW(None, windows::Win32::UI::WindowsAndMessaging::IDC_HAND).unwrap();
        let _ = SendMessageW(
            panel,
            windows::Win32::UI::WindowsAndMessaging::WM_SETCURSOR,
            Some(WPARAM(manage.0 as usize)),
            Some(LPARAM(0x0020_0001)), // HTCLIENT | WM_MOUSEMOVE
        );
        let cursor = windows::Win32::UI::WindowsAndMessaging::GetCursor();
        tray::remove();
        DestroyWindow(panel).unwrap();
        DestroyWindow(hwnd(&HIDDEN)).unwrap();
        assert!(
            visible > 20,
            "header links not painted in the first frame (visible pixels: {visible})"
        );
        assert_eq!(
            cursor.0, hand.0,
            "hovering a header link must set the hand cursor"
        );
    }
}

/// Regression: clicking the header repair link must surface visible
/// feedback immediately. The repair itself (theme apply + two broadcast
/// rounds) takes ~10s, and its completion notice lives for four seconds —
/// without the instant "repairing" line the click looks dead.
#[test]
fn repair_link_click_surfaces_notice() {
    const CHILD: &str = "IDLETRIGGER_TEST_REPAIR_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let _ui_test = crate::runtime::lock(&CONFIG_TEST_LOCK);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "render_tests::repair_link_click_surfaces_notice",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("repair child timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(status.success(), "repair child failed: {status}");
        return;
    }
    let _ui_test = crate::runtime::lock(&CONFIG_TEST_LOCK);
    *crate::runtime::lock(&CONFIG) = Some(config::Config::default());
    *I18N.write().unwrap() = Some(I18n::load("zh-CN"));
    theme::force_dark(false);
    create_windows();
    let panel = hwnd(&PANEL);
    show_panel();
    pump(30);
    unsafe {
        let repair = GetDlgItem(Some(panel), IDC_THEME_REPAIR as i32).unwrap();
        let _ = SendMessageW(repair, BM_CLICK, None, None);
        pump(300);
        let line = crate::status_line_text(IDC_THEME_SCHEDULE);
        eprintln!("schedule right after click: {line:?}");
        tray::remove();
        DestroyWindow(panel).unwrap();
        DestroyWindow(hwnd(&HIDDEN)).unwrap();
        assert!(
            line.contains("正在修复") || line.contains("Repairing"),
            "repair click gave no immediate feedback; schedule line: {line:?}"
        );
    }
}

/// Every interactive panel control must react visually to hover, press,
/// disable, and keyboard focus. An unresponsive state is instantly visible
/// to users, so each one is injected and the pixels are compared against
/// the resting state — including the two cancel links, which are shown for
/// the audit.
#[test]
fn interactive_controls_react_to_every_state() {
    const CHILD: &str = "IDLETRIGGER_TEST_STATES_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let _ui_test = crate::runtime::lock(&CONFIG_TEST_LOCK);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "render_tests::interactive_controls_react_to_every_state",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(180);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("states child timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(status.success(), "states child failed: {status}");
        return;
    }
    let _ui_test = crate::runtime::lock(&CONFIG_TEST_LOCK);
    *crate::runtime::lock(&CONFIG) = Some(config::Config::default());
    *I18N.write().unwrap() = Some(I18n::load("zh-CN"));
    theme::force_dark(false);
    create_windows();
    let panel = hwnd(&PANEL);
    show_panel();
    pump(30);
    unsafe {
        // The armed-only cancel links join the audit.
        for id in [IDC_NOSLEEP_TIMED_CANCEL, IDC_THEME_SNOOZE_CANCEL] {
            let _ = ShowWindow(
                GetDlgItem(Some(panel), id as i32).unwrap_or_default(),
                SW_SHOW,
            );
        }
        pump(10);
    }
    let ids = [
        IDC_NOSLEEP,
        IDC_IDLE,
        IDC_NOSLEEP_TIMED_30M,
        IDC_NOSLEEP_TIMED_1H,
        IDC_NOSLEEP_TIMED_2H,
        IDC_AUTOMATION,
        IDC_MANAGE_BUTTON,
        IDC_THEME_ENABLE,
        IDC_THEME_SWITCH,
        IDC_THEME_SNOOZE_30M,
        IDC_THEME_SNOOZE_1H,
        IDC_THEME_SNOOZE_MORNING,
        IDC_THEME_REPAIR,
        IDC_NOSLEEP_TIMED_CANCEL,
        IDC_THEME_SNOOZE_CANCEL,
        IDC_SYSTEM_BUTTON,
        IDC_SETTINGS_BUTTON,
        IDC_EXIT_BUTTON,
    ];
    unsafe {
        // Hash a control's pixels (every other pixel keeps this fast).
        let signature = |ctl: HWND| -> u64 {
            use std::hash::{Hash, Hasher};
            let mut rect = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(ctl, &mut rect);
            let dc = GetDC(Some(ctl));
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            let mut y = rect.top;
            while y < rect.bottom {
                let mut x = rect.left;
                while x < rect.right {
                    GetPixel(dc, x, y).0.hash(&mut hasher);
                    x += 2;
                }
                y += 2;
            }
            let _ = ReleaseDC(Some(ctl), dc);
            hasher.finish()
        };
        // UpdateWindow only: pumping after the hover message would deliver
        // the mouseless TrackMouseEvent's immediate WM_MOUSELEAVE and clear
        // the very state under audit.
        let present = |ctl: HWND| {
            let _ = windows::Win32::Graphics::Gdi::UpdateWindow(ctl);
        };
        let mut failures: Vec<String> = Vec::new();
        for id in ids {
            let Ok(ctl) = GetDlgItem(Some(panel), id as i32) else {
                failures.push(format!("{id}: missing"));
                continue;
            };
            let rest = signature(ctl);
            // Hover.
            let _ = SendMessageW(
                ctl,
                windows::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE,
                Some(WPARAM(0)),
                Some(LPARAM(0x0014_0014)),
            );
            present(ctl);
            let hover = signature(ctl);
            let _ = SendMessageW(ctl, windows::Win32::UI::Controls::WM_MOUSELEAVE, None, None);
            present(ctl);
            // Pressed.
            let _ = SendMessageW(
                ctl,
                windows::Win32::UI::WindowsAndMessaging::BM_SETSTATE,
                Some(WPARAM(1)),
                None,
            );
            present(ctl);
            let pressed = signature(ctl);
            let _ = SendMessageW(
                ctl,
                windows::Win32::UI::WindowsAndMessaging::BM_SETSTATE,
                Some(WPARAM(0)),
                None,
            );
            present(ctl);
            // Disabled.
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(ctl, false);
            present(ctl);
            let disabled = signature(ctl);
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(ctl, true);
            present(ctl);
            // Keyboard focus ring.
            nativeform::keyboard_navigation();
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(ctl));
            present(ctl);
            let focused = signature(ctl);
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(None);
            nativeform::keyboard_navigation();
            if rest == hover {
                failures.push(format!("{id}: hover is a visual no-op"));
            }
            if rest == pressed || hover == pressed {
                failures.push(format!("{id}: pressed does not read as its own state"));
            }
            if rest == disabled {
                failures.push(format!("{id}: disabled is a visual no-op"));
            }
            if rest == focused {
                failures.push(format!("{id}: keyboard focus is invisible"));
            }
        }
        tray::remove();
        DestroyWindow(panel).unwrap();
        DestroyWindow(hwnd(&HIDDEN)).unwrap();
        assert!(failures.is_empty(), "state audit failures: {failures:?}");
    }
}
