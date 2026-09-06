// Package locknotify displays lock-key state changes without intercepting input.
package locknotify

import (
	"fmt"
	"time"
	"unsafe"

	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	"github.com/JeffioZ/idletrigger/internal/i18n"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
	"golang.org/x/sys/windows"
)

var (
	user32              = windows.NewLazySystemDLL("user32.dll")
	gdi32               = windows.NewLazySystemDLL("gdi32.dll")
	registerClass       = user32.NewProc("RegisterClassExW")
	createWindow        = user32.NewProc("CreateWindowExW")
	destroyWindow       = user32.NewProc("DestroyWindow")
	defWindowProc       = user32.NewProc("DefWindowProcW")
	setTimer            = user32.NewProc("SetTimer")
	killTimer           = user32.NewProc("KillTimer")
	getKeyState         = user32.NewProc("GetKeyState")
	getForegroundWindow = user32.NewProc("GetForegroundWindow")
	monitorFromWindow   = user32.NewProc("MonitorFromWindow")
	getMonitorInfo      = user32.NewProc("GetMonitorInfoW")
	getDpi              = user32.NewProc("GetDpiForWindow")
	setWindowPos        = user32.NewProc("SetWindowPos")
	showWindow          = user32.NewProc("ShowWindow")
	systemParameters    = user32.NewProc("SystemParametersInfoW")
	beginPaint          = user32.NewProc("BeginPaint")
	endPaint            = user32.NewProc("EndPaint")
	drawText            = user32.NewProc("DrawTextW")
	deleteObject        = gdi32.NewProc("DeleteObject")
	selectObject        = gdi32.NewProc("SelectObject")
	setTextColor        = gdi32.NewProc("SetTextColor")
	setBkMode           = gdi32.NewProc("SetBkMode")
	openInputDesktop    = user32.NewProc("OpenInputDesktop")
	closeDesktop        = user32.NewProc("CloseDesktop")
	getThreadDesktop    = user32.NewProc("GetThreadDesktop")
	getObjectInfo       = user32.NewProc("GetUserObjectInformationW")
	findWindow          = user32.NewProc("FindWindowW")
	isWindowVisible     = user32.NewProc("IsWindowVisible")
	callback            = windows.NewCallback(windowProc)
	active              *notification // Only accessed on the tray UI thread.
	registered          bool
)

const cardWidth, cardHeight = 140, 88

type rect struct{ Left, Top, Right, Bottom int32 }
type windowClass struct {
	Size, Style                        uint32
	Proc                               uintptr
	ClassExtra, WindowExtra            int32
	Instance, Icon, Cursor, Background windows.Handle
	Menu, Name                         *uint16
	SmallIcon                          windows.Handle
}
type notification struct {
	options             Options
	previewOnly         bool
	hwnd                uintptr
	language            string
	baseline            bool
	states              [3]bool
	lastPoll, shown     time.Time
	animated            bool
	on                  bool
	initialAlpha, alpha byte
	symbol, text        string

	frame    *surface
	dpi      uint32
	position point
	rise     int32
}

// Options controls passive lock-key notifications independently of previews.
type Options struct {
	Enabled        bool
	Keys           [3]bool
	SkipFullscreen bool
}

// Configure applies settings through the UI queue. Disabled means no automatic
// window or timers. Explicit previews own their short-lived window separately.
func Configure(options Options, language string) {
	trayicon.Post(func() { configure(options, language) })
}

func configure(options Options, language string) {
	if !options.Enabled {
		Close()
		return
	}
	if active != nil {
		if active.previewOnly {
			Close()
		} else {
			if active.options != options || active.language != language {
				active.hide()
				active.baseline = false
			}
			active.options = options
			active.language = language
			return
		}
	}
	name := windows.StringToUTF16Ptr("IdleTriggerLockNotification")
	if !registered {
		wc := windowClass{Size: uint32(unsafe.Sizeof(windowClass{})), Proc: callback, Name: name}
		ok, _, err := registerClass.Call(uintptr(unsafe.Pointer(&wc)))
		if ok == 0 {
			mylog.Info("Lock notification class registration failed: %v", err)
			return
		}
		registered = true
	}
	// Layered + transparent makes the popup click-through across processes.
	// NOACTIVATE keeps keyboard focus in the target application.
	hwnd, _, err := createWindow.Call(0x080800A8, uintptr(unsafe.Pointer(name)), 0, 0x80000000, 0, 0, 1, 1, 0, 0, 0, 0)
	if hwnd == 0 {
		mylog.Info("Lock notification window creation failed: %v", err)
		return
	}
	active = &notification{hwnd: hwnd, language: language, options: options}
	if id, _, err := setTimer.Call(hwnd, 1, 50, 0); id == 0 {
		mylog.Info("Lock notification timer creation failed: %v", err)
		Close()
	}
}

