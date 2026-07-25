# 🛠️ 构建 IdleTrigger

[文档索引](README.md) · [English](development.md) · [项目主页](../README.zh-CN.md)

## 📦 前置条件

- Go 1.26 或更高版本
- Git
- 仓库脚本要求 PowerShell 7 或更高版本
- 仅重新生成 Windows 资源时需要 `github.com/akavel/rsrc`

IdleTrigger 面向 Windows 10 / Windows Server 2016 及以上系统，仓库同时产出 `windows/amd64` 和 `windows/386`。

主构建不支持 Windows 7。Go 1.20 是最后一个兼容版本。如未来确有需求，应独立维护 Go 1.20 legacy 分支，固定兼容依赖，并完成实机验证。

```powershell
go version
go mod download
```

## 🔨 构建

架构专用 `.syso` 文件包含图标、manifest 和 Windows 版本信息。它们是生成文件，不提交到仓库。构建前请重新生成，使资源管理器与应用显示相同版本。

```powershell
$env:CGO_ENABLED = "0"
$env:GOARCH = "amd64" # 32 位 Windows 使用 "386"
$version = "dev"
go run ./tools/resourcegen.go -version $version
$ldflags = "-s -w -H windowsgui -X github.com/JeffioZ/idletrigger/internal/version.Value=$version"
$output = if ($env:GOARCH -eq "amd64") { "IdleTrigger-x64.exe" } else { "IdleTrigger-x86.exe" }
go build -trimpath "-ldflags=$ldflags" -o $output ./cmd/idletrigger
```

`CGO_ENABLED=0` 使 EXE 保持自包含；`-H windowsgui` 避免托盘程序启动时显示控制台窗口。

## ✅ 验证

在 PowerShell 7 或更高版本中执行本地标准检查：

```powershell
.\tools\check.ps1
```

默认命令执行适合日常开发的轻量检查：模块校验、格式、工作区空白错误、短测试集、
普通 vet 和依赖边界。真实 Win32 集成及资源循环测试保留在完整模式中。发布前或较大
范围改动后，使用完整模式覆盖 `devtools`、`tools` 构建标签，并在已安装时执行
`golangci-lint`：

```powershell
.\tools\check.ps1 -Full
```

联网时可执行可选漏洞扫描；该参数可与 `-Full` 组合使用：

```powershell
.\tools\check.ps1 -Vulncheck
```

未传 `-Vulncheck` 时不会执行漏洞扫描，脚本会明确提示。若系统缓存目录不可写，
可在当前 shell 中先设置以下可选的用户本地缓存路径：

```powershell
$env:GOCACHE = Join-Path $env:LOCALAPPDATA "IdleTrigger\cache\go-build"
$env:GOLANGCI_LINT_CACHE = Join-Path $env:LOCALAPPDATA "IdleTrigger\cache\golangci-lint"
```

并显式构建两种架构：

```powershell
$env:CGO_ENABLED = "0"
$version = "dev"
go run ./tools/resourcegen.go -version $version
$ldflags = "-s -w -H windowsgui -X github.com/JeffioZ/idletrigger/internal/version.Value=$version"

$env:GOARCH = "amd64"
go build -trimpath "-ldflags=$ldflags" -o dist/IdleTrigger-x64.exe ./cmd/idletrigger

$env:GOARCH = "386"
go build -trimpath "-ldflags=$ldflags" -o dist/IdleTrigger-x86.exe ./cmd/idletrigger
```

发布工作流会先运行格式、依赖、test 和 vet 检查，再构建两种 EXE，并发布 `SHA256SUMS.txt`。

## 🚢 发布流程

推送 `v*` tag 后，工作流会创建名为 `IdleTrigger vX.Y.Z` 的草稿 Release。草稿包含两个 EXE、`SHA256SUMS.txt` 和初始变更记录。`.github/release.yml` 会对 PR 分组；直接提交仍需人工整理。

正式发布前，请应用[更新说明格式与模板](release-notes.md)。先写用户可感知的结果，保持中英文一致，核对附件和版本对比范围，并把提交明细留在末尾。

## 🧩 跨层改动检查清单

完成改动前，按涉及范围核对对应条目：

- **配置字段：** 同步更新 `Config`、默认值、规范化、校验、注释 TOML 输出、
  `IdleTrigger.example.toml`、相关 UI 文案和测试。
- **控制面板动作：** 同步更新控件 ID/选择路径、`controlpanel.Action` 映射、
  `internal/app` 唯一动作入口、配置保存、运行态协调、图标/计划刷新和映射测试。
