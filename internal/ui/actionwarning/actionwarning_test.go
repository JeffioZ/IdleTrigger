package actionwarning

import (
	"strings"
	"testing"
)

func TestWarningLayoutKeepsActionsVisibleInSmallWorkAreas(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 3.375, 4.5} {
		for _, available := range [][2]int32{{1366, 680}, {800, 560}, {480, 600}} {
			width := min(int32(warningWidth*scale+0.5), available[0])
			height, controls := warningLayout(width, available[1], scale, int32(140*scale))
			if height > available[1] || controls[0].Bottom > controls[1].Top {
				t.Fatalf("body overlaps actions: scale=%g controls=%+v", scale, controls)
			}
			for _, b := range controls {
				if b.Left < 0 || b.Top < 0 || b.Right > width || b.Bottom > height || b.Right <= b.Left || b.Bottom <= b.Top {
					t.Fatalf("unreachable control at scale %g: %+v in %dx%d", scale, b, width, height)
				}
			}
			x, y := warningOrigin(rect{Left: -available[0], Top: -available[1]}, width, height, int32(16*scale))
			if x < -available[0] || y < -available[1] || x+width > 0 || y+height > 0 {
				t.Fatal("warning outside negative-coordinate work area")
			}
		}
	}
}

func TestWarningLayoutUsesSharedRhythm(t *testing.T) {
	if warningBodyX != warningPadding || warningBodyY != warningPadding || warningWidth-warningBodyX-warningBodyWidth != warningPadding {
		t.Fatal("warning body should use the shared edge inset")
	}
	if warningButtonsY-(warningBodyY+warningBodyHeight) != warningPadding {
		t.Fatal("warning body and actions should use the shared section gap")
	}
	if warningExecuteX-(warningCancelX+warningButtonWidth) != warningButtonGap {
		t.Fatal("warning actions should use the shared control gap")
	}
	if warningWidth-(warningExecuteX+warningButtonWidth) != warningPadding || warningHeight-(warningButtonsY+warningButtonHeight) != warningPadding {
		t.Fatal("warning actions should keep the shared right and bottom inset")
	}
}

func TestWarningOriginUsesTargetMonitorWorkArea(t *testing.T) {
	work := rect{Left: -1920, Top: 0, Right: 0, Bottom: 1080}
	x, y := warningOrigin(work, 600, 300, 27)
	if x != -627 || y != 753 {
		t.Fatalf("warning origin = (%d,%d), want (-627,753)", x, y)
	}
}

func TestHideStopsCountdownWorker(t *testing.T) {
	stop := countdown.Replace()
	hideNow()
	select {
	case <-stop:
	default:
		t.Fatal("hideNow left the countdown worker running")
	}
}

func TestWarningBodyEllipsizesOnlyTheRuleName(t *testing.T) {
	value := strings.Repeat("规", 12) + "\n“Lock” will run in 10 seconds."
	got := ellipsizeLeadingLine(value, 6, func(text string) (int32, bool) {
		return int32(len([]rune(text))), true
	})
	want := strings.Repeat("规", 5) + "…\n“Lock” will run in 10 seconds."
	if got != want {
		t.Fatalf("ellipsized body = %q, want %q", got, want)
	}
}

func TestWarningBodyPreservesShortRuleName(t *testing.T) {
	const value = "Night lock\n“Lock” will run in 10 seconds."
	got := ellipsizeLeadingLine(value, 20, func(text string) (int32, bool) {
		return int32(len([]rune(text))), true
	})
	if got != value {
		t.Fatalf("short warning body changed to %q", got)
	}
}

func TestWarningBodyCollapsesMultilineRuleNameAndPreservesAction(t *testing.T) {
	const action = "“Lock” will run in 10 seconds."
	const value = "Alpha\nBeta   Gamma\n" + action
	got := ellipsizeLeadingLine(value, 11, func(text string) (int32, bool) {
		return int32(len([]rune(text))), true
	})
	const want = "Alpha Beta…\n" + action
	if got != want {
		t.Fatalf("multiline warning body = %q, want %q", got, want)
	}
}
