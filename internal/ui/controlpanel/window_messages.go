package controlpanel

import (
	"fmt"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"golang.org/x/sys/windows"
	"unsafe"
)

func (p *panel) setHover(hwnd windows.Handle) {
	id := p.controlID(hwnd)
	if id == 0 {
		return
	}
	if p.hoverID != id {
		previous := p.hoverID
		p.hoverID = id
		if previous != 0 {
			p.invalidate(previous)
		}
		p.invalidate(id)
	}
	tme := trackMouseEvent{Size: uint32(unsafe.Sizeof(trackMouseEvent{})), Flags: tmeLeave, HwndTrack: hwnd}
	pTrackMouseEvent.Call(uintptr(unsafe.Pointer(&tme)))
}

func (p *panel) clearHover(hwnd windows.Handle) {
	id := p.controlID(hwnd)
	if id != 0 && p.hoverID == id {
		p.hoverID = 0
		p.invalidate(id)
	}
}

func (p *panel) controlID(hwnd windows.Handle) uint16 {
	return p.hwndToControlID[hwnd]
}

func panelOrigin(work rect, width, height, margin int32) (int32, int32) {
	x := work.Right - width - margin
	y := work.Bottom - height - margin
	if x < work.Left {
		x = work.Left
	}
	if y < work.Top {
		y = work.Top
	}
	return x, y
}

func cursorWorkArea(fallback windows.Handle) (rect, bool) {
	var cursor struct{ X, Y int32 }
	monitor := uintptr(0)
	if result, _, _ := pGetCursorPos.Call(uintptr(unsafe.Pointer(&cursor))); result != 0 {
		anchor := rect{Left: cursor.X, Top: cursor.Y, Right: cursor.X + 1, Bottom: cursor.Y + 1}
		monitor, _, _ = pMonitorFromRect.Call(uintptr(unsafe.Pointer(&anchor)), monitorNearest)
	}
	if monitor == 0 && fallback != 0 {
		monitor, _, _ = pMonitorFromWindow.Call(uintptr(fallback), monitorNearest)
	}
	if monitor == 0 {
		return rect{}, false
	}
	info := monitorInfo{Size: uint32(unsafe.Sizeof(monitorInfo{}))}
	if result, _, _ := pGetMonitorInfo.Call(monitor, uintptr(unsafe.Pointer(&info))); result == 0 {
		return rect{}, false
	}
	return info.Work, info.Work.Right > info.Work.Left && info.Work.Bottom > info.Work.Top
}

func (p *panel) position(style, exStyle uint32) error {
	clientWidth, clientHeight := p.sc(p.metrics.style.Layout.PanelWidth), p.sc(p.clientH)
	windowDPI := uint32(dpiForWindow(p.hwnd)*96 + 0.5)
	width, height, err := nativeform.WindowSizeForClient(clientWidth, clientHeight, uintptr(style), uintptr(exStyle), windowDPI)
	if err != nil {
		return fmt.Errorf("calculate control panel bounds: %w", err)
	}
	work, ok := cursorWorkArea(p.hwnd)
	if !ok {
		return fmt.Errorf("locate control panel monitor work area")
	}
	width, height = min(width, work.Right-work.Left), min(height, work.Bottom-work.Top)
	x, y := panelOrigin(work, width, height, int32(p.sc(16)))
	insertAfter := ^uintptr(0)
	if p.developerCapturePanel || p.captureHost {
		insertAfter = 0
	}
	// Keep placement and visibility as two distinct commits. Combining them
	// with SWP_SHOWWINDOW can let DWM compose the newly visible cold-start frame
	// at CreateWindowEx's temporary coordinates before the final position is
	// applied. The caller explicitly shows the fully positioned window.
	if result, _, callErr := pSetWindowPos.Call(uintptr(p.hwnd), insertAfter, uintptr(x), uintptr(y), uintptr(width), uintptr(height), swpNoActivate); result == 0 {
		return fmt.Errorf("position control panel: %w", callErr)
	}
	p.syncViewport()
	return nil
}

