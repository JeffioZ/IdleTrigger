// Package powerstate reads Windows sleep capabilities and current battery state.
package powerstate

import (
	"sync"
	"unsafe"

	"golang.org/x/sys/windows"
)

type Capabilities struct {
	SleepAvailable     bool
	HibernateAvailable bool
}

type Status struct {
	ACLine   bool
	Battery  bool
	Percent  int
	Charging bool
	Valid    bool
}

var (
	cachedCaps Capabilities
	capsOnce   sync.Once
	powrprof   = windows.NewLazySystemDLL("powrprof.dll")
	kernel32   = windows.NewLazySystemDLL("kernel32.dll")

	pCallNtPowerInformation = powrprof.NewProc("CallNtPowerInformation")
	pGetSystemPowerStatus   = kernel32.NewProc("GetSystemPowerStatus")
)

func GetCapabilities() Capabilities {
	capsOnce.Do(func() {
		var caps Capabilities
		// Byte offsets into SYSTEM_POWER_CAPABILITIES returned by
		// CallNtPowerInformation(SystemPowerCapabilities). They follow the SDK
		// layout (SystemS3/S4 at 5/6, HiberFilePresent at 8, AoAc at 20):
		// https://learn.microsoft.com/windows/win32/api/winnt/ns-winnt-system_power_capabilities
		const (
			bufSize                 = 128
			systemS3SupportedOffset = 5
			systemS4SupportedOffset = 6
			hiberFilePresentOffset  = 8
			aoAcSupportedOffset     = 20
		)
		var buf [bufSize]byte
		const systemPowerCaps = 4
		ret, _, _ := pCallNtPowerInformation.Call(uintptr(systemPowerCaps), 0, 0,
			uintptr(unsafe.Pointer(&buf[0])), uintptr(bufSize))
		if ret != 0 {
			cachedCaps = caps
			return
		}
		caps.SleepAvailable = buf[systemS3SupportedOffset] != 0 || buf[aoAcSupportedOffset] != 0
		caps.HibernateAvailable = buf[systemS4SupportedOffset] != 0 && buf[hiberFilePresentOffset] != 0
		cachedCaps = caps
	})
	return cachedCaps
}

func GetStatus() Status {
	type sps struct {
		ACLineStatus        byte
		BatteryFlag         byte
		BatteryLifePercent  byte
		SystemStatusFlag    byte
		BatteryLifeTime     uint32
		BatteryFullLifeTime uint32
	}
	var s sps
	r, _, _ := pGetSystemPowerStatus.Call(uintptr(unsafe.Pointer(&s)))
	if r == 0 {
		return Status{}
	}
	batteryKnown := s.BatteryFlag != 255
	percent := int(s.BatteryLifePercent)
	if s.BatteryLifePercent == 255 {
		percent = -1
	}
	return Status{
		ACLine:   s.ACLineStatus == 1,
		Battery:  batteryKnown && s.BatteryFlag != 128,
		Percent:  percent,
		Charging: s.BatteryFlag&0x08 != 0,
		Valid:    s.ACLineStatus != 255 && batteryKnown,
	}
}

func OnBattery() bool {
	status := GetStatus()
	return status.Valid && status.Battery && !status.ACLine
}
