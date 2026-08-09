package config

import (
	"fmt"
	"time"

	"github.com/JeffioZ/idletrigger/internal/automation"
)

// NormalizeConfig returns a copy of cfg with invalid fields replaced by defaults.
func NormalizeConfig(cfg Config) Config {
	d := DefaultConfig()
	if cfg.Language != "auto" && cfg.Language != "en" && cfg.Language != "zh-CN" {
		cfg.Language = d.Language
	}
	if cfg.IdleTimeoutMinutes < 1 || cfg.IdleTimeoutMinutes > 7*24*60 {
		cfg.IdleTimeoutMinutes = d.IdleTimeoutMinutes
	}
	if !ValidIdleAction(cfg.IdleAction) {
		cfg.IdleAction = d.IdleAction
	}
	if cfg.IdleWarningSeconds < 0 || cfg.IdleWarningSeconds > 3600 {
		cfg.IdleWarningSeconds = d.IdleWarningSeconds
	}
	if cfg.NoSleepBatteryThreshold < 0 || cfg.NoSleepBatteryThreshold > 100 {
		cfg.NoSleepBatteryThreshold = d.NoSleepBatteryThreshold
	}
	cfg.AutomationRules, cfg.AutomationIssues = automation.PrepareRules(cfg.AutomationRules)
	if cfg.ThemeMode != "fixed" && cfg.ThemeMode != "sunrise" {
		cfg.ThemeMode = d.ThemeMode
	}
	if _, err := time.Parse("15:04", cfg.ThemeLightTime); err != nil {
		cfg.ThemeLightTime = d.ThemeLightTime
	}
	if _, err := time.Parse("15:04", cfg.ThemeDarkTime); err != nil {
		cfg.ThemeDarkTime = d.ThemeDarkTime
	}
	return cfg
}

// Validate checks values that can otherwise lead to unsafe or surprising
// runtime behavior.
func (cfg Config) Validate() error {
	if cfg.LoadError != "" {
		return fmt.Errorf("%w: %s", ErrConfigRecoveryRequired, cfg.LoadError)
	}
	switch cfg.Language {
	case "auto", "en", "zh-CN":
	default:
		return fmt.Errorf("language must be auto, en, or zh-CN")
	}
	if !ValidIdleAction(cfg.IdleAction) {
		return fmt.Errorf("invalid idle_action %q", cfg.IdleAction)
	}
	if cfg.IdleTimeoutMinutes < 1 || cfg.IdleTimeoutMinutes > 7*24*60 {
		return fmt.Errorf("idle_timeout_minutes must be between 1 and 10080")
	}
	if cfg.IdleWarningSeconds < 0 || cfg.IdleWarningSeconds > 3600 {
		return fmt.Errorf("idle_warning_seconds must be between 0 and 3600")
	}
	if cfg.NoSleepBatteryThreshold < 0 || cfg.NoSleepBatteryThreshold > 100 {
		return fmt.Errorf("nosleep_battery_threshold must be between 0 and 100")
	}
	if len(cfg.AutomationIssues) > 0 {
		return cfg.AutomationIssues[0]
	}
	if err := automation.ValidateRules(cfg.AutomationRules); err != nil {
		return err
	}
	if cfg.ThemeMode != "fixed" && cfg.ThemeMode != "sunrise" {
		return fmt.Errorf("theme_mode must be fixed or sunrise")
	}
	if _, err := time.Parse("15:04", cfg.ThemeLightTime); err != nil {
		return fmt.Errorf("invalid theme_light_time: %w", err)
	}
	if _, err := time.Parse("15:04", cfg.ThemeDarkTime); err != nil {
		return fmt.Errorf("invalid theme_dark_time: %w", err)
	}
	return nil
}
