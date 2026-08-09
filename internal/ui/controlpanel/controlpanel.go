package controlpanel

import (
	"fmt"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
	"golang.org/x/sys/windows"
	"runtime"
	"syscall"
	"unsafe"
)

func Show(state State, onAction OnAction, langFn LangFunc) error {
	panelMu.Lock()
	if active != nil {
		hwnd := active.hwnd
		panelMu.Unlock()
		pDestroyWindow.Call(uintptr(hwnd))
		return nil
	}
	panelMu.Unlock()
	return createPanel(state, onAction, langFn)
}

// Refresh rebuilds an already visible panel instead of treating it as a
// toggle. Native controls store captions at creation time, so callers use
// this after a language change to update every caption immediately.
func Refresh(state State, onAction OnAction, langFn LangFunc) error {
	panelMu.Lock()
	old := active
	panelMu.Unlock()
	if old != nil && old.hwnd != 0 {
		pDestroyWindow.Call(uintptr(old.hwnd))
	}
	return createPanel(state, onAction, langFn)
}

func createPanel(state State, onAction OnAction, langFn LangFunc) error {
	_, err := createPanelForHost(state, onAction, langFn, 0, false)
	return err
}

// Capture creates a real panel without initializing the app. The capture
// callback owns only its temporary resources; popup always destroys the HWND
// and its fonts, brushes, and icons before returning.
func Capture(state State, langFn LangFunc, scale float64, capture func(windows.Handle) error) error {
	// Win32 window creation, synchronous control messages, and destruction must
	// stay on the same OS thread throughout a headless capture.
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	restoreTextScale := font.OverrideTextScaleFactor(1)
	defer restoreTextScale()

	p, err := createPanelForHost(state, nil, langFn, scale, true)
	if err != nil {
		return err
	}
	defer func() {
		if p.hwnd != 0 {
			pDestroyWindow.Call(uintptr(p.hwnd))
		}
	}()
	p.present()
	return capture(p.hwnd)
}

func (p *panel) present() {
	nativeform.PresentFrame(p.hwnd, p.frameControls()...)
}

func (p *panel) frameControls() []windows.Handle {
	controls := make([]windows.Handle, 0, len(p.controls))
	for _, control := range p.controls {
		if control != 0 {
			controls = append(controls, control)
		}
	}
	return controls
}

func createPanelForHost(state State, onAction OnAction, langFn LangFunc, captureScale float64, captureHost bool) (*panel, error) {
	p := &panel{
		onAction:                onAction,
		lang:                    langFn,
		theme:                   state.Theme,
		metrics:                 newPanelMetrics(defaultPanelStyle, 1),
		themeSchedule:           state.ThemeSchedule,
		themeUnavailable:        state.ThemeUnavailable,
		themeUnavailableDetail:  state.ThemeUnavailableDetail,
		appVersion:              state.AppVersion,
		noSleepStatus:           state.NoSleepStatus,
		idleStatus:              state.IdleStatus,
		isChinese:               state.IsChinese,
		owner:                   state.Owner,
		automationCount:         state.AutomationCount,
		automationSummary:       state.AutomationSummary,
		developerCapturePanel:   state.DeveloperCapturePanel,
		developerWarningPreview: state.DeveloperWarningPreview,
		idleWarningSeconds:      state.IdleWarningSeconds,
		idleAction:              state.IdleAction,
		controls:                make(map[uint16]windows.Handle),
		labels:                  make(map[uint16]string),
		staticKinds:             make(map[uint16]staticKind),
		nextStaticID:            700,
		tooltips:                make(map[uint16][]uint16),
		toggles: map[uint16]bool{
			idNoSleep: state.NoSleepEnabled, idAutomationEnabled: state.AutomationEnabled,
			idIdle: state.IdleEnabled, idTheme: state.ThemeSwitchEnabled,
		},
		disabled:      make(map[uint16]bool),
		oldButtonProc: make(map[windows.Handle]uintptr),
		controlBounds: make(map[uint16]logicalBounds),
		choice:        choiceSurface{},
		captureScale:  captureScale,
		captureHost:   captureHost,
	}
	panelMu.Lock()
	active = p
	panelMu.Unlock()

	if err := ensureClass(); err != nil {
		clearPanel(p, 0)
		return nil, err
	}
	if err := p.create(); err != nil {
		clearPanel(p, 0)
		return nil, err
	}
	return p, nil
}

