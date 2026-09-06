package settingspanel

import (
	"fmt"
	"strconv"
	"strings"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/config"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
)

func (p *panel) build() error {
	const (
		contentX       = 208
		contentRight   = 676
		sectionTop     = 90
		sectionTitleH  = 20
		sectionItemGap = 10
		functionGap    = 8
		sectionGap     = 16
		checkHeight    = 28
	)
	buttonHeight, fieldHeight := nativeform.ButtonHeight, nativeform.FieldHeight
	dialogButtonWidth := nativeform.DialogButtonWidth
	p.label(idTitle, p.t("settings_title"), p.titleFont, 24, 16, 460, 24)
	p.label(idDescription, p.t("settings_description"), p.font, 24, 42, 652, 20)
	p.labelRight(idVersion, fmt.Sprintf(p.t("settings_version"), p.state.Version), p.font, 508, 18, 168, 22)
	p.button(idTabPower, p.t("settings_tab_power"), 24, 90, 156, buttonHeight)
	p.button(idTabTheme, p.t("settings_tab_theme"), 24, 134, 156, buttonHeight)
	p.button(idTabNotifications, p.t("settings_tab_notifications"), 24, 178, 156, buttonHeight)
	p.button(idTabApp, p.t("settings_tab_app"), 24, 222, 156, buttonHeight)

	// Power and idle page. Conditional values keep their place and use a
	// disabled treatment so toggling a policy never shifts the page.
	p.label(idPowerTitle, p.t("settings_power_title"), p.sectionFont, contentX, sectionTop, 468, sectionTitleH)
	p.check(idKeepScreen, p.t("settings_keep_screen"), contentX, sectionTop+sectionTitleH+sectionItemGap, 468, checkHeight)
	p.check(idBatteryAllowed, p.t("settings_battery_allowed"), contentX, 156, 468, checkHeight)
	p.label(idBatteryThresholdLabel, p.t("settings_battery_threshold"), p.font, contentX, 198, 330, 22)
	p.numericEdit(idBatteryThreshold, strconv.Itoa(p.state.NoSleepBatteryThreshold), 548, 190, 128, fieldHeight)
	p.label(idPowerHint, p.t("settings_power_hint"), p.font, contentX, 232, 468, 22)
	p.label(idIdleTitle, p.t("settings_idle_title"), p.sectionFont, contentX, 270, 468, sectionTitleH)
	p.check(idIdleEnhanced, p.t("menu_idle_enhanced"), contentX, 300, 468, checkHeight)
	p.label(idIdleTimeoutLabel, p.t("settings_idle_timeout_minutes"), p.font, contentX, 344, 330, 22)
	p.numericEdit(idIdleTimeout, strconv.Itoa(p.state.IdleTimeoutMinutes), 548, 336, 128, fieldHeight)
	p.label(idWarningLabel, p.t("settings_idle_warning_seconds"), p.font, contentX, 386, 330, 22)
	p.numericEdit(idWarningSeconds, strconv.Itoa(p.state.IdleWarningSeconds), 548, 378, 128, fieldHeight)
	p.label(idIdleActionLabel, p.t("settings_idle_action"), p.font, contentX, 428, 330, 22)
	p.combo(idIdleAction, idleActionLabels(p.text), 548, 420, 128, fieldHeight)

	// Day/night page.
	p.label(idThemeScheduleTitle, p.t("settings_theme_schedule_group"), p.sectionFont, contentX, sectionTop, 468, sectionTitleH)
	p.label(idThemeModeLabel, p.t("settings_theme_mode"), p.font, contentX, 128, 220, 22)
	p.combo(idThemeMode, []string{p.t("settings_theme_fixed"), p.t("settings_theme_sunrise")}, 456, 120, 220, fieldHeight)
	p.label(idLightTimeLabel, p.t("settings_light_time"), p.font, contentX, 170, 104, 22)
	p.edit(idLightTime, p.state.ThemeLightTime, 316, 162, 104, fieldHeight)
	p.label(idDarkTimeLabel, p.t("settings_dark_time"), p.font, 438, 170, 104, 22)
	p.edit(idDarkTime, p.state.ThemeDarkTime, 546, 162, 130, fieldHeight)
	p.label(idLocationLabel, p.t("settings_location_source"), p.font, contentX, 170, 220, 22)
	p.combo(idLocationSource, []string{p.t("settings_location_auto"), p.t("settings_location_ip")}, 456, 162, 220, fieldHeight)
	p.label(idThemeLocationStatus, p.state.ThemeLocationStatus, p.font, contentX, 204, 468, 22)
	themeHintHeight := 40
	if p.state.Chinese {
		themeHintHeight = 22
	}
	p.label(idThemeHint, p.t("settings_theme_hint"), p.font, contentX, 234, 468, themeHintHeight)
	behaviorTop := 234 + themeHintHeight + sectionGap
	p.label(idThemeBehaviorTitle, p.t("settings_theme_behavior_group"), p.sectionFont, contentX, behaviorTop, 468, sectionTitleH)
	p.check(idThemeBattery, p.t("menu_theme_battery_dark"), contentX, behaviorTop+sectionTitleH+sectionItemGap, 468, checkHeight)
	p.check(idThemeFullscreen, p.t("menu_theme_skip_fullscreen"), contentX, behaviorTop+sectionTitleH+sectionItemGap+checkHeight+functionGap, 468, checkHeight)

	// Application page.
	p.label(idAppGeneralTitle, p.t("settings_app_general_group"), p.sectionFont, contentX, sectionTop, 468, sectionTitleH)
	p.label(idLanguageLabel, p.t("settings_language"), p.font, contentX, 128, 220, 22)
	p.combo(idLanguage, []string{p.t("menu_lang_auto"), p.t("menu_lang_en"), p.t("menu_lang_zh")}, 456, 120, 220, fieldHeight)
	p.check(idHotkeys, p.t("menu_hotkeys"), contentX, 162, 468, checkHeight)
	p.check(idAutostart, p.t("menu_autostart"), contentX, 198, 468, checkHeight)
	p.check(idLogging, p.t("menu_logging"), contentX, 234, 468, checkHeight)
	p.label(idAppAboutTitle, p.t("settings_app_about_group"), p.sectionFont, contentX, 278, 468, sectionTitleH)
	projectLabel := p.t("settings_project_home_label")
	projectURL := "https://github.com/JeffioZ/IdleTrigger"
	labelWidth := p.logicalTextWidth(projectLabel, 96) + 2
	urlWidth := p.logicalTextWidth(projectURL, 376) + 2
	linkX := contentX + labelWidth + functionGap
	if p.state.Chinese {
		// GDI's CJK advance box includes more trailing whitespace than the
		// visible glyphs. Apply a small optical correction without narrowing
		// the label control itself, so the full-width colon is never clipped.
		linkX -= 10
	}
	p.label(idProjectHomeLabel, projectLabel, p.font, contentX, 310, labelWidth, 24)
	p.button(idProjectHome, projectURL, linkX, 304, min(urlWidth, contentRight-linkX), 30)

	// Screen notifications have their own page; keep the key choices together
	// and preview available even when automatic notifications are disabled.
	p.label(idNotificationsTitle, p.t("settings_lock_keys"), p.sectionFont, contentX, sectionTop, 468, sectionTitleH)
	p.check(idLockKeys, p.t("settings_lock_keys_enable"), contentX, 120, 468, checkHeight)
	p.check(idLockCaps, "Caps Lock", contentX+24, 156, 444, checkHeight)
	p.check(idLockNum, "Num Lock", contentX+24, 192, 444, checkHeight)
	p.check(idLockScroll, "Scroll Lock", contentX+24, 228, 444, checkHeight)
	p.label(idNotificationsBehavior, p.t("settings_notification_behavior"), p.sectionFont, contentX, 278, 468, sectionTitleH)
	p.check(idLockFullscreen, p.t("settings_notification_fullscreen"), contentX, 308, 468, checkHeight)
	p.label(idNotificationsHint, p.t("settings_notification_hint"), p.font, contentX, 348, 468, 42)
	p.button(idLockPreview, p.t("settings_notification_preview"), contentX, 406, 160, buttonHeight)

	footerY := contentHeight - nativeform.FormPadding - buttonHeight
	p.label(idValidation, "", p.font, contentX, footerY+8, 236, 24)
	p.button(idSave, p.t("common_save"), contentRight-dialogButtonWidth, footerY, dialogButtonWidth, buttonHeight)
	p.button(idCancel, p.t("common_cancel"), contentRight-2*dialogButtonWidth-nativeform.ControlGap, footerY, dialogButtonWidth, buttonHeight)

	p.checks[idKeepScreen] = p.state.KeepScreenOn
	p.checks[idBatteryAllowed] = p.state.NoSleepOnBattery
	p.checks[idIdleEnhanced] = p.state.IdleEnhancedMonitor
	p.checks[idThemeBattery] = p.state.ThemeDarkOnBattery
	p.checks[idThemeFullscreen] = p.state.ThemeSkipFullscreen
	p.checks[idLockKeys] = p.state.LockKeysEnabled
	p.checks[idLockCaps] = p.state.LockKeysCapsEnabled
	p.checks[idLockNum] = p.state.LockKeysNumEnabled
	p.checks[idLockScroll] = p.state.LockKeysScrollEnabled
	p.checks[idLockFullscreen] = p.state.LockKeysSkipFullscreen
	p.checks[idHotkeys] = p.state.HotkeysEnabled
	p.checks[idAutostart] = p.state.AutostartEnabled
	p.checks[idLogging] = p.state.LoggingEnabled
	for _, id := range checkIDs() {
		nativeform.UpdateCheckButtonAccessibility(p.controls[id], p.checks[id])
	}
	mode := 0
	if p.state.ThemeMode == "sunrise" {
		mode = 1
	}
	p.selectChoice(idThemeMode, mode)
	if p.state.ThemeIPLocationEnabled {
		p.selectChoice(idLocationSource, 1)
	} else {
		p.selectChoice(idLocationSource, 0)
	}
	p.selectChoice(idIdleAction, idleActionIndex(p.state.IdleAction))
	p.selectChoice(idLanguage, languageIndex(p.state.Language))
	p.annotateFields()
	p.createTooltips()
	p.page = 0
	p.applyDependentStates()
	return nil
}

