package settingspanel

import (
	"github.com/JeffioZ/idletrigger/internal/i18n"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"golang.org/x/sys/windows"
	"testing"
)

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
