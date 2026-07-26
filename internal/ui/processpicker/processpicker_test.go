package processpicker

import (
	"errors"
	"fmt"
	"os"
	"runtime"
	"sync"
	"testing"
	"time"
	"unicode/utf16"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/automation"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/processcatalog"
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"github.com/JeffioZ/idletrigger/internal/ui/wintest"
)

func TestOwnerDrawButtonsIgnoreDoubleClickNotification(t *testing.T) {
	const bnDoubleClicked = 5
	if !ownerDrawButtonClicked(bnClicked) {
		t.Fatal("BN_CLICKED should invoke an owner-draw button command")
	}
	if ownerDrawButtonClicked(bnDoubleClicked) {
		t.Fatal("BN_DOUBLECLICKED must not invoke an owner-draw button command twice")
	}
}

var (
	pickerTestUser32         = windows.NewLazySystemDLL("user32.dll")
	pickerTestGetWindowRect  = pickerTestUser32.NewProc("GetWindowRect")
	pickerTestGetClientRect  = pickerTestUser32.NewProc("GetClientRect")
	pickerTestClientToScreen = pickerTestUser32.NewProc("ClientToScreen")
	pickerTestEnumMonitors   = pickerTestUser32.NewProc("EnumDisplayMonitors")
	pickerTestGetMonitorInfo = pickerTestUser32.NewProc("GetMonitorInfoW")
	pickerTestScrollBarInfo  = pickerTestUser32.NewProc("GetScrollBarInfo")
	pickerTestGetUpdateRect  = pickerTestUser32.NewProc("GetUpdateRect")
	pickerTestPeekMessage    = pickerTestUser32.NewProc("PeekMessageW")
	pickerTestTranslate      = pickerTestUser32.NewProc("TranslateMessage")
	pickerTestDispatch       = pickerTestUser32.NewProc("DispatchMessageW")
	pickerTestGDI32          = windows.NewLazySystemDLL("gdi32.dll")
	pickerTestCreateDIB      = pickerTestGDI32.NewProc("CreateDIBSection")
	pickerTestBitBlt         = pickerTestGDI32.NewProc("BitBlt")
	pickerTestCopyMemory     = windows.NewLazySystemDLL("kernel32.dll").NewProc("RtlMoveMemory")
	pickerTestDWMFlush       = windows.NewLazySystemDLL("dwmapi.dll").NewProc("DwmFlush")
)

type pickerTestBitmapInfoHeader struct {
	Size          uint32
	Width         int32
	Height        int32
	Planes        uint16
	BitsPerPixel  uint16
	Compression   uint32
	SizeImage     uint32
	XPelsPerMeter int32
	YPelsPerMeter int32
	ColorsUsed    uint32
	ColorsNeeded  uint32
}

type pickerTestBitmapInfo struct {
	Header pickerTestBitmapInfoHeader
	Colors [1]uint32
}

type pickerDesktopCapture struct {
	bounds rect
	pixels []byte
	stride int
}

type pickerTestMessage struct {
	Window  windows.Handle
	Message uint32
	WParam  uintptr
	LParam  uintptr
	Time    uint32
	Point   struct{ X, Y int32 }
}

func TestBuildItemsKeepsRunningListDeduplicatedByName(t *testing.T) {
	path := `C:\Apps\player.exe`
	target := automation.ProcessTarget{Match: automation.MatchPath, Executable: "player.exe", Path: path}
	selected := map[string]automation.ProcessTarget{target.Key(): target}
	groups := []processcatalog.Group{{Executable: "player.exe", Count: 2}}
	items := buildItems(groups, selected, func(key string) string { return key })
	if len(items) != 1 || items[0].target.Match != automation.MatchName || items[0].count != "2" {
		t.Fatalf("items = %+v", items)
	}
}

func TestProcessPickerAutoRefreshUsesStaleActivationOnly(t *testing.T) {
	now := time.Date(2026, 7, 16, 12, 0, 0, 0, time.Local)
	scanDuration := 20 * time.Millisecond
	refreshAge := processPickerRefreshAge(scanDuration)
	if shouldAutoRefreshProcessPicker(time.Time{}, now, scanDuration, false) {
		t.Fatal("an uninitialized picker should not start a second load")
	}
	if shouldAutoRefreshProcessPicker(now.Add(-refreshAge), now, scanDuration, true) {
		t.Fatal("an in-flight load should not be restarted")
	}
	if shouldAutoRefreshProcessPicker(now.Add(-refreshAge+time.Millisecond), now, scanDuration, false) {
		t.Fatal("a fresh snapshot was refreshed too early")
	}
	if !shouldAutoRefreshProcessPicker(now.Add(-refreshAge), now, scanDuration, false) {
		t.Fatal("a stale snapshot was not refreshed after reactivation")
	}
}

func TestProcessPickerRefreshAgeAdaptsToMeasuredScanCost(t *testing.T) {
	if got := processPickerRefreshAge(0); got != processPickerAutoRefreshMinAge {
		t.Fatalf("zero-cost refresh age = %v, want %v", got, processPickerAutoRefreshMinAge)
	}
	if got := processPickerRefreshAge(100 * time.Millisecond); got != 25*time.Second {
		t.Fatalf("100 ms refresh age = %v, want 25s", got)
	}
	if got := processPickerRefreshAge(time.Second); got != processPickerAutoRefreshMaxAge {
		t.Fatalf("slow refresh age = %v, want %v", got, processPickerAutoRefreshMaxAge)
	}
}

func TestProcessPickerStatusHoldDelay(t *testing.T) {
	now := time.Unix(100, 0)
	if got := processPickerStatusHoldDelay(now, time.Time{}); got != 0 {
		t.Fatalf("zero deadline delay = %v, want 0", got)
	}
	if got := processPickerStatusHoldDelay(now, now); got != 0 {
		t.Fatalf("elapsed deadline delay = %v, want 0", got)
	}
	if got := processPickerStatusHoldDelay(now, now.Add(1500*time.Millisecond)); got != 1500*time.Millisecond {
		t.Fatalf("active deadline delay = %v, want 1.5s", got)
	}
}

