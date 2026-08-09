# 🧭 IdleTrigger User Guide

[Documentation](README.md) · [简体中文](user-guide.zh-CN.md) · [Project Home](../README.md)

## 🚀 Start

IdleTrigger supports Windows 10 / Windows Server 2016 and later. Use x64 on most PCs; use x86 only on 32-bit Windows.

1. Create a writable folder you intend to keep, such as `%LOCALAPPDATA%\IdleTrigger`.
2. Download a build into that folder: [x64](https://github.com/JeffioZ/IdleTrigger/releases/latest/download/IdleTrigger-x64.exe) for most PCs or [x86](https://github.com/JeffioZ/IdleTrigger/releases/latest/download/IdleTrigger-x86.exe) for 32-bit Windows.
3. Run the EXE. IdleTrigger appears in the notification area without opening a main window.
4. Left-click the tray icon for the control panel; right-click for **Open** and **Exit**.

## 🪟 Control Panel and Settings

The control panel keeps frequent actions and live status in one compact view:

| Area | What it provides |
| --- | --- |
| **Power Management** | Manual Stay Awake and Idle Monitoring switches, plus their effective runtime status |
| **Automatic Tasks** | Master switch, enabled count, next run, and entry to the task manager |
| **Day / Night** | Automatic switching, an immediate theme switch, and the next scheduled transition |
| **Bottom actions** | Built-in system actions, Settings, and Exit |

Open **Settings** for persistent feature behavior:

| Settings page | Available preferences |
| --- | --- |
| **Power** | Keep the display on, battery policy and threshold, idle timeout, reminder countdown, timeout action, and Enhanced Monitoring |
| **Day / Night** | Fixed-time or sunrise/sunset schedule, time-zone or approximate IP location, battery/fullscreen behavior, and theme repair |
| **Application** | Display language, global hotkeys, auto-start, debug logging, and the project link |

The Settings header always shows the current version. Every saved preference has a home in the control panel, Settings, or the automatic-task manager, so editing TOML is not required for normal use.

Use `Tab` / `Shift+Tab` to move and `Space` to activate the focused control.

> [!CAUTION]
> Save your work before using **System Controls**. These actions run immediately.

## ⚡ Power Management

### Stay Awake

Stay Awake prevents automatic sleep through Windows power requests. You can optionally keep the display on. It does not simulate keyboard or mouse input.

### Idle Monitoring

Idle Monitoring reads Windows' last-input time. After real keyboard and mouse inactivity, it can lock, sleep, hibernate, shut down, or restart. The default is 30 minutes and Sleep.

The optional pre-action reminder can be cancelled by input or by closing it. Use **Enhanced Monitoring** when a device or app repeatedly resets Windows idle time.

Stay Awake and Idle Monitoring cannot run together. Automatic tasks may change their current state without rewriting your manual settings.

## 🔁 Automatic Tasks

| Trigger | Available result |
| --- | --- |
| A process is running or a time window is active | Temporarily enable or pause Stay Awake or Idle Monitoring |
| Once, daily, weekly, process start, or all selected processes exit | Lock, sleep, hibernate, shut down, or restart |

Process targets can match an executable name or an exact EXE path. System actions always show a cancellable countdown of at least 10 seconds.

Tasks run only while IdleTrigger is running. Processes are checked about every five seconds. Schedules missed during sleep are not replayed after resume. Tasks cannot run custom commands.

## 🌗 Day / Night Themes

Switch Windows light and dark themes at fixed times or at sunrise and sunset. You can use dark mode on battery or postpone a scheduled change during fullscreen apps and games.

Sunrise and sunset use the Windows time zone by default, with optional approximate IP-based location. IP results are kept in memory only; if lookup is disabled or unavailable, IdleTrigger falls back to the Windows time zone, UTC offset, and finally a built-in default location. If Windows theme settings are unavailable or blocked, IdleTrigger disables this section but keeps its settings.

## ⚙️ Configuration

| File | Purpose |
| --- | --- |
| `IdleTrigger.toml` | Settings and automatic-task rules |
| `IdleTrigger.state.json` | Scheduler state used to avoid repeated task execution |
| `IdleTrigger.log` | Optional diagnostic log |

IdleTrigger creates these files beside the EXE when needed. Prefer **Settings** and the automatic-task manager for everyday changes. [IdleTrigger.example.toml](../IdleTrigger.example.toml) remains the complete reference for portable or advanced manual configuration. Valid file edits apply within a few seconds. To reload immediately, run:

```powershell
.\IdleTrigger-x64.exe config:reload
```

Auto-start is managed in **Settings > Application** or by CLI and is stored in the current user's Windows Run registry key.

## ⌨️ Command Line

Run the EXE without arguments to start the tray app.

```text
IdleTrigger sleep | hibernate | shutdown | restart | lock

IdleTrigger nosleep on [--screen]
IdleTrigger nosleep off | toggle | status

IdleTrigger monitor on | off | status

IdleTrigger autostart enable | disable | status
IdleTrigger config:reload
IdleTrigger status
IdleTrigger version
```

Changing `nosleep` or `monitor`, and running `config:reload`, require the tray app. Status queries and one-shot power actions do not.

## 🔄 Update or Move

Exit IdleTrigger before replacing or moving the EXE. Keep `IdleTrigger.toml` and `IdleTrigger.state.json` beside it. After a move, launch the app once from its new location to refresh auto-start.

Use [SHA256SUMS.txt](https://github.com/JeffioZ/IdleTrigger/releases/latest/download/SHA256SUMS.txt) to verify downloaded executables.

## 🧰 Logs

Enable **Debug Log** in **Settings > Application** or set `logging_enabled = true`. Logs are stored beside the EXE. If that folder is not writable, IdleTrigger uses `%TEMP%`.
