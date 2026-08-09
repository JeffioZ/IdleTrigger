package automationpanel

import (
	"fmt"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
)

const (
	managerPad               = 18
	managerButtonGap         = 8
	managerCompactButtonBase = 116*3 + 192
	managerTitleY            = formEdgePadding
	managerListY             = managerTitleY + formTextHeight + formContentGap
	managerListHeight        = 240
	managerStatusY           = managerListY + managerListHeight + formRelatedGap
	managerButtonsY          = managerStatusY + formTextHeight + formSectionGap
	managerEmptyY            = managerListY + (managerListHeight-(2*formTextHeight+formContentGap))/2
)

func (p *panel) showManager() {
	p.beginRebuild()
	defer func() {
		p.endRebuild()
		if !p.captureHost {
			target := p.controls[idList]
			if len(p.rules) == 0 {
				target = p.controls[idNew]
			}
			if target != 0 {
				pSetFocus.Call(uintptr(target))
			}
		}
	}()
	p.closeChoice(false)
	p.hideControls(editorControlIDs())
	if p.view != managerView {
		p.contentOffset = 0
	}
	p.view = managerView
	p.setCaption(p.t("automation_title"))
	p.resize(managerWidth, managerHeight)
	if !p.managerReady {
		p.child("STATIC", p.t("automation_rules_title"), wsChild|wsVisible|ssLeft, 18, managerTitleY, 564, formTextHeight, idTitle, p.sectionFont)
		surface := p.child("STATIC", "", formSurfaceStyle|wsVisible, 18, managerListY, 564, managerListHeight, idListSurface, p.font)
		pSetWindowPos.Call(uintptr(surface), 1, 0, 0, 0, 0, swpNoMove|swpNoSize|swpNoActivate)
		list := p.child("LISTBOX", "", wsChild|wsVisible|wsTabStop|wsVScroll|wsClipSiblings|lbsNotify|lbsNoIntegralHeight, 20, managerListY+2, 560, managerListHeight-4, idList, p.font)
		if scrollbar, err := nativeform.NewListboxScrollbar(nativeform.ListboxScrollbarOptions{
			Parent: p.hwnd, Listbox: list, Palette: p.palette, Background: p.palette.Surface, Scale: p.scale(),
		}); err == nil {
			p.managerScroll = scrollbar
			p.syncManagerScrollbarBounds()
		}
		p.child("STATIC", p.t("automation_empty_title"), wsChild|ssLeft, 42, managerEmptyY, 516, formTextHeight, idEmptyTitle, p.sectionFont)
		p.child("STATIC", p.t("automation_empty_body"), wsChild|ssLeft, 42, managerEmptyY+formTextHeight+formContentGap, 516, formTextHeight, idEmptyBody, p.font)
		p.child("STATIC", p.managerStatusText(), wsChild|wsVisible|ssLeft, 18, managerStatusY, 564, formTextHeight, idNext, p.font)
		p.child("BUTTON", p.t("automation_new"), wsChild|wsVisible|wsTabStop|bsOwnerDraw, 18, managerButtonsY, 116, nativeform.ButtonHeight, idNew, p.font)
		p.child("BUTTON", p.t("automation_edit"), wsChild|wsVisible|wsTabStop|bsOwnerDraw, 142, managerButtonsY, 116, nativeform.ButtonHeight, idEdit, p.font)
		p.child("BUTTON", p.t("automation_delete"), wsChild|wsVisible|wsTabStop|bsOwnerDraw, 266, managerButtonsY, 116, nativeform.ButtonHeight, idDelete, p.font)
		p.child("BUTTON", p.t("automation_toggle"), wsChild|wsVisible|wsTabStop|bsOwnerDraw, 390, managerButtonsY, 192, nativeform.ButtonHeight, idToggle, p.font)
		pSetWindowPos.Call(uintptr(p.controls[idList]), 0, 0, 0, 0, 0, swpNoMove|swpNoSize|swpNoActivate)
		for id, key := range map[uint16]string{idList: "tip_automation_list", idNew: "tip_automation_new", idEdit: "tip_automation_edit", idToggle: "tip_automation_toggle", idDelete: "tip_automation_delete"} {
			p.addTooltip(id, key)
		}
		p.managerReady = true
	} else {
		p.showControls(managerControlIDs())
		p.setText(idNext, p.managerStatusText())
	}
	p.layoutManager()
	if p.managerScroll != nil {
		p.managerScroll.SetActive(true)
	}
	p.populateRules()
}

