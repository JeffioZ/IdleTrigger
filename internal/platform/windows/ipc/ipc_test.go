package ipc

import (
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
