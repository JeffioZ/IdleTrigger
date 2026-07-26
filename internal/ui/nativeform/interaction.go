package nativeform

import (
	"sync"
	"unsafe"

	"golang.org/x/sys/windows"
)

// InteractionState contains the transient states shared by the form-style
// owner-drawn controls. Semantic state (checked/selected) stays with the
// owning window.
type InteractionState struct {
	Hovered      bool
	Pressed      bool
	Focused      bool
	FocusVisible bool
}

type trackedControl struct {
	owner      *InteractionTracker
	oldProc    uintptr
	invalidate windows.Handle
	state      InteractionState
}

// InteractionTracker subclasses standard Win32 controls only to observe
// hover, press and focus changes. Native controls continue to own their input,
// keyboard and accessibility behavior.
type InteractionTracker struct {
	focusVisible bool
}

type trackMouseEvent struct {
	Size      uint32
	Flags     uint32
	Track     windows.Handle
	HoverTime uint32
}

const (
	wmSetFocus       = 0x0007
	wmKillFocus      = 0x0008
	wmEnable         = 0x000A
	wmCancelMode     = 0x001F
	wmNCDestroy      = 0x0082
	wmKeyDown        = 0x0100
	wmSysKeyDown     = 0x0104
	wmMouseMove      = 0x0200
	wmLButtonDown    = 0x0201
	wmLButtonUp      = 0x0202
	wmMouseLeave     = 0x02A3
	wmCaptureChanged = 0x0215
	tmeLeave         = 0x00000002
)

var (
	interactionUser32          = windows.NewLazySystemDLL("user32.dll")
	interactionCallWindowProc  = interactionUser32.NewProc("CallWindowProcW")
	interactionTrackMouseEvent = interactionUser32.NewProc("TrackMouseEvent")
	interactionInvalidateRect  = interactionUser32.NewProc("InvalidateRect")
	interactionCallback        = windows.NewCallback(interactionWndProc)
	interactionMu              sync.Mutex
	interactionControls        = make(map[windows.Handle]*trackedControl)
)

// Track connects a native control to the owner-drawn surface that should be
// invalidated when its interaction state changes. Pass zero to invalidate the
// control itself.
func (t *InteractionTracker) Track(control, invalidate windows.Handle) {
	if control == 0 {
		return
	}
	if invalidate == 0 {
		invalidate = control
	}
	interactionMu.Lock()
	if _, exists := interactionControls[control]; exists {
		interactionMu.Unlock()
		return
	}
	interactionMu.Unlock()
	oldProc, _, _ := SetWindowProc(control, interactionCallback)
	if oldProc == 0 {
		return
	}
	interactionMu.Lock()
	interactionControls[control] = &trackedControl{owner: t, oldProc: oldProc, invalidate: invalidate}
	interactionMu.Unlock()
}

func (t *InteractionTracker) State(control windows.Handle) InteractionState {
	interactionMu.Lock()
	defer interactionMu.Unlock()
	entry := interactionControls[control]
	if entry == nil || entry.owner != t {
		return InteractionState{}
	}
	state := entry.state
	state.FocusVisible = state.Focused && t.focusVisible
	return state
}

// SetFocusVisible switches between mouse and keyboard focus presentation.
// Native focus is always retained for input and accessibility; only the
// owner-drawn focus outline follows the current input modality.
func (t *InteractionTracker) SetFocusVisible(visible bool) {
	if t == nil {
		return
	}
	interactionMu.Lock()
	if t.focusVisible == visible {
		interactionMu.Unlock()
		return
	}
	t.focusVisible = visible
	var invalidates []windows.Handle
	for _, entry := range interactionControls {
		if entry.owner == t && entry.state.Focused && entry.invalidate != 0 {
			invalidates = append(invalidates, entry.invalidate)
		}
	}
	interactionMu.Unlock()
	for _, hwnd := range invalidates {
		interactionInvalidateRect.Call(uintptr(hwnd), 0, 0)
	}
}

// Release restores every live subclass owned by this tracker. Controls that
// have already received WM_NCDESTROY remove themselves automatically.
func (t *InteractionTracker) Release() {
	type restore struct {
		hwnd windows.Handle
		proc uintptr
	}
	var values []restore
	interactionMu.Lock()
	t.focusVisible = false
	for hwnd, entry := range interactionControls {
		if entry.owner == t {
			values = append(values, restore{hwnd: hwnd, proc: entry.oldProc})
			delete(interactionControls, hwnd)
		}
	}
	interactionMu.Unlock()
	for _, value := range values {
		SetWindowProc(value.hwnd, value.proc)
	}
}

func interactionWndProc(hwnd windows.Handle, message uint32, wParam, lParam uintptr) uintptr {
	interactionMu.Lock()
	entry := interactionControls[hwnd]
	interactionMu.Unlock()
	if entry == nil {
		return 0
	}
	changed := false
	switch message {
	case wmKeyDown, wmSysKeyDown:
		entry.owner.SetFocusVisible(true)
	case wmMouseMove:
		if !entry.state.Hovered {
			entry.state.Hovered = true
			changed = true
			track := trackMouseEvent{Size: uint32(unsafe.Sizeof(trackMouseEvent{})), Flags: tmeLeave, Track: hwnd}
			interactionTrackMouseEvent.Call(uintptr(unsafe.Pointer(&track)))
		}
	case wmMouseLeave:
		changed = entry.state.Hovered || entry.state.Pressed
		entry.state.Hovered = false
		entry.state.Pressed = false
	case wmLButtonDown:
		entry.owner.SetFocusVisible(false)
		if !entry.state.Pressed {
			entry.state.Pressed = true
			changed = true
		}
	case wmLButtonUp:
		if entry.state.Pressed {
			entry.state.Pressed = false
			changed = true
		}
	case wmSetFocus:
		if !entry.state.Focused {
			entry.state.Focused = true
			changed = true
		}
	case wmKillFocus:
		if entry.state.Focused || entry.state.Pressed {
			entry.state.Focused = false
			entry.state.Pressed = false
			changed = true
		}
	case wmCancelMode, wmCaptureChanged:
		if entry.state.Pressed {
			entry.state.Pressed = false
			changed = true
		}
	case wmEnable:
		changed = true
		if wParam == 0 {
			entry.state.Hovered = false
			entry.state.Pressed = false
		}
	}
	if changed {
		interactionInvalidateRect.Call(uintptr(entry.invalidate), 0, 0)
	}
	result, _, _ := interactionCallWindowProc.Call(entry.oldProc, uintptr(hwnd), uintptr(message), wParam, lParam)
	if message == wmNCDestroy {
		interactionMu.Lock()
		delete(interactionControls, hwnd)
		interactionMu.Unlock()
	}
	return result
}
