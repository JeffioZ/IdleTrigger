// Package controlpanel implements IdleTrigger's native Win32 control panel.
package controlpanel

import (
	"github.com/JeffioZ/idletrigger/internal/ui/colors"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"golang.org/x/sys/windows"
	"sync"
)

type wndClassExW struct {
	Size, Style              uint32
	WndProc                  uintptr
	ClsExtra, WndExtra       int32
	Instance                 windows.Handle
	Icon, Cursor, Background windows.Handle
	MenuName, ClassName      *uint16
	IconSm                   windows.Handle
}

type rect struct{ Left, Top, Right, Bottom int32 }

type buttonRole uint8

const (
	buttonCommand buttonRole = iota
	buttonToggle
	buttonChoice
)

type buttonVisualState struct {
	Role                      buttonRole
	Active, Disabled          bool
	Hovered, Pressed, Focused bool
}

type monitorInfo struct {
	Size          uint32
	Monitor, Work rect
	Flags         uint32
}

type drawItem struct {
	CtlType, CtlID, ItemID, ItemAction, ItemState uint32
	HwndItem, HDC                                 windows.Handle
	Rect                                          rect
	ItemData                                      uintptr
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

type Action int

const (
	ActSleep Action = iota
	ActHibernate
	ActShutdown
	ActLock
	ActRestart
	ActNoSleepToggle
	ActAutomationOpen
	ActAutomationToggle
	ActIdleToggle
	ActIdleTimeout Action = 100 + iota
	ActIdleAction
	ActIdleWarningToggle
	ActIdleEnhancedMonitorToggle
	ActThemeToggle
	ActBatteryToggle
	ActFullscreenToggle
	ActIPLocationToggle
	ActSwitchTheme
	ActRepairTheme
	ActHotkeyToggle
	ActAutostartToggle
	ActLoggingToggle
	ActLanguage
	ActConfig
	ActProjectHome
	ActExit
)

type State struct {
	NoSleepEnabled, IdleEnabled                       bool
	NoSleepStatus, IdleStatus                         string
	AutomationEnabled                                 bool
	AutomationCount                                   int
	AutomationSummary                                 string
	IdleWarningEnabled                                bool
	IdleEnhancedMonitor                               bool
	IdleTimeout                                       int
	IdleWarningSeconds                                int
	IdleAction                                        string
	ThemeSwitchEnabled, DarkOnBattery, SkipFullscreen bool
	ThemeUnavailable                                  bool
	ThemeUnavailableDetail                            string
	IPLocationEnabled                                 bool
	HotkeysEnabled, AutostartEnabled, LoggingEnabled  bool
	IsChinese                                         bool
	ThemeSchedule                                     string
	IPLocationLabel                                   string
	AppVersion                                        string
	Theme                                             Theme
	Owner                                             windows.Handle
	DeveloperCapturePanel, DeveloperWarningPreview    bool
}

type LangFunc func(key string) string
type OnAction func(action Action, value int)

const (
	panelClass = "IdleTriggerPopup"

	wmDestroy          = 0x0002
	wmClose            = 0x0010
	wmActivate         = 0x0006
	wmMouseMove        = 0x0200
	wmLButtonDown      = 0x0201
	wmLButtonUp        = 0x0202
	wmMouseWheel       = 0x020A
	wmMouseLeave       = 0x02A3
	wmSetCursor        = 0x0020
	wmNcLButtonDown    = 0x00A1
	wmParentNotify     = 0x0210
	wmEraseBkgnd       = 0x0014
	wmSysColorChange   = 0x0015
	wmSettingChange    = 0x001A
	wmDrawItem         = 0x002B
	wmCommand          = 0x0111
	wmCtlColorStatic   = 0x0138
	wmCtlColorEdit     = 0x0133
	wmCtlColorListBox  = 0x0134
	wmThemeChanged     = 0x031A
	wmDpiChanged       = 0x02E0
	wmGetDpiScaledSize = 0x02E4
	wmKeyDown          = 0x0100
	wmSysKeyDown       = 0x0104
	wmKillFocus        = 0x0008
	wmOpenChoice       = 0x8001
	wmApplyDPI         = 0x8002
	wmCommitDPI        = 0x8003
	wmSetFont          = 0x0030
	wmSetIcon          = 0x0080
	bnClicked          = 0
	waInactive         = 0

	wsOverlapped       = 0x00000000
	wsPopup            = 0x80000000
	wsCaption          = 0x00C00000
	wsSysMenu          = 0x00080000
	wsThickFrame       = 0x00040000
	wsMinimizeBox      = 0x00020000
	wsMaximizeBox      = 0x00010000
	wsClipChildren     = 0x02000000
	wsClipSiblings     = 0x04000000
	wsChild            = 0x40000000
	wsVisible          = 0x10000000
	wsTabStop          = 0x00010000
	wsVScroll          = 0x00200000
	wsOverlappedWindow = wsOverlapped | wsCaption | wsSysMenu | wsThickFrame | wsMinimizeBox | wsMaximizeBox
	bsOwnerDraw        = 0x0000000B
	ssOwnerDraw        = 0x0000000D
	ssLeft             = 0x00000000
	ssRight            = 0x00000002

	wsExToolWindow = 0x00000080
	wsExTopmost    = 0x00000008
	wsExAppWindow  = 0x00040000

	swpNoSize                     = 0x0001
	swpNoMove                     = 0x0002
	swpNoZOrder                   = 0x0004
	swpNoActivate                 = 0x0010
	monitorNearest                = 2
	panelFallbackWindowCoordinate = int32(-32000)
	vkUp                          = 0x26
	vkDown                        = 0x28
	vkHome                        = 0x24
	vkEnd                         = 0x23
	vkReturn                      = 0x0D
	vkSpace                       = 0x20
	vkF4                          = 0x73
	vkEscape                      = 0x1B

	odsSelected  = 0x0001
	odsDisabled  = 0x0004
	odsFocus     = 0x0010
	odsHotlight  = 0x0040
	psSolid      = 0
	dtCenter     = 0x00000001
	dtVCenter    = 0x00000004
	dtLeft       = 0x00000000
	dtWordBreak  = 0x00000010
	dtCalcRect   = 0x00000400
	dtSingleLine = 0x00000020
	transparent  = 1
	tmeLeave     = 0x00000002

	ttsAlwaysTip       = 0x0001
	ttsNoPrefix        = 0x0002
	ttfIDIsHwnd        = 0x0001
	ttfSubclass        = 0x0010
	ttmAddTool         = 0x0432
	ttmUpdateTipText   = 0x0439
	ttmSetTipBkColor   = 0x0413
	ttmSetTipTextColor = 0x0414
	ttmSetMaxTipWidth  = 0x0418
	cbAddString        = 0x0143
	cbSetCurSel        = 0x014E
	cbSetItemHeight    = 0x0153
)

const (
	idSleep = 1 + iota
	idHibernate
	idShutdown
	idLock
	idRestart
)

const (
	idQuickActions      = 6
	idNoSleep           = 10
	idAutomation        = 11
	idAutomationEnabled = 12
	idIdle              = 20
	idIdleWarning       = 21
	idIdleEnhanced      = 22
	idTheme             = 30
	idBattery           = 32
	idFullscreen        = 33
	idThemeSwitch       = 34
	idThemeRepair       = 35
	idIPLocation        = 36
	idHotkeys           = 40
	idAutostart         = 41
	idLogging           = 42
	idIdleTimeout       = 120
	idIdleAction        = 121
	idLanguage          = 150
	idLangEN            = 152
	idLangZH            = 153
	idConfig            = 500
	idProjectHome       = 501
	idExit              = 502
	idTestWarning       = 600
)

var (
	user32   = windows.NewLazySystemDLL("user32.dll")
	gdi32    = windows.NewLazySystemDLL("gdi32.dll")
	comctl   = windows.NewLazySystemDLL("comctl32.dll")
	dwmapi   = windows.NewLazySystemDLL("dwmapi.dll")
	uxtheme  = windows.NewLazySystemDLL("uxtheme.dll")
	kernel32 = windows.NewLazySystemDLL("kernel32.dll")

	pCreateWindowEx        = user32.NewProc("CreateWindowExW")
	pLoadIcon              = user32.NewProc("LoadIconW")
	pLoadImage             = user32.NewProc("LoadImageW")
	pDestroyIcon           = user32.NewProc("DestroyIcon")
	pRegisterClassEx       = user32.NewProc("RegisterClassExW")
	pDestroyWindow         = user32.NewProc("DestroyWindow")
	pDefWindowProc         = user32.NewProc("DefWindowProcW")
	pCallWindowProc        = user32.NewProc("CallWindowProcW")
	pSendMessage           = user32.NewProc("SendMessageW")
	pSetWindowPos          = user32.NewProc("SetWindowPos")
	pBeginDeferWindowPos   = user32.NewProc("BeginDeferWindowPos")
	pDeferWindowPos        = user32.NewProc("DeferWindowPos")
	pEndDeferWindowPos     = user32.NewProc("EndDeferWindowPos")
	pPostMessage           = user32.NewProc("PostMessageW")
	pGetCursorPos          = user32.NewProc("GetCursorPos")
	pMonitorFromWindow     = user32.NewProc("MonitorFromWindow")
	pMonitorFromRect       = user32.NewProc("MonitorFromRect")
	pGetMonitorInfo        = user32.NewProc("GetMonitorInfoW")
	pGetDpiForWindow       = user32.NewProc("GetDpiForWindow")
	pGetDpiForSystem       = user32.NewProc("GetDpiForSystem")
	pSetForeground         = user32.NewProc("SetForegroundWindow")
	pLoadCursor            = user32.NewProc("LoadCursorW")
	pSetCursor             = user32.NewProc("SetCursor")
	pSetFocus              = user32.NewProc("SetFocus")
	pEnableWindow          = user32.NewProc("EnableWindow")
	pFillRect              = user32.NewProc("FillRect")
	pDrawText              = user32.NewProc("DrawTextW")
	pInvalidateRect        = user32.NewProc("InvalidateRect")
	pGetClientRect         = user32.NewProc("GetClientRect")
	pGetDC                 = user32.NewProc("GetDC")
	pReleaseDC             = user32.NewProc("ReleaseDC")
	pTrackMouseEvent       = user32.NewProc("TrackMouseEvent")
	pDeleteObject          = gdi32.NewProc("DeleteObject")
	pCreateBrush           = gdi32.NewProc("CreateSolidBrush")
	pCreatePen             = gdi32.NewProc("CreatePen")
	pGetStockObject        = gdi32.NewProc("GetStockObject")
	pRoundRect             = gdi32.NewProc("RoundRect")
	pSetTextColor          = gdi32.NewProc("SetTextColor")
	pSetBkColor            = gdi32.NewProc("SetBkColor")
	pSetBkMode             = gdi32.NewProc("SetBkMode")
	pMoveToEx              = gdi32.NewProc("MoveToEx")
	pLineTo                = gdi32.NewProc("LineTo")
	pSelectObject          = gdi32.NewProc("SelectObject")
	pInitCommonControlsEx  = comctl.NewProc("InitCommonControlsEx")
	pDwmSetWindowAttribute = dwmapi.NewProc("DwmSetWindowAttribute")
	pSetWindowTheme        = uxtheme.NewProc("SetWindowTheme")
	pGetModuleHandle       = kernel32.NewProc("GetModuleHandleW")

	classOnce sync.Once
	classErr  error
	panelMu   sync.Mutex
	active    *panel

	buttonProc = windows.NewCallback(buttonWndProc)
)

type staticKind uint8

const (
	staticNone staticKind = iota
	staticSection
	staticSubtitle
)

type trackMouseEvent struct {
	Size      uint32
	Flags     uint32
	HwndTrack windows.Handle
	HoverTime uint32
}

type initCommonControlsEx struct {
	Size uint32
	ICC  uint32
}

type windowSize struct {
	Width, Height int32
}

type panel struct {
	hwnd windows.Handle
	// panelResources owns every GDI and icon handle created for this panel.
	// It is embedded only to keep the drawing code compact; creation, refresh,
	// and destruction are centralized in resources.go.
	panelResources
	onAction                OnAction
	lang                    LangFunc
	theme                   Theme
	metrics                 panelMetrics
	clientH                 int
	appVersion              string
	idleTimeout             int
	idleWarningSeconds      int
	idleAction              string
	isChinese               bool
	owner                   windows.Handle
	iconThemeDark           bool
	themeDark               bool
	iconsInitialized        bool
	tooltip                 windows.Handle
	palette                 colors.Palette
	fontChoice              font.Choice
	controls                map[uint16]windows.Handle
	labels                  map[uint16]string
	staticKinds             map[uint16]staticKind
	tooltips                map[uint16][]uint16
	toggles, selected       map[uint16]bool
	disabled                map[uint16]bool
	oldButtonProc           map[windows.Handle]uintptr
	hoverID                 uint16
	keyboardNavigation      bool
	nextStaticID            uint16
	themeScheduleID         uint16
	automationSummaryID     uint16
	noSleepStatus           string
	idleStatus              string
	automationCount         int
	automationSummary       string
	developerCapturePanel   bool
	developerWarningPreview bool
	themeSchedule           string
	themeUnavailable        bool
	themeUnavailableDetail  string
	ipLocationLabel         string
	timeoutOptions          []timeoutChoice
	choice                  choiceSurface
	themeRefreshing         bool
	style, exStyle          uint32
	controlBounds           map[uint16]logicalBounds
	captureScale            float64
	captureHost             bool
	pendingDPI              uint32
	pendingDPISuggested     rect
	pendingDPIHasSuggested  bool
	dpiGeneration           uint32
	dpiReadyGeneration      uint32
	dpiApplyPosted          bool
	dpiCommitPosted         bool
	dpiCommitFailures       uint8
	dpiTransition           *nativeform.FrameTransition
}

// choiceSurface retains selector data while nativeform owns the one shared
// popup implementation used by both the main panel and form windows.
type choiceSurface struct {
	options  map[uint16][]string
	selected map[uint16]int
	openID   uint16
	serial   uint64
	popup    *nativeform.ChoicePopup
}

// Show opens the panel or closes the currently open panel. It must be called
// from the thread that owns the application's Win32 message loop.
