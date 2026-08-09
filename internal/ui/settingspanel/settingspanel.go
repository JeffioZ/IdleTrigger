// Package settingspanel provides a native, modal editor for configuration
// values that are too detailed for the compact tray control panel.
package settingspanel

import (
	"fmt"
	"runtime"
	"sync"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	"github.com/JeffioZ/idletrigger/internal/ui/colors"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

type State struct {
	KeepScreenOn, NoSleepOnBattery bool
	NoSleepBatteryThreshold        int
	IdleEnabled                    bool
	IdleTimeoutMinutes             int
	IdleAction                     string
	IdleWarningSeconds             int
	IdleEnhancedMonitor            bool
	ThemeMode                      string
	ThemeLightTime, ThemeDarkTime  string
	ThemeIPLocationEnabled         bool
	ThemeLocationStatus            string
	ThemeDarkOnBattery             bool
	ThemeSkipFullscreen            bool
	Language                       string
	HotkeysEnabled                 bool
	AutostartEnabled               bool
	LoggingEnabled                 bool
	Version                        string
	Revision                       string
	Chinese                        bool
	Owner                          windows.Handle
}

type SaveRequest struct {
	BaseRevision                   string
	KeepScreenOn, NoSleepOnBattery bool
	NoSleepBatteryThreshold        int
	IdleEnabled                    bool
	IdleTimeoutMinutes             int
	IdleAction                     string
	IdleWarningSeconds             int
	IdleEnhancedMonitor            bool
	ThemeMode                      string
	ThemeLightTime, ThemeDarkTime  string
	ThemeIPLocationEnabled         bool
	ThemeDarkOnBattery             bool
	ThemeSkipFullscreen            bool
	Language                       string
	HotkeysEnabled                 bool
	AutostartEnabled               bool
	LoggingEnabled                 bool
}

type SaveResult struct {
	State State
	Error string
}

type TextFunc func(string) string
type OnSave func(SaveRequest) SaveResult
type OnSaved func()
type OnCommand func()

type bounds struct{ x, y, width, height int }
type choice struct {
	labels   []string
	selected int
}

type panel struct {
	hwnd                                     windows.Handle
	state                                    State
	text                                     TextFunc
	onSave                                   OnSave
	onSaved                                  OnSaved
	onRepairTheme                            OnCommand
	onProjectHome                            OnCommand
	controls                                 map[uint16]windows.Handle
	labels                                   map[uint16]string
	bounds                                   map[uint16]bounds
	checks                                   map[uint16]bool
	choices                                  map[uint16]*choice
	surfaces                                 nativeform.ControlSurfaceSet
	interaction                              nativeform.InteractionTracker
	choiceOpen                               uint16
	choicePopup                              *nativeform.ChoicePopup
	font, sectionFont, titleFont             windows.Handle
	windowBrush, surfaceBrush, disabledBrush windows.Handle
	palette                                  colors.Palette
	themeDark                                bool
	icons                                    nativeform.WindowIcons
	style, exStyle                           uintptr
	dpiScale                                 float64
	ownerDisabled                            bool
	captureHost                              bool
	captureScale                             float64
	themeOverride                            *bool
	contentScroll                            *nativeform.Scrollbar
	tooltip                                  windows.Handle
	tooltipText                              map[windows.Handle][]uint16
	contentOffset, viewportHeight            int
	validationError                          bool
	page                                     int
}

const (
	windowClass                    = "IdleTriggerSettingsPanel"
	windowWidth                    = 700
	contentHeight                  = 524
	idTitle                 uint16 = 100
	idDescription           uint16 = 101
	idTabPower              uint16 = 102
	idTabTheme              uint16 = 103
	idTabApp                uint16 = 104
	idPowerTitle            uint16 = 110
	idKeepScreen            uint16 = 111
	idBatteryAllowed        uint16 = 112
	idBatteryThresholdLabel uint16 = 113
	idBatteryThreshold      uint16 = 114
	idWarningLabel          uint16 = 115
	idWarningSeconds        uint16 = 116
	idPowerHint             uint16 = 117
	idIdleTimeoutLabel      uint16 = 118
	idIdleTimeout           uint16 = 119
	idIdleActionLabel       uint16 = 120
	idIdleAction            uint16 = 121
	idIdleEnhanced          uint16 = 122
	idIdleTitle             uint16 = 123
	idThemeModeLabel        uint16 = 131
	idThemeMode             uint16 = 132
	idLightTimeLabel        uint16 = 133
	idLightTime             uint16 = 134
	idDarkTimeLabel         uint16 = 135
	idDarkTime              uint16 = 136
	idLocationLabel         uint16 = 137
	idLocationSource        uint16 = 138
	idThemeHint             uint16 = 143
	idThemeBattery          uint16 = 144
	idThemeFullscreen       uint16 = 145
	idThemeRepair           uint16 = 146
	idThemeScheduleTitle    uint16 = 148
	idThemeBehaviorTitle    uint16 = 149
	idLanguageLabel         uint16 = 151
	idLanguage              uint16 = 152
	idHotkeys               uint16 = 153
	idAutostart             uint16 = 154
	idLogging               uint16 = 155
	idProjectHome           uint16 = 157
	idAppGeneralTitle       uint16 = 159
	idValidation            uint16 = 160
	idAppAboutTitle         uint16 = 161
	idCancel                uint16 = 162
	idSave                  uint16 = 163
	idThemeLocationStatus   uint16 = 164
	idProjectHomeLabel      uint16 = 165
	idVersion               uint16 = 166
	idFieldSurfaceBase      uint16 = 500

	wmDestroy         = 0x0002
	wmClose           = 0x0010
	wmPaint           = 0x000f
	wmEraseBkgnd      = 0x0014
	wmDrawItem        = 0x002b
	wmCommand         = 0x0111
	wmCtlColorEdit    = 0x0133
	wmCtlColorStatic  = 0x0138
	wmCtlColorButton  = 0x0135
	wmSettingChange   = 0x001a
	wmSysColorChange  = 0x0015
	wmThemeChanged    = 0x031a
	wmDpiChanged      = 0x02e0
	wmMouseWheel      = 0x020a
	wmSetCursor       = 0x0020
	wmLButtonDown     = 0x0201
	wmSetFont         = 0x0030
	wmGetText         = 0x000d
	wmGetTextLength   = 0x000e
	wmOpenChoice      = 0x8001
	wmSize            = 0x0005
	emSetMargins      = 0x00d3
	emSetSel          = 0x00b1
	bnClicked         = 0
	enChange          = 0x0300
	wsPopup           = 0x80000000
	wsCaption         = 0x00c00000
	wsSysMenu         = 0x00080000
	wsClipChildren    = 0x02000000
	wsClipSiblings    = 0x04000000
	wsChild           = 0x40000000
	wsVisible         = 0x10000000
	wsTabStop         = 0x00010000
	wsExTopmost       = 0x00000008
	wsExAppWindow     = 0x00040000
	wsOverlapped      = 0x00000000
	wsThickFrame      = 0x00040000
	wsMinimizeBox     = 0x00020000
	wsMaximizeBox     = 0x00010000
	esAutoHScroll     = 0x0080
	esNumber          = 0x2000
	bsOwnerDraw       = 0x0000000b
	ssLeft            = 0
	ssOwnerDraw       = 0x0000000d
	formSurfaceStyle  = wsChild | wsClipSiblings | ssOwnerDraw
	swpNoZOrder       = 0x0004
	swpNoActivate     = 0x0010
	swpNoSize         = 0x0001
	swpNoMove         = 0x0002
	transparent       = 1
	opaque            = 2
	odsSelected       = 0x0001
	odsDisabled       = 0x0004
	ttsAlwaysTip      = 0x0001
	ttsNoPrefix       = 0x0002
	ttfIDIsHwnd       = 0x0001
	ttfSubclass       = 0x0010
	ttmAddTool        = 0x0432
	ttmSetMaxTipWidth = 0x0418
)

type wndClassEx struct {
	Size, Style              uint32
	WndProc                  uintptr
	ClsExtra, WndExtra       int32
	Instance                 windows.Handle
	Icon, Cursor, Background windows.Handle
	MenuName, ClassName      *uint16
	IconSm                   windows.Handle
}
type rect struct{ Left, Top, Right, Bottom int32 }
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
type drawItem struct {
	CtlType, CtlID, ItemID, ItemAction, ItemState uint32
	HwndItem, HDC                                 windows.Handle
	Rect                                          rect
	ItemData                                      uintptr
}

var (
	user32           = windows.NewLazySystemDLL("user32.dll")
	gdi32            = windows.NewLazySystemDLL("gdi32.dll")
	pCreateWindowEx  = user32.NewProc("CreateWindowExW")
	pDestroyWindow   = user32.NewProc("DestroyWindow")
	pDefWindowProc   = user32.NewProc("DefWindowProcW")
	pRegisterClassEx = user32.NewProc("RegisterClassExW")
	pSendMessage     = user32.NewProc("SendMessageW")
	pSetWindowText   = user32.NewProc("SetWindowTextW")
	pSetWindowPos    = user32.NewProc("SetWindowPos")
	pShowWindow      = user32.NewProc("ShowWindow")
	pEnableWindow    = user32.NewProc("EnableWindow")
	pIsWindow        = user32.NewProc("IsWindow")
	pIsWindowEnabled = user32.NewProc("IsWindowEnabled")
	pSetForeground   = user32.NewProc("SetForegroundWindow")
	pSetFocus        = user32.NewProc("SetFocus")
	pLoadCursor      = user32.NewProc("LoadCursorW")
	pSetCursor       = user32.NewProc("SetCursor")
	pGetClientRect   = user32.NewProc("GetClientRect")
	pGetDpiForWindow = user32.NewProc("GetDpiForWindow")
	pFillRect        = user32.NewProc("FillRect")
	pInvalidateRect  = user32.NewProc("InvalidateRect")
	pSetTextColor    = gdi32.NewProc("SetTextColor")
	pSetBkColor      = gdi32.NewProc("SetBkColor")
	pSetBkMode       = gdi32.NewProc("SetBkMode")
	pCreateBrush     = gdi32.NewProc("CreateSolidBrush")
	pDeleteObject    = gdi32.NewProc("DeleteObject")
	classOnce        sync.Once
	classErr         error
	activeMu         sync.Mutex
	active           *panel
	wndCallback      = windows.NewCallback(wndProc)
)

func Show(state State, onSave OnSave, onSaved OnSaved, onRepairTheme, onProjectHome OnCommand, text TextFunc) error {
	activeMu.Lock()
	if active != nil && active.hwnd != 0 {
		hwnd := active.hwnd
		activeMu.Unlock()
		pSetForeground.Call(uintptr(hwnd))
		return nil
	}
	p := &panel{state: state, onSave: onSave, onSaved: onSaved, onRepairTheme: onRepairTheme, onProjectHome: onProjectHome, text: text,
		controls: make(map[uint16]windows.Handle), labels: make(map[uint16]string), bounds: make(map[uint16]bounds),
		checks: make(map[uint16]bool), choices: make(map[uint16]*choice)}
	active = p
	activeMu.Unlock()
	if err := ensureClass(); err != nil {
		clearActive(p)
		return err
	}
	if err := p.create(); err != nil {
		clearActive(p)
		return err
	}
	return nil
}

func Focus() bool {
	activeMu.Lock()
	p := active
	activeMu.Unlock()
	if p == nil || p.hwnd == 0 {
		return false
	}
	pSetForeground.Call(uintptr(p.hwnd))
	return true
}

func Hide() {
	activeMu.Lock()
	p := active
	activeMu.Unlock()
	if p != nil && p.hwnd != 0 {
		pDestroyWindow.Call(uintptr(p.hwnd))
	}
}

// UpdateLocationStatus refreshes the non-blocking location summary when an IP
// lookup completes while Settings is open.
func UpdateLocationStatus(value string) {
	activeMu.Lock()
	p := active
	activeMu.Unlock()
	if p == nil || p.hwnd == 0 || p.controls[idThemeLocationStatus] == 0 {
		return
	}
	p.setText(idThemeLocationStatus, value)
}

// Capture hosts the real settings window for deterministic devtools visual checks.
func Capture(state State, text TextFunc, scale float64, dark bool, capture func(windows.Handle) error) error {
	return CapturePage(state, text, scale, dark, 0, capture)
}

// CapturePage captures one settings category using its real controls and layout.
func CapturePage(state State, text TextFunc, scale float64, dark bool, page int, capture func(windows.Handle) error) error {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	restoreTextScale := font.OverrideTextScaleFactor(1)
	defer restoreTextScale()
	if text == nil {
		text = func(key string) string { return key }
	}
	if err := ensureClass(); err != nil {
		return err
	}
	state.Owner = 0
	p := &panel{state: state, text: text, captureHost: true, captureScale: scale, themeOverride: &dark,
		controls: make(map[uint16]windows.Handle), labels: make(map[uint16]string), bounds: make(map[uint16]bounds),
		checks: make(map[uint16]bool), choices: make(map[uint16]*choice)}
	activeMu.Lock()
	if active != nil {
		activeMu.Unlock()
		return fmt.Errorf("settings panel already active")
	}
	active = p
	activeMu.Unlock()
	if err := p.create(); err != nil {
		clearActive(p)
		return err
	}
	p.page = max(0, min(page, 2))
	p.applyDependentStates()
	defer func() {
		if p.hwnd != 0 {
			pDestroyWindow.Call(uintptr(p.hwnd))
		}
	}()
	if capture == nil {
		return nil
	}
	for id, want := range map[uint16]string{
		idBatteryThreshold: fmt.Sprint(state.NoSleepBatteryThreshold), idWarningSeconds: fmt.Sprint(state.IdleWarningSeconds),
		idLightTime: state.ThemeLightTime, idDarkTime: state.ThemeDarkTime,
	} {
		if got := p.controlText(id); got != want {
			return fmt.Errorf("settings capture control %d text = %q, want %q", id, got, want)
		}
	}
	nativeform.PresentFrame(p.hwnd, p.frameControls()...)
	return capture(p.hwnd)
}

func clearActive(p *panel) {
	activeMu.Lock()
	if active == p {
		active = nil
	}
	activeMu.Unlock()
}

func ensureClass() error {
	classOnce.Do(func() {
		name, _ := windows.UTF16PtrFromString(windowClass)
		cursor, _, _ := pLoadCursor.Call(0, 32512) // IDC_ARROW
		wc := wndClassEx{Size: uint32(unsafe.Sizeof(wndClassEx{})), WndProc: wndCallback, Cursor: windows.Handle(cursor), ClassName: name}
		result, _, err := pRegisterClassEx.Call(uintptr(unsafe.Pointer(&wc)))
		if result == 0 && err != windows.ERROR_CLASS_ALREADY_EXISTS {
			classErr = fmt.Errorf("register settings panel: %w", err)
		}
	})
	return classErr
}

func (p *panel) create() error {
	class, _ := windows.UTF16PtrFromString(windowClass)
	title, _ := windows.UTF16PtrFromString(p.t("settings_title"))
	p.style = wsPopup | wsCaption | wsSysMenu | wsClipChildren
	p.exStyle = wsExTopmost
	if p.captureHost {
		p.style = wsOverlapped | wsCaption | wsSysMenu | wsThickFrame | wsMinimizeBox | wsMaximizeBox | wsClipChildren
		p.exStyle = wsExAppWindow
	}
	x, y := nativeform.InitialWindowPoint(p.state.Owner)
	hwnd, _, err := pCreateWindowEx.Call(p.exStyle, uintptr(unsafe.Pointer(class)), uintptr(unsafe.Pointer(title)), p.style,
		uintptr(x), uintptr(y), 1, 1, uintptr(p.state.Owner), 0, 0, 0)
	if hwnd == 0 {
		return fmt.Errorf("create settings panel: %w", err)
	}
	p.hwnd = windows.Handle(hwnd)
	firstFrame := nativeform.BeginFirstFrame(p.hwnd)
	p.dpiScale = p.windowScale()
	p.font, _ = font.New(int32(14*p.scale()+0.5), 400, p.state.Chinese)
	p.sectionFont, _ = font.New(int32(14*p.scale()+0.5), 600, p.state.Chinese)
	p.titleFont, _ = font.New(int32(17*p.scale()+0.5), 600, p.state.Chinese)
	if p.font == 0 || p.sectionFont == 0 || p.titleFont == 0 {
		pDestroyWindow.Call(hwnd)
		return fmt.Errorf("create settings fonts")
	}
	p.applyTheme()
	if err := p.build(); err != nil {
		pDestroyWindow.Call(hwnd)
		return err
	}
	bar, err := nativeform.NewScrollbar(nativeform.ScrollbarOptions{Parent: p.hwnd, Palette: p.palette,
		Background: p.palette.WindowBackground, Scale: p.scale(), OnChange: p.scrollTo})
	if err != nil {
		pDestroyWindow.Call(hwnd)
		return err
	}
	p.contentScroll = bar
	p.position(nil)
	if !p.captureHost && p.state.Owner != 0 {
		if enabled, _, _ := pIsWindowEnabled.Call(uintptr(p.state.Owner)); enabled != 0 {
			pEnableWindow.Call(uintptr(p.state.Owner), 0)
			p.ownerDisabled = true
		}
	}
	p.surfaces.PrepareCues()
	if err := firstFrame.Reveal(nativeform.FirstFrameOptions{RepeatShow: p.captureHost, Controls: p.frameControls()}); err != nil {
		pDestroyWindow.Call(hwnd)
		return err
	}
	if !p.captureHost {
		pSetForeground.Call(hwnd)
		pSetFocus.Call(uintptr(p.controls[idKeepScreen]))
		trayicon.SetTabNavigationWindow(p.hwnd, func() { p.interaction.SetFocusVisible(true) })
	}
	return nil
}

func (p *panel) t(key string) string {
	if p.text == nil {
		return key
	}
	return p.text(key)
}

func (p *panel) scale() float64 {
	if p.captureScale > 0 {
		return p.captureScale
	}
	if p.dpiScale > 0 {
		return p.dpiScale
	}
	return 1
}

func (p *panel) windowScale() float64 {
	dpi, _, _ := pGetDpiForWindow.Call(uintptr(p.hwnd))
	if dpi == 0 {
		return 1
	}
	return float64(dpi) / 96
}

func (p *panel) frameControls() []windows.Handle {
	out := make([]windows.Handle, 0, len(p.controls)+1)
	for _, control := range p.controls {
		if control != 0 {
			out = append(out, control)
		}
	}
	if p.contentScroll != nil && p.contentScroll.Window() != 0 {
		out = append(out, p.contentScroll.Window())
	}
	return out
}

func currentThemeDark() bool { return theme.Current() == theme.ModeDark }
