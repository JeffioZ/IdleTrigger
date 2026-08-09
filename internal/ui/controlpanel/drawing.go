package controlpanel

import (
	"github.com/JeffioZ/idletrigger/internal/platform/windows/gdiplus"
	"github.com/JeffioZ/idletrigger/internal/ui/colors"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"golang.org/x/sys/windows"
	"unsafe"
)

func (p *panel) drawDisclosureArrow(dc windows.Handle, bounds rect, up bool, color uint32) gdiplus.DrawResult {
	width := int32(p.sc(p.metrics.style.Control.ArrowWidth))
	height := int32(p.sc(p.metrics.style.Control.ArrowHeight))
	if width < 2 || height < 1 {
		return gdiplus.DrawNotStarted
	}
	penWidth := p.sc(1)
	if penWidth < 1 {
		penWidth = 1
	}
	cx := bounds.Right - int32(p.sc(18))
	cy := (bounds.Top + bounds.Bottom) / 2
	halfW := width / 2
	halfH := height / 2
	stroke := int32(penWidth)
	return gdiplus.FillPolygon(dc, disclosureArrowPolygon(cx, cy, halfW, halfH, stroke, up), color)
}

func (p *panel) drawDisclosureArrowGDI(dc windows.Handle, bounds rect, up bool, color uint32) {
	width := int32(p.sc(p.metrics.style.Control.ArrowWidth))
	height := int32(p.sc(p.metrics.style.Control.ArrowHeight))
	if width < 2 || height < 1 {
		return
	}
	penWidth := p.sc(1)
	if penWidth < 1 {
		penWidth = 1
	}
	pen, _, _ := pCreatePen.Call(psSolid, uintptr(penWidth), uintptr(color))
	if pen == 0 {
		return
	}
	defer pDeleteObject.Call(pen)
	old, _, _ := pSelectObject.Call(uintptr(dc), pen)
	defer pSelectObject.Call(uintptr(dc), old)
	cx := bounds.Right - int32(p.sc(18))
	cy := (bounds.Top + bounds.Bottom) / 2
	halfW, halfH := width/2, height/2
	if up {
		pMoveToEx.Call(uintptr(dc), uintptr(cx-halfW), uintptr(cy+halfH), 0)
		pLineTo.Call(uintptr(dc), uintptr(cx), uintptr(cy-halfH))
		pLineTo.Call(uintptr(dc), uintptr(cx+halfW), uintptr(cy+halfH))
		return
	}
	pMoveToEx.Call(uintptr(dc), uintptr(cx-halfW), uintptr(cy-halfH), 0)
	pLineTo.Call(uintptr(dc), uintptr(cx), uintptr(cy+halfH))
	pLineTo.Call(uintptr(dc), uintptr(cx+halfW), uintptr(cy-halfH))
}