func suggestedDPIBounds(lp uintptr) (rect, bool) {
	if lp == 0 {
		return rect{}, false
	}
	suggested := *(*rect)(nativeform.MessagePointer(lp))
	return suggested, suggested.Right > suggested.Left && suggested.Bottom > suggested.Top
}

func panelScaleForDPI(dpi uint32, captureScale float64) float64 {
	if captureScale > 0 {
		return captureScale
	}
	if dpi == 0 {
		dpi = 96
	}
	return float64(dpi) / 96
}

func (p *panel) desiredWindowSizeForDPI(dpi uint32) (int32, int32, error) {
	return p.desiredWindowSize(dpi, dpi)
}

func (p *panel) desiredWindowSize(layoutDPI, frameDPI uint32) (int32, int32, error) {
	scale := (panelScaleForDPI(layoutDPI, p.captureScale) * max(1, p.textScale))
	effectiveDPI := frameDPI
	if p.captureScale > 0 {
		effectiveDPI = uint32(p.captureScale*96 + 0.5)
	}
	if effectiveDPI == 0 {
		effectiveDPI = 96
	}
	clientWidth := int(float64(p.metrics.style.Layout.PanelWidth)*scale + 0.5)
	clientHeight := int(float64(p.clientH)*scale + 0.5)
	return nativeform.WindowSizeForClient(clientWidth, clientHeight, uintptr(p.style), uintptr(p.exStyle), effectiveDPI)
}

func (p *panel) applyDPIWindowSize(wp, lp uintptr) bool {
	if lp == 0 {
		return false
	}
	dpi := uint32(wp)
	if dpi == 0 {
		dpi = 96
	}
	width, height, err := p.desiredWindowSizeForDPI(dpi)
	if err != nil {
		return false
	}
	size := (*windowSize)(nativeform.MessagePointer(lp))
	size.Width, size.Height = width, height
	return true
}

func (p *panel) applySuggestedDPIBounds(suggested rect, ok bool) error {
	if !ok {
		return nil
	}
	if result, _, callErr := pSetWindowPos.Call(
		uintptr(p.hwnd), 0,
		uintptr(suggested.Left), uintptr(suggested.Top),
		uintptr(suggested.Right-suggested.Left), uintptr(suggested.Bottom-suggested.Top),
		swpNoZOrder|swpNoActivate,
	); result == 0 {
		return fmt.Errorf("apply suggested control panel DPI bounds: %w", callErr)
	}
	return nil
}

func (p *panel) queueDPIChange(wp, lp uintptr) {
	dpi := uint32(wp & 0xffff)
	if dpi == 0 {
		dpi = 96
	}
	suggested, hasSuggested := suggestedDPIBounds(lp)
	p.pendingDPI = dpi
	p.pendingDPISuggested = suggested
	p.pendingDPIHasSuggested = hasSuggested
	p.dpiGeneration++
	if p.dpiTransition == nil {
		transition := nativeform.BeginFrameTransition(p.hwnd)
		p.dpiTransition = &transition
	}
	if !p.dpiApplyPosted {
		if posted, _, _ := pPostMessage.Call(uintptr(p.hwnd), wmApplyDPI, 0, 0); posted != 0 {
			p.dpiApplyPosted = true
		}
	}
	// Acknowledge the system-provided bounds inside WM_DPICHANGED as required.
	// The window remains cloaked; the queued transaction will replace the size
	// with one calculated from the final DPI after any nested messages unwind.
	if err := p.applySuggestedDPIBounds(suggested, hasSuggested); err != nil {
		mylog.Info("Control panel suggested DPI placement failed: %v", err)
	}
	if !p.dpiApplyPosted {
		p.applyPendingDPI()
	}
}

func (p *panel) ensureDPIApplyPosted() {
	if p.dpiApplyPosted || p.hwnd == 0 {
		return
	}
	if posted, _, _ := pPostMessage.Call(uintptr(p.hwnd), wmApplyDPI, 0, 0); posted != 0 {
		p.dpiApplyPosted = true
		return
	}
	p.applyPendingDPI()
}

