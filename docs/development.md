# Development / 开发

IdleTrigger uses Rust 1.88+ (the workspace `rust-version`), native Win32 controls, and GDI+ drawing. MSVC runtime code is linked statically; runtime imports are Windows system DLLs. Supported targets are x64 and x86 MSVC, with Windows 10 1607 / Server 2016 as the API baseline.

IdleTrigger 使用 Rust 1.88+（见 workspace `rust-version`）、原生 Win32 控件和 GDI+ 绘制。MSVC 运行库静态链接，运行时依赖 Windows 系统 DLL。支持 x64、x86 MSVC，API 基线为 Windows 10 1607 / Server 2016。

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
| `selfupdate.rs` | GitHub-Releases self-update (check, verify, replace, restart) / 基于 GitHub Releases 的自更新（检查、校验、替换与重启） |
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
pwsh -NoProfile -File build/build.ps1 -Architecture all
```

Use `pwsh -NoProfile -File build/build.ps1` for x64, `-Architecture x86` for x86, or `-Architecture all` for both. Run and deliver only `dist/IdleTrigger-x64.exe` and `dist/IdleTrigger-x86.exe`. The script works from any working directory and preserves runtime configuration. Exit the executable in `dist` before replacing it.

日常执行 `pwsh -NoProfile -File build/build.ps1` 构建 x64；用 `-Architecture x86` 构建 x86，用 `-Architecture all` 构建双架构。统一运行和交付 `dist/IdleTrigger-x64.exe`、`dist/IdleTrigger-x86.exe`，不要从缓存目录运行。脚本可从任意工作目录调用，不覆盖运行配置；替换前退出 `dist` 中正在运行的程序。

`.cargo/config.toml` pins the default target to x64, the Cargo cache to `target/`, and enables static CRT linking. Plain `cargo build --release` shares the explicit x64 build directory. Architecture, profile, `deps`, `build`, and `.fingerprint` subdirectories are Cargo-managed caches. Do not create tool-specific output directories or override `CARGO_TARGET_DIR` / `--target-dir`. Diagnostic builds stay in `target/`; the build script copies only normal Release executables into `dist/`.

`.cargo/config.toml` 固定默认 x64、缓存根目录 `target/` 和静态 CRT；不带 `--target` 的构建也会进入同一个 x64 缓存目录。架构、debug/release、deps、build、.fingerprint 等目录由 Cargo 管理，不是不同 AI 工具另建的产物目录。不要按工具名新建目录，也不要覆盖 `CARGO_TARGET_DIR` / `--target-dir`。诊断版保留在 `target/`；统一脚本只向 `dist/` 输出正常 Release。

`IDLETRIGGER_VERSION` sets the displayed/resource version; local builds default to the package version. Commit dependency changes with `Cargo.lock`.

`IDLETRIGGER_VERSION` 设置显示及资源版本，本地默认包版本。依赖变化需同步提交 `Cargo.lock`。

Release size is measured on the normal `dist` executables with the same compiler and features. The release profile uses `opt-level = "z"`, full LTO, one codegen unit, and stripped symbols. Keep unwinding: window and worker guards depend on `catch_unwind`. Build dependencies and unused Windows bindings do not imply equivalent code in the linked EXE; inspect a linker map before removing functionality or dependencies.

比较 Release 体积时，使用相同编译器和功能集构建的 `dist` 正式 EXE。Release 已采用 `opt-level = "z"`、完整 LTO、单代码生成单元和符号剥离。窗口与工作线程通过 `catch_unwind` 恢复异常，因此保留栈展开。构建依赖和未使用的 Windows 绑定不等于最终 EXE 中的代码体积；精简前应结合链接映射判断。

Keep the Windows binding crates' default features disabled; the application uses explicit API features and does not need their optional standard-library/async extensions. Full COM error information remains enabled. Runtime JSON uses the string reader consistently to avoid linking both string and byte-slice parser instances. Automation and context-menu rules share `toml_rules` for table preservation and field patches; their validation policies remain separate.

Windows 绑定关闭默认功能，仅启用需要的 API，不引入其可选标准库/异步扩展；COM 完整错误信息仍保留。运行时 JSON 统一使用字符串解析入口，避免链接字符串和字节切片两套解析器实例。自动任务与右键规则通过 `toml_rules` 共用表格式保留和字段补丁实现，各自的校验策略保持独立。

Locale JSON files remain the editable source. `crates/core/build.rs` validates matching string-map keys and generates sorted byte offsets plus UTF-8 text in Cargo's `OUT_DIR`. The EXE shares keys between languages and looks up text without parsing JSON or allocating a translation map. Generated files stay in `target`; `serde_json` is reused as a build dependency and remains needed at runtime for IPC, update metadata, and automation state.

双语 JSON 仍是文案维护入口。`crates/core/build.rs` 校验字符串映射及双语键名，向 Cargo 的 `OUT_DIR` 生成有序偏移表和 UTF-8 文本。EXE 中双语共用键名，查询无需解析 JSON 或分配翻译哈希表。生成文件保留在 `target`；构建阶段复用 `serde_json`，运行时的 IPC、更新元数据和自动任务状态仍需要该依赖。

CI follows stable Rust and uses `--locked` to prevent implicit dependency changes. It runs tests on both architectures with all features, checks normal release artifacts with `.github/scripts/verify-windows-artifact.ps1`, then builds the diagnostic variant. The artifact check covers PE architecture, GUI subsystem, version fields, manifest markers, imported DLLs, and exclusion of diagnostic switches. It does not replace testing on the oldest supported Windows version.

CI 使用 stable Rust，通过 `--locked` 阻止隐式依赖变更；双架构启用全部功能运行测试，用 `.github/scripts/verify-windows-artifact.ps1` 检查正式产物，再构建诊断版。产物检查覆盖 PE 架构、GUI 子系统、版本字段、manifest 标记、DLL 导入及诊断开关排除，不能替代最低支持 Windows 版本上的运行验证。

CI also checks both architectures with Rust 1.88.0, the declared minimum version. Resolver 3 prefers compatible dependencies during updates; `Cargo.lock` still pins actual versions. Stable CI and releases share dependency caches, written only by the master branch. CI disables incremental compilation to avoid caching disposable incremental state. Releases use the stable compiler available at build time, so byte-identical output across compiler updates is not guaranteed.

CI 另用声明的最低版本 Rust 1.88.0 检查双架构编译。Resolver 3 在更新依赖时优先选择兼容最低 Rust 版本的依赖，实际版本仍由 `Cargo.lock` 锁定。stable CI 和发布共用依赖缓存，仅 master 分支写入；CI 关闭增量编译以减少无效缓存。发布使用构建当时的 stable 编译器，跨编译器版本不保证产物字节完全一致。

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
| `IDLETRIGGER_DEVTOOLS_UPDATE_PREVIEW=1` | Self-update UI preview (9.9.9, no network) / 自更新界面预览（不联网） |

Other switches are defined in `devtools.rs`. Window-DC/BitBlt captures do not prove physical mouse interaction or desktop compositing. Native-message tests cover control states; real input, high contrast, text scaling, mixed-DPI monitors, resume, and actual system actions require a suitable Windows test environment.

其他开关见 `devtools.rs`。窗口 DC/BitBlt 捕获不能证明真实鼠标及桌面合成效果。原生消息测试覆盖控件状态；真实输入、高对比、文本缩放、混合 DPI、恢复及实际系统操作需在合适的 Windows 环境验证。

Cover both languages/themes for UI changes; preserve drafts across DPI/theme changes and reuse shared controls. Config changes must update the model, example, tests, locales, and tooltips. Keep generated output, local config, logs, and diagnostic captures out of Git.

UI 修改兼顾双语和深浅色，DPI/主题变化保留草稿并复用共享控件。配置变化同步模型、示例、测试、文案及提示。生成物、本机配置、日志和诊断截图不进 Git。
