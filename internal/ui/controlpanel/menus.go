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
		p.updateToggleAccessibility(id)
	}
	p.refreshTooltip(id)
	p.invalidate(id)
}

func (p *panel) applyDependentStates() {
	p.setDisabled(idTheme, p.themeUnavailable)
	p.setDisabled(idThemeSwitch, p.themeUnavailable)
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
