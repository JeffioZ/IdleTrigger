package controlpanel

import "github.com/JeffioZ/idletrigger/internal/ui/nativeform"

func (p *panel) build() error {
	layout := p.metrics.style.Layout
	baseW, pad, gap := layout.PanelWidth, layout.Padding, layout.Gap
	sectionGap, labelGap := layout.SectionGap, layout.LabelGap
	sectionH, subtitleH := layout.SectionHeight, layout.SubtitleHeight
	section := func(text string, y int) error {
		id := p.staticID(staticSection)
		_, err := p.child("STATIC", text, wsChild|wsVisible|ssOwnerDraw, pad, y, baseW-2*pad, sectionH, id, 0)
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
		return nil
	}
	row := func(y int, labels []string, ids []uint16) (int, error) {
		width := splitRow(baseW-2*pad, len(ids), gap)
		height := p.rowHeight(labels, width)
		for index, id := range ids {
			controlWidth := width
			if roleForButton(id) == buttonToggle {
				if compactWidth := nativeform.CheckboxHitWidth(p.hwnd, p.font, labels[index], p.metrics.scale); compactWidth > 0 {
					controlWidth = min(width, compactWidth)
				}
			}
			if err := button(labels[index], pad+index*(width+gap), y, controlWidth, height, id); err != nil {
				return 0, err
			}
		}
		return height, nil
	}
	themeRow := func(y int) (int, error) {
		labels := []string{p.text("menu_theme_enable"), p.themeActionLabel(idThemeSwitch), p.themeActionLabel(idThemeRepair)}
		const minimumToggleWidth = 154
		totalWidth := baseW - 2*pad
		toggleWidth := minimumToggleWidth
		if measured := nativeform.CheckboxHitWidth(p.hwnd, p.font, labels[0], p.metrics.scale); measured > toggleWidth {
			toggleWidth = measured
		}
		// Keep the two direct theme actions equal while reserving enough width
		// for the full checkbox label in the compact control panel.
		actionWidth := (totalWidth - toggleWidth - 2*gap) / 2
		height := p.rowHeight(labels[1:], actionWidth)
		if err := button(labels[0], pad, y, toggleWidth, height, idTheme); err != nil {
			return 0, err
		}
		if err := button(labels[1], pad+toggleWidth+gap, y, actionWidth, height, idThemeSwitch); err != nil {
			return 0, err
		}
		if err := button(labels[2], pad+toggleWidth+gap+actionWidth+gap, y, actionWidth, height, idThemeRepair); err != nil {
			return 0, err
		}
		return height, nil
	}

	y := pad
	if err := section(p.text("menu_power_management"), y); err != nil {
		return err
	}
	y += sectionH + labelGap
	height, err := row(y, []string{p.text("menu_nosleep_enable"), p.text("menu_idle_enable")}, []uint16{idNoSleep, idIdle})
	if err != nil {
		return err
	}
	y += height + labelGap
	p.powerSummaryID = p.staticID(staticSubtitle)
	if _, err := p.child("STATIC", p.text("power_overview_prefix")+p.noSleepStatus+p.text("power_overview_separator")+p.idleStatus,
		wsChild|wsVisible|ssOwnerDraw, pad, y, baseW-2*pad, subtitleH, p.powerSummaryID, 0); err != nil {
		return err
	}
	y += subtitleH
	if p.developerWarningPreview {
		y += labelGap
		previewWidth := splitRow(baseW-2*pad, 2, gap)
		if err := button(p.text("msg_idle_warning_test"), pad, y, previewWidth, layout.ButtonHeight, idTestWarning); err != nil {
			return err
		}
		y += layout.ButtonHeight
	}
	y += sectionGap

	if err := section(p.text("menu_automation_section"), y); err != nil {
		return err
	}
	y += sectionH + labelGap
	height, err = row(y, []string{p.text("automation_master"), p.text("menu_automation_manage")}, []uint16{idAutomationEnabled, idAutomation})
	if err != nil {
		return err
	}
	y += height + labelGap
	p.automationSummaryID = p.staticID(staticSubtitle)
	if _, err := p.child("STATIC", p.automationSummary, wsChild|wsVisible|ssOwnerDraw, pad, y, baseW-2*pad, subtitleH, p.automationSummaryID, 0); err != nil {
		return err
	}
	y += subtitleH + sectionGap

	if err := section(p.text("menu_theme_switch"), y); err != nil {
		return err
	}
	y += sectionH + labelGap
	height, err = themeRow(y)
	if err != nil {
		return err
	}
	y += height + labelGap
	if p.themeSchedule != "" {
		p.themeScheduleID = p.staticID(staticSubtitle)
		if _, err := p.child("STATIC", p.themeSchedule, wsChild|wsVisible|ssOwnerDraw, pad, y, baseW-2*pad, subtitleH, p.themeScheduleID, 0); err != nil {
			return err
		}
		y += subtitleH
	}
	y += sectionGap
	height, err = row(y, []string{p.text("menu_system_controls"), p.text("settings_open"), p.text("menu_exit_panel")}, []uint16{idQuickActions, idSettings, idExit})
	if err != nil {
		return err
	}
	p.clientH = y + height + gap
	p.applyDependentStates()
	return nil
}

func (p *panel) themeActionLabel(id uint16) string {
	if p.themeOperationBusy {
		if id == idThemeRepair {
			return p.text("menu_theme_repairing")
		}
		return p.text("menu_theme_switching")
	}
	if id == idThemeRepair {
		return p.text("menu_theme_repair")
	}
	return p.text("menu_theme_switch_now")
}
