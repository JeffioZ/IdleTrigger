package keepawake

import "testing"

func TestEnableUpdateDisable(t *testing.T) {
	if err := Enable(false); err != nil {
		t.Fatal(err)
	}
	if !IsEnabled() || IsKeepingScreenOn() {
		t.Fatal("sleep-only request was not applied")
	}
	if err := Enable(true); err != nil {
		t.Fatal(err)
	}
	if !IsEnabled() || !IsKeepingScreenOn() {
		t.Fatal("display request was not updated")
	}
	Disable()
	if IsEnabled() || IsKeepingScreenOn() {
		t.Fatal("request was not cleared")
	}
}
