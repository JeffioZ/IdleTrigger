package nativeform

import (
	"unicode"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/platform/windows/gdiplus"
	"github.com/JeffioZ/idletrigger/internal/ui/colors"
)

type Rect struct{ Left, Top, Right, Bottom int32 }

type ControlState struct {
	Hovered  bool
	Pressed  bool
	Focused  bool
	Disabled bool
	Active   bool
	Open     bool
}

const (
	drawTransparent = 1
	drawCenter      = 0x00000001
	drawVCenter     = 0x00000004
	drawWordBreak   = 0x00000010
	drawSingleLine  = 0x00000020
	drawCalcRect    = 0x00000400
	drawLeft        = 0x00000000
	drawPSolid      = 0
)

var (
	controlUser32       = windows.NewLazySystemDLL("user32.dll")
	controlGDI32        = windows.NewLazySystemDLL("gdi32.dll")
	controlFillRect     = controlUser32.NewProc("FillRect")
	controlDrawText     = controlUser32.NewProc("DrawTextW")
	controlFrameRect    = controlUser32.NewProc("FrameRect")
	controlCreateBrush  = controlGDI32.NewProc("CreateSolidBrush")
	controlCreatePen    = controlGDI32.NewProc("CreatePen")
	controlSelectObject = controlGDI32.NewProc("SelectObject")
	controlDeleteObject = controlGDI32.NewProc("DeleteObject")
	controlRoundRect    = controlGDI32.NewProc("RoundRect")
	controlMoveToEx     = controlGDI32.NewProc("MoveToEx")
	controlLineTo       = controlGDI32.NewProc("LineTo")
	controlSetTextColor = controlGDI32.NewProc("SetTextColor")
	controlSetBkMode    = controlGDI32.NewProc("SetBkMode")
	controlTextExtent   = controlGDI32.NewProc("GetTextExtentPoint32W")
	controlGetDC        = controlUser32.NewProc("GetDC")
	controlReleaseDC    = controlUser32.NewProc("ReleaseDC")
)

type textExtent struct{ Width, Height int32 }

// TextWidth returns the rendered width of a single-line label in physical
// pixels. Native form layouts use it to keep adjacent labels and text links
// aligned across languages and DPI scales without language-specific offsets.
func TextWidth(hwnd, font windows.Handle, label string) int {
	text, err := windows.UTF16FromString(label)
	if err != nil || len(text) < 2 || hwnd == 0 || font == 0 {
		return 0
	}
	dc, _, _ := controlGetDC.Call(uintptr(hwnd))
	if dc == 0 {
		return 0
	}
	defer controlReleaseDC.Call(uintptr(hwnd), dc)
	old, _, _ := controlSelectObject.Call(dc, uintptr(font))
	defer controlSelectObject.Call(dc, old)
	bounds := Rect{}
	if height, _, _ := controlDrawText.Call(dc, uintptr(unsafe.Pointer(&text[0])), ^uintptr(0), uintptr(unsafe.Pointer(&bounds)), drawLeft|drawSingleLine|drawCalcRect); height == 0 {
		return 0
	}
	return int(bounds.Right - bounds.Left)
}

// CheckboxHitWidth returns the compact logical width that contains the
// checkbox glyph, its label, and the focus inset. Keeping the native BUTTON
// window to this width makes its click and tooltip area match what is drawn.
func CheckboxHitWidth(hwnd, font windows.Handle, label string, scale float64) int {
	if scale <= 0 {
		scale = 1
	}
	physicalTextWidth := TextWidth(hwnd, font, label)
	if physicalTextWidth <= 0 {
		return 0
	}
	logicalTextWidth := int(float64(physicalTextWidth)/scale + 0.999)
	return checkboxHitWidthForText(logicalTextWidth)
}

func checkboxHitWidthForText(textWidth int) int {
	if textWidth < 0 {
		textWidth = 0
	}
	// DrawCheckbox uses 2 px before the glyph and 8 px before the label.
	// The final 2 px keeps the focus frame from touching the last glyph.
	return 2 + CheckboxSize + 8 + textWidth + 2
}

