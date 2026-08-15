package app

import (
	"fmt"
	"time"

	"github.com/JeffioZ/idletrigger/internal/config"
	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	"github.com/JeffioZ/idletrigger/internal/i18n"
	"github.com/JeffioZ/idletrigger/internal/ui/controlpanel"
	"github.com/JeffioZ/idletrigger/internal/ui/settingspanel"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

func (s *runtimeState) restartThemeScheduler() {
	if !s.cfg.ThemeSwitchEnabled {
		return
	}
	s.startThemeScheduler()
}

func (s *runtimeState) refreshControlPanelThemeSchedule() {
	text := s.themeScheduleText(true)
	trayicon.Post(func() {
		controlpanel.UpdateThemeSchedule(text)
		settingspanel.UpdateLocationStatus(s.themeLocationStatusText())
	})
}

func (s *runtimeState) themeLocationStatusText() string {
	if s.cfg.ThemeMode != "sunrise" {
		return ""
	}
	loc := s.themeLocationInfo(false)
	if loc.Source == theme.LocationSourceIP && loc.LocationLabel != "" {
		return fmt.Sprintf(i18n.T(s.lang, "settings_location_ip_resolved"), loc.LocationLabel)
	}
	if s.cfg.ThemeIPLocationEnabled {
		return fmt.Sprintf(i18n.T(s.lang, "settings_location_ip_pending"), s.locationSourceShort(loc))
	}
	return fmt.Sprintf(i18n.T(s.lang, "settings_location_auto_status"), s.locationSourceShort(loc))
}

func (s *runtimeState) startIPLocationCycle() {
	s.cancelIPLocationRetry()
	s.ipLocationGeneration++
	s.ipLocationRetried = false
	if !s.themeIPLocationLookupEnabled() {
		return
	}
	theme.ResetIPLocationFailureCooldown()
	s.queryIPLocationInBackground(s.ipLocationGeneration)
}

func (s *runtimeState) stopIPLocationCycle() {
	s.cancelIPLocationRetry()
	s.ipLocationGeneration++
	s.ipLocationRetried = false
}

func (s *runtimeState) syncIPLocationCycle(wasEligible bool) {
	isEligible := s.themeIPLocationLookupEnabled()
	if !isEligible {
		s.stopIPLocationCycle()
		return
	}
	if !wasEligible {
		s.startIPLocationCycle()
	}
}

func (s *runtimeState) queryIPLocationInBackground(generation uint64) {
	go func() {
		loc := theme.AutoLocationInfo(true, true)
		s.post(func() {
			if generation != s.ipLocationGeneration || !s.themeIPLocationLookupEnabled() {
				return
			}
			if loc.Source != theme.LocationSourceIP {
				s.handleIPLocationFailure(generation)
				return
			}
			s.cancelIPLocationRetry()
			s.restartThemeScheduler()
			s.refreshControlPanelThemeSchedule()
			s.updateIcon()
		})
	}()
}

func ipLocationLookupEnabled(cfg config.Config) bool {
	return cfg.ThemeIPLocationEnabled && cfg.ThemeMode == "sunrise"
}

func (s *runtimeState) themeIPLocationLookupEnabled() bool {
	return s.themeAvailable() && ipLocationLookupEnabled(s.cfg)
}

func (s *runtimeState) handleIPLocationFailure(generation uint64) {
	if generation != s.ipLocationGeneration || s.ipLocationRetried {
		return
	}
	s.scheduleIPLocationRetry(generation)
}

func (s *runtimeState) scheduleIPLocationRetry(generation uint64) {
	s.cancelIPLocationRetry()
	var timer *time.Timer
	timer = time.AfterFunc(theme.IPLocationRetryInterval, func() {
		s.post(func() {
			if s.ipLocationRetry != timer || generation != s.ipLocationGeneration {
				return
			}
			s.ipLocationRetry = nil
			s.ipLocationRetried = true
			s.queryIPLocationInBackground(generation)
		})
	})
	s.ipLocationRetry = timer
}

func (s *runtimeState) cancelIPLocationRetry() {
	if s.ipLocationRetry == nil {
		return
	}
	s.ipLocationRetry.Stop()
	s.ipLocationRetry = nil
}
