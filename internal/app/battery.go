package app

import (
	"fmt"
	"time"

	"github.com/JeffioZ/idletrigger/internal/config"
	"github.com/JeffioZ/idletrigger/internal/feature/keepawake"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/powerstate"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

// syncBatteryLoop keeps the periodic battery poll dormant when neither
// battery-aware feature is enabled. Windows power broadcasts still deliver
// immediate changes; this loop catches battery-percentage threshold crossings.
func (s *runtimeState) syncBatteryLoop() {
	if s.noSleepRequested() || (s.cfg.ThemeSwitchEnabled && s.themeAvailable()) {
		if s.batteryStop != nil {
			return
		}
		s.batteryStop = make(chan struct{})
		s.batteryDone = make(chan struct{})
		go s.batteryLoop(s.batteryStop, s.batteryDone)
		return
	}
	s.stopBatteryLoop()
}

func (s *runtimeState) stopBatteryLoop() {
	if s.batteryStop == nil {
		return
	}
	close(s.batteryStop)
	<-s.batteryDone
	s.batteryStop = nil
	s.batteryDone = nil
}

// cancelDelayedBatteryRead stops and clears the coalesced post-power-event
// timer. The callback never fires twice and the timer field is owned by the
// serialized request loop, matching cancelIPLocationRetry's lifecycle.
func (s *runtimeState) cancelDelayedBatteryRead() {
	if s.delayedBatteryRead == nil {
		return
	}
	s.delayedBatteryRead.Stop()
	s.delayedBatteryRead = nil
}

// batteryLoop is a low-frequency fallback for firmware or drivers that miss
// registered AC/DC and battery-percentage notifications.
func (s *runtimeState) batteryLoop(stopCh <-chan struct{}, doneCh chan<- struct{}) {
	ticker := time.NewTicker(30 * time.Second)
	defer ticker.Stop()
	defer close(doneCh)
	for {
		select {
		case <-stopCh:
			return
		case <-ticker.C:
			select {
			case s.requestCh <- runtimeRequest{fn: func() string {
				s.refreshBatteryPolicy()
				return ""
			}}:
			case <-stopCh:
				return
			}
		}
	}
}

func (s *runtimeState) refreshBatteryPolicy() {
	s.refreshBatteryPolicyWithStatus(powerstate.GetStatus())
}

func (s *runtimeState) refreshBatteryPolicyWithStatus(ps powerstate.Status) bool {
	powerSourceChanged := ps.Valid && (!s.powerStatusKnown || powerStatusSourceChanged(s.lastPowerStatus, ps))
	s.lastPowerStatus = ps
	if ps.Valid {
		s.powerStatusKnown = true
	}
	blocked := batteryPolicyBlocks(s.cfg, ps)
	if powerSourceChanged && s.themeSched != nil {
		s.themeSched.CheckNow()
	}
	if blocked == s.batteryBlocked {
		return false
	}
	s.batteryBlocked = blocked
	mylog.Info("Battery policy changed: nosleep_blocked=%v reason=%s ac_line=%v battery=%v percent=%d charging=%v valid=%v",
		blocked, batteryPolicyReason(s.cfg, ps), ps.ACLine, ps.Battery, ps.Percent, ps.Charging, ps.Valid)
	s.reconcileRuntime()

	s.updateIcon()
	return true
}

func (s *runtimeState) handlePowerEvent(event trayicon.PowerEvent) {
	ps := powerstate.GetStatus()
	s.logPowerState(fmt.Sprintf("event:%s(0x%04x) setting=%s value=%s", powerEventName(event.Code), event.Code,
		powerSettingName(event.Setting), powerSettingValue(event)), ps)
	changed := s.refreshBatteryPolicyWithStatus(ps)
	if event.Setting == trayicon.PowerSettingACSource || event.Code == pbtAPMPowerStatusChange {
		if s.themeSched != nil {
			s.themeSched.CheckNow()
		}
		// Some drivers broadcast the setting just before GetSystemPowerStatus
		// converges. One delayed read avoids waiting for the polling fallback.
		// A single timer is reset on burst events so they cannot pile up.
		if s.delayedBatteryRead != nil {
			s.delayedBatteryRead.Stop()
		}
		s.delayedBatteryRead = time.AfterFunc(time.Second, func() {
			if s.exiting.Load() {
				return
			}
			s.post(s.refreshBatteryPolicy)
		})
	}
	if isResumePowerEvent(event.Code) {
		if s.themeSched != nil {
			s.themeSched.CheckNow()
		}
		s.requestThemeEnvironmentRecovery(themeSourceResume)
	}
	if isResumePowerEvent(event.Code) && !changed {
		if s.noSleepRequested() && !s.batteryBlocked {
			mylog.Info("Power resume: reasserting effective Stay Awake request")
		}
		s.reconcileRuntime()
	}
}

func powerStatusSourceChanged(previous, current powerstate.Status) bool {
	if !current.Valid {
		return false
	}
	if !previous.Valid {
		return true
	}
	return previous.ACLine != current.ACLine || previous.Battery != current.Battery
}

func (s *runtimeState) logPowerState(source string, ps powerstate.Status) {
	mylog.Info("Power state: source=%s ac_line=%v battery=%v percent=%d charging=%v valid=%v nosleep_configured=%v wants_nosleep=%v nosleep_blocked=%v keepawake_enabled=%v keep_screen_on=%v reason=%s",
		source, ps.ACLine, ps.Battery, ps.Percent, ps.Charging, ps.Valid,
		s.cfg.NoSleepEnabled, s.noSleepRequested(), batteryPolicyBlocks(s.cfg, ps),
		keepawake.IsEnabled(), keepawake.IsKeepingScreenOn(), batteryPolicyReason(s.cfg, ps))
}

func batteryPolicyBlocks(cfg config.Config, status powerstate.Status) bool {
	if !status.Valid || !status.Battery || status.ACLine {
		return false
	}
	return !cfg.NoSleepOnBattery ||
		(status.Percent >= 0 && status.Percent < cfg.NoSleepBatteryThreshold)
}

func batteryPolicyReason(cfg config.Config, status powerstate.Status) string {
	if !status.Valid {
		return "power-status-unknown"
	}
	if status.ACLine || !status.Battery {
		return "ac-or-no-battery"
	}
	if !cfg.NoSleepOnBattery {
		return "battery-not-allowed"
	}
	if status.Percent >= 0 && status.Percent < cfg.NoSleepBatteryThreshold {
		return "battery-below-threshold"
	}
	return "battery-allowed"
}

// ---- IPC handler ------------------------------------------------------
