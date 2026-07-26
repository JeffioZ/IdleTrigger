package hotkey

import (
	"testing"
	"time"
)

func TestDefaultBindings_Count(t *testing.T) {
	b := DefaultBindings()
	if len(b) != 3 {
		t.Fatalf("expected 3 default bindings, got %d", len(b))
	}
}

func TestDefaultBindings_UniqueLabels(t *testing.T) {
	b := DefaultBindings()
	seen := map[string]bool{}
	for _, x := range b {
		if seen[x.Label] {
			t.Errorf("duplicate label: %s", x.Label)
		}
		seen[x.Label] = true
	}
}

func TestDefaultBindings_AvoidReservedSleepShortcutAndSuppressRepeats(t *testing.T) {
	bindings := DefaultBindings()
	if got, want := bindings[0].Label, "Ctrl+Win+Shift+S"; got != want {
		t.Fatalf("sleep shortcut = %q, want %q", got, want)
	}
	if bindings[0].VK != 'S' || bindings[0].Modifier&modCtrl == 0 {
		t.Fatalf("sleep binding = %+v, want Ctrl+Win+Shift+S", bindings[0])
	}
	for _, binding := range bindings {
		if binding.Modifier&modNoRepeat == 0 {
			t.Errorf("%s does not include MOD_NOREPEAT", binding.Label)
		}
	}
}

func TestNewManager_NoPanic(t *testing.T) {
	m := NewManager(DefaultBindings(), Callbacks{})
	m.Stop()
}

func TestManager_ThreadHotkeyDispatchAndStop(t *testing.T) {
	called := make(chan struct{}, 1)
	m := NewManager([]Binding{{
		VK:       0x87, // VK_F24: deliberately uncommon in global shortcuts.
		Modifier: modCtrl | modAlt | modShift | modNoRepeat,
		Action:   ActionSleep,
		Label:    "Ctrl+Alt+Shift+F24",
	}}, Callbacks{
		OnSleep: func() { called <- struct{}{} },
	})
	failed := m.Start()
	t.Cleanup(m.Stop)
	if len(failed) != 0 {
		t.Fatalf("register hotkey failed: %v", failed)
	}

	m.mu.Lock()
	threadID := m.threadID
	m.mu.Unlock()
	if threadID == 0 {
		t.Fatal("hotkey thread ID was not published")
	}
	if posted, _, err := pPostThreadMessage.Call(uintptr(threadID), uintptr(wmHotkey), 0, 0); posted == 0 {
		t.Fatalf("post WM_HOTKEY: %v", err)
	}
	select {
	case <-called:
	case <-time.After(2 * time.Second):
		t.Fatal("hotkey callback was not dispatched")
	}
}

func TestFailed_Empty(t *testing.T) {
	var f Failed
	if len(f) != 0 {
		t.Fatal("empty Failed should have len 0")
	}
}