// DrawTextLink renders a native owner-drawn button as a conventional text
// hyperlink while retaining keyboard focus and pressed-state feedback.
func DrawTextLink(dc windows.Handle, bounds Rect, font windows.Handle, label string, palette colors.Palette, background uint32, state ControlState, scale float64) {
	fillRect(dc, bounds, background)
	color := palette.Accent
	if state.Hovered {
		color = palette.AccentHover
	}
	if state.Pressed {
		color = palette.AccentPressed
	}
	text, err := windows.UTF16FromString(label)
	if err != nil || len(text) < 2 {
		return
	}
	old, _, _ := controlSelectObject.Call(uintptr(dc), uintptr(font))
	controlSetTextColor.Call(uintptr(dc), uintptr(color))
	controlSetBkMode.Call(uintptr(dc), drawTransparent)
	textBounds := bounds
	controlDrawText.Call(uintptr(dc), uintptr(unsafe.Pointer(&text[0])), ^uintptr(0), uintptr(unsafe.Pointer(&textBounds)), drawLeft|drawVCenter|drawSingleLine)
	var extent textExtent
	controlTextExtent.Call(uintptr(dc), uintptr(unsafe.Pointer(&text[0])), uintptr(len(text)-1), uintptr(unsafe.Pointer(&extent)))
	underlineY := bounds.Top + (bounds.Bottom-bounds.Top+extent.Height)/2
	pen, _, _ := controlCreatePen.Call(drawPSolid, uintptr(max32(1, scaledPixels(1, scale))), uintptr(color))
	if pen != 0 {
		oldPen, _, _ := controlSelectObject.Call(uintptr(dc), pen)
		controlMoveToEx.Call(uintptr(dc), uintptr(bounds.Left), uintptr(underlineY), 0)
		controlLineTo.Call(uintptr(dc), uintptr(min(bounds.Right, bounds.Left+extent.Width)), uintptr(underlineY))
		controlSelectObject.Call(uintptr(dc), oldPen)
		controlDeleteObject.Call(pen)
	}
	controlSelectObject.Call(uintptr(dc), old)
	if state.Focused {
		frame := bounds
		frame.Bottom--
		frameRect(dc, frame, palette.Focus)
	}
}

func DrawSurface(dc windows.Handle, bounds Rect, palette colors.Palette, background, fill, border uint32, radius int32) {
	fillRect(dc, bounds, background)
	result := gdiplus.FillRoundedRect(dc, bounds.Left, bounds.Top, bounds.Right, bounds.Bottom, radius, fill, border)
	if result == gdiplus.DrawCompleted {
		return
	}
	if result == gdiplus.DrawMayBeDirty {
		fillRect(dc, bounds, background)
	}
	brush, _, _ := controlCreateBrush.Call(uintptr(fill))
	pen, _, _ := controlCreatePen.Call(drawPSolid, 1, uintptr(border))
	if brush == 0 || pen == 0 {
		if brush != 0 {
			controlDeleteObject.Call(brush)
		}
		if pen != 0 {
			controlDeleteObject.Call(pen)
		}
		return
	}
	oldBrush, _, _ := controlSelectObject.Call(uintptr(dc), brush)
	oldPen, _, _ := controlSelectObject.Call(uintptr(dc), pen)
	diameter := uintptr(max(2, int(radius*2)))
	controlRoundRect.Call(uintptr(dc), uintptr(bounds.Left), uintptr(bounds.Top), uintptr(bounds.Right), uintptr(bounds.Bottom), diameter, diameter)
	controlSelectObject.Call(uintptr(dc), oldPen)
	controlSelectObject.Call(uintptr(dc), oldBrush)
	controlDeleteObject.Call(pen)
	controlDeleteObject.Call(brush)
}

func DrawField(dc windows.Handle, bounds Rect, palette colors.Palette, background uint32, state ControlState, radius int32) {
	fill, border := palette.Surface, palette.Border
	if state.Disabled {
		fill, border = palette.DisabledSurface, palette.SubtleBorder
	} else if state.Focused {
		border = palette.Focus
	} else if state.Hovered {
		border = palette.Accent
	}
	DrawSurface(dc, bounds, palette, background, fill, border, radius)
}

