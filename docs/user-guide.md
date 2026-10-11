# User guide / 使用指南

## Start and exit / 启动与退出

Put the EXE in a writable folder and run it. Left-click its tray icon to open or close the panel (Esc closes it too); right-click for Open, Check for Updates, and Exit. While a modal dialog is open, another tray click brings that dialog forward. One GUI instance runs per Windows session, even when different copies of the EXE are launched.

将 EXE 放入可写目录后运行。左键托盘图标开关浮层（按 Esc 也可关闭）；右键菜单包含打开、检查更新和退出。模态对话框打开时，再点托盘会聚焦当前窗口。同一 Windows 会话只运行一个界面实例，换目录启动另一份 EXE 也不会额外打开实例。

Enable Start with Windows in Settings if needed. If already enabled, startup repairs its registered path when you run the EXE from a new location. `--minimized` starts with the panel hidden; `--delay=N` delays initialization by up to 60 seconds.

可在设置中启用自启动。若已启用自启动，将 EXE 移至新目录并运行后会自动更新注册路径。`--minimized` 隐藏初始面板，`--delay=N` 最多延迟初始化 60 秒。

The panel’s **Appearance…** link opens theme-linked appearance settings. **Lock key notifications** can be toggled directly in the panel; **Settings… → Notifications** opens the remaining notification options.

主面板昼夜模式区域的**外观联动…**可直达联动设置。**锁定键状态提示**可直接在主面板开关，通过**设置… → 屏幕提示**可选择提示按键及全屏时是否显示。

## Explorer context menu / 资源管理器右键增强

By default, one **IdleTrigger** menu contains the **Copy Path** submenu followed by your custom rules. Only rules applicable to the selected location are included. Turn off **Group in an IdleTrigger submenu** in **Menu options** to show the entries separately. Existing configurations without `ctx_menu_grouped` also default to this unified layout; `ctx_rules_submenu` controls the separate rules cascade only when grouping is off.

默认将全部功能收入一个 **IdleTrigger** 菜单：顶部是**复制路径**子菜单，下方直接显示适用于当前位置的自定义规则。在**菜单选项**中关闭**合并到 IdleTrigger 子菜单**可恢复分开展示。旧配置缺少 `ctx_menu_grouped` 时也默认使用统一菜单；`ctx_rules_submenu` 只在关闭统一菜单后控制规则是否另设子菜单。

Open **Context menu** from the panel. Its master switch applies immediately. **Custom rules** shows the saved list; choose **New rule**, double-click a row, or press Enter/F2 to edit. Save returns to the list with the saved rule selected; Cancel discards that edit. Escape goes back from the editor and asks about unsaved changes, while Ctrl+S saves. Advanced options contain the working folder, icon, and console setting. Switching to **Menu options** preserves an unfinished rule draft.

**Menu options** saves changes immediately and includes grouping, copy tools, path quoting, and the separator for multiple copied paths. The Windows 11 classic-menu switch appears only on Windows 11; confirmation restarts Explorer. With it off, entries remain under **Show more options**. Turning it off removes the current-user classic-menu override, including one created by another tool. If Explorer recovery cannot be confirmed, the app gives recovery instructions. Editing `ctx_win11_classic` in the file alone does not change Windows.

从面板打开**右键增强**。顶部总开关立即生效；**自定义规则**展示已保存的列表，点击**新建规则**或双击列表项进入编辑，也可按 Enter / F2。保存后返回列表并保留选中项；取消会放弃本次编辑。Esc 从编辑区返回，有修改时会询问；Ctrl+S 保存。工作目录、图标和隐藏控制台位于**高级选项**。切换到**菜单选项**时会保留尚未保存的规则草稿。

**菜单选项**自动保存，包含菜单合并、复制功能、路径引号及多选复制的分隔方式。经典菜单开关仅在 Windows 11 显示，确认后重启资源管理器。关闭时通过**显示更多选项**使用；开启后右键直接可见。关闭该选项会移除当前用户的经典菜单覆盖设置，包括其他工具创建的设置。若无法确认资源管理器恢复，会显示恢复方法；手动修改配置中的 `ctx_win11_classic` 不会切换系统菜单。