func (p *panel) ensureDPICommitPosted() {
	if p.dpiCommitPosted || p.hwnd == 0 {
		return
	}
	if posted, _, _ := pPostMessage.Call(uintptr(p.hwnd), wmCommitDPI, 0, 0); posted != 0 {
		p.dpiCommitPosted = true
		return
	}
	p.commitPendingDPI()
}

func (p *panel) positionForResolvedDPI(layoutDPI, frameDPI uint32, suggested rect, hasSuggested bool) error {
	width, height, err := p.desiredWindowSize(layoutDPI, frameDPI)
	if err != nil {
		return fmt.Errorf("calculate resolved control panel bounds: %w", err)
	}
	var monitor uintptr
	if hasSuggested {
		monitor, _, _ = pMonitorFromRect.Call(uintptr(unsafe.Pointer(&suggested)), monitorNearest)
	}
	if monitor == 0 {
		monitor, _, _ = pMonitorFromWindow.Call(uintptr(p.hwnd), monitorNearest)
	}
	if monitor == 0 {
		return fmt.Errorf("locate resolved control panel monitor")
	}
	info := monitorInfo{Size: uint32(unsafe.Sizeof(monitorInfo{}))}
	if result, _, callErr := pGetMonitorInfo.Call(monitor, uintptr(unsafe.Pointer(&info))); result == 0 {
		return fmt.Errorf("read resolved control panel work area: %w", callErr)
	}
	margin := int32(float64(16)*(panelScaleForDPI(layoutDPI, p.captureScale)*max(1, p.textScale)) + 0.5)
	width, height = min(width, info.Work.Right-info.Work.Left), min(height, info.Work.Bottom-info.Work.Top)
	x, y := panelOrigin(info.Work, width, height, margin)
	if result, _, callErr := pSetWindowPos.Call(
		uintptr(p.hwnd), 0, uintptr(x), uintptr(y), uintptr(width), uintptr(height),
		swpNoZOrder|swpNoActivate,
	); result == 0 {
		return fmt.Errorf("position resolved control panel DPI bounds: %w", callErr)
	}
	p.syncViewport()
	return nil
}

func (p *panel) applyPendingDPI() {
	p.dpiApplyPosted = false
	if p.hwnd == 0 || p.pendingDPI == 0 {
		return
	}
	generation := p.dpiGeneration
	dpi := p.pendingDPI
	suggested, hasSuggested := p.pendingDPISuggested, p.pendingDPIHasSuggested
	resolvedDPI := dpi
	if !p.refreshFontsForDPI(dpi) {
		// Keep the outer frame consistent with the metrics that remain active
		// when Windows cannot allocate the replacement GDI fonts.
		resolvedDPI = uint32(p.metrics.scale/max(1, p.textScale)*96 + 0.5)
		mylog.Info("Control panel DPI font rebuild failed; retaining dpi=%d", resolvedDPI)
	}
	if err := p.positionForResolvedDPI(resolvedDPI, dpi, suggested, hasSuggested); err != nil {
		mylog.Info("Control panel resolved DPI placement failed: %v", err)
	}
	if p.dpiGeneration != generation {
		p.ensureDPIApplyPosted()
		return
	}
	p.dpiReadyGeneration = generation
	p.ensureDPICommitPosted()
}

func (p *panel) dpiFrameReady() bool {
	return p.pendingDPI != 0 && p.dpiReadyGeneration == p.dpiGeneration
}

func (p *panel) commitPendingDPI() {
	p.dpiCommitPosted = false
	if p.pendingDPI == 0 {
		return
	}
	if !p.dpiFrameReady() {
		p.ensureDPIApplyPosted()
		return
	}
	transition := p.dpiTransition
	if transition != nil {
		if err := transition.Commit(p.frameControls()...); err != nil {
			p.dpiCommitFailures++
			mylog.Info("Control panel atomic DPI presentation failed: %v", err)
			if p.dpiCommitFailures < 3 {
				p.ensureDPICommitPosted()
				return
			}
			// A window which DWM still considers cloaked cannot be recovered by
			// another paint. Destroy it rather than leave an invisible active
			// panel; the next tray click creates a clean HWND.
			mylog.Info("Control panel DPI presentation abandoned after %d attempts", p.dpiCommitFailures)
			pDestroyWindow.Call(uintptr(p.hwnd))
			return
		}
	} else {
		p.present()
	}
	p.dpiTransition = nil
	p.pendingDPI = 0
	p.dpiCommitFailures = 0
	mylog.Info("UI font: surface=popup rebuilt reason=dpi-change dpi=%d face=%q client_px=%dx%d generation=%d", int(p.metrics.scale*96+0.5), p.fontChoice.Face, p.sc(p.metrics.style.Layout.PanelWidth), p.sc(p.clientH), p.dpiReadyGeneration)
}

