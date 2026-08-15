package app

import (
	"github.com/JeffioZ/idletrigger/internal/i18n"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/ui/automationpanel"
	"github.com/JeffioZ/idletrigger/internal/ui/controlpanel"
	"github.com/JeffioZ/idletrigger/internal/ui/settingspanel"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
	"github.com/JeffioZ/idletrigger/internal/version"
)

type controlPanelSnapshot struct {
	state controlpanel.State
	lang  string
}

func (s *runtimeState) showControlPanel() {
	if settingspanel.Focus() || automationpanel.Focus() {
		return
	}
	s.openControlPanel(false)
}

func (s *runtimeState) refreshControlPanel() {
	s.openControlPanel(true)
}

func (s *runtimeState) openControlPanel(refresh bool) {
	// Snapshot state off the Win32 message thread. This keeps the UI loop free
	// while the serialized state queue is busy stopping a monitor or applying a
	// config reload, and avoids a UI -> state -> worker -> UI wait cycle.
	go s.prepareControlPanel(refresh)
}

func (s *runtimeState) prepareControlPanel(refresh bool) {
	result := make(chan controlPanelSnapshot, 1)
	s.requestCh <- runtimeRequest{fn: func() string {
		result <- controlPanelSnapshot{
			state: controlpanel.State{
				NoSleepEnabled:          s.cfg.NoSleepEnabled,
				NoSleepStatus:           s.noSleepStatusText(),
				AutomationEnabled:       s.cfg.AutomationEnabled,
				AutomationCount:         s.enabledAutomationCount(),
				AutomationSummary:       s.automationOverviewText(),
				IdleEnabled:             s.cfg.IdleEnabled,
				IdleStatus:              s.monitorStatusText(),
				ThemeSwitchEnabled:      s.cfg.ThemeSwitchEnabled,
				ThemeUnavailable:        !s.themeAvailable(),
				ThemeUnavailableDetail:  s.themeUnavailableDetail(),
				ThemeOperationBusy:      s.themeOperationBusy,
				IsChinese:               i18n.ResolveLanguage(s.lang) == "zh-CN",
				ThemeSchedule:           s.themeScheduleText(true),
				AppVersion:              version.Value,
				Owner:                   trayicon.WindowHandle(),
				IdleTimeoutMinutes:      s.cfg.IdleTimeoutMinutes,
				IdleWarningSeconds:      s.cfg.IdleWarningSeconds,
				IdleAction:              string(s.cfg.IdleAction),
				DeveloperCapturePanel:   s.devtools.CapturePanel,
				DeveloperWarningPreview: s.devtools.WarningPreview,
			},
			lang: s.lang,
		}
		return ""
	}}
	snapshot := <-result
	onAction := func(action controlpanel.Action, value int) {
		s.post(func() { s.handleControlPanelAction(action, value) })
	}
	langFn := func(key string) string { return i18n.T(snapshot.lang, key) }
	trayicon.Post(func() {
		var err error
		if refresh {
			err = controlpanel.Refresh(snapshot.state, onAction, langFn)
		} else {
			err = controlpanel.Show(snapshot.state, onAction, langFn)
		}
		if err != nil {
			mylog.Info("Control panel open failed: %v", err)
		}
	})
}

func (s *runtimeState) refreshControlPanelAutomationStatus() {
	count := s.enabledAutomationCount()
	summary := s.automationOverviewText()
	enabled := s.cfg.AutomationEnabled
	trayicon.Post(func() { controlpanel.UpdateAutomationStatus(enabled, count, summary) })
}

func (s *runtimeState) refreshControlPanelPowerStatus() {
	noSleepStatus := s.noSleepStatusText()
	idleStatus := s.monitorStatusText()
	trayicon.Post(func() { controlpanel.UpdatePowerManagementStatus(noSleepStatus, idleStatus) })
}
