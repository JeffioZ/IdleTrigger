//go:build devtools

// Package screenshot hosts deterministic, side-effect-free captures of the
// real popup window. It never starts the tray or reads application config.
package screenshot

import (
	"bytes"
	"fmt"
	"image"
	"image/color"
	"image/png"
	"io"
	"math"
	"os"
	"path/filepath"
	"strings"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/automation"
	"github.com/JeffioZ/idletrigger/internal/i18n"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/darkmode"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/dpi"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/processcatalog"
	"github.com/JeffioZ/idletrigger/internal/ui/automationpanel"
	"github.com/JeffioZ/idletrigger/internal/ui/controlpanel"
	"github.com/JeffioZ/idletrigger/internal/ui/processpicker"
	"github.com/JeffioZ/idletrigger/internal/ui/settingspanel"
)

var pngSignature = []byte{0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a}

// Keep README images independent of the workstation or CI runner DPI while
// retaining the current 150% capture quality.
const readmeCaptureScale = 1.5

const (
	screenshotFrameInset    = 18
	screenshotCornerRadius  = 14
	screenshotShadowOffset  = 2
	screenshotCornerSamples = 4
)

type screenshotShadow struct {
	offset, blur, alpha int
}

var screenshotShadows = [...]screenshotShadow{
	// A tight contact shadow gives the panel a resting point; the wider layer
	// supplies depth without leaving a visible dark halo.
	{offset: 1, blur: 3, alpha: 9},
	{offset: screenshotShadowOffset, blur: 9, alpha: 8},
}

type options struct {
	captureSet string
	surface    string
	language   string
	theme      controlpanel.Theme
	output     string
}

type job struct {
	surface  string
	language string
	theme    controlpanel.Theme
	scale    float64
	path     string
}

type rect struct{ Left, Top, Right, Bottom int32 }
type point struct{ X, Y int32 }

type bitmapInfoHeader struct {
	Size          uint32
	Width, Height int32
	Planes, Bits  uint16
	Compression   uint32
	SizeImage     uint32
	XPelsPerMeter int32
	YPelsPerMeter int32
	ClrUsed       uint32
	ClrImportant  uint32
}

type bitmapInfo struct {
	Header bitmapInfoHeader
	Colors [1]uint32
}

var (
	user32 = windows.NewLazySystemDLL("user32.dll")
	gdi32  = windows.NewLazySystemDLL("gdi32.dll")
	dwmapi = windows.NewLazySystemDLL("dwmapi.dll")

	pGetWindowRect      = user32.NewProc("GetWindowRect")
	pGetClientRect      = user32.NewProc("GetClientRect")
	pClientToScreen     = user32.NewProc("ClientToScreen")
	pPrintWindow        = user32.NewProc("PrintWindow")
	pRedrawWindow       = user32.NewProc("RedrawWindow")
	pSendMessage        = user32.NewProc("SendMessageW")
	pDwmFlush           = dwmapi.NewProc("DwmFlush")
	pCreateCompatibleDC = gdi32.NewProc("CreateCompatibleDC")
	pDeleteDC           = gdi32.NewProc("DeleteDC")
	pCreateDIBSection   = gdi32.NewProc("CreateDIBSection")
	pSelectObject       = gdi32.NewProc("SelectObject")
	pDeleteObject       = gdi32.NewProc("DeleteObject")
)

// IsCommand reports whether args select the isolated screenshot execution path.
func IsCommand(args []string) bool { return len(args) > 0 && args[0] == "screenshot" }

