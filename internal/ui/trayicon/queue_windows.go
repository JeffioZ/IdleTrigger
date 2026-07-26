package trayicon

import (
	"golang.org/x/sys/windows"
	"runtime/debug"
)

// Post runs fn on the tray window's UI thread. It is intended for transient
// UI such as notifications that must be painted by a Win32 message loop.
func Post(fn func()) bool {
	if fn == nil {
		return false
	}
	wt.muUITasks.Lock()
	if wt.window == 0 || wt.uiClosing {
		wt.muUITasks.Unlock()
		return false
	}
	wt.uiTasks = append(wt.uiTasks, fn)
	window := wt.window
	wt.muUITasks.Unlock()
	result, _, _ := pPostMessage.Call(uintptr(window), wmRunUITask, 0, 0)
	return result != 0
}

// PostAndWait runs fn on the tray UI thread and waits for it to finish.
// It must not be called from the tray UI thread itself.
func PostAndWait(fn func()) bool {
	if fn == nil {
		return false
	}
	done := make(chan struct{})
	if !Post(func() {
		defer close(done)
		fn()
	}) {
		return false
	}
	wt.muUITasks.Lock()
	stopped := wt.uiStopped
	wt.muUITasks.Unlock()
	return waitForUITask(done, stopped)
}

func waitForUITask(done, stopped <-chan struct{}) bool {
	if stopped == nil {
		<-done
		return true
	}
	select {
	case <-done:
		return true
	case <-stopped:
		return false
	}
}

// WindowHandle returns the hidden native window that owns the notification
// icon. Transient app windows can use it as an owner to stay out of Alt+Tab
// without opting into the legacy tool-window title bar.
func WindowHandle() windows.Handle {
	wt.muUITasks.Lock()
	defer wt.muUITasks.Unlock()
	return wt.window
}

func (t *winTray) uiAvailable() bool {
	t.muUITasks.Lock()
	defer t.muUITasks.Unlock()
	return t.window != 0 && !t.uiClosing
}

func (t *winTray) drainUITasks() {
	t.muUITasks.Lock()
	tasks := t.uiTasks
	t.uiTasks = nil
	t.muUITasks.Unlock()
	for _, task := range tasks {
		func() {
			defer func() {
				if recovered := recover(); recovered != nil {
					reportError("tray UI task panic: %v\n%s", recovered, debug.Stack())
				}
			}()
			task()
		}()
	}
}

func (t *winTray) beginUIShutdown() {
	t.muUITasks.Lock()
	if t.uiClosing {
		t.muUITasks.Unlock()
		return
	}
	t.uiClosing = true
	t.window = 0
	t.uiTasks = nil
	if t.uiStopped != nil {
		close(t.uiStopped)
	}
	t.muUITasks.Unlock()
}

func (t *winTray) shutdown() {
	t.shutdownOnce.Do(func() {
		t.beginUIShutdown()
		t.removeNotificationIcon()
		systrayExit()
	})
}
