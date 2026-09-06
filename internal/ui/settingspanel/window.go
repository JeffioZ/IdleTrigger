package settingspanel

import (
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/ui/colors"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

func wndProc(hwnd windows.Handle, message uint32, wParam, lParam uintptr) uintptr {
	activeMu.Lock()
	p := active
	activeMu.Unlock()
	if p == nil || p.hwnd != hwnd {
		result, _, _ := pDefWindowProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
		return result
	}
	switch message {
	case wmClose:
		p.closeChoice(false)
		p.cancel()
		return 0
	case wmLButtonDown:
		p.interaction.SetFocusVisible(false)
		p.closeChoice(false)
	case wmSetCursor:
		if windows.Handle(wParam) == p.controls[idProjectHome] {
			if cursor, _, _ := pLoadCursor.Call(0, 32649); cursor != 0 {
				pSetCursor.Call(cursor)
				return 1
			}
		}
	case wmMouseWheel:
		if p.scrollWheel(wParam) {
			return 0
		}
	case wmCommand:
		p.handleCommand(uint16(wParam), uint16(wParam>>16))
		return 0
	case wmDrawItem:
		if p.drawOwnerItem((*drawItem)(nativeform.MessagePointer(lParam))) {
			return 1
		}
	case wmPaint:
		nativeform.PaintWindowBackground(hwnd, p.windowBrush)
		return 0
	case wmEraseBkgnd:
		var client rect
		pGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&client)))
		pFillRect.Call(wParam, uintptr(unsafe.Pointer(&client)), uintptr(p.windowBrush))
		return 1
	case wmCtlColorStatic:
		id := p.controlID(windows.Handle(lParam))
		textColor := p.palette.SecondaryText
		background := p.palette.WindowBackground
		brush := p.windowBrush
		if isSectionLabel(id) {
			textColor = p.palette.PrimaryText
		} else if isMutedLabel(id) {
			textColor = p.palette.MutedText
		}
		if id == idValidation && p.validationError {
			textColor = p.palette.DangerSurfaceText
		}
		if !p.checks[idBatteryAllowed] && id == idBatteryThresholdLabel {
			textColor = p.palette.DisabledText
		}
		if enabled, _, _ := pIsWindowEnabled.Call(lParam); enabled == 0 {
			textColor = p.palette.DisabledText
		}
		if _, field := p.surfaces.ForControl(id); field && !p.controlEnabled(id) {
			background = p.palette.DisabledSurface
			brush = p.disabledBrush
		}
		pSetTextColor.Call(wParam, uintptr(textColor))
		pSetBkColor.Call(wParam, uintptr(background))
		pSetBkMode.Call(wParam, opaque)
		return uintptr(brush)
	case wmCtlColorButton:
		pSetTextColor.Call(wParam, uintptr(p.palette.PrimaryText))
		pSetBkMode.Call(wParam, transparent)
		return uintptr(p.windowBrush)
	case wmCtlColorEdit:
		brush, textColor, background := p.surfaceBrush, p.palette.PrimaryText, p.palette.Surface
		if id := p.controlID(windows.Handle(lParam)); !p.controlEnabled(id) {
			brush, textColor, background = p.disabledBrush, p.palette.DisabledText, p.palette.DisabledSurface
		}
		pSetTextColor.Call(wParam, uintptr(textColor))
		pSetBkColor.Call(wParam, uintptr(background))
		return uintptr(brush)
	case wmSettingChange, wmSysColorChange, wmThemeChanged:
		if message == wmSettingChange && p.font != 0 {
			next := font.TextScaleFactor()
			if next != p.textScale {
				previous := p.textScale
				p.textScale = next
				if !p.rebuildForDPI() {
					p.textScale = previous
				} else {
					p.position(nil)
				}
			}
		}
		p.applyTheme()
		return 0
	case wmSize:
		p.syncViewport()
		return 0
	case wmDpiChanged:
		dpi := uint32(wParam & 0xffff)
		if dpi == 0 {
			dpi = 96
		}
		previousScale := p.dpiScale
		p.dpiScale = float64(dpi) / 96
		var suggested *nativeform.Rect
		if lParam != 0 {
			value := nativeform.Rect(*(*rect)(nativeform.MessagePointer(lParam)))
			suggested = &value
		}
		if p.rebuildForDPI() {
			p.position(suggested)
		} else {
			p.dpiScale = previousScale
		}
		return 0
	case wmDestroy:
		p.closeChoice(false)
		if p.viewport != nil {
			p.viewport.Close()
			p.viewport = nil
		}
		p.surfaces.Close()
		p.tooltip = 0
		p.tooltipText = nil
		trayicon.ClearTabNavigationWindow(hwnd)
		for _, handle := range []windows.Handle{p.font, p.sectionFont, p.titleFont} {
			if handle != 0 {
				pDeleteObject.Call(uintptr(handle))
			}
		}
		p.releaseBrushes()
		p.icons.Release()
		if p.ownerDisabled && p.state.Owner != 0 {
			if valid, _, _ := pIsWindow.Call(uintptr(p.state.Owner)); valid != 0 {
				pEnableWindow.Call(uintptr(p.state.Owner), 1)
				pSetForeground.Call(uintptr(p.state.Owner))
			}
		}
		p.hwnd = 0
		clearActive(p)
		return 0
	}
	result, _, _ := pDefWindowProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
	return result
}