func DrawButton(dc windows.Handle, bounds Rect, font windows.Handle, label string, palette colors.Palette, background uint32, state ControlState, radius int32, leftAligned bool) {
	fill, border, textColor := buttonVisual(palette, state)
	DrawSurface(dc, bounds, palette, background, fill, border, radius)
	drawButtonLabel(dc, bounds, font, label, textColor, leftAligned, 10, 10)
}

func buttonVisual(palette colors.Palette, state ControlState) (fill, border, textColor uint32) {
	fill, border, textColor = palette.Surface, palette.Border, palette.PrimaryText
	if state.Hovered {
		fill, border = palette.HoverSurface, palette.Accent
	}
	if state.Active {
		fill, border, textColor = palette.Selected, palette.Selected, palette.AccentText
		if state.Hovered {
			fill, border = palette.SelectedHover, palette.SelectedHover
		}
	}
	// Pressed is the strongest transient state for both normal and active
	// buttons. Keeping it last prevents selected buttons from looking merely
	// hovered while the mouse button is held.
	if state.Pressed {
		fill, border, textColor = palette.AccentPressed, palette.AccentPressed, palette.AccentText
	}
	if state.Disabled {
		fill, border, textColor = palette.DisabledSurface, palette.SubtleBorder, palette.DisabledText
	}
	if state.Focused && !state.Disabled {
		border = palette.Focus
	}
	return fill, border, textColor
}

func DrawChoice(dc windows.Handle, bounds Rect, font windows.Handle, label string, palette colors.Palette, background uint32, state ControlState, radius int32, scale float64) {
	fill, border, textColor, arrowColor := palette.Surface, palette.Border, palette.PrimaryText, palette.SecondaryText
	if state.Hovered || state.Open {
		fill, border, arrowColor = palette.HoverSurface, palette.Accent, palette.Accent
	}
	if state.Pressed {
		fill, border, textColor, arrowColor = palette.AccentPressed, palette.AccentPressed, palette.AccentText, palette.AccentText
	}
	if state.Disabled {
		fill, border, textColor, arrowColor = palette.DisabledSurface, palette.SubtleBorder, palette.DisabledText, palette.DisabledText
	}
	if state.Focused && !state.Disabled {
		border = palette.Focus
	}
	DrawSurface(dc, bounds, palette, background, fill, border, radius)
	textBounds := bounds
	textBounds.Right -= scaledPixels(30, scale)
	drawLabel(dc, textBounds, font, label, textColor, true, 10, 4)
	drawArrow(dc, bounds.Right-scaledPixels(18, scale), (bounds.Top+bounds.Bottom)/2, state.Open, arrowColor, scale)
}

func DrawCheckbox(dc windows.Handle, bounds Rect, font windows.Handle, label string, palette colors.Palette, background uint32, state ControlState, scale float64) {
	fillRect(dc, bounds, background)
	boxSize := scaledPixels(CheckboxSize, scale)
	box := Rect{Left: bounds.Left + scaledPixels(2, scale), Top: bounds.Top + (bounds.Bottom-bounds.Top-boxSize)/2}
	box.Right, box.Bottom = box.Left+boxSize, box.Top+boxSize
	drawCheckboxBox(dc, box, palette, background, state, scale)
	if state.Focused && !state.Disabled {
		frame := bounds
		inset := scaledPixels(1, scale)
		frame.Left += inset
		frame.Top += inset
		frame.Right -= inset
		frame.Bottom -= inset
		frameRect(dc, frame, palette.Focus)
	}
	textBounds := bounds
	textBounds.Left = box.Right + scaledPixels(8, scale)
	drawLabel(dc, textBounds, font, label, checkboxTextColor(palette, state), true, 0, 4)
}

