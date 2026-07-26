// Package darkmode enables dark-mode rendering for Win32 popup menus
// and controls on Windows 10 1809+.  The APIs are ordinal-only exports
// from uxtheme.dll so we load them by ordinal via GetProcAddress.
package darkmode

import (
	"syscall"

	"golang.org/x/sys/windows"
)

// Enable forces the process to use the dark (immersive) theme.
// Harmless no-op on Windows versions that lack these APIs.
func Enable() {
	SetPreferredAppMode(false)
}

// SetPreferredAppMode updates the process-level Win32 theme preference.
// forceDark is used for native popup menus, which do not always follow
// AllowDark after the system theme changes while the process stays running.
func SetPreferredAppMode(forceDark bool) {
	withUxtheme(func(uxtheme windows.Handle) {
		// SetPreferredAppMode — ordinal 135. 1 = AllowDark, 2 = ForceDark.
		proc, _ := windows.GetProcAddressByOrdinal(uxtheme, 135)
		if proc != 0 {
			mode := uintptr(1)
			if forceDark {
				mode = 2
			}
			syscall.SyscallN(proc, mode)
		}
		flushMenuThemes(uxtheme)
	})
}

// AllowWindow opts a Win32 owner window into immersive dark mode where the
// current Windows version supports it. It is harmless when the API is absent.
func AllowWindow(hwnd uintptr) {
	if hwnd == 0 {
		return
	}
	withUxtheme(func(uxtheme windows.Handle) {
		// AllowDarkModeForWindow — ordinal 133.
		proc, _ := windows.GetProcAddressByOrdinal(uxtheme, 133)
		if proc != 0 {
			syscall.SyscallN(proc, hwnd, 1)
		}
	})
}

// RefreshMenuThemes asks Windows to rebuild cached popup-menu theme resources.
// This is useful after the OS theme changes while the tray app stays running.
func RefreshMenuThemes() {
	withUxtheme(func(uxtheme windows.Handle) {
		flushMenuThemes(uxtheme)
	})
}

// AppsUseDark reports the effective Windows app theme through the same
// immersive-theme API used by native controls. The second result is false on
// Windows versions that do not expose ShouldAppsUseDarkMode.
func AppsUseDark() (dark, supported bool) {
	withUxtheme(func(uxtheme windows.Handle) {
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
	uxtheme, err := windows.LoadLibraryEx("uxtheme.dll", 0, windows.LOAD_LIBRARY_SEARCH_SYSTEM32)
	if err != nil {
		return
	}
	defer func() { _ = windows.FreeLibrary(uxtheme) }()
	fn(uxtheme)
}

func flushMenuThemes(uxtheme windows.Handle) {
	// FlushMenuThemes — ordinal 136.
	proc2, _ := windows.GetProcAddressByOrdinal(uxtheme, 136)
	if proc2 != 0 {
		syscall.SyscallN(proc2)
	}
}
