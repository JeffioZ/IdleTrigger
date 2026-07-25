package hotkey

import "testing"

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

func TestNewManager_NoPanic(t *testing.T) {
	m := NewManager(DefaultBindings(), Callbacks{})
	m.Stop()
}

func TestManager_RegisterThenStop(t *testing.T) {
	m := NewManager(DefaultBindings(), Callbacks{})
	failed := m.Start()
	_ = failed
	m.Stop()
}

func TestFailed_Empty(t *testing.T) {
	var f Failed
	if len(f) != 0 {
		t.Fatal("empty Failed should have len 0")
	}
}