// Hide destroys the currently visible panel and releases its native resources.
func Hide() {
	panelMu.Lock()
	p := active
	panelMu.Unlock()
	if p != nil && p.hwnd != 0 {
		pDestroyWindow.Call(uintptr(p.hwnd))
	}
}

// WindowHandle returns the visible main panel HWND. Automatic-task dialogs use
// it as their owner so Windows can enforce a real modal parent relationship.
func WindowHandle() windows.Handle {
	panelMu.Lock()
	defer panelMu.Unlock()
	if active == nil {
		return 0
	}
	return active.hwnd
}

// UpdateAutomationStatus refreshes the independent automatic-task section
// without rebuilding the main panel or disturbing keyboard focus.
func UpdateAutomationStatus(enabled bool, count int, summary string) {
	panelMu.Lock()
	p := active
	if p == nil || p.automationSummaryID == 0 {
		panelMu.Unlock()
		return
	}
	id := p.automationSummaryID
	hwnd := p.controls[id]
	p.automationCount = count
	p.automationSummary = summary
	p.toggles[idAutomationEnabled] = enabled
	p.labels[id] = summary
	panelMu.Unlock()

	if hwnd != 0 {
		pInvalidateRect.Call(uintptr(hwnd), 0, 1)
	}
	if toggle := p.controls[idAutomationEnabled]; toggle != 0 {
		pInvalidateRect.Call(uintptr(toggle), 0, 1)
	}
	p.refreshTooltip(idAutomation)
	p.refreshTooltip(idAutomationEnabled)
}

// UpdatePowerManagementStatus refreshes the effective runtime state shown in
// the two power-management tooltips. The toggle visuals remain the user's
// manual configuration even while an automatic task temporarily takes over.
func UpdatePowerManagementStatus(noSleepStatus, idleStatus string) {
	panelMu.Lock()
	p := active
	if p == nil {
		panelMu.Unlock()
		return
	}
	p.noSleepStatus = noSleepStatus
	p.idleStatus = idleStatus
	id := p.powerSummaryID
	hwnd := p.controls[id]
	if id != 0 {
		p.labels[id] = p.text("power_overview_prefix") + noSleepStatus + p.text("power_overview_separator") + idleStatus
	}
	panelMu.Unlock()

	p.refreshTooltip(idNoSleep)
	p.refreshTooltip(idIdle)
	if hwnd != 0 {
		pInvalidateRect.Call(uintptr(hwnd), 0, 1)
	}
}

// UpdateThemeSchedule refreshes the already visible Day/Night schedule line.
// It is intentionally layout-preserving; callers should still recreate the
// panel when a future change needs to add or remove controls.
func UpdateThemeSchedule(text string) {
	panelMu.Lock()
	p := active
	if p == nil || p.themeScheduleID == 0 {
		panelMu.Unlock()
		return
	}
	id := p.themeScheduleID
	hwnd := p.controls[id]
	p.labels[id] = text
	panelMu.Unlock()

	if hwnd != 0 {
		pInvalidateRect.Call(uintptr(hwnd), 0, 1)
	}
}

// RefreshTheme refreshes the visible panel after the system theme changes.
// It must be called from the thread that owns the application's Win32 message
// loop.
func RefreshTheme() {
	panelMu.Lock()
	p := active
	panelMu.Unlock()
	if p != nil {
		p.refreshTheme(true)
	}
}

