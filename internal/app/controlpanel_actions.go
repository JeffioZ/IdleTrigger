package app

import (
	"time"

	"github.com/JeffioZ/idletrigger/internal/config"
	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/systemaction"
	"github.com/JeffioZ/idletrigger/internal/ui/controlpanel"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

// handleControlPanelAction is the sole application-state entry point for
// control-panel actions. Each handler owns its runtime reconciliation, config
// persistence, icon refresh, and scheduler restart side effects.
func (s *runtimeState) handleControlPanelAction(action controlpanel.Action, value int) {
	if s.handleSystemControlAction(action) || s.handleIdleControlAction(action, value) || s.handleThemeControlAction(action) {
		return
	}
	s.handleGeneralControlAction(action, value)
}

func (s *runtimeState) handleSystemControlAction(action controlpanel.Action) bool {
	var systemAction config.Action
	switch action {
	case controlpanel.ActSleep:
		systemAction = config.ActionSleep
	case controlpanel.ActHibernate:
		systemAction = config.ActionHibernate
	case controlpanel.ActShutdown:
		systemAction = config.ActionShutdown
	case controlpanel.ActLock:
		systemAction = config.ActionLock
	case controlpanel.ActRestart:
		if err := systemaction.Restart(); err != nil {
			s.showError("menu_restart", err)
		}
		return true
	default:
		return false
	}
	if err := s.executeAction(systemAction); err != nil {
		s.showError(actionTranslationKey(systemAction), err)
	}
	return true
}

func (s *runtimeState) handleIdleControlAction(action controlpanel.Action, value int) bool {
	switch action {
	case controlpanel.ActNoSleepToggle:
		s.toggleNoSleep()
	case controlpanel.ActIdleToggle:
		s.setIdleEnabled(!s.cfg.IdleEnabled)
		s.saveConfig()
	default:
		return false
	}
	return true
}

func (s *runtimeState) handleThemeControlAction(action controlpanel.Action) bool {
	if !s.themeAvailable() {
		switch action {
		case controlpanel.ActThemeToggle, controlpanel.ActSwitchTheme, controlpanel.ActRepairTheme:
			return true
		}
	}
	switch action {
	case controlpanel.ActThemeToggle:
		s.cfg.ThemeSwitchEnabled = !s.cfg.ThemeSwitchEnabled
		if s.cfg.ThemeSwitchEnabled {
			s.startThemeScheduler()
		} else {
			s.stopThemeScheduler()
		}
		s.syncBatteryLoop()
		s.updateIcon()
		s.refreshControlPanelThemeSchedule()
		s.saveConfig()
	case controlpanel.ActSwitchTheme:
		mode := theme.ModeDark
		if theme.Current() == theme.ModeDark {
			mode = theme.ModeLight
		}
		if s.themeSched != nil {
			s.themeSched.HoldManualOverride(time.Now())
			mylog.Info("Manual theme override enabled until the next scheduled transition")
		}
		s.requestManualThemeSwitch(mode)
	case controlpanel.ActRepairTheme:
		s.repairTheme()
	default:
		return false
	}
	return true
}

func (s *runtimeState) handleGeneralControlAction(action controlpanel.Action, value int) {
	switch action {
	case controlpanel.ActAutomationToggle:
		s.cfg.AutomationEnabled = !s.cfg.AutomationEnabled
		s.restartAutomation()
		s.saveConfig()
		s.refreshControlPanelAutomationStatus()
	case controlpanel.ActAutomationOpen:
		s.showAutomationManager()
	case controlpanel.ActSettingsOpen:
		s.showSettingsPanel()
	case controlpanel.ActExit:
		trayicon.Post(func() {
			hideAutomationUI()
			controlpanel.Destroy()
			trayicon.Quit()
		})
	}
}

func (s *runtimeState) repairTheme() {
	if !s.themeAvailable() {
		return
	}
	s.requestManualThemeRepair()
}

func (s *runtimeState) openProjectHome() {
	if err := openWithShell(projectHomeURL, ""); err != nil {
		mylog.Info("Project home launch failed: %v", err)
	}
}
