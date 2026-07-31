package app

import (
	"time"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/config"
	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/autostart"
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
		if s.cfg.IdleTimeoutMinutes > 0 {
			s.setIdleTimeout(0)
		} else {
			s.setIdleTimeout(config.DefaultIdleTimeoutMinutes)
		}
		s.saveConfig()
	case controlpanel.ActIdleTimeout:
		if value > 0 && value <= 7*24*60 {
			s.setIdleTimeout(value)
			s.saveConfig()
		}
	case controlpanel.ActIdleAction:
		if action, ok := config.IdleActionAt(value); ok {
			s.cfg.IdleAction = action
			s.reconcileRuntime()
			s.saveConfig()
		}
	case controlpanel.ActIdleWarningToggle:
		if s.cfg.IdleWarningSeconds > 0 {
			s.cfg.IdleWarningSeconds = 0
		} else {
			s.cfg.IdleWarningSeconds = 30
		}
		s.reconcileRuntime()
		s.saveConfig()
	case controlpanel.ActIdleEnhancedMonitorToggle:
		s.cfg.IdleEnhancedMonitor = !s.cfg.IdleEnhancedMonitor
		mylog.Info("Idle monitoring enhanced mode toggled: enabled=%v", s.cfg.IdleEnhancedMonitor)
		s.reconcileRuntime()
		s.saveConfig()
	default:
		return false
	}
	return true
}

func (s *runtimeState) handleThemeControlAction(action controlpanel.Action) bool {
	if !s.themeAvailable() {
		switch action {
		case controlpanel.ActThemeToggle, controlpanel.ActBatteryToggle, controlpanel.ActFullscreenToggle,
			controlpanel.ActIPLocationToggle, controlpanel.ActSwitchTheme, controlpanel.ActRepairTheme:
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
	case controlpanel.ActBatteryToggle:
		s.cfg.ThemeDarkOnBattery = !s.cfg.ThemeDarkOnBattery
		s.restartThemeScheduler()
		s.refreshControlPanelThemeSchedule()
		s.saveConfig()
	case controlpanel.ActFullscreenToggle:
		s.cfg.ThemeSkipFullscreen = !s.cfg.ThemeSkipFullscreen
		s.restartThemeScheduler()
		s.refreshControlPanelThemeSchedule()
		s.saveConfig()
	case controlpanel.ActIPLocationToggle:
		s.cfg.ThemeIPLocationEnabled = !s.cfg.ThemeIPLocationEnabled
		s.restartThemeScheduler()
		if s.cfg.ThemeIPLocationEnabled {
			s.startIPLocationCycle()
		} else {
			s.stopIPLocationCycle()
		}
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
		s.runThemeOperation("menu_theme_switch_now", func() error {
			return theme.Switch(mode)
		}, nil)
	case controlpanel.ActRepairTheme:
		s.runThemeOperation("menu_theme_repair", func() error {
			defer trayicon.Post(controlpanel.RefreshThemeAfterSystemRepair)
			return theme.Refresh()
		}, nil)
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
	case controlpanel.ActHotkeyToggle:
		s.cfg.HotkeysEnabled = !s.cfg.HotkeysEnabled
		if s.cfg.HotkeysEnabled {
			s.startHotkeys()
		} else {
			s.stopHotkeys()
		}
		s.saveConfig()
	case controlpanel.ActAutostartToggle:
		enabled := !s.cfg.AutostartEnabled
		var err error
		if enabled {
			err = autostart.Enable()
		} else {
			err = autostart.Disable()
		}
		if err != nil {
			s.showError("menu_autostart", err)
			return
		}
		s.cfg.AutostartEnabled = enabled
		s.saveConfig()
	case controlpanel.ActLoggingToggle:
		s.cfg.LoggingEnabled = !s.cfg.LoggingEnabled
		s.applyLogging()
		s.updateIcon()
		s.saveConfig()
	case controlpanel.ActLanguage:
		previous := s.lang
		switch value {
		case 0:
			s.switchLanguage("en")
		case 1:
			s.switchLanguage("zh-CN")
		}
		if s.lang != previous {
			trayicon.Post(s.refreshControlPanel)
		}
	case controlpanel.ActConfig:
		path, err := config.Path()
		if err != nil {
			mylog.Info("Config path lookup failed: %v", err)
			return
		}
		if err := openWithShell(systemExecutable("notepad.exe"), windows.EscapeArg(path)); err != nil {
			mylog.Info("Config editor launch failed: %v", err)
		}
	case controlpanel.ActProjectHome:
		if err := openWithShell(projectHomeURL, ""); err != nil {
			mylog.Info("Project home launch failed: %v", err)
		}
	case controlpanel.ActExit:
		trayicon.Post(func() {
			hideAutomationUI()
			controlpanel.Destroy()
			trayicon.Quit()
		})
	}
}
