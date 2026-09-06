package settingspanel

import (
	"github.com/JeffioZ/idletrigger/internal/i18n"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"golang.org/x/sys/windows"
	"testing"
	"unsafe"
)

func TestScrollbarStaysAboveOverlappingEdit(t *testing.T) {
	if testing.Short() {
		t.Skip("native window test")
	}
	err := CapturePage(State{IdleTimeoutMinutes: 30}, func(k string) string { return i18n.T("en", k) }, 1, false, 0, func(hwnd windows.Handle) error {
		p := active
		pSetWindowPos.Call(uintptr(hwnd), 0, 0, 0, 600, 360, swpNoMove|swpNoZOrder|swpNoActivate)
		p.syncViewport()
		p.viewport.SetPosition(0, 0)
		var bar, edit struct{ Left, Top, Right, Bottom int32 }
		getRect := user32.NewProc("GetWindowRect")
		getRect.Call(uintptr(p.viewport.Windows()[1]), uintptr(unsafe.Pointer(&bar)))
		getRect.Call(uintptr(p.controls[idBatteryThreshold]), uintptr(unsafe.Pointer(&edit)))
		if max(bar.Left, edit.Left) >= min(bar.Right, edit.Right) || max(bar.Top, edit.Top) >= min(bar.Bottom, edit.Bottom) {
			t.Fatal("test did not exercise an overlapping edit")
		}
		getWindow := user32.NewProc("GetWindow")
		for h := uintptr(p.viewport.Windows()[1]); h != 0; {
			h, _, _ = getWindow.Call(h, 3) // GW_HWNDPREV: siblings above the bar
			if h == uintptr(p.controls[idBatteryThreshold]) {
				t.Fatal("edit covers the scrollbar")
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestTextScaleRefreshPreservesDraftAndRevealsFocusedControl(t *testing.T) {
	if testing.Short() {
		t.Skip("native window test")
	}
	for _, language := range []string{"en", "zh-CN"} {
		t.Run(language, func(t *testing.T) {
			state := State{Chinese: language == "zh-CN", IdleTimeoutMinutes: 30}
			err := CapturePage(state, func(key string) string { return i18n.T(language, key) }, 1.5, false, 0, func(hwnd windows.Handle) error {
				p := active
				p.setText(idIdleTimeout, "47")
				for _, factor := range []float64{2, 2.25, 1} {
					restore := font.OverrideTextScaleFactor(factor)
					pSendMessage.Call(uintptr(hwnd), wmSettingChange, 0, 0)
					restore()
					if p.textScale != factor || p.scale() != 1.5*factor {
						t.Fatalf("wrong scale: %g/%g", p.textScale, p.scale())
					}
					if p.controlText(idIdleTimeout) != "47" {
						t.Fatal("text-size change lost unsaved input")
					}
					tipFont, _, _ := pSendMessage.Call(uintptr(p.tooltip), 0x0031, 0, 0)
					if windows.Handle(tipFont) != p.font {
						t.Fatal("tooltip retained old font")
					}
					// A deliberately small client area exercises both axes independent
					// of the test workstation's monitor resolution.
					pSetWindowPos.Call(uintptr(hwnd), 0, 0, 0, 600, 360, swpNoMove|swpNoZOrder|swpNoActivate)
					p.syncViewport()
					pSendMessage.Call(uintptr(p.controls[idSave]), 0x0007, 0, 0)
					b, v := p.bounds[idSave], p.viewport
					if b.x < v.X || b.y < v.Y || b.x+b.width > v.X+v.Width || b.y+b.height > v.Y+v.Height {
						t.Fatalf("focused save button outside viewport: %+v, %+v", b, v)
					}
				}
				return nil
			})
			if err != nil {
				t.Fatal(err)
			}
		})
	}
}
