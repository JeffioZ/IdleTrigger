//! Named-pipe IPC server and the CLI client, mirroring the Go surface:
//! tray commands (`nosleep`, `monitor`, `status`, `config:reload`) are
//! forwarded to the running instance; direct actions (`sleep`, `lock`,
//! `autostart`, `version`) run in-process.

use crate::pipe::Pipe;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

fn pipe_name() -> Option<String> {
    let mut session = 0;
    unsafe {
        windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(
            std::process::id(),
            &mut session,
        )
    }
    .ok()?;
    Some(format!(r"\\.\pipe\IdleTrigger-{session}"))
}

struct Request {
    text: String,
    deadline: Instant,
    reply: mpsc::SyncSender<String>,
}
static REQUESTS: Mutex<Vec<Request>> = Mutex::new(Vec::new());

/// Window/configuration work belongs to the UI thread. A timed-out queued
/// request is discarded before it can mutate anything.
fn dispatch(request: String) -> String {
    let (sender, receiver) = mpsc::sync_channel(1);
    REQUESTS.lock().unwrap().push(Request {
        text: request,
        deadline: Instant::now() + Duration::from_millis(1500),
        reply: sender,
    });
    let posted = unsafe {
        windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            Some(crate::hwnd(&crate::HIDDEN)),
            crate::WM_IPC_REQUEST,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(0),
        )
    };
    if posted.is_err() {
        REQUESTS.lock().unwrap().clear();
        return "err: UI is unavailable".into();
    }
    receiver
        .recv_timeout(Duration::from_millis(1800))
        .unwrap_or_else(|_| "err: command timed out; outcome unknown".into())
}

pub fn process_requests() {
    let requests = std::mem::take(&mut *REQUESTS.lock().unwrap());
    for request in requests {
        let reply = if Instant::now() >= request.deadline {
            "err: request expired".into()
        } else {
            handle_request(&request.text)
        };
        let _ = request.reply.send(reply);
    }
}

pub fn spawn_server() {
    std::thread::Builder::new()
        .name("ipc-server".into())
        .spawn(|| {
            let Some(name) = pipe_name() else {
                crate::log_line("IPC session lookup failed");
                return;
            };
            let pipe = match Pipe::listen(&name) {
                Ok(pipe) => pipe,
                Err(error) => {
                    crate::log_line(&format!("IPC listener failed: {error}"));
                    return;
                }
            };
            while !crate::EXITING.load(Ordering::SeqCst) {
                if pipe.connect().is_err() {
                    pipe.disconnect();
                    continue;
                }
                if let Ok(request) = pipe.read() {
                    let response = dispatch(request);
                    if pipe.write(&response).is_ok() {
                        pipe.finish();
                    }
                }
                pipe.disconnect();
            }
        })
        .expect("spawn ipc server");
}
fn handle_request(request: &str) -> String {
    let request = request.trim();
    match request {
        "open" => {
            crate::show_panel();
            "ok open".into()
        }
        "nosleep:on"
        | "nosleep:off"
        | "nosleep:toggle"
        | "nosleep:on:screen"
        | "nosleep:toggle:screen" => {
            let mut target = false;
            if let Err(err) = crate::edit_config(|c| {
                target = match request {
                    "nosleep:on" | "nosleep:on:screen" => true,
                    "nosleep:off" => false,
                    _ => !c.nosleep_enabled,
                };
                c.nosleep_enabled = target;
                if request == "nosleep:on" {
                    c.keep_screen_on = false;
                }
                if request.ends_with(":screen") {
                    c.keep_screen_on = true;
                }
                if target {
                    c.idle_enabled = false;
                }
            }) {
                return format!("err: {err}");
            }
            refresh_ui();
            format!("ok nosleep={target}")
        }
        "monitor:on" | "monitor:off" | "monitor:toggle" => {
            let mut target = false;
            if let Err(err) = crate::edit_config(|c| {
                target = match request {
                    "monitor:on" => true,
                    "monitor:off" => false,
                    _ => !c.idle_enabled,
                };
                c.idle_enabled = target;
                if target {
                    c.nosleep_enabled = false;
                }
            }) {
                return format!("err: {err}");
            }
            refresh_ui();
            format!("ok monitor={target}")
        }
        "status" | "nosleep:status" | "monitor:status" => {
            let (nosleep, idle, automation) =
                crate::cfg_map(|c| (c.nosleep_enabled, c.idle_enabled, c.automation_enabled));
            let seconds = crate::IDLE_MS.load(Ordering::SeqCst) / 1000;
            let awake_running = crate::NOSLEEP_EXECUTION_ON.load(Ordering::SeqCst);
            let monitor_running = crate::idle_settings().enabled;
            format!(
                "tray=running nosleep={nosleep} monitor={idle} automation={automation} idle_seconds={seconds} nosleep_running={awake_running} monitor_running={monitor_running}"
            )
        }
        "reload" | "config:reload" => match crate::hot_reload_config() {
            Ok(()) => "ok reload".into(),
            Err(error) => format!("err: {error}"),
        },
        "ping" => "pong".into(),
        _ => format!("err: unknown request: {request}"),
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
    let name = pipe_name()?;
    for _ in 0..20 {
        if let Ok(pipe) = Pipe::open(&name) {
            // Once connected, any transmission failure is ambiguous. Never
            // replay a command whose side effects might already have run.
            return Some(match pipe.write(request).and_then(|_| pipe.read()) {
                Ok(reply) if !reply.is_empty() => reply,
                _ => "err: IPC response unavailable; command outcome unknown".into(),
            });
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}
// ---- CLI entry ------------------------------------------------------------

/// Runs the CLI in the current process. Returns the process exit code.
pub fn run_cli(args: &[String]) -> i32 {
    let Some(command) = args.first() else {
        console_println(&crate::t_pub("cli_usage"));
        return 0;
    };
    let rest = &args[1..];
    let arguments_valid = match command.as_str() {
        "nosleep" => {
            rest.len() <= 1
                || (rest.len() == 2
                    && matches!(rest[0].as_str(), "on" | "toggle")
                    && matches!(rest[1].as_str(), "--screen" | "-s"))
        }
        "monitor" | "autostart" => rest.len() <= 1,
        _ => rest.is_empty(),
    };
    if !arguments_valid {
        console_error(&crate::t_pub("cli_usage"));
        return 1;
    }
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
        "config:reload" => pipe_or_error("config:reload"),
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
    match crate::try_system_action(action) {
        Ok(()) => 0,
        Err(error) => {
            console_error(&error);
            1
        }
    }
}

/// Checks the power capabilities for sleep/hibernate (Go powerstate).
pub fn suspend_available(hibernate: bool) -> bool {
    use windows::Win32::System::Power::GetPwrCapabilities;
    unsafe {
        let mut caps = windows::Win32::System::Power::SYSTEM_POWER_CAPABILITIES::default();
        GetPwrCapabilities(&mut caps)
            && if hibernate {
                caps.SystemS4 && caps.HiberFilePresent
            } else {
                caps.SystemS1 || caps.SystemS2 || caps.SystemS3 || caps.AoAc
            }
    }
}

fn pipe_or_error(request: &str) -> i32 {
    match send(request) {
        Some(reply) => {
            // Go protocol: "err:"-prefixed replies go to stderr with exit 1.
            if let Some(detail) = reply.strip_prefix("err:") {
                console_error(&crate::t_args("cli_error_detail", &[detail.trim()]));
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
