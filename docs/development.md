# 🛠️ Build IdleTrigger

[Documentation](README.md) · [简体中文](development.zh-CN.md) · [Project Home](../README.md)

## 📦 Requirements

- Go 1.26 or later
- Git
- PowerShell 7 or later for repository scripts
- `github.com/akavel/rsrc` only when regenerating Windows resources

IdleTrigger targets Windows 10 / Windows Server 2016 and later. The repository produces both `windows/amd64` and `windows/386` binaries.

Windows 7 is not supported by the main build. Go 1.20 was the last compatible Go release. Any future legacy build should use a separate Go 1.20 branch, compatible dependencies, and real-device testing.

```powershell
go version
go mod download
```

## 🔨 Build

The architecture-specific `.syso` files contain the icon, manifest, and Windows version metadata. They are generated files and are not committed. Regenerate them before building so Explorer and the app show the same version.

```powershell
$env:CGO_ENABLED = "0"
$env:GOARCH = "amd64" # "386" for 32-bit Windows
$version = "dev"
go run ./tools/resourcegen.go -version $version
$ldflags = "-s -w -H windowsgui -X github.com/JeffioZ/idletrigger/internal/version.Value=$version"
$output = if ($env:GOARCH -eq "amd64") { "IdleTrigger-x64.exe" } else { "IdleTrigger-x86.exe" }
go build -trimpath "-ldflags=$ldflags" -o $output ./cmd/idletrigger
```

`CGO_ENABLED=0` keeps the binary self-contained. `-H windowsgui` prevents a console window when launching the tray app.

## ✅ Verify

Run the standard local checks in PowerShell 7 or later:

```powershell
.\tools\check.ps1
```

The default command is the quick development check: module verification,
formatting, working-tree whitespace, the short test suite, normal vet, and
dependency boundaries. Native Win32 integration and resource-cycle tests stay
in the full suite. Before a release or broad change, run the full build-tag
matrix and `golangci-lint` (when installed):

```powershell
.\tools\check.ps1 -Full
```

Run the optional vulnerability scan while online; it can be combined with
`-Full`:

```powershell
.\tools\check.ps1 -Vulncheck
```

Without `-Vulncheck`, the vulnerability scan is not run and the script reports
that explicitly. If the system caches are not writable, set optional user-local
cache paths for the current shell before running the script:

```powershell
$env:GOCACHE = Join-Path $env:LOCALAPPDATA "IdleTrigger\cache\go-build"
$env:GOLANGCI_LINT_CACHE = Join-Path $env:LOCALAPPDATA "IdleTrigger\cache\golangci-lint"
```

Build both targets explicitly:

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

The release workflow runs formatting, module, test, and vet checks. It then builds both executables and publishes `SHA256SUMS.txt`.

## 🚢 Release Process

Pushing a strict SemVer tag in the form `vMAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]` creates a draft named after that tag; other `v*` tags are rejected. The draft includes both executables, `SHA256SUMS.txt`, and an initial changelog. `.github/release.yml` groups pull requests; direct commits still need manual review.

Before publishing, apply the [release-notes style and template](release-notes.md). Lead with user-visible results. Keep both languages aligned, check the assets and comparison range, and leave commit details at the end.

## 🧩 Cross-Layer Change Checklist

Use the relevant row before considering a change complete:

- **Configuration field:** update `Config`, defaults, normalization, validation,
  annotated TOML output, `IdleTrigger.example.toml`, related UI text, and tests.
- **Control-panel action:** update the control ID/choice path, `controlpanel.Action`
  mapping, the single `internal/app` action entry point, persistence, runtime
  reconciliation, icon/schedule refreshes, and mapping tests.
- **User-facing text:** update English and Simplified Chinese locales together;
  keep format verbs identical and check tooltip, CLI, and screenshot consumers.
- **Windows integration:** keep business policy outside `internal/platform/windows`,
  verify error and cleanup paths, then compile both 386 and amd64 targets.