// Preview shows a fixed sample, without synthesizing input or changing settings.
// A temporary preview host is destroyed when the sample expires.
func Preview(language string) {
	trayicon.Post(func() {
		if !inputDesktopActive() {
			return
		}
		foreground, _, _ := getForegroundWindow.Call()
		if foreground == 0 {
			return
		}
		if active == nil {
			configure(Options{Enabled: true}, language)
			if active == nil {
				return
			}
			active.previewOnly = true
		}
		active.show(0, true, foreground, time.Now())
		if active != nil && active.previewOnly && active.shown.IsZero() {
			Close()
		}
	})
}

// Close runs on the tray UI thread, including during shutdown.
func Close() {
	if active != nil {
		destroyWindow.Call(active.hwnd)
	}
}

func desktopName(handle uintptr) string {
	var name [256]uint16
	var needed uint32
	ok, _, _ := getObjectInfo.Call(handle, 2, uintptr(unsafe.Pointer(&name[0])), uintptr(unsafe.Sizeof(name)), uintptr(unsafe.Pointer(&needed)))
	if ok == 0 {
		return ""
	}
	return windows.UTF16ToString(name[:])
}

func inputDesktopActive() bool {
	desktop, _, _ := openInputDesktop.Call(0, 0, 1)
	if desktop == 0 {
		return false
	}
	defer closeDesktop.Call(desktop)
	threadDesktop, _, _ := getThreadDesktop.Call(uintptr(windows.GetCurrentThreadId()))
	name := desktopName(desktop)
	return name != "" && name == desktopName(threadDesktop)
}

func (n *notification) poll(now time.Time) {
	if n.previewOnly {
		if n.shown.IsZero() || !inputDesktopActive() {
			Close()
		}
		return
	}
	// Sample on the message-pumping UI thread, never a Go worker or a pre-input
	// hook. Sub-50ms transitions can coalesce; only observed states are shown.
	foreground, _, _ := getForegroundWindow.Call()
	if foreground == 0 || !inputDesktopActive() {
		n.baseline = false
		n.hide()
		return
	}
	if now.Sub(n.lastPoll) > time.Second {
		n.baseline = false
		n.hide()
	}
	n.lastPoll = now
	blocked := n.options.SkipFullscreen && theme.SuppressPassiveNotifications()
	if blocked {
		n.hide()
	}
	var states [3]bool
	for index, key := range [...]uintptr{0x14, 0x90, 0x91} {
		value, _, _ := getKeyState.Call(key)
		states[index] = value&1 != 0
	}
	for index, changed := range n.updateStates(states, blocked) {
		if changed {
			n.show(index, states[index], foreground, now)
		}
	}
}

// Always advance the baseline, including suppressed and unselected changes.
func (n *notification) updateStates(states [3]bool, blocked bool) (changed [3]bool) {
	for index := range states {
		changed[index] = n.baseline && !blocked && n.options.Keys[index] && states[index] != n.states[index]
	}
	n.states = states
	n.baseline = true
	return changed
}

func (n *notification) hide() {
	killTimer.Call(n.hwnd, 2)
	showWindow.Call(n.hwnd, 0)
	n.shown = time.Time{}
	n.alpha = 0
}

func (n *notification) show(index int, on bool, foreground uintptr, now time.Time) {
	// System-action countdowns take priority; do not queue stale key notices.
	name := windows.StringToUTF16Ptr("IdleTriggerActionWarning")
	warning, _, _ := findWindow.Call(uintptr(unsafe.Pointer(name)), 0)
	if visible, _, _ := isWindowVisible.Call(warning); visible != 0 {
		n.hide()
		return
	}
	n.on = on
	n.symbol = [...]string{"AA", "123", "↕"}[index]
	if index == 0 && !on {
		n.symbol = "aa"
	}
	state := "lock_keys_off"
	if on {
		state = "lock_keys_on"
	}
	n.text = fmt.Sprintf("%s %s", [...]string{"Caps Lock", "Num Lock", "Scroll Lock"}[index], i18n.T(n.language, state))
	monitor, _, _ := monitorFromWindow.Call(foreground, 2)
	info := struct {
		Size          uint32
		Monitor, Work rect
		Flags         uint32
	}{}
	info.Size = uint32(unsafe.Sizeof(info))
	if ok, _, _ := getMonitorInfo.Call(monitor, uintptr(unsafe.Pointer(&info))); ok == 0 {
		return
	}
	// Read our own DPI after moving to the destination monitor. The foreground
	// application may be DPI-unaware and report 96 even on a scaled monitor.
	currentMonitor, _, _ := monitorFromWindow.Call(n.hwnd, 2)
	if currentMonitor != monitor {
		n.hide()
		setWindowPos.Call(n.hwnd, 0, uintptr(info.Work.Left), uintptr(info.Work.Top), 0, 0, 0x15)
	}
	dpi, _, _ := getDpi.Call(n.hwnd)
	if dpi == 0 {
		dpi = 96
	}
	scale := func(v int32) int32 { return (v*int32(dpi) + 48) / 96 }
	next, err := renderSurface(uint32(dpi), theme.Current() == theme.ModeDark, on, n.language, n.symbol, n.text)
	if err != nil {
		mylog.Info("Lock notification rendering failed: %v", err)
		n.hide()
		return
	}
	wasVisible := !n.shown.IsZero()
	previousRise := riseOffset(now.Sub(n.shown), n.rise, n.animated)
	n.frame.close()
	n.frame, n.dpi = next, uint32(dpi)
	n.position = point{
		info.Work.Left + (info.Work.Right-info.Work.Left-next.size.X)/2,
		info.Work.Bottom - scale(48) - next.size.Y + next.inset,
	}
	// Keep the shadow envelope inside even unusually small working areas.
	n.position.X = max(info.Work.Left, min(n.position.X, info.Work.Right-next.size.X))
	n.position.Y = max(info.Work.Top, min(n.position.Y, info.Work.Bottom-next.size.Y-scale(4)))
	var animations int32
	systemParameters.Call(0x1042, 0, uintptr(unsafe.Pointer(&animations)), 0)
	n.animated = animations != 0
	n.initialAlpha = n.alpha
	n.rise = scale(4)
	if wasVisible {
		n.rise = previousRise
	}
	n.shown = now
	n.animate(now)
	if n.shown.IsZero() {
		return
	}
	// SW_SHOWNOACTIVATE: composition and position are already committed together.
	showWindow.Call(n.hwnd, 4)
	if id, _, _ := setTimer.Call(n.hwnd, 2, 15, 0); id == 0 {
		n.hide()
	}
}