var idleActionValues = func() []string {
	var values []string
	for index := 0; ; index++ {
		action, ok := config.IdleActionAt(index)
		if !ok {
			return values
		}
		values = append(values, string(action))
	}
}()

func idleActionLabels(text TextFunc) []string {
	labels := make([]string, len(idleActionValues))
	for index, action := range idleActionValues {
		labels[index] = text("menu_action_" + action)
	}
	return labels
}

func idleActionIndex(action string) int {
	for index, candidate := range idleActionValues {
		if action == candidate {
			return index
		}
	}
	return 1 // Sleep remains the default action.
}

func languageIndex(language string) int {
	if language == "en" {
		return 1
	}
	if language == "zh-CN" {
		return 2
	}
	return 0
}

func checkIDs() []uint16 {
	return []uint16{idKeepScreen, idBatteryAllowed, idIdleEnhanced, idThemeBattery, idThemeFullscreen, idHotkeys, idAutostart, idLogging, idLockKeys, idLockCaps, idLockNum, idLockScroll, idLockFullscreen}
}

func (p *panel) label(id uint16, value string, useFont windows.Handle, x, y, width, height int) {
	p.child("STATIC", value, wsChild|wsVisible|ssLeft, id, useFont, x, y, width, height)
}

func (p *panel) labelRight(id uint16, value string, useFont windows.Handle, x, y, width, height int) {
	p.child("STATIC", value, wsChild|wsVisible|2, id, useFont, x, y, width, height)
}

