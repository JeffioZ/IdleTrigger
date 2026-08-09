package controlpanel

import (
	"github.com/JeffioZ/idletrigger/internal/platform/windows/gdiplus"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"golang.org/x/sys/windows"
	"unsafe"
)

// roundRectFocusRing draws one inset outline using only temporary GDI objects.
// The caller uses it only for rounded controls; checkbox focus intentionally
// remains square and follows the existing native-control language.
func (p *panel) roundRectFocusRing(dc windows.Handle, bounds rect, color uint32) {
	inset := int32(p.sc(p.metrics.style.Control.FocusInset))
	width := p.sc(p.metrics.style.Control.FocusRingWidth)
	if width < 1 {
		width = 1
	}
	ring := bounds
	ring.Left += inset
	ring.Top += inset
	ring.Right -= inset
	ring.Bottom -= inset
	if ring.Right-ring.Left <= int32(width) || ring.Bottom-ring.Top <= int32(width) {
		return
	}
	radius := p.sc(p.metrics.style.Control.CornerRadius) - int(inset)
	if radius < 0 {
		radius = 0
	}
	pen, _, _ := pCreatePen.Call(psSolid, uintptr(width), uintptr(color))
	if pen == 0 {
		return
	}
	defer pDeleteObject.Call(pen)
	hollow, _, _ := pGetStockObject.Call(5) // HOLLOW_BRUSH
	if hollow == 0 {
		return
	}
	oldBrush, _, _ := pSelectObject.Call(uintptr(dc), hollow)
	oldPen, _, _ := pSelectObject.Call(uintptr(dc), pen)
	pRoundRect.Call(uintptr(dc), uintptr(ring.Left), uintptr(ring.Top), uintptr(ring.Right), uintptr(ring.Bottom), uintptr(radius), uintptr(radius))
	pSelectObject.Call(uintptr(dc), oldPen)
	pSelectObject.Call(uintptr(dc), oldBrush)
}

func (p *panel) drawButton(item *drawItem) {
	id := uint16(item.CtlID)
	state := p.controlState(id, item.ItemState)
	if state.Role == buttonToggle {
		p.drawToggle(item)
		return
	}
	if id != idExit {
		nativeform.DrawButton(item.HDC, nativeRect(item.Rect), p.font, p.labels[id], p.palette, p.palette.WindowBackground, nativeControlState(state), int32(p.sc(p.metrics.style.Control.CornerRadius)/2), false)
		return
	}
	brush, borderColor, textColor := p.surfaceBrush, p.palette.Border, p.palette.PrimaryText
	if state.Hovered {
		brush = p.hoverBrush
	}
	danger := id == idExit
	if danger {
		// Exit remains semantically distinct without becoming the panel's
		// strongest default call to action. The solid danger fill is reserved
		// for deliberate hover/press feedback.
		brush, borderColor, textColor = p.surfaceBrush, p.palette.DangerSurfaceText, p.palette.DangerSurfaceText
		if state.Hovered {
			brush, borderColor, textColor = p.dangerHoverBrush, p.palette.DangerHoverBorder, p.palette.DangerText
		}
	}
	if state.Pressed {
		brush, textColor = p.pressedBrush, p.palette.AccentText
		if danger {
			brush, borderColor, textColor = p.dangerPressedBrush, p.palette.DangerPressedBorder, p.palette.DangerText
		}
	}
	if state.Disabled || item.ItemState&odsDisabled != 0 {
		brush, borderColor, textColor = p.disabledBrush, p.palette.SubtleBorder, p.palette.DisabledText
	}
	pFillRect.Call(uintptr(item.HDC), uintptr(unsafe.Pointer(&item.Rect)), uintptr(p.backgroundBrush))
	p.roundRect(item.HDC, item.Rect, brush, borderColor, p.sc(p.metrics.style.Control.CornerRadius))
	pSetTextColor.Call(uintptr(item.HDC), uintptr(textColor))
	pSetBkMode.Call(uintptr(item.HDC), transparent)
	old, _, _ := pSelectObject.Call(uintptr(item.HDC), uintptr(p.font))
	defer pSelectObject.Call(uintptr(item.HDC), old)
	text, _ := windows.UTF16PtrFromString(p.labels[id])
	r := item.Rect
	r.Left += int32(p.sc(p.metrics.style.Control.ButtonTextInset))
	r.Right -= int32(p.sc(p.metrics.style.Control.ButtonTextInset))
	drawTextCentered(item.HDC, text, r)
	if state.Focused {
		focusColor := p.palette.Focus
		if danger {
			focusColor = p.palette.DangerFocus
		}
		p.roundRectFocusRing(item.HDC, item.Rect, focusColor)
	}
}

