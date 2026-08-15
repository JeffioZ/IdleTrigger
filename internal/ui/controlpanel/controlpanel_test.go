package controlpanel

import (
	"strings"
	"testing"
	"unsafe"

	"github.com/JeffioZ/idletrigger/internal/ui/colors"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"golang.org/x/sys/windows"
)

func TestAutomationTooltipFormatsCountSummaryAndExplanation(t *testing.T) {
	texts := map[string]string{
		"tip_automation_status": "%d enabled\n%s\n%s",
		"tip_automation":        "Built-in tasks only.",
	}
	p := panel{automationCount: 2, automationSummary: "Next: 23:00", lang: func(key string) string { return texts[key] }}
	got := p.tooltipText(idAutomation)
	if got != "2 enabled\nNext: 23:00\nBuilt-in tasks only." || strings.Contains(got, "%!") {
		t.Fatalf("automation tooltip = %q", got)
	}
}

func TestPowerManagementTooltipSeparatesManualAndRuntimeState(t *testing.T) {
	texts := map[string]string{
		"tip_power_setting_status":     "Manual setting: %s.\nRuntime status: %s.\n%s",
		"tip_state_enabled":            "on",
		"tip_state_disabled":           "off",
		"tip_nosleep":                  "Stay Awake help.",
		"tip_idle":                     "Idle help.",
		"tip_idle_manual_plan_warning": "Manual plan: after %d idle minutes, run %s with a %d-second reminder",
		"menu_action_lock":             "Lock",
		"status_unknown":               "Unknown",
	}
	p := panel{
		lang:               func(key string) string { return texts[key] },
		noSleepStatus:      "Enabled by an automatic task",
		idleStatus:         "Paused by Stay Awake",
		idleTimeoutMinutes: 30,
		idleWarningSeconds: 20,
		idleAction:         "lock",
		toggles:            map[uint16]bool{idNoSleep: false, idIdle: true},
		disabled:           map[uint16]bool{},
	}
	if got := p.tooltipText(idNoSleep); got != "Manual setting: off.\nRuntime status: Enabled by an automatic task.\nStay Awake help." {
		t.Fatalf("Stay Awake tooltip = %q", got)
	}
	if got := p.tooltipText(idIdle); got != "Manual setting: on.\nRuntime status: Paused by Stay Awake.\nManual plan: after 30 idle minutes, run Lock with a 20-second reminder\nIdle help." {
		t.Fatalf("idle tooltip = %q", got)
	}
}

func TestIdleTooltipExplainsSilentManualPlan(t *testing.T) {
	texts := map[string]string{
		"tip_idle":                    "Idle help.",
		"tip_idle_manual_plan_silent": "Manual plan: after %d idle minutes, run %s without a reminder",
		"menu_action_shutdown":        "Shut down",
	}
	p := panel{
		lang:               func(key string) string { return texts[key] },
		idleTimeoutMinutes: 45,
		idleAction:         "shutdown",
	}
	if got := p.idleTooltipBody(); got != "Manual plan: after 45 idle minutes, run Shut down without a reminder\nIdle help." || strings.Contains(got, "%!") {
		t.Fatalf("idle tooltip body = %q", got)
	}
}

func TestQuickActionsKeepTheCompleteBuiltInSystemSet(t *testing.T) {
	want := []uint16{idLock, idSleep, idHibernate, idShutdown, idRestart}
	got := quickActionIDs()
	if len(got) != len(want) {
		t.Fatalf("quick actions = %v", got)
	}
	for index := range want {
		if got[index] != want[index] {
			t.Fatalf("quick action %d = %d, want %d", index, got[index], want[index])
		}
	}
}

