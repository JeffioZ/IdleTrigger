//! Named-pipe IPC server and the CLI client, mirroring the Go surface:
//! tray commands (`nosleep`, `monitor`, `status`, `config:reload`) are
//! forwarded to the running instance; direct actions (`sleep`, `lock`,
//! `autostart`, `version`) run in-process.

use std::sync::atomic::Ordering;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
    PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::core::PCWSTR;

pub const PIPE_NAME: &str = r"\\.\pipe\IdleTriggerPipe";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Spawns the pipe server thread in the primary instance.
pub fn spawn_server() {
    std::thread::Builder::new()
        .name("ipc-server".into())
        .spawn(|| {
            loop {
                if crate::EXITING.load(Ordering::SeqCst) {
                    return;
                }
                serve_one();
            }
        })
        .expect("spawn ipc server");
}

fn serve_one() {
    let name = wide(PIPE_NAME);
    unsafe {
        let pipe = CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0),
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            1024,
            1024,
            0,
            None,
        );
        if pipe.is_invalid() {
            std::thread::sleep(std::time::Duration::from_millis(500));
            return;
        }
        if ConnectNamedPipe(pipe, None).is_err() {
            let _ = windows::Win32::Foundation::CloseHandle(pipe);
            return;
        }
        let mut buffer = [0u8; 512];
        let mut read = 0u32;
        if ReadFile(pipe, Some(&mut buffer), Some(&mut read), None).is_ok() && read > 0 {
            let request = String::from_utf8_lossy(&buffer[..read as usize]).to_string();
            let response = handle_request(&request);
            let _ = WriteFile(pipe, Some(response.as_bytes()), None, None);
        }
        let _ = DisconnectNamedPipe(pipe);
        let _ = windows::Win32::Foundation::CloseHandle(pipe);
    }
}

fn handle_request(request: &str) -> String {
    let request = request.trim();
    match request {
        "open" => {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    Some(crate::hwnd(&crate::HIDDEN)),
                    crate::WM_REFRESH_UI,
                    windows::Win32::Foundation::WPARAM(0),
                    windows::Win32::Foundation::LPARAM(0),
                );
            }
            show_panel_async();
            "ok open".into()
        }
        "nosleep:on" | "nosleep:off" | "nosleep:toggle" => {
            let target = match request {
                "nosleep:on" => true,
                "nosleep:off" => false,
                _ => !crate::cfg_map(|c| c.nosleep_enabled),
            };
            crate::cfg_edit(|c| {
                c.nosleep_enabled = target;
                if target {
                    c.idle_enabled = false;
                }
            });
            crate::persist_config();
            refresh_ui();
            format!("ok nosleep={target}")
        }
        "monitor:on" | "monitor:off" | "monitor:toggle" => {
            let target = match request {
                "monitor:on" => true,
                "monitor:off" => false,
                _ => !crate::cfg_map(|c| c.idle_enabled),
            };
            crate::cfg_edit(|c| {
                c.idle_enabled = target;
                if target {
                    c.nosleep_enabled = false;
                }
            });
            crate::persist_config();
            refresh_ui();
            format!("ok monitor={target}")
        }
        "status" => {
            let (nosleep, idle, automation) =
                crate::cfg_map(|c| (c.nosleep_enabled, c.idle_enabled, c.automation_enabled));
            let seconds = crate::IDLE_MS.load(Ordering::SeqCst) / 1000;
            format!(
                "tray=running nosleep={nosleep} monitor={idle} automation={automation} idle_seconds={seconds}"
            )
        }
        "reload" => {
            crate::automation::reload_rules();
            refresh_ui();
            "ok reload".into()
        }
        _ => format!("err unknown request: {request}"),
    }
}

fn show_panel_async() {
    unsafe {
        // Show the panel via SW_SHOW; PostMessage is safe cross-thread.
        let panel = crate::hwnd(&crate::PANEL);
        let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
            panel,
            windows::Win32::UI::WindowsAndMessaging::SW_SHOW,
        );
    }
}

fn refresh_ui() {
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            Some(crate::hwnd(&crate::HIDDEN)),
            crate::WM_REFRESH_UI,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(0),
        );
    }
}

