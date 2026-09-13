<div align="center">

<h1>IdleTrigger</h1>

<p><strong>Lightweight, native Windows power automation in one portable EXE.</strong></p>

<blockquote>
  <p><strong>Rust rewrite:</strong> this repository has been rewritten from Go to Rust
  (native Win32 via windows-rs, no UI framework). Feature parity with the Go version is
  complete except for items noted below; this README now describes the Rust build.</p>
</blockquote>

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

<p align="center"><sub>Adapts to Windows light/dark mode and display DPI.<br>
New screenshots arrive with the Rust rewrite; the Go-era panel is preserved in the git history.</sub></p>

Left-click the tray icon for quick controls, then open **Settings** for all persistent preferences.

## ✨ At a Glance

| | Capability | Built for |
| --- | --- | --- |
| ⚡ | **Stay Awake** | Downloads, renders, backups, and remote sessions that must keep running. |
| ⏱️ | **Idle Actions** | Lock, sleep, hibernate, shut down, or restart after real keyboard and mouse inactivity. |
| 🔁 | **Automatic Tasks** | Control power features or run built-in actions by schedule and process state. |
| 🌗 | **Day / Night** | Switch Windows themes by time or sunrise and sunset, with battery and fullscreen options. |

**Small by design:** IdleTrigger is a portable native Win32 app for Windows 10 / Windows Server 2016 or later. It needs no installer, service, WebView, simulated input, or extra runtime. Settings stay in a readable TOML file beside the EXE.

> **System requirements (Rust build):** Windows 10 1607 / Server 2016 or later (x64 or x86).
> 1607 is the practical floor because per-monitor DPI APIs require it; the theme title-bar
> tint uses the modern DWM attribute on 20H1+ and falls back to the legacy one on 1809–1909.

## 🚀 Get Started

1. Download **x64** for most PCs, or **x86** for 32-bit Windows.
2. Put the EXE in a writable folder you intend to keep, then run it.
3. Left-click the IdleTrigger tray icon and choose your settings.

## 📚 Documentation

| | Read this |
| --- | --- |
| 📝 | [Configuration reference](IdleTrigger.example.toml) — every TOML field in English and Chinese |
| 🗂️ | [Documentation index](docs/README.md) — docs status during the Rust rewrite |

User and development guides from the Go implementation are being rewritten;
they remain available in the git history of the last Go release.

## 🤝 Credits

The Rust rewrite uses the [tray-icon](https://crates.io/crates/tray-icon), [windows-rs](https://crates.io/crates/windows), [toml_edit](https://crates.io/crates/toml_edit), and [serde](https://crates.io/crates/serde) crates. The Go implementation's tray code was adapted from [getlantern/systray v1.2.2](https://github.com/getlantern/systray) (Apache-2.0 notice preserved in the git history). Stay Awake was inspired by [NoSleep](https://github.com/CHerSun/NoSleep). The Windows 11 theme-repair behavior is an independent implementation informed by [Auto Dark Mode's DWM refresh strategy](https://github.com/AutoDarkMode/Windows-Auto-Night-Mode/blob/master/AutoDarkModeSvc/Handlers/DwmRefreshHandler.cs).

## 📄 License

[MIT](LICENSE)