// DrawCheckboxGlyph draws the same checkbox used by form controls without a
// label. It is also suitable for native list-view state images.
func DrawCheckboxGlyph(dc windows.Handle, bounds Rect, palette colors.Palette, background uint32, state ControlState, scale float64) {
	fillRect(dc, bounds, background)
	boxSize := scaledPixels(CheckboxSize, scale)
	box := Rect{
		Left: bounds.Left + (bounds.Right-bounds.Left-boxSize)/2,
		Top:  bounds.Top + (bounds.Bottom-bounds.Top-boxSize)/2,
	}
	box.Right, box.Bottom = box.Left+boxSize, box.Top+boxSize
	drawCheckboxBox(dc, box, palette, background, state, scale)
}

// DrawPopupHeader renders a non-selectable group label inside a choice popup.
func DrawPopupHeader(dc windows.Handle, bounds Rect, font windows.Handle, label string, palette colors.Palette, background uint32, scale int32) {
	fillRect(dc, bounds, background)
	drawLabel(dc, bounds, font, label, palette.SecondaryText, true, 10*scale, 6*scale)
}

// DrawTableHeaderCell gives native report headers the same semantic theme as
// the surrounding form while preserving the header's native hit testing,
// sorting, keyboard behavior and custom-draw state notifications.
func DrawTableHeaderCell(dc windows.Handle, bounds Rect, font windows.Handle, label string, palette colors.Palette, state ControlState, scale float64) {
	fill, textColor, dividerColor := tableHeaderVisual(palette, state)
	fillRect(dc, bounds, fill)
	divider := max32(1, scaledPixels(1, scale))
	fillRect(dc, Rect{Left: bounds.Left, Top: bounds.Bottom - divider, Right: bounds.Right, Bottom: bounds.Bottom}, dividerColor)
	fillRect(dc, Rect{Left: bounds.Right - divider, Top: bounds.Top, Right: bounds.Right, Bottom: bounds.Bottom}, palette.SubtleBorder)
	if label != "" {
		drawLabel(dc, bounds, font, label, textColor, true, scaledPixels(8, scale), scaledPixels(6, scale))
	}
}

func tableHeaderVisual(palette colors.Palette, state ControlState) (fill, text, divider uint32) {
	fill, text, divider = palette.ElevatedSurface, palette.PrimaryText, palette.SubtleBorder
	if state.Hovered {
		fill, divider = palette.HoverSurface, palette.Accent
	}
	if state.Pressed {
		fill, text, divider = palette.AccentPressed, palette.AccentText, palette.AccentPressed
	}
	return fill, text, divider
}

// DrawMenuOption is shared by form-window choice popups and follows the main
// control panel's menu language: quiet rows, a slim selected marker, a
// slightly stronger selected label, and stable hover/press surfaces.
func DrawMenuOption(dc windows.Handle, bounds Rect, font, selectedFont windows.Handle, label string, palette colors.Palette, background uint32, state ControlState, selected, danger bool, radius, scale int32) {
	fill, border, textColor := palette.Surface, palette.Border, palette.PrimaryText
	if state.Hovered {
		fill = palette.HoverSurface
	}
	if danger {
		fill, border, textColor = palette.DangerBackground, palette.DangerBorder, palette.DangerText
		if state.Hovered {
			fill, border = palette.DangerHover, palette.DangerHoverBorder
		}
	}
	if state.Pressed {
		fill, border = palette.ElevatedSurface, palette.AccentPressed
		if danger {
			fill, border, textColor = palette.DangerPressed, palette.DangerPressedBorder, palette.DangerText
		}
	}
	if state.Disabled {
		fill, border, textColor = palette.DisabledSurface, palette.SubtleBorder, palette.DisabledText
	}
	if state.Focused && !state.Disabled {
		border = palette.Focus
		if danger {
			border = palette.DangerFocus
		}
	}
	DrawSurface(dc, bounds, palette, background, fill, border, radius)
	if selected {
		marker := bounds
		marker.Left += MenuSurfaceInset * scale
		marker.Right = marker.Left + MenuMarkerWidth*scale
		marker.Top += 6 * scale
		marker.Bottom -= 6 * scale
		if marker.Left < marker.Right && marker.Top < marker.Bottom {
			fillRect(dc, marker, palette.Accent)
		}
		if selectedFont != 0 {
			font = selectedFont
		}
	}
	drawLabel(dc, bounds, font, label, textColor, true, 10*scale, 8*scale)
}

