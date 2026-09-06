// Package nativeform contains the small amount of Win32 theme plumbing shared
// by IdleTrigger's native form-style windows. It deliberately does not own
// layout or business controls; each surface keeps those responsibilities.
package nativeform

import (
	"strings"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/platform/windows/darkmode"
	"github.com/JeffioZ/idletrigger/internal/ui/colors"
)

const (
	dwmwaUseImmersiveDarkMode       = 20
	dwmwaUseImmersiveDarkModeLegacy = 19
	ttmSetTipBkColor                = 0x0413
	ttmSetTipTextColor              = 0x0414
)

var (
	uxtheme                = windows.NewLazySystemDLL("uxtheme.dll")
	dwmapi                 = windows.NewLazySystemDLL("dwmapi.dll")
	user32                 = windows.NewLazySystemDLL("user32.dll")
	pSetWindowTheme        = uxtheme.NewProc("SetWindowTheme")
	pDwmSetWindowAttribute = dwmapi.NewProc("DwmSetWindowAttribute")
	pSendMessage           = user32.NewProc("SendMessageW")
	pGetClassName          = user32.NewProc("GetClassNameW")
)

// ApplyFrame keeps captions and their system close button aligned with the
// main control panel. Older Windows versions ignore unsupported attributes and
// retain their native light/classic caption safely.
func ApplyFrame(hwnd windows.Handle, dark bool) {
	if hwnd == 0 {
		return
	}
	if dark {
		darkmode.AllowWindow(uintptr(hwnd))
	}
	value := uint32(0)
	if dark {
		value = 1
	}
	result, _, _ := pDwmSetWindowAttribute.Call(
		uintptr(hwnd),
		dwmwaUseImmersiveDarkMode,
		uintptr(unsafe.Pointer(&value)),
		unsafe.Sizeof(value),
	)
	if result != 0 {
		pDwmSetWindowAttribute.Call(
			uintptr(hwnd),
			dwmwaUseImmersiveDarkModeLegacy,
			uintptr(unsafe.Pointer(&value)),
			unsafe.Sizeof(value),
		)
	}
}

// ApplyControl asks Windows to render standard form controls with a coherent
// Explorer theme. Parent WM_CTLCOLOR handlers still supply IdleTrigger's exact
// semantic surface and text colors.
func ApplyControl(hwnd windows.Handle, dark bool) {
	if hwnd == 0 {
		return
	}
	name := "Explorer"
	className := controlClassName(hwnd)
	// EDIT backgrounds are painted by the parent through WM_CTLCOLOREDIT.
	// Disabling the visual-style renderer prevents Windows from painting a
	// second, mismatched disabled surface inside the owner-drawn field.
	if className == "EDIT" {
		name = ""
	} else if dark {
		name = darkControlTheme(className)
	}
	value, err := windows.UTF16PtrFromString(name)
	if err != nil {
		return
	}
	pSetWindowTheme.Call(uintptr(hwnd), uintptr(unsafe.Pointer(value)), 0)
}

func controlClassName(hwnd windows.Handle) string {
	buffer := make([]uint16, 64)
	length, _, _ := pGetClassName.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&buffer[0])), uintptr(len(buffer)))
	if length > 0 {
		return strings.ToUpper(windows.UTF16ToString(buffer[:length]))
	}
	return ""
}

func darkControlTheme(className string) string {
	if className == "COMBOBOX" {
		return "DarkMode_CFD"
	}
	return "DarkMode_Explorer"
}

// ApplyTooltip themes the tooltip and borrows its owner's font; it does not own the HFONT.
func ApplyTooltip(hwnd windows.Handle, dark bool, palette colors.Palette, font windows.Handle) {
	if hwnd == 0 {
		return
	}
	ApplyControl(hwnd, dark)
	if font != 0 {
		pSendMessage.Call(uintptr(hwnd), 0x0030, uintptr(font), 0) // WM_SETFONT; borrowed from owner.
	}
	pSendMessage.Call(uintptr(hwnd), ttmSetTipBkColor, uintptr(palette.TooltipBackground), 0)
	pSendMessage.Call(uintptr(hwnd), ttmSetTipTextColor, uintptr(palette.TooltipText), 0)
}
