//go:build devtools

package screenshot

import (
	"errors"
	"image"
	"image/color"
	"io"
	"os"
	"path/filepath"
	"testing"

	"github.com/JeffioZ/idletrigger/internal/ui/controlpanel"
)

func TestParse(t *testing.T) {
	tests := []struct {
		name    string
		args    []string
		wantErr bool
	}{
		{"single", []string{"screenshot", "--language", "en", "--theme", "dark", "--output", "out.png"}, false},
		{"automation", []string{"screenshot", "--surface", "automation", "--language", "zh-CN", "--theme", "light", "--output", "out.png"}, false},
		{"automation editor", []string{"screenshot", "--surface", "automation-editor", "--language", "en", "--theme", "dark", "--output", "out.png"}, false},
		{"readme set", []string{"screenshot", "--readme-set", "--output", "images"}, false},
		{"review set", []string{"screenshot", "--review-set", "--output", "images"}, false},
		{"popup review set", []string{"screenshot", "--popup-review-set", "--output", "images"}, false},
		{"ambiguous all", []string{"screenshot", "--all", "--output", "images"}, true},
		{"missing language", []string{"screenshot", "--theme", "dark", "--output", "out.png"}, true},
		{"missing theme", []string{"screenshot", "--language", "en", "--output", "out.png"}, true},
		{"invalid language", []string{"screenshot", "--language", "fr", "--theme", "dark", "--output", "out.png"}, true},
		{"invalid theme", []string{"screenshot", "--language", "en", "--theme", "blue", "--output", "out.png"}, true},
		{"invalid surface", []string{"screenshot", "--surface", "picker", "--language", "en", "--theme", "dark", "--output", "out.png"}, true},
		{"set conflict", []string{"screenshot", "--review-set", "--language", "en", "--output", "images"}, true},
		{"set surface conflict", []string{"screenshot", "--readme-set", "--surface", "automation", "--output", "images"}, true},
		{"multiple sets", []string{"screenshot", "--readme-set", "--review-set", "--output", "images"}, true},
		{"single png", []string{"screenshot", "--language", "en", "--theme", "light", "--output", "out.jpg"}, true},
		{"set output", []string{"screenshot", "--review-set"}, true},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			_, err := parse(test.args)
			if (err != nil) != test.wantErr {
				t.Fatalf("parse() error = %v, want error %v", err, test.wantErr)
			}
		})
	}
}

func TestSingleScreenshotSurfaceDefaultsAndOverrides(t *testing.T) {
	base, err := parse([]string{"screenshot", "--language", "en", "--theme", "light", "--output", "out.png"})
	if err != nil {
		t.Fatal(err)
	}
	if base.surface != "control" {
		t.Fatalf("default surface = %q", base.surface)
	}
	automation, err := parse([]string{"screenshot", "--surface", "automation-editor", "--language", "zh-CN", "--theme", "dark", "--output", "out.png"})
	if err != nil {
		t.Fatal(err)
	}
	if automation.surface != "automation-editor" {
		t.Fatalf("explicit surface = %q", automation.surface)
	}
}

func TestAllJobsUseStableReadmeOrder(t *testing.T) {
	opts, err := parse([]string{"screenshot", "--readme-set", "--output", "docs/images"})
	if err != nil {
		t.Fatal(err)
	}
	jobs, err := opts.jobs()
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"control-panel-en-light.png", "control-panel-en-dark.png", "control-panel-zh-CN-light.png", "control-panel-zh-CN-dark.png"}
	if len(jobs) != len(want) {
		t.Fatalf("job count = %d", len(jobs))
	}
	for i, name := range want {
		if got := filepath.Base(jobs[i].path); got != name {
			t.Errorf("job %d = %q, want %q", i, got, name)
		}
	}
}

func TestReviewSetCoversEverySurfaceLanguageAndTheme(t *testing.T) {
	opts, err := parse([]string{"screenshot", "--review-set", "--output", "dist/ui-review"})
	if err != nil {
		t.Fatal(err)
	}
	jobs, err := opts.jobs()
	if err != nil {
		t.Fatal(err)
	}
	if len(jobs) != 16 {
		t.Fatalf("review job count = %d, want 16", len(jobs))
	}
	want := map[string]bool{}
	for _, theme := range []string{"light", "dark"} {
		for _, language := range []string{"en", "zh-CN"} {
			for _, surface := range []string{"control-panel", "automation-manager", "automation-editor", "process-picker"} {
				want[surface+"-"+language+"-"+theme+".png"] = true
			}
		}
	}
	for _, job := range jobs {
		name := filepath.Base(job.path)
		if !want[name] {
			t.Errorf("unexpected review job %q", name)
		}
		delete(want, name)
	}
	if len(want) != 0 {
		t.Fatalf("missing review jobs: %v", want)
	}
}