func drawCheckboxBox(dc windows.Handle, box Rect, palette colors.Palette, background uint32, state ControlState, scale float64) {
	fill, border := palette.Surface, palette.Border
	if state.Active {
		fill, border = palette.Accent, palette.Accent
	}
	if state.Hovered {
		border = palette.AccentHover
		if state.Active {
			fill = palette.AccentHover
		}
	}
	if state.Pressed {
		fill, border = palette.AccentPressed, palette.AccentPressed
	}
	if state.Disabled {
		fill, border = palette.DisabledSurface, palette.SubtleBorder
	}
	DrawSurface(dc, box, palette, background, fill, border, scaledPixels(2, scale))
	if state.Active {
		checkColor := palette.AccentText
		if state.Disabled {
			checkColor = palette.MutedText
		}
		if gdiplus.DrawCheck(dc, box.Left, box.Top, box.Right, box.Bottom, checkColor, max32(1, scaledPixels(2, scale))) != gdiplus.DrawCompleted {
			drawCheckFallback(dc, box, checkColor, scale)
		}
	}
}

func checkboxTextColor(palette colors.Palette, state ControlState) uint32 {
	if state.Disabled {
		return palette.DisabledText
	}
	return palette.PrimaryText
}

func drawLabel(dc windows.Handle, bounds Rect, font windows.Handle, label string, color uint32, left bool, leftInset, rightInset int32) {
	text, err := windows.UTF16PtrFromString(label)
	if err != nil {
		return
	}
	bounds.Left += leftInset
	bounds.Right -= rightInset
	controlSetTextColor.Call(uintptr(dc), uintptr(color))
	controlSetBkMode.Call(uintptr(dc), drawTransparent)
	old, _, _ := controlSelectObject.Call(uintptr(dc), uintptr(font))
	flags := uintptr(drawCenter | drawVCenter | drawSingleLine)
	if left {
		flags = drawLeft | drawVCenter | drawSingleLine
	}
	controlDrawText.Call(uintptr(dc), uintptr(unsafe.Pointer(text)), ^uintptr(0), uintptr(unsafe.Pointer(&bounds)), flags)
	controlSelectObject.Call(uintptr(dc), old)
}

// drawButtonLabel measures the rendered text block before placing it.
// DrawText's DT_VCENTER/DT_SINGLELINE path rounds an odd remainder down into
// the text offset, which puts common CJK UI fonts one physical pixel below the
// optical center at fractional DPI scales. Keeping the remainder below the
// label matches the panel's row-height calculation, supports long translated
// labels, and makes every owner-drawn action use one stable baseline rule.
func drawButtonLabel(dc windows.Handle, bounds Rect, font windows.Handle, label string, color uint32, left bool, leftInset, rightInset int32) {
	text, err := windows.UTF16PtrFromString(label)
	if err != nil {
		return
	}
	bounds.Left += leftInset
	bounds.Right -= rightInset
	controlSetTextColor.Call(uintptr(dc), uintptr(color))
	controlSetBkMode.Call(uintptr(dc), drawTransparent)
	old, _, _ := controlSelectObject.Call(uintptr(dc), uintptr(font))
	defer controlSelectObject.Call(uintptr(dc), old)

	flags := uintptr(drawCenter | drawWordBreak)
	if left {
		flags = drawLeft | drawWordBreak
	}
	originalTop := bounds.Top
	measured := bounds
	if height, _, _ := controlDrawText.Call(uintptr(dc), uintptr(unsafe.Pointer(text)), ^uintptr(0), uintptr(unsafe.Pointer(&measured)), flags|drawCalcRect); height != 0 {
		textHeight := measured.Bottom - measured.Top
		bounds.Top = centeredTextTop(bounds.Top, bounds.Bottom, textHeight)
		if bounds.Top > originalTop && needsHanOpticalLift(label) {
			// Microsoft YaHei UI's Han glyph mass sits one physical pixel below
			// the measured line-box center at 100%, 150%, and 200% DPI. Keep
			// this an unscaled optical correction; geometry remains unchanged.
			bounds.Top--
			bounds.Bottom--
		}
	}
	controlDrawText.Call(uintptr(dc), uintptr(unsafe.Pointer(text)), ^uintptr(0), uintptr(unsafe.Pointer(&bounds)), flags)
}

