package ipc

import (
	"errors"
	"fmt"
	"os"
	"strings"
	"testing"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"
)

func TestPipeIdentity(t *testing.T) {
	name, err := pipeName()
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(name, pipeBaseName+"-") {
		t.Fatalf("unexpected pipe name %q", name)
	}
	if sid, err := currentLogonSID(); err != nil || !strings.HasPrefix(sid, "S-") {
		t.Fatalf("invalid logon SID %q: %v", sid, err)
	}
}

func TestPipeSecurityAttributesAreInitializedAndNonInheritable(t *testing.T) {
	sa, sd, err := pipeSA()
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if _, err := windows.LocalFree(windows.Handle(sd)); err != nil {
			t.Errorf("free security descriptor: %v", err)
		}
	}()

	if sd == nil || sa == nil || sa.SecurityDescriptor == nil {
		t.Fatal("pipe security descriptor was not initialized")
	}
	if sa.InheritHandle != 0 {
		t.Fatalf("pipe handle inheritance = %d, want disabled", sa.InheritHandle)
	}
	wantLength := uint32(unsafe.Sizeof(windows.SecurityAttributes{}))
	if sa.Length != wantLength {
		t.Fatalf("security attributes length = %d, want %d", sa.Length, wantLength)
	}
}

func TestClientPipeFlagsEnableCancellationAndLimitImpersonation(t *testing.T) {
	for name, flag := range map[string]uint32{
		"FILE_FLAG_OVERLAPPED":    windows.FILE_FLAG_OVERLAPPED,
		"SECURITY_SQOS_PRESENT":   windows.SECURITY_SQOS_PRESENT,
		"SECURITY_IDENTIFICATION": windows.SECURITY_IDENTIFICATION,
	} {
		if clientOpenFlags&flag != flag {
			t.Errorf("client pipe flags do not include %s", name)
		}
	}
}

func TestServeRejectsPrecreatedFirstPipeInstance(t *testing.T) {
	name := fmt.Sprintf(`\\.\pipe\IdleTrigger-precreated-%d-%d`, os.Getpid(), time.Now().UnixNano())
	pipePath, err := windows.UTF16PtrFromString(name)
	if err != nil {
		t.Fatal(err)
	}
	existing, err := windows.CreateNamedPipe(
		pipePath,
		pipeAccessDuplex,
		pipeTypeMessage|pipeReadModeMsg|pipeRejectRemote,
		pipeUnlimited,
		pipeBufSize,
		pipeBufSize,
		pipeTimeout,
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = windows.CloseHandle(existing) }()

	err = serve(name, func(string) string { return "" }, nil)
	if !errors.Is(err, windows.ERROR_ACCESS_DENIED) {
		t.Fatalf("serve with precreated pipe returned %v, want ERROR_ACCESS_DENIED", err)
	}
}

