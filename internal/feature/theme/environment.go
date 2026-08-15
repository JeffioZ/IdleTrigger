package theme

import (
	"errors"
	"fmt"
	"strings"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/platform/windows/powerstate"
)

type windowRect struct {
	left   int32
	top    int32
	right  int32
	bottom int32
}

type monitorInfo struct {
	size    uint32
	monitor windowRect
	work    windowRect
	flags   uint32
}

// ThemeSwitchPauseReason identifies an active presentation context in which
// changing the Windows theme would be disruptive.
type ThemeSwitchPauseReason string

const (
	ThemeSwitchPauseNone          ThemeSwitchPauseReason = ""
	ThemeSwitchPauseFullscreen    ThemeSwitchPauseReason = "fullscreen"
	ThemeSwitchPauseDirect3D      ThemeSwitchPauseReason = "direct3d-fullscreen"
	ThemeSwitchPausePresentation  ThemeSwitchPauseReason = "presentation"
	ThemeSwitchPauseForegroundGPU ThemeSwitchPauseReason = "foreground-gpu"
)

const (
	queryUserNotificationStateBusy                 = 2
	queryUserNotificationStateRunningD3DFullscreen = 3
	queryUserNotificationStatePresentationMode     = 4

	dwmWindowAttributeExtendedFrameBounds = 9
	dwmWindowAttributeCloaked             = 14
	monitorDefaultToNearest               = 2
)

var (
	errThemeEnvironmentCheckCanceled = errors.New("theme environment check canceled")

	themeEnvironmentUser32  = windows.NewLazySystemDLL("user32.dll")
	themeEnvironmentShell32 = windows.NewLazySystemDLL("shell32.dll")
	themeEnvironmentDWMAPI  = windows.NewLazySystemDLL("dwmapi.dll")

	pGetForegroundWindow      = themeEnvironmentUser32.NewProc("GetForegroundWindow")
	pGetShellWindow           = themeEnvironmentUser32.NewProc("GetShellWindow")
	pGetDesktopWindow         = themeEnvironmentUser32.NewProc("GetDesktopWindow")
	pIsWindowVisible          = themeEnvironmentUser32.NewProc("IsWindowVisible")
	pIsIconic                 = themeEnvironmentUser32.NewProc("IsIconic")
	pGetWindowRect            = themeEnvironmentUser32.NewProc("GetWindowRect")
	pGetClassName             = themeEnvironmentUser32.NewProc("GetClassNameW")
	pGetWindowThreadProcessID = themeEnvironmentUser32.NewProc("GetWindowThreadProcessId")
	pMonitorFromWindow        = themeEnvironmentUser32.NewProc("MonitorFromWindow")
	pGetMonitorInfo           = themeEnvironmentUser32.NewProc("GetMonitorInfoW")
	pGetDPIForWindow          = themeEnvironmentUser32.NewProc("GetDpiForWindow")
	pSHQueryNotificationState = themeEnvironmentShell32.NewProc("SHQueryUserNotificationState")
	pDWMGetWindowAttribute    = themeEnvironmentDWMAPI.NewProc("DwmGetWindowAttribute")
)

// DetectThemeSwitchPause checks Windows presentation state, foreground-window
// coverage, and sustained foreground 3D GPU activity. The GPU check is
// cancelable because it intentionally uses two short samples to avoid treating
// a transient desktop animation as a game.
func DetectThemeSwitchPause(cancel <-chan struct{}) (ThemeSwitchPauseReason, error) {
	if themeEnvironmentCheckCanceled(cancel) {
		return ThemeSwitchPauseNone, errThemeEnvironmentCheckCanceled
	}

	var firstErr error
	if reason, err := shellThemeSwitchPause(); err != nil {
		firstErr = err
	} else if reason != ThemeSwitchPauseNone {
		return reason, nil
	}

	hwnd, processID, fullscreen := foregroundWindowContext()
	if fullscreen {
		return ThemeSwitchPauseFullscreen, nil
	}
	if hwnd == 0 || processID == 0 {
		return ThemeSwitchPauseNone, firstErr
	}

	active, err := foregroundProcessGPUActive(processID, cancel)
	if errors.Is(err, errThemeEnvironmentCheckCanceled) {
		return ThemeSwitchPauseNone, err
	}
	if err != nil {
		if firstErr == nil {
			firstErr = err
		}
	} else if active {
		return ThemeSwitchPauseForegroundGPU, nil
	}
	return ThemeSwitchPauseNone, firstErr
}

func shellThemeSwitchPause() (ThemeSwitchPauseReason, error) {
	var state uint32
	result, _, _ := pSHQueryNotificationState.Call(uintptr(unsafe.Pointer(&state)))
	if result != 0 {
		return ThemeSwitchPauseNone, fmt.Errorf("query Windows presentation state: HRESULT 0x%08x", uint32(result))
	}
	return themeSwitchPauseReasonForNotificationState(state), nil
}

