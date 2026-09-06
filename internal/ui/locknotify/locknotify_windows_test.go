package locknotify

import (
	"flag"
	"runtime"
	"strings"
	"testing"
	"time"

	"github.com/JeffioZ/idletrigger/internal/platform/windows/dpi"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

var nativeInputTest = flag.Bool("locknotify-input-test", false, "Allow native lock-key tests to temporarily toggle and restore Caps/Num/Scroll Lock")

func TestOpacity(t *testing.T) {
	for _, initial := range []byte{0, 100, 255} {
		previous := initial
		for ms := 0; ms <= int(enterDuration/time.Millisecond); ms++ {
			current := opacity(time.Duration(ms)*time.Millisecond, initial, true)
			if current < previous {
				t.Fatal("fade-in moved backwards")
			}
			previous = current
		}
		if previous != 255 {
			t.Fatal("fade-in did not reach full opacity")
		}
	}
	previous := byte(255)
	for ms := int(holdDuration / time.Millisecond); ms <= int(notificationDuration/time.Millisecond); ms++ {
		current := opacity(time.Duration(ms)*time.Millisecond, 0, true)
		if current > previous {
			t.Fatal("fade-out moved backwards")
		}
		previous = current
	}
	if previous != 0 {
		t.Fatal("fade-out did not finish")
	}
	if opacity(0, 0, false) != 255 || opacity(notificationDuration, 0, false) != 0 {
		t.Fatal("reduced motion must show immediately and still expire")
	}
}

// Explicit opt-in: ordinary go test must never synthesize user input.
func TestNativeLockKeys(t *testing.T) {
	if !*nativeInputTest {
		t.Skip("requires -locknotify-input-test on an interactive desktop")
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	dpi.Enable()
	ran := false
	trayicon.Run(func() {
		defer trayicon.Quit()
		ran = true
		Configure(Options{Enabled: true, Keys: [3]bool{true, true, true}}, "en")
		time.Sleep(150 * time.Millisecond)
		available := false
		trayicon.PostAndWait(func() { available = active != nil && active.baseline })
		if !available {
			t.Error("notification failed to establish desktop baseline")
			return
		}
		for index, key := range [...]uintptr{0x14, 0x90, 0x91} {
			var before bool
			var foreground uintptr
			trayicon.PostAndWait(func() { before = active.states[index]; foreground, _, _ = getForegroundWindow.Call() })
			func() {
				keyEvent := user32.NewProc("keybd_event")
				defer func() {
					keyEvent.Call(key, 0, 2, 0)
					var actual bool
					trayicon.PostAndWait(func() { v, _, _ := getKeyState.Call(key); actual = v&1 != 0 })
					if actual != before {
						keyEvent.Call(key, 0, 0, 0)
						keyEvent.Call(key, 0, 2, 0)
					}
					time.Sleep(150 * time.Millisecond)
				}()
				keyEvent.Call(key, 0, 0, 0)
				keyEvent.Call(key, 0, 0, 0) // Repeat while held.
				keyEvent.Call(key, 0, 2, 0)
				time.Sleep(250 * time.Millisecond)
				trayicon.PostAndWait(func() {
					if active.states[index] == before || active.shown.IsZero() {
						t.Errorf("key 0x%x: no confirmed state change", key)
					}
					want := "On"
					if before {
						want = "Off"
					}
					if !strings.HasSuffix(active.text, want) {
						t.Errorf("key 0x%x: wrong text %q", key, active.text)
					}
					if active.alpha != 255 {
						t.Errorf("key 0x%x: fade-in incomplete: %d", key, active.alpha)
					}
					current, _, _ := getForegroundWindow.Call()
					if current != foreground {
						t.Error("notification stole focus")
					}
				})
				// An unchanged state must not extend the current notification.
				var shown time.Time
				trayicon.PostAndWait(func() { shown = active.shown })
				time.Sleep(120 * time.Millisecond)
				trayicon.PostAndWait(func() {
					if active.shown != shown {
						t.Error("repeat keydown retriggered notification")
					}
				})
			}()
		}
		time.Sleep(1800 * time.Millisecond)
		trayicon.PostAndWait(func() {
			if !active.shown.IsZero() {
				t.Error("notification did not expire")
			}
		})
		Configure(Options{}, "en")
		trayicon.PostAndWait(func() {
			if active != nil {
				t.Error("disable left native resources active")
			}
		})
	}, Close)
	if !ran {
		t.Error("tray message loop failed to start")
	}
}