// Run captures deterministic UI reference images without starting the normal app.
func Run(args []string) error {
	if len(args) == 2 && args[1] == "--help" {
		fmt.Fprintln(os.Stdout, usage())
		return nil
	}
	opts, err := parse(args)
	if err != nil {
		return err
	}
	dpi.Enable()
	run := func() error {
		jobs, err := opts.jobs()
		if err != nil {
			return err
		}
		capturedSizes := make(map[string]image.Point)
		for _, job := range jobs {
			darkmode.SetPreferredAppMode(job.theme == controlpanel.ThemeDark)
			captureWindow := func(hwnd windows.Handle) error {
				popup := strings.HasPrefix(job.surface, "popup-")
				client, err := captureBestClientFrame(hwnd, popup)
				if err != nil {
					return err
				}
				if !imageHasVisualVariation(client) {
					return fmt.Errorf("capture %s contains no rendered content", job.surface)
				}
				size := client.Bounds().Size()
				key := fmt.Sprintf("%s/%s/%.2f", job.surface, job.language, job.scale)
				if previous, ok := capturedSizes[key]; ok && size != previous {
					return fmt.Errorf("inconsistent %s client capture size %dx%d; expected %dx%d", job.language, size.X, size.Y, previous.X, previous.Y)
				}
				capturedSizes[key] = size
				if err := writePNG(job.path, framePanelScreenshot(client, job.theme)); err != nil {
					return err
				}
				return nil
			}
			text := func(key string) string { return i18n.T(job.language, key) }
			captureScale := job.scale
			if captureScale <= 0 {
				captureScale = readmeCaptureScale
			}
			var err error
			switch job.surface {
			case "automation":
				state := fixedAutomationSnapshot(job.language)
				err = automationpanel.Capture(state, text, captureScale, job.theme == controlpanel.ThemeDark, false, captureWindow)
			case "automation-editor":
				state := fixedAutomationEditorSnapshot(job.language)
				err = automationpanel.Capture(state, text, captureScale, job.theme == controlpanel.ThemeDark, true, captureWindow)
			case "process-picker":
				err = processpicker.Capture(fixedProcessPickerOptions(job.language, text), fixedProcessGroups(), captureScale, job.theme == controlpanel.ThemeDark, captureWindow)
			case "settings", "settings-theme", "settings-app":
				page := map[string]int{"settings": 0, "settings-theme": 1, "settings-app": 2}[job.surface]
				err = settingspanel.CapturePage(fixedSettingsSnapshot(job.language), text, captureScale, job.theme == controlpanel.ThemeDark, page, captureWindow)
			default:
				err = controlpanel.Capture(fixedSnapshot(job.language, job.theme), text, captureScale, func(hwnd windows.Handle) error {
					if strings.HasPrefix(job.surface, "popup-") {
						popup, popupErr := controlpanel.OpenCapturePopup(hwnd, job.surface)
						if popupErr != nil {
							return popupErr
						}
						return captureWindow(popup)
					}
					return captureWindow(hwnd)
				})
			}
			if err != nil {
				return fmt.Errorf("capture %s: %w", filepath.Base(job.path), err)
			}
		}
		return nil
	}
	return run()
}

// captureBestClientFrame makes capture resilient to the first compositor
// frame arriving before native child controls have painted. Each attempt asks
// the complete child tree to paint synchronously; the frame containing the
// most real UI detail wins instead of whichever frame happened to arrive first.
func captureBestClientFrame(hwnd windows.Handle, clientOnly bool) (*image.NRGBA, error) {
	const (
		attempts       = 3
		rdwInvalidate  = 0x0001
		rdwFrame       = 0x0400
		rdwUpdateNow   = 0x0100
		rdwAllChildren = 0x0080
	)
	var best *image.NRGBA
	bestScore := -1
	var lastErr error
	for attempt := 0; attempt < attempts; attempt++ {
		pRedrawWindow.Call(uintptr(hwnd), 0, 0, rdwInvalidate|rdwFrame|rdwUpdateNow|rdwAllChildren)
		pDwmFlush.Call()
		var frame *image.NRGBA
		if clientOnly {
			frame, lastErr = printWindowClient(hwnd)
		} else {
			var window *image.NRGBA
			window, lastErr = printWindow(hwnd)
			if lastErr == nil {
				frame, lastErr = clientCrop(hwnd, window)
			}
		}
		if lastErr != nil {
			continue
		}
		if score := visualDetailScore(frame); score > bestScore {
			best, bestScore = frame, score
		}
	}
	if best == nil {
		return nil, lastErr
	}
	return best, nil
}

func visualDetailScore(img *image.NRGBA) int {
	if img == nil || img.Bounds().Empty() {
		return 0
	}
	score := 0
	bounds := img.Bounds()
	for y := bounds.Min.Y; y < bounds.Max.Y; y++ {
		for x := bounds.Min.X; x < bounds.Max.X; x++ {
			current := img.NRGBAAt(x, y)
			if x > bounds.Min.X && colorDistance(current, img.NRGBAAt(x-1, y)) >= 18 {
				score++
			}
			if y > bounds.Min.Y && colorDistance(current, img.NRGBAAt(x, y-1)) >= 18 {
				score++
			}
		}
	}
	return score
}

func colorDistance(a, b color.NRGBA) int {
	return absInt(int(a.R)-int(b.R)) + absInt(int(a.G)-int(b.G)) + absInt(int(a.B)-int(b.B))
}

func absInt(value int) int {
	if value < 0 {
		return -value
	}
	return value
}