func themeSwitchPauseReasonForNotificationState(state uint32) ThemeSwitchPauseReason {
	switch state {
	case queryUserNotificationStateRunningD3DFullscreen:
		return ThemeSwitchPauseDirect3D
	case queryUserNotificationStatePresentationMode:
		return ThemeSwitchPausePresentation
	case queryUserNotificationStateBusy:
		return ThemeSwitchPauseFullscreen
	default:
		return ThemeSwitchPauseNone
	}
}

func foregroundWindowContext() (hwnd uintptr, processID uint32, fullscreen bool) {
	hwnd, _, _ = pGetForegroundWindow.Call()
	if hwnd == 0 {
		return 0, 0, false
	}
	shell, _, _ := pGetShellWindow.Call()
	desktop, _, _ := pGetDesktopWindow.Call()
	if hwnd == shell || hwnd == desktop {
		return 0, 0, false
	}
	if visible, _, _ := pIsWindowVisible.Call(hwnd); visible == 0 {
		return 0, 0, false
	}
	if minimized, _, _ := pIsIconic.Call(hwnd); minimized != 0 {
		return 0, 0, false
	}
	if foregroundWindowCloaked(hwnd) || excludedFullscreenWindowClass(windowClassName(hwnd)) {
		return 0, 0, false
	}

	pGetWindowThreadProcessID.Call(hwnd, uintptr(unsafe.Pointer(&processID)))
	return hwnd, processID, foregroundWindowCoversMonitor(hwnd)
}

func currentForegroundProcessID() uint32 {
	hwnd, _, _ := pGetForegroundWindow.Call()
	if hwnd == 0 {
		return 0
	}
	var processID uint32
	pGetWindowThreadProcessID.Call(hwnd, uintptr(unsafe.Pointer(&processID)))
	return processID
}

func foregroundWindowCloaked(hwnd uintptr) bool {
	var cloaked uint32
	result, _, _ := pDWMGetWindowAttribute.Call(
		hwnd,
		dwmWindowAttributeCloaked,
		uintptr(unsafe.Pointer(&cloaked)),
		unsafe.Sizeof(cloaked),
	)
	return result == 0 && cloaked != 0
}

func windowClassName(hwnd uintptr) string {
	var name [256]uint16
	length, _, _ := pGetClassName.Call(hwnd, uintptr(unsafe.Pointer(&name[0])), uintptr(len(name)))
	if length == 0 {
		return ""
	}
	return windows.UTF16ToString(name[:length])
}

func excludedFullscreenWindowClass(className string) bool {
	switch strings.ToLower(strings.TrimSpace(className)) {
	case "progman", "workerw", "shell_traywnd", "shell_secondarytraywnd":
		return true
	default:
		return false
	}
}

func foregroundWindowCoversMonitor(hwnd uintptr) bool {
	windowBounds, ok := visibleWindowBounds(hwnd)
	if !ok {
		return false
	}

	monitor, _, _ := pMonitorFromWindow.Call(hwnd, monitorDefaultToNearest)
	if monitor == 0 {
		return false
	}
	info := monitorInfo{size: uint32(unsafe.Sizeof(monitorInfo{}))}
	if ok, _, _ := pGetMonitorInfo.Call(monitor, uintptr(unsafe.Pointer(&info))); ok == 0 {
		return false
	}
	return windowCoversMonitor(windowBounds, info.monitor, fullscreenTolerance(hwnd))
}

func visibleWindowBounds(hwnd uintptr) (windowRect, bool) {
	var bounds windowRect
	result, _, _ := pDWMGetWindowAttribute.Call(
		hwnd,
		dwmWindowAttributeExtendedFrameBounds,
		uintptr(unsafe.Pointer(&bounds)),
		unsafe.Sizeof(bounds),
	)
	if result != 0 {
		if ok, _, _ := pGetWindowRect.Call(hwnd, uintptr(unsafe.Pointer(&bounds))); ok == 0 {
			return windowRect{}, false
		}
	}
	return bounds, bounds.right > bounds.left && bounds.bottom > bounds.top
}

func fullscreenTolerance(hwnd uintptr) int32 {
	const baseTolerance = 2
	if err := pGetDPIForWindow.Find(); err != nil {
		return baseTolerance
	}
	dpi, _, _ := pGetDPIForWindow.Call(hwnd)
	if dpi <= 96 {
		return baseTolerance
	}
	tolerance := int32((dpi*baseTolerance + 48) / 96)
	if tolerance < baseTolerance {
		return baseTolerance
	}
	return tolerance
}

func windowCoversMonitor(window, monitor windowRect, tolerance int32) bool {
	if tolerance < 0 || window.right <= window.left || window.bottom <= window.top ||
		monitor.right <= monitor.left || monitor.bottom <= monitor.top {
		return false
	}
	return window.left <= monitor.left+tolerance &&
		window.top <= monitor.top+tolerance &&
		window.right >= monitor.right-tolerance &&
		window.bottom >= monitor.bottom-tolerance
}

func themeEnvironmentCheckCanceled(cancel <-chan struct{}) bool {
	if cancel == nil {
		return false
	}
	select {
	case <-cancel:
		return true
	default:
		return false
	}
}

func onBattery() bool {
	return powerstate.OnBattery()
}
