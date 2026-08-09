package app

import (
	"fmt"
	"github.com/JeffioZ/idletrigger/internal/config"
	"github.com/JeffioZ/idletrigger/internal/feature/idle"
	"github.com/JeffioZ/idletrigger/internal/feature/keepawake"
	"github.com/JeffioZ/idletrigger/internal/i18n"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/ui/idlewarning"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
	"time"
)

func (s *runtimeState) startMonitor() {
	s.stopMonitor()
	if !s.idleMonitorRequested() {
		return
	}
	threshold, warnOffset, action, developerTest := s.effectiveIdleMonitorSettings()
	if developerTest {
		mylog.Info("Developer tools idle-monitor test active: effective_threshold=%s effective_action=lock effective_warning=%s config_unchanged=true", threshold, warnOffset)
	}
	snap, snapErr := idle.Snapshot()
	if snapErr != nil {
		mylog.Info("Idle monitoring starting: config_timeout_min=%d effective_threshold=%s warning=%s action=%s idle_snapshot_error=%v",
			s.cfg.IdleTimeoutMinutes, threshold, warnOffset, string(action), snapErr)
	} else {
		mylog.Info("Idle monitoring starting: config_timeout_min=%d effective_threshold=%s warning=%s action=%s tick_now=%d tick32=%d last_input=%d raw_delta_ms=%d idle=%s",
			s.cfg.IdleTimeoutMinutes, threshold, warnOffset, string(action), snap.NowTick64, snap.NowTick32, snap.LastInputTick, snap.RawDeltaMS, snap.Idle.Round(time.Second))
	}
	mylog.Info("Idle monitoring input policy: enhanced_idle_monitor=%v periodic_window=%s..%s periodic_tolerance=%s periodic_required=%d",
		s.cfg.IdleEnhancedMonitor, 20*time.Second, 2*time.Minute, 5*time.Second, 3)
	lang := s.lang
	warningSeconds := int(warnOffset / time.Second)
	noSleepEnabled := s.noSleepRequested()
	keepScreenOn := s.effectiveKeepScreenOn()
	automationActiveCount := len(s.autoState.ActiveRuleIDs)
	enhancedIdleMonitor := s.cfg.IdleEnhancedMonitor
	sampleLogInterval := 5 * time.Second
	if developerTest {
		sampleLogInterval = 2 * time.Second
	}
	var lastSampleLog time.Time
	var lastEffectiveIdle time.Duration
	idlewarning.SetOnDismiss(func() {
		s.post(func() {
			if s.mon != nil && s.idleMonitorRequested() {
				s.startMonitor()
			}
		})
	})

	s.mon = idle.New(threshold, warnOffset,
		// Show a non-activating in-app warning. Legacy notification-area
		// balloons are silently suppressed on many Windows 11 configurations.
		func() {
			actName := i18n.T(lang, actionTranslationKey(action))
			title := i18n.T(lang, "idle_warning_title")
			idlewarning.SetLanguage(i18n.ResolveLanguage(lang) == "zh-CN")
			mylog.Info("Idle warning displayed: action=%s seconds=%d", action, warningSeconds)
			idlewarning.ShowCountdown(title, warningSeconds, func(remaining int) string {
				if remaining < 0 {
					remaining = 0
				}
				return fmt.Sprintf(i18n.T(lang, "msg_idle_warning"), actName, remaining)
			})
		},
		// Execute directly from the monitor goroutine. Waiting for the tray
		// state queue would make an overdue power action depend on UI work.
		func() {
			if s.exiting.Load() {
				return
			}
			// The warning must be gone before a power action takes effect.
			trayicon.PostAndWait(idlewarning.Hide)
			if s.exiting.Load() {
				return
			}
			idleFor, idleErr := idle.IdleDuration()
			if idleErr != nil {
				mylog.Info("Idle monitoring trigger reached: effective_idle=%s action=%s raw_idle_after_trigger_error=%v",
					lastEffectiveIdle.Round(time.Second), action, idleErr)
			} else {
				mylog.Info("Idle monitoring trigger reached: effective_idle=%s action=%s raw_idle_after_trigger=%s",
					lastEffectiveIdle.Round(time.Second), action, idleFor.Round(time.Second))
			}
			if err := executeActionWithLanguage(action, lang); err != nil {
				mylog.Info("Idle monitoring action failed: action=%s error=%v", action, err)
				if !s.exiting.Load() {
					s.post(func() { s.showError(actionTranslationKey(action), err) })
				}
				return
			}
			mylog.Info("Idle monitoring action accepted: action=%s", action)
		},
		time.Second,
	)
	s.mon.SetEnhancedIdleMonitor(enhancedIdleMonitor)
	s.mon.SetOnActivity(func() { trayicon.Post(idlewarning.Hide) })
	s.mon.SetOnInputReset(func(reset idle.InputReset) {
		mylog.Info("Idle monitoring input reset: previous_last_input=%d last_input=%d session_idle_before_reset=%s threshold=%s warn_at=%s was_warned=%v was_triggered=%v ignored=%v reason=%s periodic_count=%d periodic_baseline=%s enhanced_idle_monitor=%v action=%s source=GetLastInputInfo",
			reset.PreviousLastInputTick, reset.LastInputTick, reset.SessionIdleBeforeReset.Round(time.Millisecond),
			reset.Threshold, reset.WarnAt, reset.WasWarned, reset.WasTriggered, reset.Ignored, reset.Reason,
			reset.PeriodicCount, reset.PeriodicBaseline.Round(time.Millisecond), enhancedIdleMonitor, action)
	})
	s.mon.SetOnSample(func(sample idle.Sample) {
		lastEffectiveIdle = sample.Idle
		now := time.Now()
		if !lastSampleLog.IsZero() && now.Sub(lastSampleLog) < sampleLogInterval {
			return
		}
		lastSampleLog = now
		if snap, err := idle.Snapshot(); err == nil {
			mylog.Info("Idle monitoring sample: effective_idle=%s raw_idle=%s raw_delta_ms=%d threshold=%s warn_at=%s last_input=%d tick_now=%d clamped_to_start=%v warned=%v triggered=%v input_timestamp=%v enhanced_idle_monitor=%v nosleep_requested=%v keep_screen_on=%v automation_rules_active=%d",
				sample.Idle.Round(time.Millisecond), snap.Idle.Round(time.Millisecond), snap.RawDeltaMS,
				sample.Threshold, sample.WarnAt, sample.LastInputTick, snap.NowTick64,
				sample.StartWindowClamped, sample.Warned, sample.Triggered, sample.InputTimestampAvailable,
				enhancedIdleMonitor, noSleepEnabled, keepScreenOn, automationActiveCount)
		} else {
			mylog.Info("Idle monitoring sample: effective_idle=%s threshold=%s warn_at=%s last_input=%d clamped_to_start=%v warned=%v triggered=%v input_timestamp=%v snapshot_error=%v",
				sample.Idle.Round(time.Millisecond), sample.Threshold, sample.WarnAt, sample.LastInputTick,
				sample.StartWindowClamped, sample.Warned, sample.Triggered, sample.InputTimestampAvailable, err)
		}
	})
	s.mon.Start()
}

