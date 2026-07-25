// Package ipc provides IdleTrigger's local Windows named-pipe command channel.
package ipc

import (
	"fmt"
	"os"
	"strings"
	"syscall"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"
)

const pipeBaseName = `\\.\pipe\IdleTrigger`

const (
	pipeBufSize      = 4096
	pipeAccessDuplex = 0x00000003
	pipeTypeMessage  = 0x00000004
	pipeReadModeMsg  = 0x00000002
	pipeRejectRemote = 0x00000008
	pipeUnlimited    = 255
	pipeTimeout      = 1000
	maxConnections   = 8
	genericReadWrite = 0xC0000000
)

type Handler func(cmd string) string

func pipeName() (string, error) {
	var sessionID uint32
	if err := windows.ProcessIdToSessionId(uint32(os.Getpid()), &sessionID); err != nil {
		return "", fmt.Errorf("ProcessIdToSessionId: %w", err)
	}
	return fmt.Sprintf("%s-%d", pipeBaseName, sessionID), nil
}

// pipeSA restricts the pipe to this logon session, administrators, and SYSTEM.
func pipeSA() (*windows.SecurityAttributes, unsafe.Pointer, error) {
	advapi32 := windows.NewLazySystemDLL("advapi32.dll")
	conv := advapi32.NewProc("ConvertStringSecurityDescriptorToSecurityDescriptorW")

	logonSID, err := currentLogonSID()
	if err != nil {
		return nil, nil, err
	}
	sddl, err := syscall.UTF16PtrFromString(
		"D:(A;;GA;;;" + logonSID + ")(A;;GA;;;SY)(A;;GA;;;BA)",
	)
	if err != nil {
		return nil, nil, fmt.Errorf("security descriptor string: %w", err)
	}

	var sd unsafe.Pointer
	r, _, err := conv.Call(
		uintptr(unsafe.Pointer(sddl)),
		uintptr(1), // SDDL_REVISION_1
		uintptr(unsafe.Pointer(&sd)),
		0,
	)
	if r == 0 {
		return nil, nil, fmt.Errorf("ConvertStringSecurityDescriptorToSecurityDescriptor: %v", err)
	}

	sa := &windows.SecurityAttributes{
		Length:             uint32(unsafe.Sizeof(windows.SecurityAttributes{})),
		SecurityDescriptor: (*windows.SECURITY_DESCRIPTOR)(sd),
		InheritHandle:      0,
	}
	return sa, sd, nil
}

func currentLogonSID() (string, error) {
	var token windows.Token
	if err := windows.OpenProcessToken(windows.CurrentProcess(), windows.TOKEN_QUERY, &token); err != nil {
		return "", fmt.Errorf("OpenProcessToken: %w", err)
	}
	defer func() { _ = token.Close() }()

	groups, err := token.GetTokenGroups()
	if err != nil {
		return "", fmt.Errorf("GetTokenGroups: %w", err)
	}
	for _, group := range groups.AllGroups() {
		if group.Attributes&windows.SE_GROUP_LOGON_ID == windows.SE_GROUP_LOGON_ID {
			return group.Sid.String(), nil
		}
	}
	return "", fmt.Errorf("logon SID not found")
}

func Server(handler Handler) error {
	name, err := pipeName()
	if err != nil {
		return err
	}
	return serve(name, handler, nil)
}

func serve(name string, handler Handler, stop <-chan struct{}) error {
	sa, sd, err := pipeSA()
	if err != nil {
		return err
	}
	defer func() {
		_, _ = windows.LocalFree(windows.Handle(sd))
	}()
	pipePath, err := syscall.UTF16PtrFromString(name)
	if err != nil {
		return fmt.Errorf("pipe path: %w", err)
	}

	connections := make(chan struct{}, maxConnections)
	wakeDone := make(chan struct{})
	if stop != nil {
		go wakeServerOnStop(pipePath, stop, wakeDone)
	}
	defer close(wakeDone)

	for {
		if !claimConnectionSlot(connections, stop) {
			return nil
		}
		openMode := uint32(pipeAccessDuplex)
		pipeMode := uint32(pipeTypeMessage | pipeReadModeMsg | pipeRejectRemote)

		h, err := windows.CreateNamedPipe(
			pipePath, openMode, pipeMode,
			pipeUnlimited, pipeBufSize, pipeBufSize,
			pipeTimeout, sa,
		)
		if err != nil {
			<-connections
			return fmt.Errorf("CreateNamedPipe: %w", err)
		}

		err = windows.ConnectNamedPipe(h, nil)
		if isStopped(stop) {
			_ = windows.CloseHandle(h)
			<-connections
			return nil
		}
		if err != nil && err != windows.ERROR_PIPE_CONNECTED {
			_ = windows.CloseHandle(h)
			<-connections
			continue
		}

		go func() {
			defer func() { <-connections }()
			handleConn(h, handler)
		}()
	}
}

func claimConnectionSlot(connections chan<- struct{}, stop <-chan struct{}) bool {
	if stop == nil {
		connections <- struct{}{}
		return true
	}
	select {
	case connections <- struct{}{}:
		return true
	case <-stop:
		return false
	}
}

func isStopped(stop <-chan struct{}) bool {
	if stop == nil {
		return false
	}
	select {
	case <-stop:
		return true
	default:
		return false
	}
}

func wakeServerOnStop(pipePath *uint16, stop, done <-chan struct{}) {
	select {
	case <-stop:
	case <-done:
		return
	}

	ticker := time.NewTicker(10 * time.Millisecond)
	defer ticker.Stop()
	for {
		if h, err := openPipe(pipePath); err == nil {
			_ = windows.CloseHandle(h)
			return
		}
		select {
		case <-done:
			return
		case <-ticker.C:
		}
	}
}

func handleConn(h windows.Handle, handler Handler) {
	defer func() { _ = windows.CloseHandle(h) }()

	buf := make([]byte, pipeBufSize)
	var done uint32
	err := windows.ReadFile(h, buf, &done, nil)
	if err != nil {
		return
	}

	cmd := strings.TrimSpace(string(buf[:done]))
	resp := handler(cmd)

	out := resp + "\r\n"
	if err := windows.WriteFile(h, []byte(out), nil, nil); err != nil {
		return
	}

	_ = windows.FlushFileBuffers(h)
	_ = windows.DisconnectNamedPipe(h)
}

func Send(cmd string) (string, bool) {
	name, err := pipeName()
	if err != nil {
		return "", false
	}
	return sendTo(name, cmd)
}

func sendTo(name, cmd string) (string, bool) {
	pipePath, err := syscall.UTF16PtrFromString(name)
	if err != nil {
		return "", false
	}

	h, err := openPipe(pipePath)
	if err != nil {
		return "", false
	}
	defer func() { _ = windows.CloseHandle(h) }()

	data := []byte(cmd + "\r\n")
	var written uint32
	if err := windows.WriteFile(h, data, &written, nil); err != nil || written != uint32(len(data)) {
		return "", false
	}

	buf := make([]byte, 4096)
	var done uint32
	if err := windows.ReadFile(h, buf, &done, nil); err != nil {
		return "", false
	}

	return strings.TrimSpace(string(buf[:done])), true
}

func openPipe(pipePath *uint16) (windows.Handle, error) {
	return windows.CreateFile(
		pipePath, genericReadWrite, 0, nil,
		windows.OPEN_EXISTING, 0, 0,
	)
}