const (
	enterDuration        = 120 * time.Millisecond
	holdDuration         = 1500 * time.Millisecond
	exitDuration         = 90 * time.Millisecond
	notificationDuration = holdDuration + exitDuration
)

func entryProgress(elapsed time.Duration) float64 {
	t := max(0.0, min(1.0, float64(elapsed)/float64(enterDuration)))
	return 1 - (1-t)*(1-t)*(1-t)
}

func opacity(elapsed time.Duration, initial byte, animated bool) byte {
	if elapsed >= notificationDuration {
		return 0
	}
	if !animated {
		return 255
	}
	if elapsed < enterDuration {
		return initial + byte(float64(255-initial)*entryProgress(elapsed))
	}
	if elapsed > holdDuration {
		t := float64(elapsed-holdDuration) / float64(exitDuration)
		return byte(255 * (1 - t*t))
	}
	return 255
}

func riseOffset(elapsed time.Duration, rise int32, animated bool) int32 {
	if !animated {
		return 0
	}
	return int32(float64(rise)*(1-entryProgress(elapsed)) + 0.5)
}

func (n *notification) animate(now time.Time) {
	elapsed := now.Sub(n.shown)
	if elapsed >= notificationDuration {
		n.hide()
		return
	}
	n.alpha = opacity(elapsed, n.initialAlpha, n.animated)
	position := n.position
	position.Y += riseOffset(elapsed, n.rise, n.animated)
	if n.frame == nil || !n.frame.present(n.hwnd, position, n.alpha) {
		mylog.Info("Lock notification composition failed")
		n.hide()
	}
}

func windowProc(hwnd uintptr, message uint32, wParam, lParam uintptr) uintptr {
	// Also reject activation requests from active-window tracking.
	if message == 0x0021 {
		return 3
	} // WM_MOUSEACTIVATE: MA_NOACTIVATE
	n := active
	if n != nil && n.hwnd == hwnd {
		switch message {
		case 0x0113:
			if wParam == 1 {
				n.poll(time.Now())
			} else if wParam == 2 && !n.shown.IsZero() {
				if n.previewOnly && !inputDesktopActive() {
					n.hide()
				} else {
					n.animate(time.Now())
				}
				if n.previewOnly && n.shown.IsZero() {
					Close()
				}
			}
			return 0
		case 0x000F:
			var ps [72]byte
			beginPaint.Call(hwnd, uintptr(unsafe.Pointer(&ps)))
			endPaint.Call(hwnd, uintptr(unsafe.Pointer(&ps)))
			return 0
		case 0x0014:
			return 1
		case 0x031A:
			if !n.shown.IsZero() {
				if next, err := renderSurface(n.dpi, theme.Current() == theme.ModeDark, n.on, n.language, n.symbol, n.text); err == nil {
					n.frame.close()
					n.frame = next
					n.animate(time.Now())
				}
			}
		case 0x001A, 0x0218, 0x007E, 0x02E0:
			n.baseline = false
			n.hide()
		case 0x0002:
			n.frame.close()
			active = nil
		}
	}
	result, _, _ := defWindowProc.Call(hwnd, uintptr(message), wParam, lParam)
	return result
}
