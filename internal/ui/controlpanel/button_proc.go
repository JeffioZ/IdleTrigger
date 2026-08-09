package controlpanel

import "golang.org/x/sys/windows"

func buttonWndProc(hwnd windows.Handle, msg uint32, wp, lp uintptr) uintptr {
	p := panelForButton(hwnd)
	var old uintptr
	var id uint16
	if p != nil {
		old = p.oldButtonProc[hwnd]
		id = p.controlID(hwnd)
		if handled, result := p.handleButtonMessage(hwnd, id, msg, wp); handled {
			return result
		}
	}
	if old != 0 {
		result, _, _ := pCallWindowProc.Call(old, uintptr(hwnd), uintptr(msg), wp, lp)
		if msg == wmSetFocus || msg == wmKillFocus || msg == wmEnable {
			p.updateToggleAccessibility(id)
		}
		return result
	}
	result, _, _ := pDefWindowProc.Call(uintptr(hwnd), uintptr(msg), wp, lp)
	return result
}

func (p *panel) handleButtonMessage(hwnd windows.Handle, id uint16, msg uint32, wp uintptr) (bool, uintptr) {
	switch msg {
	case wmMouseMove:
		p.setHover(hwnd)
	case wmMouseLeave:
		p.clearHover(hwnd)
	case wmLButtonDown:
		p.leaveKeyboardNavigation()
		if !p.menuClickKeepsOpen(id) {
			p.closeOpenMenus()
		}
	case wmKeyDown:
		return p.handleButtonKeyDown(id, wp)
	case wmSysKeyDown:
		if (wp == vkDown || wp == vkF4) && isPopupTrigger(id) {
			p.enterKeyboardNavigation()
			p.requestChoice(id)
			return true, 0
		}
	}
	return false, 0
}

func (p *panel) handleButtonKeyDown(id uint16, key uintptr) (bool, uintptr) {
	switch key {
	case vkUp, vkDown, vkHome, vkEnd, vkReturn, vkSpace, vkF4, vkEscape:
		// Programmatic SetFocus is not a modality change. Actual keyboard
		// navigation is the only path that enables focus-visible drawing.
		p.enterKeyboardNavigation()
	}
	if isPopupTrigger(id) && isChoiceOpenKey(key) {
		p.requestChoice(id)
		return true, 0
	}
	return false, 0
}

func isPopupTrigger(id uint16) bool {
	return id == idQuickActions
}

func isChoiceOpenKey(key uintptr) bool {
	switch key {
	case vkReturn, vkSpace, vkUp, vkDown, vkF4:
		return true
	default:
		return false
	}
}