func centeredTextTop(top, bottom, textHeight int32) int32 {
	available := bottom - top
	if textHeight <= 0 || textHeight >= available {
		return top
	}
	return top + (available-textHeight)/2
}

func needsHanOpticalLift(label string) bool {
	for _, char := range label {
		if unicode.Is(unicode.Han, char) {
			return true
		}
	}
	return false
}

func drawArrow(dc windows.Handle, x, y int32, up bool, color uint32, scale float64) {
	penWidth := uintptr(max32(1, scaledPixels(1, scale)))
	pen, _, _ := controlCreatePen.Call(drawPSolid, penWidth, uintptr(color))
	if pen == 0 {
		return
	}
	old, _, _ := controlSelectObject.Call(uintptr(dc), pen)
	halfW, halfH := scaledPixels(4, scale), scaledPixels(2, scale)
	if up {
		controlMoveToEx.Call(uintptr(dc), uintptr(x-halfW), uintptr(y+halfH), 0)
		controlLineTo.Call(uintptr(dc), uintptr(x), uintptr(y-halfH))
		controlLineTo.Call(uintptr(dc), uintptr(x+halfW), uintptr(y+halfH))
	} else {
		controlMoveToEx.Call(uintptr(dc), uintptr(x-halfW), uintptr(y-halfH), 0)
		controlLineTo.Call(uintptr(dc), uintptr(x), uintptr(y+halfH))
		controlLineTo.Call(uintptr(dc), uintptr(x+halfW), uintptr(y-halfH))
	}
	controlSelectObject.Call(uintptr(dc), old)
	controlDeleteObject.Call(pen)
}

func drawCheckFallback(dc windows.Handle, box Rect, color uint32, scale float64) {
	pen, _, _ := controlCreatePen.Call(drawPSolid, uintptr(max32(1, scaledPixels(2, scale))), uintptr(color))
	if pen == 0 {
		return
	}
	old, _, _ := controlSelectObject.Call(uintptr(dc), pen)
	controlMoveToEx.Call(uintptr(dc), uintptr(box.Left+scaledPixels(4, scale)), uintptr(box.Top+scaledPixels(9, scale)), 0)
	controlLineTo.Call(uintptr(dc), uintptr(box.Left+scaledPixels(8, scale)), uintptr(box.Top+scaledPixels(13, scale)))
	controlLineTo.Call(uintptr(dc), uintptr(box.Left+scaledPixels(15, scale)), uintptr(box.Top+scaledPixels(5, scale)))
	controlSelectObject.Call(uintptr(dc), old)
	controlDeleteObject.Call(pen)
}

func fillRect(dc windows.Handle, bounds Rect, color uint32) {
	brush, _, _ := controlCreateBrush.Call(uintptr(color))
	if brush == 0 {
		return
	}
	controlFillRect.Call(uintptr(dc), uintptr(unsafe.Pointer(&bounds)), brush)
	controlDeleteObject.Call(brush)
}

func frameRect(dc windows.Handle, bounds Rect, color uint32) {
	brush, _, _ := controlCreateBrush.Call(uintptr(color))
	if brush == 0 {
		return
	}
	controlFrameRect.Call(uintptr(dc), uintptr(unsafe.Pointer(&bounds)), brush)
	controlDeleteObject.Call(brush)
}

func max32(left, right int32) int32 {
	if left > right {
		return left
	}
	return right
}

func scaledPixels(logical int32, scale float64) int32 {
	if scale <= 0 {
		scale = 1
	}
	return int32(float64(logical)*scale + 0.5)
}
