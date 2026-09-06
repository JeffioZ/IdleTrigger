// Package actionwarning displays an explicit, cancellable countdown for a
// scheduled system action. Unlike the idle warning, keyboard or mouse activity
// does not silently cancel it.
package actionwarning

import (
	"fmt"
	"strings"
	"sync"
	"sync/atomic"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	"github.com/JeffioZ/idletrigger/internal/ui/colors"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

type Options struct {
	Title, CancelText, ExecuteText string
	Seconds                        int
	Body                           func(int) string
	OnCancel, OnExecute            func()
}

type rect struct{ Left, Top, Right, Bottom int32 }
type point struct{ X, Y int32 }
type size struct{ Width, Height int32 }
type monitorInfo struct {
	Size          uint32
	Monitor, Work rect
	Flags         uint32
}
type wndClassEx struct {
	Size, Style              uint32
	WndProc                  uintptr
	ClsExtra, WndExtra       int32
	Instance                 windows.Handle
	Icon, Cursor, Background windows.Handle
	MenuName, ClassName      *uint16
	IconSm                   windows.Handle
}
type toolInfo struct {
	Size     uint32
	Flags    uint32
	Hwnd     windows.Handle
	ID       uintptr
	Rect     rect
	Instance windows.Handle
	Text     *uint16
	LParam   uintptr
	Reserved uintptr
}

const (
	warningWidth        = 390
	warningPadding      = 16
	warningBodyHeight   = 76
	warningButtonGap    = 8
	warningButtonWidth  = nativeform.DialogButtonWidth
	warningButtonHeight = nativeform.ButtonHeight
	warningBodyX        = warningPadding
	warningBodyY        = warningPadding
	warningBodyWidth    = warningWidth - 2*warningPadding
	warningButtonsY     = warningBodyY + warningBodyHeight + warningPadding
	warningExecuteX     = warningWidth - warningPadding - warningButtonWidth
	warningCancelX      = warningExecuteX - warningButtonGap - warningButtonWidth
	warningHeight       = warningButtonsY + warningButtonHeight + warningPadding
)

const (
	windowClass           = "IdleTriggerActionWarning"
	idBody                = 101
	idCancel              = 102
	idExecute             = 103
	wmDestroy             = 0x0002
	wmClose               = 0x0010
	wmPaint               = 0x000F
	wmEraseBkgnd          = 0x0014
	wmCommand             = 0x0111
	wmCtlColorStatic      = 0x0138
	wmCtlColorButton      = 0x0135
	wmSysColorChange      = 0x0015
	wmSettingChange       = 0x001A
	wmThemeChanged        = 0x031A
	wmDpiChanged          = 0x02E0
	wmSetFont             = 0x0030
	wsPopup               = 0x80000000
	wsCaption             = 0x00C00000
	wsSysMenu             = 0x00080000
	wsChild               = 0x40000000
	wsVisible             = 0x10000000
	wsTabStop             = 0x00010000
	wsClipChildren        = 0x02000000
	wsVScroll             = 0x00200000
	esMultiline           = 0x0004
	esAutoVScroll         = 0x0040
	esReadOnly            = 0x0800
	emGetFirstVisibleLine = 0x00CE
	emLineScroll          = 0x00B6
	bsPushButton          = 0x00000000
	wsExTopmost           = 0x00000008
	wsExComposited        = 0x02000000
	swpNoActivate         = 0x0010
	swpNoZOrder           = 0x0004
	swpShowWindow         = 0x0040
	monitorNearest        = 2
	ttfIDIsHwnd           = 0x0001
	ttfSubclass           = 0x0010
	ttmAddTool            = 0x0432
	ttmSetMaxTipWidth     = 0x0418
	ttsAlwaysTip          = 0x01
	ttsNoPrefix           = 0x02
	transparent           = 1
)

var (
	user32               = windows.NewLazySystemDLL("user32.dll")
	gdi32                = windows.NewLazySystemDLL("gdi32.dll")
	pCreateWindowEx      = user32.NewProc("CreateWindowExW")
	pDestroyWindow       = user32.NewProc("DestroyWindow")
	pDefWindowProc       = user32.NewProc("DefWindowProcW")
	pRegisterClassEx     = user32.NewProc("RegisterClassExW")
	pSetWindowText       = user32.NewProc("SetWindowTextW")
	pSetWindowPos        = user32.NewProc("SetWindowPos")
	pSetForegroundWindow = user32.NewProc("SetForegroundWindow")
	pSetFocus            = user32.NewProc("SetFocus")
	pShowWindow          = user32.NewProc("ShowWindow")
	pUpdateWindow        = user32.NewProc("UpdateWindow")
	pGetDpiForWindow     = user32.NewProc("GetDpiForWindow")
	pGetCursorPos        = user32.NewProc("GetCursorPos")
	pMonitorFromWindow   = user32.NewProc("MonitorFromWindow")
	pMonitorFromRect     = user32.NewProc("MonitorFromRect")
	pGetMonitorInfo      = user32.NewProc("GetMonitorInfoW")
	pGetClientRect       = user32.NewProc("GetClientRect")
	pGetDC               = user32.NewProc("GetDC")
	pReleaseDC           = user32.NewProc("ReleaseDC")
	pFillRect            = user32.NewProc("FillRect")
	pInvalidateRect      = user32.NewProc("InvalidateRect")
	pSendMessage         = user32.NewProc("SendMessageW")
	pSetTextColor        = gdi32.NewProc("SetTextColor")
	pSetBkMode           = gdi32.NewProc("SetBkMode")
	pCreateBrush         = gdi32.NewProc("CreateSolidBrush")
	pDeleteObject        = gdi32.NewProc("DeleteObject")
	pSelectObject        = gdi32.NewProc("SelectObject")
	pGetTextExtent       = gdi32.NewProc("GetTextExtentPoint32W")
	pDrawText            = user32.NewProc("DrawTextW")

	classOnce      sync.Once
	classErr       error
	active         windows.Handle
	bodyControl    windows.Handle
	cancelControl  windows.Handle
	executeControl windows.Handle
	tooltip        windows.Handle
	tooltipText    [][]uint16
	uiFont         windows.Handle
	background     windows.Handle
	uiPalette      colors.Palette
	themeDark      bool
	windowIcons    nativeform.WindowIcons
	current        Options
	currentSeq     uint64
	nextSeq        atomic.Uint64
	countdown      nativeform.CountdownCancellation
	finished       atomic.Bool
	languageMu     sync.RWMutex
	uiChinese      *bool
	dpiScale       float64
	textScale      float64
	bodyWidth      int32
	bodyText       string
	wndCallback    = windows.NewCallback(wndProc)
)

func SetLanguage(chinese bool) {
	languageMu.Lock()
	uiChinese = &chinese
	languageMu.Unlock()
}

func Show(options Options) {
	if options.Body == nil {
		return
	}
	if options.Seconds < 0 {
		options.Seconds = 0
	}
	seq := nextSeq.Add(1)
	trayicon.Post(func() {
		showNow(options, seq)
		if active == 0 || currentSeq != seq {
			return
		}
		if options.Seconds == 0 {
			complete(seq, true)
			return
		}
		startCountdown(options, seq)
	})
}

func startCountdown(options Options, seq uint64) {
	stop := countdown.Replace()
	go func() {
		ticker := time.NewTicker(time.Second)
		defer ticker.Stop()
		for remaining := options.Seconds - 1; remaining >= 0; remaining-- {
			select {
			case <-ticker.C:
			case <-stop:
				return
			}
			value := remaining
			if !trayicon.Post(func() {
				if value == 0 {
					complete(seq, true)
					return
				}
				updateBody(seq, value)
			}) {
				return
			}
			if remaining == 0 {
				return
			}
		}
	}()
}

// Hide closes the warning without invoking either action. It is used during
// application shutdown or automation restart.
func Hide() {
	trayicon.Post(func() { hideNow() })
}

func showNow(options Options, seq uint64) {
	hideNow()
	if err := ensureClass(); err != nil {
		return
	}
	class, _ := windows.UTF16PtrFromString(windowClass)
	title, _ := windows.UTF16PtrFromString(options.Title)
	var cursor point
	pGetCursorPos.Call(uintptr(unsafe.Pointer(&cursor)))
	hwnd, _, _ := pCreateWindowEx.Call(wsExTopmost|wsExComposited, uintptr(unsafe.Pointer(class)), uintptr(unsafe.Pointer(title)), wsPopup|wsCaption|wsSysMenu|wsClipChildren, uintptr(cursor.X), uintptr(cursor.Y), 1, 1, 0, 0, 0, 0)
	if hwnd == 0 {
		return
	}
	active = windows.Handle(hwnd)
	dpiScale = scaleFromWindow(active)
	textScale = font.TextScaleFactor()
	current, currentSeq = options, seq
	finished.Store(false)
	buildControls(options)
	applyTheme()
	position(nil)
	pShowWindow.Call(hwnd, 5)
	pSetForegroundWindow.Call(hwnd)
	pSetFocus.Call(uintptr(cancelControl))
	pUpdateWindow.Call(hwnd)
	trayicon.SetTabNavigationWindow(active, nil)
}

func ensureClass() error {
	classOnce.Do(func() {
		name, err := windows.UTF16PtrFromString(windowClass)
		if err != nil {
			classErr = err
			return
		}
		wc := wndClassEx{Size: uint32(unsafe.Sizeof(wndClassEx{})), WndProc: wndCallback, ClassName: name}
		result, _, callErr := pRegisterClassEx.Call(uintptr(unsafe.Pointer(&wc)))
		if result == 0 && callErr != windows.ERROR_CLASS_ALREADY_EXISTS {
			classErr = fmt.Errorf("register scheduled action window: %w", callErr)
		}
	})
	return classErr
}

func buildControls(options Options) {
	bodyWidth = 0
	bodyText = options.Body(options.Seconds)
	scale := windowScale()
	chinese := font.SystemLanguageIsChinese()
	languageMu.RLock()
	if uiChinese != nil {
		chinese = *uiChinese
	}
	languageMu.RUnlock()
	uiFont, _ = font.NewForLayout(int32(14*scale+0.5), 400, chinese)
	bodyControl = child("EDIT", "", wsChild|wsVisible|wsTabStop|wsVScroll|esMultiline|esAutoVScroll|esReadOnly, warningBodyX, warningBodyY, warningBodyWidth, warningBodyHeight, idBody)
	pSendMessage.Call(uintptr(bodyControl), 0x00D3, 3, 0) // EM_SETMARGINS: no horizontal inset
	setBodyText()
	cancelControl = child("BUTTON", options.CancelText, wsChild|wsVisible|wsTabStop|bsPushButton, warningCancelX, warningButtonsY, warningButtonWidth, warningButtonHeight, idCancel)
	executeControl = child("BUTTON", options.ExecuteText, wsChild|wsVisible|wsTabStop|bsPushButton, warningExecuteX, warningButtonsY, warningButtonWidth, warningButtonHeight, idExecute)
	createTooltips()
}

func child(className, text string, style uintptr, x, y, width, height int, id uintptr) windows.Handle {
	class, _ := windows.UTF16PtrFromString(className)
	caption, _ := windows.UTF16PtrFromString(text)
	scale := windowScale()
	hwnd, _, _ := pCreateWindowEx.Call(0, uintptr(unsafe.Pointer(class)), uintptr(unsafe.Pointer(caption)), style, uintptr(int(float64(x)*scale)), uintptr(int(float64(y)*scale)), uintptr(int(float64(width)*scale)), uintptr(int(float64(height)*scale)), uintptr(active), id, 0, 0)
	if hwnd != 0 && uiFont != 0 {
		pSendMessage.Call(hwnd, wmSetFont, uintptr(uiFont), 1)
	}
	if hwnd != 0 {
		nativeform.ApplyControl(windows.Handle(hwnd), themeDark)
	}
	return windows.Handle(hwnd)
}

func createTooltips() {
	class, _ := windows.UTF16PtrFromString("tooltips_class32")
	hwnd, _, _ := pCreateWindowEx.Call(0, uintptr(unsafe.Pointer(class)), 0, wsPopup|ttsAlwaysTip|ttsNoPrefix, 0, 0, 0, 0, uintptr(active), 0, 0, 0)
	tooltip = windows.Handle(hwnd)
	nativeform.ApplyTooltip(tooltip, themeDark, uiPalette, uiFont)
	pSendMessage.Call(hwnd, ttmSetMaxTipWidth, 0, uintptr(int(320*windowScale())))
	addTooltip(cancelControl, current.CancelText)
	addTooltip(executeControl, current.ExecuteText)
}

func addTooltip(control windows.Handle, value string) {
	if tooltip == 0 || control == 0 {
		return
	}
	text, _ := windows.UTF16FromString(value)
	tooltipText = append(tooltipText, text)
	info := toolInfo{Size: uint32(unsafe.Sizeof(toolInfo{})), Flags: ttfIDIsHwnd | ttfSubclass, Hwnd: active, ID: uintptr(control), Text: &tooltipText[len(tooltipText)-1][0]}
	pSendMessage.Call(uintptr(tooltip), ttmAddTool, 0, uintptr(unsafe.Pointer(&info)))
}

func updateBody(seq uint64, remaining int) {
	if active == 0 || currentSeq != seq || finished.Load() {
		return
	}
	bodyText = current.Body(remaining)
	setBodyText()
}

func setBodyText() {
	// EDIT needs CRLF. Preserve reading position across countdown updates.
	value := strings.ReplaceAll(strings.ReplaceAll(fitWarningBody(bodyText), "\r\n", "\n"), "\n", "\r\n")
	text, _ := windows.UTF16PtrFromString(value)
	first, _, _ := pSendMessage.Call(uintptr(bodyControl), emGetFirstVisibleLine, 0, 0)
	pSetWindowText.Call(uintptr(bodyControl), uintptr(unsafe.Pointer(text)))
	current, _, _ := pSendMessage.Call(uintptr(bodyControl), emGetFirstVisibleLine, 0, 0)
	pSendMessage.Call(uintptr(bodyControl), emLineScroll, 0, first-current)
}

func fitWarningBody(value string) string {
	maxWidth := int32(float64(warningBodyWidth)*windowScale() + 0.5)
	if bodyWidth > 0 {
		maxWidth = bodyWidth
	}
	return ellipsizeLeadingLine(value, maxWidth, measureWarningTextWidth)
}

func ellipsizeLeadingLine(value string, maxWidth int32, measure func(string) (int32, bool)) string {
	newline := strings.LastIndexByte(value, '\n')
	if newline <= 0 {
		return value
	}
	leading := strings.Join(strings.Fields(value[:newline]), " ")
	suffix := value[newline:]
	normalized := leading + suffix
	if maxWidth <= 0 || measure == nil {
		return normalized
	}
	width, ok := measure(leading)
	if !ok || width <= maxWidth {
		return normalized
	}
	runes := []rune(leading)
	const ellipsis = "…"
	low, high := 0, len(runes)
	for low < high {
		mid := (low + high + 1) / 2
		width, measured := measure(string(runes[:mid]) + ellipsis)
		if !measured {
			return normalized
		}
		if width <= maxWidth {
			low = mid
		} else {
			high = mid - 1
		}
	}
	return string(runes[:low]) + ellipsis + suffix
}

func measureWarningTextWidth(value string) (int32, bool) {
	if active == 0 || uiFont == 0 {
		return 0, false
	}
	text, err := windows.UTF16FromString(value)
	if err != nil {
		return 0, false
	}
	dc, _, _ := pGetDC.Call(uintptr(active))
	if dc == 0 {
		return 0, false
	}
	defer pReleaseDC.Call(uintptr(active), dc)
	old, _, _ := pSelectObject.Call(dc, uintptr(uiFont))
	defer pSelectObject.Call(dc, old)
	var measured size
	ok, _, _ := pGetTextExtent.Call(
		dc,
		uintptr(unsafe.Pointer(&text[0])),
		uintptr(len(text)-1),
		uintptr(unsafe.Pointer(&measured)),
	)
	return measured.Width, ok != 0
}

func complete(seq uint64, execute bool) {
	if active == 0 || currentSeq != seq || !finished.CompareAndSwap(false, true) {
		return
	}
	options := current
	hideNow()
	if execute {
		if options.OnExecute != nil {
			options.OnExecute()
		}
	} else if options.OnCancel != nil {
		options.OnCancel()
	}
}

func hideNow() {
	countdown.Stop()
	if active != 0 {
		trayicon.ClearTabNavigationWindow(active)
		pDestroyWindow.Call(uintptr(active))
	}
	active, bodyControl, cancelControl, executeControl, tooltip = 0, 0, 0, 0, 0
	dpiScale = 0
	tooltipText = nil
	currentSeq = 0
	if uiFont != 0 {
		pDeleteObject.Call(uintptr(uiFont))
		uiFont = 0
	}
	if background != 0 {
		pDeleteObject.Call(uintptr(background))
		background = 0
	}
}

func position(suggested *rect) {
	scale := windowScale()
	dpi := uint32(scale/max(1, textScale)*96 + 0.5)
	frameWidth, frameHeight, err := nativeform.WindowSizeForClient(
		1, 1,
		wsPopup|wsCaption|wsSysMenu|wsClipChildren, wsExTopmost|wsExComposited, dpi,
	)
	if err != nil {
		return
	}
	monitor := uintptr(0)
	if suggested != nil {
		monitor, _, _ = pMonitorFromRect.Call(uintptr(unsafe.Pointer(suggested)), monitorNearest)
	}
	if monitor == 0 {
		monitor, _, _ = pMonitorFromWindow.Call(uintptr(active), monitorNearest)
	}
	info := monitorInfo{Size: uint32(unsafe.Sizeof(monitorInfo{}))}
	if ok, _, _ := pGetMonitorInfo.Call(monitor, uintptr(unsafe.Pointer(&info))); ok == 0 {
		return
	}
	sc := func(v int) int32 { return int32(float64(v)*scale + 0.5) }
	margin := sc(warningPadding)
	availableWidth := max(1, info.Work.Right-info.Work.Left-(frameWidth-1))
	availableHeight := max(1, info.Work.Bottom-info.Work.Top-(frameHeight-1))
	clientWidth := min(sc(warningWidth), availableWidth)
	pad := min(margin, clientWidth/8)
	// Reserve scrollbar space when measuring, including at enlarged text sizes.
	bodyWidth = max(1, clientWidth-2*pad-sc(nativeform.ScrollbarWidth+4))
	text, _ := windows.UTF16PtrFromString(fitWarningBody(bodyText))
	measured := rect{Right: bodyWidth}
	if dc, _, _ := pGetDC.Call(uintptr(active)); dc != 0 {
		old, _, _ := pSelectObject.Call(dc, uintptr(uiFont))
		pDrawText.Call(dc, uintptr(unsafe.Pointer(text)), ^uintptr(0), uintptr(unsafe.Pointer(&measured)), 0x10|0x400|0x800) // word break, calculate, no prefix
		pSelectObject.Call(dc, old)
		pReleaseDC.Call(uintptr(active), dc)
	}
	clientHeight, controls := warningLayout(clientWidth, availableHeight, scale, max(sc(warningBodyHeight), measured.Bottom))
	width, height := clientWidth+frameWidth-1, clientHeight+frameHeight-1
	x, y := warningOrigin(info.Work, width, height, margin)
	pSetWindowPos.Call(uintptr(active), ^uintptr(0), uintptr(x), uintptr(y), uintptr(width), uintptr(height), swpNoActivate|swpShowWindow)
	for index, control := range []windows.Handle{bodyControl, cancelControl, executeControl} {
		b := controls[index]
		pSetWindowPos.Call(uintptr(control), 0, uintptr(b.Left), uintptr(b.Top), uintptr(b.Right-b.Left), uintptr(b.Bottom-b.Top), swpNoZOrder|swpNoActivate)
	}
	setBodyText()
}

// Keep actions at the bottom of the bounded client, reserving body space
// before allowing wrapping to grow the window. Narrow clients stack actions.
func warningLayout(width, maxHeight int32, scale float64, bodyHeight int32) (int32, [3]rect) {
	sc := func(v int) int32 { return int32(float64(v)*scale + 0.5) }
	pad := min(sc(warningPadding), min(width/8, maxHeight/10))
	gap := min(sc(warningButtonGap), pad)
	buttonWidth := min(sc(warningButtonWidth), max(1, width-2*pad))
	buttonHeight := min(sc(warningButtonHeight), max(1, (maxHeight-4*pad-gap)/3))
	stack := 2*buttonWidth+gap > width-2*pad
	actionsHeight := buttonHeight
	if stack {
		actionsHeight = 2*buttonHeight + gap
	}
	height := min(maxHeight, bodyHeight+3*pad+actionsHeight)
	y := height - pad - actionsHeight
	executeX := width - pad - buttonWidth
	cancelX, executeY := executeX-buttonWidth-gap, y
	if stack {
		cancelX, executeY = executeX, y+buttonHeight+gap
	}
	return height, [3]rect{
		{Left: pad, Top: pad, Right: width - pad, Bottom: max(pad+1, y-pad)},
		{Left: cancelX, Top: y, Right: cancelX + buttonWidth, Bottom: y + buttonHeight},
		{Left: executeX, Top: executeY, Right: executeX + buttonWidth, Bottom: executeY + buttonHeight},
	}
}

func warningOrigin(work rect, width, height, margin int32) (int32, int32) {
	return max(work.Left, work.Right-width-margin), max(work.Top, work.Bottom-height-margin)
}

func windowScale() float64 {
	if dpiScale > 0 {
		return dpiScale * max(1, textScale)
	}
	return scaleFromWindow(active) * max(1, textScale)
}

func scaleFromWindow(hwnd windows.Handle) float64 {
	if hwnd == 0 {
		return 1
	}
	dpi, _, _ := pGetDpiForWindow.Call(uintptr(hwnd))
	if dpi == 0 {
		return 1
	}
	return float64(dpi) / 96
}

func applyTheme() {
	themeDark = theme.Current() == theme.ModeDark
	uiPalette = colors.ForTheme(themeDark)
	if background != 0 {
		pDeleteObject.Call(uintptr(background))
	}
	brush, _, _ := pCreateBrush.Call(uintptr(uiPalette.WindowBackground))
	background = windows.Handle(brush)
	nativeform.ApplyFrame(active, themeDark)
	scale := windowScale()
	windowIcons.Apply(active, themeDark, int(32*scale+0.5), int(16*scale+0.5), false)
	for _, control := range []windows.Handle{bodyControl, cancelControl, executeControl} {
		nativeform.ApplyControl(control, themeDark)
		if control != 0 {
			pInvalidateRect.Call(uintptr(control), 0, 1)
		}
	}
	nativeform.ApplyTooltip(tooltip, themeDark, uiPalette, uiFont)
	if active != 0 {
		pInvalidateRect.Call(uintptr(active), 0, 1)
	}
}

func rebuildForDPI(scale float64, suggested *rect) bool {
	chinese := font.SystemLanguageIsChinese()
	languageMu.RLock()
	if uiChinese != nil {
		chinese = *uiChinese
	}
	languageMu.RUnlock()
	newFont, _ := font.NewForLayout(int32(14*scale+0.5), 400, chinese)
	if newFont == 0 {
		return false
	}
	oldFont := uiFont
	uiFont = newFont
	for _, control := range []windows.Handle{bodyControl, cancelControl, executeControl} {
		if control != 0 {
			pSendMessage.Call(uintptr(control), wmSetFont, uintptr(uiFont), 0)
		}
	}
	if tooltip != 0 {
		pSendMessage.Call(uintptr(tooltip), wmSetFont, uintptr(uiFont), 0)
		pSendMessage.Call(uintptr(tooltip), ttmSetMaxTipWidth, 0, uintptr(int(320*scale)))
	}
	if oldFont != 0 {
		pDeleteObject.Call(uintptr(oldFont))
	}
	position(suggested)
	return true
}

func wndProc(hwnd windows.Handle, message uint32, wParam, lParam uintptr) uintptr {
	switch message {
	case wmClose:
		complete(currentSeq, false)
		return 0
	case wmCommand:
		switch uint16(wParam) {
		case idCancel:
			complete(currentSeq, false)
		case idExecute:
			complete(currentSeq, true)
		}
		return 0
	case wmPaint:
		nativeform.PaintWindowBackground(hwnd, background)
		return 0
	case wmEraseBkgnd:
		var bounds rect
		pGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&bounds)))
		pFillRect.Call(wParam, uintptr(unsafe.Pointer(&bounds)), uintptr(background))
		return 1
	case wmCtlColorStatic, wmCtlColorButton:
		pSetTextColor.Call(wParam, uintptr(uiPalette.PrimaryText))
		pSetBkMode.Call(wParam, transparent)
		return uintptr(background)
	case wmSettingChange, wmSysColorChange, wmThemeChanged:
		if message == wmSettingChange && uiFont != 0 {
			next := font.TextScaleFactor()
			if next != textScale {
				previous := textScale
				textScale = next
				if !rebuildForDPI(windowScale(), nil) {
					textScale = previous
				}
			}
		}
		applyTheme()
		return 0
	case wmDpiChanged:
		transition := nativeform.BeginFrameTransition(active)
		dpi := uint32(wParam & 0xffff)
		if dpi == 0 {
			dpi = 96
		}
		oldScale := dpiScale
		dpiScale = float64(dpi) / 96
		var suggested *rect
		if lParam != 0 {
			value := *(*rect)(nativeform.MessagePointer(lParam))
			suggested = &value
		}
		if rebuildForDPI(windowScale(), suggested) {
			windowIcons.Apply(active, themeDark, int(32*dpiScale+0.5), int(16*dpiScale+0.5), true)
		} else {
			dpiScale = oldScale
			position(suggested)
		}
		committed := false
		for range 3 {
			if err := transition.Commit(bodyControl, cancelControl, executeControl); err == nil {
				committed = true
				break
			}
		}
		if !committed {
			hideNow()
		}
		return 0
	case wmDestroy:
		countdown.Stop()
		windowIcons.Release()
		if active == hwnd {
			active = 0
		}
		return 0
	}
	result, _, _ := pDefWindowProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
	return result
}
