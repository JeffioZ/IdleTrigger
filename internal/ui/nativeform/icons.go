package nativeform

import (
	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/platform/windows/resourceid"
)

const (
	wmSetIcon = 0x0080
	iconSmall = 0
	iconBig   = 1
	imageIcon = 1
)

var (
	iconUser32          = windows.NewLazySystemDLL("user32.dll")
	iconKernel32        = windows.NewLazySystemDLL("kernel32.dll")
	iconLoadImage       = iconUser32.NewProc("LoadImageW")
	iconDestroy         = iconUser32.NewProc("DestroyIcon")
	iconSendMessage     = iconUser32.NewProc("SendMessageW")
	iconGetModuleHandle = iconKernel32.NewProc("GetModuleHandleW")
)

// WindowIcons owns the per-window title-bar icons used by form-style windows.
// A dark mark is used on a light caption and a light mark on a dark caption,
// matching the main control panel. Handles loaded by LoadImageW are released
// when replaced or when the window is destroyed.
type WindowIcons struct {
	large, small windows.Handle
	hwnd         windows.Handle
	dark         bool
	initialized  bool
}

// Apply refreshes the title-bar icon when the theme or DPI changes.
func (i *WindowIcons) Apply(hwnd windows.Handle, dark bool, largeSize, smallSize int, force bool) {
	if hwnd == 0 || (i.initialized && !force && i.hwnd == hwnd && i.dark == dark) {
		return
	}
	module, _, _ := iconGetModuleHandle.Call(0)
	if module == 0 {
		return
	}
	resource := uintptr(resourceid.TrayDarkIconID)
	if dark {
		resource = uintptr(resourceid.TrayLightIconID)
	}
	large, _, _ := iconLoadImage.Call(module, resource, imageIcon, uintptr(largeSize), uintptr(largeSize), 0)
	small, _, _ := iconLoadImage.Call(module, resource, imageIcon, uintptr(smallSize), uintptr(smallSize), 0)
	if large == 0 || small == 0 {
		if large != 0 {
			iconDestroy.Call(large)
		}
		if small != 0 {
			iconDestroy.Call(small)
		}
		return
	}
	oldWindow := i.hwnd
	oldLarge, oldSmall := i.large, i.small
	iconSendMessage.Call(uintptr(hwnd), wmSetIcon, iconBig, large)
	iconSendMessage.Call(uintptr(hwnd), wmSetIcon, iconSmall, small)
	if oldWindow != 0 && oldWindow != hwnd {
		iconSendMessage.Call(uintptr(oldWindow), wmSetIcon, iconBig, 0)
		iconSendMessage.Call(uintptr(oldWindow), wmSetIcon, iconSmall, 0)
	}
	i.large, i.small = windows.Handle(large), windows.Handle(small)
	i.hwnd = hwnd
	i.dark, i.initialized = dark, true
	destroyWindowIcons(oldLarge, oldSmall)
}

// Release frees every owned icon handle.
func (i *WindowIcons) Release() {
	if i.hwnd != 0 {
		iconSendMessage.Call(uintptr(i.hwnd), wmSetIcon, iconBig, 0)
		iconSendMessage.Call(uintptr(i.hwnd), wmSetIcon, iconSmall, 0)
	}
	i.release()
	i.hwnd = 0
	i.initialized = false
}

func (i *WindowIcons) release() {
	destroyWindowIcons(i.large, i.small)
	i.large, i.small = 0, 0
}

func destroyWindowIcons(icons ...windows.Handle) {
	for _, icon := range icons {
		if icon != 0 {
			iconDestroy.Call(uintptr(icon))
		}
	}
}
