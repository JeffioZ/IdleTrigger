# User guide / 使用指南

## Start and exit / 启动与退出

Put the EXE in a writable folder and run it. Left-click its tray icon to open or close the panel; right-click for Open and Exit. While a modal dialog is open, another tray click brings that dialog forward. One GUI instance runs per Windows session, even when different copies of the EXE are launched.

将 EXE 放入可写目录后运行。左键托盘图标开关浮层，右键可打开或退出。模态对话框打开时，再点托盘会聚焦当前窗口。同一 Windows 会话只运行一个界面实例，换目录启动另一份 EXE 也不会额外打开实例。

Enable Start with Windows in Settings if needed. If already enabled, startup repairs its registered path when you run the EXE from a new location. `--minimized` starts with the panel hidden; `--delay=N` delays initialization by up to 60 seconds.

可在设置中启用自启动。若已启用自启动，将 EXE 移至新目录并运行后会自动更新注册路径。`--minimized` 隐藏初始面板，`--delay=N` 最多延迟初始化 60 秒。

## Power management / 电源管理

- **Stay Awake** prevents automatic system sleep. Display-on is a separate option. Battery restrictions pause the effective state without changing your saved switch.
- **Idle Monitoring** uses Windows keyboard/mouse inactivity and starts a fresh countdown when enabled. Input, cancellation, or a changed action invalidates an existing warning. Zero warning seconds disables the warning, not the action.
- Stay Awake takes precedence when both features are requested. The panel shows the effective state after task overrides and pauses.
- Enhanced monitoring is optional, for periodic resets of the Windows idle counter; ordinary input still resets the timer.

- **保持唤醒**阻止系统自动睡眠；屏幕常亮是单独选项。电池限制暂停实际状态，不改写已保存开关。
- **空闲监测**读取 Windows 键鼠空闲状态，启用时重新计时。输入、取消或修改动作会使已有提醒失效；提醒秒数为 0 时仍会执行动作。
- 两项功能同时收到启用请求时，保持唤醒优先。面板显示合并任务覆盖和暂停条件后的状态。
- 增强监测用于空闲计数被周期性重置的情况，默认关闭；普通输入仍重置计时。

## Automatic tasks / 自动任务

Use **Manage Tasks → New**, then choose an action and its trigger. State tasks temporarily request Stay Awake or Idle Monitoring. Event tasks offer built-in Windows actions with a cancellable countdown of at least 10 seconds. Disabling or changing a rule cancels an outdated pending action.

在“管理任务 → 新建”中选择操作及触发条件。状态任务临时请求保持唤醒或空闲监测；事件任务只提供内置 Windows 操作，包含至少 10 秒的可取消倒计时。禁用或修改规则会取消已经失效的待执行动作。

Process-name targets match all instances of that name; Browse selects an exact EXE path. The picker reads names, counts, and file descriptions. It does not launch files, save PIDs, or inspect process memory. Search and sorting preserve checked targets, including those hidden by the filter.

进程名匹配全部同名实例，浏览选择的 EXE 按路径匹配。选择器读取名称、实例数及文件说明，不启动文件、不保存 PID、不读取进程内存。筛选、排序保留已勾选目标，包括被筛选隐藏的项。

A shared scan runs about every five seconds, so very short processes can be missed. Process-start triggers establish a baseline: already-running processes are not new starts. Disappearance has a five-second grace period; failed scans do not mean that all processes exited. Scheduled process conditions can skip a blocked occurrence or wait until its deadline. A window crossing midnight uses its starting weekday. Tasks require IdleTrigger to be running and are not installed into Windows Task Scheduler.

共享扫描约每 5 秒一次，极短进程可能漏检。进程启动触发器先建立基线，已有进程不算新启动；消失有 5 秒宽限，扫描失败不等于全部退出。计划进程条件不满足时可跳过或等待至截止时间。跨午夜时间段按开始日判断星期。任务依赖 IdleTrigger 运行，不会安装到 Windows 任务计划程序。

