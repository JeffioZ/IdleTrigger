package trayicon

import (
	"golang.org/x/sys/windows"
	"sync"
	"sync/atomic"
	"unsafe"
)

var (
	g32                     = windows.NewLazySystemDLL("Gdi32.dll")
	pCreateCompatibleBitmap = g32.NewProc("CreateCompatibleBitmap")
	pCreateCompatibleDC     = g32.NewProc("CreateCompatibleDC")
	pDeleteDC               = g32.NewProc("DeleteDC")
	pSelectObject           = g32.NewProc("SelectObject")

	k32              = windows.NewLazySystemDLL("Kernel32.dll")
	pGetModuleHandle = k32.NewProc("GetModuleHandleW")

	s32                     = windows.NewLazySystemDLL("Shell32.dll")
	pSHAppBarMessage        = s32.NewProc("SHAppBarMessage")
	pShellNotifyIcon        = s32.NewProc("Shell_NotifyIconW")
	pShellNotifyIconGetRect = s32.NewProc("Shell_NotifyIconGetRect")

	shcore                    = windows.NewLazySystemDLL("Shcore.dll")
	pGetScaleFactorForMonitor = shcore.NewProc("GetScaleFactorForMonitor")

	u32                                 = windows.NewLazySystemDLL("User32.dll")
	pCreateMenu                         = u32.NewProc("CreateMenu")
	pCreatePopupMenu                    = u32.NewProc("CreatePopupMenu")
	pCreateWindowEx                     = u32.NewProc("CreateWindowExW")
	pDefWindowProc                      = u32.NewProc("DefWindowProcW")
	pRemoveMenu                         = u32.NewProc("RemoveMenu")
	pDestroyWindow                      = u32.NewProc("DestroyWindow")
	pDispatchMessage                    = u32.NewProc("DispatchMessageW")
	pDrawIconEx                         = u32.NewProc("DrawIconEx")
	pGetCursorPos                       = u32.NewProc("GetCursorPos")
	pGetDC                              = u32.NewProc("GetDC")
	pGetMessage                         = u32.NewProc("GetMessageW")
	pIsChild                            = u32.NewProc("IsChild")
	pIsDialogMessage                    = u32.NewProc("IsDialogMessageW")
	pGetSystemMetrics                   = u32.NewProc("GetSystemMetrics")
	pInsertMenuItem                     = u32.NewProc("InsertMenuItemW")
	pKillTimer                          = u32.NewProc("KillTimer")
	pLoadCursor                         = u32.NewProc("LoadCursorW")
	pDestroyIcon                        = u32.NewProc("DestroyIcon")
	pLoadIcon                           = u32.NewProc("LoadIconW")
	pLoadImage                          = u32.NewProc("LoadImageW")
	pMonitorFromRect                    = u32.NewProc("MonitorFromRect")
	pPostMessage                        = u32.NewProc("PostMessageW")
	pPostQuitMessage                    = u32.NewProc("PostQuitMessage")
	pRegisterClass                      = u32.NewProc("RegisterClassExW")
	pRegisterWindowMessage              = u32.NewProc("RegisterWindowMessageW")
	pRegisterPowerSettingNotification   = u32.NewProc("RegisterPowerSettingNotification")
	pUnregisterPowerSettingNotification = u32.NewProc("UnregisterPowerSettingNotification")
	pReleaseDC                          = u32.NewProc("ReleaseDC")
	pSetForegroundWindow                = u32.NewProc("SetForegroundWindow")
	pSetMenuInfo                        = u32.NewProc("SetMenuInfo")
	pSetMenuItemInfo                    = u32.NewProc("SetMenuItemInfoW")
	pSetTimer                           = u32.NewProc("SetTimer")
	pTrackPopupMenu                     = u32.NewProc("TrackPopupMenu")
	pTranslateMessage                   = u32.NewProc("TranslateMessage")
	pUnregisterClass                    = u32.NewProc("UnregisterClassW")
)

// The notification-area host must remain a real hidden top-level window so it
// receives broadcast power, theme and Explorer-restart messages. It does not
// need a caption, frame, default placement or client area: WS_POPUP at 0x0
// cannot flash a normal window frame even if the shell is still composing the
// process's first top-level HWND.
const trayHostWindowStyle = 0x80000000 // WS_POPUP, deliberately without WS_VISIBLE.
const trayHostWindowCoordinate = int32(-32000)

const (
	wmKeyDown = 0x0100
	wmClose   = 0x0010
	vkTab     = 0x09
	vkEscape  = 0x1b
)

// message matches the Win32 MSG layout used by GetMessageW and
// IsDialogMessageW.
type message struct {
	WindowHandle windows.Handle
	Message      uint32
	Wparam       uintptr
	Lparam       uintptr
	Time         uint32
	Pt           point
}

type tabNavigationEntry struct {
	hwnd        windows.Handle
	onNavigated func()
}

