# User guide / 使用指南

## Start and exit / 启动与退出

Put the EXE in a writable folder and run it. Left-click its tray icon to open or close the panel (Esc closes it too); right-click for Open, Check for Updates, and Exit. While a modal dialog is open, another tray click brings that dialog forward. One GUI instance runs per Windows session, even when different copies of the EXE are launched.

将 EXE 放入可写目录后运行。左键托盘图标开关浮层（按 Esc 也可关闭）；右键菜单包含打开、检查更新和退出。模态对话框打开时，再点托盘会聚焦当前窗口。同一 Windows 会话只运行一个界面实例，换目录启动另一份 EXE 也不会额外打开实例。

Enable Start with Windows in Settings if needed. If already enabled, startup repairs its registered path when you run the EXE from a new location. `--minimized` starts with the panel hidden; `--delay=N` delays initialization by up to 60 seconds.

可在设置中启用自启动。若已启用自启动，将 EXE 移至新目录并运行后会自动更新注册路径。`--minimized` 隐藏初始面板，`--delay=N` 最多延迟初始化 60 秒。

## Explorer context menu / 资源管理器右键增强

Enable **Settings → Context menu** and save to register the menus for the current Windows user. On Windows 11 they normally appear under **Show more options**. The optional classic-menu switch applies immediately after confirmation and restarts Explorer; save work in Explorer windows first. IdleTrigger does not claim a classic-menu override created by another tool. The `ctx_win11_classic` configuration value records the last UI action; editing that value alone does not change Windows.

在**设置 → 右键增强**中启用并保存后，菜单注册到当前 Windows 用户。Windows 11 默认在**显示更多选项**中显示。经典菜单开关在确认后立即生效并重启 Explorer，请先处理好已打开的资源管理器窗口；IdleTrigger 不接管其他工具设置的经典菜单。配置中的 `ctx_win11_classic` 仅记录界面操作，手工修改该值不会改动系统。

- **Copy path** supports one selected item: full path, shortcut target, parent folder, file name, name without extension, or extension. Shortcut target is shown only for `.lnk` files. It also works without the tray app running. Multi-selection is deliberately unavailable: separate static-verb processes cannot reliably identify one complete selection, and must not silently overwrite or combine different copy operations. The old join-separator preference remains in the configuration for compatibility.
- **Custom rules** have a title, program, scope, extension filter, arguments, optional icon and working folder, and individual enabled state. New rules start with `"%1"` as the argument template; `%1`, `%L`, and `%V` substitute the selected path once. **Preview arguments** uses an example path and does not run the command. An empty working folder uses the selected folder, or a selected file's parent.
- Filters accept simple extensions such as `*.txt;*.dll`, `.txt`, or `txt`; empty means all types. Compound patterns such as `*.tar.gz` and other wildcards are rejected. Scope and filtering apply in both flat and submenu modes. Rules can launch once per selected item; Explorer's Document selection model normally limits these verbs to 15 selected items.
- Save and Cancel apply to the rule being edited. Switching, adding, or closing with a draft asks whether to save or discard it. If the configuration changes externally, saving refuses to overwrite it; Cancel reloads the latest rules. Same-title rules remain separate. Invalid entries are disabled and retained so other rules can still be edited. At most 32 rules are registered; oversized external documents remain intact and should be reduced in the manager.
- Move Up/Down changes menu order. An enabled separator requires submenu mode; leading and trailing separators are omitted. The manager explains when the master switch is off or registration needs repair. Use **More settings → Repair** for the saved configuration. Turn the master switch off and save before removing or moving the EXE; if moved while enabled, run the new copy after exiting the old instance to repair paths.

