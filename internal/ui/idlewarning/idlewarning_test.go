package idlewarning

import "testing"

func TestShow_NoPanic(t *testing.T)     { Show("test", "body") }
func TestHide_NoPanic(t *testing.T)     { Hide() }
func TestSetOnDismiss_Nil(t *testing.T) { SetOnDismiss(nil) }
func TestShowHideRepeat(t *testing.T) {
	for i := 0; i < 3; i++ {
		Show("t", "b")
		Hide()
	}
}

func TestHideStopsCountdownWorker(t *testing.T) {
	stop := countdown.Replace()
	Hide()
	select {
	case <-stop:
	default:
		t.Fatal("Hide left the countdown worker running")
	}
}

func TestWarningMinimumHeightIsCompact(t *testing.T) {
	if warningMinHeight >= 112 {
		t.Fatalf("minimum warning height = %d, expected a more compact card", warningMinHeight)
	}
}

func TestWarningUsesNativeCaptionAndCloseMenu(t *testing.T) {
	want := uintptr(wsPopup | wsCaption | wsSysMenu)
	if got := warningWindowStyle(); got != want {
		t.Fatalf("warningWindowStyle() = %#x, want %#x", got, want)
	}
}