func (p *panel) layoutManager() {
	if !p.managerReady {
		return
	}
	contentWidth, standardWidth, toggleWidth := managerLayoutWidths(p.viewportWidth, p.clientHeight > p.viewportHeight)
	p.setManagerBounds(idTitle, managerPad, managerTitleY, contentWidth, formTextHeight)
	p.setManagerBounds(idListSurface, managerPad, managerListY, contentWidth, managerListHeight)
	p.setManagerBounds(idList, managerPad+2, managerListY+2, max(1, contentWidth-4), managerListHeight-4)
	p.setManagerBounds(idEmptyTitle, managerPad+24, managerEmptyY, max(1, contentWidth-48), formTextHeight)
	p.setManagerBounds(idEmptyBody, managerPad+24, managerEmptyY+formTextHeight+formContentGap, max(1, contentWidth-48), formTextHeight)
	p.setManagerBounds(idNext, managerPad, managerStatusY, contentWidth, formTextHeight)

	x := managerPad
	for _, button := range []struct {
		id    uint16
		width int
	}{{idNew, standardWidth}, {idEdit, standardWidth}, {idDelete, standardWidth}, {idToggle, toggleWidth}} {
		p.setManagerBounds(button.id, x, managerButtonsY, button.width, nativeform.ButtonHeight)
		x += button.width + managerButtonGap
	}
	p.syncManagerScrollbarBounds()
}

func managerLayoutWidths(viewportWidth int, needsScrollbar bool) (contentWidth, standardWidth, toggleWidth int) {
	layoutWidth := min(managerWidth, max(1, viewportWidth))
	reserve := 0
	if needsScrollbar {
		reserve = nativeform.ScrollbarWidth + 4
	}
	contentWidth = max(1, layoutWidth-2*managerPad-reserve)
	buttonSpace := max(4, contentWidth-3*managerButtonGap)
	standardWidth = max(1, buttonSpace*116/managerCompactButtonBase)
	toggleWidth = max(1, buttonSpace-3*standardWidth)
	return contentWidth, standardWidth, toggleWidth
}

func (p *panel) setManagerBounds(id uint16, x, y, width, height int) {
	p.bounds[id] = logicalBounds{X: x, Y: y, Width: width, Height: height}
	p.positionControl(p.controls[id], x, y, width, height)
}

func (p *panel) syncManagerScrollbarBounds() {
	if p.managerScroll == nil {
		return
	}
	scale := p.scale()
	bounds := p.bounds[idList]
	width := int(float64(nativeform.ScrollbarWidth)*scale + 0.5)
	inset := max(1, int(2*scale+0.5))
	x := int(float64(bounds.X+bounds.Width)*scale+0.5) - width - inset
	y := int(float64(bounds.Y-p.contentOffset)*scale+0.5) + inset
	height := int(float64(bounds.Height)*scale+0.5) - 2*inset
	p.managerScroll.SetBounds(x, y, width, max(1, height))
}

func (p *panel) populateRules() {
	list := p.controls[idList]
	selectedID := p.selectedRuleID
	if index := p.selectedRule(); index >= 0 {
		selectedID = p.rules[index].ID
	}
	if top, _, _ := pSendMessage.Call(uintptr(list), lbGetTopIndex, 0, 0); top != ^uintptr(0) {
		p.listTopIndex = int(top)
	}
	pSendMessage.Call(uintptr(list), lbResetContent, 0, 0)
	selectedIndex := -1
	for index, rule := range p.rules {
		state := p.t("automation_rule_disabled")
		if rule.Enabled {
			state = p.t("automation_rule_enabled")
		}
		summary := p.ruleSummary(rule)
		if issue, invalid := p.issueForRule(index); invalid {
			state = p.t("automation_rule_invalid")
			summary = issue.Message
		}
		label := fmt.Sprintf("%s  %s — %s", state, rule.Name, summary)
		value, _ := windows.UTF16PtrFromString(label)
		pSendMessage.Call(uintptr(list), lbAddString, 0, uintptr(unsafe.Pointer(value)))
		if rule.ID == selectedID {
			selectedIndex = index
		}
	}
	if len(p.rules) == 0 {
		p.show(idList, false)
		p.show(idEmptyTitle, true)
		p.show(idEmptyBody, true)
	} else {
		p.show(idList, true)
		p.show(idEmptyTitle, false)
		p.show(idEmptyBody, false)
		if selectedIndex < 0 {
			selectedIndex = 0
		}
		pSendMessage.Call(uintptr(list), lbSetCurSel, uintptr(selectedIndex), 0)
		p.selectedRuleID = p.rules[selectedIndex].ID
		pSendMessage.Call(uintptr(list), lbSetTopIndex, uintptr(p.listTopIndex), 0)
	}
	p.updateManagerActions()
	if p.managerScroll != nil {
		p.managerScroll.Sync()
	}
}

func (p *panel) showEditor(index int) {
	p.rememberManagerView()
	draft := defaultRule()
	if index >= 0 && index < len(p.rules) {
		draft = p.rules[index]
	}
	p.showEditorDraft(index, draft)
}

func (p *panel) rememberManagerView() {
	if p.view != managerView || p.controls[idList] == 0 {
		return
	}
	if index := p.selectedRule(); index >= 0 {
		p.selectedRuleID = p.rules[index].ID
	}
	if top, _, _ := pSendMessage.Call(uintptr(p.controls[idList]), lbGetTopIndex, 0, 0); top != ^uintptr(0) {
		p.listTopIndex = int(top)
	}
}