- **Theme presentation guard:** preserve `theme_skip_fullscreen` as the persisted
  compatibility key. Check Windows notification state and visible DWM bounds
  before sampling GPU activity; GPU sampling must target only the foreground
  process's rendering engines, require consecutive samples, run only while a
  switch is pending, remain cancelable, and fail open. Do not request elevation,
  open the foreground process, or add a global GPU monitor.
- **Developer-only capability:** keep it behind `devtools`, update dependency
  boundary expectations, and verify the normal build excludes it.
- **Automatic task:** update `internal/automation`, scheduler behavior and tests,
  TOML output, both locales and tooltips, the manager/picker UI, and app runtime
  reconciliation. State actions are temporary runtime requests and must not
  rewrite manual toggles; pause wins over enable for the same feature. Event
  actions must retain a cancellable countdown. Process-start events establish a
  baseline first and fire only when all selected targets change from absent to
  at least one running target. Process-exit events continue to wait for every
  matching instance plus the 5-second grace period, preventing startup and
  brief-restart false positives.
- **Native form window:** automatic-task, task-editor, process-picker, and action-
  countdown windows should share `internal/ui/nativeform` caption, theme-aware
  icons, control theming, rounded field surfaces, and hover/press/focus/disabled
  states. Owner-drawn choices and checkboxes should follow the control panel's
  visual language instead of falling back to unthemed native dropdowns.
  Long choice popups and report lists should share the same menu spacing,
  corner radius, and themed scrollbar. Complete owner-drawn backgrounds,
  borders, and text in an off-screen buffer and commit once so hover and view
  transitions never expose intermediate paint passes. Empty edit cues must use
  theme-aware muted text instead of the fixed native cue color, which can become
  unreadable in dark mode.
  Convert the intended client size through `AdjustWindowRectEx` before
  `CreateWindowEx`; handle background erase, full invalidation after theme changes,
  DPI changes, and owner enable/disable while preserving nested modality. Forms
  need visible labels, progressive disclosure, inline validation with first-error
  focus, and confirmation before discarding an edited task.
- **Process picker:** use a native report list with checkboxes as the multi-choice
  mechanism and a single focused row. The Process, Description, and Instances columns
  must fit without a horizontal scrollbar. The running list contains one row per
  executable name; exact-file targets come only from Browse and stay in the
  selection preview. Search, staged asynchronous loading, empty results, refresh
  disabling, header sorting, and selection preview must remain recoverable.
  Commit the first repaint after async rows are inserted; sortable headers need
  hover/press states, and the themed vertical scrollbar must not reintroduce a
  horizontal scrollbar. Choice popups, the task list, the process list, and the
  current-selection preview should reuse the shared scrollbar. A stale snapshot
  may refresh when the picker becomes active again, but filtering, sorting,
  checks, focus, and the visible anchor must be preserved without re-reading
  descriptions that are already known.
  Clearly distinguish name and exact-file matching in the preview; never persist
  a PID or use a description as identity.
- **Process metadata:** use Toolhelp names first. Keep one representative
  description per executable name, trying same-name instances only until one
  yields a description, with a bounded worker count. Request
  `PROCESS_QUERY_LIMITED_INFORMATION` only when a path is required. Browsed EXEs
  may be validated and read for description but
  must never be launched. Do not add debug privilege, process-memory access,
  injection, process termination,
  arbitrary launch, a service, or Task Scheduler integration.

The automated checks enforce locale-key coverage, configuration/example parity,
control-panel action paths, build-tag dependency boundaries, and direct package
layering. The process-automation boundary also rejects debug-privilege,
process-memory, injection, and forced-termination APIs. These checks complement,
rather than replace, behavior-specific tests.

## 🎨 Regenerate Resources

The application icon and the two taskbar icons have separate artwork. Regenerate
the application ICO first, then the purpose-built taskbar variants:

```powershell
go run ./tools/appicon/main.go build/windows/icons
go run ./tools/trayicons/main.go build/windows/icons
```

Then regenerate both architecture resources. Use the identical version value in the resource command and release build:

```powershell
$version = Read-Host "Release version without the leading v (for example, 1.9.0)"
go run ./tools/resourcegen.go -version $version
```