func (p *panel) logicalTextWidth(value string, fallback int) int {
	width := nativeform.TextWidth(p.hwnd, p.font, value)
	if width <= 0 {
		return fallback
	}
	return max(1, int(float64(width)/p.scale()+0.5))
}

func (p *panel) button(id uint16, value string, x, y, width, height int) {
	p.child("BUTTON", value, wsChild|wsVisible|wsTabStop|bsOwnerDraw, id, p.font, x, y, width, height)
	p.interaction.Track(p.controls[id], p.controls[id])
}

func (p *panel) check(id uint16, value string, x, y, width, height int) {
	if compactWidth := nativeform.CheckboxHitWidth(p.hwnd, p.font, value, p.scale()); compactWidth > 0 {
		width = min(width, compactWidth)
	}
	p.child("BUTTON", value, wsChild|wsVisible|wsTabStop|bsOwnerDraw, id, p.font, x, y, width, height)
	nativeform.AnnotateCheckButton(p.controls[id], value, false)
	p.interaction.TrackCheck(p.controls[id], p.controls[id], func() bool { return p.checks[id] })
}

func (p *panel) combo(id uint16, labels []string, x, y, width, height int) {
	p.child("BUTTON", "", wsChild|wsVisible|wsTabStop|bsOwnerDraw, id, p.font, x, y, width, height)
	p.choices[id] = &choice{labels: append([]string(nil), labels...)}
	p.interaction.Track(p.controls[id], p.controls[id])
}