func TestPopupReviewSetCoversEveryScaleLanguageThemeAndPopup(t *testing.T) {
	opts, err := parse([]string{"screenshot", "--popup-review-set", "--output", "dist/ui-popup-review"})
	if err != nil {
		t.Fatal(err)
	}
	jobs, err := opts.jobs()
	if err != nil {
		t.Fatal(err)
	}
	if len(jobs) != 48 {
		t.Fatalf("popup review job count = %d, want 48", len(jobs))
	}
	wantScales := map[float64]bool{1: true, 1.5: true, 2: true}
	seen := make(map[string]bool)
	for _, job := range jobs {
		if !wantScales[job.scale] {
			t.Fatalf("unexpected popup review scale %.2f", job.scale)
		}
		name := filepath.Base(job.path)
		if seen[name] {
			t.Fatalf("duplicate popup review job %q", name)
		}
		seen[name] = true
	}
}

func TestPopupCaptureRejectsAnUnpaintedClient(t *testing.T) {
	blank := image.NewNRGBA(image.Rect(0, 0, 8, 8))
	if imageHasVisualVariation(blank) {
		t.Fatal("blank popup capture was accepted")
	}
	blank.SetNRGBA(4, 4, color.NRGBA{R: 1, A: 255})
	if !imageHasVisualVariation(blank) {
		t.Fatal("painted popup capture was rejected")
	}
}

func TestVisualDetailScorePrefersRenderedControls(t *testing.T) {
	blank := image.NewNRGBA(image.Rect(0, 0, 40, 24))
	for y := 0; y < 24; y++ {
		for x := 0; x < 40; x++ {
			blank.SetNRGBA(x, y, color.NRGBA{R: 32, G: 32, B: 32, A: 255})
		}
	}
	rendered := image.NewNRGBA(blank.Bounds())
	copy(rendered.Pix, blank.Pix)
	for x := 4; x < 36; x++ {
		rendered.SetNRGBA(x, 5, color.NRGBA{R: 230, G: 230, B: 230, A: 255})
		rendered.SetNRGBA(x, 18, color.NRGBA{R: 90, G: 140, B: 240, A: 255})
	}
	if got, base := visualDetailScore(rendered), visualDetailScore(blank); got <= base {
		t.Fatalf("rendered detail score = %d, blank = %d", got, base)
	}
}

func TestFixedSnapshotIsExplicit(t *testing.T) {
	state := fixedSnapshot("zh-CN", controlpanel.ThemeDark)
	if !state.IsChinese || state.Theme != controlpanel.ThemeDark || !state.NoSleepEnabled || state.IdleEnabled || !state.ThemeSwitchEnabled {
		t.Fatalf("fixture is not explicit: %#v", state)
	}
	if state.NoSleepStatus != "已启用" || state.IdleStatus != "已禁用" {
		t.Fatalf("fixture runtime status is not explicit: %#v", state)
	}
	if state.IPLocationEnabled || state.HotkeysEnabled || state.Theme == controlpanel.ThemeFollowSystem {
		t.Fatalf("fixture permits external state: %#v", state)
	}
}

func TestFixedSnapshotUsesSunriseSchedule(t *testing.T) {
	tests := []struct {
		language string
		want     string
	}{
		{"en", "Sunrise: 07:00 / Sunset: 19:00 · Estimated by timezone"},
		{"zh-CN", "日出：07:00 / 日落：19:00 · 按时区推算"},
	}
	for _, test := range tests {
		t.Run(test.language, func(t *testing.T) {
			if got := fixedSnapshot(test.language, controlpanel.ThemeLight).ThemeSchedule; got != test.want {
				t.Fatalf("ThemeSchedule = %q, want %q", got, test.want)
			}
		})
	}
}

func TestFixedAutomationSnapshotIsExplicit(t *testing.T) {
	state := fixedAutomationSnapshot("zh-CN")
	if !state.Chinese || len(state.Rules) < 12 || state.NextText == "" {
		t.Fatalf("automation fixture is not explicit: %#v", state)
	}
}

