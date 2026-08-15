package trayicon

import (
	"testing"
	"unsafe"

	"golang.org/x/sys/windows"
)

type testPowerBroadcastSetting struct {
	PowerSetting windows.GUID
	DataLength   uint32
	Data         uint32
}

func TestPowerBroadcastSettingDataOffsetMatchesWindowsABI(t *testing.T) {
	if got := unsafe.Sizeof(powerBroadcastSetting{}); got != 20 {
		t.Fatalf("POWERBROADCAST_SETTING header size = %d, want 20", got)
	}
}

func TestDecodeRegisteredPowerEvents(t *testing.T) {
	for _, test := range []struct {
		name    string
		guid    windows.GUID
		value   uint32
		setting PowerSetting
	}{
		{name: "AC source", guid: guidACDCSource, value: 1, setting: PowerSettingACSource},
		{name: "battery percentage", guid: guidBatteryPercentage, value: 42, setting: PowerSettingBatteryPercentage},
	} {
		t.Run(test.name, func(t *testing.T) {
			payload := testPowerBroadcastSetting{PowerSetting: test.guid, DataLength: 4, Data: test.value}
			got := decodePowerEvent(pbtPowerSettingChange, uintptr(unsafe.Pointer(&payload)))
			if got.Code != pbtPowerSettingChange || got.Setting != test.setting || !got.HasValue || got.Value != test.value {
				t.Fatalf("decoded event = %+v", got)
			}
		})
	}
}

func TestDecodePowerEventRejectsUnknownOrShortPayload(t *testing.T) {
	payload := testPowerBroadcastSetting{PowerSetting: windows.GUID{Data1: 1}, DataLength: 4, Data: 9}
	if got := decodePowerEvent(pbtPowerSettingChange, uintptr(unsafe.Pointer(&payload))); got.Setting != PowerSettingNone || got.HasValue {
		t.Fatalf("unknown setting decoded: %+v", got)
	}
	payload.PowerSetting = guidACDCSource
	payload.DataLength = 3
	if got := decodePowerEvent(pbtPowerSettingChange, uintptr(unsafe.Pointer(&payload))); got.HasValue {
		t.Fatalf("short setting payload decoded: %+v", got)
	}
}