func (p *panel) controlID(hwnd windows.Handle) uint16 {
	return p.hwndToControlID[hwnd]
}

func isSectionLabel(id uint16) bool {
	switch id {
	case idTitle, idPowerTitle, idIdleTitle, idThemeScheduleTitle,
		idThemeBehaviorTitle, idAppGeneralTitle, idAppAboutTitle, idNotificationsTitle, idNotificationsBehavior:
		return true
	default:
		return false
	}
}

func isMutedLabel(id uint16) bool {
	return id == idDescription || id == idVersion || id == idPowerHint || id == idThemeHint || id == idThemeLocationStatus || id == idValidation || id == idNotificationsHint
}

func (p *panel) drawOwnerItem(item *drawItem) bool {
	if item == nil || item.HDC == 0 {
		return false
	}
	bounds := nativeform.Rect{Left: item.Rect.Left, Top: item.Rect.Top, Right: item.Rect.Right, Bottom: item.Rect.Bottom}
	if nativeform.DrawBuffered(item.HDC, bounds, func(dc windows.Handle, local nativeform.Rect) {
		copy := *item
		copy.HDC = dc
		copy.Rect = rect(local)
		p.drawOwnerItemDirect(&copy)
	}) {
		return true
	}
	return p.drawOwnerItemDirect(item)
}

func (p *panel) drawOwnerItemDirect(item *drawItem) bool {
	id := uint16(item.CtlID)
	bounds := nativeform.Rect{Left: item.Rect.Left, Top: item.Rect.Top, Right: item.Rect.Right, Bottom: item.Rect.Bottom}
	radius := max(int32(3), int32(6*p.scale()+0.5))
	if field, ok := p.surfaces.ForSurface(id); ok {
		interaction := p.interaction.State(field.Control)
		nativeform.DrawField(item.HDC, bounds, p.palette, p.palette.WindowBackground, nativeform.ControlState{
			Hovered: interaction.Hovered, Focused: interaction.FocusVisible, Disabled: !p.controlEnabled(field.ControlID),
		}, radius)
		return true
	}
	control := p.controls[id]
	if control == 0 {
		return false
	}
	interaction := p.interaction.State(control)
	state := nativeform.ControlState{Hovered: interaction.Hovered, Pressed: interaction.Pressed || item.ItemState&odsSelected != 0,
		Focused: interaction.FocusVisible, Disabled: item.ItemState&odsDisabled != 0 || !p.controlEnabled(id)}
	if containsID(checkIDs(), id) {
		state.Active = p.checks[id]
		nativeform.DrawCheckbox(item.HDC, bounds, p.font, p.labels[id], p.palette, p.palette.WindowBackground, state, p.scale())
		return true
	}
	if id == idTabPower || id == idTabTheme || id == idTabApp || id == idTabNotifications {
		state.Active = id == []uint16{idTabPower, idTabTheme, idTabApp, idTabNotifications}[p.page]
	}
	if id == idSave {
		state.Active = true
	}
	if id == idProjectHome {
		nativeform.DrawTextLink(item.HDC, bounds, p.font, p.labels[id], p.palette, p.palette.WindowBackground, state, p.scale())
		return true
	}
	if _, ok := p.choices[id]; ok {
		state.Open = p.choiceOpen == id
		nativeform.DrawChoice(item.HDC, bounds, p.font, p.labels[id], p.palette, p.palette.WindowBackground, state, radius, p.scale())
		return true
	}
	nativeform.DrawButton(item.HDC, bounds, p.font, p.labels[id], p.palette, p.palette.WindowBackground, state, radius, false)
	return true
}

