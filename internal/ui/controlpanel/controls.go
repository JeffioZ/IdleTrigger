package controlpanel

import (
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"golang.org/x/sys/windows"
	"unsafe"
)

func (p *panel) build() error {
	layout := p.metrics.style.Layout
	baseW, pad, gap := layout.PanelWidth, layout.Padding, layout.Gap
	sectionGap, labelGap := layout.SectionGap, layout.LabelGap
	buttonH, sectionH, subtitleH := layout.ButtonHeight, layout.SectionHeight, layout.SubtitleHeight
	section := func(text string, y int) error {
		id := p.staticID(staticSection)
		_, err := p.child("STATIC", text, wsChild|wsVisible|ssOwnerDraw, pad, y, baseW-2*pad, sectionH, id, 0)
		return err
	}
	subtitle := func(text string, x, y, width int) error {
		id := p.staticID(staticSubtitle)
		_, err := p.child("STATIC", text, wsChild|wsVisible|ssOwnerDraw, x, y, width, subtitleH, id, 0)
		return err
	}
	button := func(text string, x, y, width, height int, id uint16) error {
		hwnd, err := p.child("BUTTON", text, wsChild|wsVisible|wsTabStop|bsOwnerDraw, x, y, width, height, id, p.font)
		if err != nil {
			return err
		}
		p.subclassButton(hwnd)
		if roleForButton(id) == buttonToggle {
			nativeform.AnnotateCheckButton(hwnd, text, p.toggles[id])
		}
		return err
	}
	choice := func(id uint16, x, y, width, height int, options []string, current int) error {
		if err := button(options[current], x, y, width, height, id); err != nil {
			return err
		}
		p.choice.options[id] = append([]string(nil), options...)
		p.choice.selected[id] = current
		p.labels[id] = options[current]
		return nil
	}
	choiceRow := func(x, y, totalW int, labels []string, ids []uint16) (int, error) {
		width := splitRow(totalW, len(ids), gap)
		height := p.rowHeight(labels, width)
		for i, id := range ids {
			if err := button(labels[i], x+i*(width+gap), y, width, height, id); err != nil {
				return 0, err
			}
		}
		return height, nil
	}
	choiceRowWidths := func(x, y int, labels []string, ids, widths []uint16) (int, error) {
		height := buttonH
		for i, width := range widths {
			if candidate := p.rowHeight([]string{labels[i]}, int(width)); candidate > height {
				height = candidate
			}
		}
		for i, id := range ids {
			if err := button(labels[i], x, y, int(widths[i]), height, id); err != nil {
				return 0, err
			}
			x += int(widths[i]) + gap
		}
		return height, nil
	}
	y := pad
	if err := section(p.text("menu_power_management"), y); err != nil {
		return err
	}
	y += sectionH + labelGap
	height, err := choiceRow(pad, y, baseW-2*pad, []string{p.text("menu_nosleep_enable"), p.text("menu_idle_enable")}, []uint16{idNoSleep, idIdle})
	if err != nil {
		return err
	}
	y += height + 4
	height, err = choiceRow(pad, y, baseW-2*pad, []string{p.text("menu_idle_warning"), p.text("menu_idle_enhanced")}, []uint16{idIdleWarning, idIdleEnhanced})
	if err != nil {
		return err
	}
	y += height + gap
	childX := pad
	childW := baseW - 2*pad
	fieldW := (childW - gap) / 2
	if p.developerWarningPreview {
		if err := button(p.text("msg_idle_warning_test"), childX, y, fieldW, buttonH, idTestWarning); err != nil {
			return err
		}
		y += buttonH + gap
	}
	if err := subtitle(p.text("menu_idle_timeout"), childX, y, fieldW); err != nil {
		return err
	}
	secondX := childX + fieldW + gap
	if err := subtitle(p.text("menu_idle_action"), secondX, y, fieldW); err != nil {
		return err
	}
	y += subtitleH + labelGap
	if err := choice(idIdleTimeout, childX, y, fieldW, buttonH, timeoutLabels(p.idleTimeout, p.isChinese, &p.timeoutOptions), timeoutIndex(p.timeoutOptions, p.idleTimeout)); err != nil {
		return err
	}
	if err := choice(idIdleAction, secondX, y, fieldW, buttonH, actionLabels(p), actionIndex(p.idleAction)); err != nil {
		return err
	}
	y += buttonH + sectionGap
	if err := section(p.text("menu_automation_section"), y); err != nil {
		return err
	}
	y += sectionH + labelGap
	automationSummaryID := p.staticID(staticSubtitle)
	p.automationSummaryID = automationSummaryID
	if _, err := p.child("STATIC", p.automationSummary, wsChild|wsVisible|ssOwnerDraw, pad, y, baseW-2*pad, subtitleH, automationSummaryID, 0); err != nil {
		return err
	}
	y += subtitleH + labelGap
	if _, err = choiceRow(pad, y, baseW-2*pad, []string{p.text("automation_master"), p.text("menu_automation_manage")}, []uint16{idAutomationEnabled, idAutomation}); err != nil {
		return err
	}
	y += buttonH + sectionGap
	if err := section(p.text("menu_theme_switch"), y); err != nil {
		return err
	}
	y += sectionH + labelGap
	if p.themeSchedule != "" {
		id := p.staticID(staticSubtitle)
		p.themeScheduleID = id
		if _, err := p.child("STATIC", p.themeSchedule, wsChild|wsVisible|ssOwnerDraw, pad, y, baseW-2*pad, subtitleH, id, 0); err != nil {
			return err
		}
		y += subtitleH + labelGap
	}
	height, err = choiceRow(pad, y, baseW-2*pad, []string{p.text("menu_theme_enable"), p.text("menu_theme_skip_fullscreen")}, []uint16{idTheme, idFullscreen})
	if err != nil {
		return err
	}
	y += height + 4
	height, err = choiceRow(pad, y, baseW-2*pad, []string{p.text("menu_theme_battery_dark"), p.text("menu_theme_ip_location")}, []uint16{idBattery, idIPLocation})
	if err != nil {
		return err
	}
	y += height + gap
	height, err = choiceRow(pad, y, baseW-2*pad, []string{p.text("menu_theme_switch_now"), p.text("menu_theme_repair")}, []uint16{idThemeSwitch, idThemeRepair})
	if err != nil {
		return err
	}
	y += height + sectionGap
	if err := section(p.text("menu_preferences"), y); err != nil {
		return err
	}
	y += sectionH + labelGap
	generalLabels := []string{p.text("menu_hotkeys"), p.text("menu_autostart"), p.text("menu_logging")}
	generalIDs := []uint16{idHotkeys, idAutostart, idLogging}
	if p.isChinese {
		height, err = choiceRow(pad, y, baseW-2*pad, generalLabels, generalIDs)
	} else {
		// English labels are shorter than their Chinese counterparts. Keep this
		// row compact so the final two toggles read as one related group.
		height, err = choiceRowWidths(pad, y, generalLabels, generalIDs, []uint16{160, 126, 126})
	}
	if err != nil {
		return err
	}
	y += height + sectionGap
	bottomW := (baseW - 2*pad - gap) / 2
	bottomRow1 := []string{p.text("menu_system_controls"), p.text("menu_language_settings")}
	bottomRow2 := []string{p.text("menu_open_config"), p.text("menu_exit_panel")}
	bottomH := p.rowHeight(append(bottomRow1, bottomRow2...), bottomW)
	// Project Home is deliberately a compact text link, not a third command
	// button. Its hit target follows the text instead of spanning the panel,
	// so adjacent whitespace remains neutral.
	projectHomeH := 24
	projectHomeY := y + bottomH*2 + gap
	projectHomeW := p.textLinkWidth(p.text("menu_project_home"))
	projectHomeX := pad + (baseW-2*pad-projectHomeW)/2
	p.clientH = projectHomeY + projectHomeH + gap
	if _, err = choiceRow(pad, y, baseW-2*pad, bottomRow1, []uint16{idQuickActions, idLanguage}); err != nil {
		return err
	}
	if _, err = choiceRow(pad, y+bottomH+gap, baseW-2*pad, bottomRow2, []uint16{idConfig, idExit}); err != nil {
		return err
	}
	if err := button(p.text("menu_project_home"), projectHomeX, projectHomeY, projectHomeW, projectHomeH, idProjectHome); err != nil {
		return err
	}
	p.applyDependentStates()
	return nil
}