// framePanelScreenshot gives README captures a transparent rounded frame and
// a deliberately quiet shadow. This presentation step applies only to image
// output; it never affects the live tray panel's shape or rendering.
func framePanelScreenshot(panel *image.NRGBA, theme controlpanel.Theme) *image.NRGBA {
	width, height := panel.Bounds().Dx(), panel.Bounds().Dy()
	frame := image.NewNRGBA(image.Rect(0, 0, width+2*screenshotFrameInset, height+2*screenshotFrameInset))

	for y := 0; y < frame.Bounds().Dy(); y++ {
		for x := 0; x < frame.Bounds().Dx(); x++ {
			for _, shadow := range screenshotShadows {
				distance := roundedRectDistance(x-screenshotFrameInset, y-screenshotFrameInset-shadow.offset, width, height, screenshotCornerRadius)
				if distance < 0 {
					distance = 0
				}
				alpha := int(math.Round(float64(shadow.alpha) * math.Exp(-(distance*distance)/(2*float64(shadow.blur*shadow.blur)))))
				if alpha > 0 {
					frame.SetNRGBA(x, y, overNRGBA(frame.NRGBAAt(x, y), colorNRGBA(0, 0, 0, uint8(alpha))))
				}
			}
		}
	}
	// The one-pixel, theme-aware outline does the separation work on similarly
	// colored documentation backgrounds. It lets the shadows stay restrained.
	outline := screenshotOutline(theme)
	for y := 0; y < height+2; y++ {
		for x := 0; x < width+2; x++ {
			coverage := roundedRectCoverage(x, y, width+2, height+2, screenshotCornerRadius+1)
			if coverage == 0 {
				continue
			}
			outlinePixel := outline
			outlinePixel.A = uint8(math.Round(float64(outlinePixel.A) * coverage))
			frameX, frameY := x+screenshotFrameInset-1, y+screenshotFrameInset-1
			frame.SetNRGBA(frameX, frameY, overNRGBA(frame.NRGBAAt(frameX, frameY), outlinePixel))
		}
	}

	for y := 0; y < height; y++ {
		for x := 0; x < width; x++ {
			coverage := roundedRectCoverage(x, y, width, height, screenshotCornerRadius)
			if coverage == 0 {
				continue
			}
			pixel := panel.NRGBAAt(panel.Bounds().Min.X+x, panel.Bounds().Min.Y+y)
			pixel.A = uint8(math.Round(float64(pixel.A) * coverage))
			frameX, frameY := x+screenshotFrameInset, y+screenshotFrameInset
			frame.SetNRGBA(frameX, frameY, overNRGBA(frame.NRGBAAt(frameX, frameY), pixel))
		}
	}
	return frame
}

func screenshotOutline(theme controlpanel.Theme) color.NRGBA {
	if theme == controlpanel.ThemeDark {
		return colorNRGBA(143, 153, 165, 64)
	}
	return colorNRGBA(125, 133, 144, 48)
}

// overNRGBA applies source-over compositing for straight-alpha NRGBA pixels.
// Partial rounded-edge pixels must blend with the frame beneath them instead
// of replacing it, otherwise the curve reads as a cut-out above the shadow.
func overNRGBA(dst, src color.NRGBA) color.NRGBA {
	sa, da := float64(src.A)/255, float64(dst.A)/255
	oa := sa + da*(1-sa)
	if oa == 0 {
		return color.NRGBA{}
	}
	return color.NRGBA{
		R: uint8(math.Round((float64(src.R)*sa + float64(dst.R)*da*(1-sa)) / oa)),
		G: uint8(math.Round((float64(src.G)*sa + float64(dst.G)*da*(1-sa)) / oa)),
		B: uint8(math.Round((float64(src.B)*sa + float64(dst.B)*da*(1-sa)) / oa)),
		A: uint8(math.Round(oa * 255)),
	}
}

// roundedRectCoverage uses a tiny fixed supersample grid so the transparent
// outer curve remains smooth even after GitHub scales a screenshot down.
func roundedRectCoverage(x, y, width, height, radius int) float64 {
	covered := 0
	for sampleY := 0; sampleY < screenshotCornerSamples; sampleY++ {
		for sampleX := 0; sampleX < screenshotCornerSamples; sampleX++ {
			px := float64(x) + (float64(sampleX)+0.5)/screenshotCornerSamples
			py := float64(y) + (float64(sampleY)+0.5)/screenshotCornerSamples
			if insideRoundedRectPoint(px, py, width, height, radius) {
				covered++
			}
		}
	}
	return float64(covered) / float64(screenshotCornerSamples*screenshotCornerSamples)
}

