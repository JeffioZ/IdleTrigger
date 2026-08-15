// Package trayicon provides IdleTrigger's native Windows notification-area
// icon, menu, and UI event loop. It is derived from getlantern/systray.
package trayicon

import (
	"errors"
	"runtime"
	"sync"
	"sync/atomic"
)

var (
	systrayReady    func()
	systrayExit     func()
	menuItems       = make(map[uint32]*MenuItem)
	menuItemsLock   sync.RWMutex
	errorHandler    = func(string, ...interface{}) {}
	errorLock       sync.RWMutex
	callbackLock    sync.RWMutex
	onLeftClick     func()
	onPowerChange   func(PowerEvent)
	onDisplayChange func()
	onThemeChange   func()

	currentID = uint32(0)
	quitOnce  sync.Once

	errTrayUnavailable = errors.New("tray UI is unavailable")
)

// SetErrorHandler routes internal tray errors to the application's logger.
func SetErrorHandler(handler func(string, ...interface{})) {
	errorLock.Lock()
	defer errorLock.Unlock()
	if handler == nil {
		errorHandler = func(string, ...interface{}) {}
		return
	}
	errorHandler = handler
}

func reportError(format string, args ...interface{}) {
	errorLock.RLock()
	handler := errorHandler
	errorLock.RUnlock()
	handler(format, args...)
}

func init() {
	runtime.LockOSThread()
}

// MenuItem tracks one native tray menu item.
type MenuItem struct {
	// ClickedCh is the channel which will be notified when the menu item is clicked
	ClickedCh chan struct{}

	// id uniquely identify a menu item, not supposed to be modified
	id uint32
	// title is the text shown on menu item
	title string
}

// newMenuItem returns a populated MenuItem object
func newMenuItem(title string) *MenuItem {
	return &MenuItem{
		ClickedCh: make(chan struct{}),
		id:        atomic.AddUint32(&currentID, 1),
		title:     title,
	}
}

// Run initializes GUI and starts the event loop, then invokes the onReady
// callback. It blocks until trayicon.Quit() is called.
func Run(onReady func(), onExit func()) {
	if !register(onReady, onExit) {
		return
	}
	nativeLoop()
}

func register(onReady func(), onExit func()) bool {
	if onReady == nil {
		systrayReady = func() {}
	} else {
		systrayReady = onReady
	}
	// unlike onReady, onExit runs in the event loop to make sure it has time to
	// finish before the process terminates
	if onExit == nil {
		onExit = func() {}
	}
	systrayExit = onExit
	return registerSystray()
}

// Quit the systray
func Quit() {
	quitOnce.Do(quit)
}

// AddMenuItem adds a command to IdleTrigger's flat tray menu.
// It can be safely invoked from different goroutines.
func AddMenuItem(title string) *MenuItem {
	item := newMenuItem(title)
	item.update()
	return item
}

// SetTitle set the text to display on a menu item
func (item *MenuItem) SetTitle(title string) {
	item.title = title
	item.update()
}

// update propagates changes on a menu item to systray
func (item *MenuItem) update() {
	menuItemsLock.Lock()
	menuItems[item.id] = item
	menuItemsLock.Unlock()
	addOrUpdateMenuItem(item)
}

func systrayMenuItemSelected(id uint32) {
	menuItemsLock.RLock()
	item, ok := menuItems[id]
	menuItemsLock.RUnlock()
	if !ok {
		reportError("No menu item with ID %v", id)
		return
	}
	select {
	case item.ClickedCh <- struct{}{}:
	// in case no one waiting for the channel
	default:
	}
}

// SetOnLeftClick sets the callback used when the user left-clicks the tray icon.
func SetOnLeftClick(fn func()) {
	callbackLock.Lock()
	onLeftClick = fn
	callbackLock.Unlock()
}

// SetOnPowerChange sets the callback used for Windows power-state changes.
func SetOnPowerChange(fn func(PowerEvent)) {
	callbackLock.Lock()
	onPowerChange = fn
	callbackLock.Unlock()
}

// SetOnDisplayChange sets the callback used when Windows reports a display
// topology change. The callback is separate from tray-icon convergence so the
// application can delay theme recovery until the desktop is stable.
func SetOnDisplayChange(fn func()) {
	callbackLock.Lock()
	onDisplayChange = fn
	callbackLock.Unlock()
}

// SetOnThemeChange sets the callback used for a burst of theme/color changes.
func SetOnThemeChange(fn func()) {
	callbackLock.Lock()
	onThemeChange = fn
	callbackLock.Unlock()
}

func callbacks() (leftClick func(), powerChange func(PowerEvent), displayChange func(), themeChange func()) {
	callbackLock.RLock()
	defer callbackLock.RUnlock()
	return onLeftClick, onPowerChange, onDisplayChange, onThemeChange
}

// PowerSetting identifies the registered power setting carried by a Windows
// PBT_POWERSETTINGCHANGE broadcast.
type PowerSetting uint8

const (
	PowerSettingNone PowerSetting = iota
	PowerSettingACSource
	PowerSettingBatteryPercentage
)

// PowerEvent preserves the broadcast code and, when available, the DWORD
// payload for one of IdleTrigger's explicitly registered power settings.
type PowerEvent struct {
	Code     uint32
	Setting  PowerSetting
	Value    uint32
	HasValue bool
}