func quickActionTranslationKey(id uint16) string {
	switch id {
	case idLock:
		return "menu_lock"
	case idSleep:
		return "menu_sleep"
	case idHibernate:
		return "menu_hibernate"
	case idShutdown:
		return "menu_shutdown"
	default:
		return "menu_restart"
	}
}

func (p *panel) staticID(kind staticKind) uint16 {
	id := p.nextStaticID
	p.nextStaticID++
	p.staticKinds[id] = kind
	return id
}

func (p *panel) rowHeight(labels []string, width int) int {
	if p.hwnd == 0 || p.font == 0 {
		return p.metrics.style.Layout.ButtonHeight
	}
	dc, _, _ := pGetDC.Call(uintptr(p.hwnd))
	if dc == 0 {
		return p.metrics.style.Layout.ButtonHeight
	}
	defer pReleaseDC.Call(uintptr(p.hwnd), dc)
	old, _, _ := pSelectObject.Call(dc, uintptr(p.font))
	defer pSelectObject.Call(dc, old)

	rowH := p.metrics.style.Layout.ButtonHeight
	availableW := int32(p.sc(width - 16))
	for _, label := range labels {
		text, err := windows.UTF16PtrFromString(label)
		if err != nil {
			continue
		}
		bounds := rect{Right: availableW}
		pDrawText.Call(dc, uintptr(unsafe.Pointer(text)), ^uintptr(0), uintptr(unsafe.Pointer(&bounds)), dtCenter|dtWordBreak|dtCalcRect)
		textH := int(float64(bounds.Bottom-bounds.Top)/p.metrics.scale + 0.999)
		if candidate := textH + 12; candidate > rowH {
			rowH = candidate
		}
	}
	return rowH
}

