package trayicon

import (
	"github.com/JeffioZ/idletrigger/internal/platform/windows/darkmode"
	"golang.org/x/sys/windows"
	"sync"
	"unsafe"
)

// WindowProc callback function that processes messages sent to a window.
// https://msdn.microsoft.com/en-us/library/windows/desktop/ms633573(v=vs.85).aspx
func (t *winTray) wndProc(hWnd windows.Handle, message uint32, wParam, lParam uintptr) (lResult uintptr) {
	const (
		WM_RBUTTONUP      = 0x0205
		WM_LBUTTONUP      = 0x0202
		WM_COMMAND        = 0x0111
		WM_TIMER          = 0x0113
		WM_ENDSESSION     = 0x0016
		WM_POWERBROADCAST = 0x0218
		WM_CLOSE          = 0x0010
		WM_DESTROY        = 0x0002
	)
	switch message {
	case wmSettingChange, wmSysColorChange, wmThemeChanged:
		notifyThemeChange()
		if isTrayIconRefreshMessage(message) {
			t.startTrayIconConvergence()
		}
	case wmDisplayChange, wmDPIChanged:
		t.startTrayIconConvergence()
		if message == wmDisplayChange {
			_, _, callback, _ := callbacks()
			if callback != nil {
				go callback()
			}
		}
	case wmStartTrayIconConvergence:
		t.startTrayIconConvergence()
	case WM_TIMER:
		if wParam == trayIconProbeTimerID {
			t.continueTrayIconConvergence()
		}
	case wmRunUITask:
		t.drainUITasks()
	case WM_COMMAND:
		menuItemId := int32(wParam)
		// https://docs.microsoft.com/en-us/windows/win32/menurc/wm-command#menus
		if menuItemId != -1 {
			systrayMenuItemSelected(uint32(wParam))
		}
	case WM_POWERBROADCAST:
		_, callback, _, _ := callbacks()
		if callback != nil {
			event := decodePowerEvent(uint32(wParam), lParam)
			go callback(event)
		}
		lResult = 1
	case WM_CLOSE:
		pDestroyWindow.Call(uintptr(t.window))
		t.wcex.unregister()
	case WM_DESTROY:
		t.stopTrayIconConvergence()
		t.shutdown()
		pPostQuitMessage.Call(uintptr(int32(0)))
	case WM_ENDSESSION:
		if wParam != 0 {
			t.stopTrayIconConvergence()
			t.shutdown()
		}
	case t.wmSystrayMessage:
		switch lParam {
		case WM_LBUTTONUP:
			callback, _, _, _ := callbacks()
			if callback != nil {
				callback()
			} else {
				t.showMenu()
			}
		case WM_RBUTTONUP:
			t.showMenu()
		}
	case t.wmTaskbarCreated: // on explorer.exe restarts
		t.muIconLifecycle.Lock()
		t.muNID.Lock()
		if t.nid != nil {
			t.nid.add()
		}
		t.muNID.Unlock()
		t.muIconLifecycle.Unlock()
		t.startTrayIconConvergence()
	default:
		// Calls the default window procedure to provide default processing for any window messages that an application does not process.
		// https://msdn.microsoft.com/en-us/library/windows/desktop/ms633572(v=vs.85).aspx
		lResult, _, _ = pDefWindowProc.Call(
			uintptr(hWnd),
			uintptr(message),
			uintptr(wParam),
			uintptr(lParam),
		)
	}
	return
}

func (t *winTray) initInstance() error {
	const IDI_APPLICATION = 32512
	const IDC_ARROW = 32512 // Standard arrow
	const NIF_MESSAGE = 0x00000001

	// https://msdn.microsoft.com/en-us/library/windows/desktop/ms644931(v=vs.85).aspx
	const WM_USER = 0x0400

	const (
		className  = "SystrayClass"
		windowName = ""
	)

	t.wmSystrayMessage = WM_USER + 1
	t.muUITasks.Lock()
	t.uiTasks = nil
	t.uiStopped = make(chan struct{})
	t.uiClosing = false
	t.muUITasks.Unlock()
	t.shutdownOnce = sync.Once{}
	t.menu = 0
	t.visibleItems = nil

	taskbarEventNamePtr, _ := windows.UTF16PtrFromString("TaskbarCreated")
	// https://msdn.microsoft.com/en-us/library/windows/desktop/ms644947
	res, _, _ := pRegisterWindowMessage.Call(
		uintptr(unsafe.Pointer(taskbarEventNamePtr)),
	)
	t.wmTaskbarCreated = uint32(res)

	t.muLoadedImages.Lock()
	t.loadedImages = make(map[loadedImageKey]windows.Handle)
	t.iconsReleased = false
	t.muLoadedImages.Unlock()
	t.trayIconKey = loadedImageKey{}
	t.trayIconProbeAttempt = 0

	instanceHandle, _, err := pGetModuleHandle.Call(0)
	if instanceHandle == 0 {
		return err
	}
	t.instance = windows.Handle(instanceHandle)

	// https://msdn.microsoft.com/en-us/library/windows/desktop/ms648072(v=vs.85).aspx
	iconHandle, _, err := pLoadIcon.Call(0, uintptr(IDI_APPLICATION))
	if iconHandle == 0 {
		return err
	}
	t.icon = windows.Handle(iconHandle)

	// https://msdn.microsoft.com/en-us/library/windows/desktop/ms648391(v=vs.85).aspx
	cursorHandle, _, err := pLoadCursor.Call(0, uintptr(IDC_ARROW))
	if cursorHandle == 0 {
		return err
	}
	t.cursor = windows.Handle(cursorHandle)

	classNamePtr, err := windows.UTF16PtrFromString(className)
	if err != nil {
		return err
	}

	windowNamePtr, err := windows.UTF16PtrFromString(windowName)
	if err != nil {
		return err
	}

	t.wcex = &wndClassEx{
		WndProc:    windows.NewCallback(t.wndProc),
		Instance:   t.instance,
		Icon:       t.icon,
		Cursor:     t.cursor,
		Background: windows.Handle(6), // (COLOR_WINDOW + 1)
		ClassName:  classNamePtr,
		IconSm:     t.icon,
	}
	if err := t.wcex.register(); err != nil {
		return err
	}

	hiddenCoordinate := trayHostWindowCoordinate
	windowHandle, _, err := pCreateWindowEx.Call(
		uintptr(0),
		uintptr(unsafe.Pointer(classNamePtr)),
		uintptr(unsafe.Pointer(windowNamePtr)),
		uintptr(trayHostWindowStyle),
		uintptr(uint32(hiddenCoordinate)),
		uintptr(uint32(hiddenCoordinate)),
		0,
		0,
		uintptr(0),
		uintptr(0),
		uintptr(t.instance),
		uintptr(0),
	)
	if windowHandle == 0 {
		return err
	}
	t.window = windows.Handle(windowHandle)
	darkmode.AllowWindow(uintptr(t.window))
	t.registerPowerNotifications()

	t.muNID.Lock()
	defer t.muNID.Unlock()
	t.nid = &notifyIconData{
		Wnd:             windows.Handle(t.window),
		ID:              100,
		Flags:           NIF_MESSAGE,
		CallbackMessage: t.wmSystrayMessage,
	}
	t.nid.Size = uint32(unsafe.Sizeof(*t.nid))

	return t.nid.add()
}
