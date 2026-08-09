package app

import (
	"reflect"
	"testing"

	"github.com/JeffioZ/idletrigger/internal/config"
)

func TestEverySerializedConfigFieldHasAUIOwner(t *testing.T) {
	owners := map[string]string{
		"language": "settings", "idle_enabled": "control panel", "idle_timeout_minutes": "settings", "idle_action": "settings",
		"idle_warning_seconds": "settings", "idle_enhanced_monitor": "settings", "nosleep_enabled": "control panel",
		"keep_screen_on": "settings", "nosleep_on_battery": "settings", "nosleep_battery_threshold": "settings",
		"hotkeys_enabled": "settings", "automation_enabled": "control panel", "automation_rules": "automation manager",
		"logging_enabled": "settings", "theme_switch_enabled": "control panel", "theme_light_time": "settings",
		"theme_dark_time": "settings", "theme_mode": "settings", "theme_ip_location_enabled": "settings",
		"theme_dark_on_battery": "settings", "theme_skip_fullscreen": "settings",
	}
	typeOfConfig := reflect.TypeOf(config.Config{})
	for index := 0; index < typeOfConfig.NumField(); index++ {
		field := typeOfConfig.Field(index)
		key := field.Tag.Get("toml")
		if key == "" || key == "-" {
			continue
		}
		if owners[key] == "" {
			t.Errorf("serialized config field %s (%s) has no declared UI owner", field.Name, key)
		}
	}
	for key := range owners {
		found := false
		for index := 0; index < typeOfConfig.NumField(); index++ {
			if typeOfConfig.Field(index).Tag.Get("toml") == key {
				found = true
				break
			}
		}
		if !found {
			t.Errorf("UI coverage table contains stale config key %q", key)
		}
	}
}

func TestSettingsPanelStateMapsDetailedConfig(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.KeepScreenOn, cfg.NoSleepOnBattery, cfg.NoSleepBatteryThreshold = true, true, 42
	cfg.IdleWarningSeconds, cfg.ThemeMode = 75, "sunrise"
	cfg.IdleEnabled, cfg.IdleTimeoutMinutes, cfg.IdleAction, cfg.IdleEnhancedMonitor = true, 45, config.ActionLock, true
	cfg.ThemeLightTime, cfg.ThemeDarkTime = "06:12", "20:34"
	cfg.ThemeIPLocationEnabled = true
	cfg.SourceRevision = "revision"
	cfg.ThemeDarkOnBattery, cfg.ThemeSkipFullscreen = true, true
	cfg.Language, cfg.HotkeysEnabled, cfg.AutostartEnabled, cfg.LoggingEnabled = "zh-CN", true, true, true
	state := (&runtimeState{cfg: cfg, lang: "zh-CN"}).settingsPanelState()
	if !state.KeepScreenOn || !state.NoSleepOnBattery || state.NoSleepBatteryThreshold != 42 || !state.IdleEnabled || state.IdleTimeoutMinutes != 45 ||
		state.IdleAction != "lock" || !state.IdleEnhancedMonitor || state.IdleWarningSeconds != 75 ||
		state.ThemeMode != "sunrise" || state.ThemeLightTime != "06:12" || state.ThemeDarkTime != "20:34" ||
		!state.ThemeIPLocationEnabled ||
		!state.ThemeDarkOnBattery || !state.ThemeSkipFullscreen || state.Language != "zh-CN" || !state.HotkeysEnabled ||
		!state.AutostartEnabled || !state.LoggingEnabled || state.Revision != "revision" || !state.Chinese {
		t.Fatalf("settings state did not preserve config: %+v", state)
	}
}