// RefreshThemeAfterSystemRepair forces one complete frame even when the
// semantic light/dark palette did not change. A full Windows theme-file apply
// can repaint native controls progressively; this final atomic presentation
// replaces any intermediate surface without hiding the panel for the repair.
func RefreshThemeAfterSystemRepair() {
	panelMu.Lock()
	p := active
	panelMu.Unlock()
	if p != nil {
		p.refreshThemeWithForce(true, true)
	}
}

func Destroy() {
	Hide()
}

func ensureClass() error {
	classOnce.Do(func() {
		name, err := windows.UTF16PtrFromString(panelClass)
		if err != nil {
			classErr = err
			return
		}
		module, _, _ := pGetModuleHandle.Call(0)
		fallbackIcon := classFallbackIcon(module)
		wc := wndClassExW{
			Size:      uint32(unsafe.Sizeof(wndClassExW{})),
			WndProc:   windows.NewCallback(wndProc),
			Instance:  windows.Handle(module),
			Icon:      fallbackIcon,
			ClassName: name,
			IconSm:    fallbackIcon,
		}
		if cursor, _, _ := pLoadCursor.Call(0, 32512); cursor != 0 {
			wc.Cursor = windows.Handle(cursor)
		}
		if result, _, callErr := pRegisterClassEx.Call(uintptr(unsafe.Pointer(&wc))); result == 0 && callErr != syscall.Errno(1410) {
			classErr = fmt.Errorf("register popup class: %w", callErr)
		}
	})
	return classErr
}

func (p *panel) create() error {
	titleText := "IdleTrigger"
	if p.appVersion != "" && p.appVersion != "dev" {
		titleText += " v" + p.appVersion
	}
	title, _ := windows.UTF16PtrFromString(titleText)
	name, _ := windows.UTF16PtrFromString(panelClass)
	style := uint32(wsPopup | wsCaption | wsSysMenu | wsClipChildren)
	exStyle := uint32(wsExTopmost)
	owner := uintptr(p.owner)
	if p.developerCapturePanel || p.captureHost {
		// Capture mode uses a normal app window shape so screenshot tools can
		// detect the whole panel instead of treating it as a transient tray popup.
		style = uint32(wsOverlappedWindow | wsClipChildren)
		exStyle = uint32(wsExAppWindow)
		owner = 0
	}
	// Create hidden on the destination monitor instead of at a default or
	// primary-monitor origin. Besides making GetDpiForWindow correct before the
	// first layout, this keeps even a compositor-retained creation rectangle
	// away from the top-left corner. The off-screen coordinate remains the safe
	// fallback when monitor discovery itself fails.
	creationX, creationY := panelFallbackWindowCoordinate, panelFallbackWindowCoordinate
	if work, ok := cursorWorkArea(0); ok {
		creationX, creationY = work.Right-1, work.Bottom-1
	}
	hwnd, _, callErr := pCreateWindowEx.Call(uintptr(exStyle), uintptr(unsafe.Pointer(name)), uintptr(unsafe.Pointer(title)), uintptr(style), uintptr(uint32(creationX)), uintptr(uint32(creationY)), 1, 1, owner, 0, 0, 0)
	if hwnd == 0 {
		return fmt.Errorf("create control panel: %w", callErr)
	}
	p.hwnd = windows.Handle(hwnd)
	firstFrame := nativeform.BeginFirstFrame(p.hwnd)
	p.style, p.exStyle = style, exStyle
	scale := dpiForWindow(p.hwnd)
	if p.captureScale > 0 {
		scale = p.captureScale
	}
	p.metrics = newPanelMetrics(defaultPanelStyle, scale)
	p.font = p.makeFont(p.metrics.style.Fonts.BodySize, p.metrics.style.Fonts.BodyWeight)
	p.sectionFont = p.makeFont(p.metrics.style.Fonts.SectionSize, p.metrics.style.Fonts.SectionWeight)
	p.subtitleFont = p.makeFont(p.metrics.style.Fonts.SubtitleSize, p.metrics.style.Fonts.SubtitleWeight)
	p.createTooltip()
	if p.font == 0 || p.sectionFont == 0 || p.subtitleFont == 0 {
		pDestroyWindow.Call(uintptr(p.hwnd))
		return fmt.Errorf("create control panel fonts failed")
	}
	mylog.Info("UI font: surface=popup ui_language=%s system_language=%s system_locale=%s face=%q reason=%s dpi=%d body_px=%d", p.fontChoice.UILanguage, p.fontChoice.SystemLanguage, p.fontChoice.SystemLocale, p.fontChoice.Face, p.fontChoice.Reason, int(p.metrics.scale*96+0.5), p.sc(int(p.metrics.style.Fonts.BodySize)))
	p.refreshTheme(false)
	if err := p.build(); err != nil {
		pDestroyWindow.Call(uintptr(p.hwnd))
		return err
	}
	if !p.captureHost {
		trayicon.SetTabNavigationWindow(p.hwnd, p.enterKeyboardNavigation)
	}
	// Position the still-hidden top-level window first. This mirrors the native
	// form windows and prevents a cold-start frame at the temporary creation
	// coordinates from reaching the desktop compositor.
	if err := p.position(style, exStyle); err != nil {
		pDestroyWindow.Call(uintptr(p.hwnd))
		return err
	}
	if err := firstFrame.Reveal(nativeform.FirstFrameOptions{
		RepeatShow: p.captureHost,
		Controls:   p.frameControls(),
	}); err != nil {
		pDestroyWindow.Call(uintptr(p.hwnd))
		return err
	}
	if !p.captureHost {
		pSetForeground.Call(uintptr(p.hwnd))
	}
	return nil
}