func wndProc(hwnd windows.Handle, msg uint32, wp, lp uintptr) uintptr {
	p := panelFor(hwnd)
	if p != nil {
		switch msg {
		case wmClose:
			Hide()
			return 0
		case wmActivate:
			if uint16(wp) == waInactive && !nativeform.IsChoicePopupOwnedBy(windows.Handle(lp), p.hwnd) {
				p.closeOpenMenus()
			}
		case wmNcLButtonDown:
			p.closeOpenMenus()
		case wmLButtonDown:
			p.leaveKeyboardNavigation()
			p.closeOpenMenus()
		case wmParentNotify:
			if uint16(wp) == wmLButtonDown {
				p.leaveKeyboardNavigation()
			}
		case wmOpenChoice:
			if uint16(wp) == idQuickActions {
				p.openQuickMenu()
			}
			return 0
		case 0x020A: // WM_MOUSEWHEEL
			if p.viewport != nil && p.viewport.Wheel(wp) {
				return 0
			}
		case wmDestroy:
			if p.viewport != nil {
				p.viewport.Close()
				p.viewport = nil
			}
			clearPanel(p, hwnd)
		case wmEraseBkgnd:
			p.fill(windows.Handle(wp), p.backgroundBrush)
			return 1
		case wmCtlColorStatic:
			pSetTextColor.Call(wp, uintptr(p.palette.PrimaryText))
			pSetBkMode.Call(wp, transparent)
			return uintptr(p.backgroundBrush)
		case wmCtlColorEdit, wmCtlColorListBox:
			pSetTextColor.Call(wp, uintptr(p.palette.PrimaryText))
			pSetBkColor.Call(wp, uintptr(p.palette.Surface))
			return uintptr(p.surfaceBrush)
		case wmDrawItem:
			if lp != 0 {
				p.drawItemBuffered(drawItemFromLParam(lp))
				return 1
			}
		case wmSettingChange, wmSysColorChange, wmThemeChanged:
			if msg == wmSettingChange && p.font != 0 {
				next := font.TextScaleFactor()
				if next != p.textScale {
					previous := p.textScale
					p.textScale = next
					if p.refreshFontsForDPI(uint32(dpiForWindow(p.hwnd)*96 + 0.5)) {
						_ = p.position(p.style, p.exStyle)
					} else {
						p.textScale = previous
					}
				}
			}
			p.refreshTheme(true)
			// Theme application already updates every child and commits a complete
			// frame. Letting DefWindowProc process the same broadcast afterward can
			// reopen native hot-state painting outside that atomic transaction.
			return 0
		case wmGetDpiScaledSize:
			if p.applyDPIWindowSize(wp, lp) {
				return 1
			}
		case wmDpiChanged:
			p.queueDPIChange(wp, lp)
			return 0
		case wmApplyDPI:
			p.applyPendingDPI()
			return 0
		case wmCommitDPI:
			p.commitPendingDPI()
			return 0
		case wmCommand:
			id, notification := uint16(wp), uint16(wp>>16)
			if notification == bnClicked {
				p.handleCommand(id)
				return 0
			}
		}
	}
	result, _, _ := pDefWindowProc.Call(uintptr(hwnd), uintptr(msg), wp, lp)
	return result
}

