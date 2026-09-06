package trayicon

import (
	"golang.org/x/sys/windows"
	"slices"
	"testing"
)

func TestRejectedUITaskNeverRunsWithLaterWakeup(t *testing.T) {
	tray := &winTray{window: windows.Handle(1)}
	var ran []int
	accept := func(windows.Handle) bool { return true }
	reject := func(windows.Handle) bool {
		if tray.muUITasks.TryLock() {
			tray.muUITasks.Unlock()
			t.Error("wake-up and rollback are not protected by the queue lock")
		}
		return false
	}
	if !tray.post(func() { ran = append(ran, 1) }, accept) {
		t.Fatal("first task rejected")
	}
	if tray.post(func() { ran = append(ran, 2) }, reject) {
		t.Fatal("failed wake-up accepted")
	}
	if len(tray.uiTasks) != 1 {
		t.Fatal("failed task retained or earlier task removed")
	}
	if !tray.post(func() {
		ran = append(ran, 3)
		// A callback must be able to enqueue another callback without deadlock.
		tray.post(func() { ran = append(ran, 4) }, accept)
	}, accept) {
		t.Fatal("subsequent task rejected")
	}
	tray.drainUITasks()
	tray.drainUITasks()
	if !slices.Equal(ran, []int{1, 3, 4}) {
		t.Fatalf("executed tasks = %v", ran)
	}
	tray.beginUIShutdown()
	if tray.post(func() { t.Error("task ran after shutdown") }, func(windows.Handle) bool {
		t.Error("wake-up attempted after shutdown")
		return true
	}) {
		t.Fatal("task accepted after shutdown")
	}
}
