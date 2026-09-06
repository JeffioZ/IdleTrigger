package locknotify

import (
	"golang.org/x/sys/windows"
	"runtime"
	"testing"
	"time"

	"github.com/JeffioZ/idletrigger/internal/ui/font"
)

func TestSurfaceAlphaAndMetrics(t *testing.T) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	restore := font.OverrideTextScaleFactor(1)
	defer restore()
	for _, dpi := range []uint32{96, 120, 144, 192} {
		for _, dark := range []bool{false, true} {
			s, err := renderSurface(dpi, dark, true, "zh-CN", "AA", "Caps Lock 开")
			if err != nil {
				t.Fatal(err)
			}
			expectedWidth := int32((cardWidth + 2*shadowInset) * dpi / 96)
			if s.size.X < expectedWidth || s.size.Y < int32(cardHeight*dpi/96) {
				t.Errorf("DPI %d: clipped surface %+v", dpi, s.size)
			}
			partial := false
			for p := 0; p < len(s.pixels); p += 4 {
				a := s.pixels[p+3]
				if a > 0 && a < 255 {
					partial = true
				}
				if s.pixels[p] > a || s.pixels[p+1] > a || s.pixels[p+2] > a {
					t.Fatalf("DPI %d: non-premultiplied pixel at %d", dpi, p)
				}
			}
			center := int((s.size.Y/2*s.size.X + s.size.X/2) * 4)
			if s.pixels[center+3] != 255 {
				t.Error("card interior must be opaque")
			}
			if s.pixels[3] != 0 {
				t.Error("outer corner must be transparent")
			}
			if !partial {
				t.Error("missing smooth edge/shadow coverage")
			}
			s.close()
		}
	}
}

func TestSurfaceFitsAccessibilityText(t *testing.T) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	restore := font.OverrideTextScaleFactor(2)
	defer restore()
	s, err := renderSurface(96, false, false, "en", "123", "Scroll Lock Off")
	if err != nil {
		t.Fatal(err)
	}
	defer s.close()
	if s.size.X <= cardWidth+2*shadowInset || s.size.Y <= cardHeight+2*shadowInset {
		t.Fatal("large accessibility text did not expand its card")
	}
}

func TestRiseAndReducedMotion(t *testing.T) {
	previous := int32(8)
	for ms := 0; ms <= 120; ms++ {
		got := riseOffset(time.Duration(ms)*time.Millisecond, 8, true)
		if got > previous || got < 0 {
			t.Fatal("entrance moved backwards")
		}
		previous = got
	}
	if previous != 0 || riseOffset(0, 8, false) != 0 {
		t.Fatal("rise did not settle or ignored reduced motion")
	}
}

func TestSurfaceReleasesGDIResources(t *testing.T) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	render := func() {
		s, err := renderSurface(144, false, true, "en", "123", "Num Lock On")
		if err != nil {
			t.Fatal(err)
		}
		s.close()
	}
	render()
	resources := user32.NewProc("GetGuiResources")
	process := uintptr(windows.CurrentProcess())
	before, _, _ := resources.Call(process, 0)
	for i := 0; i < 16; i++ {
		render()
	}
	after, _, _ := resources.Call(process, 0)
	if after != before {
		t.Fatalf("GDI resources grew from %d to %d", before, after)
	}
}