func (p *panel) refreshFontsForDPI(dpi uint32) bool {
	p.closeChoice(false)
	newScale := panelScaleForDPI(dpi, p.captureScale) * max(1, p.textScale)
	if newScale <= 0 || newScale == p.metrics.scale {
		return true
	}
	oldMetrics, oldChoice := p.metrics, p.fontChoice
	p.metrics = newPanelMetrics(p.metrics.style, newScale)
	p.fontChoice = font.Choice{}
	newFont := p.makeFont(p.metrics.style.Fonts.BodySize, p.metrics.style.Fonts.BodyWeight)
	newSectionFont := p.makeFont(p.metrics.style.Fonts.SectionSize, p.metrics.style.Fonts.SectionWeight)
	newSubtitleFont := p.makeFont(p.metrics.style.Fonts.SubtitleSize, p.metrics.style.Fonts.SubtitleWeight)
	if newFont == 0 || newSectionFont == 0 || newSubtitleFont == 0 {
		for _, font := range []windows.Handle{newFont, newSectionFont, newSubtitleFont} {
			if font != 0 {
				pDeleteObject.Call(uintptr(font))
			}
		}
		p.metrics, p.fontChoice = oldMetrics, oldChoice
		return false
	}
	oldFont, oldSection, oldSubtitle := p.font, p.sectionFont, p.subtitleFont
	p.font, p.sectionFont, p.subtitleFont = newFont, newSectionFont, newSubtitleFont
	p.setWindowIcons(p.resolveTheme(), true)
	p.positionDPIControls()
	for id, hwnd := range p.controls {
		if p.staticKinds[id] == staticNone {
			// The DPI transaction presents the complete frame after every child
			// has its final bounds and font. Avoid one eager repaint per button.
			pSendMessage.Call(uintptr(hwnd), wmSetFont, uintptr(p.font), 0)
		}
	}
	if p.tooltip != 0 {
		pSendMessage.Call(uintptr(p.tooltip), wmSetFont, uintptr(p.font), 0)
		pSendMessage.Call(uintptr(p.tooltip), ttmSetMaxTipWidth, 0, uintptr(p.sc(360)))
	}
	for _, font := range []windows.Handle{oldFont, oldSection, oldSubtitle} {
		if font != 0 {
			pDeleteObject.Call(uintptr(font))
		}
	}
	return true
}

func (p *panel) positionDPIControls() {
	x, y := 0, 0
	if p.viewport != nil {
		x, y = p.viewport.X, p.viewport.Y
	}
	flags := uintptr(swpNoZOrder | swpNoActivate)
	deferred, _, _ := pBeginDeferWindowPos.Call(uintptr(len(p.controlBounds)))
	if deferred != 0 {
		for id, bounds := range p.controlBounds {
			hwnd := p.controls[id]
			if hwnd == 0 {
				continue
			}
			next, _, _ := pDeferWindowPos.Call(
				deferred, uintptr(hwnd), 0,
				uintptr(p.sc(bounds.x-x)), uintptr(p.sc(bounds.y-y)),
				uintptr(p.sc(bounds.width)), uintptr(p.sc(bounds.height)), flags,
			)
			if next == 0 {
				deferred = 0
				break
			}
			deferred = next
		}
		if deferred != 0 {
			if committed, _, _ := pEndDeferWindowPos.Call(deferred); committed != 0 {
				return
			}
		}
	}

	// Resource exhaustion can make the deferred-window transaction unavailable.
	// Preserve the previous positioning behavior as a complete fallback.
	for id, bounds := range p.controlBounds {
		if hwnd := p.controls[id]; hwnd != 0 {
			pSetWindowPos.Call(
				uintptr(hwnd), 0,
				uintptr(p.sc(bounds.x-x)), uintptr(p.sc(bounds.y-y)),
				uintptr(p.sc(bounds.width)), uintptr(p.sc(bounds.height)), flags,
			)
		}
	}
}

type drawItemPointer *drawItem

func drawItemFromLParam(lp uintptr) *drawItem {
	return *(*drawItemPointer)(unsafe.Pointer(&lp))
}

func (p *panel) syncViewport() {
	if p.viewport == nil {
		return
	}
	p.viewport.Sync(p.hwnd, p.metrics.scale, p.metrics.style.Layout.PanelWidth, p.clientH, p.palette)
	p.positionDPIControls()
	pInvalidateRect.Call(uintptr(p.hwnd), 0, 0)
}
