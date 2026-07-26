// Package idlewarning displays an in-app, non-activating idle warning.
package idlewarning

import (
	"fmt"
	"sync"
	"sync/atomic"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/ui/colors"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

type rect struct{ Left, Top, Right, Bottom int32 }
type point struct{ X, Y int32 }

type wndClassEx struct {
	Size, Style              uint32
	WndProc                  uintptr
	ClsExtra, WndExtra       int32
	Instance                 windows.Handle
	Icon, Cursor, Background windows.Handle
	MenuName, ClassName      *uint16
	IconSm                   windows.Handle
}

type monitorInfo struct {
	Size          uint32
	Monitor, Work rect
	Flags         uint32
}

// paintStruct is an opaque buffer large enough for PAINTSTRUCT on 32-bit and
// 64-bit Windows. Only BeginPaint/EndPaint consume it, so this avoids Go
// struct-alignment differences between architectures.
type paintStruct struct{ data [72]byte }

const (
	warningClass = "IdleTriggerIdleWarning"

	wmDestroy        = 0x0002
	wmClose          = 0x0010
	wmPaint          = 0x000F
	wmEraseBkgnd     = 0x0014
	wmSysColorChange = 0x0015
	wmSettingChange  = 0x001A
	wmDpiChanged     = 0x02E0
	wmThemeChanged   = 0x031A
	wsPopup          = 0x80000000
	wsCaption        = 0x00C00000
	wsSysMenu        = 0x00080000
	wsExTool         = 0x00000080
	wsExTopmost      = 0x00000008
	wsExNoActivate   = 0x08000000
	swpNoActivate    = 0x0010
	swpShowWindow    = 0x0040
	monitorNearest   = 2
	dtLeft           = 0x00000000
	dtCenter         = 0x00000001
	dtVCenter        = 0x00000004
	dtWordBreak      = 0x00000010
	dtCalcRect       = 0x00000400
	transparent      = 1
	warningMinHeight = 92
)

var (
	user32 = windows.NewLazySystemDLL("user32.dll")
	gdi32  = windows.NewLazySystemDLL("gdi32.dll")
	dwmapi = windows.NewLazySystemDLL("dwmapi.dll")

	pCreateWindowEx        = user32.NewProc("CreateWindowExW")
	pDestroyWindow         = user32.NewProc("DestroyWindow")
	pDefWindowProc         = user32.NewProc("DefWindowProcW")
	pRegisterClassEx       = user32.NewProc("RegisterClassExW")
	pGetCursorPos          = user32.NewProc("GetCursorPos")
	pMonitorFromWindow     = user32.NewProc("MonitorFromWindow")
	pMonitorFromRect       = user32.NewProc("MonitorFromRect")
	pGetMonitorInfo        = user32.NewProc("GetMonitorInfoW")
	pSetWindowPos          = user32.NewProc("SetWindowPos")
	pUpdateWindow          = user32.NewProc("UpdateWindow")
	pBeginPaint            = user32.NewProc("BeginPaint")
	pEndPaint              = user32.NewProc("EndPaint")
	pGetClientRect         = user32.NewProc("GetClientRect")
	pGetDC                 = user32.NewProc("GetDC")
	pReleaseDC             = user32.NewProc("ReleaseDC")
	pInvalidateRect        = user32.NewProc("InvalidateRect")
	pFillRect              = user32.NewProc("FillRect")
	pDrawText              = user32.NewProc("DrawTextW")
	pSetTextColor          = gdi32.NewProc("SetTextColor")
	pSetBkMode             = gdi32.NewProc("SetBkMode")
	pGetDpiForWindow       = user32.NewProc("GetDpiForWindow")
	pDeleteObject          = gdi32.NewProc("DeleteObject")
	pCreateBrush           = gdi32.NewProc("CreateSolidBrush")
	pSelectObject          = gdi32.NewProc("SelectObject")
	pDwmSetWindowAttribute = dwmapi.NewProc("DwmSetWindowAttribute")

	classOnce  sync.Once
	classErr   error
	active     windows.Handle
	title      string
	body       string
	activeSeq  uint64
	titleFont  windows.Handle
	bodyFont   windows.Handle
	fontChoice font.Choice
	uiChinese  *bool
	nextSeq    atomic.Uint64
	countdown  nativeform.CountdownCancellation
	dismissMu  sync.RWMutex
	onDismiss  func()
	languageMu sync.RWMutex

	warningProc = windows.NewCallback(wndProc)
)

// Show schedules a warning on the tray UI thread. It never activates the
// window, so displaying it does not itself reset the user's idle time.
func Show(titleText, bodyText string) {
	seq := nextSeq.Add(1)
	if !trayicon.Post(func() {
		showNow(titleText, bodyText, seq)
	}) {
		return
	}
}

// ShowCountdown displays a warning and refreshes its body once per second.
// bodyForSecond receives the remaining second count, including the initial
// value. The warning remains non-activating; user activity is still observed by
// the idle monitor rather than by this window.
func ShowCountdown(titleText string, seconds int, bodyForSecond func(int) string) {
	if bodyForSecond == nil {
		return
	}
	if seconds < 0 {
		seconds = 0
	}
	seq := nextSeq.Add(1)
	initial := bodyForSecond(seconds)
	if !trayicon.Post(func() {
		showNow(titleText, initial, seq)
		if seconds > 0 && active != 0 && activeSeq == seq {
			startCountdown(seq, seconds, bodyForSecond)
		}
	}) {
		return
	}
}

func startCountdown(seq uint64, seconds int, bodyForSecond func(int) string) {
	stop := countdown.Replace()
	go func() {
		ticker := time.NewTicker(time.Second)
		defer ticker.Stop()
		for remaining := seconds - 1; remaining >= 0; remaining-- {
			select {
			case <-ticker.C:
			case <-stop:
				return
			}
			text := bodyForSecond(remaining)
			if !trayicon.Post(func() { updateBodyNow(seq, text) }) {
				return
			}
			if remaining == 0 {
				return
			}
		}
	}()
}

// SetOnDismiss sets the callback used when the user explicitly closes the warning.
func SetOnDismiss(fn func()) {
	dismissMu.Lock()
	onDismiss = fn
	dismissMu.Unlock()
}

// SetLanguage supplies the resolved application UI language before a warning
// is shown, including when it differs from the Windows display language.
func SetLanguage(chinese bool) {
	languageMu.Lock()
	uiChinese = &chinese
	languageMu.Unlock()
}

// Hide closes the currently displayed warning. It must run on the tray UI thread.
func Hide() {
	countdown.Stop()
	if active != 0 {
		pDestroyWindow.Call(uintptr(active))
	}
	activeSeq = 0
	for _, font := range []windows.Handle{titleFont, bodyFont} {
		if font != 0 {
			pDeleteObject.Call(uintptr(font))
		}
	}
	titleFont, bodyFont = 0, 0
}

func showNow(titleText, bodyText string, seq uint64) {
	Hide()
	if err := ensureClass(); err != nil {
		return
	}
	class, _ := windows.UTF16PtrFromString(warningClass)
	caption, _ := windows.UTF16PtrFromString("IdleTrigger")
	var cursor point
	pGetCursorPos.Call(uintptr(unsafe.Pointer(&cursor)))
	hwnd, _, _ := pCreateWindowEx.Call(
		wsExTool|wsExTopmost|wsExNoActivate,
		uintptr(unsafe.Pointer(class)), uintptr(unsafe.Pointer(caption)), warningWindowStyle(),
		uintptr(cursor.X), uintptr(cursor.Y), 1, 1, 0, 0, 0, 0,
	)
	if hwnd == 0 {
		return
	}
	active = windows.Handle(hwnd)
	activeSeq = seq
	title, body = titleText, bodyText
	rebuildFonts(active)
	applyFrameTheme(active, theme.Current() == theme.ModeDark)
	mylog.Info("UI font: surface=idle-warning ui_language=%s system_language=%s system_locale=%s face=%q reason=%s dpi=%d title_px=%d body_px=%d", fontChoice.UILanguage, fontChoice.SystemLanguage, fontChoice.SystemLocale, fontChoice.Face, fontChoice.Reason, dpiForWindow(active), scaledFontSize(active, 15), scaledFontSize(active, 13))
	position(active, nil)
}

func updateBodyNow(seq uint64, bodyText string) {
	if active == 0 || activeSeq != seq {
		return
	}
	if body == bodyText {
		return
	}
	body = bodyText
	position(active, nil)
	pInvalidateRect.Call(uintptr(active), 0, 0)
	pUpdateWindow.Call(uintptr(active))
}

func ensureClass() error {
	classOnce.Do(func() {
		name, err := windows.UTF16PtrFromString(warningClass)
		if err != nil {
			classErr = err
			return
		}
		wc := wndClassEx{Size: uint32(unsafe.Sizeof(wndClassEx{})), WndProc: warningProc, ClassName: name}
		result, _, callErr := pRegisterClassEx.Call(uintptr(unsafe.Pointer(&wc)))
		if result == 0 && callErr != windows.ERROR_CLASS_ALREADY_EXISTS {
			classErr = fmt.Errorf("register idle warning class: %w", callErr)
		}
	})
	return classErr
}

func position(hwnd windows.Handle, suggested *rect) {
	sc := func(v int32) int32 { return scaleForWindow(hwnd, v) }
	width, margin := sc(348), sc(16)
	bodyWidth := width - 2*margin - sc(10)
	bodyHeight := measureText(hwnd, body, sc(13), 400, bodyWidth)
	// The body starts below the header with its own top and bottom inset. Keep
	// those in the calculated client height so wrapped text is never clipped.
	height := sc(52) + sc(12) + bodyHeight + sc(14)
	if minimum := sc(warningMinHeight); height < minimum {
		height = minimum
	}
	if outerWidth, outerHeight, err := nativeform.WindowSizeForClient(
		int(width), int(height), warningWindowStyle(), wsExTool|wsExTopmost|wsExNoActivate, uint32(dpiForWindow(hwnd)),
	); err == nil {
		width, height = outerWidth, outerHeight
	}
	monitor := uintptr(0)
	if suggested != nil {
		monitor, _, _ = pMonitorFromRect.Call(uintptr(unsafe.Pointer(suggested)), monitorNearest)
	}
	if monitor == 0 {
		monitor, _, _ = pMonitorFromWindow.Call(uintptr(hwnd), monitorNearest)
	}
	info := monitorInfo{Size: uint32(unsafe.Sizeof(monitorInfo{}))}
	if monitor != 0 {
		pGetMonitorInfo.Call(monitor, uintptr(unsafe.Pointer(&info)))
	}
	x := info.Work.Right - width - margin
	y := info.Work.Bottom - height - margin
	pSetWindowPos.Call(uintptr(hwnd), ^uintptr(0), uintptr(x), uintptr(y), uintptr(width), uintptr(height), swpNoActivate|swpShowWindow)
	pUpdateWindow.Call(uintptr(hwnd))
}

func warningWindowStyle() uintptr { return wsPopup | wsCaption | wsSysMenu }

func applyFrameTheme(hwnd windows.Handle, dark bool) {
	value := uint32(0)
	if dark {
		value = 1
	}
	const (
		dwmwaUseImmersiveDarkMode       = 20
		dwmwaUseImmersiveDarkModeLegacy = 19
	)
	result, _, _ := pDwmSetWindowAttribute.Call(uintptr(hwnd), dwmwaUseImmersiveDarkMode, uintptr(unsafe.Pointer(&value)), unsafe.Sizeof(value))
	if result != 0 {
		pDwmSetWindowAttribute.Call(uintptr(hwnd), dwmwaUseImmersiveDarkModeLegacy, uintptr(unsafe.Pointer(&value)), unsafe.Sizeof(value))
	}
}

func measureText(hwnd windows.Handle, text string, size, weight, maxWidth int32) int32 {
	if text == "" || maxWidth <= 0 {
		return 0
	}
	dc, _, _ := pGetDC.Call(uintptr(hwnd))
	if dc == 0 {
		return 0
	}
	defer pReleaseDC.Call(uintptr(hwnd), dc)
	font := bodyFont
	if font == 0 {
		return 0
	}
	old, _, _ := pSelectObject.Call(dc, uintptr(font))
	defer pSelectObject.Call(dc, old)
	value, err := windows.UTF16PtrFromString(text)
	if err != nil {
		return 0
	}
	bounds := rect{Right: maxWidth}
	pDrawText.Call(dc, uintptr(unsafe.Pointer(value)), ^uintptr(0), uintptr(unsafe.Pointer(&bounds)), dtLeft|dtWordBreak|dtCalcRect)
	return bounds.Bottom - bounds.Top
}

func scaleForWindow(hwnd windows.Handle, v int32) int32 {
	dpi, _, _ := pGetDpiForWindow.Call(uintptr(hwnd))
	if dpi == 0 {
		return v
	}
	return int32(float64(v)*float64(dpi)/96 + 0.5)
}

func dpiForWindow(hwnd windows.Handle) uintptr {
	dpi, _, _ := pGetDpiForWindow.Call(uintptr(hwnd))
	if dpi == 0 {
		return 96
	}
	return dpi
}

func scaledFontSize(hwnd windows.Handle, size int32) int32 {
	return int32(float64(size)*float64(dpiForWindow(hwnd))/96 + 0.5)
}

func rebuildFonts(hwnd windows.Handle) {
	chinese := font.SystemLanguageIsChinese()
	languageMu.RLock()
	if uiChinese != nil {
		chinese = *uiChinese
	}
	languageMu.RUnlock()
	newTitle, choice := font.New(scaledFontSize(hwnd, 15), 600, chinese)
	newBody, _ := font.New(scaledFontSize(hwnd, 13), 400, chinese)
	if newTitle == 0 || newBody == 0 {
		if newTitle != 0 {
			pDeleteObject.Call(uintptr(newTitle))
		}
		if newBody != 0 {
			pDeleteObject.Call(uintptr(newBody))
		}
		return
	}
	oldTitle, oldBody := titleFont, bodyFont
	titleFont, bodyFont, fontChoice = newTitle, newBody, choice
	for _, font := range []windows.Handle{oldTitle, oldBody} {
		if font != 0 {
			pDeleteObject.Call(uintptr(font))
		}
	}
}

func wndProc(hwnd windows.Handle, message uint32, wParam, lParam uintptr) uintptr {
	switch message {
	case wmSettingChange, wmSysColorChange, wmThemeChanged:
		applyFrameTheme(hwnd, theme.Current() == theme.ModeDark)
		pInvalidateRect.Call(uintptr(hwnd), 0, 0)
		return 0
	case wmPaint:
		paint(hwnd)
		return 0
	case wmEraseBkgnd:
		return 1
	case wmClose:
		dismiss()
		return 0
	case wmDpiChanged:
		rebuildFonts(hwnd)
		var suggested *rect
		if lParam != 0 {
			value := *(*rect)(nativeform.MessagePointer(lParam))
			suggested = &value
		}
		position(hwnd, suggested)
		mylog.Info("UI font: surface=idle-warning rebuilt reason=dpi-change dpi=%d face=%q", dpiForWindow(hwnd), fontChoice.Face)
		return 0
	case wmDestroy:
		if active == hwnd {
			countdown.Stop()
			active = 0
			activeSeq = 0
		}
		return 0
	}
	result, _, _ := pDefWindowProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
	return result
}

func dismiss() {
	Hide()
	dismissMu.RLock()
	fn := onDismiss
	dismissMu.RUnlock()
	if fn != nil {
		fn()
	}
}

func paint(hwnd windows.Handle) {
	var ps paintStruct
	dc, _, _ := pBeginPaint.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&ps)))
	if dc == 0 {
		return
	}
	defer pEndPaint.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&ps)))
	var client rect
	pGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&client)))
	dark := theme.Current() == theme.ModeDark
	palette := colors.ForTheme(dark)
	brush := makeBrush(palette.WindowBackground)
	defer pDeleteObject.Call(uintptr(brush))
	pFillRect.Call(dc, uintptr(unsafe.Pointer(&client)), uintptr(brush))
	sc := func(v int32) int32 { return scaleForWindow(hwnd, v) }
	margin := sc(16)
	headerBottom := sc(52)
	header := rect{Right: client.Right, Bottom: headerBottom}
	headerBrush := makeBrush(palette.ElevatedSurface)
	pFillRect.Call(dc, uintptr(unsafe.Pointer(&header)), uintptr(headerBrush))
	pDeleteObject.Call(uintptr(headerBrush))
	separator := rect{Left: 0, Top: headerBottom - sc(1), Right: client.Right, Bottom: headerBottom}
	separatorBrush := makeBrush(palette.SubtleBorder)
	pFillRect.Call(dc, uintptr(unsafe.Pointer(&separator)), uintptr(separatorBrush))
	pDeleteObject.Call(uintptr(separatorBrush))
	titleTop, titleBottom := sc(15), sc(42)
	// Use the same compact title accent as the main panel.  A taller bar makes
	// the smaller warning title look visually top-heavy at higher DPI scales.
	accentHeight := sc(16)
	// DrawText places this font's visible glyphs slightly above the centre of
	// its layout rectangle. Compensate for that GDI ascent so the accent aligns
	// with the title the user actually sees, rather than only with its bounds.
	accentTop := titleTop + (titleBottom-titleTop-accentHeight)/2 - sc(3)
	accentRect := rect{Left: margin, Top: accentTop, Right: margin + sc(3), Bottom: accentTop + accentHeight}
	accentBrush := makeBrush(palette.Accent)
	pFillRect.Call(dc, uintptr(unsafe.Pointer(&accentRect)), uintptr(accentBrush))
	pDeleteObject.Call(uintptr(accentBrush))
	pSetBkMode.Call(dc, transparent)
	pSetTextColor.Call(dc, uintptr(palette.PrimaryText))
	drawText(dc, title, rect{Left: margin + sc(10), Top: titleTop, Right: client.Right - margin, Bottom: titleBottom}, titleFont, dtLeft)
	pSetTextColor.Call(dc, uintptr(palette.SecondaryText))
	drawText(dc, body, rect{Left: margin + sc(10), Top: headerBottom + sc(12), Right: client.Right - margin, Bottom: client.Bottom - sc(14)}, bodyFont, dtLeft|dtWordBreak)
}

func drawText(dc uintptr, text string, bounds rect, font windows.Handle, format uintptr) {
	ptr, err := windows.UTF16PtrFromString(text)
	if err != nil {
		return
	}
	if font != 0 {
		old, _, _ := pSelectObject.Call(dc, uintptr(font))
		defer pSelectObject.Call(dc, old)
	}
	pDrawText.Call(dc, uintptr(unsafe.Pointer(ptr)), ^uintptr(0), uintptr(unsafe.Pointer(&bounds)), format)
}

func makeBrush(color uint32) windows.Handle {
	result, _, _ := pCreateBrush.Call(uintptr(color))
	return windows.Handle(result)
}
