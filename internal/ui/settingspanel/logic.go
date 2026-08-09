package settingspanel

import (
	"reflect"
	"strconv"
	"strings"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
)

func (p *panel) toggleChoice(id uint16) {
	if p.choiceOpen == id {
		p.closeChoice(true)
		return
	}
	p.openChoice(id)
}

func (p *panel) openChoice(id uint16) {
	p.closeChoice(false)
	item := p.choices[id]
	if item == nil || len(item.labels) == 0 {
		return
	}
	items := make([]nativeform.ChoicePopupItem, len(item.labels))
	for index, label := range item.labels {
		items[index] = nativeform.ChoicePopupItem{Label: label, Value: index}
	}
	p.choiceOpen = id
	popup, err := nativeform.ShowChoicePopup(nativeform.ChoicePopupOptions{
		Owner: p.hwnd, Anchor: p.controls[id], Font: p.font, SelectedFont: p.sectionFont,
		Palette: p.palette, Dark: p.themeDark, Scale: p.scale(), Selected: item.selected,
		MaxVisible: 5, Items: items, RestoreAnchorOnCancel: true,
		OnSelect: func(index int) {
			p.choicePopup = nil
			p.choiceOpen = 0
			p.selectChoice(id, index)
			p.applyDependentStates()
			p.clearValidation()
			pSetFocus.Call(uintptr(p.controls[id]))
		},
		OnClose: func() {
			p.choicePopup = nil
			p.choiceOpen = 0
			pInvalidateRect.Call(uintptr(p.controls[id]), 0, 0)
		},
	})
	if err != nil {
		p.choiceOpen = 0
		return
	}
	p.choicePopup = popup
	pInvalidateRect.Call(uintptr(p.controls[id]), 0, 0)
}

func (p *panel) closeChoice(returnFocus bool) {
	id, popup := p.choiceOpen, p.choicePopup
	p.choiceOpen, p.choicePopup = 0, nil
	if popup != nil {
		popup.Close()
	}
	if id != 0 && p.controls[id] != 0 {
		pInvalidateRect.Call(uintptr(p.controls[id]), 0, 0)
		if returnFocus {
			pSetFocus.Call(uintptr(p.controls[id]))
		}
	}
}

func (p *panel) handleCommand(id, notification uint16) {
	if notification == enChange {
		p.sanitizeEdit(id)
		p.clearValidation()
		return
	}
	if notification != bnClicked {
		return
	}
	switch id {
	case idKeepScreen, idBatteryAllowed, idIdleEnhanced, idThemeBattery, idThemeFullscreen, idHotkeys, idAutostart, idLogging:
		p.setChecked(id, !p.checks[id])
		p.applyDependentStates()
		p.clearValidation()
	case idThemeMode, idLocationSource, idIdleAction, idLanguage:
		p.toggleChoice(id)
	case idTabPower, idTabTheme, idTabApp:
		p.page = map[uint16]int{idTabPower: 0, idTabTheme: 1, idTabApp: 2}[id]
		p.closeChoice(false)
		p.applyDependentStates()
	case idThemeRepair:
		if p.onRepairTheme != nil {
			p.onRepairTheme()
		}
	case idProjectHome:
		if p.onProjectHome != nil {
			p.onProjectHome()
		}
	case idCancel:
		p.cancel()
	case idSave:
		p.save()
	}
}

func (p *panel) requestFromControls() (SaveRequest, uint16, string) {
	draft := settingsDraft{
		BaseRevision:        p.state.Revision,
		KeepScreenOn:        p.checks[idKeepScreen],
		NoSleepOnBattery:    p.checks[idBatteryAllowed],
		BatteryThreshold:    p.controlText(idBatteryThreshold),
		IdleTimeout:         p.controlText(idIdleTimeout),
		WarningSeconds:      p.controlText(idWarningSeconds),
		IdleAction:          p.choiceIndex(idIdleAction),
		IdleEnhancedMonitor: p.checks[idIdleEnhanced],
		IdleEnabled:         p.state.IdleEnabled,
		ThemeMode:           p.choiceIndex(idThemeMode),
		LightTime:           p.controlText(idLightTime),
		DarkTime:            p.controlText(idDarkTime),
		LocationSource:      p.choiceIndex(idLocationSource),
		ThemeDarkOnBattery:  p.checks[idThemeBattery],
		ThemeSkipFullscreen: p.checks[idThemeFullscreen],
		Language:            p.choiceIndex(idLanguage),
		HotkeysEnabled:      p.checks[idHotkeys], AutostartEnabled: p.checks[idAutostart], LoggingEnabled: p.checks[idLogging],
	}
	request, id, key := parseSettingsDraft(draft)
	if key != "" {
		return request, id, p.t(key)
	}
	return request, 0, ""
}

