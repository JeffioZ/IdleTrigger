package automationpanel

import (
	"fmt"
	"reflect"
	"strconv"
	"strings"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/automation"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/processcatalog"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"github.com/JeffioZ/idletrigger/internal/ui/processpicker"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

func (p *panel) showEditorDraft(index int, draft automation.Rule) {
	p.beginRebuild()
	defer p.endRebuild()
	p.closeChoice(false)
	p.hideControls(managerControlIDs())
	if p.managerScroll != nil {
		p.managerScroll.SetActive(false)
	}
	if p.view != editorView {
		p.contentOffset = 0
	}
	p.view = editorView
	p.editing = index
	p.draft = draft
	p.originalDraft = draft
	p.processDescriptions = make(map[string]string)
	if index >= 0 {
		p.setCaption(p.t("automation_edit_title"))
	} else {
		p.setCaption(p.t("automation_new_title"))
	}
	if !p.editorReady {
		p.createEditorControls()
	}
	p.setText(idName, p.draft.Name)
	p.setText(idDate, p.draft.Date)
	p.setText(idTime, p.draft.Time)
	p.setText(idEndTime, p.draft.EndTime)
	p.setText(idIdleMinutes, strconv.Itoa(p.draft.IdleMinutes))
	p.setText(idWarningSeconds, strconv.Itoa(p.draft.WarningSeconds))
	p.setText(idMaxWait, strconv.Itoa(p.draft.MaxWaitMinutes))
	status := p.t("automation_runtime_note")
	p.editorStatusError = false
	if issue, invalid := p.issueForRule(index); invalid {
		status = issue.Message
		p.editorStatusError = true
	}
	p.setText(idValidation, status)
	for dayIndex, day := range editorWeekdays {
		p.setChecked(idWeekdayBase+uint16(dayIndex), containsDay(p.draft.Days, day))
	}
	p.setChecked(idKeepScreen, p.draft.KeepScreenOn)
	p.setChoiceOptions(idAction, actionLabels(p.text))
	p.setChoiceOptions(idProcessLogic, processLabels(p.text))
	p.setChoiceOptions(idBlockedPolicy, blockedLabels(p.text))
	p.selectCombo(idAction, actionIndex(p.draft.Action))
	p.setTriggerOptions(p.draft.Action, p.draft.Trigger)
	p.selectCombo(idProcessLogic, processIndex(p.draft.ProcessLogic))
	p.selectCombo(idBlockedPolicy, blockedIndex(p.draft.BlockedPolicy))
	p.layoutEditor()
	p.refreshProcessInfoTooltip()
	p.loadProcessDescriptions()
}

func (p *panel) createEditorControls() {
	p.namedLabel(idBasicsTitle, p.t("automation_basics"), p.sectionFont)
	p.namedLabel(idNameLabel, p.t("automation_name"), p.font)
	p.edit(idName, "")
	p.namedLabel(idNameHint, p.t("automation_name_hint"), p.font)
	p.namedLabel(idActionLabel, p.t("automation_action"), p.font)
	p.combo(idAction, 0, 0, 314, actionLabels(p.text))
	p.namedLabel(idTriggerLabel, p.t("automation_trigger"), p.font)
	p.combo(idTrigger, 0, 0, 314, nil)
	p.namedLabel(idTriggerTitle, p.t("automation_trigger_conditions"), p.sectionFont)
	p.namedLabel(idDateLabel, p.t("automation_date"), p.font)
	p.edit(idDate, "")
	p.namedLabel(idTimeLabel, p.t("automation_time"), p.font)
	p.timeEdit(idTime, "")
	p.namedLabel(idEndTimeLabel, p.t("automation_end_time"), p.font)
	p.timeEdit(idEndTime, "")
	p.namedLabel(idDaysLabel, p.t("automation_days"), p.font)
	p.child("BUTTON", p.t("automation_days_workdays"), wsChild|wsTabStop|bsOwnerDraw, 0, 0, 1, 1, idDaysWorkdays, p.font)
	p.child("BUTTON", p.t("automation_days_everyday"), wsChild|wsTabStop|bsOwnerDraw, 0, 0, 1, 1, idDaysEveryday, p.font)
	for index, day := range editorWeekdays {
		style := uintptr(wsChild | bsOwnerDraw)
		if index == 0 {
			style |= wsTabStop
		}
		button := p.child("BUTTON", p.t("automation_day_"+day), style, 0, 0, 1, 1, idWeekdayBase+uint16(index), p.font)
		pSetWindowSubclass.Call(uintptr(button), weekdayCallback, weekdaySubclassID, 0)
	}
	p.namedLabel(idProcessLogicLabel, p.t("automation_process_logic"), p.font)
	p.combo(idProcessLogic, 0, 0, 314, processLabels(p.text))
	p.child("BUTTON", p.t("automation_choose_processes"), wsChild|wsTabStop|bsOwnerDraw, 0, 0, 1, 1, idChooseProcesses, p.font)
	p.child("STATIC", "", wsChild|ssLeft, 0, 0, 1, 1, idProcessSummary, p.font)
	p.child("BUTTON", "i", wsChild|wsTabStop|bsOwnerDraw, 0, 0, 1, 1, idProcessInfo, p.font)
	p.namedLabel(idOptionsTitle, p.t("automation_action_options"), p.sectionFont)
	p.child("BUTTON", p.t("automation_keep_screen"), wsChild|wsTabStop|bsOwnerDraw, 0, 0, 1, 1, idKeepScreen, p.font)
	p.namedLabel(idIdleMinutesLabel, p.t("automation_idle_minutes"), p.font)
	p.edit(idIdleMinutes, "")
	p.namedLabel(idWarningLabel, p.t("automation_warning_seconds"), p.font)
	p.edit(idWarningSeconds, "")
	p.namedLabel(idBlockedLabel, p.t("automation_blocked_policy"), p.font)
	p.combo(idBlockedPolicy, 0, 0, 314, blockedLabels(p.text))
	p.namedLabel(idMaxWaitLabel, p.t("automation_max_wait"), p.font)
	p.edit(idMaxWait, "")
	p.namedLabel(idNoOptions, p.t("automation_no_action_options"), p.font)
	p.namedLabel(idValidation, p.t("automation_runtime_note"), p.font)
	p.child("BUTTON", p.t("common_cancel"), wsChild|wsTabStop|bsOwnerDraw, 0, 0, 1, 1, idCancel, p.font)
	p.child("BUTTON", p.t("common_save"), wsChild|wsTabStop|bsOwnerDraw, 0, 0, 1, 1, idSave, p.font)
	for id, key := range map[uint16]string{idName: "tip_automation_name", idAction: "tip_automation_action", idTrigger: "tip_automation_trigger", idDate: "tip_automation_date", idTime: "tip_automation_time", idEndTime: "tip_automation_end_time", idProcessLogic: "tip_process_logic", idChooseProcesses: "tip_choose_processes", idKeepScreen: "tip_keep_screen", idIdleMinutes: "tip_idle_minutes", idWarningSeconds: "tip_warning_seconds", idBlockedPolicy: "tip_blocked_policy", idMaxWait: "tip_max_wait", idSave: "tip_automation_save", idCancel: "tip_automation_cancel"} {
		p.addTooltip(id, key)
	}
	for index := range editorWeekdays {
		p.addTooltip(idWeekdayBase+uint16(index), "tip_automation_days")
	}
	p.addTooltip(idDaysWorkdays, "tip_automation_days_workdays")
	p.addTooltip(idDaysEveryday, "tip_automation_days_everyday")
	p.editorReady = true
}

func defaultRule() automation.Rule {
	now := time.Now()
	return automation.Rule{ID: fmt.Sprintf("rule-%x", now.UnixNano()), Name: "", Enabled: true, Action: automation.ActionStayAwake, Trigger: automation.TriggerProcessRunning, Date: now.Format("2006-01-02"), Time: now.Add(time.Hour).Format("15:04"), EndTime: now.Add(2 * time.Hour).Format("15:04"), Days: []string{"mon", "tue", "wed", "thu", "fri"}, ProcessLogic: automation.ProcessAny, IdleMinutes: automation.DefaultIdleMinutes, WarningSeconds: automation.DefaultWarningSeconds, BlockedPolicy: automation.BlockedSkip, MaxWaitMinutes: 60}
}

func (p *panel) saveEditor() {
	p.syncDraft()
	if id, message := p.validateDraft(); message != "" {
		p.setEditorError(id, message)
		return
	}
	if p.draft.Name == "" {
		p.draft.Name = p.ruleSummary(p.draft)
	}
	candidate := append([]automation.Rule(nil), p.rules...)
	if p.editing >= 0 && p.editing < len(candidate) {
		candidate[p.editing] = p.draft
	} else {
		candidate = append(candidate, p.draft)
	}
	normalized, issues := automation.PrepareRules(candidate)
	if len(issues) > 0 {
		p.setEditorError(idSave, issues[0].Message)
		return
	}
	p.selectedRuleID = p.draft.ID
	if ok, message := p.notifySave(normalized); !ok {
		p.setEditorError(idSave, message)
		return
	}
	p.showManager()
}

func (p *panel) syncDraft() {
	p.draft.Name = cleanSingleLine(p.controlText(idName))
	p.draft.Action = actionAt(p.comboIndex(idAction))
	p.draft.Trigger = p.triggerValue()
	p.draft.ProcessLogic = processAt(p.comboIndex(idProcessLogic))
	p.draft.BlockedPolicy = blockedAt(p.comboIndex(idBlockedPolicy))
	p.draft.Date = cleanSingleLine(p.controlText(idDate))
	p.draft.Time = cleanSingleLine(p.controlText(idTime))
	p.draft.EndTime = cleanSingleLine(p.controlText(idEndTime))
	p.draft.Days = nil
	for index, day := range editorWeekdays {
		if p.checked(idWeekdayBase + uint16(index)) {
			p.draft.Days = append(p.draft.Days, day)
		}
	}
	p.draft.KeepScreenOn = p.checked(idKeepScreen)
	p.draft.IdleMinutes, _ = strconv.Atoi(cleanSingleLine(p.controlText(idIdleMinutes)))
	p.draft.WarningSeconds, _ = strconv.Atoi(cleanSingleLine(p.controlText(idWarningSeconds)))
	p.draft.MaxWaitMinutes, _ = strconv.Atoi(cleanSingleLine(p.controlText(idMaxWait)))
}

func cleanSingleLine(value string) string {
	value = strings.ReplaceAll(value, "\r", " ")
	value = strings.ReplaceAll(value, "\n", " ")
	return strings.TrimSpace(value)
}

func (p *panel) setTriggerOptions(action automation.Action, desired automation.Trigger) {
	if automation.IsStateAction(action) {
		p.triggerOptions = []automation.Trigger{automation.TriggerProcessRunning, automation.TriggerTimeWindow}
	} else {
		p.triggerOptions = []automation.Trigger{automation.TriggerOnce, automation.TriggerDaily, automation.TriggerWeekly, automation.TriggerProcessStarted, automation.TriggerProcessExited}
	}
	selected := 0
	labels := make([]string, 0, len(p.triggerOptions))
	for index, trigger := range p.triggerOptions {
		labels = append(labels, p.t(triggerKey(trigger)))
		if trigger == desired {
			selected = index
		}
	}
	p.setChoiceOptions(idTrigger, labels)
	p.selectCombo(idTrigger, selected)
}

func (p *panel) triggerValue() automation.Trigger {
	index := p.comboIndex(idTrigger)
	if index < 0 || index >= len(p.triggerOptions) {
		return p.triggerOptions[0]
	}
	return p.triggerOptions[index]
}

func (p *panel) layoutEditor() {
	if p.layoutingEditor {
		return
	}
	p.layoutingEditor = true
	defer func() { p.layoutingEditor = false }()
	suggested := p.pendingSuggested
	p.pendingSuggested = nil
	layoutWidth := min(editorWidth, max(1, p.viewportWidth))
	for range 3 {
		height := p.layoutEditorContent(layoutWidth)
		p.resize(editorWidth, height)
		nextWidth := min(editorWidth, max(1, p.viewportWidth))
		if nextWidth == layoutWidth || p.layoutErr != nil {
			break
		}
		layoutWidth = nextWidth
	}
	if suggested != nil && p.layoutErr == nil {
		p.pendingSuggested = suggested
		p.resize(editorWidth, p.clientHeight)
	}
	p.setText(idProcessSummary, p.processSummary())
}

func (p *panel) layoutEditorContent(layoutWidth int) int {
	p.closeChoice(false)
	action := actionAt(p.comboIndex(idAction))
	trigger := p.triggerValue()
	p.setText(idTimeLabel, p.t(automationTimeLabelKey(trigger)))
	const pad, gap, fieldH, labelH = nativeform.FormPadding, nativeform.ControlGap, nativeform.FieldHeight, formTextHeight
	reserve := 0
	if p.clientHeight > p.viewportHeight {
		reserve = nativeform.ScrollbarWidth + 4
	}
	contentW := max(1, layoutWidth-2*pad-reserve)
	columnW := (contentW - gap) / 2
	for _, id := range editorControlIDs() {
		p.show(id, false)
	}
	y := formEdgePadding
	p.place(idBasicsTitle, pad, y, contentW, labelH, true)
	y += labelH + formContentGap
	p.place(idNameLabel, pad, y, contentW, labelH, true)
	y += labelH + formLabelGap
	p.place(idName, pad, y, contentW, fieldH, true)
	y += fieldH + formLabelGap
	p.place(idNameHint, pad, y, contentW, labelH, true)
	y += labelH + formSectionGap
	p.place(idActionLabel, pad, y, columnW, labelH, true)
	p.place(idTriggerLabel, pad+columnW+gap, y, columnW, labelH, true)
	y += labelH + formLabelGap
	p.placeCombo(idAction, pad, y, columnW, true)
	p.placeCombo(idTrigger, pad+columnW+gap, y, columnW, true)
	y += fieldH + formSectionGap
	p.place(idTriggerTitle, pad, y, contentW, labelH, true)
	y += labelH + formContentGap
	rowFields := func(leftLabel, leftControl, rightLabel, rightControl uint16) {
		p.place(leftLabel, pad, y, columnW, labelH, true)
		p.place(rightLabel, pad+columnW+gap, y, columnW, labelH, true)
		p.place(leftControl, pad, y+labelH+formLabelGap, columnW, fieldH, true)
		p.place(rightControl, pad+columnW+gap, y+labelH+formLabelGap, columnW, fieldH, true)
		y += labelH + formLabelGap + fieldH + formRelatedGap
	}
	switch trigger {
	case automation.TriggerOnce:
		rowFields(idDateLabel, idDate, idTimeLabel, idTime)
	case automation.TriggerDaily:
		p.place(idTimeLabel, pad, y, columnW, labelH, true)
		p.place(idTime, pad, y+labelH+formLabelGap, columnW, fieldH, true)
		y += labelH + formLabelGap + fieldH + formRelatedGap
	case automation.TriggerWeekly:
		p.place(idTimeLabel, pad, y, columnW, labelH, true)
		p.place(idTime, pad, y+labelH+formLabelGap, columnW, fieldH, true)
		y += labelH + formLabelGap + fieldH + formRelatedGap
		y = p.layoutWeekdays(y, contentW)
	case automation.TriggerTimeWindow:
		rowFields(idTimeLabel, idTime, idEndTimeLabel, idEndTime)
		y = p.layoutWeekdays(y, contentW)
	}

	processRequired := trigger == automation.TriggerProcessRunning || trigger == automation.TriggerProcessStarted || trigger == automation.TriggerProcessExited
	p.setText(idProcessLogicLabel, p.t("automation_optional_process"))
	if processRequired {
		p.setText(idProcessLogicLabel, p.t("automation_process_condition"))
	}
	p.place(idProcessLogicLabel, pad, y, contentW, labelH, true)
	y += labelH + formLabelGap
	switch trigger {
	case automation.TriggerProcessExited:
		p.draft.ProcessLogic = automation.ProcessNone
		p.selectCombo(idProcessLogic, processIndex(automation.ProcessNone))
		p.place(idChooseProcesses, pad, y, contentW, fieldH, true)
	case automation.TriggerProcessStarted:
		p.draft.ProcessLogic = automation.ProcessAny
		p.selectCombo(idProcessLogic, processIndex(automation.ProcessAny))
		p.place(idChooseProcesses, pad, y, contentW, fieldH, true)
	default:
		p.placeCombo(idProcessLogic, pad, y, columnW, true)
		p.place(idChooseProcesses, pad+columnW+gap, y, columnW, fieldH, true)
	}
	y += fieldH + formRelatedGap
	p.place(idProcessSummary, pad, y, contentW, labelH, true)
	showInfo := len(p.draft.Processes) > 0
	if showInfo {
		p.place(idProcessSummary, pad, y, contentW-30, labelH, true)
	}
	p.place(idProcessInfo, pad+contentW-processSummaryRowH, y, processSummaryRowH, processSummaryRowH, showInfo)
	y += processSummaryRowH + formContentGap

	p.place(idOptionsTitle, pad, y, contentW, labelH, true)
	y += labelH + formContentGap
	switch action {
	case automation.ActionStayAwake:
		p.place(idKeepScreen, pad, y, contentW, checkboxRowHeight, true)
		y += checkboxRowHeight
	case automation.ActionEnableIdle:
		p.place(idIdleMinutesLabel, pad, y, columnW, labelH, true)
		p.place(idIdleMinutes, pad, y+labelH+formLabelGap, columnW, fieldH, true)
		y += labelH + formLabelGap + fieldH
	case automation.ActionPauseStayAwake, automation.ActionPauseIdle:
		p.place(idNoOptions, pad, y, contentW, labelH, true)
		y += labelH
	default:
		p.place(idWarningLabel, pad, y, columnW, labelH, true)
		p.place(idWarningSeconds, pad, y+labelH+formLabelGap, columnW, fieldH, true)
		y += labelH + formLabelGap + fieldH
		if len(p.draft.Processes) > 0 && trigger != automation.TriggerProcessStarted && trigger != automation.TriggerProcessExited {
			y += formRelatedGap
			p.place(idBlockedLabel, pad, y, columnW, labelH, true)
			p.placeCombo(idBlockedPolicy, pad, y+labelH+formLabelGap, columnW, true)
			if blockedAt(p.comboIndex(idBlockedPolicy)) == automation.BlockedWait {
				p.place(idMaxWaitLabel, pad+columnW+gap, y, columnW, labelH, true)
				p.place(idMaxWait, pad+columnW+gap, y+labelH+formLabelGap, columnW, fieldH, true)
			}
			y += labelH + formLabelGap + fieldH
		}
	}
	// The status row belongs to the option above it, while the buttons form a
	// separate footer. Keep its bounds stable in both normal and error states so
	// validation feedback never resizes or rebuilds the editor after Save.
	y += formRelatedGap
	p.place(idValidation, pad, y, contentW, labelH, true)
	y += labelH + formSectionGap
	p.place(idCancel, pad+contentW-220, y, 102, nativeform.ButtonHeight, true)
	p.place(idSave, pad+contentW-110, y, 110, nativeform.ButtonHeight, true)
	y += nativeform.ButtonHeight + formEdgePadding
	return y
}

func (p *panel) layoutWeekdays(y, contentW int) int {
	const pad, gap = nativeform.FormPadding, nativeform.ControlGap
	buttonW := (contentW - gap*(len(editorWeekdays)-1)) / len(editorWeekdays)
	const quickW, quickH = 88, 24
	p.place(idDaysLabel, pad, y+(quickH-formTextHeight)/2, contentW-2*(quickW+gap), formTextHeight, true)
	p.place(idDaysWorkdays, pad+contentW-2*quickW-gap, y, quickW, quickH, true)
	p.place(idDaysEveryday, pad+contentW-quickW, y, quickW, quickH, true)
	y += quickH + formRelatedGap
	for index := range editorWeekdays {
		p.place(idWeekdayBase+uint16(index), pad+index*(buttonW+gap), y, buttonW, nativeform.FieldHeight, true)
	}
	return y + nativeform.FieldHeight + formRelatedGap
}

func (p *panel) handleCommand(id, notification uint16) {
	if notification == bnSetFocus || notification == enSetFocus || notification == lbnSetFocus {
		p.ensureControlVisible(id)
	}
	if p.view == managerView {
		p.handleManager(id, notification)
	} else {
		p.handleEditor(id, notification)
	}
}
func (p *panel) handleManager(id, notification uint16) {
	switch id {
	case idList:
		switch notification {
		case lbnDblClk:
			p.editSelected()
		case lbnSelChange:
			if index := p.selectedRule(); index >= 0 {
				p.selectedRuleID = p.rules[index].ID
			}
			p.updateManagerActions()
		}
	case idNew:
		if ownerDrawButtonClicked(notification) {
			p.showEditor(-1)
		}
	case idEdit:
		if ownerDrawButtonClicked(notification) {
			p.editSelected()
		}
	case idToggle:
		if ownerDrawButtonClicked(notification) {
			if index := p.selectedRule(); index >= 0 {
				candidate := append([]automation.Rule(nil), p.rules...)
				candidate[index].Enabled = !candidate[index].Enabled
				if ok, message := p.notifySave(candidate); !ok {
					p.setText(idNext, message)
				}
				p.populateRules()
			}
		}
	case idDelete:
		if ownerDrawButtonClicked(notification) {
			if index := p.selectedRule(); index >= 0 {
				if !p.confirm(p.t("automation_delete_title"), fmt.Sprintf(p.t("automation_delete_confirm"), p.rules[index].Name)) {
					return
				}
				candidate := append([]automation.Rule(nil), p.rules...)
				candidate = append(candidate[:index], candidate[index+1:]...)
				p.selectedRuleID = ""
				if ok, message := p.notifySave(candidate); !ok {
					p.setText(idNext, message)
				}
				p.populateRules()
			}
		}
	}
}
func (p *panel) handleEditor(id, notification uint16) {
	if notification == enChange {
		switch id {
		case idName, idDate, idTime, idEndTime, idIdleMinutes, idWarningSeconds, idMaxWait:
			p.clearEditorError()
		}
	}
	if _, ok := p.choices[id]; ok {
		if ownerDrawButtonClicked(notification) {
			// Open after the native BUTTON finishes its click processing. Opening
			// synchronously from BN_CLICKED lets the button restore focus and close
			// the popup immediately on some Windows versions.
			pPostMessage.Call(uintptr(p.hwnd), wmOpenChoice, uintptr(id), 0)
		}
		// Owner-drawn buttons also send focus notifications. When the popup
		// takes focus, treating BN_KILLFOCUS as a regular command would close
		// the popup that has just opened.
		return
	}
	if id >= idWeekdayBase && id < idWeekdayBase+uint16(len(editorWeekdays)) && ownerDrawButtonClicked(notification) {
		p.setChecked(id, !p.checked(id))
		p.clearEditorError()
		return
	}
	p.closeChoice(false)
	switch id {
	case idDaysWorkdays:
		if ownerDrawButtonClicked(notification) {
			p.selectWeekdays(false)
		}
	case idDaysEveryday:
		if ownerDrawButtonClicked(notification) {
			p.selectWeekdays(true)
		}
	case idKeepScreen:
		if ownerDrawButtonClicked(notification) {
			p.setChecked(id, !p.checked(id))
		}
	case idChooseProcesses:
		if ownerDrawButtonClicked(notification) {
			p.syncDraft()
			_ = processpicker.Show(processpicker.Options{Owner: p.hwnd, Selected: p.draft.Processes, Descriptions: p.processDescriptions, Chinese: p.state.Chinese, Text: p.text, OnConfirm: func(targets []automation.ProcessTarget, descriptions map[string]string) {
				p.draft.Processes = targets
				p.processDescriptions = descriptions
				p.setText(idProcessSummary, p.processSummary())
				p.refreshProcessInfoTooltip()
				p.clearEditorError()
				p.relayoutEditor()
			}})
		}
	case idProcessInfo:
		if ownerDrawButtonClicked(notification) && len(p.draft.Processes) > 0 {
			p.showProcessDetails()
		}
	case idSave:
		if ownerDrawButtonClicked(notification) {
			p.saveEditor()
		}
	case idCancel:
		if ownerDrawButtonClicked(notification) {
			p.cancelEditor()
		}
	}
}

func ownerDrawButtonClicked(notification uint16) bool {
	return notification == bnClicked
}

func (p *panel) selectWeekdays(everyDay bool) {
	for index := range editorWeekdays {
		p.setChecked(idWeekdayBase+uint16(index), everyDay || index < 5)
	}
	p.clearEditorError()
}

func (p *panel) handleChoiceChanged(id uint16) {
	p.syncDraft()
	if id == idAction {
		p.setTriggerOptions(p.draft.Action, p.draft.Trigger)
	}
	p.clearEditorError()
	p.relayoutEditor()
}

func (p *panel) clearEditorError() {
	if !p.editorStatusError {
		return
	}
	p.editorStatusError = false
	p.setText(idValidation, p.t("automation_runtime_note"))
}

func (p *panel) relayoutEditor() {
	p.beginRebuild()
	p.layoutEditor()
	p.endRebuild()
}

func (p *panel) editSelected() {
	if index := p.selectedRule(); index >= 0 {
		p.selectedRuleID = p.rules[index].ID
		p.showEditor(index)
	}
}

func (p *panel) updateManagerActions() {
	index := p.selectedRule()
	enabled := index >= 0
	for _, id := range []uint16{idEdit, idDelete, idToggle} {
		p.enable(id, enabled)
	}
	label := p.t("automation_toggle")
	if enabled {
		if p.rules[index].Enabled {
			label = p.t("automation_disable")
		} else {
			label = p.t("automation_enable")
		}
	}
	p.setText(idToggle, label)
}

func (p *panel) cancelEditor() {
	p.syncDraft()
	if editorChangesRequireConfirmation(p.editing, p.originalDraft, p.draft) && !p.confirm(p.t("automation_discard_title"), p.t("automation_discard_confirm")) {
		return
	}
	if p.pendingState != nil {
		p.acceptState(*p.pendingState)
	}
	p.showManager()
}

func editorChangesRequireConfirmation(editing int, original, draft automation.Rule) bool {
	if !editorHasAnyChanges(original, draft) {
		return false
	}
	if editing >= 0 {
		return true
	}
	return !reflect.DeepEqual(newRuleIntentOf(draft), newRuleIntentOf(original))
}

func editorHasAnyChanges(original, draft automation.Rule) bool {
	return !reflect.DeepEqual(draft, original)
}

// newRuleIntent is the explicit discard boundary for an unsaved rule. Runtime
// defaults and action options alone do not create a runnable task; identity,
// action, trigger, schedule, days and process targets represent user work.
// Keeping this projection explicit prevents newly added Rule fields from
// silently changing cancel behavior.
type newRuleIntent struct {
	Name         string
	Action       automation.Action
	Trigger      automation.Trigger
	Time         string
	EndTime      string
	Date         string
	Days         []string
	ProcessLogic automation.ProcessLogic
	Processes    []automation.ProcessTarget
}

func newRuleIntentOf(rule automation.Rule) newRuleIntent {
	return newRuleIntent{
		Name: rule.Name, Action: rule.Action, Trigger: rule.Trigger,
		Time: rule.Time, EndTime: rule.EndTime, Date: rule.Date,
		Days: append([]string(nil), rule.Days...), ProcessLogic: rule.ProcessLogic,
		Processes: append([]automation.ProcessTarget(nil), rule.Processes...),
	}
}
func (p *panel) selectedRule() int {
	value, _, _ := pSendMessage.Call(uintptr(p.controls[idList]), lbGetCurSel, 0, 0)
	if int(value) < 0 || int(value) >= len(p.rules) || value == ^uintptr(0) {
		return -1
	}
	return int(value)
}
func (p *panel) notifySave(rules []automation.Rule) (bool, string) {
	if p.onSave == nil {
		p.rules = append([]automation.Rule(nil), rules...)
		p.state.Rules = append([]automation.Rule(nil), rules...)
		p.state.Issues = nil
		return true, ""
	}
	result := p.onSave(SaveRequest{BaseRevision: p.state.Revision, Rules: append([]automation.Rule(nil), rules...)})
	if result.Error != "" {
		if p.view == managerView && (result.State.Revision != "" || result.State.Rules != nil) {
			p.acceptState(result.State)
			p.managerNotice = result.Error
		} else if result.State.Revision != "" && result.State.Revision != p.state.Revision {
			pending := cloneState(result.State)
			p.pendingState = &pending
		}
		return false, result.Error
	}
	p.managerNotice = ""
	p.acceptState(result.State)
	return true, ""
}
func (p *panel) processSummary() string {
	if len(p.draft.Processes) == 0 {
		return p.t("automation_no_processes")
	}
	return fmt.Sprintf(p.t("automation_process_count"), len(p.draft.Processes))
}

func (p *panel) processDetails() string {
	targets := automation.NormalizeTargets(p.draft.Processes)
	lines := make([]string, 0, len(targets))
	for _, target := range targets {
		name := target.Executable
		if description := p.processDescriptions[target.Key()]; description != "" {
			name = fmt.Sprintf(p.t("process_name_description"), name, description)
		}
		if target.Match == automation.MatchPath {
			lines = append(lines, fmt.Sprintf(p.t("automation_process_detail_path"), name, target.Path))
		} else {
			lines = append(lines, fmt.Sprintf(p.t("automation_process_detail_name"), name))
		}
	}
	return strings.Join(lines, "\n")
}

func (p *panel) refreshProcessInfoTooltip() {
	if p.controls[idProcessInfo] == 0 || len(p.draft.Processes) == 0 {
		return
	}
	p.addTooltipValue(idProcessInfo, p.processDetails())
}

func (p *panel) showProcessDetails() {
	title, _ := windows.UTF16PtrFromString(p.t("automation_process_details_title"))
	body, _ := windows.UTF16PtrFromString(p.processDetails())
	const mbOKInformation = 0x00000000 | 0x00000040
	user32.NewProc("MessageBoxW").Call(uintptr(p.hwnd), uintptr(unsafe.Pointer(body)), uintptr(unsafe.Pointer(title)), mbOKInformation)
}

func (p *panel) loadProcessDescriptions() {
	targets := automation.NormalizeTargets(p.draft.Processes)
	if len(targets) == 0 {
		return
	}
	ruleID := p.draft.ID
	go func() {
		descriptions := make(map[string]string, len(targets))
		names := make(map[string]struct{})
		for _, target := range targets {
			if target.Match == automation.MatchPath {
				if description := processcatalog.FileDescription(target.Path); description != "" {
					descriptions[target.Key()] = description
				}
			} else {
				names[strings.ToLower(target.Executable)] = struct{}{}
			}
		}
		if len(names) > 0 {
			if instances, err := processcatalog.SnapshotNames(); err == nil {
				filtered := instances[:0]
				for _, instance := range instances {
					if _, wanted := names[strings.ToLower(instance.Executable)]; wanted {
						filtered = append(filtered, instance)
					}
				}
				for _, group := range processcatalog.GroupInstances(processcatalog.EnrichDescriptions(filtered)) {
					if group.Description != "" {
						target := automation.ProcessTarget{Match: automation.MatchName, Executable: group.Executable}
						descriptions[target.Key()] = group.Description
					}
				}
			}
		}
		if len(descriptions) == 0 {
			return
		}
		trayicon.Post(func() {
			if p.hwnd == 0 || p.view != editorView || p.draft.ID != ruleID {
				return
			}
			for key, value := range descriptions {
				p.processDescriptions[key] = value
			}
			p.refreshProcessInfoTooltip()
		})
	}()
}
func (p *panel) ruleSummary(rule automation.Rule) string {
	action := p.t(actionKey(rule.Action))
	switch rule.Trigger {
	case automation.TriggerProcessRunning:
		return fmt.Sprintf(p.t("automation_summary_process_running"), action, len(rule.Processes))
	case automation.TriggerProcessStarted:
		return fmt.Sprintf(p.t("automation_summary_process_started"), action, len(rule.Processes))
	case automation.TriggerProcessExited:
		return fmt.Sprintf(p.t("automation_summary_process_exited"), action, len(rule.Processes))
	case automation.TriggerTimeWindow:
		return fmt.Sprintf(p.t("automation_summary_time_window"), action, rule.Time, rule.EndTime, p.daySummary(rule.Days))
	case automation.TriggerOnce:
		return fmt.Sprintf(p.t("automation_summary_once"), action, rule.Date, rule.Time)
	case automation.TriggerDaily:
		return fmt.Sprintf(p.t("automation_summary_daily"), action, rule.Time)
	case automation.TriggerWeekly:
		return fmt.Sprintf(p.t("automation_summary_weekly"), action, p.daySummary(rule.Days), rule.Time)
	}
	return action
}

func automationTimeLabelKey(trigger automation.Trigger) string {
	if trigger == automation.TriggerTimeWindow {
		return "automation_time"
	}
	return "automation_execution_time"
}

func (p *panel) daySummary(days []string) string {
	all := true
	for _, day := range editorWeekdays {
		if !containsDay(days, day) {
			all = false
			break
		}
	}
	if all {
		return p.t("automation_days_everyday")
	}
	workdays := !containsDay(days, "sat") && !containsDay(days, "sun")
	for _, day := range editorWeekdays[:5] {
		workdays = workdays && containsDay(days, day)
	}
	if workdays {
		return p.t("automation_days_workdays")
	}
	labels := make([]string, 0, len(days))
	for _, day := range editorWeekdays {
		if !containsDay(days, day) {
			continue
		}
		label := p.t("automation_day_" + day)
		if p.state.Chinese {
			label = "周" + label
		}
		labels = append(labels, label)
	}
	return strings.Join(labels, p.t("automation_days_separator"))
}
