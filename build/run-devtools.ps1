# Devtools 诊断启动器：运行 target 中的诊断 EXE（不进 dist，符合仓库约定）。
# 用法示例：
#   pwsh -NoProfile -File build\run-devtools.ps1 -Preview -Log          # 更新预览 + 强制日志
#   pwsh -NoProfile -File build\run-devtools.ps1 -Preview -Log -AppData D:\scratch\ck   # 数据目录演练
#   pwsh -NoProfile -File build\run-devtools.ps1 -Capture -StepMs 800   # 截图序列
param(
    # 自更新 UI 预览：注入 9.9.9 模拟版本与发布说明（不联网）
    [switch]$Preview,
    # 强制写日志
    [switch]$Log,
    # 数据目录覆盖（AppData 命名方案 + 一次性迁移演练；不传则保持 EXE 旁旧模式）
    [string]$AppData = '',
    # 截图序列（面板/设置五页/任务管理器等 → EXE 旁 BMP，完成后退出）
    [switch]$Capture,
    # 截图步进间隔毫秒（200..10000）
    [int]$StepMs = 0,
    # 在第 N 步截图后冻结序列（供手工交互）
    [int]$StopAfter = 0,
    # 额外变量，形如 -Extra IDLETRIGGER_DEVTOOLS_THEME=dark,WARNING_PREVIEW=1
    [string[]]$Extra = @()
)

$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo 'target\x86_64-pc-windows-msvc\release\IdleTrigger.exe'
if (-not (Test-Path $exe)) {
    Write-Host "诊断 EXE 不存在：$exe" -ForegroundColor Red
    Write-Host '先构建：cargo build --locked --release --features devtools --target x86_64-pc-windows-msvc'
    exit 1
}

$env:IDLETRIGGER_DEVTOOLS = '1'
if ($Log) { $env:IDLETRIGGER_DEVTOOLS_LOG = '1' }
if ($Preview) { $env:IDLETRIGGER_DEVTOOLS_UPDATE_PREVIEW = '1' }
if ($AppData) { $env:IDLETRIGGER_DEVTOOLS_APPDATA = $AppData }
if ($Capture) { $env:IDLETRIGGER_DEVTOOLS_CAPTURE_PANEL = '1' }
if ($StepMs -gt 0) { $env:IDLETRIGGER_DEVTOOLS_CAPTURE_STEP_MS = "$StepMs" }
if ($StopAfter -gt 0) { $env:IDLETRIGGER_DEVTOOLS_CAPTURE_STOP_AFTER = "$StopAfter" }
foreach ($pair in $Extra) {
    $kv = $pair -split '=', 2
    if ($kv.Length -eq 2) { Set-Item -Path "Env:$($kv[0])" -Value $kv[1] }
}

$dataNote = if ($AppData) { "数据目录：$AppData" } else { '数据：EXE 旁旧模式（target\...\release\IdleTrigger.toml）' }
Write-Host "启动诊断实例：$exe" -ForegroundColor Cyan
Write-Host "  $dataNote"
if ($Log -and -not $AppData) {
    Write-Host '  日志：target\x86_64-pc-windows-msvc\release\IdleTrigger.log'
}

& $exe
