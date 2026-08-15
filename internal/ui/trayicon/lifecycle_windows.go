package trayicon

import "unsafe"

func registerSystray() bool {
	if err := wt.initInstance(); err != nil {
		reportError("Unable to init instance: %v", err)
		wt.abortInitialization()
		return false
	}

	if err := wt.createMenu(); err != nil {
		reportError("Unable to create menu: %v", err)
		wt.abortInitialization()
		return false
	}

	go systrayReady()
	return true
}

func (t *winTray) abortInitialization() {
	t.muUITasks.Lock()
	window := t.window
	t.muUITasks.Unlock()
	t.shutdown()
	if window != 0 {
		pDestroyWindow.Call(uintptr(window))
	}
	if t.wcex != nil {
		_ = t.wcex.unregister()
		t.wcex = nil
	}
}

func nativeLoop() {
	// Main message pump.
	m := &message{}
	for {
		ret, _, err := pGetMessage.Call(uintptr(unsafe.Pointer(m)), 0, 0, 0)

		// If the function retrieves a message other than WM_QUIT, the return value is nonzero.
		// If the function retrieves the WM_QUIT message, the return value is zero.
		// If there is an error, the return value is -1
		// https://msdn.microsoft.com/en-us/library/windows/desktop/ms644936(v=vs.85).aspx
		switch int32(ret) {
		case -1:
			reportError("Error at message loop: %v", err)
			return
		case 0:
			return
		default:
			if dispatchTabNavigation(m) {
				continue
			}
			pTranslateMessage.Call(uintptr(unsafe.Pointer(m)))
			pDispatchMessage.Call(uintptr(unsafe.Pointer(m)))
		}
	}
}

func quit() {
	const WM_CLOSE = 0x0010

	pPostMessage.Call(
		uintptr(wt.window),
		WM_CLOSE,
		0,
		0,
	)
}

// SetIconResource sets the tray icon from an RT_GROUP_ICON resource embedded
// in the current executable.
func SetIconResource(resourceID uint16) {
	if err := wt.setIcon(resourceID); err != nil {
		if err == errTrayUnavailable {
			return
		}
		reportError("Unable to set icon: %v", err)
		return
	}
}

// SetTooltip sets the systray tooltip to display on mouse hover of the tray icon,
// only available on Mac and Windows.
func SetTooltip(tooltip string) {
	if err := wt.setTooltip(tooltip); err != nil {
		if err == errTrayUnavailable {
			return
		}
		reportError("Unable to set tooltip: %v", err)
		return
	}
}

func addOrUpdateMenuItem(item *MenuItem) {
	if !wt.uiAvailable() {
		return
	}
	err := wt.addOrUpdateMenuItem(uint32(item.id), item.title)
	if err != nil {
		reportError("Unable to addOrUpdateMenuItem: %v", err)
		return
	}
}