/// Sends one request to the running instance and prints the reply. Returns
/// false when the pipe is unavailable (tray not running).
pub fn send(request: &str) -> Option<String> {
    let attempts = 20;
    for _ in 0..attempts {
        if let Some(reply) = try_send(request) {
            return Some(reply);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

fn try_send(request: &str) -> Option<String> {
    let name = wide(PIPE_NAME);
    unsafe {
        let Ok(file) = windows::Win32::Storage::FileSystem::CreateFileW(
            PCWSTR(name.as_ptr()),
            windows::Win32::Storage::FileSystem::FILE_GENERIC_READ.0
                | windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE.0,
            windows::Win32::Storage::FileSystem::FILE_SHARE_READ,
            None,
            windows::Win32::Storage::FileSystem::OPEN_EXISTING,
            Default::default(),
            None,
        ) else {
            return None;
        };
        let mut written = 0u32;
        let ok = WriteFile(file, Some(request.as_bytes()), Some(&mut written), None).is_ok()
            && written as usize == request.len();
        // Message pipes need the client to read the reply on the same handle;
        // for simplicity the tray writes its reply before we read.
        let mut buffer = [0u8; 512];
        let mut read = 0u32;
        let reply = if ok {
            windows::Win32::Storage::FileSystem::ReadFile(
                file,
                Some(&mut buffer),
                Some(&mut read),
                None,
            )
            .ok()
            .map(|_| String::from_utf8_lossy(&buffer[..read as usize]).to_string())
        } else {
            None
        };
        let _ = windows::Win32::Foundation::CloseHandle(file);
        reply
    }
}

// ---- CLI entry ------------------------------------------------------------

/// Runs the CLI in the current process. Returns the process exit code.
pub fn run_cli(args: &[String]) -> i32 {
    let Some(command) = args.first() else {
        console_println(&crate::t_pub("cli_usage"));
        return 0;
    };
    let rest = &args[1..];
    match command.as_str() {
        "sleep" => direct_action("sleep"),
        "hibernate" => direct_action("hibernate"),
        "shutdown" => direct_action("shutdown"),
        "restart" => direct_action("restart"),
        "lock" => direct_action("lock"),
        "nosleep" => {
            let mode = rest.first().map(String::as_str).unwrap_or("status");
            match mode {
                "on" | "off" | "toggle" => {
                    // Go --screen/-s: also keep the display on.
                    let screen = rest.iter().any(|a| a == "--screen" || a == "-s");
                    pipe_or_error(&format!(
                        "nosleep:{mode}{}",
                        if screen { ":screen" } else { "" }
                    ))
                }
                "status" => pipe_or_error("status"),
                _ => {
                    console_println(&crate::t_pub("cli_usage_nosleep"));
                    1
                }
            }
        }
        "monitor" => {
            let mode = rest.first().map(String::as_str).unwrap_or("status");
            match mode {
                "on" | "off" | "toggle" => pipe_or_error(&format!("monitor:{mode}")),
                "status" => pipe_or_error("status"),
                _ => {
                    console_println(&crate::t_pub("cli_usage_monitor"));
                    1
                }
            }
        }
        "autostart" => {
            let mode = rest.first().map(String::as_str).unwrap_or("status");
            match mode {
                "enable" => {
                    let exe = std::env::current_exe()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if crate::system::autostart_enable(&exe) {
                        console_println(&crate::t_pub("msg_autostart_enabled"));
                        0
                    } else {
                        console_println("autostart enable failed");
                        1
                    }
                }
                "disable" => {
                    if crate::system::autostart_disable() {
                        console_println(&crate::t_pub("msg_autostart_disabled"));
                        0
                    } else {
                        console_println("autostart disable failed");
                        1
                    }
                }
                "status" => {
                    console_println(if crate::system::autostart_is_enabled() {
                        "enabled"
                    } else {
                        "disabled"
                    });
                    0
                }
                _ => {
                    console_println(&crate::t_pub("cli_usage_autostart"));
                    1
                }
            }
        }
        "config:reload" => pipe_or_error("reload"),
        "status" => pipe_or_error("status"),
        "version" | "--version" | "-V" => {
            console_println(crate::APP_VERSION);
            0
        }
        "--help" | "-h" | "help" => {
            console_println(&crate::t_pub("cli_usage"));
            0
        }
        _ => {
            console_println(&crate::t_pub("cli_unknown"));
            console_println(&crate::t_pub("cli_usage"));
            1
        }
    }
}

/// Direct actions print a progress line first (Go msg_* keys) and verify
/// sleep/hibernate capability before attempting (Go powerstate check).
fn direct_action(action: &str) -> i32 {
    // Capability pre-check for suspend paths (Go cli_error_*_unavailable).
    if matches!(action, "sleep" | "hibernate") {
        let available = suspend_available(action == "hibernate");
        if !available {
            let key = if action == "hibernate" {
                "cli_error_hibernate_unavailable"
            } else {
                "cli_error_sleep_unavailable"
            };
            console_error(&crate::t_pub(key));
            return 1;
        }
    }
    let progress_key = match action {
        "sleep" => Some("msg_sleeping"),
        "hibernate" => Some("msg_hibernating"),
        "shutdown" => Some("msg_shutting_down"),
        "restart" => Some("msg_restarting"),
        "lock" => Some("msg_locking"),
        _ => None,
    };
    if let Some(key) = progress_key {
        console_println(&crate::t_pub(key));
    }
    crate::execute_system_action(action);
    0
}

/// Checks the power capabilities for sleep/hibernate (Go powerstate).
fn suspend_available(hibernate: bool) -> bool {
    use windows::Win32::System::Power::{GetPwrCapabilities, GetSystemPowerStatus};
    unsafe {
        if hibernate {
            let mut caps = windows::Win32::System::Power::SYSTEM_POWER_CAPABILITIES::default();
            if GetPwrCapabilities(&mut caps) {
                return caps.HiberFilePresent;
            }
            return false;
        }
        let mut status = windows::Win32::System::Power::SYSTEM_POWER_STATUS::default();
        // Sleep is available unless the system reports no sleep states.
        GetSystemPowerStatus(&mut status).is_ok()
    }
}

fn pipe_or_error(request: &str) -> i32 {
    match send(request) {
        Some(reply) => {
            // Go protocol: "err:"-prefixed replies go to stderr with exit 1.
            if let Some(detail) = reply.strip_prefix("err:") {
                console_error(&format!("{} {detail}", crate::t_pub("cli_error_detail")));
                1
            } else {
                console_println(&reply);
                0
            }
        }
        None => {
            console_error(&crate::t_pub("cli_error_tray_not_running"));
            1
        }
    }
}

/// Attaches to the parent console (windowsgui subsystem) and prints there.
pub fn attach_console() {
    unsafe {
        let kernel32 = windows::Win32::System::LibraryLoader::GetModuleHandleW(windows::core::w!(
            "kernel32.dll"
        ))
        .unwrap_or_default();
        let attach = windows::Win32::System::LibraryLoader::GetProcAddress(
            kernel32,
            windows::core::s!("AttachConsole"),
        );
        let Some(attach) = attach else {
            return;
        };
        let attach: unsafe extern "system" fn(u32) -> i32 = std::mem::transmute(attach);
        const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
        attach(ATTACH_PARENT_PROCESS);
    }
}

fn console_println(text: &str) {
    write_console(get_stdout(), text);
}

/// Errors go to stderr (Go cli_error_detail path).
fn console_error(text: &str) {
    write_console(get_stderr(), text);
}

fn get_stdout() -> windows::Win32::Foundation::HANDLE {
    unsafe {
        windows::Win32::System::Console::GetStdHandle(
            windows::Win32::System::Console::STD_OUTPUT_HANDLE,
        )
        .unwrap_or_default()
    }
}

fn get_stderr() -> windows::Win32::Foundation::HANDLE {
    unsafe {
        windows::Win32::System::Console::GetStdHandle(
            windows::Win32::System::Console::STD_ERROR_HANDLE,
        )
        .unwrap_or_default()
    }
}

fn write_console(handle: windows::Win32::Foundation::HANDLE, text: &str) {
    if handle.is_invalid() {
        return;
    }
    unsafe {
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        wide.push(b'\n' as u16);
        // WriteConsoleW only works on real consoles; redirected pipes and
        // files need raw UTF-8 WriteFile bytes instead.
        if windows::Win32::System::Console::WriteConsoleW(handle, &wide, None, None).is_err() {
            let mut bytes = text.as_bytes().to_vec();
            bytes.push(b'\n');
            let mut written = 0u32;
            let _ = windows::Win32::Storage::FileSystem::WriteFile(
                handle,
                Some(&bytes),
                Some(&mut written),
                None,
            );
        }
    }
}

#[allow(dead_code)]
fn _pin(_h: HANDLE) {}
