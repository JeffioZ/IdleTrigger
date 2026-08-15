package app

import (
	"fmt"

	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

const (
	pbtAPMSuspend           uint32 = 0x0004
	pbtAPMResumeSuspend     uint32 = 0x0007
	pbtAPMResumeAutomatic   uint32 = 0x0012
	pbtAPMPowerStatusChange uint32 = 0x000a
	pbtPowerSettingChange   uint32 = 0x8013
)

func powerEventName(event uint32) string {
	switch event {
	case pbtAPMSuspend:
		return "suspend"
	case pbtAPMResumeSuspend:
		return "resume-user"
	case pbtAPMResumeAutomatic:
		return "resume-automatic"
	case pbtAPMPowerStatusChange:
		return "power-status-change"
	case pbtPowerSettingChange:
		return "power-setting-change"
	default:
		return "unknown"
	}
}

func isResumePowerEvent(event uint32) bool {
	return event == pbtAPMResumeSuspend || event == pbtAPMResumeAutomatic
}

func powerSettingName(setting trayicon.PowerSetting) string {
	switch setting {
	case trayicon.PowerSettingACSource:
		return "ac-source"
	case trayicon.PowerSettingBatteryPercentage:
		return "battery-percent"
	default:
		return "none"
	}
}

func powerSettingValue(event trayicon.PowerEvent) string {
	if !event.HasValue {
		return "n/a"
	}
	if event.Setting == trayicon.PowerSettingACSource {
		switch event.Value {
		case 0:
			return "ac"
		case 1:
			return "battery"
		case 2:
			return "short-term"
		}
	}
	return fmt.Sprintf("%d", event.Value)
}
