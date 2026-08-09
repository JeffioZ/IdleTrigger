package controlpanel

import "github.com/JeffioZ/idletrigger/internal/ui/nativeform"

func (p *panel) openPopup(id uint16, selected int, keepReselection bool, items []nativeform.ChoicePopupItem, onSelect func(int)) {
	if p.disabled[id] || p.controls[id] == 0 || len(items) == 0 {
		return
	}
	if p.choice.openID == id && p.choice.popup != nil && p.choice.popup.IsOpen() {
		p.closeChoice(true)
		return
	}
	p.closeChoice(false)
	p.choice.serial++
	serial := p.choice.serial
	p.choice.openID = id
	popup, err := nativeform.ShowChoicePopup(nativeform.ChoicePopupOptions{
		Owner: p.hwnd, Anchor: p.controls[id], Font: p.font, SelectedFont: p.sectionFont,
		Palette: p.palette, Dark: p.themeDark, Scale: p.metrics.scale,
		Selected: selected, MaxVisible: 6, Items: items,
		KeepOpenOnReselect: keepReselection, RestoreAnchorOnCancel: true,
		PreferAbove:  id == idQuickActions,
		FocusVisible: p.keyboardNavigation,
		OnSelect:     onSelect,
		OnClose: func() {
			if p.choice.serial != serial {
				return
			}
			p.choice.popup = nil
			if p.choice.openID == id {
				p.choice.openID = 0
			}
			p.invalidate(id)
		},
	})
	if err != nil || popup == nil || !popup.IsOpen() {
		p.choice.openID = 0
		p.choice.popup = nil
		p.invalidate(id)
		return
	}
	p.choice.popup = popup
	p.invalidate(id)
}

func (p *panel) requestChoice(id uint16) {
	pPostMessage.Call(uintptr(p.hwnd), wmOpenChoice, uintptr(id), 0)
}

func (p *panel) closeChoice(returnFocus bool) {
	openID := p.choice.openID
	popup := p.choice.popup
	if openID == 0 && popup == nil {
		return
	}
	// Invalidate callbacks from the popup being closed before clearing the
	// local state. A delayed WM_DESTROY from an older popup must never clear a
	// newer selector.
	p.choice.serial++
	p.choice.openID = 0
	p.choice.popup = nil
	if popup != nil {
		popup.Close()
	}
	if openID != 0 {
		p.invalidate(openID)
	}
	if returnFocus && p.hwnd != 0 && openID != 0 {
		pSetFocus.Call(uintptr(p.controls[openID]))
	}
}
