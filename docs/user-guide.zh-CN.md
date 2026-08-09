# 🧭 IdleTrigger 使用指南

[文档索引](README.md) · [English](user-guide.md) · [项目主页](../README.zh-CN.md)

## 🚀 开始使用

IdleTrigger 支持 Windows 10 / Windows Server 2016 及以上系统。大多数电脑使用 x64；只有 32 位 Windows 才使用 x86。

1. 新建一个准备长期保留且可写的目录，例如 `%LOCALAPPDATA%\IdleTrigger`。
2. 将对应版本下载到该目录：[x64](https://github.com/JeffioZ/IdleTrigger/releases/latest/download/IdleTrigger-x64.exe) 适用于大多数电脑，[x86](https://github.com/JeffioZ/IdleTrigger/releases/latest/download/IdleTrigger-x86.exe) 仅用于 32 位 Windows。
3. 运行 EXE。程序不会打开主窗口，请在通知区域找到 IdleTrigger。
4. 左键托盘图标打开控制浮层；右键使用“打开”和“退出”菜单。

## 🪟 控制浮层

| 区块 | 功能 |
| --- | --- |
| **电源管理** | 保持唤醒、空闲监测、空闲时长和空闲动作 |
| **自动任务** | 总开关、启用数量、下次执行时间和任务管理器 |
| **昼夜模式** | 计划主题切换，以及电池和全屏行为 |
| **通用设置** | 全局热键、开机自启、调试日志、语言和配置 |
| **系统控制** | 立即锁定、睡眠、休眠、关机或重启 |

按 `Tab` / `Shift+Tab` 移动焦点，按 `Space` 激活当前控件。

> [!CAUTION]
> 使用“系统控制”前请保存工作，这些动作会立即执行。

## ⚡ 电源管理

### 保持唤醒

保持唤醒通过 Windows 电源请求阻止自动睡眠，也可选择保持屏幕开启；它不会模拟键盘或鼠标输入。

### 空闲监测

空闲监测读取 Windows 最后输入时间。键盘、鼠标持续无操作后，可自动锁定、睡眠、休眠、关机或重启。默认时长为 30 分钟，动作为睡眠。

执行前提醒可通过键鼠输入或关闭提示取消。如果设备或应用反复刷新 Windows 空闲时间，可尝试“增强监测”。

保持唤醒和空闲监测不能同时运行。自动任务可以临时改变当前状态，但不会改写手动设置。

## 🔁 自动任务

| 触发条件 | 可执行结果 |
| --- | --- |
| 指定进程正在运行，或处于某个时间段 | 临时启用或暂停保持唤醒、空闲监测 |
| 单次、每天、每周、进程启动或全部所选进程退出 | 锁定、睡眠、休眠、关机或重启 |

进程目标可以匹配可执行文件名或具体 EXE 路径。系统动作始终显示至少 10 秒、可取消的倒计时。

任务只在 IdleTrigger 运行时生效。进程状态约每 5 秒检查一次。睡眠期间错过的计划不会补执行，任务也不能运行自定义命令。

## 🌗 昼夜主题

可按固定时间或日出日落切换 Windows 深浅色主题。还可在电池供电时使用深色，或在全屏应用和游戏期间暂缓切换。

日出日落默认根据 Windows 时区估算，也可选择 IP 粗略定位。如果 Windows 主题设置不可用或被策略阻止，IdleTrigger 会停用该区块，但保留配置。

## ⚙️ 配置

| 文件 | 用途 |
| --- | --- |
| `IdleTrigger.toml` | 设置与自动任务规则 |
| `IdleTrigger.state.json` | 防止计划任务重复执行的调度状态 |
| `IdleTrigger.log` | 可选诊断日志 |

IdleTrigger 会按需在 EXE 旁边创建这些文件。所有字段见 [IdleTrigger.example.toml](../IdleTrigger.example.toml)。有效修改会在数秒内应用。如需立即重载，运行：

```powershell
.\IdleTrigger-x64.exe config:reload
```

开机自启由浮层或 CLI 管理，保存在当前用户的 Windows Run 注册表项中。

## ⌨️ 命令行

不带参数运行 EXE 即启动托盘程序。

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

修改 `nosleep` 或 `monitor` 状态，以及执行 `config:reload`，都需要托盘程序正在运行。状态查询和一次性电源动作不受此限制。

## 🔄 升级或移动

替换或移动 EXE 前，先退出 IdleTrigger，并保留同目录的 `IdleTrigger.toml` 和 `IdleTrigger.state.json`。移动后请从新位置启动一次，以刷新开机自启路径。

可使用 [SHA256SUMS.txt](https://github.com/JeffioZ/IdleTrigger/releases/latest/download/SHA256SUMS.txt) 校验下载的 EXE。

## 🧰 日志

在浮层开启“调试日志”，或设置 `logging_enabled = true`。日志优先保存在 EXE 同目录；目录不可写时使用 `%TEMP%`。
