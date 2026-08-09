package controlpanel

import (
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"golang.org/x/sys/windows"
	"unsafe"
)

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

func roleForButton(id uint16) buttonRole {
	switch id {
	case idNoSleep, idAutomationEnabled, idIdle, idTheme:
		return buttonToggle
	default:
		return buttonCommand
	}
}

func visualStateForButton(id uint16, toggleOn, disabled bool) buttonVisualState {
	role := roleForButton(id)
	active := role == buttonToggle && toggleOn
	return buttonVisualState{Role: role, Active: active, Disabled: disabled}
}

func (p *panel) visualState(id uint16) buttonVisualState {
	return visualStateForButton(id, p.toggles[id], p.disabled[id])
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
func (p *panel) invalidate(id uint16) {
	if hwnd := p.controls[id]; hwnd != 0 {
		// Every panel control is owner-drawn and covers its complete client area.
		// Do not expose an erased intermediate frame when rapid hover transitions
		// invalidate adjacent controls before WM_DRAWITEM coalesces their paints.
		pInvalidateRect.Call(uintptr(hwnd), 0, 0)
	}
}
