package trayicon

import (
	"unsafe"

	"golang.org/x/sys/windows"
)

const pbtPowerSettingChange uint32 = 0x8013

var (
	guidACDCSource = windows.GUID{
		Data1: 0x5d3e9a59, Data2: 0xe9d5, Data3: 0x4b00,
		Data4: [8]byte{0xa6, 0xbd, 0xff, 0x34, 0xff, 0x51, 0x65, 0x48},
	}
	guidBatteryPercentage = windows.GUID{
		Data1: 0xa7ad8041, Data2: 0xb45a, Data3: 0x4cae,
		Data4: [8]byte{0x87, 0xa3, 0xee, 0xcb, 0xb4, 0x68, 0xa9, 0xe1},
	}
)

type powerBroadcastSetting struct {
	PowerSetting windows.GUID
	DataLength   uint32
}

func (t *winTray) registerPowerNotifications() {
	const deviceNotifyWindowHandle = 0
	for _, setting := range []*windows.GUID{&guidACDCSource, &guidBatteryPercentage} {
		handle, _, err := pRegisterPowerSettingNotification.Call(
			uintptr(t.window),
			uintptr(unsafe.Pointer(setting)),
			deviceNotifyWindowHandle,
		)
		if handle == 0 {
			reportError("Unable to register power setting notification: %v", err)
			continue
		}
		t.powerNotifications = append(t.powerNotifications, windows.Handle(handle))
	}
}

func (t *winTray) unregisterPowerNotifications() {
	for _, handle := range t.powerNotifications {
		if handle == 0 {
			continue
		}
		result, _, err := pUnregisterPowerSettingNotification.Call(uintptr(handle))
		if result == 0 {
			reportError("Unable to unregister power setting notification: %v", err)
		}
	}
	t.powerNotifications = nil
}

func decodePowerEvent(code uint32, lParam uintptr) PowerEvent {
	event := PowerEvent{Code: code}
	if code != pbtPowerSettingChange || lParam == 0 {
		return event
	}
	setting := (*powerBroadcastSetting)(powerMessagePointer(lParam))
	if setting.DataLength < 4 {
		return event
	}
	switch setting.PowerSetting {
	case guidACDCSource:
		event.Setting = PowerSettingACSource
	case guidBatteryPercentage:
		event.Setting = PowerSettingBatteryPercentage
	default:
		return event
	}
	data := unsafe.Add(unsafe.Pointer(setting), unsafe.Sizeof(*setting))
	event.Value = *(*uint32)(data)
	event.HasValue = true
	return event
}

// powerMessagePointer recovers the pointer carried by the synchronous
// WM_POWERBROADCAST LPARAM. The payload is valid only for the duration of the
// window callback and is decoded before the application callback is started.
func powerMessagePointer(value uintptr) unsafe.Pointer {
	return *(*unsafe.Pointer)(unsafe.Pointer(&value))
}