func TestSendTimeoutCancelsPendingRead(t *testing.T) {
	name := fmt.Sprintf(`\\.\pipe\IdleTrigger-timeout-%d-%d`, os.Getpid(), time.Now().UnixNano())
	pipePath, err := windows.UTF16PtrFromString(name)
	if err != nil {
		t.Fatal(err)
	}
	server, err := windows.CreateNamedPipe(
		pipePath,
		pipeAccessDuplex,
		pipeTypeMessage|pipeReadModeMsg|pipeRejectRemote,
		1,
		pipeBufSize,
		pipeBufSize,
		pipeTimeout,
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = windows.CloseHandle(server) }()

	connected := make(chan error, 1)
	go func() {
		err := windows.ConnectNamedPipe(server, nil)
		if errors.Is(err, windows.ERROR_PIPE_CONNECTED) {
			err = nil
		}
		connected <- err
	}()

	const timeout = 100 * time.Millisecond
	started := time.Now()
	if response, ok := sendToTimeout(name, "status", timeout); ok {
		t.Fatalf("unexpected response from non-responsive server: %q", response)
	}
	elapsed := time.Since(started)
	if elapsed < timeout/2 {
		t.Fatalf("send returned before the I/O timeout: %v", elapsed)
	}
	if elapsed > time.Second {
		t.Fatalf("cancelling timed-out I/O took too long: %v", elapsed)
	}
	select {
	case err := <-connected:
		if err != nil {
			t.Fatalf("connect test pipe: %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("test client never connected to the non-responsive server")
	}
}

func TestServePreservesResponseUntilDelayedClientRead(t *testing.T) {
	name := fmt.Sprintf(`\\.\pipe\IdleTrigger-delayed-read-%d-%d`, os.Getpid(), time.Now().UnixNano())
	stop := make(chan struct{})
	done := make(chan error, 1)
	handled := make(chan struct{}, 1)
	stopClosed := false
	serverDone := false
	var client windows.Handle
	t.Cleanup(func() {
		if client != 0 {
			_ = windows.CloseHandle(client)
		}
		if !stopClosed {
			close(stop)
		}
		if !serverDone {
			select {
			case <-done:
			case <-time.After(2 * time.Second):
				t.Error("delayed-read IPC server did not stop during cleanup")
			}
		}
	})

	go func() {
		done <- serve(name, func(cmd string) string {
			handled <- struct{}{}
			return "ack:" + cmd
		}, stop)
	}()

	pipePath, err := windows.UTF16PtrFromString(name)
	if err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for client == 0 && time.Now().Before(deadline) {
		select {
		case err := <-done:
			serverDone = true
			t.Fatalf("delayed-read IPC server stopped before accepting a request: %v", err)
		default:
		}
		client, err = openPipe(pipePath)
		if err != nil {
			client = 0
			time.Sleep(10 * time.Millisecond)
		}
	}
	if client == 0 {
		t.Fatal("delayed-read IPC server did not accept a client")
	}

	request := []byte("delayed\r\n")
	written, err := writeWithTimeout(client, request, time.Second)
	if err != nil || written != uint32(len(request)) {
		t.Fatalf("write delayed request: bytes=%d err=%v", written, err)
	}
	select {
	case <-handled:
	case <-time.After(time.Second):
		t.Fatal("delayed request was not handled")
	}

	// Ensure the server has time to write and attempt its disconnect before the
	// client starts reading. Without the bounded close wait, DisconnectNamedPipe
	// discards this buffered response.
	time.Sleep(100 * time.Millisecond)
	buffer := make([]byte, pipeBufSize)
	read, err := readWithTimeout(client, buffer, time.Second)
	if err != nil {
		t.Fatalf("read delayed response: %v", err)
	}
	if response := strings.TrimSpace(string(buffer[:read])); response != "ack:delayed" {
		t.Fatalf("delayed response = %q, want %q", response, "ack:delayed")
	}
	if err := windows.CloseHandle(client); err != nil {
		t.Fatalf("close delayed-read client: %v", err)
	}
	client = 0

	close(stop)
	stopClosed = true
	select {
	case err := <-done:
		serverDone = true
		if err != nil {
			t.Fatalf("stop delayed-read IPC server: %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("delayed-read IPC server did not stop")
	}
}

func TestServeRoundTripAndStop(t *testing.T) {
	name := fmt.Sprintf(`\\.\pipe\IdleTrigger-test-%d-%d`, os.Getpid(), time.Now().UnixNano())
	stop := make(chan struct{})
	done := make(chan error, 1)
	stopClosed := false
	serverDone := false
	t.Cleanup(func() {
		if !stopClosed {
			close(stop)
		}
		if !serverDone {
			select {
			case <-done:
			case <-time.After(2 * time.Second):
				t.Error("IPC test server did not stop during cleanup")
			}
		}
	})

	go func() {
		done <- serve(name, func(cmd string) string {
			return "ack:" + cmd
		}, stop)
	}()

	deadline := time.Now().Add(2 * time.Second)
	for attempt := 0; attempt < 4; attempt++ {
		var connected bool
		for !connected && time.Now().Before(deadline) {
			pipePath, pathErr := windows.UTF16PtrFromString(name)
			if pathErr != nil {
				t.Fatal(pathErr)
			}
			if h, openErr := openPipe(pipePath); openErr == nil {
				_ = windows.CloseHandle(h)
				connected = true
			} else {
				time.Sleep(10 * time.Millisecond)
			}
		}
		if !connected {
			t.Fatal("IPC test server did not survive early client disconnects")
		}
	}

	var response string
	var ok bool
	for !ok && time.Now().Before(deadline) {
		select {
		case err := <-done:
			serverDone = true
			t.Fatalf("IPC test server stopped before accepting a request: %v", err)
		default:
		}
		response, ok = sendTo(name, "  status  ")
		if !ok {
			time.Sleep(10 * time.Millisecond)
		}
	}
	if !ok {
		t.Fatal("IPC test server did not accept a request")
	}
	if response != "ack:status" {
		t.Fatalf("IPC response = %q, want %q", response, "ack:status")
	}
	for attempt := 0; attempt < 64; attempt++ {
		command := fmt.Sprintf("status-%d", attempt)
		var response string
		var ok bool
		attemptDeadline := time.Now().Add(time.Second)
		for !ok && time.Now().Before(attemptDeadline) {
			response, ok = sendTo(name, command)
			if !ok {
				time.Sleep(time.Millisecond)
			}
		}
		if !ok {
			t.Fatalf("stress round trip %d failed", attempt)
		}
		if want := "ack:" + command; response != want {
			t.Fatalf("stress response %d = %q, want %q", attempt, response, want)
		}
	}

	close(stop)
	stopClosed = true
	select {
	case err := <-done:
		serverDone = true
		if err != nil {
			t.Fatalf("stop IPC test server: %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("IPC test server did not stop")
	}
}
