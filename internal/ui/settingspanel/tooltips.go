package settingspanel

import (
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
)

func (p *panel) createTooltips() {
	class, err := windows.UTF16PtrFromString("tooltips_class32")
	if err != nil {
		return
	}
	hwnd, _, _ := pCreateWindowEx.Call(0, uintptr(unsafe.Pointer(class)), 0, wsPopup|ttsAlwaysTip|ttsNoPrefix,
		0, 0, 0, 0, uintptr(p.hwnd), 0, 0, 0)
	if hwnd == 0 {
		return
	}
	p.tooltip = windows.Handle(hwnd)
	p.tooltipText = make(map[windows.Handle][]uint16)
	pSendMessage.Call(hwnd, ttmSetMaxTipWidth, 0, uintptr(int(380*p.scale())))
	nativeform.ApplyTooltip(p.tooltip, p.themeDark, p.palette, p.font)

	for _, binding := range settingsTooltipBindings() {
		p.addTooltip(binding.key, binding.ids...)
	}
}

type tooltipBinding struct {
	key string
	ids []uint16
}

func settingsTooltipBindings() []tooltipBinding {
	return []tooltipBinding{
		{"tip_keep_screen", []uint16{idKeepScreen}},
		{"tip_nosleep_battery", []uint16{idBatteryAllowed}},
		{"tip_nosleep_battery_threshold", []uint16{idBatteryThresholdLabel, idBatteryThreshold}},
		{"tip_idle_timeout", []uint16{idIdleTimeoutLabel, idIdleTimeout}},
		{"tip_idle_action", []uint16{idIdleActionLabel, idIdleAction}},
		{"tip_idle_warning_seconds", []uint16{idWarningLabel, idWarningSeconds}},
		{"tip_idle_enhanced", []uint16{idIdleEnhanced}},
		{"tip_theme_mode", []uint16{idThemeModeLabel, idThemeMode}},
		{"tip_theme_light_time", []uint16{idLightTimeLabel, idLightTime}},
		{"tip_theme_dark_time", []uint16{idDarkTimeLabel, idDarkTime}},
		{"tip_theme_location_source", []uint16{idLocationLabel, idLocationSource}},
		{"tip_theme_location_status", []uint16{idThemeLocationStatus}},
		{"tip_battery_theme", []uint16{idThemeBattery}},
		{"tip_fullscreen", []uint16{idThemeFullscreen}},
		{"tip_language", []uint16{idLanguageLabel, idLanguage}},
		{"tip_lock_keys", []uint16{idLockKeys}},
		{"tip_lock_key_selection", []uint16{idLockCaps, idLockNum, idLockScroll}},
		{"tip_notification_fullscreen", []uint16{idLockFullscreen}},
		{"tip_notification_preview", []uint16{idLockPreview}},
		{"tip_hotkeys", []uint16{idHotkeys}},
		{"tip_autostart", []uint16{idAutostart}},
		{"tip_logging", []uint16{idLogging}},
		{"tip_project_home", []uint16{idProjectHome}},
		{"tip_settings_cancel", []uint16{idCancel}},
		{"tip_settings_save", []uint16{idSave}},
	}
}

func (p *panel) addTooltip(key string, ids ...uint16) {
	if p.tooltip == 0 {
		return
	}
	value := p.t(key)
	if value == "" || value == key {
		return
	}
	for _, id := range ids {
		p.addTooltipHandle(p.controls[id], value)
		if field, ok := p.surfaces.ForControl(id); ok {
			p.addTooltipHandle(field.Surface, value)
		}
	}
}

func (p *panel) addTooltipHandle(control windows.Handle, value string) {
	if control == 0 {
		return
	}
	text, err := windows.UTF16FromString(value)
	if err != nil || len(text) == 0 {
		return
	}
	p.tooltipText[control] = text
	info := toolInfo{Size: uint32(unsafe.Sizeof(toolInfo{})), Flags: ttfIDIsHwnd | ttfSubclass,
		Hwnd: p.hwnd, ID: uintptr(control), Text: &text[0]}
	pSendMessage.Call(uintptr(p.tooltip), ttmAddTool, 0, uintptr(unsafe.Pointer(&info)))
}

func (p *panel) annotateFields() {
	for id, labelID := range map[uint16]uint16{
		idBatteryThreshold: idBatteryThresholdLabel,
		idIdleTimeout:      idIdleTimeoutLabel,
		idWarningSeconds:   idWarningLabel,
		idLightTime:        idLightTimeLabel,
		idDarkTime:         idDarkTimeLabel,
	} {
		nativeform.SetAccessibleName(p.controls[id], p.labels[labelID])
	}
	for id, labelID := range map[uint16]uint16{
		idIdleAction:     idIdleActionLabel,
		idThemeMode:      idThemeModeLabel,
		idLocationSource: idLocationLabel,
		idLanguage:       idLanguageLabel,
	} {
		p.updateChoiceAccessibleName(id, labelID)
	}
}

func (p *panel) updateChoiceAccessibleName(id, labelID uint16) {
	name := p.labels[labelID]
	if selected := p.labels[id]; selected != "" {
		name += ": " + selected
	}
	nativeform.SetAccessibleName(p.controls[id], name)
}