var tabNavigation struct {
	sync.RWMutex
	hwnd        windows.Handle
	onNavigated func()
	stack       []tabNavigationEntry
}

var themeChangePending atomic.Bool

const (
	wmSysColorChange = 0x0015
	wmSettingChange  = 0x001A
	wmDisplayChange  = 0x007E
	wmDPIChanged     = 0x02E0
	wmThemeChanged   = 0x031A
)

func isThemeChangeMessage(message uint32) bool {
	return message == wmSettingChange || message == wmSysColorChange || message == wmThemeChanged
}

func isTrayIconRefreshMessage(message uint32) bool {
	return message == wmSettingChange || message == wmDisplayChange || message == wmDPIChanged
}

func notifyThemeChange() {
	_, _, _, callback := callbacks()
	if callback == nil || !themeChangePending.CompareAndSwap(false, true) {
		return
	}
	go func() {
		defer themeChangePending.Store(false)
		callback()
	}()
}

// SetTabNavigationWindow activates standard dialog Tab navigation for one
// modeless window. Nested owned dialogs temporarily stack the previous window.
// Only WM_KEYDOWN/VK_TAB messages addressed to the active window or one of its
// children are passed to IsDialogMessageW.
func SetTabNavigationWindow(hwnd windows.Handle, onNavigated func()) {
	tabNavigation.Lock()
	if tabNavigation.hwnd == hwnd {
		tabNavigation.onNavigated = onNavigated
		tabNavigation.Unlock()
		return
	}
	filtered := tabNavigation.stack[:0]
	for _, entry := range tabNavigation.stack {
		if entry.hwnd != hwnd {
			filtered = append(filtered, entry)
		}
	}
	tabNavigation.stack = filtered
	if tabNavigation.hwnd != 0 {
		tabNavigation.stack = append(tabNavigation.stack, tabNavigationEntry{hwnd: tabNavigation.hwnd, onNavigated: tabNavigation.onNavigated})
	}
	tabNavigation.hwnd = hwnd
	tabNavigation.onNavigated = onNavigated
	tabNavigation.Unlock()
}

// ClearTabNavigationWindow removes a window's registration and restores the
// previous owned window when the active dialog closes.
func ClearTabNavigationWindow(hwnd windows.Handle) {
	tabNavigation.Lock()
	if tabNavigation.hwnd == hwnd {
		if last := len(tabNavigation.stack) - 1; last >= 0 {
			entry := tabNavigation.stack[last]
			tabNavigation.stack = tabNavigation.stack[:last]
			tabNavigation.hwnd = entry.hwnd
			tabNavigation.onNavigated = entry.onNavigated
		} else {
			tabNavigation.hwnd = 0
			tabNavigation.onNavigated = nil
		}
	} else {
		filtered := tabNavigation.stack[:0]
		for _, entry := range tabNavigation.stack {
			if entry.hwnd != hwnd {
				filtered = append(filtered, entry)
			}
		}
		tabNavigation.stack = filtered
	}
	tabNavigation.Unlock()
}

func isTabNavigationMessage(m *message, dialog windows.Handle, isChild bool) bool {
	return dialog != 0 && m != nil && m.Message == wmKeyDown && m.Wparam == vkTab && (m.WindowHandle == dialog || isChild)
}

func isDialogEscapeMessage(m *message, dialog windows.Handle, isChild bool) bool {
	return dialog != 0 && m != nil && m.Message == wmKeyDown && m.Wparam == vkEscape && (m.WindowHandle == dialog || isChild)
}

func dispatchTabNavigation(m *message) bool {
	tabNavigation.RLock()
	dialog := tabNavigation.hwnd
	onNavigated := tabNavigation.onNavigated
	tabNavigation.RUnlock()
	if dialog == 0 || m == nil {
		return false
	}
	isChild := false
	if m.WindowHandle != 0 && m.WindowHandle != dialog {
		child, _, _ := pIsChild.Call(uintptr(dialog), uintptr(m.WindowHandle))
		isChild = child != 0
	}
	if isDialogEscapeMessage(m, dialog, isChild) {
		pPostMessage.Call(uintptr(dialog), wmClose, 0, 0)
		return true
	}
	if !isTabNavigationMessage(m, dialog, isChild) {
		return false
	}
	processed, _, _ := pIsDialogMessage.Call(uintptr(dialog), uintptr(unsafe.Pointer(m)))
	if processed == 0 {
		return false
	}
	if onNavigated != nil {
		onNavigated()
	}
	return true
}

// Contains window class information.
// It is used with the RegisterClassEx and GetClassInfoEx functions.
// https://msdn.microsoft.com/en-us/library/ms633577.aspx
type wndClassEx struct {
	Size, Style                        uint32
	WndProc                            uintptr
	ClsExtra, WndExtra                 int32
	Instance, Icon, Cursor, Background windows.Handle
	MenuName, ClassName                *uint16
	IconSm                             windows.Handle
}