func nativeRect(bounds rect) nativeform.Rect {
	return nativeform.Rect{Left: bounds.Left, Top: bounds.Top, Right: bounds.Right, Bottom: bounds.Bottom}
}

func nativeControlState(state buttonVisualState) nativeform.ControlState {
	return nativeform.ControlState{
		Hovered: state.Hovered, Pressed: state.Pressed, Focused: state.Focused,
		Disabled: state.Disabled, Active: state.Active,
	}
}

func (p *panel) roundRect(dc windows.Handle, bounds rect, brush windows.Handle, borderColor uint32, cornerDiameter int) {
	if fillColor, ok := p.cardFillColor(brush); ok {
		result := gdiplus.FillRoundedRect(dc, bounds.Left, bounds.Top, bounds.Right, bounds.Bottom, int32(cornerDiameter/2), fillColor, borderColor)
		if result == gdiplus.DrawCompleted {
			return
		}
		if result == gdiplus.DrawMayBeDirty {
			// A failed final GDI+ fill can have changed any card pixel, including
			// anti-aliased corner coverage. Rebuild the complete GDI card instead
			// of drawing only its outline over possibly dirty pixels.
			pFillRect.Call(uintptr(dc), uintptr(unsafe.Pointer(&bounds)), uintptr(p.backgroundBrush))
		}
	}
	p.roundRectGDI(dc, bounds, brush, borderColor, cornerDiameter)
}

func (p *panel) roundRectGDI(dc windows.Handle, bounds rect, brush windows.Handle, borderColor uint32, cornerDiameter int) {
	pen, _, _ := pCreatePen.Call(psSolid, 1, uintptr(borderColor))
	if pen == 0 {
		pFillRect.Call(uintptr(dc), uintptr(unsafe.Pointer(&bounds)), uintptr(brush))
		return
	}
	oldBrush, _, _ := pSelectObject.Call(uintptr(dc), uintptr(brush))
	oldPen, _, _ := pSelectObject.Call(uintptr(dc), pen)
	pRoundRect.Call(uintptr(dc), uintptr(bounds.Left), uintptr(bounds.Top), uintptr(bounds.Right), uintptr(bounds.Bottom), uintptr(cornerDiameter), uintptr(cornerDiameter))
	pSelectObject.Call(uintptr(dc), oldPen)
	pSelectObject.Call(uintptr(dc), oldBrush)
	pDeleteObject.Call(pen)
}

func (p *panel) cardFillColor(brush windows.Handle) (uint32, bool) {
	if brush == 0 {
		return 0, false
	}
	switch brush {
	case p.surfaceBrush:
		return p.palette.Surface, true
	case p.hoverBrush:
		return p.palette.HoverSurface, true
	case p.pressedBrush:
		return p.palette.AccentPressed, true
	case p.disabledBrush:
		return p.palette.DisabledSurface, true
	case p.dangerBrush:
		return p.palette.DangerBackground, true
	case p.dangerHoverBrush:
		return p.palette.DangerHover, true
	case p.dangerPressedBrush:
		return p.palette.DangerPressed, true
	default:
		return 0, false
	}
}