func insideRoundedRectPoint(px, py float64, width, height, radius int) bool {
	if width <= 0 || height <= 0 {
		return false
	}
	if px < 0 || py < 0 || px > float64(width) || py > float64(height) {
		return false
	}
	r := float64(radius)
	if limit := float64(width) / 2; r > limit {
		r = limit
	}
	if limit := float64(height) / 2; r > limit {
		r = limit
	}
	cx := math.Max(r, math.Min(float64(width)-r, px))
	cy := math.Max(r, math.Min(float64(height)-r, py))
	dx, dy := px-cx, py-cy
	return dx*dx+dy*dy <= r*r
}

func roundedRectDistance(x, y, width, height, radius int) float64 {
	px, py := float64(x)+0.5, float64(y)+0.5
	if insideRoundedRectPoint(px, py, width, height, radius) {
		return -1
	}
	r := float64(radius)
	if limit := float64(width) / 2; r > limit {
		r = limit
	}
	if limit := float64(height) / 2; r > limit {
		r = limit
	}
	cx := math.Max(r, math.Min(float64(width)-r, px))
	cy := math.Max(r, math.Min(float64(height)-r, py))
	return math.Hypot(px-cx, py-cy) - r
}

func colorNRGBA(r, g, b, a uint8) color.NRGBA { return color.NRGBA{R: r, G: g, B: b, A: a} }

func parse(args []string) (options, error) {
	if !IsCommand(args) {
		return options{}, fmt.Errorf("expected screenshot command")
	}
	var opts options
	for i := 1; i < len(args); i++ {
		switch args[i] {
		case "--readme-set", "--review-set", "--popup-review-set":
			if opts.captureSet != "" {
				return options{}, fmt.Errorf("screenshot capture set specified more than once")
			}
			opts.captureSet = strings.TrimSuffix(strings.TrimPrefix(args[i], "--"), "-set")
		case "--all":
			return options{}, fmt.Errorf("screenshot --all is ambiguous; use --readme-set or --review-set")
		case "--surface", "--language", "--theme", "--output":
			if i+1 >= len(args) {
				return options{}, fmt.Errorf("screenshot option %q needs a value", args[i])
			}
			value := args[i+1]
			switch args[i] {
			case "--surface":
				if opts.surface != "" {
					return options{}, fmt.Errorf("screenshot surface specified more than once")
				}
				if value != "control" && value != "automation" && value != "automation-editor" && value != "process-picker" && value != "settings" && value != "settings-theme" && value != "settings-app" && value != "popup-system" {
					return options{}, fmt.Errorf("unsupported screenshot surface %q", value)
				}
				opts.surface = value
			case "--language":
				if opts.language != "" {
					return options{}, fmt.Errorf("screenshot language specified more than once")
				}
				if value != "en" && value != "zh-CN" {
					return options{}, fmt.Errorf("screenshot language must be en or zh-CN")
				}
				opts.language = value
			case "--theme":
				if opts.theme != controlpanel.ThemeFollowSystem {
					return options{}, fmt.Errorf("screenshot theme specified more than once")
				}
				switch value {
				case "light":
					opts.theme = controlpanel.ThemeLight
				case "dark":
					opts.theme = controlpanel.ThemeDark
				default:
					return options{}, fmt.Errorf("screenshot theme must be light or dark")
				}
			case "--output":
				if opts.output != "" {
					return options{}, fmt.Errorf("screenshot output specified more than once")
				}
				opts.output = value
			}
			i++
		default:
			return options{}, fmt.Errorf("unknown screenshot option %q", args[i])
		}
	}
	if opts.captureSet != "" {
		if opts.surface != "" || opts.language != "" || opts.theme != controlpanel.ThemeFollowSystem {
			return options{}, fmt.Errorf("screenshot --%s-set cannot be combined with --surface, --language, or --theme", opts.captureSet)
		}
		if opts.output == "" {
			return options{}, fmt.Errorf("screenshot --%s-set requires --output DIRECTORY", opts.captureSet)
		}
		return opts, nil
	}
	if opts.language == "" || opts.theme == controlpanel.ThemeFollowSystem || opts.output == "" {
		return options{}, fmt.Errorf("single screenshot requires --language, --theme, and --output FILE.png")
	}
	if opts.surface == "" {
		opts.surface = "control"
	}
	if !strings.EqualFold(filepath.Ext(opts.output), ".png") {
		return options{}, fmt.Errorf("single screenshot --output must be a .png file")
	}
	return opts, nil
}

