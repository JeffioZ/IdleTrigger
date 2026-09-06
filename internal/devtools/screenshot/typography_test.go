//go:build devtools

package screenshot

import (
	"flag"
	"fmt"
	"github.com/JeffioZ/idletrigger/internal/i18n"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/dpi"
	"github.com/JeffioZ/idletrigger/internal/ui/automationpanel"
	"github.com/JeffioZ/idletrigger/internal/ui/controlpanel"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"github.com/JeffioZ/idletrigger/internal/ui/processpicker"
	"github.com/JeffioZ/idletrigger/internal/ui/settingspanel"
	"golang.org/x/sys/windows"
	"path/filepath"
	"testing"
)

var typographyOutput = flag.String("typography-output", "", "optional native typography review capture directory")

func TestTypographyReview(t *testing.T) {
	if *typographyOutput == "" {
		t.Skip("opt-in native captures")
	}
	dpi.Enable()
	for _, language := range []string{"en", "zh-CN"} {
		for _, dark := range []bool{false, true} {
			for _, factor := range []float64{1, 2.25} {
				for _, surface := range []string{"control", "settings", "automation", "process-picker"} {
					t.Run(fmt.Sprintf("%s-%s-dark%t-text%.0f", surface, language, dark, factor*100), func(t *testing.T) {
						text := func(key string) string { return i18n.T(language, key) }
						theme := controlpanel.ThemeLight
						if dark {
							theme = controlpanel.ThemeDark
						}
						capture := func(hwnd windows.Handle) error {
							restore := font.OverrideTextScaleFactor(factor)
							defer restore()
							pSendMessage.Call(uintptr(hwnd), 0x001A, 0, 0)
							frame, err := captureBestClientFrame(hwnd, false)
							if err != nil {
								return err
							}
							return writePNG(filepath.Join(*typographyOutput, fmt.Sprintf("%s-%s-dark%t-text%.0f.png", surface, language, dark, factor*100)), frame)
						}
						var err error
						switch surface {
						case "control":
							err = controlpanel.Capture(fixedSnapshot(language, theme), text, 1.5, capture)
						case "settings":
							err = settingspanel.CapturePage(fixedSettingsSnapshot(language), text, 1.5, dark, 3, capture)
						case "automation":
							err = automationpanel.Capture(fixedAutomationEditorSnapshot(language), text, 1.5, dark, true, capture)
						case "process-picker":
							err = processpicker.Capture(fixedProcessPickerOptions(language, text), fixedProcessGroups(), 1.5, dark, capture)
						}
						if err != nil {
							t.Fatal(err)
						}
					})
				}
			}
		}
	}
}
