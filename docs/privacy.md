# Privacy / 隐私

IdleTrigger runs entirely on your PC and makes no telemetry, crash-reporting, or analytics requests. There is no account, no machine identifier, and no usage statistics. Everything the app stores — the TOML configuration and the optional log file — stays in files beside the EXE.

IdleTrigger 完全运行在本机，不含遥测、崩溃上报或分析统计；没有账号、机器标识或使用统计。程序保存的所有内容（TOML 配置与可选日志）都留在 EXE 旁边的文件里。

## Network use / 联网行为

Only two features ever touch the network, and both contact public services directly with plain GET requests:

只有两个功能会联网，且都是向公共服务直接发起普通 GET 请求：

- **Update check** (on by default, `update_check_enabled`): fetches this project's release feed on GitHub when the control panel opens, plus manual checks from the tray menu and Settings. After you confirm an update, the new EXE is downloaded from GitHub and verified with SHA-256. Turn the automatic check off in Settings if you prefer to update manually.

- **自动更新检查**（默认开启，`update_check_enabled`）：打开控制面板时获取本项目的 GitHub 发布 feed，也可从托盘菜单与设置手动检查；确认更新后从 GitHub 下载新 EXE 并做 SHA-256 校验。如需纯手动更新，可在设置中关闭自动检查。

- **Sunrise/sunset IP location** (off by default, `theme_ip_location_enabled`): when enabled, queries `https://ipwho.is/` once to estimate the city used for sunrise and sunset times. A successful result is cached in memory for 24 hours and never written to the config file; failures retry after 30 minutes. When off or unavailable, scheduling falls back to the Windows timezone, then UTC offset, then your configured default coordinates.

- **日出日落 IP 定位**（默认关闭，`theme_ip_location_enabled`）：启用后请求一次 `https://ipwho.is/`，估算用于计算日出日落的城市；成功结果仅在内存缓存 24 小时，绝不写入配置文件；失败 30 分钟后重试。关闭或不可用时，依次回退到 Windows 时区、UTC 偏移和配置的默认位置。

Those services see what any HTTPS request from your PC reveals — your IP address and standard request headers. IdleTrigger sends nothing else: no identifiers, no diagnostics, no content from your files.

上述服务只能看到任何 HTTPS 请求都会暴露的信息——你的 IP 地址和标准请求头。除此之外 IdleTrigger 不发送任何内容：没有标识符、诊断数据或文件内容。

## Local data and system changes / 本地数据与系统改动

- The configuration lives in `config.toml` inside the per-user data folder `%LOCALAPPDATA%\IdleTrigger`; the optional `app.log` is written there only while logging is enabled. The first launch moves any legacy EXE-adjacent data files into this folder and removes the originals. Deleting the EXE folder plus the data folder removes the app together with its data.
- 配置保存在用户数据目录 `%LOCALAPPDATA%\IdleTrigger` 的 `config.toml`；可选的 `app.log` 仅在启用日志时写入该目录。首次启动会把 EXE 旁的历史数据文件自动迁入该目录并删除原件。删除 EXE 目录与数据目录即可同时移除程序与数据。
- Enabling **Start with Windows** adds a per-user Run entry to the registry, and the Explorer context-menu enhancement registers its entries in the registry as well.
- 启用「开机自启」会在注册表写入当前用户的 Run 项；资源管理器右键增强同样通过注册表注册条目。

Questions or concerns? [Open an issue](https://github.com/JeffioZ/IdleTrigger/issues).

有疑问或顾虑？[提一个 issue](https://github.com/JeffioZ/IdleTrigger/issues)。