func usage() string {
	return "usage:\n  IdleTrigger.exe screenshot --readme-set --output DIRECTORY\n  IdleTrigger.exe screenshot --review-set --output DIRECTORY\n  IdleTrigger.exe screenshot --popup-review-set --output DIRECTORY\n  IdleTrigger.exe screenshot [--surface control|automation|automation-editor|process-picker|settings|settings-theme|settings-app] --language en|zh-CN --theme light|dark --output FILE.png"
}

func (opts options) jobs() ([]job, error) {
	if opts.captureSet == "" {
		return []job{{surface: opts.surface, language: opts.language, theme: opts.theme, path: opts.output}}, nil
	}
	if opts.captureSet == "readme" {
		return []job{
			{surface: "control", language: "en", theme: controlpanel.ThemeLight, path: filepath.Join(opts.output, "control-panel-en-light.png")},
			{surface: "control", language: "en", theme: controlpanel.ThemeDark, path: filepath.Join(opts.output, "control-panel-en-dark.png")},
			{surface: "control", language: "zh-CN", theme: controlpanel.ThemeLight, path: filepath.Join(opts.output, "control-panel-zh-CN-light.png")},
			{surface: "control", language: "zh-CN", theme: controlpanel.ThemeDark, path: filepath.Join(opts.output, "control-panel-zh-CN-dark.png")},
		}, nil
	}
	if opts.captureSet == "popup-review" {
		var jobs []job
		for _, scale := range []struct {
			name  string
			value float64
		}{{"100", 1}, {"150", 1.5}, {"200", 2}} {
			for _, theme := range []struct {
				value controlpanel.Theme
				name  string
			}{{controlpanel.ThemeLight, "light"}, {controlpanel.ThemeDark, "dark"}} {
				for _, language := range []string{"en", "zh-CN"} {
					name := fmt.Sprintf("popup-system-%s-%s-%s.png", language, theme.name, scale.name)
					jobs = append(jobs, job{surface: "popup-system", language: language, theme: theme.value, scale: scale.value, path: filepath.Join(opts.output, name)})
				}
			}
		}
		return jobs, nil
	}
	var jobs []job
	for _, theme := range []struct {
		value controlpanel.Theme
		name  string
	}{{controlpanel.ThemeLight, "light"}, {controlpanel.ThemeDark, "dark"}} {
		for _, language := range []string{"en", "zh-CN"} {
			for _, surface := range []string{"control", "automation", "automation-editor", "process-picker", "settings", "settings-theme", "settings-app"} {
				name := fmt.Sprintf("%s-%s-%s.png", screenshotSurfaceFilename(surface), language, theme.name)
				jobs = append(jobs, job{surface: surface, language: language, theme: theme.value, path: filepath.Join(opts.output, name)})
			}
		}
	}
	return jobs, nil
}

func screenshotSurfaceFilename(surface string) string {
	switch surface {
	case "control":
		return "control-panel"
	case "automation":
		return "automation-manager"
	default:
		return surface
	}
}

func fixedSettingsSnapshot(language string) settingspanel.State {
	return settingspanel.State{KeepScreenOn: true, NoSleepOnBattery: false, NoSleepBatteryThreshold: 20, Version: "devtools",
		IdleTimeoutMinutes: 30, IdleAction: "lock", IdleWarningSeconds: 30, IdleEnhancedMonitor: true,
		ThemeMode: "sunrise", ThemeLightTime: "07:00", ThemeDarkTime: "19:00", ThemeIPLocationEnabled: true,
		ThemeLocationStatus: fmt.Sprintf(i18n.T(language, "settings_location_ip_resolved"), "Shanghai, China"),
		ThemeDarkOnBattery:  true, ThemeSkipFullscreen: true, Language: "auto", HotkeysEnabled: true,
		AutostartEnabled: true, LoggingEnabled: false, Chinese: language == "zh-CN"}
}