func dpiForWindow(hwnd windows.Handle) float64 {
	dpi, _, _ := pGetDpiForWindow.Call(uintptr(hwnd))
	if dpi == 0 {
		dpi, _, _ = pGetDpiForSystem.Call()
	}
	if dpi == 0 {
		return 1
	}
	return float64(dpi) / 96
}

func (p *panel) makeFont(size int32, weight int32) windows.Handle {
	font, choice := font.New(int32(float64(size)*p.metrics.scale+0.5), weight, p.isChinese)
	if p.fontChoice.Face == "" {
		p.fontChoice = choice
	}
	return font
}

func (p *panel) sc(value int) int { return p.metrics.px(value) }
func (p *panel) text(key string) string {
	if p.lang == nil {
		return key
	}
	return p.lang(key)
}

func (p *panel) child(className, text string, style uint32, x, y, width, height int, id uint16, font windows.Handle) (windows.Handle, error) {
	class, err := windows.UTF16PtrFromString(className)
	if err != nil {
		return 0, err
	}
	caption, err := windows.UTF16PtrFromString(text)
	if err != nil {
		return 0, err
	}
	// Owner-drawn siblings repaint independently during DPI and theme changes;
	// clipping keeps their buffered paint regions isolated.
	style |= wsClipSiblings
	hwnd, _, callErr := pCreateWindowEx.Call(0, uintptr(unsafe.Pointer(class)), uintptr(unsafe.Pointer(caption)), uintptr(style), uintptr(p.sc(x)), uintptr(p.sc(y)), uintptr(p.sc(width)), uintptr(p.sc(height)), uintptr(p.hwnd), uintptr(id), 0, 0)
	if hwnd == 0 {
		return 0, fmt.Errorf("create %s control: %w", className, callErr)
	}
	if font != 0 {
		pSendMessage.Call(hwnd, wmSetFont, uintptr(font), 1)
	}
	if id != 0 {
		p.controls[id] = windows.Handle(hwnd)
		p.controlBounds[id] = logicalBounds{x, y, width, height}
		p.labels[id] = text
		p.addTooltip(id, windows.Handle(hwnd))
	}
	return windows.Handle(hwnd), nil
}
