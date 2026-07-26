package automationpanel

import (
	"runtime"
	"testing"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/automation"
	"github.com/JeffioZ/idletrigger/internal/i18n"
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
	automationTestUser32         = windows.NewLazySystemDLL("user32.dll")
	automationTestGetWindowRect  = automationTestUser32.NewProc("GetWindowRect")
	automationTestGetClientRect  = automationTestUser32.NewProc("GetClientRect")
	automationTestClientToScreen = automationTestUser32.NewProc("ClientToScreen")
	automationTestEnumMonitors   = automationTestUser32.NewProc("EnumDisplayMonitors")
	automationTestGetMonitorInfo = automationTestUser32.NewProc("GetMonitorInfoW")
)

func TestChoiceFocusNotificationDoesNotClosePopup(t *testing.T) {
	const bnKillFocus = 7
	p := &panel{
		choices:    map[uint16]*choiceField{idAction: {}},
		choiceOpen: idAction,
	}
	p.handleEditor(idAction, bnKillFocus)
	if p.choiceOpen != idAction {
		t.Fatal("a choice button focus notification closed the popup")
	}
}

func TestEditorChoiceTriggerTogglesARealSharedPopup(t *testing.T) {
	requireNativeIntegration(t)
	err := Capture(State{}, func(key string) string { return key }, 1, false, true, func(hwnd windows.Handle) error {
		p := activePanelForTest(t, hwnd)
		p.openChoice(idAction)
		popup := p.choicePopup
		if p.choiceOpen != idAction || popup == nil || !popup.IsOpen() || popup.Window() == 0 {
			t.Fatal("editor choice trigger did not create the shared native popup")
		}
		p.toggleChoice(idAction)
		if p.choiceOpen != 0 || p.choicePopup != nil || popup.IsOpen() {
			t.Fatal("clicking the open editor choice trigger did not close its popup")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestEditorChoiceOwnerFocusBridgeStillClosesOnAnchorToggle(t *testing.T) {
	requireNativeIntegration(t)
	err := Capture(State{}, func(key string) string { return key }, 1, false, true, func(hwnd windows.Handle) error {
		p := activePanelForTest(t, hwnd)
		p.openChoice(idAction)
		popup := p.choicePopup
		if popup == nil || !popup.IsOpen() {
			t.Fatal("editor choice popup did not open")
		}
		// A real mouse click can move focus through the owner before the native
		// button receives BN_CLICKED. That bridge must not destroy the popup, or
		// the deferred toggle would interpret the click as a new open request.
		pSetFocus.Call(uintptr(hwnd))
		if p.choiceOpen != idAction || p.choicePopup != popup || !popup.IsOpen() {
			t.Fatal("focus moving through the owner closed the popup before the anchor toggle")
		}
		p.toggleChoice(idAction)
		if p.choiceOpen != 0 || p.choicePopup != nil || popup.IsOpen() {
			t.Fatal("clicking the open choice anchor reopened the popup instead of closing it")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestKeepScreenMouseClickDoesNotExposeAKeyboardFocusOutline(t *testing.T) {
	requireNativeIntegration(t)
	err := Capture(State{}, func(key string) string { return key }, 1, false, true, func(hwnd windows.Handle) error {
		p := activePanelForTest(t, hwnd)
		control := p.controls[idKeepScreen]
		if control == 0 {
			t.Fatal("editor keep-screen checkbox was not created")
		}
		p.interaction.SetFocusVisible(true)
		const wmLButtonUp = 0x0202
		const pointInside = uintptr(5 | (5 << 16))
		pSendMessage.Call(uintptr(control), wmLButtonDown, 1, pointInside)
		pSendMessage.Call(uintptr(control), wmLButtonUp, 0, pointInside)
		state := p.interaction.State(control)
		if !state.Focused {
			t.Fatal("mouse click did not leave the native checkbox focused; test cannot verify focus presentation")
		}
		if state.FocusVisible {
			t.Fatal("mouse-focused checkbox retained the keyboard-only focus outline")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestNewRuleOptionOnlyChangesDoNotRequireDiscardConfirmation(t *testing.T) {
	original := defaultRule()
	draft := original
	draft.KeepScreenOn = !draft.KeepScreenOn
	draft.IdleMinutes++
	draft.WarningSeconds++
	draft.BlockedPolicy = automation.BlockedWait
	draft.MaxWaitMinutes++
	if editorChangesRequireConfirmation(-1, original, draft) {
		t.Fatal("option-only changes on an otherwise untouched new rule should not prompt")
	}
	if !editorChangesRequireConfirmation(0, original, draft) {
		t.Fatal("the same changes to an existing rule must still prompt")
	}
}

func TestNewRuleStructuralChangesRequireDiscardConfirmation(t *testing.T) {
	original := defaultRule()
	tests := []struct {
		name   string
		mutate func(*automation.Rule)
	}{
		{"name", func(rule *automation.Rule) { rule.Name = "Night task" }},
		{"action", func(rule *automation.Rule) { rule.Action = automation.ActionShutdown }},
		{"time", func(rule *automation.Rule) { rule.Time = "23:00" }},
		{"process", func(rule *automation.Rule) {
			rule.Processes = []automation.ProcessTarget{{Match: automation.MatchName, Executable: "player.exe"}}
		}},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			draft := original
			test.mutate(&draft)
			if !editorChangesRequireConfirmation(-1, original, draft) {
				t.Fatal("meaningful new-rule work should prompt before discard")
			}
		})
	}
}

func TestSystemActionOffersProcessLifecycleTriggers(t *testing.T) {
	p := &panel{
		choices: map[uint16]*choiceField{},
		text:    func(key string) string { return key },
	}
	p.setTriggerOptions(automation.ActionShutdown, automation.TriggerProcessStarted)

	if got := p.triggerValue(); got != automation.TriggerProcessStarted {
		t.Fatalf("selected trigger = %q, want %q", got, automation.TriggerProcessStarted)
	}
	want := []automation.Trigger{
		automation.TriggerOnce,
		automation.TriggerDaily,
		automation.TriggerWeekly,
		automation.TriggerProcessStarted,
		automation.TriggerProcessExited,
	}
	if len(p.triggerOptions) != len(want) {
		t.Fatalf("trigger count = %d, want %d", len(p.triggerOptions), len(want))
	}
	for index := range want {
		if p.triggerOptions[index] != want[index] {
			t.Fatalf("trigger[%d] = %q, want %q", index, p.triggerOptions[index], want[index])
		}
	}
}

func TestAutomationTimeLabelDistinguishesExecutionAndWindowStart(t *testing.T) {
	for _, trigger := range []automation.Trigger{automation.TriggerOnce, automation.TriggerDaily, automation.TriggerWeekly} {
		if got := automationTimeLabelKey(trigger); got != "automation_execution_time" {
			t.Fatalf("%s time label = %q", trigger, got)
		}
	}
	if got := automationTimeLabelKey(automation.TriggerTimeWindow); got != "automation_time" {
		t.Fatalf("time-window label = %q", got)
	}
}

func TestTimeEditNormalizesToTimeGrammar(t *testing.T) {
	tests := []struct {
		name      string
		value     string
		final     bool
		wantValue string
		wantCaret int
	}{
		{name: "already formatted", value: "12:34", wantValue: "12:34", wantCaret: 5},
		{name: "separator inserted", value: "1234", wantValue: "12:34", wantCaret: 5},
		{name: "unexpected characters removed", value: "12a:3b4", wantValue: "12:34", wantCaret: 5},
		{name: "full width normalized", value: "１２：３４", wantValue: "12:34", wantCaret: 5},
		{name: "single hour padded on blur", value: "9:30", final: true, wantValue: "09:30", wantCaret: 5},
		{name: "leading separator ignored", value: ":12", wantValue: "12", wantCaret: 2},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			gotValue, gotCaret := normalizeTimeEditText(test.value, len([]rune(test.value)), test.final)
			if gotValue != test.wantValue || gotCaret != test.wantCaret {
				t.Fatalf("normalizeTimeEditText(%q) = %q, %d; want %q, %d", test.value, gotValue, gotCaret, test.wantValue, test.wantCaret)
			}
		})
	}
}

func TestTimeEditsRejectUnexpectedCharactersAndNormalizeOnBlur(t *testing.T) {
	requireNativeIntegration(t)
	err := Capture(State{}, func(key string) string { return key }, 1, false, true, func(hwnd windows.Handle) error {
		p := activePanelForTest(t, hwnd)
		p.setText(idTime, "")
		for _, value := range []rune{'１', '2', 'x', '3', '4', '5'} {
			pSendMessage.Call(uintptr(p.controls[idTime]), timeWMChar, uintptr(value), 0)
		}
		if got := p.controlText(idTime); got != "12:34" {
			t.Fatalf("time edit text = %q, want %q", got, "12:34")
		}

		p.setText(idTime, "9:30")
		pSendMessage.Call(uintptr(p.controls[idTime]), timeWMKillFocus, 0, 0)
		if got := p.controlText(idTime); got != "09:30" {
			t.Fatalf("time edit text after blur = %q, want %q", got, "09:30")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestEditorValidationKeepsLayoutAndVisibilityStable(t *testing.T) {
	requireNativeIntegration(t)
	err := Capture(State{}, func(key string) string { return key }, 1, false, true, func(hwnd windows.Handle) error {
		p := activePanelForTest(t, hwnd)
		beforeHeight := p.clientHeight
		beforeStatus := p.bounds[idValidation]
		beforeSave := p.bounds[idSave]

		p.beginRebuild()
		p.show(idSave, false)
		if visible, _, _ := pIsWindowVisible.Call(uintptr(p.controls[idSave])); visible == 0 {
			t.Fatal("rebuild exposed an intermediate hidden Save button")
		}
		p.show(idSave, true)
		p.endRebuild()

		p.saveEditor()
		if !p.editorStatusError {
			t.Fatal("invalid editor save did not enter the error status")
		}
		if p.clientHeight != beforeHeight || p.bounds[idValidation] != beforeStatus || p.bounds[idSave] != beforeSave {
			t.Fatalf("validation moved editor layout: height %d->%d status %+v->%+v save %+v->%+v", beforeHeight, p.clientHeight, beforeStatus, p.bounds[idValidation], beforeSave, p.bounds[idSave])
		}
		if visible, _, _ := pIsWindowVisible.Call(uintptr(p.controls[idSave])); visible == 0 {
			t.Fatal("Save button was not visible after validation")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestAutomationSpacingUsesSharedRhythm(t *testing.T) {
	requireNativeIntegration(t)
	err := Capture(State{}, func(key string) string { return key }, 1, false, false, func(hwnd windows.Handle) error {
		p := activePanelForTest(t, hwnd)
		title := p.bounds[idTitle]
		list := p.bounds[idListSurface]
		emptyTitle := p.bounds[idEmptyTitle]
		emptyBody := p.bounds[idEmptyBody]
		status := p.bounds[idNext]
		button := p.bounds[idNew]
		if got := list.Y - (title.Y + title.Height); got != formContentGap {
			t.Fatalf("manager title-to-list gap = %d, want %d", got, formContentGap)
		}
		if got := status.Y - (list.Y + list.Height); got != formRelatedGap {
			t.Fatalf("manager list-to-status gap = %d, want %d", got, formRelatedGap)
		}
		if got := button.Y - (status.Y + status.Height); got != formSectionGap {
			t.Fatalf("manager status-to-buttons gap = %d, want %d", got, formSectionGap)
		}
		if got := managerHeight - (button.Y + button.Height); got != formEdgePadding {
			t.Fatalf("manager bottom padding = %d, want %d", got, formEdgePadding)
		}
		if got := emptyBody.Y - (emptyTitle.Y + emptyTitle.Height); got != formContentGap {
			t.Fatalf("manager empty-state text gap = %d, want %d", got, formContentGap)
		}
		topSpace := emptyTitle.Y - list.Y
		bottomSpace := list.Y + list.Height - (emptyBody.Y + emptyBody.Height)
		if topSpace != bottomSpace {
			t.Fatalf("manager empty state is not vertically centered: top=%d bottom=%d", topSpace, bottomSpace)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}

	err = Capture(State{}, func(key string) string { return key }, 1, false, true, func(hwnd windows.Handle) error {
		p := activePanelForTest(t, hwnd)
		basics := p.bounds[idBasicsTitle]
		nameLabel := p.bounds[idNameLabel]
		name := p.bounds[idName]
		nameHint := p.bounds[idNameHint]
		actionLabel := p.bounds[idActionLabel]
		actionField := p.bounds[idAction]
		triggerTitle := p.bounds[idTriggerTitle]
		processLabel := p.bounds[idProcessLogicLabel]
		processField := p.bounds[idProcessLogic]
		processSummary := p.bounds[idProcessSummary]
		optionsTitle := p.bounds[idOptionsTitle]
		option := p.bounds[idKeepScreen]
		status := p.bounds[idValidation]
		button := p.bounds[idCancel]
		for name, value := range map[string]int{
			"basics-to-name-label":     nameLabel.Y - (basics.Y + basics.Height),
			"options-title-to-control": option.Y - (optionsTitle.Y + optionsTitle.Height),
		} {
			if value != formContentGap {
				t.Fatalf("editor %s gap = %d, want %d", name, value, formContentGap)
			}
		}
		if got := status.Y - (option.Y + option.Height); got != formRelatedGap {
			t.Fatalf("editor option-to-status gap = %d, want %d", got, formRelatedGap)
		}
		if got := button.Y - (status.Y + status.Height); got != formSectionGap {
			t.Fatalf("editor status-to-buttons gap = %d, want %d", got, formSectionGap)
		}
		for name, value := range map[string]int{
			"name-label-to-field":    name.Y - (nameLabel.Y + nameLabel.Height),
			"field-to-name-hint":     nameHint.Y - (name.Y + name.Height),
			"action-label-to-field":  actionField.Y - (actionLabel.Y + actionLabel.Height),
			"process-label-to-field": processField.Y - (processLabel.Y + processLabel.Height),
		} {
			if value != formLabelGap {
				t.Fatalf("editor %s gap = %d, want %d", name, value, formLabelGap)
			}
		}
		for name, value := range map[string]int{
			"hint-to-action-row":         actionLabel.Y - (nameHint.Y + nameHint.Height),
			"action-field-to-section":    triggerTitle.Y - (actionField.Y + actionField.Height),
			"process-summary-to-section": optionsTitle.Y - (processSummary.Y + processSummary.Height),
		} {
			if value != formSectionGap {
				t.Fatalf("editor %s gap = %d, want %d", name, value, formSectionGap)
			}
		}
		if got := processSummary.Y - (processField.Y + processField.Height); got != formRelatedGap {
			t.Fatalf("editor process-field-to-summary gap = %d, want %d", got, formRelatedGap)
		}
		if got := p.clientHeight - (button.Y + button.Height); got != formEdgePadding {
			t.Fatalf("editor bottom padding = %d, want %d", got, formEdgePadding)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestEditorVariantSpacingUsesSameTokens(t *testing.T) {
	requireNativeIntegration(t)
	captureRule := func(name string, rule automation.Rule, verify func(*panel)) {
		t.Helper()
		t.Run(name, func(t *testing.T) {
			err := Capture(State{Rules: []automation.Rule{rule}}, func(key string) string { return key }, 1, false, true, func(hwnd windows.Handle) error {
				verify(activePanelForTest(t, hwnd))
				return nil
			})
			if err != nil {
				t.Fatal(err)
			}
		})
	}

	once := defaultRule()
	once.Action = automation.ActionShutdown
	once.Trigger = automation.TriggerOnce
	once.Processes = nil
	captureRule("once-event-action", once, func(p *panel) {
		date := p.bounds[idDate]
		processLabel := p.bounds[idProcessLogicLabel]
		optionsTitle := p.bounds[idOptionsTitle]
		warningLabel := p.bounds[idWarningLabel]
		warning := p.bounds[idWarningSeconds]
		status := p.bounds[idValidation]
		if got := processLabel.Y - (date.Y + date.Height); got != formRelatedGap {
			t.Fatalf("date-to-process gap = %d, want %d", got, formRelatedGap)
		}
		if got := warningLabel.Y - (optionsTitle.Y + optionsTitle.Height); got != formContentGap {
			t.Fatalf("options-title-to-warning gap = %d, want %d", got, formContentGap)
		}
		if got := warning.Y - (warningLabel.Y + warningLabel.Height); got != formLabelGap {
			t.Fatalf("warning-label-to-field gap = %d, want %d", got, formLabelGap)
		}
		if got := status.Y - (warning.Y + warning.Height); got != formRelatedGap {
			t.Fatalf("warning-to-status gap = %d, want %d", got, formRelatedGap)
		}
	})

	window := defaultRule()
	window.Trigger = automation.TriggerTimeWindow
	captureRule("time-window-weekdays", window, func(p *panel) {
		timeField := p.bounds[idTime]
		quick := p.bounds[idDaysWorkdays]
		weekday := p.bounds[idWeekdayBase]
		processLabel := p.bounds[idProcessLogicLabel]
		for name, value := range map[string]int{
			"time-to-quick-row":           quick.Y - (timeField.Y + timeField.Height),
			"quick-row-to-weekdays":       weekday.Y - (quick.Y + quick.Height),
			"weekdays-to-process-section": processLabel.Y - (weekday.Y + weekday.Height),
		} {
			if value != formRelatedGap {
				t.Fatalf("%s gap = %d, want %d", name, value, formRelatedGap)
			}
		}
	})

	blocked := defaultRule()
	blocked.Action = automation.ActionShutdown
	blocked.Trigger = automation.TriggerOnce
	blocked.Processes = []automation.ProcessTarget{{Match: automation.MatchName, Executable: "player.exe"}}
	blocked.BlockedPolicy = automation.BlockedWait
	captureRule("blocked-event-action", blocked, func(p *panel) {
		warning := p.bounds[idWarningSeconds]
		blockedLabel := p.bounds[idBlockedLabel]
		blockedField := p.bounds[idBlockedPolicy]
		status := p.bounds[idValidation]
		if got := blockedLabel.Y - (warning.Y + warning.Height); got != formRelatedGap {
			t.Fatalf("warning-to-blocked-policy gap = %d, want %d", got, formRelatedGap)
		}
		if got := blockedField.Y - (blockedLabel.Y + blockedLabel.Height); got != formLabelGap {
			t.Fatalf("blocked-label-to-field gap = %d, want %d", got, formLabelGap)
		}
		if got := status.Y - (blockedField.Y + blockedField.Height); got != formRelatedGap {
			t.Fatalf("blocked-policy-to-status gap = %d, want %d", got, formRelatedGap)
		}
	})
}

func TestRuleSummaryIncludesEffectiveDays(t *testing.T) {
	english := &panel{text: func(key string) string { return i18n.T("en", key) }}
	weekly := automation.Rule{Action: automation.ActionShutdown, Trigger: automation.TriggerWeekly, Time: "23:00", Days: []string{"wed", "mon"}}
	if got, want := english.ruleSummary(weekly), "Shut Down · Mon, Wed at 23:00"; got != want {
		t.Fatalf("weekly summary = %q, want %q", got, want)
	}
	chinese := &panel{state: State{Chinese: true}, text: func(key string) string { return i18n.T("zh-CN", key) }}
	window := automation.Rule{Action: automation.ActionStayAwake, Trigger: automation.TriggerTimeWindow, Time: "08:00", EndTime: "18:00", Days: []string{"mon", "tue", "wed", "thu", "fri"}}
	if got, want := chinese.ruleSummary(window), "启用保持唤醒 · 08:00–18:00 · 工作日"; got != want {
		t.Fatalf("time-window summary = %q, want %q", got, want)
	}
}

func requireNativeIntegration(t *testing.T) {
	t.Helper()
	if testing.Short() {
		t.Skip("skipping native Win32 integration test in short mode")
	}
}

func TestProcessTooltipBufferRemainsBoundedAcrossRefreshes(t *testing.T) {
	requireNativeIntegration(t)
	err := Capture(State{}, func(key string) string { return key }, 1, false, true, func(hwnd windows.Handle) error {
		p := activePanelForTest(t, hwnd)
		p.draft.Processes = []automation.ProcessTarget{{Match: automation.MatchName, Executable: "player.exe"}}
		p.refreshProcessInfoTooltip()
		before := len(p.tooltipText)
		if before == 0 {
			t.Fatal("editor created no tooltip buffers")
		}
		for range 100 {
			p.refreshProcessInfoTooltip()
		}
		if after := len(p.tooltipText); after != before {
			t.Fatalf("tooltip buffers grew across refreshes: before=%d after=%d", before, after)
		}
		if len(p.tooltipText[idProcessInfo]) == 0 {
			t.Fatal("process tooltip buffer was not retained")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestManagerUsesAuthoritativeStateWhenSaveIsRejected(t *testing.T) {
	latest := State{Revision: "new", Rules: []automation.Rule{{ID: "latest"}}}
	p := &panel{
		view:   managerView,
		state:  State{Revision: "old", Rules: []automation.Rule{{ID: "old"}}},
		rules:  []automation.Rule{{ID: "old"}},
		onSave: func(SaveRequest) SaveResult { return SaveResult{State: latest, Error: "conflict"} },
	}
	if ok, _ := p.notifySave([]automation.Rule{{ID: "stale"}}); ok {
		t.Fatal("rejected save reported success")
	}
	if p.state.Revision != "new" || len(p.rules) != 1 || p.rules[0].ID != "latest" || p.managerNotice != "conflict" {
		t.Fatalf("manager state = %+v, rules = %+v", p.state, p.rules)
	}
}

func TestEditorKeepsDraftBaseWhenExternalStateArrives(t *testing.T) {
	latest := State{Revision: "new", Rules: []automation.Rule{{ID: "latest"}}}
	p := &panel{
		view:   editorView,
		state:  State{Revision: "old", Rules: []automation.Rule{{ID: "old"}}},
		rules:  []automation.Rule{{ID: "old"}},
		onSave: func(SaveRequest) SaveResult { return SaveResult{State: latest, Error: "conflict"} },
	}
	if ok, _ := p.notifySave([]automation.Rule{{ID: "draft"}}); ok {
		t.Fatal("rejected save reported success")
	}
	if p.state.Revision != "old" || p.pendingState == nil || p.pendingState.Revision != "new" {
		t.Fatalf("editor state = %+v, pending = %+v", p.state, p.pendingState)
	}
}

func TestAutomationWindowsRemainOperableAcrossDPIAndSmallWorkArea(t *testing.T) {
	requireNativeIntegration(t)
	work := nativeform.Rect{Right: 1366, Bottom: 768}
	for _, scale := range []float64{1, 1.25, 1.5, 2} {
		for _, editor := range []bool{false, true} {
			name := "manager"
			controls := []uint16{idNew, idEdit, idDelete, idToggle}
			if editor {
				name = "editor"
				controls = []uint16{idCancel, idSave}
			}
			t.Run(name+"-"+formatTestScale(scale), func(t *testing.T) {
				err := Capture(State{}, func(key string) string { return key }, scale, false, editor, func(hwnd windows.Handle) error {
					p := activePanelForTest(t, hwnd)
					p.resizeInWorkArea(p.clientWidth, p.clientHeight, &work)
					if p.layoutErr != nil {
						t.Fatalf("small-work-area layout: %v", p.layoutErr)
					}
					assertRectInside(t, windowRectForTest(t, hwnd), work)
					p.scrollContentTo(max(0, p.clientHeight-p.viewportHeight))
					client := clientScreenRectForTest(t, hwnd)
					for _, id := range controls {
						assertRectInside(t, windowRectForTest(t, p.controls[id]), client)
					}
					return nil
				})
				if err != nil {
					t.Fatal(err)
				}
			})
		}
	}
}

func TestManagerLayoutFitsCICompactViewport(t *testing.T) {
	const viewportWidth = 514
	contentWidth, standardWidth, toggleWidth := managerLayoutWidths(viewportWidth, true)
	right := managerPad + 3*standardWidth + 3*managerButtonGap + toggleWidth
	wantRight := managerPad + contentWidth
	if right != wantRight {
		t.Fatalf("compact button row right edge = %d, want content edge %d", right, wantRight)
	}
	limit := viewportWidth - managerPad - nativeform.ScrollbarWidth - 4
	if right > limit {
		t.Fatalf("compact button row right edge = %d, viewport limit %d", right, limit)
	}

	contentWidth, standardWidth, toggleWidth = managerLayoutWidths(managerWidth, false)
	if contentWidth != 564 || standardWidth != 116 || toggleWidth != 192 {
		t.Fatalf("normal manager layout = content %d standard %d toggle %d", contentWidth, standardWidth, toggleWidth)
	}
}

func TestAutomationWindowsApplySuggestedRectAcrossDPIChanges(t *testing.T) {
	requireNativeIntegration(t)
	for _, editor := range []bool{false, true} {
		t.Run(map[bool]string{false: "manager", true: "editor"}[editor], func(t *testing.T) {
			err := Capture(State{}, func(key string) string { return key }, 1, false, editor, func(hwnd windows.Handle) error {
				p := activePanelForTest(t, hwnd)
				p.captureScale = 0
				retainedID := uint16(idNew)
				if editor {
					retainedID = idName
					p.setText(idName, "unsaved DPI draft")
				}
				retainedHandle := p.controls[retainedID]
				workAreas := monitorWorkAreasForTest(t)
				for index, dpi := range []uint32{96, 120, 144, 192, 120} {
					work := workAreas[index%len(workAreas)]
					scale := float64(dpi) / 96
					width, height, err := nativeform.WindowSizeForClient(
						int(float64(p.clientWidth)*scale+0.5), int(float64(p.clientHeight)*scale+0.5),
						p.style, p.exStyle, dpi,
					)
					if err != nil {
						t.Fatal(err)
					}
					suggested := nativeform.Rect{Left: work.Left + 13, Top: work.Top + 17, Right: work.Left + 13 + width, Bottom: work.Top + 17 + height}
					pSendMessage.Call(uintptr(hwnd), wmDpiChanged, uintptr(dpi|(dpi<<16)), uintptr(unsafe.Pointer(&suggested)))
					if p.dpiScale != scale {
						t.Fatalf("DPI scale = %.2f, want %.2f", p.dpiScale, scale)
					}
					want := nativeform.ConstrainRect(suggested, work)
					if got := windowRectForTest(t, hwnd); got != want {
						t.Fatalf("window rect after %d DPI = %+v, want suggested/clamped %+v", dpi, got, want)
					}
					if p.controls[retainedID] != retainedHandle {
						t.Fatalf("control %d HWND changed after %d DPI", retainedID, dpi)
					}
					if editor && p.controlText(idName) != "unsaved DPI draft" {
						t.Fatalf("unsaved editor text changed after %d DPI", dpi)
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

func TestAutomationWindowsReleaseResourcesAcrossRepresentativeCycles(t *testing.T) {
	requireNativeIntegration(t)
	const (
		stabilizationCycles = 8
		measuredCycles      = 8
	)
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	text := func(key string) string { return key }
	for range stabilizationCycles {
		if err := Capture(State{}, text, 1, false, true, nil); err != nil {
			t.Fatal(err)
		}
	}
	before, err := wintest.StableResources()
	if err != nil {
		t.Fatal(err)
	}
	runMeasuredCycles := func() {
		for index := 0; index < measuredCycles; index++ {
			if err := Capture(State{}, text, 1, index%2 == 0, index%2 != 0, nil); err != nil {
				t.Fatalf("cycle %d: %v", index+1, err)
			}
		}
	}
	runMeasuredCycles()
	after, err := wintest.StableResources()
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("automation resources across %d cycles before=%+v after=%+v", measuredCycles, before, after)
	if after.GDI > before.GDI || after.USER > before.USER {
		runMeasuredCycles()
		repeated, repeatErr := wintest.StableResources()
		if repeatErr != nil {
			t.Fatal(repeatErr)
		}
		if repeated.GDI > after.GDI || repeated.USER > after.USER {
			t.Fatalf("GUI resources kept growing across repeated automation-window cycles: before=%+v after=%+v repeated=%+v", before, after, repeated)
		}
		t.Logf("automation cycles initialized stable process resources: before=%+v after=%+v repeated=%+v", before, after, repeated)
	}
}

func activePanelForTest(t *testing.T, hwnd windows.Handle) *panel {
	t.Helper()
	activeMu.Lock()
	p := active
	activeMu.Unlock()
	if p == nil || p.hwnd != hwnd {
		t.Fatalf("active automation panel = %p hwnd=%v, want hwnd=%v", p, func() windows.Handle {
			if p != nil {
				return p.hwnd
			}
			return 0
		}(), hwnd)
	}
	return p
}

func formatTestScale(scale float64) string {
	return map[float64]string{1: "100", 1.25: "125", 1.5: "150", 2: "200"}[scale]
}

func windowRectForTest(t *testing.T, hwnd windows.Handle) nativeform.Rect {
	t.Helper()
	var value nativeform.Rect
	if ok, _, callErr := automationTestGetWindowRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&value))); ok == 0 {
		t.Fatalf("GetWindowRect(%v): %v", hwnd, callErr)
	}
	return value
}

func clientScreenRectForTest(t *testing.T, hwnd windows.Handle) nativeform.Rect {
	t.Helper()
	var value nativeform.Rect
	if ok, _, callErr := automationTestGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&value))); ok == 0 {
		t.Fatalf("GetClientRect(%v): %v", hwnd, callErr)
	}
	topLeft := point{X: value.Left, Y: value.Top}
	bottomRight := point{X: value.Right, Y: value.Bottom}
	automationTestClientToScreen.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&topLeft)))
	automationTestClientToScreen.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&bottomRight)))
	return nativeform.Rect{Left: topLeft.X, Top: topLeft.Y, Right: bottomRight.X, Bottom: bottomRight.Y}
}

func monitorWorkAreasForTest(t *testing.T) []nativeform.Rect {
	t.Helper()
	var values []nativeform.Rect
	callback := windows.NewCallback(func(monitor, hdc, clip, data uintptr) uintptr {
		type info struct {
			Size          uint32
			Monitor, Work nativeform.Rect
			Flags         uint32
		}
		value := info{Size: uint32(unsafe.Sizeof(info{}))}
		if ok, _, _ := automationTestGetMonitorInfo.Call(monitor, uintptr(unsafe.Pointer(&value))); ok != 0 {
			values = append(values, value.Work)
		}
		return 1
	})
	if ok, _, callErr := automationTestEnumMonitors.Call(0, 0, callback, 0); ok == 0 {
		t.Fatalf("EnumDisplayMonitors: %v", callErr)
	}
	if len(values) == 0 {
		t.Fatal("EnumDisplayMonitors returned no work areas")
	}
	t.Logf("exercising WM_DPICHANGED across %d monitor work area(s)", len(values))
	return values
}

func assertRectInside(t *testing.T, inner, outer nativeform.Rect) {
	t.Helper()
	if inner.Left < outer.Left || inner.Top < outer.Top || inner.Right > outer.Right || inner.Bottom > outer.Bottom {
		t.Fatalf("rect %+v escapes %+v", inner, outer)
	}
}
