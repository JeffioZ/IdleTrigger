package locknotify

import (
	"flag"
	"runtime"
	"testing"
	"time"

	"github.com/JeffioZ/idletrigger/internal/platform/windows/dpi"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

func TestSuppressedChangesNeverReplay(t *testing.T) {
	n := notification{options: Options{Keys: [3]bool{true, false, true}}}
	zero := [3]bool{}
	if got := n.updateStates(zero, false); got != zero {
		t.Fatal("initial sample notified")
	}
	states := [3]bool{true, true, false}
	if got := n.updateStates(states, true); got != zero {
		t.Fatal("fullscreen sample notified")
	}
	if got := n.updateStates(states, false); got != zero {
		t.Fatal("fullscreen exit replayed changes")
	}
	states = [3]bool{false, false, true}
	if got := n.updateStates(states, false); got != [3]bool{true, false, true} {
		t.Fatalf("key selection ignored: %v", got)
	}
	n.options.Keys[1] = true
	if got := n.updateStates(states, false); got != zero {
		t.Fatal("enabling a key replayed its state")
	}
	n.baseline = false
	if got := n.updateStates([3]bool{true, true, true}, false); got != zero {
		t.Fatal("resume replayed changes")
	}
}

var nativePreviewTest = flag.Bool("locknotify-preview-test", false, "Show a temporary sample to check focus, real key states and cleanup; never synthesize input")

func TestNativePreview(t *testing.T) {
	if !*nativePreviewTest {
		t.Skip("requires -locknotify-preview-test on an interactive desktop")
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	dpi.Enable()
	ran := false
	trayicon.Run(func() {
		defer trayicon.Quit()
		ran = true
		var foreground uintptr
		var before [3]bool
		trayicon.PostAndWait(func() {
			foreground, _, _ = getForegroundWindow.Call()
			for i, key := range [...]uintptr{0x14, 0x90, 0x91} {
				value, _, _ := getKeyState.Call(key)
				before[i] = value&1 != 0
			}
		})
		Preview("en")
		time.Sleep(250 * time.Millisecond)
		trayicon.PostAndWait(func() {
			if active == nil || !active.previewOnly || active.shown.IsZero() {
				t.Error("sample not displayed")
				return
			}
			sendMessage := user32.NewProc("SendMessageW")
			if result, _, _ := sendMessage.Call(active.hwnd, 0x0021, foreground, 0); result != 3 {
				t.Error("preview allowed mouse activation")
			}
			current, _, _ := getForegroundWindow.Call()
			if current != foreground {
				t.Error("preview changed foreground window")
			}
			for i, key := range [...]uintptr{0x14, 0x90, 0x91} {
				value, _, _ := getKeyState.Call(key)
				if (value&1 != 0) != before[i] {
					t.Error("preview changed a real key state")
				}
			}
		})
		time.Sleep(1700 * time.Millisecond)
		trayicon.PostAndWait(func() {
			if active != nil {
				t.Error("temporary preview window was not destroyed")
			}
		})
	}, Close)
	if !ran {
		t.Fatal("tray message loop did not start")
	}
}