func TestSamePickerItemsDetectsOnlyVisibleProcessChanges(t *testing.T) {
	target := automation.ProcessTarget{Match: automation.MatchName, Executable: "player.exe"}
	base := []item{{target: target, name: "player.exe", description: "Player", count: "1", search: "player.exe player"}}
	if !samePickerItems(base, append([]item(nil), base...)) {
		t.Fatal("identical process rows should keep a silent activation refresh invisible")
	}
	changed := append([]item(nil), base...)
	changed[0].count = "2"
	if samePickerItems(base, changed) {
		t.Fatal("an instance-count change should publish the refreshed process rows")
	}
}

func TestBuildItemsKeepsMissingSelectedTargetsDeterministic(t *testing.T) {
	alpha := automation.ProcessTarget{Match: automation.MatchName, Executable: "Alpha.exe"}
	zeta := automation.ProcessTarget{Match: automation.MatchName, Executable: "zeta.exe"}
	selected := map[string]automation.ProcessTarget{
		zeta.Key():  zeta,
		alpha.Key(): alpha,
	}
	text := func(string) string { return "Not running" }
	first := buildItems(nil, selected, text)
	second := buildItems(nil, selected, text)
	if len(first) != 2 || first[0].target.Key() != alpha.Key() || first[1].target.Key() != zeta.Key() {
		t.Fatalf("missing selected process order = %+v, want Alpha.exe then zeta.exe", first)
	}
	if !samePickerItems(first, second) {
		t.Fatal("identical missing selected processes should keep automatic refresh silent")
	}
}