func TestVisualStateForButtonRoles(t *testing.T) {
	tests := []struct {
		name           string
		id             uint16
		toggleOn, down bool
		wantRole       buttonRole
		wantActive     bool
	}{
		{name: "toggle on", id: idNoSleep, toggleOn: true, wantRole: buttonToggle, wantActive: true},
		{name: "toggle off", id: idTheme, wantRole: buttonToggle, wantActive: false},
		{name: "system controls remains a command", id: idQuickActions, toggleOn: true, wantRole: buttonCommand, wantActive: false},
		{name: "disabled state is retained", id: idIdle, toggleOn: true, down: true, wantRole: buttonToggle, wantActive: true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := visualStateForButton(tt.id, tt.toggleOn, tt.down)
			if got.Role != tt.wantRole || got.Active != tt.wantActive || got.Disabled != tt.down {
				t.Fatalf("visualStateForButton(%d) = %#v, want role=%d active=%v disabled=%v", tt.id, got, tt.wantRole, tt.wantActive, tt.down)
			}
		})
	}
}

func TestControlStateRetainsInteractiveFlags(t *testing.T) {
	p := &panel{
		toggles:            map[uint16]bool{idNoSleep: true},
		disabled:           map[uint16]bool{idNoSleep: true},
		hoverID:            idNoSleep,
		keyboardNavigation: true,
	}
	state := p.controlState(idNoSleep, odsSelected|odsFocus)
	if !state.Active || !state.Hovered || !state.Pressed || !state.Disabled || !state.Focused {
		t.Fatalf("control state lost an existing interaction flag: %#v", state)
	}
}

func TestControlStateIncludesNativeDisabledFlag(t *testing.T) {
	p := &panel{toggles: map[uint16]bool{}, disabled: map[uint16]bool{}}
	if state := p.controlState(idSettings, odsDisabled); !state.Disabled {
		t.Fatalf("native disabled flag was lost: %#v", state)
	}
}

func TestPopupTriggerIsLimitedToSystemControls(t *testing.T) {
	if !isPopupTrigger(idQuickActions) {
		t.Fatal("system controls must open the shared popup")
	}
	for _, id := range []uint16{idSettings, idExit, idSleep, idThemeSwitch, idThemeRepair} {
		if isPopupTrigger(id) {
			t.Fatalf("command %d must not be a popup trigger", id)
		}
	}
}

func TestDangerQuickActionsAreLimitedToShutdownAndRestart(t *testing.T) {
	for _, id := range []uint16{idLock, idSleep, idHibernate} {
		if isDangerQuickAction(id) {
			t.Fatalf("ordinary quick action %d was marked dangerous", id)
		}
	}
	for _, id := range []uint16{idShutdown, idRestart} {
		if !isDangerQuickAction(id) {
			t.Fatalf("danger quick action %d was not marked dangerous", id)
		}
	}
}

func TestPopupMenuItemsKeepOnlySemanticDifferences(t *testing.T) {
	p := &panel{lang: func(key string) string { return key }}
	quick := p.quickMenuItems()
	if len(quick) != len(quickActionIDs()) {
		t.Fatalf("quick items = %d", len(quick))
	}
	for _, item := range quick {
		wantDanger := item.Value == idShutdown || item.Value == idRestart
		if item.Danger != wantDanger {
			t.Fatalf("quick item %+v danger = %v, want %v", item, item.Danger, wantDanger)
		}
	}
}

func TestButtonRoleMappingCoversEveryPanelAction(t *testing.T) {
	for _, id := range []uint16{idNoSleep, idAutomationEnabled, idIdle, idTheme} {
		if got := roleForButton(id); got != buttonToggle {
			t.Fatalf("toggle id %d has role %d", id, got)
		}
	}
	for _, id := range []uint16{idQuickActions, idAutomation, idLock, idSleep, idHibernate, idShutdown, idRestart, idThemeSwitch, idThemeRepair, idSettings, idExit} {
		if got := roleForButton(id); got != buttonCommand {
			t.Fatalf("command id %d has role %d", id, got)
		}
	}
}

