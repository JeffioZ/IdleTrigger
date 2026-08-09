# 🧭 IdleTrigger User Guide

[Documentation](README.md) · [简体中文](user-guide.zh-CN.md) · [Project Home](../README.md)

## 🚀 Start

IdleTrigger supports Windows 10 / Windows Server 2016 and later. Use x64 on most PCs; use x86 only on 32-bit Windows.

1. Create a writable folder you intend to keep, such as `%LOCALAPPDATA%\IdleTrigger`.
2. Download a build into that folder: [x64](https://github.com/JeffioZ/IdleTrigger/releases/latest/download/IdleTrigger-x64.exe) for most PCs or [x86](https://github.com/JeffioZ/IdleTrigger/releases/latest/download/IdleTrigger-x86.exe) for 32-bit Windows.
3. Run the EXE. IdleTrigger appears in the notification area without opening a main window.
4. Left-click the tray icon for the control panel; right-click for **Open** and **Exit**.

## 🪟 Control Panel

| Section | What it controls |
| --- | --- |
| **Power Management** | Stay Awake, Idle Monitoring, idle timeout, and idle action |
| **Automatic Tasks** | Master switch, enabled count, next run, and task manager |
| **Day / Night** | Scheduled theme switching and battery/fullscreen behavior |
| **General Settings** | Global hotkeys, auto-start, debug logging, language, and configuration |
| **System Controls** | Lock, sleep, hibernate, shut down, or restart immediately |

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

Sunrise and sunset use the Windows time zone by default, with optional approximate IP-based location. If Windows theme settings are unavailable or blocked, IdleTrigger disables this section but keeps its settings.

## ⚙️ Configuration

| File | Purpose |
| --- | --- |
| `IdleTrigger.toml` | Settings and automatic-task rules |
| `IdleTrigger.state.json` | Scheduler state used to avoid repeated task execution |
| `IdleTrigger.log` | Optional diagnostic log |

IdleTrigger creates these files beside the EXE when needed. See [IdleTrigger.example.toml](../IdleTrigger.example.toml) for every field. Valid edits apply within a few seconds. To reload immediately, run:

```powershell
.\IdleTrigger-x64.exe config:reload
```

Auto-start is managed by the panel or CLI and is stored in the current user's Windows Run registry key.

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

Enable **Debug Log** in the panel or set `logging_enabled = true`. Logs are stored beside the EXE. If that folder is not writable, IdleTrigger uses `%TEMP%`.
