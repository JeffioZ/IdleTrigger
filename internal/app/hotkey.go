package app

import (
	"github.com/JeffioZ/idletrigger/internal/config"
	"github.com/JeffioZ/idletrigger/internal/feature/keepawake"
	"github.com/JeffioZ/idletrigger/internal/i18n"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/dialog"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/hotkey"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/systemaction"
)

func (s *runtimeState) startHotkeys() {
	s.stopHotkeys()
	mgr := hotkey.NewManager(hotkey.DefaultBindings(), hotkey.Callbacks{
		OnSleep: func() {
			s.post(func() {
				if err := s.executeAction(config.ActionSleep); err != nil {
					s.showError("menu_sleep", err)
				}
			})
		},
		OnLock: func() {
			s.post(func() {
				if err := systemaction.Lock(); err != nil {
					s.showError("menu_lock", err)
				}
			})
		},
		OnToggleNoSleep: func() { s.post(func() { s.toggleNoSleep() }) },
	})
	s.hotkeyMgr = mgr
	failed := mgr.Start()
	if len(failed) > 0 {
		s.showHotkeyConflict(failed)
	}
}

func (s *runtimeState) stopHotkeys() {
	if s.hotkeyMgr != nil {
		s.hotkeyMgr.Stop()
		s.hotkeyMgr = nil
	}
}

func (s *runtimeState) showHotkeyConflict(failed hotkey.Failed) {
	body := ""
	for _, f := range failed {
		body += "• " + f + "\n"
	}
	dialog.Warn(
		i18n.T(s.lang, "app_title"),
		i18n.T(s.lang, "msg_hotkey_conflict"),
		body,
	)
}

func (s *runtimeState) toggleNoSleep() {
	s.setNoSleep(!s.cfg.NoSleepEnabled, s.cfg.KeepScreenOn)
	mylog.Info("NoSleep toggled: enabled=%v keep_screen_on=%v", keepawake.IsEnabled(), keepawake.IsKeepingScreenOn())
	s.saveConfig()
}

func (s *runtimeState) setNoSleep(enabled, keepScreenOn bool) {
	setNoSleepConfig(&s.cfg, enabled, keepScreenOn)
	s.reconcileRuntime()
	s.updateIcon()
}

func (s *runtimeState) setIdleEnabled(enabled bool) {
	setIdleEnabledConfig(&s.cfg, enabled)
	s.reconcileRuntime()
	s.updateIcon()
}

func setNoSleepConfig(cfg *config.Config, enabled, keepScreenOn bool) {
	cfg.NoSleepEnabled = enabled
	if enabled {
		cfg.KeepScreenOn = keepScreenOn
		cfg.IdleEnabled = false
	}
}

func setIdleEnabledConfig(cfg *config.Config, enabled bool) {
	cfg.IdleEnabled = enabled
	if enabled {
		cfg.NoSleepEnabled = false
	}
}