// Registers a window class for subsequent use in calls to the CreateWindow or CreateWindowEx function.
// https://msdn.microsoft.com/en-us/library/ms633587.aspx
func (w *wndClassEx) register() error {
	w.Size = uint32(unsafe.Sizeof(*w))
	res, _, err := pRegisterClass.Call(uintptr(unsafe.Pointer(w)))
	if res == 0 {
		return err
	}
	return nil
}

// Unregisters a window class, freeing the memory required for the class.
// https://msdn.microsoft.com/en-us/library/ms644899.aspx
func (w *wndClassEx) unregister() error {
	res, _, err := pUnregisterClass.Call(
		uintptr(unsafe.Pointer(w.ClassName)),
		uintptr(w.Instance),
	)
	if res == 0 {
		return err
	}
	return nil
}

// Contains information that the system needs to display notifications in the notification area.
// Used by Shell_NotifyIcon.
// https://msdn.microsoft.com/en-us/library/windows/desktop/bb773352(v=vs.85).aspx
// https://msdn.microsoft.com/en-us/library/windows/desktop/bb762159
type notifyIconData struct {
	Size                       uint32
	Wnd                        windows.Handle
	ID, Flags, CallbackMessage uint32
	Icon                       windows.Handle
	Tip                        [128]uint16
	State, StateMask           uint32
	Info                       [256]uint16
	Timeout, Version           uint32
	InfoTitle                  [64]uint16
	InfoFlags                  uint32
	GuidItem                   windows.GUID
	BalloonIcon                windows.Handle
}

func (nid *notifyIconData) add() error {
	const NIM_ADD = 0x00000000
	res, _, err := pShellNotifyIcon.Call(
		uintptr(NIM_ADD),
		uintptr(unsafe.Pointer(nid)),
	)
	if res == 0 {
		return err
	}
	return nil
}

func (nid *notifyIconData) modify() error {
	const NIM_MODIFY = 0x00000001
	res, _, err := pShellNotifyIcon.Call(
		uintptr(NIM_MODIFY),
		uintptr(unsafe.Pointer(nid)),
	)
	if res == 0 {
		return err
	}
	return nil
}

func (nid *notifyIconData) delete() error {
	const NIM_DELETE = 0x00000002
	res, _, err := pShellNotifyIcon.Call(
		uintptr(NIM_DELETE),
		uintptr(unsafe.Pointer(nid)),
	)
	if res == 0 {
		return err
	}
	return nil
}

// Contains information about a menu item.
// https://msdn.microsoft.com/en-us/library/windows/desktop/ms647578(v=vs.85).aspx
type menuItemInfo struct {
	Size, Mask, Type, State     uint32
	ID                          uint32
	SubMenu, Checked, Unchecked windows.Handle
	ItemData                    uintptr
	TypeData                    *uint16
	Cch                         uint32
	BMPItem                     windows.Handle
}

// The POINT structure defines the x- and y- coordinates of a point.
// https://msdn.microsoft.com/en-us/library/windows/desktop/dd162805(v=vs.85).aspx
type point struct {
	X, Y int32
}

type rect struct {
	Left, Top, Right, Bottom int32
}

type notifyIconIdentifier struct {
	Size     uint32
	Wnd      windows.Handle
	ID       uint32
	GuidItem windows.GUID
}

type appBarData struct {
	Size            uint32
	Wnd             windows.Handle
	CallbackMessage uint32
	Edge            uint32
	Rect            rect
	LParam          uintptr
}

// Contains information about loaded resources
type loadedImageKey struct {
	resourceID uint16
	width      uint32
	height     uint32
}

type winTray struct {
	instance,
	icon,
	cursor,
	window windows.Handle

	loadedImages         map[loadedImageKey]windows.Handle
	muLoadedImages       sync.RWMutex
	iconsReleased        bool
	muIconLifecycle      sync.Mutex
	trayIconKey          loadedImageKey
	trayIconProbeAttempt int
	// menus keeps track of the submenus keyed by the menu item ID, plus 0
	// which corresponds to the main popup menu.
	menus   map[uint32]windows.Handle
	muMenus sync.RWMutex
	// menuOf keeps track of the menu each menu item belongs to.
	menuOf   map[uint32]windows.Handle
	muMenuOf sync.RWMutex
	// menuItemIcons maintains the bitmap of each menu item (if applies). It's
	// needed to show the icon correctly when showing a previously hidden menu
	// item again.
	menuItemIcons   map[uint32]windows.Handle
	muMenuItemIcons sync.RWMutex
	visibleItems    map[uint32][]uint32
	muVisibleItems  sync.RWMutex

	nid   *notifyIconData
	muNID sync.RWMutex
	wcex  *wndClassEx

	uiTasks            []func()
	uiStopped          chan struct{}
	uiClosing          bool
	muUITasks          sync.Mutex
	shutdownOnce       sync.Once
	powerNotifications []windows.Handle

	wmSystrayMessage,
	wmTaskbarCreated uint32
}