- **Copy path** reads the complete selection and writes once: full paths, shortcut targets, parent folders, file names, stems, or extensions. Shortcut targets appear for `.lnk` files. Results follow Explorer's order, use one line per item by default, and apply the selected quoting policy to each path. Set `ctx_copy_separator = "space"` to use spaces. Unreadable paths abort the operation instead of silently omitting items. Folder-background copying uses the current folder.
- **Custom rules** provide a title, program, scope, extension filter, arguments, icon, working folder, enabled state, and selection mode. **Show for one item only** hides the rule for multiple selections. **Run once per item** preserves the previous behavior; Explorer normally limits this mode to 15 items. **Run once for all items** runs the program once with the complete selection. Folder-background rules do not need a selection mode.
- Use `"%1"` for one path (`%L` and `%V` are equivalent), or the standalone argument `%*` for all paths with automatic quoting. Together mode requires `%*` for item selections. Selecting it replaces the default `"%1"` template with `%*`; custom arguments are preserved for you to adjust. The target program must accept multiple path arguments. **Preview** shows sample expansion without executing anything. An empty working folder uses the first selected folder or the parent of the first selected file.
- Filters accept simple extensions such as `*.txt;*.dll`, `.txt`, or `txt`; empty means all types. Compound extensions such as `*.tar.gz` and other wildcards are rejected. Selection scope and filters are checked before launch; a batch with any incompatible item is rejected as a whole. Oversized expanded command lines are rejected before starting a program.
- **Save rule** returns to the list with confirmation. Further changes start by opening the rule again. **Cancel** abandons the current edit. Back/Close with unsaved edits asks how to proceed. External changes cannot be overwritten silently; cancel the edit and reopen the rule to load the latest version. Same-title rules remain distinct. Enable/Disable in the list applies immediately to the selected item.
- Up/Down changes order. Adding a separator saves it immediately after resolving any pending draft. Enabled separators require a submenu and appear only between rules. Invalid rules are retained but disabled. At most 32 rules are registered; oversized external documents remain intact. Before moving or deleting the EXE, disable the feature; after moving an enabled copy, exit the old instance and run the new one to repair registered paths.

- **复制路径**一次读取完整选择集并写入剪贴板，支持完整路径、快捷方式目标、所在目录、文件名、不含扩展名的名称和扩展名；快捷方式目标仅对 `.lnk` 显示。顺序跟随资源管理器，默认每项一行，每个路径分别应用引号设置；配置 `ctx_copy_separator = "space"` 可改为空格连接。读取失败会中止，不会静默漏项。空白处复制使用当前文件夹。
- **自定义规则**支持名称、程序、范围、扩展名、参数、图标、工作目录、启停和多选行为。**仅单选时显示**会在多选时隐藏规则；**每个项目单独运行**保留旧行为，通常最多支持 15 个所选项；**所有项目一起运行**只启动一次程序。文件夹空白处不显示多选设置。
- `"%1"` 传入一个路径，`%L`、`%V` 与之相同；独立参数 `%*` 传入全部路径并自动逐项加引号。一起传入模式处理选中项时必须使用 `%*`。切换该模式会把默认 `"%1"` 替换成 `%*`，自定义参数保留供调整。目标程序也必须支持多个路径参数。**预览参数**仅展示示例，不执行程序；工作目录留空时使用第一个所选文件夹或第一个文件的所在目录。
- 过滤支持 `*.txt;*.dll`、`.txt`、`txt` 等简单扩展名，留空不限类型；不支持 `*.tar.gz` 等复合扩展名及其他通配符。执行前检查范围和类型，批量选择中存在不适用项时整批中止；展开后的命令行过长也会在启动前中止。
- **保存规则**后返回列表并显示结果，继续修改时重新打开该规则；**取消**放弃本次编辑。返回或关闭窗口时，有未保存修改会询问处理方式。外部配置变化不会被静默覆盖，取消编辑后重新打开可加载最新版本。同名规则分别维护。列表底部的启用 / 停用立即作用于当前选中项。
- 上移、下移改变顺序；添加分隔线会先处理未保存草稿，再立即保存。启用的分隔线要求子菜单，且只显示在规则之间。损坏规则禁用并保留；最多注册 32 条，外部超额配置不会截断。移动或删除 EXE 前应关闭功能；启用时移动了程序，则退出旧实例后运行新副本修复菜单路径。

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

Files live beside the EXE. Loading does not rewrite configuration. App saves preserve comments, unrelated keys, and unchanged rule text; failures or conflicts keep the editor open. Valid external edits reload automatically. External edits with syntax errors, mistyped settings, or malformed rule containers retain the last valid runtime configuration and menu registrations: repair the file and save again. Individual invalid rules remain disabled and retained. A field with the wrong type (for example, a number written as text) falls back to its default with a startup notice, and saving once rewrites it correctly. Back up both TOML and state files before restoring a setup.

文件均位于 EXE 旁。读取配置不改写文件，保存保留注释、无关字段及未修改规则原文；失败或冲突时保留编辑窗口。有效外部修改会自动加载，外部修改存在语法错误、设置字段类型错误或规则列表结构损坏时，保留最近的有效运行配置与菜单注册，修正文件后重新保存即可；单条规则错误仍按禁用并保留处理。字段类型写错（如数字写成文本）时回退默认值并在启动时提示，保存一次即可改正。恢复配置前建议备份 TOML 和 state 文件。

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