- **复制路径**支持单个所选项目的完整路径、快捷方式目标、所在文件夹、文件名、不含扩展名的名称或扩展名；目标路径仅对 `.lnk` 显示。托盘未运行时也可使用。当前明确不支持多选：多个静态菜单进程无法可靠识别一次完整选择，不能静默覆盖路径或混入另一次复制；旧拼接分隔符配置仅为兼容保留。
- **自定义规则**可编辑标题、程序、作用范围、扩展名过滤、参数、图标、工作目录及单独启停。新规则默认参数为 `"%1"`；`%1`、`%L`、`%V` 只替换一次所选路径。**预览参数**使用示例路径，不执行命令。工作目录留空时，使用所选文件夹或所选文件的所在目录。
- 过滤支持 `*.txt;*.dll`、`.txt`、`txt` 等简单扩展名，留空不限类型；不支持 `*.tar.gz` 等复合模式及其他通配符。平铺和子菜单模式均遵循范围及过滤条件。规则可对每个所选项目分别启动一次；Explorer 的 Document 模型通常将这类菜单限制在 15 个所选项目以内。
- 保存和取消针对当前规则。切换、新建、关闭前有草稿时，会询问保存或放弃。检测到外部配置变化时拒绝覆盖，点击取消可加载最新规则；同名规则仍分别维护。损坏规则禁用并保留，不阻止编辑其他正常规则。最多注册 32 条；外部超额配置不截断，应在管理器中减少数量。
- 上移、下移会改变菜单顺序。点击“分隔线”直接追加到列表末尾并保存；列表为空、末尾已有分隔线或达到数量上限时不可添加。有未保存草稿时会先询问如何处理。启用的分隔线会使子菜单模式生效，首尾分隔线不显示；末尾分隔线可在后方添加规则后用于分组，也可上移。管理器会说明总开关关闭、注册不完整等状态；**更多设置 → 修复**依据已保存配置执行。删除或移动 EXE 前应关闭总开关并保存；启用时移动了程序，则先退出旧实例，再运行新副本修复菜单路径。

## Power management / 电源管理

- **Stay Awake** prevents automatic system sleep. Display-on is a separate option. Battery restrictions pause the effective state without changing your saved switch. An optional lock pause pauses it while the workstation is locked, resuming the previous state on unlock. An armed lock pause or battery policy is noted on the Stay Awake tooltip.
- **Timed Stay Awake** chips arm a temporary overlay for 30 minutes, 1 hour, or 2 hours without touching the saved switch; it expires automatically, a restart clears it, any request that turns Stay Awake off drops it, and clicking the armed chip again (or the cancel link on the status line) ends it early. The panel status line and tray tooltip show what is keeping the machine awake — which task rule, or the timed overlay with its remaining time.
- **Idle Monitoring** uses Windows keyboard/mouse inactivity and starts a fresh countdown when enabled. Input, cancellation, or a changed action invalidates an existing warning. The warning countdown runs at least 10 seconds and can always cancel the action.
- Stay Awake takes precedence when both features are requested. The panel shows the effective state after task overrides and pauses.
- Enhanced idle detection is optional, for periodic resets of the Windows idle counter; ordinary input still resets the timer. While it is on, the panel's idle status line and the idle row tooltip say so.

- **保持唤醒**阻止系统自动睡眠；屏幕常亮是单独选项。电池限制暂停实际状态，不改写已保存开关；锁定期间可选暂停，解锁后恢复原状态。锁定暂停与电池策略开启后，保持唤醒的悬停提示会标注。
- **限时保持唤醒**通过面板预设临时保持 30 分钟、1 小时或 2 小时，不改写已保存开关；到期自动恢复，重启后消失，任何关闭保持唤醒的请求都会同时取消限时，再次点击已选预设或状态行的取消链接可提前结束。面板状态行与托盘 tooltip 会说明当前是谁在阻止睡眠——任务规则，或限时及其剩余时间。
- **空闲监测**读取 Windows 键鼠空闲状态，启用时重新计时。输入、取消或修改动作会使已有提醒失效；提醒倒计时最少 10 秒，且始终可取消本次动作。
- 两项功能同时收到启用请求时，保持唤醒优先。面板显示合并任务覆盖和暂停条件后的状态。
- 增强空闲监测用于空闲计数被周期性重置的情况，默认关闭；普通输入仍重置计时。开启期间，面板空闲状态行与空闲监测行的 tooltip 会标明。

## Automatic tasks / 自动任务

Use **Manage Tasks → New**, which opens a template menu: keep awake while a process runs, a nightly shutdown, stay awake during work hours, or a blank rule. Templates only prefill the editor — every field stays editable. State tasks temporarily request Stay Awake or Idle Monitoring. Event tasks offer built-in Windows actions (lock, sleep, hibernate, shutdown, restart, turn off display, log off) plus session, resume, and battery-threshold triggers, with a cancellable countdown of at least 10 seconds. A battery rule fires once when the percentage drops below its level and re-arms after recovery. Disabling or changing a rule cancels an outdated pending action.

