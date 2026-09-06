package app

import (
	"errors"
	"fmt"

	"github.com/JeffioZ/idletrigger/internal/config"
	"github.com/JeffioZ/idletrigger/internal/i18n"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/autostart"
	"github.com/JeffioZ/idletrigger/internal/ui/controlpanel"
	"github.com/JeffioZ/idletrigger/internal/ui/settingspanel"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
	"github.com/JeffioZ/idletrigger/internal/version"
)

func (s *runtimeState) showSettingsPanel() {
	state := s.settingsPanelState()
	lang := s.lang
	trayicon.Post(func() {
		err := settingspanel.Show(state, func(request settingspanel.SaveRequest) settingspanel.SaveResult {
			return s.requestSettingsSave(request)
		}, func() { s.refreshControlPanel() },
			func() { s.post(func() { s.openProjectHome() }) },
			func(key string) string { return i18n.T(lang, key) })
		if err != nil {
			mylog.Info("Settings panel open failed: %v", err)
		}
	})
}

func (s *runtimeState) settingsPanelState() settingspanel.State {
	return settingspanel.State{
		KeepScreenOn: s.cfg.KeepScreenOn, NoSleepOnBattery: s.cfg.NoSleepOnBattery,
		NoSleepBatteryThreshold: s.cfg.NoSleepBatteryThreshold, IdleEnabled: s.cfg.IdleEnabled, IdleTimeoutMinutes: s.cfg.IdleTimeoutMinutes,
		IdleAction: string(s.cfg.IdleAction), IdleWarningSeconds: s.cfg.IdleWarningSeconds,
		IdleEnhancedMonitor: s.cfg.IdleEnhancedMonitor,
		ThemeMode:           s.cfg.ThemeMode, ThemeLightTime: s.cfg.ThemeLightTime, ThemeDarkTime: s.cfg.ThemeDarkTime,
		ThemeIPLocationEnabled: s.cfg.ThemeIPLocationEnabled, ThemeLocationStatus: s.themeLocationStatusText(), ThemeDarkOnBattery: s.cfg.ThemeDarkOnBattery,
		ThemeSkipFullscreen: s.cfg.ThemeSkipFullscreen, Language: s.cfg.Language,
		LockKeysEnabled:        s.cfg.LockKeysEnabled,
		LockKeysCapsEnabled:    s.cfg.LockKeysCapsEnabled,
		LockKeysNumEnabled:     s.cfg.LockKeysNumEnabled,
		LockKeysScrollEnabled:  s.cfg.LockKeysScrollEnabled,
		LockKeysSkipFullscreen: s.cfg.LockKeysSkipFullscreen,
		HotkeysEnabled:         s.cfg.HotkeysEnabled, AutostartEnabled: s.cfg.AutostartEnabled, LoggingEnabled: s.cfg.LoggingEnabled,
		Version: version.Value, Revision: s.cfg.SourceRevision,
		Chinese: i18n.ResolveLanguage(s.lang) == "zh-CN", Owner: controlpanel.WindowHandle(),
	}
}

func (s *runtimeState) requestSettingsSave(request settingspanel.SaveRequest) settingspanel.SaveResult {
	result := make(chan settingspanel.SaveResult, 1)
	s.post(func() { result <- s.saveSettings(request) })
	return <-result
}

