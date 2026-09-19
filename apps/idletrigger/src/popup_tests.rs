//! Opt-in native popup checks inside the isolated rendering-test child.
use super::*;
use windows::Win32::UI::Controls::*;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

fn text(window: HWND) -> String {
    let mut buffer = vec![0u16; 8192];
    let length = unsafe { GetWindowTextW(window, &mut buffer) };
    String::from_utf16_lossy(&buffer[..length as usize])
}

pub(super) fn dialog(
    title: &str,
    body: &str,
    default: MESSAGEBOX_RESULT,
    response: MESSAGEBOX_RESULT,
    path: std::path::PathBuf,
    open: impl FnOnce(),
) {
    let title = title.to_owned();
    let body = body.to_owned();
    let worker = std::thread::spawn(move || unsafe {
        for _ in 0..200 {
            std::thread::sleep(Duration::from_millis(10));
            let mut previous = None;
            while let Ok(window) = FindWindowExW(None, previous, w!("#32770"), None) {
                previous = Some(window);
                let mut pid = 0;
                GetWindowThreadProcessId(window, Some(&mut pid));
                if pid != std::process::id()
                    || !IsWindowVisible(window).as_bool()
                    || text(window) != title
                {
                    continue;
                }
                std::thread::sleep(Duration::from_millis(50));
                // MessageBox exposes its top-level HWND before finishing the
                // button row; visibility alone does not mean initialization ended.
                for _ in 0..100 {
                    if GetDlgItem(Some(window), response.0).is_ok()
                        || (response == IDOK && GetDlgItem(Some(window), IDCANCEL.0).is_ok())
                    {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                let content_ok = child_geometry(window)
                    .iter()
                    .any(|(child, _)| text(HWND(*child as *mut _)) == body);
                let default_id = SendMessageW(window, DM_GETDEFID, None, None).0 as u32 & 0xffff;
                // An MB_OK message box can expose its sole dismiss button as
                // IDCANCEL. Resolve the real control ID for its command notification.
                let button = GetDlgItem(Some(window), response.0).or_else(|error| {
                    if response == IDOK {
                        GetDlgItem(Some(window), IDCANCEL.0)
                    } else {
                        Err(error)
                    }
                });
                let button_ok = button.is_ok();
                let ids: Vec<_> = child_geometry(window)
                    .iter()
                    .map(|(child, _)| {
                        (
                            GetDlgCtrlID(HWND(*child as *mut _)),
                            text(HWND(*child as *mut _)),
                        )
                    })
                    .collect();
                let capture = capture::capture_print_client_bmp(window, &path);
                // Always dismiss our own dialog before reporting assertion failures.
                if let Ok(button) = button {
                    PostMessageW(
                        Some(window),
                        WM_COMMAND,
                        WPARAM(GetDlgCtrlID(button) as usize),
                        LPARAM(button.0 as isize),
                    )
                    .unwrap();
                }
                if !button_ok {
                    let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                assert!(content_ok, "dialog body differs: {title}");
                assert_eq!(default_id, default.0 as u32, "default button: {title}");
                assert!(button_ok, "missing response {}: {ids:?}", response.0);
                capture.unwrap();
                return;
            }
        }
        panic!("dialog did not appear: {title}");
    });
    open();
    worker.join().unwrap();
}

pub(super) fn discard(
    window: HWND,
    edit: i32,
    value: &str,
    title: &str,
    body: &str,
    folder: &std::path::Path,
    prefix: &str,
) {
    let baseline = format!("{:?}", CONFIG.lock().unwrap());
    unsafe {
        SetWindowTextW(
            GetDlgItem(Some(window), edit).unwrap(),
            PCWSTR(wide(value).as_ptr()),
        )
        .unwrap();
    }
    for response in [IDNO, IDYES] {
        dialog(
            &t(title),
            &t(body),
            IDNO,
            response,
            folder.join(format!("{prefix}-discard-{}.bmp", response.0)),
            || unsafe {
                SendMessageW(window, WM_CLOSE, None, None);
            },
        );
        if response == IDNO {
            assert!(unsafe { IsWindowVisible(window) }.as_bool());
            assert_eq!(
                text(unsafe { GetDlgItem(Some(window), edit) }.unwrap()),
                value
            );
        } else {
            assert!(!unsafe { IsWindowVisible(window) }.as_bool());
        }
        assert_eq!(format!("{:?}", CONFIG.lock().unwrap()), baseline);
    }
}

pub(super) fn tooltips(owner: HWND, folder: &std::path::Path, prefix: &str) {
    unsafe {
        let mut previous = None;
        let mut total = 0;
        while let Ok(tip) = FindWindowExW(None, previous, TOOLTIPS_CLASSW, None) {
            previous = Some(tip);
            if GetWindow(tip, GW_OWNER).unwrap_or_default() != owner {
                continue;
            }
            let count = SendMessageW(tip, TTM_GETTOOLCOUNT, None, None).0;
            for index in 0..count {
                let mut buffer = vec![0u16; 8192];
                let mut tool = TTTOOLINFOW {
                    cbSize: size_of::<TTTOOLINFOW>() as u32,
                    ..Default::default()
                };
                assert_ne!(
                    SendMessageW(
                        tip,
                        TTM_ENUMTOOLSW,
                        Some(WPARAM(index as usize)),
                        Some(LPARAM(&mut tool as *mut _ as isize))
                    )
                    .0,
                    0
                );
                tool.lpszText = windows::core::PWSTR(buffer.as_mut_ptr());
                SendMessageW(
                    tip,
                    TTM_GETTEXTW,
                    Some(WPARAM(buffer.len())),
                    Some(LPARAM(&mut tool as *mut _ as isize)),
                );
                let length = buffer.iter().position(|v| *v == 0).unwrap();
                let content = String::from_utf16_lossy(&buffer[..length]);
                assert!(!content.trim().is_empty(), "empty tooltip {prefix}/{index}");
                assert!(!content.starts_with("tip_"), "untranslated tooltip");
                total += 1;
                // Exercise the actual tooltip renderer without moving the user's cursor.
                // Tracking mode is temporary; normal hover timing is not covered here.
                let flags = tool.uFlags;
                tool.uFlags |= TTF_TRACK | TTF_ABSOLUTE;
                SendMessageW(
                    tip,
                    TTM_SETTOOLINFOW,
                    None,
                    Some(LPARAM(&tool as *const _ as isize)),
                );
                SendMessageW(
                    tip,
                    TTM_TRACKPOSITION,
                    None,
                    Some(LPARAM(160 | (160 << 16))),
                );
                SendMessageW(
                    tip,
                    TTM_TRACKACTIVATE,
                    Some(WPARAM(1)),
                    Some(LPARAM(&tool as *const _ as isize)),
                );
                pump(5);
                let visible = IsWindowVisible(tip).as_bool();
                if visible {
                    capture::capture_print_client_bmp(
                        tip,
                        &folder.join(format!("{prefix}-tip-{index}.bmp")),
                    )
                    .unwrap();
                }
                SendMessageW(
                    tip,
                    TTM_TRACKACTIVATE,
                    Some(WPARAM(0)),
                    Some(LPARAM(&tool as *const _ as isize)),
                );
                tool.uFlags = flags;
                SendMessageW(
                    tip,
                    TTM_SETTOOLINFOW,
                    None,
                    Some(LPARAM(&tool as *const _ as isize)),
                );
                assert!(visible, "tooltip failed to show: {prefix}/{index}");
            }
        }
        assert!(total > 0, "no registered tooltips: {prefix}");
        eprintln!("verified {total} tooltip texts and native renderings: {prefix}");
    }
}
