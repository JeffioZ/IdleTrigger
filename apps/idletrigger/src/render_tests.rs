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
        let _ui_test = CONFIG_TEST_LOCK.lock().unwrap();
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
    *CONFIG.lock().unwrap() = Some(config::Config {
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
        // Final rendered background of the actual checkbox and static text
        // must use the new palette; sample a margin outside their glyphs.
        for id in [IDC_NOSLEEP, STATIC_SECTION_BASE, STATIC_SUBTITLE_BASE] {
            let control = unsafe { GetDlgItem(Some(panel), id as i32) }.unwrap();
            unsafe {
                let dc = GetDC(Some(control));
                let pixel = GetPixel(dc, 0, 0);
                let _ = ReleaseDC(Some(control), dc);
                assert_eq!(
                    pixel.0,
                    theme::bg_color(),
                    "control {id} has a stale background"
                );
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
            {
                use tray_icon::menu::ContextMenu;
                theme::prepare_popup_menu(panel, dark);
                let menu = tray_menu();
                capture_native_popup(
                    panel,
                    folder.join(format!("{language}-tray-{dark}.bmp")),
                    true,
                    || unsafe {
                        menu.show_context_menu_for_hwnd(panel.0 as isize, None);
                    },
                );
            }
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
            let baseline = automation::RULES.lock().unwrap().clone();
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
            assert_eq!(*automation::RULES.lock().unwrap(), baseline);
            automation::RULES.lock().unwrap().clear();
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
            *automation::PENDING_ACTION.lock().unwrap() = Some(automation::PendingAction {
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
