package controlpanel

import (
	"unsafe"

	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/ui/colors"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"golang.org/x/sys/windows"
)

var applyPanelControlTheme = nativeform.ApplyControl

func (p *panel) refreshTheme(invalidate bool) {
	p.refreshThemeWithForce(invalidate, false)
}

func (p *panel) refreshThemeWithForce(invalidate, force bool) {
	if p.themeRefreshing {
		return
	}
	dark := p.resolveTheme()
	palette := colors.ForTheme(dark)
	// Windows broadcasts several closely related color/theme messages for one
	// user selection. Avoid rebuilding GDI resources and presenting duplicate
	// frames once the requested semantic palette is already active.
	if !force && invalidate && p.backgroundBrush != 0 && p.themeDark == dark && p.palette == palette {
		return
	}
	p.themeRefreshing = true
	defer func() { p.themeRefreshing = false }()
	var transition *nativeform.FrameTransition
	if invalidate && p.hwnd != 0 {
		value := nativeform.BeginFrameTransition(p.hwnd)
		transition = &value
	}
	p.closeChoice(false)
	p.themeDark = dark
	p.setWindowIcons(dark, false)
	p.palette = palette
	p.rebuildBrushes(p.palette)
	p.applyFrameTheme(dark)
	// SetWindowTheme is sticky for child controls. Reapply it inside the
	// cloaked transaction so native hot/focus state is rebuilt for the same
	// palette as the owner-drawn frame instead of remaining stale until restart.
	for _, control := range p.controls {
		applyPanelControlTheme(control, dark)
	}
	p.applyTooltipTheme(dark)
	p.syncViewport()
	if transition != nil {
		// Mark the parent background for erase before PresentFrame synchronously
		// paints every child. The compositor stays cloaked until the complete new
		// palette exists, so no default/black child-control frame can leak out.
		pInvalidateRect.Call(uintptr(p.hwnd), 0, 1)
		for attempt := 1; attempt <= 3; attempt++ {
			if err := transition.Commit(p.frameControls()...); err == nil {
				return
			} else {
				mylog.Info("Control panel atomic theme presentation failed (attempt %d): %v", attempt, err)
			}
		}
		// A window left cloaked after repeated DWM failures cannot be recovered by
		// another invalidation. Destroy it so the next tray click opens a clean one.
		pDestroyWindow.Call(uintptr(p.hwnd))
	}
}

func (p *panel) applyTooltipTheme(dark bool) {
	if p.tooltip == 0 {
		return
	}
	if dark {
		if name, err := windows.UTF16PtrFromString("DarkMode_Explorer"); err == nil {
			pSetWindowTheme.Call(uintptr(p.tooltip), uintptr(unsafe.Pointer(name)), 0)
		}
	} else {
		pSetWindowTheme.Call(uintptr(p.tooltip), 0, 0)
	}
	pSendMessage.Call(uintptr(p.tooltip), wmSetFont, uintptr(p.font), 0)
	pSendMessage.Call(uintptr(p.tooltip), ttmSetTipBkColor, uintptr(p.palette.TooltipBackground), 0)
	pSendMessage.Call(uintptr(p.tooltip), ttmSetTipTextColor, uintptr(p.palette.TooltipText), 0)
}

func (p *panel) applyFrameTheme(dark bool) {
	if p.hwnd == 0 {
		return
	}
	// Win11 uses attribute 20. Windows 10 1809 exposed the same preference as
	// attribute 19; earlier systems ignore both calls and keep their native
	// light/classic caption safely.
	value := uint32(0)
	if dark {
		value = 1
	}
	const (
		dwmwaUseImmersiveDarkMode       = 20
		dwmwaUseImmersiveDarkModeLegacy = 19
	)
	result, _, _ := pDwmSetWindowAttribute.Call(
		uintptr(p.hwnd),
		dwmwaUseImmersiveDarkMode,
		uintptr(unsafe.Pointer(&value)),
		unsafe.Sizeof(value),
	)
	if result != 0 {
		pDwmSetWindowAttribute.Call(
			uintptr(p.hwnd),
			dwmwaUseImmersiveDarkModeLegacy,
			uintptr(unsafe.Pointer(&value)),
			unsafe.Sizeof(value),
		)
	}
}

func isDangerQuickAction(id uint16) bool { return id == idShutdown || id == idRestart }