在“管理任务 → 新建”中先从模板菜单选择：进程运行时保持唤醒、每天定时关机、工作时段保持唤醒，或空白规则。模板只负责预填编辑器——所有字段仍可修改。状态任务临时请求保持唤醒或空闲监测；事件任务提供内置 Windows 操作（锁定、睡眠、休眠、关机、重启、关闭显示器、注销）与锁定/解锁、从睡眠恢复、电量阈值触发器，包含至少 10 秒的可取消倒计时。电量规则在百分比低于阈值时触发一次，回充到阈值后重新武装。禁用或修改规则会取消已经失效的待执行动作。

Process-name targets match all instances of that name; Browse selects an exact EXE path. The picker reads names, counts, and file descriptions. It does not launch files, save PIDs, or inspect process memory. Search and sorting preserve checked targets, including those hidden by the filter.

进程名匹配全部同名实例，浏览选择的 EXE 按路径匹配。选择器读取名称、实例数及文件说明，不启动文件、不保存 PID、不读取进程内存。筛选、排序保留已勾选目标，包括被筛选隐藏的项。

A shared scan runs about every five seconds, so very short processes can be missed. Process-start triggers establish a baseline: already-running processes are not new starts. Disappearance has a five-second grace period; failed scans do not mean that all processes exited. Scheduled process conditions can skip a blocked occurrence or wait until its deadline. A window crossing midnight uses its starting weekday. Equal start and end times mean the entire selected day. Tasks require IdleTrigger to be running and are not installed into Windows Task Scheduler.

共享扫描约每 5 秒一次，极短进程可能漏检。进程启动触发器先建立基线，已有进程不算新启动；消失有 5 秒宽限，扫描失败不等于全部退出。计划进程条件不满足时可跳过或等待至截止时间。跨午夜时间段按开始日判断星期；开始和结束时间相同时，表示所选日期全天生效。任务依赖 IdleTrigger 运行，不会安装到 Windows 任务计划程序。

Schedules remain due during the scheduled minute and the following minute, including after startup or resume; older times are skipped. A scheduled action is queued only after its occurrence checkpoint is saved successfully. Save failures skip the action and display an error; the same occurrence is not retried during that app session. After restarting, an unsaved occurrence can be reconsidered if it is still within the grace window, but must be saved before it can run.

计划在指定分钟和下一分钟内仍可触发，包括启动或恢复之后；超过此窗口不补执行。计划动作只有在触发记录成功保存后才进入倒计时。保存失败会跳过动作并提示错误，同次运行不重试该次计划；重启后，未保存的记录若仍在容差窗口内可重新判断，但仍须先成功保存才能执行。

## Themes and notifications / 主题与提示

Theme scheduling supports fixed times and sunrise/sunset. Optional IP lookup uses `ipwho.is`, caching successful coordinates in memory for 24 hours and retrying failures after 30 minutes. Fallbacks are Windows timezone, UTC offset, and default coordinates. Polar day/night uses the configured fixed times.

昼夜主题支持固定时间和日出日落。可选 IP 定位使用 `ipwho.is`，成功结果在内存缓存 24 小时，失败后 30 分钟再试；依次使用 Windows 时区、UTC 偏移、默认位置兜底。极昼极夜使用配置中的固定时间。

Fullscreen/presentation and foreground game activity can pause automatic switching. Battery dark mode can override the schedule, and while it is in effect the schedule line reads "On battery: dark". A manual switch lasts until the next scheduled transition, and the panel's snooze chips postpone the next scheduled switch by 30 minutes, 1 hour, or until the next light boundary — the schedule line shows the deadline, and clicking the armed chip again or the cancel link clears it. Win+Shift+D performs a manual switch when global hotkeys are enabled. Optional appearance linkage applies a per-side wallpaper and an installed cursor scheme together with each switch; failures are logged and never block the theme itself. Theme repair is available on demand and reports failures. Automatic switching also coalesces display/resume events, waits for a stable display configuration, and limits repeated repairs. Lock-key notices can be enabled independently and hidden during fullscreen use.

全屏、演示和前台游戏活动可暂停自动切换。电池深色选项可覆盖计划，生效期间调度行显示「电池供电：深色」。手动切换保留至下一次计划转换；面板上的推迟预设可将下一次计划切换延后 30 分钟、1 小时或直至下一次浅色边界——调度行显示截止时间，再次点击已选预设或取消链接可立即恢复。启用全局热键时，Win+Shift+D 可手动切换昼夜。可选的外观联动会在每次切换时同时应用对应的壁纸与已安装指针方案；单项失败只记录日志，不阻塞主题切换本身。可手动运行主题修复并查看失败提示；自动切换也会合并显示器和恢复事件，等待显示配置稳定，并限制重复修复。锁定键提示可单独启用，并在全屏时隐藏。