func fixedSnapshot(language string, theme controlpanel.Theme) controlpanel.State {
	chinese := language == "zh-CN"
	schedule := fmt.Sprintf(i18n.T(language, "theme_schedule_sunrise_format"), "07:00", "19:00")
	schedule = fmt.Sprintf(i18n.T(language, "theme_schedule_source_format"), schedule, i18n.T(language, "theme_location_utc_offset"))
	automationSummary := fmt.Sprintf(i18n.T(language, "automation_overview_next"), 2, "2026-07-15 23:00")
	return controlpanel.State{
		NoSleepEnabled:     true,
		NoSleepStatus:      i18n.T(language, "status_enabled"),
		AutomationEnabled:  true,
		IdleEnabled:        false,
		IdleStatus:         i18n.T(language, "status_disabled"),
		AutomationCount:    2,
		AutomationSummary:  automationSummary,
		ThemeSwitchEnabled: true,
		IsChinese:          chinese,
		ThemeSchedule:      schedule,
		AppVersion:         "dev",
		Theme:              theme,
	}
}

func fixedAutomationSnapshot(language string) automationpanel.State {
	state := automationpanel.State{
		Chinese:  language == "zh-CN",
		NextText: fmt.Sprintf(i18n.T(language, "automation_next_format"), "2026-07-15 23:00"),
		Rules: []automation.Rule{
			{
				ID: "capture-awake", Name: "OBS", Enabled: true,
				Action: automation.ActionStayAwake, Trigger: automation.TriggerProcessRunning,
				ProcessLogic: automation.ProcessAny,
				Processes:    []automation.ProcessTarget{{Match: automation.MatchName, Executable: "obs64.exe"}},
			},
			{
				ID: "capture-shutdown", Name: "Night", Enabled: true,
				Action: automation.ActionShutdown, Trigger: automation.TriggerDaily, Time: "23:00",
				WarningSeconds: automation.DefaultWarningSeconds, BlockedPolicy: automation.BlockedSkip,
			},
		},
	}
	for index := 0; index < 12; index++ {
		state.Rules = append(state.Rules, automation.Rule{
			ID: fmt.Sprintf("capture-scheduled-%02d", index+1), Name: fmt.Sprintf("Task %02d", index+1), Enabled: index%3 != 0,
			Action: automation.ActionLock, Trigger: automation.TriggerDaily, Time: fmt.Sprintf("%02d:30", (index+7)%24),
			WarningSeconds: automation.DefaultWarningSeconds, BlockedPolicy: automation.BlockedSkip,
		})
	}
	return state
}

func fixedAutomationEditorSnapshot(language string) automationpanel.State {
	return automationpanel.State{
		Chinese: language == "zh-CN",
		Rules: []automation.Rule{{
			ID: "capture-weekly", Enabled: true,
			Action: automation.ActionShutdown, Trigger: automation.TriggerWeekly,
			Time: "23:00", Days: []string{"mon", "tue", "wed", "thu", "fri"},
			WarningSeconds: automation.DefaultWarningSeconds, BlockedPolicy: automation.BlockedSkip,
		}},
	}
}

func fixedProcessPickerOptions(language string, text func(string) string) processpicker.Options {
	nameTarget := automation.ProcessTarget{Match: automation.MatchName, Executable: "obs64.exe"}
	secondNameTarget := automation.ProcessTarget{Match: automation.MatchName, Executable: "Code.exe"}
	thirdNameTarget := automation.ProcessTarget{Match: automation.MatchName, Executable: "msedge.exe"}
	fourthNameTarget := automation.ProcessTarget{Match: automation.MatchName, Executable: "notepad.exe"}
	pathTarget := automation.ProcessTarget{Match: automation.MatchPath, Executable: "player.exe", Path: `C:\Apps\Player\player.exe`}
	return processpicker.Options{
		Chinese:      language == "zh-CN",
		Text:         text,
		Selected:     []automation.ProcessTarget{nameTarget, secondNameTarget, thirdNameTarget, fourthNameTarget, pathTarget},
		Descriptions: map[string]string{nameTarget.Key(): "OBS Studio", secondNameTarget.Key(): "Visual Studio Code", thirdNameTarget.Key(): "Microsoft Edge", fourthNameTarget.Key(): "Notepad", pathTarget.Key(): "Media Player"},
	}
}

func fixedProcessGroups() []processcatalog.Group {
	// Keep enough deterministic rows to exercise the themed list scrollbar in
	// visual regression captures without reading the host process table.
	return []processcatalog.Group{
		{Executable: "audiodg.exe", Description: "Windows Audio Device Graph Isolation", Count: 1},
		{Executable: "Code.exe", Description: "Visual Studio Code", Count: 4},
		{Executable: "dwm.exe", Description: "Desktop Window Manager", Count: 1},
		{Executable: "explorer.exe", Description: "Windows Explorer", Count: 1},
		{Executable: "msedge.exe", Description: "Microsoft Edge", Count: 8},
		{Executable: "notepad.exe", Description: "Notepad", Count: 1},
		{Executable: "obs64.exe", Description: "OBS Studio", Count: 2},
		{Executable: "player.exe", Description: "Media Player", Count: 3},
		{Executable: "SearchHost.exe", Description: "Search", Count: 1},
		{Executable: "ShellExperienceHost.exe", Description: "Windows Shell Experience Host", Count: 1},
		{Executable: "StartMenuExperienceHost.exe", Description: "Start", Count: 1},
		{Executable: "TextInputHost.exe", Description: "Windows Input Experience", Count: 1},
	}
}

