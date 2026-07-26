package app

import (
	"errors"
	"fmt"
	"github.com/JeffioZ/idletrigger/internal/config"
	"github.com/JeffioZ/idletrigger/internal/feature/idle"
	"github.com/JeffioZ/idletrigger/internal/feature/keepawake"
	"github.com/JeffioZ/idletrigger/internal/i18n"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/autostart"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/dialog"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/powerstate"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/systemaction"
	"github.com/JeffioZ/idletrigger/internal/ui/controlpanel"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
	"strings"
	"time"
)

var saveConfigAtRevision = config.SaveAtRevision

func (s *runtimeState) reloadConfig() error {
	wasIPLocationEligible := s.themeIPLocationLookupEnabled()
	previousLanguage := s.cfg.Language
	newCfg, err := config.Load()
	if err != nil {
		return err
	}
	s.cfg = newCfg
	s.applyLogging()
	s.detectThemeSupport()
	if enabled, updated, err := autostart.EnsureCurrent(); err == nil {
		s.cfg.AutostartEnabled = enabled
		if updated {
			mylog.Info("Autostart entry updated to current executable path")
		}
	} else {
		mylog.Info("Autostart check failed: %v", err)
	}
	s.stopMonitor()
	s.stopHotkeys()
	s.stopAutomation()
	s.stopThemeScheduler()
	keepawake.Disable()
	// Config compatibility: if both NoSleep and idle monitor are enabled,
	// resolve the conflict — NoSleep takes priority.
	if s.cfg.NoSleepEnabled && s.cfg.IdleTimeoutMinutes > 0 {
		s.cfg.IdleTimeoutMinutes = 0
	}
	s.lang = s.cfg.Language
	s.applyLanguage()
	s.batteryBlocked = false
	s.refreshBatteryPolicy()
	if s.cfg.HotkeysEnabled {
		s.startHotkeys()
	}
	s.startAutomation()
	if s.cfg.ThemeSwitchEnabled {
		s.startThemeScheduler()
	}
	s.syncIPLocationCycle(wasIPLocationEligible)
	s.reconcileRuntime()
	s.updateIcon()
	s.rememberConfigModTime()
	if previousLanguage != s.cfg.Language {
		trayicon.Post(hideAutomationUI)
	} else {
		s.publishAutomationPanelState()
	}
	trayicon.Post(controlpanel.Hide)
	return nil
}

func (s *runtimeState) fmtNoSleepStatus() string {
	return s.statusLine("status_nosleep", s.noSleepStatusText())
}

func (s *runtimeState) fmtStatus() string {
	ns := s.noSleepStatusText()
	mon := s.monitorStatusText()
	ps := powerstate.GetStatus()
	pow := i18n.T(s.lang, "status_unknown")
	if ps.Valid && ps.ACLine {
		pow = i18n.T(s.lang, "status_ac_power")
	} else if ps.Valid && ps.Battery && ps.Percent >= 0 {
		pow = fmt.Sprintf(i18n.T(s.lang, "status_battery"), ps.Percent)
	}

	idleText := i18n.T(s.lang, "status_unknown")
	if d, err := idle.IdleDuration(); err == nil {
		idleText = i18n.FormatDuration(s.lang, d)
	}

	hk := i18n.T(s.lang, "status_disabled")
	if s.cfg.HotkeysEnabled {
		hk = i18n.T(s.lang, "status_enabled")
	}
	automationStatus := i18n.T(s.lang, "status_disabled")
	if s.cfg.AutomationEnabled {
		automationStatus = fmt.Sprintf(i18n.T(s.lang, "status_automation_count"), s.enabledAutomationCount())
	}

	return strings.Join([]string{
		s.statusLine("status_tray", i18n.T(s.lang, "status_running")),
		s.statusLine("status_nosleep", ns),
		s.statusLine("status_monitor", mon),
		s.statusLine("status_automation", automationStatus),
		s.statusLine("status_power", pow),
		s.statusLine("status_idle_time", idleText),
		s.statusLine("status_hotkeys", hk),
	}, "\n")
}

func (s *runtimeState) statusLine(labelKey, value string) string {
	return fmt.Sprintf(i18n.T(s.lang, "status_line"), i18n.T(s.lang, labelKey), value)
}

func (s *runtimeState) monitorStatusText() string {
	if s.mon != nil {
		if s.devtools.IdleMonitorEnabled() {
			return s.developerIdleMonitorStatus()
		}
		threshold, _, action, _ := s.effectiveIdleMonitorSettings()
		return fmt.Sprintf(
			i18n.T(s.lang, "status_monitor_active"),
			int(threshold/time.Minute),
			i18n.T(s.lang, actionTranslationKey(action)),
		)
	}
	if s.idleSuspended() {
		return i18n.T(s.lang, "status_paused_by_nosleep")
	}
	if s.idleAutomationPaused() {
		return i18n.T(s.lang, "status_paused_by_automation")
	}
	return i18n.T(s.lang, "status_disabled")
}

