package app

import (
	"fmt"
	"github.com/JeffioZ/idletrigger/internal/feature/keepawake"
	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	"github.com/JeffioZ/idletrigger/internal/i18n"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/resourceid"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
	"github.com/JeffioZ/idletrigger/internal/version"
	"strings"
	"time"
)

func (s *runtimeState) updateIcon() {
	darkTheme := theme.Current() == theme.ModeDark
	if darkTheme {
		trayicon.SetIconResource(resourceid.TrayLightIconID)
	} else {
		trayicon.SetIconResource(resourceid.TrayDarkIconID)
	}
	s.trayThemeDark = darkTheme
	trayicon.SetTooltip(s.buildTooltip())
}

func (s *runtimeState) refreshTrayThemeIcon() {
	if (theme.Current() == theme.ModeDark) != s.trayThemeDark {
		s.updateIcon()
	}
}

func (s *runtimeState) buildTooltip() string {
	lines := []string{tooltipTitle(version.Value)}
	lines = append(lines, s.statusLine("tooltip_nosleep", shortStatus(s.lang, keepawake.IsEnabled())))
	if s.idleSuspended() {
		lines = append(lines, s.statusLine("tooltip_idle", i18n.T(s.lang, "status_paused")))
	} else if s.idleAutomationPaused() {
		lines = append(lines, s.statusLine("tooltip_idle", i18n.T(s.lang, "status_paused")))
	} else if s.devtools.IdleMonitorEnabled() && s.mon != nil {
		lines = append(lines, s.statusLine("tooltip_idle", s.developerIdleMonitorStatus()))
	} else if s.mon != nil {
		threshold, _, action, _ := s.effectiveIdleMonitorSettings()
		lines = append(lines, s.statusLine("tooltip_idle", fmt.Sprintf("%d%s %s", int(threshold/time.Minute), shortMinuteUnit(s.lang), i18n.T(s.lang, actionTranslationKey(action)))))
	} else {
		lines = append(lines, s.statusLine("tooltip_idle", shortStatus(s.lang, false)))
	}
	lines = append(lines, s.statusLine("tooltip_theme", s.themeTooltipValueShort()))
	if count := s.enabledAutomationCount(); count > 0 {
		lines = append(lines, s.statusLine("tooltip_automation", fmt.Sprintf(i18n.T(s.lang, "status_automation_count"), count)))
	}
	return tooltipText(lines)
}

func tooltipTitle(appVersion string) string {
	if appVersion == "" || appVersion == "dev" {
		return "IdleTrigger"
	}
	return "IdleTrigger v" + appVersion
}

func shortStatus(lang string, enabled bool) string {
	if enabled {
		return i18n.T(lang, "status_short_on")
	}
	return i18n.T(lang, "status_short_off")
}

func shortMinuteUnit(lang string) string {
	if i18n.ResolveLanguage(lang) == "zh-CN" {
		return "分"
	}
	return "m"
}

func (s *runtimeState) themeTooltipValueShort() string {
	if !s.themeAvailable() {
		return i18n.T(s.lang, "status_unsupported")
	}
	if !s.cfg.ThemeSwitchEnabled {
		return i18n.T(s.lang, "status_short_off")
	}
	if schedule := s.themeScheduleTextShort(); schedule != "" {
		return i18n.T(s.lang, "status_short_on") + " " + schedule
	}
	return i18n.T(s.lang, "status_short_on")
}

func (s *runtimeState) themeScheduleText(showSource bool) string {
	return s.formatThemeSchedule(showSource, false)
}

func (s *runtimeState) themeScheduleTextShort() string {
	return s.formatThemeSchedule(false, true)
}

func (s *runtimeState) formatThemeSchedule(showSource, short bool) string {
	if !s.themeAvailable() {
		return i18n.T(s.lang, "theme_unavailable")
	}
	loc := theme.LocationInfo{Latitude: s.cfg.ThemeLatitude, Longitude: s.cfg.ThemeLongitude, Source: theme.LocationSourceConfigured}
	if s.cfg.ThemeMode == "sunrise" {
		loc = s.themeLocationInfo(false)
	}
	scheduleInfo := theme.ScheduleInfoFor(s.cfg.ThemeMode, s.cfg.ThemeLightTime, s.cfg.ThemeDarkTime, loc.Latitude, loc.Longitude, time.Now())
	if !scheduleInfo.OK {
		return i18n.T(s.lang, "theme_schedule_unavailable")
	}
	formatKey := "theme_schedule_format"
	fallbackKey := "theme_schedule_fallback_format"
	if s.cfg.ThemeMode == "sunrise" {
		formatKey = "theme_schedule_sunrise_format"
	}
	if short {
		formatKey = "theme_schedule_short_format"
		fallbackKey = "theme_schedule_short_fallback_format"
		if s.cfg.ThemeMode == "sunrise" {
			formatKey = "theme_schedule_sunrise_short_format"
		}
	}
	schedule := fmt.Sprintf(i18n.T(s.lang, formatKey), scheduleInfo.LightTime, scheduleInfo.DarkTime)
	if scheduleInfo.FixedFallback {
		return fmt.Sprintf(i18n.T(s.lang, fallbackKey), schedule)
	}
	if s.cfg.ThemeMode != "sunrise" || !showSource {
		return schedule
	}
	return fmt.Sprintf(i18n.T(s.lang, "theme_schedule_source_format"), schedule, s.locationSourceShort(loc))
}

func (s *runtimeState) locationSourceShort(loc theme.LocationInfo) string {
	switch loc.Source {
	case theme.LocationSourceConfigured:
		return i18n.T(s.lang, "theme_location_configured")
	case theme.LocationSourceIP:
		return i18n.T(s.lang, "theme_location_ip")
	case theme.LocationSourceTimezone:
		return i18n.T(s.lang, "theme_location_timezone")
	case theme.LocationSourceUTCOffset:
		return i18n.T(s.lang, "theme_location_utc_offset")
	default:
		return i18n.T(s.lang, "theme_location_default")
	}
}

func (s *runtimeState) themeLocationInfo(blockIPLookup bool) theme.LocationInfo {
	lat, lon := s.cfg.ThemeLatitude, s.cfg.ThemeLongitude
	if lat != 0 || lon != 0 {
		return theme.LocationInfo{Latitude: lat, Longitude: lon, Source: theme.LocationSourceConfigured}
	}
	return theme.AutoLocationInfo(s.cfg.ThemeIPLocationEnabled, blockIPLookup)
}

func (s *runtimeState) ipLocationLabel() string {
	if !s.cfg.ThemeIPLocationEnabled {
		return ""
	}
	loc := s.themeLocationInfo(false)
	if loc.Source == theme.LocationSourceIP {
		return loc.LocationLabel
	}
	return ""
}

func tooltipText(lines []string) string {
	clean := make([]string, 0, len(lines))
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line != "" {
			clean = append(clean, line)
		}
	}
	return strings.Join(clean, "\n")
}

// ---- idle monitor -----------------------------------------------------