// textLinkWidth keeps the native link's pointer/focus target exactly aligned
// with its localized label, so surrounding whitespace remains noninteractive.
func (p *panel) textLinkWidth(label string) int {
	if p.hwnd == 0 || p.font == 0 {
		return 160
	}
	dc, _, _ := pGetDC.Call(uintptr(p.hwnd))
	if dc == 0 {
		return 160
	}
	defer pReleaseDC.Call(uintptr(p.hwnd), dc)
	old, _, _ := pSelectObject.Call(dc, uintptr(p.font))
	defer pSelectObject.Call(dc, old)
	text, err := windows.UTF16PtrFromString(label)
	if err != nil {
		return 160
	}
	bounds := rect{}
	pDrawText.Call(dc, uintptr(unsafe.Pointer(text)), ^uintptr(0), uintptr(unsafe.Pointer(&bounds)), dtSingleLine|dtCalcRect)
	width := int(float64(bounds.Right-bounds.Left)/p.metrics.scale + 0.999)
	return width
}

// projectHomeTextVerticalOffset compensates for the visibly asymmetric font
// metrics in this compact link row. Microsoft YaHei UI needs one more logical
// pixel than the English font chain to appear centered with the hover underline.
func (p *panel) projectHomeTextVerticalOffset() int {
	if p.isChinese {
		return p.sc(3)
	}
	return p.sc(2)
}

func roleForButton(id uint16) buttonRole {
	switch id {
	case idNoSleep, idAutomationEnabled, idIdle, idIdleWarning, idIdleEnhanced,
		idTheme, idBattery, idFullscreen, idIPLocation, idHotkeys, idAutostart, idLogging:
		return buttonToggle
	case idLangEN, idLangZH:
		return buttonChoice
	default:
		return buttonCommand
	}
}

func visualStateForButton(id uint16, toggleOn, choiceSelected, disabled bool) buttonVisualState {
	role := roleForButton(id)
	active := false
	switch role {
	case buttonToggle:
		active = toggleOn
	case buttonChoice:
		active = choiceSelected
	}
	return buttonVisualState{Role: role, Active: active, Disabled: disabled}
}

func (p *panel) visualState(id uint16) buttonVisualState {
	return visualStateForButton(id, p.toggles[id], p.selected[id], p.disabled[id])
}

// controlState is the single source for owner-drawn button states. The
// semantic state comes from the panel model. Hover uses the panel's explicit
// mouse tracker as its only source: native ODS_HOTLIGHT can lag one control
// behind during fast pointer movement and make two owner-drawn controls flash.
func (p *panel) controlState(id uint16, itemState uint32) buttonVisualState {
	state := p.visualState(id)
	state.Disabled = state.Disabled || itemState&odsDisabled != 0
	state.Hovered = p.hoverID == id
	state.Pressed = itemState&odsSelected != 0
	state.Focused = p.shouldDrawFocusOutline(itemState)
	return state
}

func isMenuTrigger(id uint16) bool { return id == idQuickActions || id == idLanguage }

func actionTranslationKey(value string) string {
	switch value {
	case "sleep", "hibernate", "shutdown", "lock":
		return "menu_action_" + value
	default:
		return "menu_action_sleep"
	}
}

func (p *panel) setChoice(group []uint16, selected uint16) {
	for _, id := range group {
		p.selected[id] = id == selected
	}
}
func (p *panel) toggle(id uint16) {
	p.toggles[id] = !p.toggles[id]
	p.updateToggleAccessibility(id)
	p.refreshTooltip(id)
	p.invalidate(id)
}
func (p *panel) setToggle(id uint16, value bool) {
	p.toggles[id] = value
	p.updateToggleAccessibility(id)
	p.refreshTooltip(id)
	p.invalidate(id)
}

func (p *panel) updateToggleAccessibility(id uint16) {
	if roleForButton(id) == buttonToggle {
		nativeform.UpdateCheckButtonAccessibility(p.controls[id], p.toggles[id])
	}
}
func (p *panel) choose(group []uint16, selected uint16) {
	p.setChoice(group, selected)
	for _, id := range group {
		p.refreshTooltip(id)
		p.invalidate(id)
	}
}
func (p *panel) invalidate(id uint16) {
	if hwnd := p.controls[id]; hwnd != 0 {
		// Every panel control is owner-drawn and covers its complete client area.
		// Do not expose an erased intermediate frame when rapid hover transitions
		// invalidate adjacent controls before WM_DRAWITEM coalesces their paints.
		pInvalidateRect.Call(uintptr(hwnd), 0, 0)
	}
}