func (s *runtimeState) noSleepStatusText() string {
	if s.noSleepAutomationPaused() {
		return i18n.T(s.lang, "status_paused_by_automation")
	}
	if s.noSleepRequested() && s.batteryBlocked {
		return i18n.T(s.lang, "status_paused_by_battery")
	}
	if keepawake.IsKeepingScreenOn() {
		return i18n.T(s.lang, "status_enabled_keep_screen")
	}
	if keepawake.IsEnabled() {
		return i18n.T(s.lang, "status_enabled")
	}
	return i18n.T(s.lang, "status_disabled")
}

// ---- helpers ----------------------------------------------------------

func (s *runtimeState) executeAction(a config.Action) error {
	return executeActionWithLanguage(a, s.lang)
}

func executeActionWithLanguage(a config.Action, lang string) error {
	if !actionAvailable(a, powerstate.GetCapabilities()) {
		switch a {
		case config.ActionSleep:
			return errors.New(i18n.T(lang, "cli_error_sleep_unavailable"))
		case config.ActionHibernate:
			return errors.New(i18n.T(lang, "cli_error_hibernate_unavailable"))
		}
	}
	switch a {
	case config.ActionSleep:
		return systemaction.Sleep()
	case config.ActionHibernate:
		return systemaction.Hibernate()
	case config.ActionShutdown:
		return systemaction.Shutdown()
	case config.ActionLock:
		return systemaction.Lock()
	default:
		return fmt.Errorf("unsupported action %q", a)
	}
}

func actionAvailable(action config.Action, capabilities powerstate.Capabilities) bool {
	switch action {
	case config.ActionSleep:
		return capabilities.SleepAvailable
	case config.ActionHibernate:
		return capabilities.HibernateAvailable
	default:
		return true
	}
}

func actionTranslationKey(a config.Action) string {
	return "menu_action_" + string(a)
}

// switchLanguage updates the active language, refreshes all menu text,
// and persists the choice.
func (s *runtimeState) switchLanguage(lang string) {
	trayicon.Post(hideAutomationUI)
	s.lang = lang
	s.cfg.Language = lang
	s.applyLanguage()
	mylog.Info("Language switched: %s", lang)
	s.saveConfig()
}

func (s *runtimeState) applyLanguage() {
	T := func(key string) string { return i18n.T(s.lang, key) }
	trayicon.SetTitle(T("app_title"))
	if s.menuOpen != nil {
		s.menuOpen.SetTitle(T("menu_open_panel"))
	}
	if s.menuExit != nil {
		s.menuExit.SetTitle(T("menu_exit"))
	}
	s.updateIcon()
}

func (s *runtimeState) saveConfigErr() error {
	revision, err := s.persistConfigAtRevision(s.cfg, s.cfg.SourceRevision)
	if err != nil {
		return err
	}
	s.cfg.SourceRevision = revision
	return nil
}

func (s *runtimeState) persistConfigAtRevision(candidate config.Config, expectedRevision string) (string, error) {
	// The config watcher runs independently of the tray UI goroutine.  Mark the
	// whole atomic-replace window as self-authored so it cannot race the
	// mod-time snapshot and schedule reloadConfig (which hides the control panel).
	s.selfConfigWrite.Store(true)
	defer s.selfConfigWrite.Store(false)
	revision, err := saveConfigAtRevision(candidate, expectedRevision)
	if err != nil {
		mylog.Info("Config save failed: %v", err)
		return "", err
	}
	s.rememberConfigModTime()
	candidate.SourceRevision = revision
	if s.callbacks.OnConfigChanged != nil {
		s.callbacks.OnConfigChanged(candidate)
	}
	return revision, nil
}

func (s *runtimeState) saveConfig() {
	if err := s.saveConfigErr(); err != nil {
		s.warnConfigSaveError(err)
	}
}

func (s *runtimeState) warnConfigSaveError(err error) {
	if errors.Is(err, config.ErrConfigRecoveryRequired) {
		msg := i18n.T(s.lang, "warning_config_recovery") + "\n\n" + err.Error()
		dialog.Warn(i18n.T(s.lang, "app_title"), "", msg)
		return
	}
	msg := fmt.Sprintf(i18n.T(s.lang, "msg_config_save_failed"), err.Error())
	dialog.Warn(i18n.T(s.lang, "app_title"), "", msg)
}

func (s *runtimeState) warnConfigLoadError(loadError string) {
	msg := i18n.T(s.lang, "warning_config_defaults") + "\n" +
		i18n.T(s.lang, "warning_config_recovery") + "\n\n" + loadError
	dialog.Warn(i18n.T(s.lang, "app_title"), "", msg)
}

func (s *runtimeState) showError(actionKey string, err error) {
	actionName := i18n.T(s.lang, actionKey)
	msg := fmt.Sprintf(i18n.T(s.lang, "msg_action_failed"), actionName, err.Error())
	dialog.Warn(i18n.T(s.lang, "app_title"), "", msg)
}
