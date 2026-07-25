// Package singleinstance keeps one IdleTrigger process per Windows session.
package singleinstance

import (
	"fmt"
	"os"

	"golang.org/x/sys/windows"
)

// Guard owns the per-session mutex until Release is called.
type Guard struct {
	handle windows.Handle
}

// Acquire reports whether this process is the first IdleTrigger instance in
// the current Windows session.
func Acquire() (*Guard, bool, error) {
	var sessionID uint32
	if err := windows.ProcessIdToSessionId(uint32(os.Getpid()), &sessionID); err != nil {
		return nil, false, fmt.Errorf("resolve session: %w", err)
	}
	return acquireNamed(fmt.Sprintf("Local\\IdleTrigger-%d", sessionID))
}

func acquireNamed(name string) (*Guard, bool, error) {
	namePtr, err := windows.UTF16PtrFromString(name)
	if err != nil {
		return nil, false, fmt.Errorf("encode instance mutex name: %w", err)
	}
	handle, err := windows.CreateMutex(nil, false, namePtr)
	if handle == 0 {
		return nil, false, fmt.Errorf("create instance mutex: %w", err)
	}
	if err == windows.ERROR_ALREADY_EXISTS {
		_ = windows.CloseHandle(handle)
		return nil, false, nil
	}
	if err != nil {
		_ = windows.CloseHandle(handle)
		return nil, false, fmt.Errorf("create instance mutex: %w", err)
	}
	return &Guard{handle: handle}, true, nil
}

// Release closes the mutex handle. It is safe to call more than once.
func (g *Guard) Release() {
	if g == nil || g.handle == 0 {
		return
	}
	_ = windows.CloseHandle(g.handle)
	g.handle = 0
}
