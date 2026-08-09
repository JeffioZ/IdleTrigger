<div align="center">

<h1>IdleTrigger</h1>

<p><strong>轻量、原生的 Windows 电源自动化托盘工具，一个 EXE 即可使用。</strong></p>

<p>让任务持续运行，在真实输入空闲后执行电源动作，<br>并按时间或进程自动管理电源状态与 Windows 主题。</p>

<p>
  <a href="https://github.com/JeffioZ/IdleTrigger/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/JeffioZ/IdleTrigger?display_name=tag&amp;sort=semver&amp;style=flat&amp;color=37BFF3"></a>
  <a href="https://github.com/JeffioZ/IdleTrigger/actions/workflows/ci.yml"><img alt="Lint 状态" src="https://img.shields.io/github/actions/workflow/status/JeffioZ/IdleTrigger/ci.yml?branch=master&amp;style=flat&amp;logo=github&amp;label=Lint"></a>
  <a href="LICENSE"><img alt="MIT 许可证" src="https://img.shields.io/github/license/JeffioZ/IdleTrigger?style=flat&amp;color=7C3AED"></a>
  <a href="https://github.com/JeffioZ/IdleTrigger/releases"><img alt="累计下载" src="https://img.shields.io/github/downloads/JeffioZ/IdleTrigger/total?style=flat&amp;label=downloads&amp;color=0F9D7A"></a>
</p>

<p>
  <a href="https://github.com/JeffioZ/IdleTrigger/releases/latest/download/IdleTrigger-x64.exe"><img alt="下载适用于 64 位 Windows 的 x64 版本" src="https://img.shields.io/badge/%E4%B8%8B%E8%BD%BD-x64-0078D4?style=flat&amp;logo=windows11&amp;logoColor=white"></a>
  <a href="https://github.com/JeffioZ/IdleTrigger/releases/latest/download/IdleTrigger-x86.exe"><img alt="下载适用于 32 位 Windows 的 x86 版本" src="https://img.shields.io/badge/%E4%B8%8B%E8%BD%BD-x86-64748B?style=flat&amp;logo=windows11&amp;logoColor=white"></a>
</p>

<p><a href="README.md">English</a></p>

</div>

## 🪟 原生控制浮层

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/control-panel-zh-CN-dark.png">
    <img src="docs/images/control-panel-zh-CN-light.png" alt="IdleTrigger 简体中文控制浮层" width="430">
  </picture>
</p>

<p align="center"><sub>跟随 Windows 深浅色与显示器 DPI；截图会随 GitHub 主题切换。</sub></p>

左键托盘图标使用快捷控制，需要调整持久化偏好时打开“设置”。

## ✨ 核心能力

| | 能力 | 适用场景 |
| --- | --- | --- |
| ⚡ | **保持唤醒** | 下载、渲染、备份或远程连接期间阻止自动睡眠。 |
| ⏱️ | **空闲动作** | 真实键盘、鼠标持续无操作后，自动锁定、睡眠、休眠或关机。 |
| 🔁 | **自动任务** | 按计划或进程状态控制电源功能，也可执行内置系统动作。 |
| 🌗 | **昼夜主题** | 按时间或日出日落切换 Windows 主题，并可适配电池和全屏场景。 |

**为轻量而设计：** IdleTrigger 是面向 Windows 10 / Windows Server 2016 及以上的原生 Win32 便携程序。无需安装器、服务、WebView、模拟输入或额外运行时。设置保存在 EXE 旁边的可读 TOML 文件中。

## 🚀 三步开始

1. 大多数电脑下载 **x64**；32 位 Windows 下载 **x86**。
2. 将 EXE 放入准备长期保留的可写目录，然后运行。
3. 左键 IdleTrigger 托盘图标，完成常用设置。

## 📚 文档

| | 说明 |
| --- | --- |
| 🧭 | [使用指南](docs/user-guide.zh-CN.md)——功能、自动任务、配置、命令行和升级方式 |
| 📝 | [配置参考](IdleTrigger.example.toml)——所有 TOML 字段的中英文说明 |
| 🛠️ | [构建与开发](docs/development.zh-CN.md)——本地构建、检查、资源和发布流程 |
| 🗂️ | [文档索引](docs/README.md)——集中查看项目全部文档 |

## 🤝 致谢

托盘集成基于 [getlantern/systray v1.2.2](https://github.com/getlantern/systray) 调整（[Apache-2.0 声明](internal/ui/trayicon/LICENSE)）。

项目使用 [BurntSushi/toml](https://github.com/BurntSushi/toml) 和 [golang.org/x/sys](https://pkg.go.dev/golang.org/x/sys)；保持唤醒功能受到 [NoSleep](https://github.com/CHerSun/NoSleep) 启发。Windows 11 主题修复为独立实现，其行为参考了 [Auto Dark Mode 的 DWM 刷新策略](https://github.com/AutoDarkMode/Windows-Auto-Night-Mode/blob/master/AutoDarkModeSvc/Handlers/DwmRefreshHandler.cs)。

## 📄 许可证

[MIT](LICENSE)