func printWindow(hwnd windows.Handle) (*image.NRGBA, error) {
	var bounds rect
	if ok, _, err := pGetWindowRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&bounds))); ok == 0 {
		return nil, fmt.Errorf("GetWindowRect: %w", err)
	}
	width, height := bounds.Right-bounds.Left, bounds.Bottom-bounds.Top
	if width <= 0 || height <= 0 {
		return nil, fmt.Errorf("invalid capture size %dx%d", width, height)
	}
	dc, _, err := pCreateCompatibleDC.Call(0)
	if dc == 0 {
		return nil, fmt.Errorf("CreateCompatibleDC: %w", err)
	}
	defer pDeleteDC.Call(dc)
	info := bitmapInfo{Header: bitmapInfoHeader{Size: uint32(unsafe.Sizeof(bitmapInfoHeader{})), Width: width, Height: -height, Planes: 1, Bits: 32}}
	var pixels unsafe.Pointer
	bitmap, _, err := pCreateDIBSection.Call(dc, uintptr(unsafe.Pointer(&info)), 0, uintptr(unsafe.Pointer(&pixels)), 0, 0)
	if bitmap == 0 || pixels == nil {
		return nil, fmt.Errorf("CreateDIBSection: %w", err)
	}
	old, _, _ := pSelectObject.Call(dc, bitmap)
	defer func() { pSelectObject.Call(dc, old); pDeleteObject.Call(bitmap) }()
	const pwRenderFullContent = 0x00000002
	if ok, _, err := pPrintWindow.Call(uintptr(hwnd), dc, pwRenderFullContent); ok == 0 {
		return nil, fmt.Errorf("PrintWindow: %w", err)
	}
	return dibSectionImage(pixels, int(width), int(height)), nil
}

func printWindowClient(hwnd windows.Handle) (*image.NRGBA, error) {
	var bounds rect
	if ok, _, err := pGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&bounds))); ok == 0 {
		return nil, fmt.Errorf("GetClientRect: %w", err)
	}
	width, height := bounds.Right-bounds.Left, bounds.Bottom-bounds.Top
	if width <= 0 || height <= 0 {
		return nil, fmt.Errorf("invalid client capture size %dx%d", width, height)
	}
	dc, _, err := pCreateCompatibleDC.Call(0)
	if dc == 0 {
		return nil, fmt.Errorf("CreateCompatibleDC: %w", err)
	}
	defer pDeleteDC.Call(dc)
	info := bitmapInfo{Header: bitmapInfoHeader{Size: uint32(unsafe.Sizeof(bitmapInfoHeader{})), Width: width, Height: -height, Planes: 1, Bits: 32}}
	var pixels unsafe.Pointer
	bitmap, _, err := pCreateDIBSection.Call(dc, uintptr(unsafe.Pointer(&info)), 0, uintptr(unsafe.Pointer(&pixels)), 0, 0)
	if bitmap == 0 || pixels == nil {
		return nil, fmt.Errorf("CreateDIBSection: %w", err)
	}
	old, _, _ := pSelectObject.Call(dc, bitmap)
	defer func() { pSelectObject.Call(dc, old); pDeleteObject.Call(bitmap) }()
	const (
		wmPrintClient = 0x0318
		prfClient     = 0x00000004
	)
	pSendMessage.Call(uintptr(hwnd), wmPrintClient, dc, prfClient)
	return dibSectionImage(pixels, int(width), int(height)), nil
}

func dibSectionImage(pixels unsafe.Pointer, width, height int) *image.NRGBA {
	stride := width * 4
	src := unsafe.Slice((*byte)(pixels), stride*height)
	img := image.NewNRGBA(image.Rect(0, 0, width, height))
	for y := 0; y < height; y++ {
		for x := 0; x < width; x++ {
			offset := y*stride + x*4
			img.Pix[offset], img.Pix[offset+1], img.Pix[offset+2], img.Pix[offset+3] = src[offset+2], src[offset+1], src[offset], 0xff
		}
	}
	return img
}