type settingsDraft struct {
	BaseRevision, BatteryThreshold, IdleTimeout, WarningSeconds               string
	LightTime, DarkTime                                                       string
	KeepScreenOn, NoSleepOnBattery                                            bool
	IdleEnabled, IdleEnhancedMonitor, ThemeDarkOnBattery, ThemeSkipFullscreen bool
	HotkeysEnabled, AutostartEnabled, LoggingEnabled                          bool
	IdleAction, ThemeMode, LocationSource, Language                           int
}

func parseSettingsDraft(draft settingsDraft) (SaveRequest, uint16, string) {
	request := SaveRequest{BaseRevision: draft.BaseRevision, KeepScreenOn: draft.KeepScreenOn,
		NoSleepOnBattery: draft.NoSleepOnBattery, IdleEnabled: draft.IdleEnabled, IdleEnhancedMonitor: draft.IdleEnhancedMonitor,
		ThemeLightTime: draft.LightTime, ThemeDarkTime: draft.DarkTime, ThemeDarkOnBattery: draft.ThemeDarkOnBattery,
		ThemeSkipFullscreen: draft.ThemeSkipFullscreen, HotkeysEnabled: draft.HotkeysEnabled,
		AutostartEnabled: draft.AutostartEnabled, LoggingEnabled: draft.LoggingEnabled}
	thresholdText := strings.TrimSpace(draft.BatteryThreshold)
	if !draft.NoSleepOnBattery && thresholdText == "" {
		thresholdText = "0"
	}
	threshold, err := strconv.Atoi(thresholdText)
	if err != nil || threshold < 0 || threshold > 100 {
		return request, idBatteryThreshold, "settings_error_battery_threshold"
	}
	request.NoSleepBatteryThreshold = threshold
	idleTimeout, err := strconv.Atoi(draft.IdleTimeout)
	if err != nil || idleTimeout < 1 || idleTimeout > 7*24*60 {
		return request, idIdleTimeout, "settings_error_idle_timeout"
	}
	request.IdleTimeoutMinutes = idleTimeout
	request.IdleAction = idleActionValues[max(0, min(draft.IdleAction, len(idleActionValues)-1))]
	warning, err := strconv.Atoi(draft.WarningSeconds)
	if err != nil || warning < 0 || warning > 3600 {
		return request, idWarningSeconds, "settings_error_warning_seconds"
	}
	request.IdleWarningSeconds = warning
	request.Language = []string{"auto", "en", "zh-CN"}[max(0, min(draft.Language, 2))]
	if draft.ThemeMode == 1 {
		request.ThemeMode = "sunrise"
	} else {
		request.ThemeMode = "fixed"
	}
	if len(request.ThemeLightTime) != 5 {
		return request, idLightTime, "settings_error_light_time"
	}
	if _, err := time.Parse("15:04", request.ThemeLightTime); err != nil {
		return request, idLightTime, "settings_error_light_time"
	}
	if len(request.ThemeDarkTime) != 5 {
		return request, idDarkTime, "settings_error_dark_time"
	}
	if _, err := time.Parse("15:04", request.ThemeDarkTime); err != nil {
		return request, idDarkTime, "settings_error_dark_time"
	}
	request.ThemeIPLocationEnabled = draft.LocationSource == 1
	return request, 0, ""
}

