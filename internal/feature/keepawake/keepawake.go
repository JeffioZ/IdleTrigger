// Package keepawake prevents Windows from automatically activating sleep mode
// or turning off the display via the Win32 SetThreadExecutionState API.
package keepawake

import (
	"fmt"
	"runtime"
	"sync"
	"sync/atomic"

	"golang.org/x/sys/windows"
)

var (
	kernel32 = windows.NewLazySystemDLL("kernel32.dll")
	enabled  atomic.Bool
	keepScr  atomic.Bool

	pSetThreadExecutionState = kernel32.NewProc("SetThreadExecutionState")

	mu     sync.Mutex
	worker *executionWorker
)

type executionWorker struct {
	updates chan updateRequest
	stop    chan chan struct{}
}

type updateRequest struct {
	keepScreen bool
	done       chan error
}

const (
	esContinuous      = 0x80000000
	esSystemRequired  = 0x00000001
	esDisplayRequired = 0x00000002
)

// Enable activates NoSleep. Calls are serialized onto one locked OS thread,
// because Windows tracks continuous execution requests per calling thread.
func Enable(keepScreenOn bool) error {
	mu.Lock()
	defer mu.Unlock()

	var err error
	created := false
	if worker == nil {
		worker = &executionWorker{
			updates: make(chan updateRequest),
			stop:    make(chan chan struct{}),
		}
		created = true
		ready := make(chan error)
		go worker.loop(keepScreenOn, ready)
		err = <-ready
	} else {
		done := make(chan error)
		worker.updates <- updateRequest{keepScreen: keepScreenOn, done: done}
		err = <-done
	}

	if err != nil {
		if created && worker != nil {
			done := make(chan struct{})
			worker.stop <- done
			<-done
			worker = nil
		}
		keepScr.Store(false)
		enabled.Store(false)
		return err
	}
	keepScr.Store(keepScreenOn)
	enabled.Store(true)
	return nil
}

// Disable clears the request on the same OS thread that created it.
func Disable() {
	mu.Lock()
	defer mu.Unlock()

	if worker == nil {
		enabled.Store(false)
		keepScr.Store(false)
		return
	}

	done := make(chan struct{})
	worker.stop <- done
	<-done
	worker = nil
	enabled.Store(false)
	keepScr.Store(false)
}

func IsEnabled() bool { return enabled.Load() }

func IsKeepingScreenOn() bool { return keepScr.Load() }

func (w *executionWorker) loop(initialKeepScreen bool, ready chan error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	ready <- setExecutionState(initialKeepScreen)

	for {
		select {
		case req := <-w.updates:
			req.done <- setExecutionState(req.keepScreen)
		case done := <-w.stop:
			pSetThreadExecutionState.Call(uintptr(esContinuous))
			close(done)
			return
		}
	}
}

func setExecutionState(keepScreen bool) error {
	flags := uintptr(esContinuous | esSystemRequired)
	if keepScreen {
		flags |= esDisplayRequired
	}
	r, _, err := pSetThreadExecutionState.Call(flags)
	if r == 0 {
		return fmt.Errorf("SetThreadExecutionState flags=0x%x: %w", flags, err)
	}
	return nil
}