func TestFocusOutlineIsVisibleOnlyDuringKeyboardNavigation(t *testing.T) {
	p := &panel{}
	if p.shouldDrawFocusOutline(odsFocus) {
		t.Fatal("initial mouse-oriented panel should not show a focus outline")
	}
	p.enterKeyboardNavigation()
	if !p.shouldDrawFocusOutline(odsFocus) {
		t.Fatal("keyboard navigation should show the focused control outline")
	}
	p.leaveKeyboardNavigation()
	if p.shouldDrawFocusOutline(odsFocus) {
		t.Fatal("mouse interaction should hide the focus-visible outline without changing focus")
	}
	if p.shouldDrawFocusOutline(0) {
		t.Fatal("an unfocused control must not show a focus outline")
	}
}

func TestWindowIconThemeAndReloadDecisions(t *testing.T) {
	if appIconResourceID != 2 {
		t.Fatalf("class fallback resource = %d, want 2", appIconResourceID)
	}
	if got := windowIconResourceID(false); got != trayDarkIconResourceID {
		t.Fatalf("light theme resource = %d, want %d", got, trayDarkIconResourceID)
	}
	if got := windowIconResourceID(true); got != trayLightIconResourceID {
		t.Fatalf("dark theme resource = %d, want %d", got, trayLightIconResourceID)
	}
	for _, tt := range []struct {
		name                                   string
		initialized, current, requested, force bool
		want                                   bool
	}{
		{name: "initial load", want: true},
		{name: "theme change", initialized: true, current: false, requested: true, want: true},
		{name: "dpi refresh", initialized: true, current: true, requested: true, force: true, want: true},
		{name: "same theme skips reload", initialized: true, current: true, requested: true, want: false},
	} {
		t.Run(tt.name, func(t *testing.T) {
			if got := shouldReloadWindowIcons(tt.initialized, tt.current, tt.requested, tt.force); got != tt.want {
				t.Fatalf("shouldReloadWindowIcons() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestRefreshActionsKeepPanelOpen(t *testing.T) {
	for _, action := range []Action{ActSwitchTheme, ActRepairTheme, ActSettingsOpen, ActAutomationOpen} {
		if actionClosesPanel(action) {
			t.Fatalf("action %d should keep the panel available for an immediate refresh", action)
		}
	}
	for _, action := range []Action{ActSleep, ActRestart, ActExit} {
		if !actionClosesPanel(action) {
			t.Fatalf("action %d should close the panel", action)
		}
	}
}

func TestIdlePreviewActionTranslationKey(t *testing.T) {
	for _, action := range []string{"lock", "sleep", "hibernate", "shutdown", "restart"} {
		if got, want := idlePreviewActionTranslationKey(action), "menu_action_"+action; got != want {
			t.Fatalf("idlePreviewActionTranslationKey(%q) = %q, want %q", action, got, want)
		}
	}
	if got := idlePreviewActionTranslationKey("invalid"); got != "menu_action_lock" {
		t.Fatalf("invalid preview action key = %q", got)
	}
}

func TestPanelOriginPinsToWorkAreaBottomRight(t *testing.T) {
	work := rect{Left: 1920, Top: 0, Right: 3840, Bottom: 1040}
	x, y := panelOrigin(work, 720, 600, 16)
	if x != 3104 || y != 424 {
		t.Fatalf("panelOrigin() = (%d, %d), want (3104, 424)", x, y)
	}
}

func TestPanelOriginDoesNotEscapeSmallWorkArea(t *testing.T) {
	work := rect{Left: 100, Top: 50, Right: 500, Bottom: 300}
	x, y := panelOrigin(work, 720, 600, 16)
	if x != work.Left || y != work.Top {
		t.Fatalf("panelOrigin() = (%d, %d), want (%d, %d)", x, y, work.Left, work.Top)
	}
}

func TestPanelFallbackCoordinateCannotReachTheDesktop(t *testing.T) {
	if panelFallbackWindowCoordinate > -30000 {
		t.Fatalf("panel fallback coordinate %d is not safely outside the desktop", panelFallbackWindowCoordinate)
	}
}

func TestOwnedBrushesIncludesEveryPanelBrush(t *testing.T) {
	p := &panel{}
	p.dangerPressedBrush = windows.Handle(10)
	brushes := p.ownedBrushes()
	if len(brushes) != 10 {
		t.Fatalf("owned brush count = %d, want 10", len(brushes))
	}
	for _, brush := range brushes {
		if brush == p.dangerPressedBrush {
			return
		}
	}
	t.Fatal("danger pressed brush is missing from the release inventory")
}

func TestPopupMetricsUseOneDPITransform(t *testing.T) {
	metrics := newPanelMetrics(defaultPanelStyle, 1.5)
	if got := metrics.px(metrics.style.Layout.PanelWidth); got != 729 {
		t.Fatalf("scaled panel width = %d, want 729", got)
	}
}

func TestSuggestedDPIBoundsUsesTheSystemRectangle(t *testing.T) {
	want := rect{Left: -640, Top: 80, Right: 120, Bottom: 680}
	got, ok := suggestedDPIBounds(uintptr(unsafe.Pointer(&want)))
	if !ok || got != want {
		t.Fatalf("suggested DPI bounds = (%+v, %v), want (%+v, true)", got, ok, want)
	}
	invalid := rect{Left: 10, Top: 10, Right: 10, Bottom: 20}
	if _, ok := suggestedDPIBounds(uintptr(unsafe.Pointer(&invalid))); ok {
		t.Fatal("empty suggested DPI bounds must use the fallback placement")
	}
}

func TestPanelScaleUsesTheMessageDPI(t *testing.T) {
	if got := panelScaleForDPI(144, 0); got != 1.5 {
		t.Fatalf("message DPI scale = %v, want 1.5", got)
	}
	if got := panelScaleForDPI(144, 2); got != 2 {
		t.Fatalf("capture DPI scale = %v, want 2", got)
	}
}

func TestDPIFrameCommitsOnlyTheLatestGeneration(t *testing.T) {
	p := panel{pendingDPI: 144, dpiGeneration: 3, dpiReadyGeneration: 2}
	if p.dpiFrameReady() {
		t.Fatal("stale DPI layout must remain cloaked")
	}
	p.dpiReadyGeneration = 3
	if !p.dpiFrameReady() {
		t.Fatal("latest DPI layout should be ready to present")
	}
}

func TestPositionPreservesScaledClientBounds(t *testing.T) {
	const (
		scale            = 1.5
		testClientHeight = 300
	)
	err := Capture(State{}, func(key string) string { return key }, scale, func(hwnd windows.Handle) error {
		p := panelFor(hwnd)
		if p == nil {
			t.Fatal("capture panel is not registered")
		}
		// Keep the real capture scale while using a height that fits the smaller
		// virtual desktop exposed by GitHub's Windows runners.
		p.clientH = testClientHeight
		if err := p.position(p.style, p.exStyle); err != nil {
			t.Fatal(err)
		}
		width, height, err := nativeform.ClientSize(hwnd)
		if err != nil {
			t.Fatal(err)
		}
		wantWidth := p.sc(p.metrics.style.Layout.PanelWidth)
		wantHeight := p.sc(p.clientH)
		if width != wantWidth || height != wantHeight {
			t.Fatalf("capture client = %dx%d, want %dx%d", width, height, wantWidth, wantHeight)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestVisualStateTokensKeepSpecifiedLogicalSizes(t *testing.T) {
	control := defaultPanelStyle.Control
	if control.FocusInset != 2 || control.FocusRingWidth != 2 {
		t.Fatalf("focus tokens = inset %d width %d, want 2/2", control.FocusInset, control.FocusRingWidth)
	}
	metrics := newPanelMetrics(defaultPanelStyle, 1.5)
	if got := metrics.px(control.FocusRingWidth); got != 3 {
		t.Fatalf("scaled focus ring width = %d, want 3", got)
	}
}

func TestExplicitThemeDoesNotReadSystemTheme(t *testing.T) {
	if (&panel{theme: ThemeLight}).resolveTheme() {
		t.Fatal("explicit light theme resolved as dark")
	}
	if !(&panel{theme: ThemeDark}).resolveTheme() {
		t.Fatal("explicit dark theme resolved as light")
	}
}

func TestThemeRefreshCommitsCompleteFrame(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping native Win32 integration test in short mode")
	}
	getUpdateRect := windows.NewLazySystemDLL("user32.dll").NewProc("GetUpdateRect")
	err := Capture(State{Theme: ThemeLight}, func(key string) string { return key }, 1, func(hwnd windows.Handle) error {
		p := panelFor(hwnd)
		if p == nil {
			t.Fatal("capture panel is not active")
		}
		originalApplyControlTheme := applyPanelControlTheme
		defer func() { applyPanelControlTheme = originalApplyControlTheme }()
		themedControls := make(map[windows.Handle]int)
		applyPanelControlTheme = func(control windows.Handle, dark bool) {
			if dark {
				themedControls[control]++
			}
			originalApplyControlTheme(control, dark)
		}
		p.theme = ThemeDark
		p.hoverID = idThemeSwitch
		// Match the unique theme/color broadcasts sent by the theme feature and
		// include one duplicate to guard against late platform notifications.
		for _, message := range []uint32{wmSettingChange, wmSettingChange, wmSysColorChange, wmThemeChanged, wmThemeChanged} {
			pSendMessage.Call(uintptr(hwnd), uintptr(message), 0, 0)
		}
		if p.hwnd == 0 {
			t.Fatal("atomic theme refresh destroyed the capture panel")
		}
		if !p.themeDark || p.palette != colors.ForTheme(true) {
			t.Fatalf("theme refresh retained the light palette: dark=%v palette=%+v", p.themeDark, p.palette)
		}
		for id, control := range p.controls {
			if control == 0 {
				continue
			}
			if pending, _, _ := getUpdateRect.Call(uintptr(control), 0, 0); pending != 0 {
				t.Fatalf("theme refresh left control %d with a deferred paint region", id)
			}
			if count := themedControls[control]; count != 1 {
				t.Fatalf("theme broadcasts applied the dark native theme to control %d %d times, want once", id, count)
			}
		}
		if p.hoverID != idThemeSwitch || !p.controlState(idThemeSwitch, 0).Hovered {
			t.Fatal("theme broadcasts dropped the stable hover state")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestThemeRepairCompletionForcesCompleteFrame(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping native Win32 integration test in short mode")
	}
	getUpdateRect := windows.NewLazySystemDLL("user32.dll").NewProc("GetUpdateRect")
	err := Capture(State{Theme: ThemeDark}, func(key string) string { return key }, 1, func(hwnd windows.Handle) error {
		p := panelFor(hwnd)
		if p == nil {
			t.Fatal("capture panel is not active")
		}
		originalApplyControlTheme := applyPanelControlTheme
		defer func() { applyPanelControlTheme = originalApplyControlTheme }()
		themedControls := make(map[windows.Handle]int)
		applyPanelControlTheme = func(control windows.Handle, dark bool) {
			themedControls[control]++
			originalApplyControlTheme(control, dark)
		}

		RefreshThemeAfterSystemRepair()
		for id, control := range p.controls {
			if control == 0 {
				continue
			}
			if count := themedControls[control]; count != 1 {
				t.Fatalf("repair completion themed control %d %d times, want once", id, count)
			}
			if pending, _, _ := getUpdateRect.Call(uintptr(control), 0, 0); pending != 0 {
				t.Fatalf("repair completion left control %d with a deferred paint region", id)
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestThemeActionsShareOneEqualGridRow(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping native Win32 integration test in short mode")
	}
	labels := map[string]string{
		"menu_theme_enable":     "Enable Auto Switch",
		"menu_theme_switch_now": "Switch Theme",
		"menu_theme_repair":     "Repair Theme",
	}
	err := Capture(State{Theme: ThemeLight}, func(key string) string {
		if label := labels[key]; label != "" {
			return label
		}
		return key
	}, 1, func(hwnd windows.Handle) error {
		p := panelFor(hwnd)
		if p == nil {
			t.Fatal("capture panel is not active")
		}
		toggle := p.controlBounds[idTheme]
		switchTheme := p.controlBounds[idThemeSwitch]
		repairTheme := p.controlBounds[idThemeRepair]
		if toggle.x >= switchTheme.x || switchTheme.x >= repairTheme.x {
			t.Fatalf("theme controls are not in visual order: toggle=%+v switch=%+v repair=%+v", toggle, switchTheme, repairTheme)
		}
		if switchTheme.y != repairTheme.y || switchTheme.width != repairTheme.width || switchTheme.height != repairTheme.height {
			t.Fatalf("theme action grid is uneven: switch=%+v repair=%+v", switchTheme, repairTheme)
		}
		if toggle.width <= switchTheme.width {
			t.Fatalf("theme toggle width = %d, want more than action width %d", toggle.width, switchTheme.width)
		}
		if gap := repairTheme.x - switchTheme.x - switchTheme.width; gap != p.metrics.style.Layout.Gap {
			t.Fatalf("theme action gap = %d, want %d", gap, p.metrics.style.Layout.Gap)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestControlStateCombinesModelAndNativeState(t *testing.T) {
	p := &panel{
		toggles:            map[uint16]bool{idIdle: true},
		disabled:           map[uint16]bool{},
		hoverID:            idIdle,
		keyboardNavigation: true,
	}
	state := p.controlState(idIdle, odsSelected|odsFocus)
	if !state.Active || !state.Hovered || !state.Pressed || !state.Focused || state.Disabled {
		t.Fatalf("control state = %#v", state)
	}
}

func TestUnavailableThemeDisablesEveryThemeControl(t *testing.T) {
	p := &panel{
		themeUnavailable: true,
		toggles:          map[uint16]bool{idTheme: true},
		disabled:         map[uint16]bool{},
		controls:         map[uint16]windows.Handle{},
		tooltips:         map[uint16][]uint16{},
	}
	p.applyDependentStates()
	for _, id := range []uint16{idTheme, idThemeSwitch, idThemeRepair} {
		if !p.disabled[id] {
			t.Fatalf("theme control %d remained enabled", id)
		}
	}
}

func TestAvailableThemeLeavesItsVisibleControlsEnabled(t *testing.T) {
	p := &panel{
		toggles:  map[uint16]bool{idTheme: true},
		disabled: map[uint16]bool{},
		controls: map[uint16]windows.Handle{},
		tooltips: map[uint16][]uint16{},
	}
	p.applyDependentStates()
	for _, id := range []uint16{idTheme, idThemeSwitch, idThemeRepair} {
		if p.disabled[id] {
			t.Fatalf("theme control %d remained disabled", id)
		}
	}
}

func TestBusyThemeOperationDisablesActionsButKeepsScheduleToggle(t *testing.T) {
	p := &panel{
		themeOperationBusy: true,
		toggles:            map[uint16]bool{idTheme: true},
		disabled:           map[uint16]bool{},
		controls:           map[uint16]windows.Handle{},
		tooltips:           map[uint16][]uint16{},
	}
	p.applyDependentStates()
	if p.disabled[idTheme] {
		t.Fatal("busy manual operation disabled the independent schedule toggle")
	}
	for _, id := range []uint16{idThemeSwitch, idThemeRepair} {
		if !p.disabled[id] {
			t.Fatalf("busy theme action %d remained enabled", id)
		}
	}
}

func TestThemeActionBusyLabelsRemainRoleSpecific(t *testing.T) {
	texts := map[string]string{
		"menu_theme_switch_now": "Switch Theme", "menu_theme_repair": "Repair Theme",
		"menu_theme_switching": "Switching…", "menu_theme_repairing": "Repairing…",
	}
	p := &panel{lang: func(key string) string { return texts[key] }}
	if got := p.themeActionLabel(idThemeSwitch); got != "Switch Theme" {
		t.Fatalf("idle switch label = %q", got)
	}
	p.themeOperationBusy = true
	if got := p.themeActionLabel(idThemeSwitch); got != "Switching…" {
		t.Fatalf("busy switch label = %q", got)
	}
	if got := p.themeActionLabel(idThemeRepair); got != "Repairing…" {
		t.Fatalf("busy repair label = %q", got)
	}
}

func TestOwnerDrawnButtonsIgnoreStaleNativeHotlight(t *testing.T) {
	p := &panel{
		toggles:  map[uint16]bool{},
		disabled: map[uint16]bool{},
	}
	for _, id := range []uint16{idNoSleep, idThemeSwitch, idThemeRepair, idQuickActions, idSettings, idExit} {
		if state := p.controlState(id, odsHotlight); state.Hovered {
			t.Fatalf("owner-drawn control %d retained stale native hotlight", id)
		}
	}
	sequence := []uint16{idNoSleep, idIdle, idThemeSwitch, idThemeRepair, idSettings, idExit}
	for index, current := range sequence {
		p.hoverID = current
		if state := p.controlState(current, 0); !state.Hovered {
			t.Fatalf("tracked hover was lost for control %d", current)
		}
		if index > 0 {
			previous := sequence[index-1]
			if state := p.controlState(previous, odsHotlight); state.Hovered {
				t.Fatalf("rapid transition %d -> %d restored stale native hotlight", previous, current)
			}
		}
	}
}

func TestChoiceTriggerKeyboardKeysOpenTheSharedPopup(t *testing.T) {
	for _, key := range []uintptr{vkReturn, vkSpace, vkUp, vkDown, vkF4} {
		if !isChoiceOpenKey(key) {
			t.Fatalf("key %#x should open a choice popup", key)
		}
	}
	for _, key := range []uintptr{vkEscape, vkHome, vkEnd} {
		if isChoiceOpenKey(key) {
			t.Fatalf("key %#x must not open a closed choice popup", key)
		}
	}
}

func TestSystemControlsTriggerTogglesARealSharedPopup(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping native Win32 integration test in short mode")
	}
	err := Capture(State{}, func(key string) string { return key }, 1, func(hwnd windows.Handle) error {
		p := panelFor(hwnd)
		if p == nil {
			t.Fatal("capture panel is not active")
		}
		p.openQuickMenu()
		popup := p.choice.popup
		if p.choice.openID != idQuickActions || popup == nil || !popup.IsOpen() || popup.Window() == 0 {
			t.Fatal("system controls did not create the shared native popup")
		}
		p.openQuickMenu()
		if p.choice.openID != 0 || p.choice.popup != nil || popup.IsOpen() {
			t.Fatal("clicking the open choice trigger did not close its popup")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestSystemControlsUseTheSharedPopup(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping native Win32 integration test in short mode")
	}
	err := Capture(State{}, func(key string) string { return key }, 1, func(hwnd windows.Handle) error {
		p := panelFor(hwnd)
		if p == nil {
			t.Fatal("capture panel is not active")
		}
		for _, id := range quickActionIDs() {
			if p.controls[id] != 0 {
				t.Fatalf("legacy fixed-menu child %d still exists", id)
			}
		}
		p.openQuickMenu()
		quick := p.choice.popup
		if p.choice.openID != idQuickActions || quick == nil || !quick.IsOpen() {
			t.Fatal("system controls did not open the shared popup")
		}
		p.closeChoice(false)
		if quick.IsOpen() || p.choice.openID != 0 || p.choice.popup != nil {
			t.Fatal("system-controls popup did not close through the shared lifecycle")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestPanelBackgroundClickClosesEveryOpenMenu(t *testing.T) {
	p := &panel{
		choice: choiceSurface{
			openID: idQuickActions,
		},
	}
	p.closeOpenMenus()
	if p.choice.openID != 0 {
		t.Fatalf("background click left popup open: %d", p.choice.openID)
	}
}

func TestMenuClickKeepsOnlyTheOpenSurfaceInteractive(t *testing.T) {
	p := &panel{choice: choiceSurface{openID: idQuickActions}}
	if !p.menuClickKeepsOpen(idQuickActions) {
		t.Fatal("clicking the open system-controls trigger should keep its popup alive until the deferred toggle")
	}
	if p.menuClickKeepsOpen(idSleep) {
		t.Fatal("popup values are no longer owner child controls")
	}
	if p.menuClickKeepsOpen(idSettings) {
		t.Fatal("another trigger must close the current menu before switching")
	}
}

func TestCommandActionMapsDirectCommands(t *testing.T) {
	p := &panel{}
	tests := []struct {
		id     uint16
		action Action
	}{
		{idSleep, ActSleep},
		{idHibernate, ActHibernate},
		{idShutdown, ActShutdown},
		{idLock, ActLock},
		{idRestart, ActRestart},
		{idThemeSwitch, ActSwitchTheme},
		{idThemeRepair, ActRepairTheme},
		{idSettings, ActSettingsOpen},
		{idExit, ActExit},
	}
	for _, test := range tests {
		action, value, ok := p.commandAction(test.id)
		if !ok || action != test.action || value != 0 {
			t.Fatalf("command %d = (%d, %d, %v), want (%d, 0, true)", test.id, action, value, ok, test.action)
		}
	}
}

func TestToggleCommandsPreserveIdleMutualExclusion(t *testing.T) {
	p := &panel{
		toggles:  map[uint16]bool{idIdle: true},
		disabled: map[uint16]bool{},
	}
	action, ok := p.toggleCommand(idNoSleep)
	if !ok || action != ActNoSleepToggle || !p.toggles[idNoSleep] || p.toggles[idIdle] {
		t.Fatalf("Stay Awake toggle = action %d, ok=%v, toggles=%v", action, ok, p.toggles)
	}

	p.toggles[idNoSleep] = true
	action, ok = p.toggleCommand(idIdle)
	if !ok || action != ActIdleToggle || !p.toggles[idIdle] || p.toggles[idNoSleep] {
		t.Fatalf("idle toggle = action %d, ok=%v, toggles=%v", action, ok, p.toggles)
	}
}

func TestSettingsCommandOpensTheDedicatedSettingsWindow(t *testing.T) {
	p := &panel{}
	action, value, ok := p.commandAction(idSettings)
	if !ok || action != ActSettingsOpen || value != 0 {
		t.Fatalf("settings command = (%v, %d, %v)", action, value, ok)
	}
}

func TestEveryControlPanelActionHasAUICommandPath(t *testing.T) {
	p := &panel{
		toggles:  map[uint16]bool{},
		disabled: map[uint16]bool{},
	}
	mapped := map[Action]bool{}
	for _, id := range []uint16{
		idAutomation, idSleep, idHibernate, idShutdown, idLock, idRestart,
		idThemeSwitch, idThemeRepair, idSettings, idExit,
	} {
		action, _, ok := p.commandAction(id)
		if !ok {
			t.Fatalf("command ID %d has no action", id)
		}
		mapped[action] = true
	}
	for _, id := range []uint16{
		idNoSleep, idAutomationEnabled, idIdle, idTheme,
	} {
		action, ok := p.toggleCommand(id)
		if !ok {
			t.Fatalf("toggle ID %d has no action", id)
		}
		mapped[action] = true
	}

	for action := ActSleep; action <= ActIdleToggle; action++ {
		if !mapped[action] {
			t.Errorf("control panel action %d has no UI command path", action)
		}
	}
	for action := ActThemeToggle; action <= ActExit; action++ {
		if !mapped[action] {
			t.Errorf("control panel action %d has no UI command path", action)
		}
	}
}