- **用户可见文案：** 英文与简体中文同时更新，格式占位符保持一致，并检查
  tooltip、CLI 和截图调用方。
- **Windows 集成：** 业务策略不得下沉到 `internal/platform/windows`；检查错误和
  资源释放路径，并分别编译 386、amd64。
- **主题切换保护：** 配置兼容键保持为 `theme_skip_fullscreen`。先检查 Windows
  通知状态和 DWM 可见边界，再按需采样 GPU；GPU 只能统计前台进程的渲染引擎，须连续
  多次命中，且仅在主题确实待切换时运行，并支持取消和失败放行。不得申请管理员权限、
  打开前台进程或增加全局 GPU 常驻监测。
- **仅开发能力：** 保持在 `devtools` 构建标签后，更新依赖边界预期，并确认正式
  构建不包含该能力。
- **自动任务：** 同步更新 `internal/automation`、调度行为与测试、TOML 输出、
  中英文文案和 tooltip、任务管理器/进程选择器，以及 app 运行态协调；事件动作
  必须保留可取消倒计时。状态操作只能形成临时运行态请求，不得改写手动开关；同一功能
  同时收到启用和暂停请求时，暂停优先。进程启动事件必须先建立运行基线，只在所选目标
  从全部未运行变为至少一个运行时触发；进程退出事件继续等待全部匹配实例退出并遵守
  5 秒宽限，避免程序刚启动或短暂重启造成误触发。
- **原生表单窗口：** 自动任务、任务编辑、进程选择和动作倒计时应复用
  `internal/ui/nativeform` 的标题栏、深浅色图标、圆角输入外框及 hover/press/focus/disabled
  状态；下拉选项和复选框应延续控制浮层的视觉语言，不得退回未适配主题的原生样式。
  长下拉浮层与报表列表应复用同一套菜单行距、圆角和主题滚动条；owner-draw 控件须先在
  离屏缓冲中完成背景、边框和文字，再一次性提交，避免 hover 或页面切换时分帧闪烁。
  空输入框提示须使用随主题变化的弱化文字色，不得依赖深色模式下可能不可读的原生固定颜色。
  传给 `CreateWindowEx` 的外框尺寸须由
  目标客户区经 `AdjustWindowRectEx` 计算。窗口必须处理背景擦除、主题切换后的整窗
  重绘、DPI 变化和 owner 启停；嵌套窗口保持逐级模态关系。表单应使用明确标签、
  按当前选择渐进显示字段、就地校验并聚焦首个错误；关闭存在未保存修改的编辑器前须确认。
- **进程选择器：** 使用原生报表列表，以复选框承担多选并只保留一个当前焦点行；“进程名称、
  说明、实例数”三列须在可视宽度内完整容纳，默认不得出现横向滚动条。运行中进程按
  可执行文件名去重；精确文件只能由“浏览文件”加入，并保留在当前选择预览中。
  搜索、分阶段异步加载、空结果、刷新禁用、表头排序和当前选择预览均须保持可恢复状态。
  异步结果写入列表后须完成首帧重绘；可排序表头要提供 hover/press 状态，纵向滚动条须
  跟随深浅色主题且不得让列表重新产生横向滚动条。下拉浮层、任务列表、进程列表和当前
  选择预览应复用共享滚动条；窗口重新激活后的过期快照可轻量刷新，但须保留筛选、排序、
  勾选、焦点和可视锚点，并避免重复读取已有说明。
  名称匹配与精确文件匹配必须在预览中清楚区分，PID 和说明不得作为规则标识保存。
- **进程元数据：** 优先使用 Toolhelp 进程名；每个进程名最多从一个可访问实例读取代表性说明，
  使用有上限的工作线程，并仅在需要路径时申请 `PROCESS_QUERY_LIMITED_INFORMATION`。
  浏览的 EXE 可校验格式和读取说明，但不得启动；不得加入调试权限、进程内存访问、代码注入、
  结束进程、任意程序启动、服务或 Windows 任务计划。

自动检查会覆盖翻译 Key 引用、配置与示例一致性、控制面板动作路径、构建标签依赖
边界和包的直接分层关系。进程自动任务边界还会拒绝调试权限、进程内存、注入和
强制结束进程相关 API；这些检查用于补充而不是替代具体行为测试。

## 🎨 重新生成资源

主图标与两套托盘图标采用独立图稿。更新时先生成主 ICO，再生成专门适配任务栏的托盘变体：