func imageHasVisualVariation(img *image.NRGBA) bool {
	if img == nil || img.Bounds().Empty() {
		return false
	}
	first := img.NRGBAAt(img.Bounds().Min.X, img.Bounds().Min.Y)
	for y := img.Bounds().Min.Y; y < img.Bounds().Max.Y; y++ {
		for x := img.Bounds().Min.X; x < img.Bounds().Max.X; x++ {
			if img.NRGBAAt(x, y) != first {
				return true
			}
		}
	}
	return false
}

func clientCrop(hwnd windows.Handle, captured image.Image) (*image.NRGBA, error) {
	var windowRect, clientRect rect
	if ok, _, err := pGetWindowRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&windowRect))); ok == 0 {
		return nil, fmt.Errorf("GetWindowRect for client crop: %w", err)
	}
	if ok, _, err := pGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&clientRect))); ok == 0 {
		return nil, fmt.Errorf("GetClientRect for client crop: %w", err)
	}
	clientOrigin := point{}
	if ok, _, err := pClientToScreen.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&clientOrigin))); ok == 0 {
		return nil, fmt.Errorf("ClientToScreen for client crop: %w", err)
	}
	crop := image.Rect(int(clientOrigin.X-windowRect.Left), int(clientOrigin.Y-windowRect.Top), int(clientOrigin.X-windowRect.Left+clientRect.Right-clientRect.Left), int(clientOrigin.Y-windowRect.Top+clientRect.Bottom-clientRect.Top))
	return cropImage(captured, crop)
}

func cropImage(source image.Image, crop image.Rectangle) (*image.NRGBA, error) {
	if crop.Empty() || !crop.In(source.Bounds()) {
		return nil, fmt.Errorf("client crop %v is outside captured bitmap %v", crop, source.Bounds())
	}
	result := image.NewNRGBA(image.Rect(0, 0, crop.Dx(), crop.Dy()))
	for y := crop.Min.Y; y < crop.Max.Y; y++ {
		for x := crop.Min.X; x < crop.Max.X; x++ {
			result.Set(x-crop.Min.X, y-crop.Min.Y, source.At(x, y))
		}
	}
	return result, nil
}

func writePNG(path string, img image.Image) error {
	return writePNGWith(path, img, encodePNG)
}

func writePNGWith(path string, img image.Image, encode func(io.Writer, image.Image) error) error {
	directory := filepath.Dir(path)
	if err := os.MkdirAll(directory, 0o755); err != nil {
		return fmt.Errorf("create screenshot output directory: %w", err)
	}
	temporary, err := os.CreateTemp(directory, ".IdleTrigger-screenshot-*.png.tmp")
	if err != nil {
		return fmt.Errorf("create temporary screenshot PNG: %w", err)
	}
	temporaryPath := temporary.Name()
	committed := false
	defer func() {
		if !committed {
			_ = os.Remove(temporaryPath)
		}
	}()
	if err := encode(temporary, img); err != nil {
		_ = temporary.Close()
		return err
	}
	if err := temporary.Close(); err != nil {
		return fmt.Errorf("close screenshot PNG: %w", err)
	}
	if _, err := validatePNGFile(temporaryPath); err != nil {
		return fmt.Errorf("validate screenshot PNG before replace: %w", err)
	}
	if err := os.Rename(temporaryPath, path); err != nil {
		return fmt.Errorf("replace screenshot PNG: %w", err)
	}
	committed = true
	return nil
}

func encodePNG(w io.Writer, img image.Image) error {
	if err := png.Encode(w, img); err != nil {
		return fmt.Errorf("encode screenshot PNG: %w", err)
	}
	return nil
}

func validatePNGFile(path string) (image.Point, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return image.Point{}, fmt.Errorf("read PNG: %w", err)
	}
	return validatePNG(data)
}

func validatePNG(data []byte) (image.Point, error) {
	if len(data) == 0 || !bytes.HasPrefix(data, pngSignature) {
		return image.Point{}, fmt.Errorf("invalid PNG signature")
	}
	config, err := png.DecodeConfig(bytes.NewReader(data))
	if err != nil {
		return image.Point{}, fmt.Errorf("decode PNG: %w", err)
	}
	if config.Width <= 0 || config.Height <= 0 {
		return image.Point{}, fmt.Errorf("invalid PNG size %dx%d", config.Width, config.Height)
	}
	return image.Pt(config.Width, config.Height), nil
}
