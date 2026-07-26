package app

import (
	"os"
	"path/filepath"
	"time"

	"github.com/JeffioZ/idletrigger/internal/config"
	"github.com/JeffioZ/idletrigger/internal/feature/keepawake"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/powerstate"
)

func (s *runtimeState) applyLogging() {
	enabled := s.effectiveLoggingEnabled()
	if enabled == s.loggingActive {
		return
	}
	s.loggingActive = enabled
	if enabled {
		exeDir := os.TempDir()
		if exePath, err := os.Executable(); err == nil {
			exeDir = filepath.Dir(exePath)
		}
		mylog.Init(true, exeDir)
		mylog.Info("Debug logging enabled")
		ps := powerstate.GetStatus()
		s.logPowerState("logging-enabled", ps)
		mylog.Info("Runtime snapshot: nosleep_configured=%v automation_enabled=%v automation_rules=%d automation_active=%d wants_nosleep=%v battery_blocked=%v keepawake_enabled=%v keep_screen_on=%v idle_timeout_min=%d monitor_running=%v",
			s.cfg.NoSleepEnabled, s.cfg.AutomationEnabled, s.enabledAutomationCount(), len(s.autoState.ActiveRuleIDs),
			s.noSleepRequested(), batteryPolicyBlocks(s.cfg, ps), keepawake.IsEnabled(),
			keepawake.IsKeepingScreenOn(), s.cfg.IdleTimeoutMinutes, s.mon != nil)
		return
	}
	mylog.Info("Debug logging disabled")
	mylog.Close()
}

func (s *runtimeState) effectiveLoggingEnabled() bool {
	return s.cfg.LoggingEnabled || s.devtools.ForceLog
}

func (s *runtimeState) watchConfig() {
	cfgPath, err := config.Path()
	if err != nil {
		return
	}
	var lastMod time.Time
	if info, err := os.Stat(cfgPath); err == nil {
		lastMod = info.ModTime()
	}
	ticker := time.NewTicker(3 * time.Second)
	defer ticker.Stop()
	for range ticker.C {
		info, err := os.Stat(cfgPath)
		if err != nil {
			continue
		}
		modTime := info.ModTime()
		reload, nextLastMod := configWatchDecision(
			lastMod,
			modTime,
			s.selfConfigWrite.Load(),
			s.selfConfigMod.Load(),
		)
		lastMod = nextLastMod
		if !reload {
			continue
		}
		reloadError := s.call(func() string {
			if err := s.reloadConfig(); err != nil {
				return err.Error()
			}
			return ""
		})
		if reloadError != "" {
			mylog.Info("config reload failed; will retry: %s", reloadError)
			continue
		}
		lastMod = modTime
		mylog.Info("config reloaded from disk")
	}
}

func configWatchDecision(lastMod, modTime time.Time, selfWriting bool, selfMod int64) (reload bool, nextLastMod time.Time) {
	if !modTime.After(lastMod) {
		return false, lastMod
	}
	if selfWriting {
		return false, lastMod
	}
	if modTime.UnixNano() == selfMod {
		return false, modTime
	}
	return true, lastMod
}

func (s *runtimeState) rememberConfigModTime() {
	path, err := config.Path()
	if err != nil {
		return
	}
	if info, err := os.Stat(path); err == nil {
		s.selfConfigMod.Store(info.ModTime().UnixNano())
	}
}