```powershell
go run ./tools/appicon/main.go build/windows/icons
go run ./tools/trayicons/main.go build/windows/icons
```

然后重新生成两种架构资源。资源命令与发布构建必须使用同一个版本号：

```powershell
$version = Read-Host "请输入本次发布版本号（不带开头的 v，例如 1.9.0）"
go run ./tools/resourcegen.go -version $version
```

请将 `app.ico`、两个托盘 ICO、`build/windows/manifest.xml` 和生成器一并提交。不要提交 `.syso` 文件；发布工作流会按 tag 重新生成。

## 📸 重新生成 README 截图

截图生成属于维护能力，只在使用 `devtools` 构建标签时编译。辅助脚本会临时构建
devtools EXE、重新生成四张受版本管理的图片、校验 PNG 尺寸，并删除临时构建目录：

```powershell
.\tools\capture-screenshots.ps1
```

如只想验证截图流程而不覆盖仓库图片，可指定临时输出目录：

```powershell
.\tools\capture-screenshots.ps1 -OutputDirectory (Join-Path $env:TEMP "IdleTrigger-screenshots")
```

如需审查全部原生界面，可一次生成主界面、自动任务管理器、任务编辑器和进程选择器的
中英文、深浅色共 16 张图片。默认输出到已忽略的 `dist/ui-review/`，不会覆盖 README
现有四张公开截图：

```powershell
.\tools\capture-screenshots.ps1 -CaptureSet Review
```

底层 devtools 命令明确区分 `screenshot --readme-set` 与
`screenshot --review-set`；含义模糊的旧 `--all` 参数会被拒绝。

正式 EXE 明确不包含 `screenshot` 命令及其 PNG/压缩依赖。

## 📴 离线构建

先在联网环境 vendor 依赖，再将包含 `vendor/` 的仓库复制到离线机器：

```powershell
go mod vendor

$env:CGO_ENABLED = "0"
$env:GOARCH = "amd64"
go run ./tools/resourcegen.go -version dev
go build -mod=vendor -trimpath -ldflags="-s -w -H windowsgui" -o dist/IdleTrigger-x64.exe ./cmd/idletrigger
```

## 🔁 开发调试

```powershell
go test ./...
go run ./tools/resourcegen.go -version dev
$env:CGO_ENABLED = "0"
$env:GOARCH = "amd64"
go build -tags devtools -trimpath -ldflags="-H windowsgui" -o dist/IdleTrigger-x64-devtools.exe ./cmd/idletrigger
.\dist\IdleTrigger-x64-devtools.exe

# 托盘程序启动后，在第二个终端中执行：
cmd /c .\dist\IdleTrigger-x64-devtools.exe nosleep on
cmd /c .\dist\IdleTrigger-x64-devtools.exe nosleep status
cmd /c .\dist\IdleTrigger-x64-devtools.exe monitor on
cmd /c .\dist\IdleTrigger-x64-devtools.exe diagnostics idle
```

开发构建使用 Windows GUI 子系统，避免启动托盘程序时闪出控制台窗口。验证 CLI
输出时，请通过 `cmd /c` 执行，或用 `Start-Process` 重定向 stdout/stderr；
PowerShell 直接运行 GUI 子系统 EXE 时，可能在输出完成绑定前就返回。
`diagnostics`、截图和本机测试环境变量等维护能力只存在于 devtools 构建。

4 个面向使用者的人工验收启动脚本作为明确例外受版本管理，并放在本地构建旁边的
`dist/`，因此可以把 devtools EXE 直接拖到脚本上；公共实现仍集中在
`tools/devtools/`，不会形成重复逻辑。它们要求 PowerShell 7，默认使用
`dist/IdleTrigger-x64-devtools.exe`，会拒绝正式构建和已有的
IdleTrigger 实例，并确保开发变量只在子进程中生效：

```text
Start-IdleTrigger-Devtools-Idle-Monitor-Test.bat [devtools.exe] [10..600]
Start-IdleTrigger-Devtools-Input-Trace.bat [devtools.exe]
Start-IdleTrigger-Devtools-UI-Capture.bat [devtools.exe]
Start-IdleTrigger-Devtools-Warning-Preview.bat [devtools.exe]
```

可以直接运行对应模式脚本，也可以将 devtools EXE 拖到脚本上。`dist/` 中的其他文件
仍是忽略的构建产物；共享 BAT 桥接脚本和 PowerShell 实现属于内部实现，不应直接运行。

发布构建保持未加壳、自包含。不要使用 UPX 或同类加壳工具，以免增加诊断和杀毒软件分析成本。