Schedules remain due during the scheduled minute and the following minute, including after startup or resume; older times are skipped. If saving an occurrence checkpoint fails, the action still proceeds and the error is logged. Restarting before the record can be saved may allow it to run again.

计划在指定分钟和下一分钟内仍可触发，包括启动或恢复之后；超过此窗口不补执行。触发记录保存失败时仍继续执行并记日志，记录成功保存前重启可能再次触发。

## Themes and notifications / 主题与提示

Theme scheduling supports fixed times and sunrise/sunset. Optional IP lookup uses `ipwho.is`, caching successful coordinates in memory for 24 hours and retrying failures after 30 minutes. Fallbacks are Windows timezone, UTC offset, and default coordinates. Polar day/night uses the configured fixed times.

昼夜主题支持固定时间和日出日落。可选 IP 定位使用 `ipwho.is`，成功结果在内存缓存 24 小时，失败后 30 分钟再试；依次使用 Windows 时区、UTC 偏移、默认位置兜底。极昼极夜使用配置中的固定时间。

Fullscreen/presentation and foreground game activity can pause automatic switching. Battery dark mode can override the schedule. A manual switch lasts until the next scheduled transition. Theme repair is available on demand and reports failures. Automatic switching also coalesces display/resume events, waits for a stable display configuration, and limits repeated repairs. Lock-key notices can be enabled independently and hidden during fullscreen use.

全屏、演示和前台游戏活动可暂停自动切换。电池深色选项可覆盖计划，手动切换保留至下一次计划转换。可手动运行主题修复并查看失败提示；自动切换也会合并显示器和恢复事件，等待显示配置稳定，并限制重复修复。锁定键提示可单独启用，并在全屏时隐藏。

## Files and saves / 文件与保存

| File / 文件 | Purpose / 用途 |
| --- | --- |
| `IdleTrigger.toml` | Settings and rules / 设置与规则 |
| `IdleTrigger.state.json` | Occurrence checkpoints / 任务触发记录 |
| `IdleTrigger.log`, `IdleTrigger.log.1` | Optional rotating diagnostics / 可选轮转日志 |
| `IdleTrigger-panic.txt` | Fatal-error details when available / 严重错误详情 |

Files live beside the EXE. Loading does not rewrite configuration. App saves preserve comments, unrelated keys, and unchanged rule text; failures or conflicts keep the editor open. Valid external edits reload automatically. Invalid reloads retain the last valid runtime configuration: repair the file and save again. Back up both TOML and state files before restoring a setup.

文件均位于 EXE 旁。读取配置不改写文件，保存保留注释、无关字段及未修改规则原文；失败或冲突时保留编辑窗口。有效外部修改会自动加载，错误重载保留最近的有效运行配置，修正文件后重新保存即可。恢复配置前建议备份 TOML 和 state 文件。

## Command line / 命令行

Run from a terminal; `help` lists options. Tray-state commands require a running instance. Errors return a nonzero exit code. Direct system actions execute immediately, outside the automatic-task countdown.

从终端运行，`help` 查看选项。控制托盘状态的命令要求主程序已运行，失败返回非零退出码。直接系统操作立即执行，不走自动任务倒计时。

| Command / 命令 | Effect / 效果 |
| --- | --- |
| `help`, `version` | Usage and version / 用法与版本 |
| `status` | Effective status / 实际状态 |
| `nosleep on\|off\|toggle\|status` | Stay Awake; `--screen` also requests display-on / 保持唤醒，`--screen` 同时请求屏幕常亮 |
| `monitor on\|off\|toggle\|status` | Idle Monitoring / 空闲监测 |
| `config:reload` | Reload settings / 重载配置 |
| `autostart enable\|disable\|status` | Login startup / 登录自启动 |
| `lock`, `sleep`, `hibernate`, `shutdown`, `restart` | Direct Windows action / 直接执行系统操作 |

For reports, include Windows/app versions, language, display scaling, reproduction steps, and relevant screenshots. Review logs and configuration for private paths before sharing.

反馈时请附系统及应用版本、语言、显示缩放、复现步骤和截图。分享日志、配置前检查其中的私人路径。