## Self-update / 自动更新

The panel checks quietly in the background each time it is shown (at most once a minute, switchable in Settings). The tray menu and Settings also offer an explicit **Check for updates**: up-to-date and failure answers arrive as standalone dialogs, and a found update opens the one-click confirm directly. One confirmation downloads, SHA-256-verifies, installs, and restarts the app; the `update/` staging folder beside the EXE is reclaimed automatically after a successful upgrade. A staging folder that holds files the updater did not create is left untouched.

每次打开控制面板时在后台静默检查一次（每分钟最多一次，可在设置中关闭）。托盘菜单与设置中的**检查更新**为显式入口：已是最新或失败会以独立提示框作答，发现新版则直接弹出一次性确认。确认后自动下载、SHA-256 校验、安装并重启；EXE 旁的 `update/` 暂存目录在升级成功后自动回收，若其中存在非更新程序创建的文件则原样保留。

## Files and saves / 文件与保存

| File / 文件 | Purpose / 用途 |
| --- | --- |
| `IdleTrigger.toml` | Settings and rules / 设置与规则 |
| `IdleTrigger.state.json` | Occurrence checkpoints / 任务触发记录 |
| `IdleTrigger.state.json.corrupt-*` | Backup kept when the state file could not be parsed / 状态文件无法解析时保留的备份 |
| `update/` | Self-update staging (downloaded EXE, checksums, replace script); reclaimed automatically after a successful upgrade / 自更新暂存（下载的程序、校验与替换脚本），升级成功后自动回收 |
| `IdleTrigger.log`, `IdleTrigger.log.1` | Optional rotating diagnostics / 可选轮转日志 |
| `IdleTrigger-panic.txt` | Fatal-error details when available / 严重错误详情 |

Files live beside the EXE. Loading does not rewrite configuration. App saves preserve comments, unrelated keys, and unchanged rule text; failures or conflicts keep the editor open. Valid external edits reload automatically. Invalid reloads retain the last valid runtime configuration: repair the file and save again. A field with the wrong type (for example, a number written as text) falls back to its default with a startup notice, and saving once rewrites it correctly. Back up both TOML and state files before restoring a setup.

文件均位于 EXE 旁。读取配置不改写文件，保存保留注释、无关字段及未修改规则原文；失败或冲突时保留编辑窗口。有效外部修改会自动加载，错误重载保留最近的有效运行配置，修正文件后重新保存即可。字段类型写错（如数字写成文本）时回退默认值并在启动时提示，保存一次即可改正。恢复配置前建议备份 TOML 和 state 文件。

## Command line / 命令行

Run from a terminal; `help` lists options. Tray-state commands require a running instance. Errors return a nonzero exit code. Direct system actions execute immediately, outside the automatic-task countdown.

从终端运行，`help` 查看选项。控制托盘状态的命令要求主程序已运行，失败返回非零退出码。直接系统操作立即执行，不走自动任务倒计时。

| Command / 命令 | Effect / 效果 |
| --- | --- |
| `help`, `version` | Usage and version / 用法与版本 |
| `status` | Effective status / 实际状态 |
| `nosleep on\|off\|toggle\|status` | Stay Awake; `--screen` also requests display-on; `on --for <n>[m\|h\|s]` keeps awake for a limited time (bare numbers are minutes) and expires automatically / 保持唤醒，`--screen` 同时请求屏幕常亮；`on --for <n>[m\|h\|s]` 限时保持（不带单位按分钟），到期自动恢复 |
| `monitor on\|off\|toggle\|status` | Idle Monitoring / 空闲监测 |
| `config:reload` | Reload settings / 重载配置 |
| `autostart enable\|disable\|status` | Login startup / 登录自启动 |
| `lock`, `sleep`, `hibernate`, `shutdown`, `restart`, `logoff` | Direct Windows action / 直接执行系统操作 |
| `screen off` | Turn off the display / 关闭显示器 |

For reports, include Windows/app versions, language, display scaling, reproduction steps, and relevant screenshots. Review logs and configuration for private paths before sharing.

反馈时请附系统及应用版本、语言、显示缩放、复现步骤和截图。分享日志、配置前检查其中的私人路径。
