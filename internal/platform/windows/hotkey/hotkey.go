// Package hotkey registers and dispatches IdleTrigger's global Windows hotkeys.
package hotkey

import (
	"runtime"
	"sync"
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
	Label    string // "Ctrl+Win+Shift+S"
}

type Manager struct {
	bindings  []Binding
	mu        sync.Mutex
	threadID  uint32
	readyCh   chan struct{}
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
	modAlt      = 0x0001
	modCtrl     = 0x0002
	modShift    = 0x0004
	modWin      = 0x0008
	modNoRepeat = 0x4000
	wmHotkey    = 0x0312
	wmQuit      = 0x0012
	wmUser      = 0x0400
	pmNoRemove  = 0x0000
)

var (
	user32              = windows.NewLazySystemDLL("user32.dll")
	kernel32            = windows.NewLazySystemDLL("kernel32.dll")
	pRegisterHotKey     = user32.NewProc("RegisterHotKey")
	pGetMessage         = user32.NewProc("GetMessageW")
	pPeekMessage        = user32.NewProc("PeekMessageW")
	pUnregisterHotKey   = user32.NewProc("UnregisterHotKey")
	pPostThreadMessage  = user32.NewProc("PostThreadMessageW")
	pGetCurrentThreadID = kernel32.NewProc("GetCurrentThreadId")
)

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
		{VK: 'S', Modifier: modCtrl | modWin | modShift | modNoRepeat, Action: ActionSleep, Label: "Ctrl+Win+Shift+S"},
		{VK: 'L', Modifier: modWin | modShift | modNoRepeat, Action: ActionLock, Label: "Win+Shift+L"},
		{VK: 'N', Modifier: modWin | modShift | modNoRepeat, Action: ActionToggleNoSleep, Label: "Win+Shift+N"},
	}
}

func NewManager(bindings []Binding, cbs Callbacks) *Manager {
	return &Manager{
		bindings:  bindings,
		callbacks: cbs,
	}
}

// Register calls RegisterHotKey for the current thread. It must run on the
// locked thread whose message queue is pumped by Run.
func (m *Manager) Register() Failed {
	var failed Failed
	for i, b := range m.bindings {
		r, _, _ := pRegisterHotKey.Call(0, uintptr(i), uintptr(b.Modifier), uintptr(b.VK))
		if r == 0 {
			failed = append(failed, b.Label)
		}
	}
	return failed
}

// Run enters the message pump; blocks until Stop().
func (m *Manager) Run() {
	var message msg
	for {
		// GetMessage blocks until a message arrives. Stop posts WM_QUIT to this
		// thread, which makes GetMessage return 0 and exit the loop naturally.
		r, _, _ := pGetMessage.Call(uintptr(unsafe.Pointer(&message)), 0, 0, 0)
		if r == 0 || r == ^uintptr(0) {
			return
		}
		if message.Message == wmHotkey {
			m.dispatch(int(message.WParam))
		}
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
	m.readyCh = make(chan struct{})
	m.doneCh = make(chan struct{})
	readyCh := m.readyCh
	doneCh := m.doneCh
	m.mu.Unlock()

	resultCh := make(chan Failed, 1)
	go func() {
		runtime.LockOSThread()
		defer runtime.UnlockOSThread()
		defer close(doneCh)

		// A thread does not have a message queue until it calls a User32 queue
		// function. Create it before publishing the thread ID so Stop can always
		// use PostThreadMessage safely.
		var message msg
		_, _, _ = pPeekMessage.Call(
			uintptr(unsafe.Pointer(&message)),
			0,
			uintptr(wmUser),
			uintptr(wmUser),
			uintptr(pmNoRemove),
		)
		tid, _, _ := pGetCurrentThreadID.Call()
		m.mu.Lock()
		m.threadID = uint32(tid)
		m.mu.Unlock()
		close(readyCh)

		failed := m.Register()
		resultCh <- failed

		// Run the message pump and clean up on the same thread that registered
		// the thread-scoped hotkeys.
		m.Run()
		for i := range m.bindings {
			_, _, _ = pUnregisterHotKey.Call(0, uintptr(i))
		}
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
	readyCh := m.readyCh
	doneCh := m.doneCh
	started := m.started
	m.mu.Unlock()
	if !started || readyCh == nil || doneCh == nil {
		return
	}

	<-readyCh
	m.mu.Lock()
	tid := m.threadID
	m.mu.Unlock()
	if tid == 0 {
		<-doneCh
		return
	}
	posted, _, _ := pPostThreadMessage.Call(uintptr(tid), uintptr(wmQuit), 0, 0)
	if posted == 0 {
		// Avoid deadlocking shutdown if the thread is already exiting or its
		// message queue is no longer available.
		return
	}
	<-doneCh
}

func (m *Manager) dispatch(id int) {
	if id < 0 || id >= len(m.bindings) {
		return
	}
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
