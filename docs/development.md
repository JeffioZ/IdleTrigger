# Development / 开发

IdleTrigger uses Rust, native Win32 controls, and GDI+ drawing. MSVC runtime code is linked statically; runtime imports are Windows system DLLs. Supported targets are x64 and x86 MSVC, with Windows 10 1607 / Server 2016 as the API baseline.

IdleTrigger 使用 Rust、原生 Win32 控件和 GDI+ 绘制。MSVC 运行库静态链接，运行时依赖 Windows 系统 DLL。支持 x64、x86 MSVC，API 基线为 Windows 10 1607 / Server 2016。

## Layout / 目录

| Path | Responsibility / 职责 |
| --- | --- |
| `apps/idletrigger/src/main.rs` | Event loop, panel, tray, idle warning, and power state / 事件循环、面板、托盘、空闲预警与电源状态 |
| `runtime.rs` | Runtime state: config document, serialized writes, log file / 运行态：配置文档、串行写入与日志 |
| `automation.rs`, `idle_monitor.rs` | Rule evaluation and idle clock / 规则计算与空闲时钟 |
| `automation_ui.rs`, `settings_ui.rs`, `popups.rs` | Forms and warnings / 表单与提醒 |
| `paint.rs`, `nativeform.rs`, `choice.rs`, `list_style.rs`, `dpi.rs`, `viewport.rs`, `accessibility.rs`, `tooltips.rs` | Shared native controls and accessibility / 共享原生控件及辅助功能 |
| `theme*.rs`, `iplocate.rs`, `gpu_activity.rs`, `display.rs` | Themes and platform detection / 主题及平台检测 |
| `system.rs`, `single_instance.rs`, `ipc.rs`, `pipe.rs` | System actions, instance ownership, CLI/IPC / 系统操作、单实例及通信 |
| `crates/core/` | Config, rules, TOML document edits, locales / 配置、规则、TOML 编辑与文案 |
| `build/windows/` | Manifest and icons / 清单及图标 |
| `.github/workflows/` | Verification and draft releases / 验证及发布草稿 |

Config writes are serialized and published only after successful saving. Drafts retain an opening baseline for conflict detection. Workers post messages to the UI thread rather than directly changing controls. Runtime task state and occurrence bookkeeping are separate from user settings.

配置写入串行执行，保存成功后发布；草稿保留打开时基线以检测冲突。后台线程通过消息通知 UI，不直接修改控件。任务运行态、触发记录和用户设置分别维护。

## Build / 构建

Install Rust MSVC, Visual Studio C++ Build Tools, and Windows SDK. From the repository root / 安装对应工具后在仓库根目录执行：

```powershell
$env:RUSTUP_TOOLCHAIN = 'stable'
rustup toolchain install stable --profile minimal --component clippy,rustfmt --target x86_64-pc-windows-msvc,i686-pc-windows-msvc
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features --target x86_64-pc-windows-msvc
cargo test --locked --workspace --all-features --target i686-pc-windows-msvc
cargo build --locked --release --target x86_64-pc-windows-msvc
cargo build --locked --release --target i686-pc-windows-msvc
```

Outputs: `target/<target>/release/IdleTrigger.exe`. `.cargo/config.toml` enables static CRT linking. `IDLETRIGGER_VERSION` sets the displayed/resource version; local builds default to the package version. Commit dependency changes with `Cargo.lock`.

产物为上述路径，`.cargo/config.toml` 启用静态 CRT。`IDLETRIGGER_VERSION` 设置显示及资源版本，本地默认包版本。依赖变化需同步提交 `Cargo.lock`。

CI follows stable Rust and uses `--locked` to prevent implicit dependency changes. It runs tests on both architectures with all features, checks normal release artifacts with `.github/scripts/verify-windows-artifact.ps1`, then builds the diagnostic variant. The artifact check covers PE architecture, GUI subsystem, version fields, manifest markers, imported DLLs, and exclusion of diagnostic switches. It does not replace testing on the oldest supported Windows version.

CI 使用 stable Rust，通过 `--locked` 阻止隐式依赖变更；双架构启用全部功能运行测试，用 `.github/scripts/verify-windows-artifact.ps1` 检查正式产物，再构建诊断版。产物检查覆盖 PE 架构、GUI 子系统、版本字段、manifest 标记、DLL 导入及诊断开关排除，不能替代最低支持 Windows 版本上的运行验证。

## Diagnostics / 诊断

```powershell
cargo build --locked --release --features devtools --target x86_64-pc-windows-msvc
```

Only devtools builds recognize these switches, all requiring `IDLETRIGGER_DEVTOOLS=1`. Use an isolated writable folder and separate configuration with automatic features disabled. Preview modes must not execute real system actions.

仅 devtools 构建识别以下开关，均要求 `IDLETRIGGER_DEVTOOLS=1`。使用独立可写目录及配置，关闭自动功能。预览不得执行真实系统操作。

| Variable | Purpose / 用途 |
| --- | --- |
| `IDLETRIGGER_DEVTOOLS_LOG=1` | Logging / 日志 |
| `IDLETRIGGER_DEVTOOLS_INPUT_TRACE=1` | Input-state changes / 输入变化 |
| `IDLETRIGGER_DEVTOOLS_THEME=dark\|light` | App-only theme override / 仅覆盖应用主题 |
| `IDLETRIGGER_DEVTOOLS_CAPTURE_PANEL=1` | Capture client areas and exit / 捕获客户区序列后退出 |
| `IDLETRIGGER_DEVTOOLS_CAPTURE_STEP_MS=200..10000` | Capture interval / 捕获步进间隔 |
| `IDLETRIGGER_DEVTOOLS_WARNING_PREVIEW=1` | Idle-warning preview / 空闲提醒预览 |
| `IDLETRIGGER_DEVTOOLS_IDLE_MONITOR_SECONDS=10..600` | Temporary test timeout / 临时测试时长 |

Other switches are defined in `devtools.rs`. Window-DC/BitBlt captures do not prove physical mouse interaction or desktop compositing. Native-message tests cover control states; real input, high contrast, text scaling, mixed-DPI monitors, resume, and actual system actions require a suitable Windows test environment.

其他开关见 `devtools.rs`。窗口 DC/BitBlt 捕获不能证明真实鼠标及桌面合成效果。原生消息测试覆盖控件状态；真实输入、高对比、文本缩放、混合 DPI、恢复及实际系统操作需在合适的 Windows 环境验证。

Cover both languages/themes for UI changes; preserve drafts across DPI/theme changes and reuse shared controls. Config changes must update the model, example, tests, locales, and tooltips. Keep generated output, local config, logs, and diagnostic captures out of Git.

UI 修改兼顾双语和深浅色，DPI/主题变化保留草稿并复用共享控件。配置变化同步模型、示例、测试、文案及提示。生成物、本机配置、日志和诊断截图不进 Git。
