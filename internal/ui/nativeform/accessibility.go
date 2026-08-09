package nativeform

import (
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

// Active Accessibility dynamic annotation lets the app preserve the native
// BUTTON input implementation and its compact owner-drawn visuals while
// exposing the semantic role and state that the BS_OWNERDRAW style cannot
// describe on its own.
const (
	accObjIDClient           = uint32(0xfffffffc)
	accChildIDSelf           = uint32(0)
	accRoleSystemCheckButton = int32(0x2c)
	accStateUnavailable      = int32(0x00000001)
	accStateFocused          = int32(0x00000004)
	accStateChecked          = int32(0x00000010)
	accStateFocusable        = int32(0x00100000)
	accEventStateChange      = uintptr(0x800a)
	accVTInt32               = uint16(3)
	accCLSCTXInprocServer    = uintptr(1)
	accCoInitApartment       = uintptr(2)
)

var (
	accOle32             = windows.NewLazySystemDLL("ole32.dll")
	accUser32            = windows.NewLazySystemDLL("user32.dll")
	pAccCoInitializeEx   = accOle32.NewProc("CoInitializeEx")
	pAccCoUninitialize   = accOle32.NewProc("CoUninitialize")
	pAccCoCreateInstance = accOle32.NewProc("CoCreateInstance")
	pAccGetFocus         = accUser32.NewProc("GetFocus")
	pAccIsWindowEnabled  = accUser32.NewProc("IsWindowEnabled")
	pAccNotifyWinEvent   = accUser32.NewProc("NotifyWinEvent")

	accCLSIDPropServices = windows.GUID{Data1: 0xb5f8350b, Data2: 0x0548, Data3: 0x48b1, Data4: [8]byte{0xa6, 0xee, 0x88, 0xbd, 0x00, 0xb4, 0xa5, 0xe7}}
	accIIDPropServices   = windows.GUID{Data1: 0x6e26e776, Data2: 0x04f0, Data3: 0x495d, Data4: [8]byte{0x80, 0xe4, 0x33, 0x30, 0x35, 0x2e, 0x31, 0x69}}
	accPropName          = windows.GUID{Data1: 0x608d3df8, Data2: 0x8128, Data3: 0x4aa7, Data4: [8]byte{0xa4, 0x28, 0xf5, 0x5e, 0x49, 0x26, 0x72, 0x91}}
	accPropRole          = windows.GUID{Data1: 0xcb905ff2, Data2: 0x7bd1, Data3: 0x4c05, Data4: [8]byte{0xb3, 0xc8, 0xe6, 0xc2, 0x41, 0x36, 0x4d, 0x70}}
	accPropState         = windows.GUID{Data1: 0xa8d4d5b0, Data2: 0x0a21, Data3: 0x42d0, Data4: [8]byte{0xa5, 0xc0, 0x51, 0x4e, 0x98, 0x4f, 0x45, 0x7b}}

	setAccessibleIntValue    = setHwndAccessibleInt
	setAccessibleStringValue = setHwndAccessibleString
)

type accPropServices struct{ vtable *accPropServicesVTable }

type accPropServicesVTable struct {
	queryInterface, addRef, release                            uintptr
	setPropValue, setPropServer, clearProps                    uintptr
	setHwndProp, setHwndPropStr, setHwndPropServer             uintptr
	clearHwndProps, composeHwndIdentity, decomposeHwndIdentity uintptr
}

type accVariant struct {
	Type, Reserved1, Reserved2, Reserved3 uint16
	Value                                 int64
}

func withAccPropServices(run func(uintptr, *accPropServicesVTable)) bool {
	initialized, _, _ := pAccCoInitializeEx.Call(0, accCoInitApartment)
	shouldUninitialize := int32(initialized) >= 0
	if shouldUninitialize {
		defer pAccCoUninitialize.Call()
	}
	var service unsafe.Pointer
	hr, _, _ := pAccCoCreateInstance.Call(
		uintptr(unsafe.Pointer(&accCLSIDPropServices)), 0, accCLSCTXInprocServer,
		uintptr(unsafe.Pointer(&accIIDPropServices)), uintptr(unsafe.Pointer(&service)),
	)
	if int32(hr) < 0 || service == nil {
		return false
	}
	object := (*accPropServices)(service)
	serviceAddress := uintptr(service)
	defer syscall.SyscallN(object.vtable.release, serviceAddress)
	run(serviceAddress, object.vtable)
	return true
}

func setHwndAccessibleString(hwnd windows.Handle, property windows.GUID, value string) bool {
	text, err := windows.UTF16PtrFromString(value)
	if hwnd == 0 || err != nil {
		return false
	}
	success := false
	withAccPropServices(func(service uintptr, table *accPropServicesVTable) {
		hr := callSetHwndPropString(table.setHwndPropStr, service, hwnd, &property, text)
		success = int32(hr) >= 0
	})
	return success
}

func setHwndAccessibleInt(hwnd windows.Handle, property windows.GUID, value int32) bool {
	if hwnd == 0 {
		return false
	}
	variant := accVariant{Type: accVTInt32, Value: int64(value)}
	success := false
	withAccPropServices(func(service uintptr, table *accPropServicesVTable) {
		hr := callSetHwndPropInt(table.setHwndProp, service, hwnd, &property, variant)
		success = int32(hr) >= 0
	})
	return success
}

// AnnotateCheckButton supplies the accessible checkbox role, name and current
// state for an owner-drawn native BUTTON.
func AnnotateCheckButton(hwnd windows.Handle, name string, checked bool) {
	setAccessibleStringValue(hwnd, accPropName, name)
	setAccessibleIntValue(hwnd, accPropRole, accRoleSystemCheckButton)
	UpdateCheckButtonAccessibility(hwnd, checked)
}

// SetAccessibleName gives a compact/icon-only native control a descriptive
// screen-reader name without changing its visible caption.
func SetAccessibleName(hwnd windows.Handle, name string) {
	setAccessibleStringValue(hwnd, accPropName, name)
}

// UpdateCheckButtonAccessibility refreshes the state annotation after checked,
// focus, or enabled state changes and announces the change to assistive tools.
func UpdateCheckButtonAccessibility(hwnd windows.Handle, checked bool) {
	state := accessibleCheckState(hwnd, checked)
	if setAccessibleIntValue(hwnd, accPropState, state) {
		pAccNotifyWinEvent.Call(accEventStateChange, uintptr(hwnd), uintptr(accObjIDClient), uintptr(accChildIDSelf))
	}
}

func accessibleCheckState(hwnd windows.Handle, checked bool) int32 {
	state := accStateFocusable
	if checked {
		state |= accStateChecked
	}
	if hwnd != 0 {
		if enabled, _, _ := pAccIsWindowEnabled.Call(uintptr(hwnd)); enabled == 0 {
			state |= accStateUnavailable
		}
		if focused, _, _ := pAccGetFocus.Call(); windows.Handle(focused) == hwnd {
			state |= accStateFocused
		}
	}
	return state
}