func TestFixedAutomationEditorSnapshotCoversNameCueAndWeekdays(t *testing.T) {
	state := fixedAutomationEditorSnapshot("zh-CN")
	if !state.Chinese || len(state.Rules) != 1 {
		t.Fatalf("automation editor fixture is not explicit: %#v", state)
	}
	rule := state.Rules[0]
	if rule.Name != "" || rule.Trigger != "weekly" || len(rule.Days) != 5 {
		t.Fatalf("automation editor fixture does not cover cue and weekdays: %#v", rule)
	}
}

func TestCropImage(t *testing.T) {
	source := image.NewNRGBA(image.Rect(0, 0, 10, 10))
	cropped, err := cropImage(source, image.Rect(2, 3, 8, 9))
	if err != nil {
		t.Fatal(err)
	}
	if got, want := cropped.Bounds().Size(), image.Pt(6, 6); got != want {
		t.Fatalf("size = %v, want %v", got, want)
	}
	if _, err := cropImage(source, image.Rect(-1, 0, 4, 4)); err == nil {
		t.Fatal("expected bounds error")
	}
}

func TestFramePanelScreenshotAddsRoundedCornersAndShadow(t *testing.T) {
	panel := image.NewNRGBA(image.Rect(0, 0, 40, 30))
	for y := 0; y < 30; y++ {
		for x := 0; x < 40; x++ {
			panel.SetNRGBA(x, y, color.NRGBA{R: 40, G: 80, B: 120, A: 255})
		}
	}
	framed := framePanelScreenshot(panel, controlpanel.ThemeLight)
	if got, want := framed.Bounds().Size(), image.Pt(40+2*screenshotFrameInset, 30+2*screenshotFrameInset); got != want {
		t.Fatalf("size = %v, want %v", got, want)
	}
	if got := framed.NRGBAAt(0, 0); got.A != 0 {
		t.Fatalf("outer corner = %#v, want transparent", got)
	}
	if got, want := framed.NRGBAAt(screenshotFrameInset+20, screenshotFrameInset+15), panel.NRGBAAt(20, 15); got != want {
		t.Fatalf("panel center = %#v, want %#v", got, want)
	}
	if got := framed.NRGBAAt(screenshotFrameInset+1, screenshotFrameInset+8); got.A == 0 || got.A == 255 {
		t.Fatalf("rounded edge = %#v, want antialiased alpha", got)
	}
	if got := framed.NRGBAAt(screenshotFrameInset+20, screenshotFrameInset+30+screenshotShadowOffset+2); got.A == 0 {
		t.Fatal("expected visible shadow below the panel")
	}
}

func TestFramePanelScreenshotUsesThemeAwareOutline(t *testing.T) {
	panel := image.NewNRGBA(image.Rect(0, 0, 40, 30))
	light := framePanelScreenshot(panel, controlpanel.ThemeLight)
	dark := framePanelScreenshot(panel, controlpanel.ThemeDark)
	point := image.Pt(screenshotFrameInset-1, screenshotFrameInset+15)
	lightPixel, darkPixel := light.NRGBAAt(point.X, point.Y), dark.NRGBAAt(point.X, point.Y)
	if lightPixel.A == 0 || darkPixel.A == 0 {
		t.Fatalf("outline pixels = %#v, %#v; want visible", lightPixel, darkPixel)
	}
	if lightPixel == darkPixel {
		t.Fatalf("theme outline is identical: %#v", lightPixel)
	}
}

func TestPNGValidationAndAtomicWrite(t *testing.T) {
	path := filepath.Join(t.TempDir(), "nested", "panel.png")
	first := image.NewNRGBA(image.Rect(0, 0, 3, 2))
	if err := writePNG(path, first); err != nil {
		t.Fatal(err)
	}
	second := image.NewNRGBA(image.Rect(0, 0, 4, 5))
	if err := writePNG(path, second); err != nil {
		t.Fatal(err)
	}
	size, err := validatePNGFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if size != image.Pt(4, 5) {
		t.Fatalf("size = %v", size)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := validatePNG(data); err != nil {
		t.Fatal(err)
	}
	if _, err := validatePNG([]byte("not a PNG")); err == nil {
		t.Fatal("expected invalid PNG error")
	}
}

func TestAtomicWriteRemovesTemporaryFileAfterFailure(t *testing.T) {
	directory := t.TempDir()
	path := filepath.Join(directory, "panel.png")
	err := writePNGWith(path, image.NewNRGBA(image.Rect(0, 0, 1, 1)), func(io.Writer, image.Image) error {
		return errors.New("encode failed")
	})
	if err == nil {
		t.Fatal("expected encode error")
	}
	entries, err := os.ReadDir(directory)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 0 {
		t.Fatalf("temporary files were not cleaned up: %#v", entries)
	}
}