func (s *runtimeState) saveSettings(request settingspanel.SaveRequest) settingspanel.SaveResult {
	if request.BaseRevision != s.cfg.SourceRevision {
		return settingspanel.SaveResult{State: s.settingsPanelState(), Error: i18n.T(s.lang, "settings_save_conflict")}
	}
	candidate := s.cfg
	candidate.KeepScreenOn = request.KeepScreenOn
	candidate.NoSleepOnBattery = request.NoSleepOnBattery
	candidate.NoSleepBatteryThreshold = request.NoSleepBatteryThreshold
	candidate.IdleEnabled = request.IdleEnabled
	candidate.IdleTimeoutMinutes = request.IdleTimeoutMinutes
	candidate.IdleAction = config.Action(request.IdleAction)
	candidate.IdleWarningSeconds = request.IdleWarningSeconds
	candidate.IdleEnhancedMonitor = request.IdleEnhancedMonitor
	candidate.ThemeMode = request.ThemeMode
	candidate.ThemeLightTime = request.ThemeLightTime
	candidate.ThemeDarkTime = request.ThemeDarkTime
	candidate.ThemeIPLocationEnabled = request.ThemeIPLocationEnabled
	candidate.ThemeDarkOnBattery = request.ThemeDarkOnBattery
	candidate.ThemeSkipFullscreen = request.ThemeSkipFullscreen
	candidate.Language = request.Language
	candidate.LockKeysEnabled = request.LockKeysEnabled
	candidate.LockKeysCapsEnabled = request.LockKeysCapsEnabled
	candidate.LockKeysNumEnabled = request.LockKeysNumEnabled
	candidate.LockKeysScrollEnabled = request.LockKeysScrollEnabled
	candidate.LockKeysSkipFullscreen = request.LockKeysSkipFullscreen
	candidate.HotkeysEnabled = request.HotkeysEnabled
	candidate.AutostartEnabled = request.AutostartEnabled
	candidate.LoggingEnabled = request.LoggingEnabled
	if err := candidate.Validate(); err != nil {
		return settingspanel.SaveResult{State: s.settingsPanelState(), Error: fmt.Sprintf(i18n.T(s.lang, "settings_save_failed"), err.Error())}
	}
	wasIPLocationEligible := s.themeIPLocationLookupEnabled()
	previous := s.cfg
	if candidate.AutostartEnabled != previous.AutostartEnabled {
		if err := setAutostart(candidate.AutostartEnabled); err != nil {
			return settingspanel.SaveResult{State: s.settingsPanelState(), Error: fmt.Sprintf(i18n.T(s.lang, "settings_save_failed"), err.Error())}
		}
	}
	revision, err := s.persistConfigAtRevision(candidate, request.BaseRevision)
	if err != nil {
		if candidate.AutostartEnabled != previous.AutostartEnabled {
			if rollbackErr := setAutostart(previous.AutostartEnabled); rollbackErr != nil {
				mylog.Info("Autostart rollback failed: %v", rollbackErr)
			}
		}
		if errors.Is(err, config.ErrConfigChanged) {
			if reloadErr := s.reloadConfig(); reloadErr != nil {
				mylog.Info("Settings save conflict reload failed: %v", reloadErr)
			}
			return settingspanel.SaveResult{State: s.settingsPanelState(), Error: i18n.T(s.lang, "settings_save_conflict")}
		}
		return settingspanel.SaveResult{State: s.settingsPanelState(), Error: fmt.Sprintf(i18n.T(s.lang, "settings_save_failed"), err.Error())}
	}
	candidate.SourceRevision = revision
	s.cfg = candidate
	if candidate.HotkeysEnabled != previous.HotkeysEnabled {
		if candidate.HotkeysEnabled {
			s.startHotkeys()
		} else {
			s.stopHotkeys()
		}
	}
	if candidate.LoggingEnabled != previous.LoggingEnabled {
		s.applyLogging()
	}
	if candidate.Language != previous.Language {
		s.lang = candidate.Language
		s.applyLanguage()
		mylog.Info("Language switched: %s", candidate.Language)
	}
	s.batteryBlocked = false
	s.refreshBatteryPolicy()
	s.stopThemeScheduler()
	if s.cfg.ThemeSwitchEnabled {
		s.startThemeScheduler()
	}
	s.syncIPLocationCycle(wasIPLocationEligible)
	s.reconcileRuntime()
	s.refreshControlPanelThemeSchedule()
	s.updateIcon()
	return settingspanel.SaveResult{State: s.settingsPanelState()}
}

func setAutostart(enabled bool) error {
	if enabled {
		return autostart.Enable()
	}
	return autostart.Disable()
}
