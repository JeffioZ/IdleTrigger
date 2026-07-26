package controlpanel

import (
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"golang.org/x/sys/windows"
)

func (p *panel) openQuickMenu() {
	items := p.quickMenuItems()
	p.openPopup(idQuickActions, -1, false, items, func(value int) {
		p.handleCommand(uint16(value))
	})
}

func (p *panel) quickMenuItems() []nativeform.ChoicePopupItem {
	ids := quickActionIDs()
	items := make([]nativeform.ChoicePopupItem, len(ids))
	for index, id := range ids {
		items[index] = nativeform.ChoicePopupItem{
			Label: p.text(quickActionTranslationKey(id)), Value: int(id), Danger: isDangerQuickAction(id),
		}
	}
	return items
}

func (p *panel) openLanguageMenu() {
	items, selected := p.languageMenuItems()
	ids := languageIDs()
	p.openPopup(idLanguage, selected, true, items, func(index int) {
		if index >= 0 && index < len(ids) {
			p.selectLanguage(ids[index], index)
		}
	})
}

func (p *panel) languageMenuItems() ([]nativeform.ChoicePopupItem, int) {
	selected := 0
	if p.selected[idLangZH] {
		selected = 1
	}
	labels := []string{p.text("menu_lang_en"), p.text("menu_lang_zh")}
	items := make([]nativeform.ChoicePopupItem, len(labels))
	for index, label := range labels {
		items[index] = nativeform.ChoicePopupItem{Label: label, Value: index}
	}
	return items, selected
}

// closeOpenMenus handles an in-panel click outside the currently open menu.
func (p *panel) closeOpenMenus() {
	p.closeChoice(false)
}

func (p *panel) menuClickKeepsOpen(id uint16) bool {
	return p.choice.openID != 0 && id == p.choice.openID
}

func (p *panel) setDisabled(id uint16, value bool) {
	if p.disabled[id] == value {
		return
	}
	p.disabled[id] = value
	if hwnd := p.controls[id]; hwnd != 0 {
		enabled := uintptr(1)
		if value {
			enabled = 0
		}
		pEnableWindow.Call(uintptr(hwnd), enabled)
	}
	p.refreshTooltip(id)
	p.invalidate(id)
}

func (p *panel) applyDependentStates() {
	monitorEnabled := p.toggles[idIdle]
	p.setDisabled(idIdleWarning, !monitorEnabled)
	p.setDisabled(idIdleEnhanced, !monitorEnabled)
	p.setDisabled(idIdleTimeout, !monitorEnabled)
	p.setDisabled(idIdleAction, !monitorEnabled)
	p.setDisabled(idTestWarning, !monitorEnabled)
	themeEnabled := p.toggles[idTheme] && !p.themeUnavailable
	p.setDisabled(idTheme, p.themeUnavailable)
	p.setDisabled(idFullscreen, !themeEnabled)
	p.setDisabled(idBattery, !themeEnabled)
	p.setDisabled(idIPLocation, !themeEnabled)
	p.setDisabled(idThemeSwitch, p.themeUnavailable)
	p.setDisabled(idThemeRepair, p.themeUnavailable)
}

func (p *panel) setKeyboardNavigation(active bool) {
	if p.keyboardNavigation == active {
		return
	}
	p.keyboardNavigation = active
	for id := range p.controls {
		p.invalidate(id)
	}
}

func (p *panel) enterKeyboardNavigation() { p.setKeyboardNavigation(true) }

func (p *panel) leaveKeyboardNavigation() { p.setKeyboardNavigation(false) }

func (p *panel) shouldDrawFocusOutline(itemState uint32) bool {
	return p.keyboardNavigation && itemState&odsFocus != 0
}

func (p *panel) subclassButton(hwnd windows.Handle) {
	if hwnd == 0 {
		return
	}
	old, _, _ := setWindowProc(hwnd, buttonProc)
	if old != 0 {
		p.oldButtonProc[hwnd] = old
	}
}

func setWindowProc(hwnd windows.Handle, proc uintptr) (uintptr, uintptr, error) {
	return nativeform.SetWindowProc(hwnd, proc)
}
