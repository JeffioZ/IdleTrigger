package controlpanel

import (
	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/ui/colors"
)

// panelResources is the complete set of native resources owned by one panel.
// HWND children, including the tooltip, are owned by the panel HWND and are
// destroyed by Windows with their parent. Fonts, brushes, and icons below are
// created by IdleTrigger and must be explicitly released exactly once.
type panelResources struct {
	font, sectionFont, subtitleFont windows.Handle

	backgroundBrush    windows.Handle
	surfaceBrush       windows.Handle
	hoverBrush         windows.Handle
	subtleBorderBrush  windows.Handle
	accentBrush        windows.Handle
	pressedBrush       windows.Handle
	disabledBrush      windows.Handle
	dangerBrush        windows.Handle
	dangerHoverBrush   windows.Handle
	dangerPressedBrush windows.Handle

	largeIcon, smallIcon windows.Handle
}

func (p *panel) ownedBrushes() []windows.Handle {
	return []windows.Handle{
		p.backgroundBrush, p.surfaceBrush, p.hoverBrush, p.subtleBorderBrush,
		p.accentBrush, p.pressedBrush, p.disabledBrush, p.dangerBrush,
		p.dangerHoverBrush, p.dangerPressedBrush,
	}
}

func (p *panel) releaseBrushes() {
	for _, brush := range p.ownedBrushes() {
		if brush != 0 {
			pDeleteObject.Call(uintptr(brush))
		}
	}
	p.backgroundBrush = 0
	p.surfaceBrush = 0
	p.hoverBrush = 0
	p.subtleBorderBrush = 0
	p.accentBrush = 0
	p.pressedBrush = 0
	p.disabledBrush = 0
	p.dangerBrush = 0
	p.dangerHoverBrush = 0
	p.dangerPressedBrush = 0
}

func (p *panel) rebuildBrushes(palette colors.Palette) {
	p.releaseBrushes()
	p.backgroundBrush = makeBrush(palette.WindowBackground)
	p.surfaceBrush = makeBrush(palette.Surface)
	p.hoverBrush = makeBrush(palette.HoverSurface)
	p.subtleBorderBrush = makeBrush(palette.SubtleBorder)
	p.accentBrush = makeBrush(palette.Accent)
	p.pressedBrush = makeBrush(palette.AccentPressed)
	p.disabledBrush = makeBrush(palette.DisabledSurface)
	p.dangerBrush = makeBrush(palette.DangerBackground)
	p.dangerHoverBrush = makeBrush(palette.DangerHover)
	p.dangerPressedBrush = makeBrush(palette.DangerPressed)
}

func (p *panel) releaseFonts() {
	for _, font := range []windows.Handle{p.font, p.sectionFont, p.subtitleFont} {
		if font != 0 {
			pDeleteObject.Call(uintptr(font))
		}
	}
	p.font, p.sectionFont, p.subtitleFont = 0, 0, 0
}

func (p *panel) releaseIcons() {
	for _, icon := range []windows.Handle{p.largeIcon, p.smallIcon} {
		if icon != 0 {
			pDestroyIcon.Call(uintptr(icon))
		}
	}
	p.largeIcon, p.smallIcon = 0, 0
	p.iconsInitialized = false
	p.iconThemeDark = false
}

// releaseNativeResources is idempotent and is called only after the panel is
// no longer active. It never destroys the HWND itself; its caller owns that
// lifecycle boundary.
func (p *panel) releaseNativeResources() {
	p.releaseFonts()
	p.releaseBrushes()
	p.releaseIcons()
}

func makeBrush(color uint32) windows.Handle {
	result, _, _ := pCreateBrush.Call(uintptr(color))
	return windows.Handle(result)
}
