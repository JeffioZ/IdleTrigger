package font

import (
	"math"
	"sync/atomic"
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

const (
	roInitMultithreaded = uintptr(1)
	textScaleMinimum    = 1.0
	textScaleMaximum    = 2.25
)

var (
	textScaleCombase = windows.NewLazySystemDLL("combase.dll")
	pRoInitialize    = textScaleCombase.NewProc("RoInitialize")
	pRoUninitialize  = textScaleCombase.NewProc("RoUninitialize")
	pRoActivate      = textScaleCombase.NewProc("RoActivateInstance")
	pCreateHString   = textScaleCombase.NewProc("WindowsCreateString")
	pDeleteHString   = textScaleCombase.NewProc("WindowsDeleteString")

	iidUISettings2       = windows.GUID{Data1: 0xbad82401, Data2: 0x2721, Data3: 0x44f9, Data4: [8]byte{0xbb, 0x91, 0x2b, 0xb2, 0x28, 0xbe, 0x44, 0x2f}}
	textScaleOverride    atomic.Uint64
	queryTextScaleFactor = querySystemTextScaleFactor
)

type inspectable struct{ vtable *inspectableVTable }
type inspectableVTable struct {
	queryInterface, addRef, release             uintptr
	getIids, getRuntimeClassName, getTrustLevel uintptr
}

// TextScaleFactor returns the Windows accessibility text-size preference.
// Win32/GDI does not apply this setting automatically to custom text surfaces.
func TextScaleFactor() float64 {
	if bits := textScaleOverride.Load(); bits != 0 {
		return math.Float64frombits(bits)
	}
	value := queryTextScaleFactor()
	if value < textScaleMinimum || value > textScaleMaximum {
		return textScaleMinimum
	}
	return value
}

// OverrideTextScaleFactor keeps deterministic capture/test hosts independent
// of the workstation accessibility setting. The returned function restores the
// previous override.
func OverrideTextScaleFactor(value float64) func() {
	previous := textScaleOverride.Swap(math.Float64bits(value))
	return func() { textScaleOverride.Store(previous) }
}

func scaleRequestedSize(size int32) int32 {
	if size <= 0 {
		return size
	}
	return max(1, int32(math.Round(float64(size)*TextScaleFactor())))
}

func querySystemTextScaleFactor() float64 {
	initialized, _, _ := pRoInitialize.Call(roInitMultithreaded)
	if int32(initialized) >= 0 {
		defer pRoUninitialize.Call()
	}
	className, err := windows.UTF16FromString("Windows.UI.ViewManagement.UISettings")
	if err != nil {
		return textScaleMinimum
	}
	var classString uintptr
	hr, _, _ := pCreateHString.Call(uintptr(unsafe.Pointer(&className[0])), uintptr(len(className)-1), uintptr(unsafe.Pointer(&classString)))
	if int32(hr) < 0 || classString == 0 {
		return textScaleMinimum
	}
	defer pDeleteHString.Call(classString)
	var instance unsafe.Pointer
	hr, _, _ = pRoActivate.Call(classString, uintptr(unsafe.Pointer(&instance)))
	if int32(hr) < 0 || instance == nil {
		return textScaleMinimum
	}
	base := (*inspectable)(instance)
	instanceAddress := uintptr(instance)
	defer syscall.SyscallN(base.vtable.release, instanceAddress)
	var settings2 unsafe.Pointer
	hr, _, _ = syscall.SyscallN(base.vtable.queryInterface, instanceAddress, uintptr(unsafe.Pointer(&iidUISettings2)), uintptr(unsafe.Pointer(&settings2)))
	if int32(hr) < 0 || settings2 == nil {
		return textScaleMinimum
	}
	view := (*inspectable)(settings2)
	settingsAddress := uintptr(settings2)
	defer syscall.SyscallN(view.vtable.release, settingsAddress)
	var value float64
	getTextScaleFactor := (*[7]uintptr)(unsafe.Pointer(view.vtable))[6]
	hr, _, _ = syscall.SyscallN(getTextScaleFactor, settingsAddress, uintptr(unsafe.Pointer(&value)))
	if int32(hr) < 0 {
		return textScaleMinimum
	}
	return value
}