func (s *runtimeState) stopMonitor() {
	idlewarning.SetOnDismiss(nil)
	trayicon.Post(idlewarning.Hide)
	if s.mon != nil {
		s.mon.Stop()
		s.mon = nil
		mylog.Info("Idle monitoring stopped")
	}
}

func (s *runtimeState) reconcileRuntime() {
	defer s.refreshControlPanelPowerStatus()
	s.syncBatteryLoop()
	wantsNoSleep := s.noSleepRequested()
	keepScreenOn := s.effectiveKeepScreenOn()
	mylog.Info("Runtime reconcile: nosleep_manual=%v nosleep_automation=%v automation_pause_nosleep=%v wants_nosleep=%v keep_screen_on=%v battery_blocked=%v idle_timeout_min=%d automation_idle=%v automation_pause_idle=%v monitor_running=%v",
		s.cfg.NoSleepEnabled, s.autoState.StayAwake, s.autoState.PauseStayAwake, wantsNoSleep, keepScreenOn, s.batteryBlocked, s.cfg.IdleTimeoutMinutes, s.autoState.EnableIdle, s.autoState.PauseIdle, s.mon != nil)
	if wantsNoSleep && !s.batteryBlocked {
		if s.devtools.IdleMonitorEnabled() {
			mylog.Info("Developer tools idle-monitor test suppressed: Stay Awake remains mutually exclusive; config_unchanged=true")
		}
		s.stopMonitor()
		if err := keepawake.Enable(keepScreenOn); err != nil {
			mylog.Info("Stay Awake enable failed: api=SetThreadExecutionState continuous=true system_required=true display_required=%v error=%v", keepScreenOn, err)
			s.showError("menu_nosleep", err)
		} else {
			mylog.Info("Stay Awake enabled: api=SetThreadExecutionState continuous=true system_required=true display_required=%v", keepScreenOn)
		}
		return
	}

	keepawake.Disable()
	if s.idleMonitorRequested() {
		s.startMonitor()
	} else {
		s.stopMonitor()
	}
}

