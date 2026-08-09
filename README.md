<div align="center">

<h1>IdleTrigger</h1>

<p><strong>Lightweight, native Windows power automation in one portable EXE.</strong></p>

<p>Keep work running, respond to real input inactivity,<br>and automate power or Windows themes by time and process.</p>

<p>
  <a href="https://github.com/JeffioZ/IdleTrigger/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/JeffioZ/IdleTrigger?display_name=tag&amp;sort=semver&amp;style=flat&amp;color=37BFF3"></a>
  <a href="https://github.com/JeffioZ/IdleTrigger/actions/workflows/ci.yml"><img alt="Lint status" src="https://img.shields.io/github/actions/workflow/status/JeffioZ/IdleTrigger/ci.yml?branch=master&amp;style=flat&amp;logo=github&amp;label=Lint"></a>
  <a href="LICENSE"><img alt="MIT license" src="https://img.shields.io/github/license/JeffioZ/IdleTrigger?style=flat&amp;color=7C3AED"></a>
  <a href="https://github.com/JeffioZ/IdleTrigger/releases"><img alt="Total downloads" src="https://img.shields.io/github/downloads/JeffioZ/IdleTrigger/total?style=flat&amp;label=downloads&amp;color=0F9D7A"></a>
</p>

<p>
  <a href="https://github.com/JeffioZ/IdleTrigger/releases/latest/download/IdleTrigger-x64.exe"><img alt="Download x64 for 64-bit Windows" src="https://img.shields.io/badge/Download-x64-0078D4?style=flat&amp;logo=windows11&amp;logoColor=white"></a>
  <a href="https://github.com/JeffioZ/IdleTrigger/releases/latest/download/IdleTrigger-x86.exe"><img alt="Download x86 for 32-bit Windows" src="https://img.shields.io/badge/Download-x86-64748B?style=flat&amp;logo=windows11&amp;logoColor=white"></a>
</p>

<p><a href="README.zh-CN.md">简体中文</a></p>

</div>

## 🪟 Native Control Panel

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/control-panel-en-dark.png">
    <img src="docs/images/control-panel-en-light.png" alt="IdleTrigger control panel" width="430">
  </picture>
</p>

<p align="center"><sub>Follows Windows light/dark mode and display DPI. The screenshot follows your GitHub theme.</sub></p>

Left-click the tray icon for quick controls, then open **Settings** for all persistent preferences.

## ✨ At a Glance

| | Capability | Built for |
| --- | --- | --- |
| ⚡ | **Stay Awake** | Downloads, renders, backups, and remote sessions that must keep running. |
| ⏱️ | **Idle Actions** | Lock, sleep, hibernate, shut down, or restart after real keyboard and mouse inactivity. |
| 🔁 | **Automatic Tasks** | Control power features or run built-in actions by schedule and process state. |
| 🌗 | **Day / Night** | Switch Windows themes by time or sunrise and sunset, with battery and fullscreen options. |

**Small by design:** IdleTrigger is a portable native Win32 app for Windows 10 / Windows Server 2016 or later. It needs no installer, service, WebView, simulated input, or extra runtime. Settings stay in a readable TOML file beside the EXE.

## 🚀 Get Started

1. Download **x64** for most PCs, or **x86** for 32-bit Windows.
2. Put the EXE in a writable folder you intend to keep, then run it.
3. Left-click the IdleTrigger tray icon and choose your settings.

## 📚 Documentation

| | Read this |
| --- | --- |
| 🧭 | [User guide](docs/user-guide.md) — features, automatic tasks, configuration, CLI, and updates |
| 📝 | [Configuration reference](IdleTrigger.example.toml) — every TOML field in English and Chinese |
| 🛠️ | [Build and development](docs/development.md) — local builds, checks, resources, and release process |
| 🗂️ | [Documentation index](docs/README.md) — all project documents in one place |

## 🤝 Credits

Tray integration is adapted from [getlantern/systray v1.2.2](https://github.com/getlantern/systray) ([Apache-2.0 notice](internal/ui/trayicon/LICENSE)).

Built with [BurntSushi/toml](https://github.com/BurntSushi/toml) and [golang.org/x/sys](https://pkg.go.dev/golang.org/x/sys). Stay Awake was inspired by [NoSleep](https://github.com/CHerSun/NoSleep). The Windows 11 theme-repair behavior is an independent implementation informed by [Auto Dark Mode's DWM refresh strategy](https://github.com/AutoDarkMode/Windows-Auto-Night-Mode/blob/master/AutoDarkModeSvc/Handlers/DwmRefreshHandler.cs).

## 📄 License

[MIT](LICENSE)