func TestProcessPickerPreviewIsAReadOnlyNeutralSurface(t *testing.T) {
	requireNativeIntegration(t)
	if err := Capture(testPickerOptions(), nil, 1, false, func(hwnd windows.Handle) error {
		p := activePickerForTest(t, hwnd)
		preview := p.controls[idPreview]
		pSendMessage.Call(uintptr(preview), 0x0200, 0, 0) // WM_MOUSEMOVE
		pSendMessage.Call(uintptr(preview), 0x0007, 0, 0) // WM_SETFOCUS
		if state := p.interaction.State(preview); state != (nativeform.InteractionState{}) {
			t.Fatalf("read-only preview exposed interactive field state: %+v", state)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestProcessPickerHeaderTracksHoverAndPressedStates(t *testing.T) {
	requireNativeIntegration(t)
	if err := Capture(testPickerOptions(), nil, 1, false, func(hwnd windows.Handle) error {
		p := activePickerForTest(t, hwnd)
		var first rect
		if ok, _, _ := pSendMessage.Call(uintptr(p.header), hdmGetItemRect, 0, uintptr(unsafe.Pointer(&first))); ok == 0 {
			t.Fatal("read first process header item")
		}
		x := uint16((first.Left + first.Right) / 2)
		y := uint16((first.Top + first.Bottom) / 2)
		point := uintptr(x) | uintptr(y)<<16

		pSendMessage.Call(uintptr(p.header), headerMouseMove, 0, point)
		if p.headerHover != 1 || p.headerPressed != 0 {
			t.Fatalf("header mouse move state = hover %d pressed %d, want 1/0", p.headerHover, p.headerPressed)
		}
		pSendMessage.Call(uintptr(p.header), headerLButtonDown, 0, point)
		if p.headerHover != 1 || p.headerPressed != 1 {
			t.Fatalf("header mouse down state = hover %d pressed %d, want 1/1", p.headerHover, p.headerPressed)
		}
		pSendMessage.Call(uintptr(p.header), headerLButtonUp, 0, point)
		if p.headerHover != 1 || p.headerPressed != 0 {
			t.Fatalf("header mouse up state = hover %d pressed %d, want 1/0", p.headerHover, p.headerPressed)
		}
		pSendMessage.Call(uintptr(p.header), headerMouseLeave, 0, 0)
		if p.headerHover != 0 || p.headerPressed != 0 {
			t.Fatalf("header mouse leave state = hover %d pressed %d, want 0/0", p.headerHover, p.headerPressed)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestProcessPickerCompletedLoadCommitsListHeaderAndPreviewPaint(t *testing.T) {
	requireNativeIntegration(t)
	err := Capture(testPickerOptions(), nil, 1, false, func(hwnd windows.Handle) error {
		p := activePickerForTest(t, hwnd)
		generation := nextGeneration.Add(1)
		p.generation = generation
		p.finishLoad(generation, []processcatalog.Instance{
			{Executable: "alpha.exe", Description: "Alpha", PID: 1},
			{Executable: "beta.exe", Description: "Beta", PID: 2},
		}, nil, true, time.Millisecond)
		for name, control := range map[string]windows.Handle{
			"list": p.controls[idList], "header": p.header, "preview": p.controls[idPreview],
		} {
			if control == 0 {
				t.Fatalf("%s control is missing", name)
			}
			if pending, _, _ := pickerTestGetUpdateRect.Call(uintptr(control), 0, 0); pending != 0 {
				t.Fatalf("%s retained a deferred paint region after the completed load", name)
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestUnchangedAutomaticRefreshDoesNotRepaintVisibleProcessState(t *testing.T) {
	requireNativeIntegration(t)
	groups := []processcatalog.Group{
		{Executable: "alpha.exe", Description: "Alpha", Count: 1},
		{Executable: "beta.exe", Description: "Beta", Count: 2},
	}
	instances := []processcatalog.Instance{
		{Executable: "alpha.exe", Description: "Alpha", PID: 1},
		{Executable: "beta.exe", Description: "Beta", PID: 2},
		{Executable: "beta.exe", Description: "Beta", PID: 3},
	}
	if err := Capture(testPickerOptions(), groups, 1, false, func(hwnd windows.Handle) error {
		p := activePickerForTest(t, hwnd)
		p.repaint()
		generation := nextGeneration.Add(1)
		p.generation = generation
		p.loadMode = processLoadAutomatic
		p.loadInFlight = true
		p.finishLoad(generation, instances, nil, true, time.Millisecond)
		if p.loadInFlight {
			t.Fatal("completed automatic refresh remained in flight")
		}
		for name, control := range map[string]windows.Handle{
			"status": p.controls[idStatus], "list": p.controls[idList], "preview": p.controls[idPreview],
		} {
			if pending, _, _ := pickerTestGetUpdateRect.Call(uintptr(control), 0, 0); pending != 0 {
				t.Fatalf("unchanged automatic refresh invalidated %s", name)
			}
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

// This opt-in diagnostic deliberately settles the deterministic Capture host
// before sampling desktop pixels. It verifies final native rendering only; it
// is not a substitute for the production Show path's asynchronous first frame.
func TestManualSettledProcessPickerVisiblePixels(t *testing.T) {
	if os.Getenv("IDLETRIGGER_MANUAL_VISIBLE_UI") != "1" {
		t.Skip("set IDLETRIGGER_MANUAL_VISIBLE_UI=1 for a settled-frame desktop rendering diagnostic")
	}
	requireNativeIntegration(t)
	groups := []processcatalog.Group{
		{Executable: "alpha.exe", Description: "Alpha process", Count: 1},
		{Executable: "beta.exe", Description: "Beta process", Count: 2},
	}
	err := Capture(testPickerOptions(), groups, 1, false, func(hwnd windows.Handle) error {
		p := activePickerForTest(t, hwnd)
		cueWindow := p.controls[idSearch]
		const showWithoutSizing = 0x0001 | 0x0040
		// The package test binary has no application manifest, so mixed-DPI
		// secondary monitors can virtualize GetWindowRect while desktop BitBlt
		// still uses physical pixels. Keep this screen-pixel assertion on the
		// primary monitor; product mixed-DPI coverage lives in the HWND tests.
		pSetWindowPos.Call(uintptr(hwnd), ^uintptr(0), 80, 80, 0, 0, showWithoutSizing)
		pSetForeground.Call(uintptr(hwnd))
		p.repaint()
		nativeform.PresentControl(p.controls[idSearch], true)
		nativeform.PresentControl(p.header, true)
		pumpPickerPaintMessages()
		pickerTestDWMFlush.Call()
		capture := capturePickerDesktopWindow(t, hwnd)
		checks := []struct {
			name       string
			control    windows.Handle
			background uint32
			minimum    int
		}{
			{"search cue", cueWindow, p.palette.Surface, 12},
			{"process header", p.header, p.palette.Surface, 12},
			{"process rows", p.controls[idList], p.palette.Surface, 30},
			{"selection preview", p.controls[idPreview], p.palette.Surface, 12},
		}
		failed := false
		for _, check := range checks {
			if pixels := visibleContrastingPixels(t, capture, check.control, check.background); pixels < check.minimum {
				t.Errorf("%s has only %d contrasting on-screen pixels, want at least %d", check.name, pixels, check.minimum)
				failed = true
			}
		}
		if got := p.controlText(idSearch); got != "" {
			t.Errorf("cue leaked into logical search text: %q", got)
			failed = true
		}
		pSendMessage.Call(uintptr(p.controls[idSearch]), 0x0102, 'x', 0) // WM_CHAR
		if got := p.controlText(idSearch); got != "x" {
			t.Errorf("first typed character after cue = %q, want x", got)
			failed = true
		}
		empty, _ := windows.UTF16PtrFromString("")
		pSetWindowText.Call(uintptr(p.controls[idSearch]), uintptr(unsafe.Pointer(empty)))
		if got := p.controlText(idSearch); got != "" {
			t.Errorf("restored cue leaked into logical search text: %q", got)
			failed = true
		}
		if failed {
			return fmt.Errorf("visible desktop pixel checks failed")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func pumpPickerPaintMessages() {
	const removeMessage = 0x0001
	for range 512 {
		var message pickerTestMessage
		available, _, _ := pickerTestPeekMessage.Call(uintptr(unsafe.Pointer(&message)), 0, 0, 0, removeMessage)
		if available == 0 {
			return
		}
		pickerTestTranslate.Call(uintptr(unsafe.Pointer(&message)))
		pickerTestDispatch.Call(uintptr(unsafe.Pointer(&message)))
	}
}

func capturePickerDesktopWindow(t *testing.T, hwnd windows.Handle) pickerDesktopCapture {
	t.Helper()
	var bounds rect
	if ok, _, _ := pGetWindowRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&bounds))); ok == 0 {
		t.Fatalf("read screen bounds for %v", hwnd)
	}
	width := int(bounds.Right - bounds.Left)
	height := int(bounds.Bottom - bounds.Top)
	if width <= 0 || height <= 0 {
		t.Fatalf("invalid window bounds %+v", bounds)
	}
	screenDC, _, _ := pGetDC.Call(0)
	if screenDC == 0 {
		t.Fatal("get desktop DC")
	}
	defer pReleaseDC.Call(0, screenDC)
	memoryDC, _, _ := pCreateCompatibleDC.Call(screenDC)
	if memoryDC == 0 {
		t.Fatal("create capture DC")
	}
	defer pDeleteDC.Call(memoryDC)

	bitmapInfo := pickerTestBitmapInfo{
		Header: pickerTestBitmapInfoHeader{
			Size:         uint32(unsafe.Sizeof(pickerTestBitmapInfoHeader{})),
			Width:        int32(width),
			Height:       -int32(height),
			Planes:       1,
			BitsPerPixel: 32,
		},
	}
	var bits uintptr
	bitmap, _, _ := pickerTestCreateDIB.Call(
		memoryDC,
		uintptr(unsafe.Pointer(&bitmapInfo)),
		0,
		uintptr(unsafe.Pointer(&bits)),
		0,
		0,
	)
	if bitmap == 0 || bits == 0 {
		t.Fatal("create capture bitmap")
	}
	oldBitmap, _, _ := pSelectObject.Call(memoryDC, bitmap)
	if oldBitmap == 0 {
		pDeleteObject.Call(bitmap)
		t.Fatal("select capture bitmap")
	}
	defer func() {
		pSelectObject.Call(memoryDC, oldBitmap)
		pDeleteObject.Call(bitmap)
	}()

	const sourceCopy = 0x00cc0020
	if ok, _, _ := pickerTestBitBlt.Call(
		memoryDC,
		0,
		0,
		uintptr(width),
		uintptr(height),
		screenDC,
		uintptr(int64(bounds.Left)),
		uintptr(int64(bounds.Top)),
		sourceCopy,
	); ok == 0 {
		t.Fatal("copy visible desktop pixels")
	}
	stride := width * 4
	pixels := make([]byte, stride*height)
	pickerTestCopyMemory.Call(uintptr(unsafe.Pointer(&pixels[0])), bits, uintptr(len(pixels)))
	return pickerDesktopCapture{bounds: bounds, pixels: pixels, stride: stride}
}

func visibleContrastingPixels(t *testing.T, capture pickerDesktopCapture, control windows.Handle, background uint32) int {
	t.Helper()
	var bounds rect
	if ok, _, _ := pGetWindowRect.Call(uintptr(control), uintptr(unsafe.Pointer(&bounds))); ok == 0 {
		t.Fatalf("read screen bounds for %v", control)
	}
	left := max(int(bounds.Left-capture.bounds.Left)+6, 0)
	top := max(int(bounds.Top-capture.bounds.Top)+3, 0)
	right := min(int(bounds.Right-capture.bounds.Left)-18, capture.stride/4)
	bottom := min(int(bounds.Bottom-capture.bounds.Top)-3, len(capture.pixels)/capture.stride)
	count, samples, maximumDistance := 0, 0, 0
	for y := top; y < bottom; y++ {
		row := capture.pixels[y*capture.stride : (y+1)*capture.stride]
		for x := left; x < right; x++ {
			offset := x * 4
			color := uint32(row[offset+2]) | uint32(row[offset+1])<<8 | uint32(row[offset])<<16
			samples++
			distance := colorDistance(color, background)
			if distance > maximumDistance {
				maximumDistance = distance
			}
			if distance >= 90 {
				count++
			}
		}
	}
	t.Logf("visible pixels hwnd=%v bounds=%+v background=%06x samples=%d contrasting=%d max-distance=%d", control, bounds, background&0xffffff, samples, count, maximumDistance)
	return count
}

func colorDistance(left, right uint32) int {
	abs := func(value int) int {
		if value < 0 {
			return -value
		}
		return value
	}
	return abs(int(left&0xff)-int(right&0xff)) +
		abs(int((left>>8)&0xff)-int((right>>8)&0xff)) +
		abs(int((left>>16)&0xff)-int((right>>16)&0xff))
}

func TestProcessPickerEmptyMessageFollowsExplicitViewPhase(t *testing.T) {
	p := &picker{options: Options{Text: func(key string) string { return key }}}
	tests := []struct {
		name   string
		phase  pickerViewPhase
		filter string
		want   string
	}{
		{"loading", pickerViewLoading, "", "process_picker_loading"},
		{"error", pickerViewError, "", "process_picker_load_failed"},
		{"ready empty", pickerViewReady, "", "process_picker_empty"},
		{"ready filtered", pickerViewReady, "player", "process_picker_no_results"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			p.viewPhase = test.phase
			if got := p.emptyMessage(test.filter); got != test.want {
				t.Fatalf("empty message = %q, want %q", got, test.want)
			}
		})
	}
}

func TestDescriptionCandidatesSkipPositiveAndNegativeCacheUnlessManual(t *testing.T) {
	instances := []processcatalog.Instance{
		{PID: 1, Executable: "cached.exe"},
		{PID: 2, Executable: "missing.exe"},
		{PID: 3, Executable: "new.exe"},
		{PID: 4, Executable: "new.exe"},
	}
	key := func(name string) string {
		return (automation.ProcessTarget{Match: automation.MatchName, Executable: name}).Key()
	}
	cached := map[string]string{key("cached.exe"): "Cached"}
	attempted := map[string]struct{}{key("missing.exe"): {}}
	candidates, attempts := descriptionCandidates(instances, cached, attempted, false)
	if len(candidates) != 2 || len(attempts) != 1 || attempts[0] != key("new.exe") {
		t.Fatalf("automatic candidates=%+v attempts=%+v", candidates, attempts)
	}
	candidates, attempts = descriptionCandidates(instances, cached, attempted, true)
	if len(candidates) != 3 || len(attempts) != 2 {
		t.Fatalf("manual candidates=%+v attempts=%+v", candidates, attempts)
	}
}

func TestFilterMatchesNameOrDescription(t *testing.T) {
	items := []item{
		{name: "player.exe", description: "Media Player", search: "player.exe media player"},
		{name: "render.exe", description: "Renderer", search: "render.exe renderer"},
	}
	got := filterItems(items, "media")
	if len(got) != 1 || got[0].name != "player.exe" {
		t.Fatalf("filtered items = %+v", got)
	}
}

func TestSameRelativeTargetOrder(t *testing.T) {
	values := func(names ...string) []item {
		items := make([]item, 0, len(names))
		for _, name := range names {
			items = append(items, item{target: automation.ProcessTarget{Match: automation.MatchName, Executable: name}})
		}
		return items
	}
	if !sameRelativeTargetOrder(values("a.exe", "b.exe", "c.exe"), values("a.exe", "c.exe")) {
		t.Fatal("filtering should preserve the relative row order")
	}
	if !sameRelativeTargetOrder(values("a.exe", "c.exe"), values("a.exe", "b.exe", "c.exe")) {
		t.Fatal("clearing a filter should preserve the relative row order")
	}
	if sameRelativeTargetOrder(values("a.exe", "b.exe", "c.exe"), values("c.exe", "b.exe", "a.exe")) {
		t.Fatal("sorting should require an ordered rebuild")
	}
	if sameRelativeTargetOrder(values("a.exe"), values("b.exe")) {
		t.Fatal("a disjoint refresh should use the fast full rebuild")
	}
}

func TestProcessColumnWidthsNeverRequireHorizontalScrolling(t *testing.T) {
	for _, test := range []struct {
		clientWidth int
		scale       float64
	}{
		{clientWidth: 996, scale: 1},
		{clientWidth: 1245, scale: 1.25},
		{clientWidth: 40, scale: 1},
	} {
		widths := processColumnWidths(test.clientWidth, test.scale)
		total := widths[0] + widths[1] + widths[2]
		reserve := max(1, int(float64(processScrollbarLane)*test.scale+0.5))
		if total > max(0, test.clientWidth-reserve) {
			t.Fatalf("processColumnWidths(%d, %.2f) = %v (total %d, reserve %d)", test.clientWidth, test.scale, widths, total, reserve)
		}
		for _, width := range widths {
			if width < 0 {
				t.Fatalf("processColumnWidths(%d, %.2f) contains a negative width: %v", test.clientWidth, test.scale, widths)
			}
		}
	}

	wide := processColumnWidths(996, 1)
	if wide[1] != 310 || wide[0] <= wide[1] {
		t.Fatalf("wide process picker should keep description compact and give spare room to process names: %v", wide)
	}
}

func TestProcessPickerSpacingUsesSharedRhythm(t *testing.T) {
	if pickerPreviewHeight%4 != 0 {
		t.Fatal("selection preview height should follow the shared 4 px size grid")
	}
	if pickerHelperY-(pickerHeadingY+pickerTextHeight) != pickerLabelGap {
		t.Fatal("heading and helper should use the compact related-text gap")
	}
	if pickerSearchY-(pickerHelperY+pickerTextHeight) != pickerRelatedGap ||
		pickerListY-(pickerSearchY+pickerFieldHeight) != pickerRelatedGap ||
		pickerStatusY-(pickerListY+pickerListHeight) != pickerRelatedGap ||
		pickerPreviewTitleY-(pickerStatusY+pickerTextHeight) != pickerRelatedGap {
		t.Fatal("process picker related controls should use the shared 8 px gap")
	}
	if pickerPreviewY-(pickerPreviewTitleY+pickerTextHeight) != pickerLabelGap {
		t.Fatal("selection labels should use the shared 4 px label gap")
	}
	privacyTopGap := pickerPrivacyY - (pickerPreviewY + pickerPreviewHeight)
	privacyBottomGap := pickerButtonsY - (pickerPrivacyY + pickerTextHeight)
	if privacyTopGap != pickerRelatedGap-pickerTextOpticalBias ||
		privacyBottomGap != pickerRelatedGap+pickerTextOpticalBias ||
		privacyTopGap+privacyBottomGap != 2*pickerRelatedGap {
		t.Fatal("process picker footer text should be optically centered within the shared 8 px rhythm")
	}
	if difference := privacyBottomGap - privacyTopGap; difference < -2 || difference > 2 {
		t.Fatalf("process picker footer gaps differ by %d px, want at most 2 px", difference)
	}
	if windowHeight-(pickerButtonsY+nativeform.ButtonHeight) != pickerBottomPadding {
		t.Fatal("process picker buttons should keep the shared bottom inset")
	}
	if emptyY := pickerListY + (pickerListHeight-pickerTextHeight)/2; emptyY*2+pickerTextHeight != pickerListY*2+pickerListHeight {
		t.Fatal("empty-state text should be vertically centered in the process list")
	}
}

func TestProcessPickerExcludesOnlyItsOwnPID(t *testing.T) {
	instances := []processcatalog.Instance{
		{PID: 10, Executable: "IdleTrigger.exe"},
		{PID: 11, Executable: "other.exe"},
	}
	filtered := excludePickerProcess(instances, 10)
	if len(filtered) != 1 || filtered[0].PID != 11 {
		t.Fatalf("filtered instances = %+v", filtered)
	}
}

func TestWriteUTF16TextTerminatesAndTruncates(t *testing.T) {
	buffer := make([]uint16, 5)
	writeUTF16Text(&buffer[0], int32(len(buffer)), "进程说明")
	if buffer[len(buffer)-1] != 0 {
		t.Fatalf("buffer is not terminated: %v", buffer)
	}
	got := string(utf16.Decode(buffer[:len(buffer)-1]))
	if got != "进程说明" {
		t.Fatalf("decoded text = %q, want %q", got, "进程说明")
	}

	short := make([]uint16, 4)
	writeUTF16Text(&short[0], int32(len(short)), "abcdef")
	if got := string(utf16.Decode(short[:3])); got != "abc" || short[3] != 0 {
		t.Fatalf("truncated buffer = %v, decoded = %q", short, got)
	}
}

func TestSelectionLimitRejectsOnlyTheAdditionalTarget(t *testing.T) {
	selected := make(map[string]automation.ProcessTarget, automation.MaxProcessesPerRule)
	for index := 0; index < automation.MaxProcessesPerRule; index++ {
		target := automation.ProcessTarget{Match: automation.MatchName, Executable: fmt.Sprintf("process-%02d.exe", index)}
		selected[target.Key()] = target
	}
	existing := selected["name:process-00.exe"]
	if !canAddSelection(selected, existing) {
		t.Fatal("an already-selected target should remain selectable at the limit")
	}
	additional := automation.ProcessTarget{Match: automation.MatchName, Executable: "additional.exe"}
	if canAddSelection(selected, additional) {
		t.Fatal("a 65th process target was accepted")
	}
	pathTarget := automation.ProcessTarget{Match: automation.MatchPath, Executable: "process-00.exe", Path: `C:\Apps\process-00.exe`}
	delete(selected, existing.Key())
	selected[pathTarget.Key()] = pathTarget
	if !canAddSelection(selected, existing) {
		t.Fatal("a name target that replaces an exact-path target should be allowed at the limit")
	}
	if got := normalizeSelected(selected); len(got) != automation.MaxProcessesPerRule {
		t.Fatalf("normalization changed the existing selection: %d", len(got))
	}
}

func requireNativeIntegration(t *testing.T) {
	t.Helper()
	if testing.Short() {
		t.Skip("skipping native Win32 integration test in short mode")
	}
}

func TestProcessPickerDefaultColumnsDoNotShowHorizontalScrollBar(t *testing.T) {
	requireNativeIntegration(t)
	groups := make([]processcatalog.Group, 40)
	for index := range groups {
		groups[index] = processcatalog.Group{Executable: fmt.Sprintf("player-%02d.exe", index), Description: "Media Player", Count: 2}
	}
	if err := Capture(testPickerOptions(), groups, 1, false, func(hwnd windows.Handle) error {
		p := activePickerForTest(t, hwnd)
		assertNoHorizontalProcessScroll(t, p)
		assertHeaderLeavesScrollbarLane(t, p)
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestProcessPickerRemainsOperableAcrossDPIAndSmallWorkArea(t *testing.T) {
	requireNativeIntegration(t)
	work := nativeform.Rect{Right: 1366, Bottom: 768}
	groups := make([]processcatalog.Group, 40)
	for index := range groups {
		groups[index] = processcatalog.Group{Executable: fmt.Sprintf("player-%02d.exe", index), Description: "Media Player", Count: 2}
	}
	for _, scale := range []float64{1, 1.25, 1.5, 2} {
		t.Run(formatPickerScale(scale), func(t *testing.T) {
			err := Capture(testPickerOptions(), groups, scale, false, func(hwnd windows.Handle) error {
				p := activePickerForTest(t, hwnd)
				p.positionInWorkArea(&work)
				if p.layoutErr != nil {
					t.Fatalf("small-work-area layout: %v", p.layoutErr)
				}
				assertPickerRectInside(t, pickerWindowRect(t, hwnd), work)
				p.scrollContentTo(max(0, windowHeight-p.viewportHeight))
				client := pickerClientScreenRect(t, hwnd)
				for _, id := range []uint16{idCancel, idConfirm} {
					assertPickerRectInside(t, pickerWindowRect(t, p.controls[id]), client)
				}
				wheelDelta := int16(-120)
				pSendMessage.Call(uintptr(p.controls[idList]), wmMouseWheel, uintptr(uint32(uint16(wheelDelta))<<16), 0)
				top, _, _ := pSendMessage.Call(uintptr(p.controls[idList]), lvmGetTopIndex, 0, 0)
				if top == 0 {
					t.Fatal("process list did not respond to the mouse wheel")
				}
				assertNoHorizontalProcessScroll(t, p)
				assertHeaderLeavesScrollbarLane(t, p)
				return nil
			})
			if err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestProcessPickerAppliesSuggestedRectAcrossDPIChanges(t *testing.T) {
	requireNativeIntegration(t)
	err := Capture(testPickerOptions(), nil, 1, false, func(hwnd windows.Handle) error {
		p := activePickerForTest(t, hwnd)
		p.captureScale = 0
		listHandle := p.controls[idList]
		workAreas := pickerMonitorWorkAreas(t)
		for index, dpi := range []uint32{96, 120, 144, 192, 120} {
			work := workAreas[index%len(workAreas)]
			scale := float64(dpi) / 96
			width, height, err := nativeform.WindowSizeForClient(int(windowWidth*scale+0.5), int(windowHeight*scale+0.5), p.style, p.exStyle, dpi)
			if err != nil {
				t.Fatal(err)
			}
			suggested := nativeform.Rect{Left: work.Left + 19, Top: work.Top + 23, Right: work.Left + 19 + width, Bottom: work.Top + 23 + height}
			pSendMessage.Call(uintptr(hwnd), wmDpiChanged, uintptr(dpi|(dpi<<16)), uintptr(unsafe.Pointer(&suggested)))
			if p.dpiScale != scale {
				t.Fatalf("DPI scale = %.2f, want %.2f", p.dpiScale, scale)
			}
			want := nativeform.ConstrainRect(suggested, work)
			if got := pickerWindowRect(t, hwnd); got != want {
				t.Fatalf("window rect after %d DPI = %+v, want %+v", dpi, got, want)
			}
			if p.controls[idList] != listHandle {
				t.Fatalf("process list HWND changed after %d DPI", dpi)
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestProcessPickerReleasesResourcesAcrossRepresentativeCycles(t *testing.T) {
	requireNativeIntegration(t)
	const (
		stabilizationCycles = 8
		measuredCycles      = 8
	)
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	groups := []processcatalog.Group{{Executable: "player.exe", Description: "Media Player", Count: 2}}
	// A native dark-mode ListView lazily loads process-wide common-control
	// caches. Warm both themes before measuring a representative second batch.
	for index := 0; index < stabilizationCycles; index++ {
		if err := Capture(testPickerOptions(), groups, 1, index%2 != 0, nil); err != nil {
			t.Fatalf("stabilization cycle %d: %v", index+1, err)
		}
	}
	before, err := wintest.StableResources()
	if err != nil {
		t.Fatal(err)
	}
	runMeasuredCycles := func() {
		for index := 0; index < measuredCycles; index++ {
			if err := Capture(testPickerOptions(), groups, 1, index%2 == 0, nil); err != nil {
				t.Fatalf("cycle %d: %v", index+1, err)
			}
		}
	}
	runMeasuredCycles()
	after, err := wintest.StableResources()
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("process-picker resources across %d cycles before=%+v after=%+v", measuredCycles, before, after)
	if after.GDI > before.GDI || after.USER > before.USER {
		runMeasuredCycles()
		repeated, repeatErr := wintest.StableResources()
		if repeatErr != nil {
			t.Fatal(repeatErr)
		}
		if repeated.GDI > after.GDI || repeated.USER > after.USER {
			t.Fatalf("GUI resources kept growing across repeated process-picker cycles: before=%+v after=%+v repeated=%+v", before, after, repeated)
		}
		t.Logf("process-picker cycles initialized stable process resources: before=%+v after=%+v repeated=%+v", before, after, repeated)
	}
}

func TestProcessPickerCreationFailuresReleasePartialResources(t *testing.T) {
	requireNativeIntegration(t)
	if err := Capture(testPickerOptions(), nil, 1, false, nil); err != nil {
		t.Fatal(err)
	}
	type fault struct {
		name    string
		install func() func()
	}
	createFailure := func(failAt int) func() func() {
		return func() func() {
			original := createWindowForPicker
			calls := 0
			createWindowForPicker = func(exStyle uintptr, class, caption *uint16, style uintptr, x, y, width, height int, parent windows.Handle, id uintptr) (windows.Handle, error) {
				calls++
				if calls == failAt {
					return 0, errors.New("injected CreateWindowEx failure")
				}
				return original(exStyle, class, caption, style, x, y, width, height, parent, id)
			}
			return func() { createWindowForPicker = original }
		}
	}
	faults := []fault{
		{"top-level HWND", createFailure(1)},
		{"first child HWND", createFailure(2)},
		{"list HWND", createFailure(9)},
		{"tooltip HWND", createFailure(18)},
		{"font", func() func() {
			original := newFontForPicker
			newFontForPicker = func(int32, int32, bool) (windows.Handle, font.Choice) { return 0, font.Choice{} }
			return func() { newFontForPicker = original }
		}},
		{"brush", func() func() {
			original := createBrushForPicker
			calls := 0
			createBrushForPicker = func(color uint32) windows.Handle {
				calls++
				if calls == 1 {
					return 0
				}
				return original(color)
			}
			return func() { createBrushForPicker = original }
		}},
		{"cue banner", func() func() {
			original := newCueBannerForPicker
			newCueBannerForPicker = func(windows.Handle, string, uint32, float64) (*nativeform.CueBanner, error) {
				return nil, errors.New("injected cue failure")
			}
			return func() { newCueBannerForPicker = original }
		}},
		{"list scrollbar", func() func() {
			original := newScrollbarForPicker
			newScrollbarForPicker = func(nativeform.ScrollbarOptions) (*nativeform.Scrollbar, error) {
				return nil, errors.New("injected scrollbar failure")
			}
			return func() { newScrollbarForPicker = original }
		}},
		{"preview scrollbar", func() func() {
			original := newListboxScrollForPicker
			newListboxScrollForPicker = func(nativeform.ListboxScrollbarOptions) (*nativeform.ListboxScrollbar, error) {
				return nil, errors.New("injected listbox scrollbar failure")
			}
			return func() { newListboxScrollForPicker = original }
		}},
		{"content scrollbar", func() func() {
			original := newScrollbarForPicker
			calls := 0
			newScrollbarForPicker = func(options nativeform.ScrollbarOptions) (*nativeform.Scrollbar, error) {
				calls++
				if calls == 2 {
					return nil, errors.New("injected content scrollbar failure")
				}
				return original(options)
			}
			return func() { newScrollbarForPicker = original }
		}},
	}
	for _, test := range faults {
		t.Run(test.name, func(t *testing.T) {
			runtime.LockOSThread()
			defer runtime.UnlockOSThread()
			if err := Capture(testPickerOptions(), nil, 1, false, nil); err != nil {
				t.Fatal(err)
			}
			runFault := func() {
				restore := test.install()
				err := Capture(testPickerOptions(), nil, 1, false, nil)
				restore()
				if err == nil {
					t.Fatal("injected creation failure reported success")
				}
				activeMu.Lock()
				remaining := active
				activeMu.Unlock()
				if remaining != nil {
					t.Fatalf("active picker survived creation failure: %+v", remaining)
				}
			}
			before, err := wintest.StableResources()
			if err != nil {
				t.Fatal(err)
			}
			runFault()
			after, snapshotErr := wintest.StableResources()
			if snapshotErr != nil {
				t.Fatal(snapshotErr)
			}
			if after.GDI > before.GDI || after.USER > before.USER {
				// Some Win32 controls retain process-wide resources the first time a
				// particular creation path is exercised. A real leak keeps growing
				// when the identical failure path is repeated.
				runFault()
				repeated, repeatErr := wintest.StableResources()
				if repeatErr != nil {
					t.Fatal(repeatErr)
				}
				if repeated.GDI > after.GDI || repeated.USER > after.USER {
					t.Fatalf("GUI resources kept growing across repeated injected failures: before=%+v after=%+v repeated=%+v", before, after, repeated)
				}
				t.Logf("injected failure initialized stable process resources: before=%+v after=%+v repeated=%+v", before, after, repeated)
				return
			}
			t.Logf("injected failure resources before=%+v after=%+v", before, after)
		})
	}
}

func TestProcessPickerIgnoresCallbacksThatArriveAfterClose(t *testing.T) {
	requireNativeIntegration(t)
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	originalSnapshot := snapshotNamesForPicker
	originalEnrich := enrichDescriptionsForPicker
	originalPost := postPickerUI
	defer func() {
		snapshotNamesForPicker = originalSnapshot
		enrichDescriptionsForPicker = originalEnrich
		postPickerUI = originalPost
	}()
	started := make(chan struct{})
	release := make(chan struct{})
	snapshotNamesForPicker = func() ([]processcatalog.Instance, error) {
		close(started)
		<-release
		return []processcatalog.Instance{{Executable: "late.exe"}}, nil
	}
	enrichDescriptionsForPicker = func(values []processcatalog.Instance) []processcatalog.Instance { return values }
	var callbacks []func()
	var callbackMu sync.Mutex
	postPickerUI = func(callback func()) bool {
		callbackMu.Lock()
		callbacks = append(callbacks, callback)
		callbackMu.Unlock()
		return true
	}
	if err := Show(testPickerOptions()); err != nil {
		t.Fatal(err)
	}
	activeMu.Lock()
	p := active
	activeMu.Unlock()
	if p == nil {
		t.Fatal("picker was not active")
	}
	<-started
	Hide()
	close(release)
	p.workers.Wait()
	callbackMu.Lock()
	late := append([]func(){}, callbacks...)
	callbackMu.Unlock()
	for _, callback := range late {
		callback()
	}
	if !p.closed.Load() || p.hwnd != 0 {
		t.Fatalf("closed picker retained live state: closed=%v hwnd=%v", p.closed.Load(), p.hwnd)
	}
	activeMu.Lock()
	remaining := active
	activeMu.Unlock()
	if remaining != nil {
		t.Fatalf("late callback restored active picker: %+v", remaining)
	}
	if len(p.items) != 0 {
		t.Fatalf("late callback rewrote closed picker items: %+v", p.items)
	}
}

func testPickerOptions() Options {
	return Options{Text: func(key string) string { return key }}
}

func activePickerForTest(t *testing.T, hwnd windows.Handle) *picker {
	t.Helper()
	activeMu.Lock()
	p := active
	activeMu.Unlock()
	if p == nil || p.hwnd != hwnd {
		t.Fatalf("active process picker does not match hwnd %v", hwnd)
	}
	return p
}

func formatPickerScale(scale float64) string {
	return map[float64]string{1: "100", 1.25: "125", 1.5: "150", 2: "200"}[scale]
}

func pickerWindowRect(t *testing.T, hwnd windows.Handle) nativeform.Rect {
	t.Helper()
	var value nativeform.Rect
	if ok, _, callErr := pickerTestGetWindowRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&value))); ok == 0 {
		t.Fatalf("GetWindowRect(%v): %v", hwnd, callErr)
	}
	return value
}

func pickerClientScreenRect(t *testing.T, hwnd windows.Handle) nativeform.Rect {
	t.Helper()
	var value nativeform.Rect
	if ok, _, callErr := pickerTestGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&value))); ok == 0 {
		t.Fatalf("GetClientRect(%v): %v", hwnd, callErr)
	}
	topLeft := struct{ X, Y int32 }{X: value.Left, Y: value.Top}
	bottomRight := struct{ X, Y int32 }{X: value.Right, Y: value.Bottom}
	pickerTestClientToScreen.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&topLeft)))
	pickerTestClientToScreen.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&bottomRight)))
	return nativeform.Rect{Left: topLeft.X, Top: topLeft.Y, Right: bottomRight.X, Bottom: bottomRight.Y}
}

func pickerMonitorWorkAreas(t *testing.T) []nativeform.Rect {
	t.Helper()
	var values []nativeform.Rect
	callback := windows.NewCallback(func(monitor, hdc, clip, data uintptr) uintptr {
		type info struct {
			Size          uint32
			Monitor, Work nativeform.Rect
			Flags         uint32
		}
		value := info{Size: uint32(unsafe.Sizeof(info{}))}
		if ok, _, _ := pickerTestGetMonitorInfo.Call(monitor, uintptr(unsafe.Pointer(&value))); ok != 0 {
			values = append(values, value.Work)
		}
		return 1
	})
	if ok, _, callErr := pickerTestEnumMonitors.Call(0, 0, callback, 0); ok == 0 {
		t.Fatalf("EnumDisplayMonitors: %v", callErr)
	}
	if len(values) == 0 {
		t.Fatal("EnumDisplayMonitors returned no work areas")
	}
	t.Logf("exercising WM_DPICHANGED across %d monitor work area(s)", len(values))
	return values
}

func assertPickerRectInside(t *testing.T, inner, outer nativeform.Rect) {
	t.Helper()
	if inner.Left < outer.Left || inner.Top < outer.Top || inner.Right > outer.Right || inner.Bottom > outer.Bottom {
		t.Fatalf("rect %+v escapes %+v", inner, outer)
	}
}

func assertNoHorizontalProcessScroll(t *testing.T, p *picker) {
	t.Helper()
	type scrollBarInfo struct {
		Size                        uint32
		Scroll                      nativeform.Rect
		Line, ThumbTop, ThumbBottom int32
		Reserved                    int32
		State                       [6]uint32
	}
	value := scrollBarInfo{Size: uint32(unsafe.Sizeof(scrollBarInfo{}))}
	const (
		objectHorizontalScroll = ^uintptr(5) // OBJID_HSCROLL (-6)
		stateInvisible         = 0x00008000
	)
	ok, _, _ := pickerTestScrollBarInfo.Call(uintptr(p.controls[idList]), objectHorizontalScroll, uintptr(unsafe.Pointer(&value)))
	if ok != 0 && value.State[0]&stateInvisible == 0 {
		t.Fatalf("process list horizontal scrollbar is visible: %+v", value)
	}
}

func assertHeaderLeavesScrollbarLane(t *testing.T, p *picker) {
	t.Helper()
	var headerClient, last rect
	if ok, _, _ := pGetClientRect.Call(uintptr(p.header), uintptr(unsafe.Pointer(&headerClient))); ok == 0 {
		t.Fatal("read process header client bounds")
	}
	if ok, _, _ := pSendMessage.Call(uintptr(p.header), hdmGetItemRect, 2, uintptr(unsafe.Pointer(&last))); ok == 0 {
		t.Fatal("read last process header item")
	}
	gap := headerClient.Right - last.Right
	want := int32(max(1, int(float64(processScrollbarLane)*p.scale()+0.5)))
	if gap < want-1 || gap > want+1 {
		t.Fatalf("process header scrollbar lane = %d px, want %d (client=%+v last=%+v)", gap, want, headerClient, last)
	}
}
