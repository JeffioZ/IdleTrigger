package nativeform

import (
	"unsafe"

	"golang.org/x/sys/windows"
)

const windowProcIndex = ^uintptr(3) // GWLP_WNDPROC / GWL_WNDPROC (-4)

var (
	windowProcUser32       = windows.NewLazySystemDLL("user32.dll")
	windowSetWindowLong    = windowProcUser32.NewProc("SetWindowLongW")
	windowSetWindowLongPtr = windowProcUser32.NewProc("SetWindowLongPtrW")
)

// SetWindowProc replaces a window procedure and returns the previous one.
// Win32 exposes SetWindowLongW on 32-bit and SetWindowLongPtrW on 64-bit.
func SetWindowProc(hwnd windows.Handle, proc uintptr) (uintptr, uintptr, error) {
	if unsafe.Sizeof(uintptr(0)) == 4 {
		return windowSetWindowLong.Call(uintptr(hwnd), windowProcIndex, proc)
	}
	return windowSetWindowLongPtr.Call(uintptr(hwnd), windowProcIndex, proc)
}