func (p *panel) edit(id uint16, value string, x, y, width, height int) {
	p.editWithStyle(id, value, x, y, width, height, 0)
}

func (p *panel) numericEdit(id uint16, value string, x, y, width, height int) {
	p.editWithStyle(id, value, x, y, width, height, esNumber)
}

func (p *panel) editWithStyle(id uint16, value string, x, y, width, height int, extraStyle uintptr) {
	surfaceID := idFieldSurfaceBase + id
	p.child("STATIC", "", formSurfaceStyle|wsVisible, surfaceID, p.font, x, y, width, height)
	inner := p.child("EDIT", value, wsChild|wsVisible|wsTabStop|wsClipSiblings|esAutoHScroll|extraStyle, id, p.font, x+2, y+7, width-4, 20)
	_, _ = p.surfaces.Add(nativeform.ControlSurfaceOptions{ControlID: id, SurfaceID: surfaceID,
		Control: inner, Surface: p.controls[surfaceID], CueColor: p.palette.MutedText, Scale: p.scale(), Tracker: &p.interaction})
	margin := int(6*p.scale() + 0.5)
	pSendMessage.Call(uintptr(inner), emSetMargins, 3, uintptr(margin|(margin<<16)))
}

func (p *panel) child(className, value string, style uintptr, id uint16, useFont windows.Handle, x, y, width, height int) windows.Handle {
	class, _ := windows.UTF16PtrFromString(className)
	text, _ := windows.UTF16PtrFromString(value)
	scale := p.scale()
	hwnd, _, _ := pCreateWindowEx.Call(0, uintptr(unsafe.Pointer(class)), uintptr(unsafe.Pointer(text)), style,
		uintptr(int(float64(x)*scale)), uintptr(int(float64(y)*scale)), uintptr(max(1, int(float64(width)*scale))), uintptr(max(1, int(float64(height)*scale))),
		uintptr(p.hwnd), uintptr(id), 0, 0)
	if hwnd == 0 {
		return 0
	}
	handle := windows.Handle(hwnd)
	p.controls[id] = handle
	if id != 0 {
		p.hwndToControlID[handle] = id
	}
	p.labels[id] = value
	p.bounds[id] = bounds{x: x, y: y, width: width, height: height}
	if useFont != 0 {
		pSendMessage.Call(hwnd, wmSetFont, uintptr(useFont), 1)
	}
	nativeform.ApplyControl(handle, p.themeDark)
	return handle
}