func (s *runtimeState) noSleepRequested() bool {
	return (s.cfg.NoSleepEnabled || s.autoState.StayAwake) && !s.autoState.PauseStayAwake
}

func (s *runtimeState) effectiveKeepScreenOn() bool {
	return !s.autoState.PauseStayAwake && ((s.cfg.NoSleepEnabled && s.cfg.KeepScreenOn) || (s.autoState.StayAwake && s.autoState.KeepScreenOn))
}

func (s *runtimeState) noSleepAutomationPaused() bool {
	return s.autoState.PauseStayAwake && (s.cfg.NoSleepEnabled || s.autoState.StayAwake)
}

// idleSuspended reports the intentional conflict resolution between effective
// keep-awake and the idle action.
func (s *runtimeState) idleSuspended() bool {
	return s.idleMonitorDemanded() && s.noSleepRequested() && s.mon == nil
}

func (s *runtimeState) idleMonitorRequested() bool {
	return s.idleMonitorDemanded() && !s.autoState.PauseIdle
}

func (s *runtimeState) idleMonitorDemanded() bool {
	return s.cfg.IdleEnabled || s.autoState.EnableIdle || s.devtools.IdleMonitorEnabled()
}

func (s *runtimeState) idleAutomationPaused() bool {
	return s.idleMonitorDemanded() && s.autoState.PauseIdle && !s.noSleepRequested() && s.mon == nil
}

func (s *runtimeState) effectiveIdleMonitorSettings() (time.Duration, time.Duration, config.Action, bool) {
	if s.devtools.IdleMonitorEnabled() {
		return time.Duration(s.devtools.IdleMonitorSeconds) * time.Second, 5 * time.Second, config.ActionLock, true
	}
	minutes := 0
	if s.cfg.IdleEnabled {
		minutes = s.cfg.IdleTimeoutMinutes
	}
	if minutes <= 0 && s.autoState.EnableIdle {
		minutes = s.autoState.IdleMinutes
	}
	return time.Duration(minutes) * time.Minute, time.Duration(s.cfg.IdleWarningSeconds) * time.Second, s.cfg.IdleAction, false
}

func (s *runtimeState) developerIdleMonitorStatus() string {
	return fmt.Sprintf(i18n.T(s.lang, "status_monitor_test_active"), s.devtools.IdleMonitorSeconds, i18n.T(s.lang, "menu_action_lock"))
}

// ---- hotkeys ----------------------------------------------------------