func disclosureArrowPolygon(cx, cy, halfW, halfH, stroke int32, up bool) []gdiplus.Point {
	if up {
		return []gdiplus.Point{{X: cx - halfW, Y: cy + halfH}, {X: cx - halfW + stroke, Y: cy + halfH}, {X: cx, Y: cy - halfH + stroke}, {X: cx + halfW - stroke, Y: cy + halfH}, {X: cx + halfW, Y: cy + halfH}, {X: cx, Y: cy - halfH}}
	}
	return []gdiplus.Point{{X: cx - halfW, Y: cy - halfH}, {X: cx - halfW + stroke, Y: cy - halfH}, {X: cx, Y: cy + halfH - stroke}, {X: cx + halfW - stroke, Y: cy - halfH}, {X: cx + halfW, Y: cy - halfH}, {X: cx, Y: cy + halfH}}
}

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
	if id == idIdleTimeout || id == idIdleAction {
		p.drawChoiceButton(item, state)
		return
	}
	if state.Role == buttonToggle {
		p.drawToggle(item)
		return
	}
	if isMenuTrigger(id) {
		p.drawMenuTrigger(item)
		return
	}
	if id == idProjectHome {
		p.drawProjectHomeLink(item, state)
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

// drawProjectHomeLink is a text-only link. Its HWND is only text-width, so
// surrounding panel whitespace remains inert.
func (p *panel) drawProjectHomeLink(item *drawItem, state buttonVisualState) {
	textColor := projectHomeLinkColor(p.palette, p.themeDark, state, item.ItemState)
	pFillRect.Call(uintptr(item.HDC), uintptr(unsafe.Pointer(&item.Rect)), uintptr(p.backgroundBrush))
	text, err := windows.UTF16PtrFromString(p.labels[idProjectHome])
	if err != nil {
		return
	}
	pSetTextColor.Call(uintptr(item.HDC), uintptr(textColor))
	pSetBkMode.Call(uintptr(item.HDC), transparent)
	old, _, _ := pSelectObject.Call(uintptr(item.HDC), uintptr(p.font))
	defer pSelectObject.Call(uintptr(item.HDC), old)
	textBounds := item.Rect
	pDrawText.Call(uintptr(item.HDC), uintptr(unsafe.Pointer(text)), ^uintptr(0), uintptr(unsafe.Pointer(&textBounds)), dtSingleLine|dtCalcRect)
	textW, textH := textBounds.Right-textBounds.Left, textBounds.Bottom-textBounds.Top
	textBounds.Left = item.Rect.Left + (item.Rect.Right-item.Rect.Left-textW)/2
	textBounds.Right = textBounds.Left + textW
	textBounds.Top = item.Rect.Top + (item.Rect.Bottom-item.Rect.Top-textH)/2 + int32(p.projectHomeTextVerticalOffset())
	textBounds.Bottom = textBounds.Top + textH
	pDrawText.Call(uintptr(item.HDC), uintptr(unsafe.Pointer(text)), ^uintptr(0), uintptr(unsafe.Pointer(&textBounds)), dtLeft|dtSingleLine)
	if !state.Disabled && item.ItemState&odsDisabled == 0 && (state.Hovered || state.Pressed) {
		pen, _, _ := pCreatePen.Call(psSolid, 1, uintptr(textColor))
		if pen != 0 {
			previous, _, _ := pSelectObject.Call(uintptr(item.HDC), pen)
			underlineY := textBounds.Bottom
			pMoveToEx.Call(uintptr(item.HDC), uintptr(textBounds.Left), uintptr(underlineY), 0)
			pLineTo.Call(uintptr(item.HDC), uintptr(textBounds.Right), uintptr(underlineY))
			pSelectObject.Call(uintptr(item.HDC), previous)
			pDeleteObject.Call(pen)
		}
	}
	if state.Focused {
		focusBounds := textBounds
		focusBounds.Left -= int32(p.sc(3))
		focusBounds.Right += int32(p.sc(3))
		focusBounds.Top -= int32(p.sc(2))
		focusBounds.Bottom += int32(p.sc(2))
		p.roundRectFocusRing(item.HDC, focusBounds, p.palette.Focus)
	}
}

// projectHomeLinkColor keeps the compact text link distinct from filled
// controls. The light palette needs a clearer hover step because there is no
// surface fill behind the label; dark mode already has that contrast.
func projectHomeLinkColor(palette colors.Palette, dark bool, state buttonVisualState, itemState uint32) uint32 {
	if state.Disabled || itemState&odsDisabled != 0 {
		return palette.DisabledText
	}
	if !dark {
		if state.Pressed {
			return colors.RGB(0, 60, 102)
		}
		if state.Hovered {
			return colors.RGB(0, 90, 158)
		}
		return palette.Accent
	}
	if state.Pressed {
		return palette.AccentPressed
	}
	if state.Hovered {
		return palette.AccentHover
	}
	return palette.Accent
}

func (p *panel) drawChoiceButton(item *drawItem, state buttonVisualState) {
	controlState := nativeControlState(state)
	controlState.Open = p.triggerOpen(uint16(item.CtlID))
	nativeform.DrawChoice(item.HDC, nativeRect(item.Rect), p.font, p.labels[uint16(item.CtlID)], p.palette, p.palette.WindowBackground, controlState, int32(p.sc(p.metrics.style.Control.CornerRadius)/2), p.metrics.scale)
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

// drawMenuTrigger keeps click-open menus visually distinct from commands that
// execute immediately: a quieter rounded card at rest, with accent treatment
// reserved for hover.
func (p *panel) drawMenuTrigger(item *drawItem) {
	id := uint16(item.CtlID)
	state := p.controlState(id, item.ItemState)
	open := p.triggerOpen(id)
	brush := p.surfaceBrush
	borderColor := p.palette.SubtleBorder
	textColor := p.palette.SecondaryText
	arrowColor := p.palette.SecondaryText
	if state.Hovered {
		brush = p.hoverBrush
		borderColor = p.palette.Accent
		textColor = p.palette.PrimaryText
		arrowColor = p.palette.Accent
	}
	if open {
		brush = p.hoverBrush
		borderColor = p.palette.Accent
		textColor = p.palette.PrimaryText
		arrowColor = p.palette.Accent
	}
	if state.Pressed {
		brush = p.pressedBrush
		if !open {
			borderColor = p.palette.AccentPressed
		}
		textColor = p.palette.AccentText
		arrowColor = p.palette.AccentText
	}
	pFillRect.Call(uintptr(item.HDC), uintptr(unsafe.Pointer(&item.Rect)), uintptr(p.backgroundBrush))
	p.roundRect(item.HDC, item.Rect, brush, borderColor, p.sc(p.metrics.style.Control.CornerRadius))
	// Fixed menus rise above their triggers, unlike the regular choice menus.
	arrowResult := p.drawDisclosureArrow(item.HDC, item.Rect, !open, arrowColor)
	if arrowResult != gdiplus.DrawCompleted {
		if arrowResult == gdiplus.DrawMayBeDirty {
			p.roundRect(item.HDC, item.Rect, brush, borderColor, p.sc(p.metrics.style.Control.CornerRadius))
		}
		p.drawDisclosureArrowGDI(item.HDC, item.Rect, !open, arrowColor)
	}

	pSetTextColor.Call(uintptr(item.HDC), uintptr(textColor))
	pSetBkMode.Call(uintptr(item.HDC), transparent)
	old, _, _ := pSelectObject.Call(uintptr(item.HDC), uintptr(p.font))
	defer pSelectObject.Call(uintptr(item.HDC), old)
	text, _ := windows.UTF16PtrFromString(p.labels[id])
	bounds := item.Rect
	bounds.Left += int32(p.sc(p.metrics.style.Control.ButtonTextInset))
	bounds.Right -= int32(p.sc(p.metrics.style.Control.ButtonTextInset))
	drawTextCentered(item.HDC, text, bounds)
	if state.Focused {
		p.roundRectFocusRing(item.HDC, item.Rect, p.palette.Focus)
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