func (p *panel) setText(id uint16, value string) {
	text, _ := windows.UTF16PtrFromString(value)
	pSetWindowText.Call(uintptr(p.controls[id]), uintptr(unsafe.Pointer(text)))
	p.labels[id] = value
	pInvalidateRect.Call(uintptr(p.controls[id]), 0, 0)
}

func (p *panel) controlText(id uint16) string {
	control := p.controls[id]
	length, _, _ := pSendMessage.Call(uintptr(control), wmGetTextLength, 0, 0)
	buffer := make([]uint16, int(length)+1)
	if len(buffer) > 0 {
		pSendMessage.Call(uintptr(control), wmGetText, uintptr(len(buffer)), uintptr(unsafe.Pointer(&buffer[0])))
	}
	return strings.TrimSpace(p.surfaces.LogicalText(control, windows.UTF16ToString(buffer)))
}

func (p *panel) selectChoice(id uint16, index int) {
	item := p.choices[id]
	if item == nil || len(item.labels) == 0 {
		return
	}
	index = max(0, min(index, len(item.labels)-1))
	item.selected = index
	p.setText(id, item.labels[index])
	if labelID := choiceLabelID(id); labelID != 0 {
		p.updateChoiceAccessibleName(id, labelID)
	}
}

func choiceLabelID(id uint16) uint16 {
	switch id {
	case idIdleAction:
		return idIdleActionLabel
	case idThemeMode:
		return idThemeModeLabel
	case idLocationSource:
		return idLocationLabel
	case idLanguage:
		return idLanguageLabel
	default:
		return 0
	}
}

func (p *panel) choiceIndex(id uint16) int {
	if item := p.choices[id]; item != nil {
		return item.selected
	}
	return 0
}

func (p *panel) setChecked(id uint16, checked bool) {
	p.checks[id] = checked
	nativeform.UpdateCheckButtonAccessibility(p.controls[id], checked)
	pInvalidateRect.Call(uintptr(p.controls[id]), 0, 0)
}

func (p *panel) setEnabled(id uint16, enabled bool) {
	flag := uintptr(0)
	if enabled {
		flag = 1
	}
	pEnableWindow.Call(uintptr(p.controls[id]), flag)
	if field, ok := p.surfaces.ForControl(id); ok {
		pInvalidateRect.Call(uintptr(field.Surface), 0, 0)
	}
	pInvalidateRect.Call(uintptr(p.controls[id]), 0, 0)
}

func (p *panel) applyDependentStates() {
	for page, ids := range pageControlIDs() {
		for _, id := range ids {
			p.setVisible(id, page == p.page)
		}
	}
	if p.page == 0 {
		enabled := p.checks[idBatteryAllowed]
		p.setEnabled(idBatteryThreshold, enabled)
		for _, id := range []uint16{idBatteryThresholdLabel} {
			pInvalidateRect.Call(uintptr(p.controls[id]), 0, 0)
		}
	}
	if p.page == 1 {
		sunrise := p.choiceIndex(idThemeMode) == 1
		for _, id := range []uint16{idLightTimeLabel, idLightTime, idDarkTimeLabel, idDarkTime} {
			p.setVisible(id, !sunrise)
		}
		for _, id := range []uint16{idLocationLabel, idLocationSource} {
			p.setVisible(id, sunrise)
		}
		p.setVisible(idThemeLocationStatus, sunrise)
		p.setVisible(idThemeHint, sunrise)
	}
	for _, id := range []uint16{idLockCaps, idLockNum, idLockScroll, idLockFullscreen} {
		p.setEnabled(id, p.checks[idLockKeys])
	}
	for _, id := range []uint16{idTabPower, idTabTheme, idTabApp, idTabNotifications} {
		pInvalidateRect.Call(uintptr(p.controls[id]), 0, 0)
	}
}

