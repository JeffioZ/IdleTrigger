//go:build 386

package nativeform

import (
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

func callSetHwndPropInt(method, service uintptr, hwnd windows.Handle, property *windows.GUID, value accVariant) uintptr {
	propertyWords := *(*[4]uint32)(unsafe.Pointer(property))
	first := uintptr(value.Type) | uintptr(value.Reserved1)<<16
	second := uintptr(value.Reserved2) | uintptr(value.Reserved3)<<16
	bits := uint64(value.Value)
	result, _, _ := syscall.SyscallN(
		method, service, uintptr(hwnd), uintptr(accObjIDClient), uintptr(accChildIDSelf),
		uintptr(propertyWords[0]), uintptr(propertyWords[1]), uintptr(propertyWords[2]), uintptr(propertyWords[3]),
		first, second, uintptr(uint32(bits)), uintptr(uint32(bits>>32)),
	)
	return result
}

func callSetHwndPropString(method, service uintptr, hwnd windows.Handle, property *windows.GUID, value *uint16) uintptr {
	propertyWords := *(*[4]uint32)(unsafe.Pointer(property))
	result, _, _ := syscall.SyscallN(
		method, service, uintptr(hwnd), uintptr(accObjIDClient), uintptr(accChildIDSelf),
		uintptr(propertyWords[0]), uintptr(propertyWords[1]), uintptr(propertyWords[2]), uintptr(propertyWords[3]),
		uintptr(unsafe.Pointer(value)),
	)
	return result
}
