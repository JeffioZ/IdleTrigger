package trayicon

import (
	"fmt"
	"strings"
	"testing"
	"unsafe"

	"golang.org/x/sys/windows"
)

func TestMissingMenuItemUsesErrorHandler(t *testing.T) {
	var message string
	SetErrorHandler(func(format string, args ...interface{}) {
		message = fmt.Sprintf(format, args...)
	})
	t.Cleanup(func() { SetErrorHandler(nil) })

	systrayMenuItemSelected(^uint32(0))
	if !strings.Contains(message, "No menu item with ID") {
		t.Fatalf("unexpected error message: %q", message)
	}
}

func TestTabNavigationMessageScope(t *testing.T) {
	dialog := windows.Handle(100)
	child := windows.Handle(101)
	tests := []struct {
		name    string
		message *message
		isChild bool
		want    bool
	}{
		{name: "popup tab", message: &message{WindowHandle: dialog, Message: wmKeyDown, Wparam: vkTab}, want: true},
		{name: "child tab", message: &message{WindowHandle: child, Message: wmKeyDown, Wparam: vkTab}, isChild: true, want: true},
		{name: "unrelated window", message: &message{WindowHandle: child, Message: wmKeyDown, Wparam: vkTab}, want: false},
		{name: "enter remains normal", message: &message{WindowHandle: dialog, Message: wmKeyDown, Wparam: 0x0D}, want: false},
		{name: "tab character remains normal", message: &message{WindowHandle: dialog, Message: 0x0102, Wparam: vkTab}, want: false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := isTabNavigationMessage(tt.message, dialog, tt.isChild); got != tt.want {
				t.Fatalf("isTabNavigationMessage() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestNestedTabNavigationRestoresOwner(t *testing.T) {
	tabNavigation.Lock()
	tabNavigation.hwnd = 0
	tabNavigation.onNavigated = nil
	tabNavigation.stack = nil
	tabNavigation.Unlock()
	t.Cleanup(func() {
		tabNavigation.Lock()
		tabNavigation.hwnd = 0
		tabNavigation.onNavigated = nil
		tabNavigation.stack = nil
		tabNavigation.Unlock()
	})

	owner, child, warning := windows.Handle(100), windows.Handle(200), windows.Handle(300)
	ownerNavigated := 0
	SetTabNavigationWindow(owner, func() { ownerNavigated++ })
	SetTabNavigationWindow(child, nil)
	SetTabNavigationWindow(warning, nil)
	ClearTabNavigationWindow(child)
	ClearTabNavigationWindow(warning)
	if tabNavigation.hwnd != owner || len(tabNavigation.stack) != 0 {
		t.Fatalf("active=%d stack=%+v", tabNavigation.hwnd, tabNavigation.stack)
	}
	tabNavigation.onNavigated()
	if ownerNavigated != 1 {
		t.Fatal("nested dialog cleanup did not restore the owner's Tab-navigation callback")
	}
	ClearTabNavigationWindow(owner)
}

func TestThemeChangeMessageScope(t *testing.T) {
	for _, message := range []uint32{wmSettingChange, wmSysColorChange, wmThemeChanged} {
		if !isThemeChangeMessage(message) {
			t.Fatalf("theme message %#x was not recognized", message)
		}
	}
	if isThemeChangeMessage(wmKeyDown) {
		t.Fatal("keyboard messages must not trigger a theme refresh")
	}
}

func TestTrayIconRefreshMessageScope(t *testing.T) {
	for _, message := range []uint32{wmSettingChange, wmDisplayChange, wmDPIChanged} {
		if !isTrayIconRefreshMessage(message) {
			t.Fatalf("display message %#x was not recognized", message)
		}
	}
	if isTrayIconRefreshMessage(wmThemeChanged) {
		t.Fatal("theme-only messages must not reload the same tray icon")
	}
}

func TestTrayIconSideUsesShellReportedBounds(t *testing.T) {
	tests := []struct {
		name string
		rect rect
		want uint32
	}{
		{name: "24px icon", rect: rect{Left: 100, Top: 20, Right: 124, Bottom: 44}, want: 24},
		{name: "square inside taller slot", rect: rect{Left: 100, Top: 20, Right: 132, Bottom: 60}, want: 32},
		{name: "empty", rect: rect{}, want: 0},
		{name: "smaller than a tray icon", rect: rect{Right: 8, Bottom: 8}, want: 0},
		{name: "reversed", rect: rect{Left: 20, Top: 20, Right: 10, Bottom: 10}, want: 0},
		{name: "implausibly large", rect: rect{Right: 300, Bottom: 300}, want: 0},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := trayIconSide(tt.rect); got != tt.want {
				t.Fatalf("trayIconSide(%+v) = %d, want %d", tt.rect, got, tt.want)
			}
		})
	}
}

func TestNotifyIconIdentifierMatchesWindowsLayout(t *testing.T) {
	want := uintptr(28)
	appBarWant := uintptr(36)
	if unsafe.Sizeof(uintptr(0)) == 8 {
		want = 40
		appBarWant = 48
	}
	if got := unsafe.Sizeof(notifyIconIdentifier{}); got != want {
		t.Fatalf("NOTIFYICONIDENTIFIER size = %d, want %d", got, want)
	}
	if got := unsafe.Sizeof(rect{}); got != 16 {
		t.Fatalf("RECT size = %d, want 16", got)
	}
	if got := unsafe.Sizeof(appBarData{}); got != appBarWant {
		t.Fatalf("APPBARDATA size = %d, want %d", got, appBarWant)
	}
}

func TestSmallIconSideForScale(t *testing.T) {
	tests := map[uint32]uint32{
		0: 0, 100: 16, 125: 20, 150: 24, 175: 28, 200: 32, 300: 48, 400: 64, 600: 0,
	}
	for scale, want := range tests {
		if got := smallIconSideForScale(scale); got != want {
			t.Errorf("smallIconSideForScale(%d) = %d, want %d", scale, got, want)
		}
	}
}

func TestTrayHostWindowCannotExposeANormalFrame(t *testing.T) {
	const (
		wsVisible = 0x10000000
		wsCaption = 0x00C00000
		wsPopup   = 0x80000000
	)
	if trayHostWindowStyle&wsPopup == 0 {
		t.Fatal("tray host must remain a top-level popup for broadcast messages")
	}
	if trayHostWindowStyle&(wsVisible|wsCaption) != 0 {
		t.Fatalf("tray host style %#x can expose a visible framed window", uint32(trayHostWindowStyle))
	}
	if trayHostWindowCoordinate > -30000 {
		t.Fatalf("tray host creation coordinate %d is not safely outside the desktop", trayHostWindowCoordinate)
	}
}

func TestTakeLoadedIconsForReleaseTransfersHandlesOnce(t *testing.T) {
	tray := &winTray{loadedImages: map[loadedImageKey]windows.Handle{
		{resourceID: 1, width: 16, height: 16}: 101,
		{resourceID: 2, width: 32, height: 32}: 202,
		{resourceID: 3, width: 48, height: 48}: 0,
	}}
	first := tray.takeLoadedIconsForRelease()
	if len(first) != 2 || !tray.iconsReleased || tray.loadedImages != nil {
		t.Fatalf("first release = %#v, released=%v, cache=%#v", first, tray.iconsReleased, tray.loadedImages)
	}
	seen := map[windows.Handle]bool{}
	for _, icon := range first {
		seen[icon] = true
	}
	if !seen[101] || !seen[202] {
		t.Fatalf("released handles = %#v, want 101 and 202", first)
	}
	if second := tray.takeLoadedIconsForRelease(); len(second) != 0 {
		t.Fatalf("second release = %#v, want no handles", second)
	}
}

func TestWaitForUITaskStopsDuringShutdown(t *testing.T) {
	done := make(chan struct{})
	stopped := make(chan struct{})
	close(stopped)
	if waitForUITask(done, stopped) {
		t.Fatal("shutdown should cancel a pending synchronous UI task")
	}

	close(done)
	if !waitForUITask(done, make(chan struct{})) {
		t.Fatal("completed UI task should report success")
	}
}

func TestBeginUIShutdownIsIdempotent(t *testing.T) {
	stopped := make(chan struct{})
	tray := &winTray{
		window:    123,
		uiTasks:   []func(){func() {}},
		uiStopped: stopped,
	}
	tray.beginUIShutdown()
	tray.beginUIShutdown()
	if tray.window != 0 || !tray.uiClosing || tray.uiTasks != nil {
		t.Fatalf("shutdown state: window=%v closing=%v tasks=%d", tray.window, tray.uiClosing, len(tray.uiTasks))
	}
	select {
	case <-stopped:
	default:
		t.Fatal("shutdown did not notify synchronous UI waiters")
	}
}

func TestClosedTrayRejectsIconCacheAndTooltipUpdates(t *testing.T) {
	tray := &winTray{iconsReleased: true}
	if _, err := tray.loadIconResource(loadedImageKey{resourceID: 1, width: 16, height: 16}); err != errTrayUnavailable {
		t.Fatalf("closed icon cache error = %v, want %v", err, errTrayUnavailable)
	}
	if err := tray.setTooltip("ignored"); err != errTrayUnavailable {
		t.Fatalf("closed tooltip error = %v, want %v", err, errTrayUnavailable)
	}
}
