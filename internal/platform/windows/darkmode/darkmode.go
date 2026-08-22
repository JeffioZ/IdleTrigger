// Package darkmode enables dark-mode rendering for Win32 popup menus
// and controls on Windows 10 1809+.  The APIs are ordinal-only exports
// from uxtheme.dll so we load them by ordinal via GetProcAddress.
package darkmode

import (
	"sync"
	"syscall"

	"golang.org/x/sys/windows"
)

const (
	windows10Build1809 = 17763
	windows10Build1903 = 18362

	preferredAppModeAllowDark  = 1
	preferredAppModeForceDark  = 2
	preferredAppModeForceLight = 3
)

type processThemePreference uint8

const (
	processThemeFollowSystem processThemePreference = iota
	processThemeForceLight
	processThemeForceDark
)

// Enable lets immersive Win32 surfaces follow the active Windows app theme.
// Harmless no-op on Windows versions that lack these APIs.
func Enable() {
	applyProcessTheme(processThemeFollowSystem, 0)
}

// SetAppTheme gives deterministic captures and other explicit-theme surfaces
// a process preference which does not depend on a stale immersive-policy cache.
func SetAppTheme(dark bool) {
	applyProcessTheme(forcedThemePreference(dark), 0)
}

// PreparePopupMenu synchronizes every process- and owner-level setting used by
// Windows to render a native popup menu. It must run immediately before
// TrackPopupMenu so a long-lived tray process cannot retain the previous theme.
func PreparePopupMenu(hwnd uintptr, dark bool) {
	applyProcessTheme(forcedThemePreference(dark), hwnd)
}

// AllowWindow opts a Win32 owner window into immersive dark mode where the
// current Windows version supports it. It is harmless when the API is absent.
func AllowWindow(hwnd uintptr) {
	if hwnd == 0 || windows.RtlGetVersion().BuildNumber < windows10Build1809 {
		return
	}
	withUxtheme(func(uxtheme windows.Handle) {
		setWindowDarkAllowed(uxtheme, hwnd, true)
	})
}

func forcedThemePreference(dark bool) processThemePreference {
	if dark {
		return processThemeForceDark
	}
	return processThemeForceLight
}

func applyProcessTheme(preference processThemePreference, hwnd uintptr) {
	build := windows.RtlGetVersion().BuildNumber
	argument, supported := preferredAppModeArgument(build, preference)
	if !supported {
		return
	}
	withUxtheme(func(uxtheme windows.Handle) {
		refreshImmersiveColorPolicyState(uxtheme)
		setPreferredAppMode(uxtheme, argument)
		if hwnd != 0 {
			setWindowDarkAllowed(uxtheme, hwnd, preference == processThemeForceDark)
		}
		flushMenuThemes(uxtheme)
	})
}

// preferredAppModeArgument accounts for ordinal 135 changing signature in
// Windows 10 1903. Build 1809 exposes AllowDarkModeForApp(BOOL), whereas 1903+
// exposes SetPreferredAppMode(PreferredAppMode).
func preferredAppModeArgument(build uint32, preference processThemePreference) (uintptr, bool) {
	switch preference {
	case processThemeFollowSystem, processThemeForceLight, processThemeForceDark:
	default:
		return 0, false
	}
	if build < windows10Build1809 {
		return 0, false
	}
	if build < windows10Build1903 {
		if preference == processThemeForceLight {
			return 0, true
		}
		return 1, true
	}
	switch preference {
	case processThemeFollowSystem:
		return preferredAppModeAllowDark, true
	case processThemeForceLight:
		return preferredAppModeForceLight, true
	case processThemeForceDark:
		return preferredAppModeForceDark, true
	}
	return 0, false
}

func setPreferredAppMode(uxtheme windows.Handle, argument uintptr) {
	// SetPreferredAppMode on Windows 10 1903+, AllowDarkModeForApp on 1809.
	proc, _ := windows.GetProcAddressByOrdinal(uxtheme, 135)
	if proc != 0 {
		syscall.SyscallN(proc, argument)
	}
}

func setWindowDarkAllowed(uxtheme windows.Handle, hwnd uintptr, allowed bool) {
	// AllowDarkModeForWindow — ordinal 133.
	proc, _ := windows.GetProcAddressByOrdinal(uxtheme, 133)
	if proc == 0 {
		return
	}
	value := uintptr(0)
	if allowed {
		value = 1
	}
	syscall.SyscallN(proc, hwnd, value)
}

func refreshImmersiveColorPolicyState(uxtheme windows.Handle) {
	// RefreshImmersiveColorPolicyState — ordinal 104.
	proc, _ := windows.GetProcAddressByOrdinal(uxtheme, 104)
	if proc != 0 {
		syscall.SyscallN(proc)
	}
}

// AppsUseDark reports the effective Windows app theme through the same
// immersive-theme API used by native controls. The second result is false on
// Windows versions that do not expose ShouldAppsUseDarkMode.
func AppsUseDark() (dark, supported bool) {
	if windows.RtlGetVersion().BuildNumber < windows10Build1809 {
		return false, false
	}
	withUxtheme(func(uxtheme windows.Handle) {
		refreshImmersiveColorPolicyState(uxtheme)
		// ShouldAppsUseDarkMode — ordinal 132.
		proc, _ := windows.GetProcAddressByOrdinal(uxtheme, 132)
		if proc == 0 {
			return
		}
		result, _, _ := syscall.SyscallN(proc)
		dark = result != 0
		supported = true
	})
	return dark, supported
}

func withUxtheme(fn func(uxtheme windows.Handle)) {
	uxtheme := loadUxtheme()
	if uxtheme == 0 {
		return
	}
	fn(uxtheme)
}

var (
	uxthemeOnce   sync.Once
	uxthemeHandle windows.Handle
)

// loadUxtheme loads uxtheme.dll exactly once for the process lifetime. The
// module is loaded with LOAD_LIBRARY_SEARCH_SYSTEM32 so version spoofing cannot
// redirect the ordinal-only entry points, and it is intentionally never freed:
// popup menus recur frequently and the OS reclaims the handle on process exit.
func loadUxtheme() windows.Handle {
	uxthemeOnce.Do(func() {
		handle, err := windows.LoadLibraryEx("uxtheme.dll", 0, windows.LOAD_LIBRARY_SEARCH_SYSTEM32)
		if err == nil {
			uxthemeHandle = handle
		}
	})
	return uxthemeHandle
}

func flushMenuThemes(uxtheme windows.Handle) {
	// FlushMenuThemes — ordinal 136.
	proc2, _ := windows.GetProcAddressByOrdinal(uxtheme, 136)
	if proc2 != 0 {
		syscall.SyscallN(proc2)
	}
}
