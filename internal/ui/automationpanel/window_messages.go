package automationpanel

import (
	"github.com/JeffioZ/idletrigger/internal/ui/font"
	"strings"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/automation"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
	"github.com/JeffioZ/idletrigger/internal/ui/processpicker"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

func wndProc(hwnd windows.Handle, message uint32, wParam, lParam uintptr) uintptr {
	activeMu.Lock()
	p := active
	activeMu.Unlock()
	if p == nil || p.hwnd != hwnd {
		result, _, _ := pDefWindowProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
		return result
	}
	switch message {
	case wmClose:
		p.closeChoice(false)
		if p.view == editorView {
			p.cancelEditor()
		} else {
			pDestroyWindow.Call(uintptr(hwnd))
		}
		return 0
	case wmLButtonDown:
		p.interaction.SetFocusVisible(false)
		p.closeChoice(false)
	case wmMouseWheel:
		if p.scrollWheel(wParam) {
			return 0
		}
	case wmOpenChoice:
		if p.view == editorView {
			p.toggleChoice(uint16(wParam))
		}
		return 0
	case wmPrewarmEditor:
		if p.view == managerView && !p.editorReady {
			p.createEditorControls()
		}
		return 0
	case wmCommand:
		p.handleCommand(uint16(wParam), uint16(wParam>>16))
		return 0
	case wmDrawItem:
		if p.drawOwnerItem((*drawItem)(nativeform.MessagePointer(lParam))) {
			return 1
		}
	case wmPaint:
		nativeform.PaintWindowBackground(hwnd, p.windowBrush)
		return 0
	case wmEraseBkgnd:
		var bounds rect
		pGetClientRect.Call(uintptr(hwnd), uintptr(unsafe.Pointer(&bounds)))
		pFillRect.Call(wParam, uintptr(unsafe.Pointer(&bounds)), uintptr(p.windowBrush))
		return 1
	case wmCtlColorStatic:
		textColor := p.palette.PrimaryText
		controlID := p.controlID(windows.Handle(lParam))
		if isSecondaryLabel(controlID) {
			textColor = p.palette.SecondaryText
		} else if isMutedLabel(controlID) {
			textColor = p.palette.MutedText
		}
		if controlID == idValidation {
			textColor = p.palette.MutedText
			if p.editorStatusError {
				if p.themeDark {
					textColor = p.palette.DangerBorder
				} else {
					textColor = p.palette.DangerBackground
				}
			}
		}
		brush := p.windowBrush
		backgroundColor := p.palette.WindowBackground
		if controlID == idEmptyTitle || controlID == idEmptyBody {
			brush = p.surfaceBrush
			backgroundColor = p.palette.Surface
		}
		pSetTextColor.Call(wParam, uintptr(textColor))
		pSetBkColor.Call(wParam, uintptr(backgroundColor))
		pSetBkMode.Call(wParam, opaque)
		return uintptr(brush)
	case wmCtlColorButton:
		pSetTextColor.Call(wParam, uintptr(p.palette.PrimaryText))
		pSetBkMode.Call(wParam, transparent)
		return uintptr(p.windowBrush)
	case wmCtlColorEdit, wmCtlColorList:
		brush := p.surfaceBrush
		textColor := p.palette.PrimaryText
		backgroundColor := p.palette.Surface
		if cueColor, cue := p.surfaces.CueColor(windows.Handle(lParam)); cue {
			textColor = cueColor
		}
		if enabled, _, _ := pIsWindowEnabled.Call(lParam); enabled == 0 {
			brush = p.disabledBrush
			textColor = p.palette.DisabledText
			backgroundColor = p.palette.DisabledSurface
		}
		pSetTextColor.Call(wParam, uintptr(textColor))
		pSetBkColor.Call(wParam, uintptr(backgroundColor))
		return uintptr(brush)
	case wmSettingChange, wmSysColorChange, wmThemeChanged:
		if message == wmSettingChange && p.font != 0 {
			next := font.TextScaleFactor()
			if next != p.textScale {
				previous := p.textScale
				p.textScale = next
				if !p.rebuildForDPI() {
					p.textScale = previous
				}
			}
		}
		p.applyTheme()
		return 0
	case wmDpiChanged:
		if p.font != 0 {
			transition := nativeform.BeginFrameTransition(p.hwnd)
			dpi := uint32(wParam & 0xffff)
			if dpi == 0 {
				dpi = 96
			}
			previousScale := p.dpiScale
			p.dpiScale = float64(dpi) / 96
			if lParam != 0 {
				suggested := nativeform.Rect(*(*rect)(nativeform.MessagePointer(lParam)))
				p.pendingSuggested = &suggested
			}
			if p.rebuildForDPI() {
				scale := p.scale()
				p.icons.Apply(p.hwnd, p.themeDark, int(32*scale+0.5), int(16*scale+0.5), true)
			} else {
				p.dpiScale = previousScale
				p.pendingSuggested = nil
			}
			committed := false
			for range 3 {
				if err := transition.Commit(p.frameControls()...); err == nil {
					committed = true
					break
				}
			}
			if !committed {
				pDestroyWindow.Call(uintptr(p.hwnd))
			}
		}
		return 0
	case wmDestroy:
		processpicker.Hide()
		if p.contentScroll != nil {
			p.contentScroll.Close()
			p.contentScroll = nil
		}
		p.surfaces.Close()
		if p.managerScroll != nil {
			p.managerScroll.Close()
			p.managerScroll = nil
		}
		trayicon.ClearTabNavigationWindow(hwnd)
		if p.font != 0 {
			pDeleteObject.Call(uintptr(p.font))
		}
		if p.sectionFont != 0 {
			pDeleteObject.Call(uintptr(p.sectionFont))
		}
		p.releaseBrushes()
		p.icons.Release()
		if p.ownerDisabled && p.state.Owner != 0 {
			if valid, _, _ := pIsWindow.Call(uintptr(p.state.Owner)); valid != 0 {
				pEnableWindow.Call(uintptr(p.state.Owner), 1)
				pSetForeground.Call(uintptr(p.state.Owner))
			}
			p.ownerDisabled = false
		}
		activeMu.Lock()
		if active == p {
			active = nil
		}
		activeMu.Unlock()
		p.hwnd = 0
		return 0
	}
	result, _, _ := pDefWindowProc.Call(uintptr(hwnd), uintptr(message), wParam, lParam)
	return result
}

func (p *panel) controlID(hwnd windows.Handle) uint16 {
	for id, control := range p.controls {
		if control == hwnd {
			return id
		}
	}
	return 0
}

func isSecondaryLabel(id uint16) bool {
	switch id {
	case idNameLabel, idActionLabel, idTriggerLabel, idDateLabel, idTimeLabel, idEndTimeLabel, idDaysLabel,
		idProcessLogicLabel, idIdleMinutesLabel, idWarningLabel, idBlockedLabel, idMaxWaitLabel,
		idProcessSummary, idNext, idEmptyBody:
		return true
	default:
		return false
	}
}

func isMutedLabel(id uint16) bool {
	switch id {
	case idNameHint, idNoOptions:
		return true
	default:
		return false
	}
}

var actions = []automation.Action{automation.ActionStayAwake, automation.ActionPauseStayAwake, automation.ActionEnableIdle, automation.ActionPauseIdle, automation.ActionLock, automation.ActionSleep, automation.ActionHibernate, automation.ActionShutdown, automation.ActionRestart}
var processModes = []automation.ProcessLogic{automation.ProcessAny, automation.ProcessAll, automation.ProcessNone}
var blockedModes = []automation.BlockedPolicy{automation.BlockedSkip, automation.BlockedWait}
var editorWeekdays = []string{"mon", "tue", "wed", "thu", "fri", "sat", "sun"}

func managerControlIDs() []uint16 {
	return []uint16{idTitle, idListSurface, idList, idEmptyTitle, idEmptyBody, idNext, idNew, idEdit, idDelete, idToggle}
}

func editorControlIDs() []uint16 {
	ids := []uint16{
		idBasicsTitle, idNameLabel, idName, idNameHint, idActionLabel, idAction, idTriggerLabel, idTrigger,
		idTriggerTitle, idDateLabel, idDate, idTimeLabel, idTime, idEndTimeLabel, idEndTime, idDaysLabel, idDaysWorkdays, idDaysEveryday,
		idProcessLogicLabel, idProcessLogic, idChooseProcesses, idProcessSummary, idProcessInfo, idOptionsTitle, idKeepScreen,
		idIdleMinutesLabel, idIdleMinutes, idWarningLabel, idWarningSeconds, idBlockedLabel, idBlockedPolicy,
		idMaxWaitLabel, idMaxWait, idNoOptions, idValidation, idCancel, idSave,
	}
	for index := range editorWeekdays {
		ids = append(ids, idWeekdayBase+uint16(index))
	}
	return ids
}

func containsDay(days []string, wanted string) bool {
	for _, day := range days {
		if strings.EqualFold(strings.TrimSpace(day), wanted) {
			return true
		}
	}
	return false
}

func triggerKey(trigger automation.Trigger) string {
	switch trigger {
	case automation.TriggerProcessRunning:
		return "automation_trigger_process_running"
	case automation.TriggerProcessStarted:
		return "automation_trigger_process_started"
	case automation.TriggerProcessExited:
		return "automation_trigger_process_exited"
	case automation.TriggerTimeWindow:
		return "automation_trigger_time_window"
	case automation.TriggerOnce:
		return "automation_trigger_once"
	case automation.TriggerDaily:
		return "automation_trigger_daily"
	case automation.TriggerWeekly:
		return "automation_trigger_weekly"
	default:
		return "automation_trigger"
	}
}

func (p *panel) validateDraft() (uint16, string) {
	trigger := p.draft.Trigger
	if (trigger == automation.TriggerProcessRunning || trigger == automation.TriggerProcessStarted || trigger == automation.TriggerProcessExited) && len(p.draft.Processes) == 0 {
		return idChooseProcesses, p.t("automation_error_process_required")
	}
	if trigger == automation.TriggerOnce {
		if _, err := time.Parse("2006-01-02", p.draft.Date); err != nil {
			return idDate, p.t("automation_error_date")
		}
	}
	if trigger == automation.TriggerOnce || trigger == automation.TriggerDaily || trigger == automation.TriggerWeekly || trigger == automation.TriggerTimeWindow {
		if _, err := time.Parse("15:04", p.draft.Time); err != nil {
			return idTime, p.t("automation_error_time")
		}
	}
	if trigger == automation.TriggerTimeWindow {
		if _, err := time.Parse("15:04", p.draft.EndTime); err != nil {
			return idEndTime, p.t("automation_error_end_time")
		}
	}
	if (trigger == automation.TriggerWeekly || trigger == automation.TriggerTimeWindow) && len(p.draft.Days) == 0 {
		return idWeekdayBase, p.t("automation_error_days")
	}
	if p.draft.Action == automation.ActionEnableIdle && (p.draft.IdleMinutes <= 0 || p.draft.IdleMinutes > 7*24*60) {
		return idIdleMinutes, p.t("automation_error_idle_minutes")
	}
	if automation.IsEventAction(p.draft.Action) && (p.draft.WarningSeconds < automation.MinWarningSeconds || p.draft.WarningSeconds > 3600) {
		return idWarningSeconds, p.t("automation_error_warning")
	}
	if automation.IsEventAction(p.draft.Action) && len(p.draft.Processes) > 0 && trigger != automation.TriggerProcessStarted && trigger != automation.TriggerProcessExited && p.draft.BlockedPolicy == automation.BlockedWait && (p.draft.MaxWaitMinutes <= 0 || p.draft.MaxWaitMinutes > 7*24*60) {
		return idMaxWait, p.t("automation_error_max_wait")
	}
	return 0, ""
}

func (p *panel) setEditorError(id uint16, message string) {
	p.editorStatusError = true
	p.setText(idValidation, message)
	if control := p.controls[id]; control != 0 {
		pSetFocus.Call(uintptr(control))
	}
}

func actionAt(i int) automation.Action {
	if i < 0 || i >= len(actions) {
		return actions[0]
	}
	return actions[i]
}
func actionIndex(v automation.Action) int {
	for i, x := range actions {
		if x == v {
			return i
		}
	}
	return 0
}
func processAt(i int) automation.ProcessLogic {
	if i < 0 || i >= len(processModes) {
		return processModes[0]
	}
	return processModes[i]
}
func processIndex(v automation.ProcessLogic) int {
	for i, x := range processModes {
		if x == v {
			return i
		}
	}
	return 0
}
func blockedAt(i int) automation.BlockedPolicy {
	if i < 0 || i >= len(blockedModes) {
		return blockedModes[0]
	}
	return blockedModes[i]
}
func blockedIndex(v automation.BlockedPolicy) int {
	for i, x := range blockedModes {
		if x == v {
			return i
		}
	}
	return 0
}
func actionLabels(t TextFunc) []string {
	out := make([]string, len(actions))
	for i, v := range actions {
		out[i] = t(actionKey(v))
	}
	return out
}
func processLabels(t TextFunc) []string {
	return []string{t("automation_process_any"), t("automation_process_all"), t("automation_process_none")}
}
func blockedLabels(t TextFunc) []string {
	return []string{t("automation_blocked_skip"), t("automation_blocked_wait")}
}
func actionKey(v automation.Action) string {
	switch v {
	case automation.ActionStayAwake:
		return "automation_action_stay_awake"
	case automation.ActionPauseStayAwake:
		return "automation_action_pause_stay_awake"
	case automation.ActionEnableIdle:
		return "automation_action_enable_idle"
	case automation.ActionPauseIdle:
		return "automation_action_pause_idle"
	case automation.ActionLock:
		return "menu_lock"
	case automation.ActionSleep:
		return "menu_sleep"
	case automation.ActionHibernate:
		return "menu_hibernate"
	case automation.ActionShutdown:
		return "menu_shutdown"
	case automation.ActionRestart:
		return "menu_restart"
	}
	return "menu_more"
}