func containsID(ids []uint16, target uint16) bool {
	for _, id := range ids {
		if id == target {
			return true
		}
	}
	return false
}

func (p *panel) controlEnabled(id uint16) bool {
	enabled, _, _ := pIsWindowEnabled.Call(uintptr(p.controls[id]))
	return enabled != 0
}

func (p *panel) applyTheme() {
	p.closeChoice(false)
	p.themeDark = currentThemeDark()
	if p.themeOverride != nil {
		p.themeDark = *p.themeOverride
	}
	p.palette = colors.ForTheme(p.themeDark)
	p.releaseBrushes()
	p.windowBrush = makeBrush(p.palette.WindowBackground)
	p.surfaceBrush = makeBrush(p.palette.Surface)
	p.disabledBrush = makeBrush(p.palette.DisabledSurface)
	nativeform.ApplyFrame(p.hwnd, p.themeDark)
	scale := p.scale()
	p.icons.Apply(p.hwnd, p.themeDark, int(32*scale+0.5), int(16*scale+0.5), false)
	for _, control := range p.controls {
		nativeform.ApplyControl(control, p.themeDark)
		pInvalidateRect.Call(uintptr(control), 0, 0)
	}
	p.surfaces.SetCueTheme(p.palette.MutedText)
	if p.tooltip != 0 {
		pSendMessage.Call(uintptr(p.tooltip), wmSetFont, uintptr(p.font), 0)
		nativeform.ApplyTooltip(p.tooltip, p.themeDark, p.palette, p.font)
	}
	if p.viewport != nil {
		p.syncViewport()
	}
	pInvalidateRect.Call(uintptr(p.hwnd), 0, 0)
}

func makeBrush(color uint32) windows.Handle {
	value, _, _ := pCreateBrush.Call(uintptr(color))
	return windows.Handle(value)
}

func (p *panel) releaseBrushes() {
	for _, brush := range []windows.Handle{p.windowBrush, p.surfaceBrush, p.disabledBrush} {
		if brush != 0 {
			pDeleteObject.Call(uintptr(brush))
		}
	}
	p.windowBrush, p.surfaceBrush, p.disabledBrush = 0, 0, 0
}

func (p *panel) rebuildForDPI() bool {
	newFont, _ := font.NewForLayout(int32(14*p.scale()+0.5), 400, p.state.Chinese)
	newSection, _ := font.NewForLayout(int32(14*p.scale()+0.5), 600, p.state.Chinese)
	newTitle, _ := font.NewForLayout(int32(17*p.scale()+0.5), 600, p.state.Chinese)
	if newFont == 0 || newSection == 0 || newTitle == 0 {
		if newFont != 0 {
			pDeleteObject.Call(uintptr(newFont))
		}
		if newSection != 0 {
			pDeleteObject.Call(uintptr(newSection))
		}
		if newTitle != 0 {
			pDeleteObject.Call(uintptr(newTitle))
		}
		return false
	}
	p.closeChoice(false)
	oldFont, oldSection, oldTitle := p.font, p.sectionFont, p.titleFont
	p.font, p.sectionFont, p.titleFont = newFont, newSection, newTitle
	for id, control := range p.controls {
		useFont := p.font
		if isSectionLabel(id) {
			useFont = p.sectionFont
		}
		if id == idTitle {
			useFont = p.titleFont
		}
		pSendMessage.Call(uintptr(control), wmSetFont, uintptr(useFont), 0)
	}
	p.surfaces.SetScale(p.scale())
	if p.tooltip != 0 {
		pSendMessage.Call(uintptr(p.tooltip), wmSetFont, uintptr(p.font), 0)
		pSendMessage.Call(uintptr(p.tooltip), ttmSetMaxTipWidth, 0, uintptr(int(380*p.scale())))
	}
	margin := int(6*p.scale() + 0.5)
	for _, id := range []uint16{idBatteryThreshold, idIdleTimeout, idWarningSeconds, idLightTime, idDarkTime} {
		pSendMessage.Call(uintptr(p.controls[id]), emSetMargins, 3, uintptr(margin|(margin<<16)))
	}
	p.syncViewport()
	if oldFont != 0 {
		pDeleteObject.Call(uintptr(oldFont))
	}
	if oldSection != 0 {
		pDeleteObject.Call(uintptr(oldSection))
	}
	if oldTitle != 0 {
		pDeleteObject.Call(uintptr(oldTitle))
	}
	return true
}