func pageControlIDs() map[int][]uint16 {
	return map[int][]uint16{
		0: {idPowerTitle, idKeepScreen, idBatteryAllowed, idBatteryThresholdLabel, idBatteryThreshold, idPowerHint, idIdleTitle,
			idIdleTimeoutLabel, idIdleTimeout, idIdleActionLabel, idIdleAction, idWarningLabel, idWarningSeconds, idIdleEnhanced},
		1: {idThemeScheduleTitle, idThemeBehaviorTitle, idThemeModeLabel, idThemeMode, idLightTimeLabel, idLightTime, idDarkTimeLabel, idDarkTime,
			idLocationLabel, idLocationSource, idThemeLocationStatus, idThemeBattery, idThemeFullscreen, idThemeHint},
		2: {idAppGeneralTitle, idAppAboutTitle, idLanguageLabel, idLanguage, idHotkeys, idAutostart, idLogging, idProjectHomeLabel, idProjectHome},
		3: {idNotificationsTitle, idLockKeys, idLockCaps, idLockNum, idLockScroll, idNotificationsBehavior, idLockFullscreen, idNotificationsHint, idLockPreview},
	}
}

func (p *panel) setVisible(id uint16, visible bool) {
	show := uintptr(0)
	if visible {
		show = 5
	}
	pShowWindow.Call(uintptr(p.controls[id]), show)
	if field, ok := p.surfaces.ForControl(id); ok {
		pShowWindow.Call(uintptr(field.Surface), show)
	}
}

func (p *panel) position(suggested *nativeform.Rect) {
	scale := p.scale()
	nativeform.PlaceWindow(nativeform.WindowPlacement{Window: p.hwnd, Anchor: p.hwnd, Owner: p.state.Owner,
		Style: p.style, ExStyle: p.exStyle, ClientWidth: int(windowWidth*scale + 0.5), ClientHeight: int(contentHeight*scale + 0.5),
		DPI: uint32(scale/max(1, p.textScale)*96 + 0.5), Suggested: suggested})
	p.syncViewport()
}

func (p *panel) syncViewport() {
	if p.viewport == nil {
		return
	}
	p.viewport.Sync(p.hwnd, p.scale(), windowWidth, contentHeight, p.palette)
	for id := range p.controls {
		if _, field := p.surfaces.ForControl(id); !field {
			p.positionControl(id)
		}
	}
	for id := range p.controls {
		if _, field := p.surfaces.ForControl(id); field {
			p.positionControl(id)
		}
	}
	pInvalidateRect.Call(uintptr(p.hwnd), 0, 0)
}
func (p *panel) positionControl(id uint16) {
	control := p.controls[id]
	b, ok := p.bounds[id]
	if !ok || control == 0 {
		return
	}
	if field, ok := p.surfaces.ForControl(id); ok {
		surfaceBounds := p.bounds[field.SurfaceID]
		p.positionHandle(field.Surface, surfaceBounds)
		innerHeight := min(20, surfaceBounds.height-4)
		inner := bounds{x: surfaceBounds.x + 2, y: surfaceBounds.y + (surfaceBounds.height-innerHeight)/2, width: surfaceBounds.width - 4, height: innerHeight}
		p.positionHandle(control, inner)
		// Keep the native EDIT above its owner-drawn field surface. PrintWindow
		// and some DPI transitions otherwise let the surface obscure its text.
		pSetWindowPos.Call(uintptr(control), 0, 0, 0, 0, 0, swpNoMove|swpNoSize|swpNoActivate)
		return
	}
	p.positionHandle(control, b)
}

func (p *panel) positionHandle(control windows.Handle, b bounds) {
	offsetX, offsetY := 0, 0
	if p.viewport != nil {
		offsetX, offsetY = p.viewport.X, p.viewport.Y
	}
	scale := p.scale()
	pSetWindowPos.Call(uintptr(control), 0, uintptr(int(float64(b.x-offsetX)*scale)), uintptr(int(float64(b.y-offsetY)*scale)),
		uintptr(max(1, int(float64(b.width)*scale))), uintptr(max(1, int(float64(b.height)*scale))), swpNoZOrder|swpNoActivate)
}

func (p *panel) scrollWheel(wParam uintptr) bool {
	return p.viewport != nil && p.viewport.Wheel(wParam)
}