func requestFromState(state State) SaveRequest {
	return SaveRequest{BaseRevision: state.Revision, KeepScreenOn: state.KeepScreenOn,
		NoSleepOnBattery: state.NoSleepOnBattery, NoSleepBatteryThreshold: state.NoSleepBatteryThreshold,
		IdleEnabled: state.IdleEnabled, IdleTimeoutMinutes: state.IdleTimeoutMinutes, IdleAction: state.IdleAction,
		IdleWarningSeconds: state.IdleWarningSeconds, IdleEnhancedMonitor: state.IdleEnhancedMonitor, ThemeMode: state.ThemeMode,
		ThemeLightTime: state.ThemeLightTime, ThemeDarkTime: state.ThemeDarkTime,
		ThemeIPLocationEnabled: state.ThemeIPLocationEnabled, ThemeDarkOnBattery: state.ThemeDarkOnBattery,
		ThemeSkipFullscreen: state.ThemeSkipFullscreen, Language: state.Language, HotkeysEnabled: state.HotkeysEnabled,
		AutostartEnabled: state.AutostartEnabled, LoggingEnabled: state.LoggingEnabled}
}

func (p *panel) save() {
	request, id, message := p.requestFromControls()
	if message != "" {
		p.setError(id, message)
		return
	}
	if p.onSave == nil {
		pDestroyWindow.Call(uintptr(p.hwnd))
		return
	}
	result := p.onSave(request)
	if result.Error != "" {
		p.setError(0, result.Error)
		return
	}
	pDestroyWindow.Call(uintptr(p.hwnd))
	if p.onSaved != nil {
		p.onSaved()
	}
}

func (p *panel) cancel() {
	request, _, _ := p.requestFromControls()
	request.BaseRevision = p.state.Revision
	if !reflect.DeepEqual(request, requestFromState(p.state)) && !p.confirm(p.t("settings_discard_title"), p.t("settings_discard_confirm")) {
		return
	}
	pDestroyWindow.Call(uintptr(p.hwnd))
}

func (p *panel) setError(id uint16, message string) {
	p.validationError = true
	p.setText(idValidation, message)
	if id != 0 && p.controls[id] != 0 {
		p.page = pageForControl(id)
		p.applyDependentStates()
		pSetFocus.Call(uintptr(p.controls[id]))
	}
}

func pageForControl(id uint16) int {
	for page, ids := range pageControlIDs() {
		if containsID(ids, id) {
			return page
		}
	}
	return 0
}

func (p *panel) sanitizeEdit(id uint16) {
	var filtered string
	value := p.controlText(id)
	switch id {
	case idBatteryThreshold:
		filtered = filterDigits(value, 3)
	case idIdleTimeout:
		filtered = filterDigits(value, 5)
	case idWarningSeconds:
		filtered = filterDigits(value, 4)
	case idLightTime, idDarkTime:
		filtered = filterTime(value)
	default:
		return
	}
	if filtered != value {
		p.setText(id, filtered)
		pSendMessage.Call(uintptr(p.controls[id]), emSetSel, ^uintptr(0), ^uintptr(0))
	}
}

func filterDigits(value string, limit int) string {
	out := make([]rune, 0, min(len(value), limit))
	for _, r := range value {
		if r >= '0' && r <= '9' && len(out) < limit {
			out = append(out, r)
		}
	}
	return string(out)
}

func filterTime(value string) string {
	digits := filterDigits(value, 4)
	if len(digits) > 2 {
		return digits[:2] + ":" + digits[2:]
	}
	if len(digits) == 2 && strings.Contains(value, ":") {
		return digits + ":"
	}
	return digits
}

func (p *panel) clearValidation() {
	if !p.validationError {
		return
	}
	p.validationError = false
	p.setText(idValidation, "")
}

func (p *panel) confirm(title, body string) bool {
	titlePtr, _ := windows.UTF16PtrFromString(title)
	bodyPtr, _ := windows.UTF16PtrFromString(body)
	const yesNoWarningDefaultNo = 0x00000004 | 0x00000030 | 0x00000100
	result, _, _ := user32.NewProc("MessageBoxW").Call(uintptr(p.hwnd), uintptr(unsafe.Pointer(bodyPtr)), uintptr(unsafe.Pointer(titlePtr)), yesNoWarningDefaultNo)
	return result == 6
}
