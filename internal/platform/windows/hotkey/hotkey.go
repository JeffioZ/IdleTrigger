// Package hotkey registers and dispatches IdleTrigger's global Windows hotkeys.
package hotkey

import (
	"runtime"
	"sync"
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

type Action int

const (
	ActionSleep Action = iota
	ActionLock
	ActionToggleNoSleep
)

type Binding struct {
	VK       uint32
	Modifier uint32
	Action   Action
	Label    string // "Win+Shift+S"
}

type Manager struct {
	bindings  []Binding
	hwnd      windows.Handle
	mu        sync.Mutex
	threadID  uint32
	doneCh    chan struct{}
	started   bool
	callbacks Callbacks
}

type Callbacks struct {
	OnSleep         func()
	OnLock          func()
	OnToggleNoSleep func()
}

type Failed []string

const (
	modAlt    = 0x0001
	modCtrl   = 0x0002
	modShift  = 0x0004
	modWin    = 0x0008
	wmHotkey  = 0x0312
	className = "IdleTriggerHotkey"
)

var (
	user32   = windows.NewLazySystemDLL("user32.dll")
	kernel32 = windows.NewLazySystemDLL("kernel32.dll")

	pRegisterClassEx    = user32.NewProc("RegisterClassExW")
	pCreateWindowEx     = user32.NewProc("CreateWindowExW")
	pRegisterHotKey     = user32.NewProc("RegisterHotKey")
	pGetMessage         = user32.NewProc("GetMessageW")
	pDispatchMessage    = user32.NewProc("DispatchMessageW")
	pUnregisterHotKey   = user32.NewProc("UnregisterHotKey")
	pDestroyWindow      = user32.NewProc("DestroyWindow")
	pUnregisterClass    = user32.NewProc("UnregisterClassW")
	pPostThreadMessage  = user32.NewProc("PostThreadMessageW")
	pDefWindowProc      = user32.NewProc("DefWindowProcW")
	pGetModuleHandle    = kernel32.NewProc("GetModuleHandleW")
	pGetCurrentThreadID = kernel32.NewProc("GetCurrentThreadId")
)

type wndClassExW struct {
	Size       uint32
	Style      uint32
	WndProc    uintptr
	ClsExtra   int32
	WndExtra   int32
	Instance   windows.Handle
	Icon       windows.Handle
	Cursor     windows.Handle
	Background windows.Handle
	MenuName   *uint16
	ClassName  *uint16
	IconSm     windows.Handle
}

type msg struct {
	Hwnd    windows.Handle
	Message uint32
	WParam  uintptr
	LParam  uintptr
	Time    uint32
	Pt      struct{ X, Y int32 }
}

func DefaultBindings() []Binding {
	return []Binding{
		{VK: 'S', Modifier: modWin | modShift, Action: ActionSleep, Label: "Win+Shift+S"},
		{VK: 'L', Modifier: modWin | modShift, Action: ActionLock, Label: "Win+Shift+L"},
		{VK: 'N', Modifier: modWin | modShift, Action: ActionToggleNoSleep, Label: "Win+Shift+N"},
	}
}

func NewManager(bindings []Binding, cbs Callbacks) *Manager {
	return &Manager{
		bindings:  bindings,
		callbacks: cbs,
	}
}

// Register creates the message-only window and calls RegisterHotKey for each
// binding.  Returns labels for any that failed (conflict with another app).
func (m *Manager) Register() Failed {
	classNamePtr, err := syscall.UTF16PtrFromString(className)
	if err != nil {
		return failedBindings(m.bindings)
	}
	hInst, _, _ := pGetModuleHandle.Call(0)
	if hInst == 0 {
		return failedBindings(m.bindings)
	}

	var wc wndClassExW
	wc.Size = uint32(unsafe.Sizeof(wc))
	wc.WndProc = windows.NewCallback(m.wndProc)
	wc.Instance = windows.Handle(hInst)
	wc.ClassName = classNamePtr

	if registered, _, callErr := pRegisterClassEx.Call(uintptr(unsafe.Pointer(&wc))); registered == 0 && callErr != windows.ERROR_CLASS_ALREADY_EXISTS {
		return failedBindings(m.bindings)
	}

	const wsOverlapped = 0
	const hwndMessage = ^uintptr(0)
	ret, _, _ := pCreateWindowEx.Call(
		0, uintptr(unsafe.Pointer(classNamePtr)), 0,
		uintptr(wsOverlapped), 0, 0, 0, 0,
		hwndMessage, 0, hInst, 0,
	)
	m.hwnd = windows.Handle(ret)
	if m.hwnd == 0 {
		return failedBindings(m.bindings)
	}

	var failed Failed
	for i, b := range m.bindings {
		r, _, _ := pRegisterHotKey.Call(uintptr(m.hwnd), uintptr(i), uintptr(b.Modifier), uintptr(b.VK))
		if r == 0 {
			failed = append(failed, b.Label)
		}
	}
	return failed
}

// Run enters the message pump; blocks until Stop().
func (m *Manager) Run() {
	msg2 := &msg{}
	for {
		// GetMessage blocks until a message arrives. Stop posts WM_QUIT to this
		// thread, which makes GetMessage return 0 and exit the loop naturally.
		r, _, _ := pGetMessage.Call(uintptr(unsafe.Pointer(msg2)), 0, 0, 0)
		if r == 0 || r == ^uintptr(0) {
			return
		}
		_, _, _ = pDispatchMessage.Call(uintptr(unsafe.Pointer(msg2)))
	}
}

// Start launches the hotkey goroutine, which locks the OS thread and does
// Register + message loop + cleanup all on the same thread. Returns the
// Failed list from registration.
func (m *Manager) Start() Failed {
	m.mu.Lock()
	if m.started {
		m.mu.Unlock()
		return nil
	}
	m.started = true
	m.doneCh = make(chan struct{})
	doneCh := m.doneCh
	m.mu.Unlock()

	resultCh := make(chan Failed, 1)
	go func() {
		runtime.LockOSThread()
		defer runtime.UnlockOSThread()
		defer close(doneCh)

		tid, _, _ := pGetCurrentThreadID.Call()
		m.mu.Lock()
		m.threadID = uint32(tid)
		m.mu.Unlock()

		failed := m.Register()
		resultCh <- failed

		// Run message pump on this same thread, then clean up.
		if m.hwnd != 0 {
			m.Run()

			// Unregister hotkeys and destroy window on this thread.
			for i := range m.bindings {
				_, _, _ = pUnregisterHotKey.Call(uintptr(m.hwnd), uintptr(i))
			}
			_, _, _ = pDestroyWindow.Call(uintptr(m.hwnd))
			m.hwnd = 0
		}
		classNamePtr, _ := syscall.UTF16PtrFromString(className)
		hInst, _, _ := pGetModuleHandle.Call(0)
		_, _, _ = pUnregisterClass.Call(uintptr(unsafe.Pointer(classNamePtr)), hInst)
		m.mu.Lock()
		m.threadID = 0
		m.started = false
		m.mu.Unlock()
	}()
	return <-resultCh
}

// Stop signals the hotkey goroutine to exit. Cleanup is handled inside
// Start()'s goroutine on the same OS thread.
func (m *Manager) Stop() {
	m.mu.Lock()
	tid := m.threadID
	doneCh := m.doneCh
	started := m.started
	m.mu.Unlock()
	if !started || doneCh == nil {
		return
	}
	if tid == 0 {
		return
	}
	const wmQuit = 0x0012
	posted, _, _ := pPostThreadMessage.Call(uintptr(tid), uintptr(wmQuit), 0, 0)
	if posted == 0 {
		// Avoid deadlocking shutdown if the thread is already exiting or its
		// message queue is no longer available.
		return
	}
	<-doneCh
}

func (m *Manager) wndProc(hwnd windows.Handle, msg2 uint32, wParam, lParam uintptr) uintptr {
	if msg2 == wmHotkey {
		id := int(wParam)
		if id >= 0 && id < len(m.bindings) {
			switch m.bindings[id].Action {
			case ActionSleep:
				if m.callbacks.OnSleep != nil {
					m.callbacks.OnSleep()
				}
			case ActionLock:
				if m.callbacks.OnLock != nil {
					m.callbacks.OnLock()
				}
			case ActionToggleNoSleep:
				if m.callbacks.OnToggleNoSleep != nil {
					m.callbacks.OnToggleNoSleep()
				}
			}
		}
		return 0
	}
	r, _, _ := pDefWindowProc.Call(uintptr(hwnd), uintptr(msg2), wParam, lParam)
	return r
}

func failedBindings(bindings []Binding) Failed {
	failed := make(Failed, 0, len(bindings))
	for _, binding := range bindings {
		failed = append(failed, binding.Label)
	}
	return failed
}
