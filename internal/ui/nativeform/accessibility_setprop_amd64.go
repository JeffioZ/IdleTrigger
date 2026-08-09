//go:build amd64

package nativeform

import (
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

func callSetHwndPropInt(method, service uintptr, hwnd windows.Handle, property *windows.GUID, value accVariant) uintptr {
	result, _, _ := syscall.SyscallN(
		method, service, uintptr(hwnd), uintptr(accObjIDClient), uintptr(accChildIDSelf),
		uintptr(unsafe.Pointer(property)), uintptr(unsafe.Pointer(&value)),
	)
	return result
}

func callSetHwndPropString(method, service uintptr, hwnd windows.Handle, property *windows.GUID, value *uint16) uintptr {
	result, _, _ := syscall.SyscallN(
		method, service, uintptr(hwnd), uintptr(accObjIDClient), uintptr(accChildIDSelf),
		uintptr(unsafe.Pointer(property)), uintptr(unsafe.Pointer(value)),
	)
	return result
}
