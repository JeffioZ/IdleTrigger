package controlpanel

import (
	"fmt"
	"unsafe"

	"golang.org/x/sys/windows"
)

func (p *panel) createTooltip() {
	icc := initCommonControlsEx{Size: uint32(unsafe.Sizeof(initCommonControlsEx{})), ICC: 0x000000FF}
	pInitCommonControlsEx.Call(uintptr(unsafe.Pointer(&icc)))
	class, err := windows.UTF16PtrFromString("tooltips_class32")
	if err != nil {
		return
	}
	hwnd, _, callErr := pCreateWindowEx.Call(0, uintptr(unsafe.Pointer(class)), 0, uintptr(wsPopup|ttsAlwaysTip|ttsNoPrefix), 0, 0, 0, 0, uintptr(p.hwnd), 0, 0, 0)
	if hwnd == 0 {
		_ = callErr
		return
	}
	p.tooltip = windows.Handle(hwnd)
	pSendMessage.Call(hwnd, ttmSetMaxTipWidth, 0, uintptr(p.sc(360)))
	// Keep the native tooltip from flashing back immediately when the pointer
	// moves a few pixels across an owner-drawn control.
}

func (p *panel) addTooltip(id uint16, hwnd windows.Handle) {
	if p.tooltip == 0 || hwnd == 0 {
		return
	}
	text := p.tooltipText(id)
	if text == "" {
		return
	}
	p.tooltips[id] = windows.StringToUTF16(text)
	ti := toolInfo{
		Size:  uint32(unsafe.Sizeof(toolInfo{})),
		Flags: ttfIDIsHwnd | ttfSubclass,
		Hwnd:  p.hwnd,
		ID:    uintptr(hwnd),
		Text:  &p.tooltips[id][0],
	}
	pSendMessage.Call(uintptr(p.tooltip), ttmAddTool, 0, uintptr(unsafe.Pointer(&ti)))
}

func (p *panel) refreshTooltip(id uint16) {
	if p.tooltip == 0 || p.controls[id] == 0 {
		return
	}
	text := p.tooltipText(id)
	if text == "" {
		return
	}
	p.tooltips[id] = windows.StringToUTF16(text)
	ti := toolInfo{
		Size:  uint32(unsafe.Sizeof(toolInfo{})),
		Flags: ttfIDIsHwnd,
		Hwnd:  p.hwnd,
		ID:    uintptr(p.controls[id]),
		Text:  &p.tooltips[id][0],
	}
	pSendMessage.Call(uintptr(p.tooltip), ttmUpdateTipText, 0, uintptr(unsafe.Pointer(&ti)))
}

func (p *panel) tooltipText(id uint16) string {
	if p.themeUnavailable && isThemeControl(id) {
		body := p.text("tip_theme_unavailable")
		if p.themeUnavailableDetail != "" {
			body += "\n" + p.themeUnavailableDetail
		}
		return body
	}
	key := ""
	switch id {
	case idQuickActions:
		key = "tip_quick_actions"
	case idLock:
		key = "tip_lock"
	case idSleep:
		key = "tip_sleep"
	case idHibernate:
		key = "tip_hibernate"
	case idShutdown:
		key = "tip_shutdown"
	case idRestart:
		key = "tip_restart"
	case idNoSleep:
		return p.withPowerStatusTooltip(id, p.noSleepStatus, p.text("tip_nosleep"))
	case idAutomation:
		if p.automationSummary != "" {
			return fmt.Sprintf(p.text("tip_automation_status"), p.automationCount, p.automationSummary, p.text("tip_automation"))
		}
		key = "tip_automation"
	case idAutomationEnabled:
		key = "tip_automation_master"
	case idIdle:
		return p.withPowerStatusTooltip(id, p.idleStatus, p.idleTooltipBody())
	case idTheme:
		key = "tip_theme"
	case idThemeSwitch:
		key = "tip_theme_switch"
	case idThemeRepair:
		key = "tip_theme_repair"
	case idSettings:
		key = "tip_settings"
	case idTestWarning:
		key = "tip_idle_warning_preview"
	case idExit:
		key = "tip_exit"
	}
	if key == "" {
		return ""
	}
	return p.withStateTooltip(id, p.text(key))
}

func (p *panel) idleTooltipBody() string {
	body := p.text("tip_idle")
	if p.idleTimeoutMinutes <= 0 || p.idleAction == "" {
		return body
	}
	action := p.text(idlePreviewActionTranslationKey(p.idleAction))
	plan := ""
	if p.idleWarningSeconds > 0 {
		plan = fmt.Sprintf(p.text("tip_idle_manual_plan_warning"), p.idleTimeoutMinutes, action, p.idleWarningSeconds)
	} else {
		plan = fmt.Sprintf(p.text("tip_idle_manual_plan_silent"), p.idleTimeoutMinutes, action)
	}
	return plan + "\n" + body
}

func isThemeControl(id uint16) bool {
	return id == idTheme || id == idThemeSwitch || id == idThemeRepair
}

func (p *panel) withPowerStatusTooltip(id uint16, runtimeStatus, body string) string {
	manualKey := "tip_state_disabled"
	if p.visualState(id).Active {
		manualKey = "tip_state_enabled"
	}
	if runtimeStatus == "" {
		runtimeStatus = p.text("status_unknown")
	}
	return fmt.Sprintf(p.text("tip_power_setting_status"), p.text(manualKey), runtimeStatus, body)
}

func (p *panel) withStateTooltip(id uint16, body string) string {
	state := p.visualState(id)
	switch state.Role {
	case buttonToggle:
		key := "tip_state_disabled"
		if state.Active {
			key = "tip_state_enabled"
		}
		return fmt.Sprintf(p.text("tip_toggle_state"), p.text(key), body)
	default:
		return body
	}
}