Commit `app.ico`, both tray ICO files, `build/windows/manifest.xml`, and the generators together. Do not commit `.syso` files. The release workflow regenerates them from the tag.

## 📸 Regenerate README Screenshots

Screenshot generation is a maintenance-only capability and is compiled only
with the `devtools` build tag. The helper builds a temporary devtools EXE,
regenerates all four checked-in images, validates their PNG dimensions, and
removes the temporary build directory:

```powershell
.\tools\capture-screenshots.ps1
```

To validate the process without replacing the checked-in images, provide a
temporary output directory:

```powershell
.\tools\capture-screenshots.ps1 -OutputDirectory (Join-Path $env:TEMP "IdleTrigger-screenshots")
```

For visual review of every native surface, generate the control panel,
automatic-task manager, task editor, and process picker in both languages and
themes. This writes 16 ignored images to `dist/ui-review/` by default and does
not replace the four README images:

```powershell
.\tools\capture-screenshots.ps1 -CaptureSet Review
```

The underlying devtools command names the two sets explicitly as
`screenshot --readme-set` and `screenshot --review-set`; the ambiguous legacy
`--all` option is rejected.

The normal release EXE deliberately does not contain the `screenshot` command
or its PNG/compression dependencies.

## 📴 Offline Build

Vendor dependencies while online, then copy the repository including `vendor/` to the offline machine:

```powershell
go mod vendor

$env:CGO_ENABLED = "0"
$env:GOARCH = "amd64"
go run ./tools/resourcegen.go -version dev
go build -mod=vendor -trimpath -ldflags="-s -w -H windowsgui" -o dist/IdleTrigger-x64.exe ./cmd/idletrigger
```

## 🔁 Development Loop

```powershell
go test ./...
go run ./tools/resourcegen.go -version dev
$env:CGO_ENABLED = "0"
$env:GOARCH = "amd64"
go build -tags devtools -trimpath -ldflags="-s -w -H windowsgui -X github.com/JeffioZ/idletrigger/internal/version.Value=devtools" -o dist/IdleTrigger-x64-devtools.exe ./cmd/idletrigger
.\dist\IdleTrigger-x64-devtools.exe

# In a second terminal after the tray app starts:
cmd /c .\dist\IdleTrigger-x64-devtools.exe nosleep on
cmd /c .\dist\IdleTrigger-x64-devtools.exe nosleep status
cmd /c .\dist\IdleTrigger-x64-devtools.exe monitor on
cmd /c .\dist\IdleTrigger-x64-devtools.exe diagnostics idle
```

The dev build uses the Windows GUI subsystem so tray startup does not flash a
console window. For CLI output checks, run commands through `cmd /c` or redirect
stdout/stderr with `Start-Process`; direct PowerShell invocation of GUI-subsystem
EXEs can return before output is attached.
Maintenance capabilities such as `diagnostics`, screenshots, and local test
environment variables exist only in devtools builds.

The four tracked, user-facing launch wrappers for representative manual checks
live beside local builds in `dist/`, so a devtools EXE can be dragged directly
onto them. Their shared implementation remains in `tools/devtools/` to avoid
duplicated logic. The wrappers require PowerShell 7, default to
`dist/IdleTrigger-x64-devtools.exe`, reject release builds and existing
IdleTrigger instances, and keep all developer variables scoped to the child
process:

```text
Start-IdleTrigger-Devtools-Idle-Monitor-Test.bat [devtools.exe] [10..600]
Start-IdleTrigger-Devtools-Input-Trace.bat [devtools.exe]
Start-IdleTrigger-Devtools-UI-Capture.bat [devtools.exe]
Start-IdleTrigger-Devtools-Warning-Preview.bat [devtools.exe]
```

Run or drag a devtools EXE onto one of the mode-specific wrappers. Other files
under `dist/` remain ignored build outputs; the four wrappers are explicit
tracked exceptions. The shared BAT bridge and PowerShell engine are
implementation details and should not be invoked directly.

Release builds stay unpacked and self-contained. Do not use UPX or similar
packers: they complicate diagnostics and can increase antivirus scrutiny.
